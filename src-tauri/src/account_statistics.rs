//! Account-reported activity is independent of locally reconstructed token history.
use serde::{Deserialize, Serialize};

use crate::models::{CodexStatisticsSource, ProviderStatus};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsageSummary {
    pub lifetime_tokens: Option<u64>,
    pub peak_daily_tokens: Option<u64>,
    pub longest_running_turn_sec: Option<u64>,
    pub current_streak_days: Option<u64>,
    pub longest_streak_days: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyTokenUsage {
    pub date: String,
    pub tokens: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsageSnapshot {
    pub source: CodexStatisticsSource,
    pub status: ProviderStatus,
    pub message: Option<String>,
    pub account: Option<String>,
    pub account_id: Option<String>,
    pub summary: AccountUsageSummary,
    pub daily_usage: Option<Vec<DailyTokenUsage>>,
    pub service_updated_at: Option<String>,
    pub updated_at: Option<String>,
}
