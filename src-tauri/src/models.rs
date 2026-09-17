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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppSettings {
    pub refresh_interval_seconds: u64,
    pub enabled_providers: Vec<ProviderId>,
    pub theme: Theme,
    #[serde(default)]
    pub codex_statistics_source: CodexStatisticsSource,
    #[serde(default)]
    pub codex_web_extras: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: 300,
            enabled_providers: vec![ProviderId::Codex, ProviderId::Claude],
            theme: Theme::System,
            codex_statistics_source: CodexStatisticsSource::Local,
            codex_web_extras: false,
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
                "codexStatisticsSource": "local",
                "codexWebExtras": false
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
    fn older_settings_keep_local_statistics_and_web_extras_opted_out() {
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
        assert!(!restored.codex_web_extras);
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
            assert!(settings.codex_web_extras);
        }
        selected["codexStatisticsSource"] = json!("cookies");
        assert!(serde_json::from_value::<AppSettings>(selected).is_err());
    }
}
