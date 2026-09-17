use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Codex,
    Claude,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CodexStatisticsSource {
    #[default]
    Local,
    Auto,
    Oauth,
    Pat,
    Cli,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", from = "CompatibleAppSettings")]
pub struct AppSettings {
    pub refresh_interval_seconds: u64,
    pub enabled_providers: Vec<ProviderId>,
    pub theme: Theme,
    #[serde(default)]
    pub codex_statistics_source: CodexStatisticsSource,
}

// Accept the removed preference in older settings, but never expose or persist it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompatibleAppSettings {
    refresh_interval_seconds: u64,
    enabled_providers: Vec<ProviderId>,
    theme: Theme,
    #[serde(default)]
    codex_statistics_source: CodexStatisticsSource,
    #[serde(default, rename = "codexWebExtras")]
    _removed_web_extras: serde::de::IgnoredAny,
}

impl From<CompatibleAppSettings> for AppSettings {
    fn from(settings: CompatibleAppSettings) -> Self {
        Self {
            refresh_interval_seconds: settings.refresh_interval_seconds,
            enabled_providers: settings.enabled_providers,
            theme: settings.theme,
            codex_statistics_source: settings.codex_statistics_source,
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: 300,
            enabled_providers: vec![ProviderId::Codex, ProviderId::Claude],
            theme: Theme::System,
            codex_statistics_source: CodexStatisticsSource::Local,
        }
    }
}

impl AppSettings {
    pub fn validated(mut self) -> Result<Self, String> {
        if ![60, 300, 900].contains(&self.refresh_interval_seconds) {
            return Err("刷新间隔仅支持 60、300 或 900 秒".into());
        }

        let mut unique = Vec::with_capacity(2);
        for provider in self.enabled_providers {
            if !unique.contains(&provider) {
                unique.push(provider);
            }
        }
        self.enabled_providers = unique;
        // Older versions exposed authentication strategies as preferences. Every
        // remote preference now uses automatic selection; retain the enum internally
        // to describe the actual transport and decode existing settings.
        if self.codex_statistics_source != CodexStatisticsSource::Local {
            self.codex_statistics_source = CodexStatisticsSource::Auto;
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DataMode {
    Local,
    Live,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderStatus {
    Ready,
    Unavailable,
    Error,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub label: String,
    pub used_percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    pub id: ProviderId,
    pub name: String,
    pub plan: String,
    pub source: DataMode,
    pub status: ProviderStatus,
    pub account: Option<String>,
    pub message: Option<String>,
    pub windows: Vec<UsageWindow>,
    pub updated_at: Option<String>,
    /// Credential scope stays inside Rust and its private disk envelope, never IPC.
    #[serde(skip)]
    pub(crate) cache_scope: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSnapshot {
    pub providers: Vec<ProviderUsage>,
    pub updated_at: String,
    pub mode: DataMode,
    pub revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_validate_intervals_and_deduplicate_ids_in_order() {
        let settings = AppSettings {
            enabled_providers: vec![ProviderId::Claude, ProviderId::Codex, ProviderId::Claude],
            ..AppSettings::default()
        }
        .validated()
        .unwrap();
        assert_eq!(
            settings.enabled_providers,
            [ProviderId::Claude, ProviderId::Codex]
        );

        for invalid_interval in [0, 1, 59, 301, u64::MAX] {
            assert!(AppSettings {
                refresh_interval_seconds: invalid_interval,
                ..AppSettings::default()
            }
            .validated()
            .is_err());
        }
    }

    #[test]
    fn settings_use_frontend_camel_case_and_reject_unknown_provider() {
        let value = serde_json::to_value(AppSettings::default()).unwrap();
        assert_eq!(
            value,
            json!({
                "refreshIntervalSeconds": 300,
                "enabledProviders": ["codex", "claude"],
                "theme": "system",
                "codexStatisticsSource": "local"
            })
        );
        assert!(serde_json::from_value::<AppSettings>(json!({
            "refreshIntervalSeconds": 300,
            "enabledProviders": ["unknown"],
            "theme": "system"
        }))
        .is_err());
    }

    #[test]
    fn older_settings_keep_local_statistics_and_discard_removed_web_preference() {
        let old = json!({
            "refreshIntervalSeconds": 300,
            "enabledProviders": ["codex"],
            "theme": "system"
        });
        let restored: AppSettings = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(
            restored.codex_statistics_source,
            CodexStatisticsSource::Local
        );
        let mut selected = old;
        selected["codexStatisticsSource"] = json!("pat");
        selected["codexWebExtras"] = json!(true);
        for legacy in ["auto", "oauth", "pat", "cli"] {
            selected["codexStatisticsSource"] = json!(legacy);
            let settings = serde_json::from_value::<AppSettings>(selected.clone())
                .unwrap()
                .validated()
                .unwrap();
            assert_eq!(
                settings.codex_statistics_source,
                CodexStatisticsSource::Auto
            );
            assert!(serde_json::to_value(settings)
                .unwrap()
                .get("codexWebExtras")
                .is_none());
        }
        selected["codexStatisticsSource"] = json!("cookies");
        assert!(serde_json::from_value::<AppSettings>(selected).is_err());
    }

    #[test]
    fn settings_still_reject_unknown_preferences() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value["unknownSetting"] = json!(true);
        assert!(serde_json::from_value::<AppSettings>(value).is_err());
    }
}
