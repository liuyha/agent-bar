//! Read local account state inside Rust; only account metadata and usage reach the UI.
mod claude;
mod codex;

pub(crate) use codex::{
    account_statistics_cache_scope_key, account_statistics_scope_key, collect_account_statistics,
};

use chrono::{DateTime, SecondsFormat, Utc};

use crate::models::{
    AppSettings, DashboardSnapshot, DataMode, ProviderId, ProviderStatus, ProviderUsage,
};

pub trait UsageProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn fetch_usage(&self, now: DateTime<Utc>) -> Result<ProviderUsage, String>;
}

pub fn local_providers() -> Vec<Box<dyn UsageProvider>> {
    vec![
        Box::new(codex::CodexProvider),
        Box::new(claude::ClaudeProvider),
    ]
}

pub fn pending_provider(id: ProviderId) -> ProviderUsage {
    ProviderUsage {
        id,
        name: match id {
            ProviderId::Codex => "Codex",
            ProviderId::Claude => "Claude",
        }
        .into(),
        plan: String::new(),
        source: DataMode::Local,
        status: ProviderStatus::Unavailable,
        account: None,
        message: Some("正在读取本机账号…".into()),
        windows: Vec::new(),
        updated_at: None,
    }
}

pub fn initial_snapshot(settings: &AppSettings) -> DashboardSnapshot {
    DashboardSnapshot {
        providers: settings
            .enabled_providers
            .iter()
            .copied()
            .map(pending_provider)
            .collect(),
        updated_at: iso_time(Utc::now()),
        mode: DataMode::Live,
        revision: 0,
    }
}

pub fn collect_snapshot(
    settings: &AppSettings,
    providers: &[Box<dyn UsageProvider>],
    now: DateTime<Utc>,
) -> DashboardSnapshot {
    // Providers run independently: a missing Claude login cannot hide Codex usage.
    let usages = std::thread::scope(|scope| {
        let workers: Vec<_> = providers
            .iter()
            .filter(|provider| settings.enabled_providers.contains(&provider.id()))
            .map(|provider| {
                (
                    provider.id(),
                    scope.spawn(move || provider.fetch_usage(now)),
                )
            })
            .collect();
        workers
            .into_iter()
            .map(|(id, worker)| {
                match worker
                    .join()
                    .unwrap_or_else(|_| Err("读取账号时发生异常，请重试".into()))
                {
                    Ok(usage) => usage,
                    Err(message) => ProviderUsage {
                        status: ProviderStatus::Error,
                        message: Some(message),
                        ..pending_provider(id)
                    },
                }
            })
            .collect()
    });
    DashboardSnapshot {
        providers: usages,
        updated_at: iso_time(Utc::now()),
        mode: DataMode::Live,
        revision: 0,
    }
}

fn iso_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingProvider;
    impl UsageProvider for FailingProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Claude
        }
        fn fetch_usage(&self, _: DateTime<Utc>) -> Result<ProviderUsage, String> {
            Err("服务暂时不可用".into())
        }
    }

    #[test]
    fn failures_are_isolated_and_disabled_providers_are_not_collected() {
        let providers: Vec<Box<dyn UsageProvider>> = vec![Box::new(FailingProvider)];
        let snapshot = collect_snapshot(&AppSettings::default(), &providers, Utc::now());
        assert_eq!(snapshot.providers[0].status, ProviderStatus::Error);
        assert!(snapshot.providers[0].windows.is_empty());
        let json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(json["mode"], "live");
        assert_eq!(json["providers"][0]["source"], "local");
        assert!(json["providers"][0]["updatedAt"].is_null());
        let disabled = AppSettings {
            enabled_providers: vec![],
            ..AppSettings::default()
        };
        assert!(collect_snapshot(&disabled, &providers, Utc::now())
            .providers
            .is_empty());
    }
}
