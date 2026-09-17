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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppSettings {
    pub refresh_interval_seconds: u64,
    pub enabled_providers: Vec<ProviderId>,
    pub theme: Theme,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: 300,
            enabled_providers: vec![ProviderId::Codex, ProviderId::Claude],
            theme: Theme::System,
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
                "theme": "system"
            })
        );
        assert!(serde_json::from_value::<AppSettings>(json!({
            "refreshIntervalSeconds": 300,
            "enabledProviders": ["unknown"],
            "theme": "system"
        }))
        .is_err());
    }
}
