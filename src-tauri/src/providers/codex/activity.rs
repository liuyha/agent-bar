//! Account activity from Codex's own profile endpoint, independently of local logs.
//! Contract: openai/codex e22e6523eb5b87f1872e823fa6c97906c2fc671a,
//! backend-client/src/client.rs and backend-client/src/types.rs.
//! No response bodies, bearer credentials, or upstream error text leave this module.

use std::{collections::BTreeMap, env, fs::File, io::Read, path::Path};

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    account_statistics::{AccountUsageSnapshot, AccountUsageSummary, DailyTokenUsage},
    models::{CodexStatisticsSource, ProviderStatus},
};

use super::{auth, http, AppServer, REQUEST_TIMEOUT};

const PROFILE_URL: &str = "https://chatgpt.com/backend-api/wham/profiles/me";
const INVALID_ACTIVITY: &str = "Codex 服务端使用统计格式无法识别，请更新 Codex 后重试";
const OAUTH_REFRESH: &str =
    "Codex OAuth 凭据需要刷新，请先在 Codex 中恢复登录，或选择自动 / CLI 来源";
const PARTIAL_STATS: &str = "服务端提示部分使用统计暂不可用，已展示已返回的数据";
const NO_STATS: &str = "服务端暂未返回可展示的 Token 活动数据";

/// Request guard, including homes without readable credentials. The digest never
/// enters logs, frontend data, settings, or network requests.
pub(crate) fn account_statistics_scope_key() -> String {
    let home = auth::resolve_home(env::var_os("CODEX_HOME"), env::var_os("HOME"));
    scope_key(home.as_deref())
}

fn scope_key(home: Option<&Path>) -> String {
    scope_keys(home).0
}

/// Only an identifiable native login can own a persistent cache. Missing,
/// unreadable, invalid, or API-key-only auth must never share an empty scope.
pub(crate) fn account_statistics_cache_scope_key() -> Option<String> {
    let home = auth::resolve_home(env::var_os("CODEX_HOME"), env::var_os("HOME"));
    cache_scope_key(home.as_deref())
}

pub(super) fn cache_scope_key(home: Option<&Path>) -> Option<String> {
    let (key, cacheable) = scope_keys(home);
    cacheable.then_some(key)
}

fn scope_keys(home: Option<&Path>) -> (String, bool) {
    let mut digest = Sha256::new();
    let mut cacheable = home.is_some();
    if let Some(home) = home {
        digest.update(home.as_os_str().as_encoded_bytes());
        for name in ["auth.json", "config.toml"] {
            digest.update(name.as_bytes());
            let bytes = File::open(home.join(name)).and_then(|file| {
                let mut bytes = Vec::new();
                file.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
                Ok(bytes)
            });
            match bytes {
                Ok(bytes) => {
                    if bytes.len() > 256 * 1024 {
                        cacheable = false;
                    }
                    if name == "auth.json"
                        && !auth::parse(&bytes)
                            .is_ok_and(|auth| auth.pat.is_some() || auth.oauth.is_some())
                    {
                        cacheable = false;
                    }
                    digest.update([1]);
                    digest.update(bytes);
                }
                Err(error) => {
                    if name == "auth.json" || error.kind() != std::io::ErrorKind::NotFound {
                        cacheable = false;
                    }
                    digest.update([0]);
                    digest.update(format!("{:?}", error.kind()).as_bytes());
                }
            }
        }
    } else {
        digest.update(b"no-codex-home");
    }
    (format!("{:x}", digest.finalize()), cacheable)
}

pub(crate) fn collect_account_statistics(source: CodexStatisticsSource) -> AccountUsageSnapshot {
    let now = Utc::now();
    let home = auth::resolve_home(env::var_os("CODEX_HOME"), env::var_os("HOME"));
    let result = collect_scoped(
        source,
        now,
        home.as_deref(),
        |credential| fetch_direct(credential, now),
        || fetch_cli(now, home.as_deref()),
    );
    result.unwrap_or_else(|message| empty_snapshot(source, ProviderStatus::Error, &message))
}

fn collect_scoped(
    source: CodexStatisticsSource,
    now: DateTime<Utc>,
    home: Option<&Path>,
    direct: impl FnMut(http::Credential<'_>) -> Result<AccountUsageSnapshot, http::FetchError>,
    cli: impl FnOnce() -> Result<AccountUsageSnapshot, String>,
) -> Result<AccountUsageSnapshot, String> {
    if source == CodexStatisticsSource::Local {
        return Ok(empty_snapshot(
            source,
            ProviderStatus::Unavailable,
            "当前使用本机日志统计",
        ));
    }
    if source == CodexStatisticsSource::Cli {
        return cli();
    }
    // Match quota collection: custom backends remain owned by the scoped CLI.
    // Explicit direct sources fail closed instead of querying another origin.
    if auth::uses_custom_backend(home) {
        return if source == CodexStatisticsSource::Auto {
            cli()
        } else {
            Err("当前 Codex 使用自定义服务地址，请选择 CLI 来源读取服务端统计".into())
        };
    }
    fetch_with_strategies(source, auth::load(home), now, direct, cli)
}

fn fetch_with_strategies(
    source: CodexStatisticsSource,
    credentials: Result<auth::Credentials, auth::CredentialError>,
    now: DateTime<Utc>,
    mut direct: impl FnMut(http::Credential<'_>) -> Result<AccountUsageSnapshot, http::FetchError>,
    cli: impl FnOnce() -> Result<AccountUsageSnapshot, String>,
) -> Result<AccountUsageSnapshot, String> {
    let automatic = source == CodexStatisticsSource::Auto;
    let credentials =
        match credentials {
            Ok(credentials) => credentials,
            Err(auth::CredentialError::Missing) if automatic => return cli(),
            Err(auth::CredentialError::Unreadable) if automatic => return match cli() {
                Ok(snapshot) if snapshot.status == ProviderStatus::Ready => Ok(snapshot),
                _ => Err(
                    "无法读取 Codex 登录文件，且本机 Codex 未能恢复使用统计；请检查权限或重新登录"
                        .into(),
                ),
            },
            Err(auth::CredentialError::Missing) => {
                return Err("所选来源没有可用的本机 Codex 登录凭据，请先在 Codex 中登录".into())
            }
            Err(auth::CredentialError::Unreadable) => {
                return Err("无法读取 Codex 登录文件，请检查文件权限后重试".into())
            }
            Err(auth::CredentialError::Invalid) => {
                return Err("Codex 登录文件格式无法识别，请在 Codex 中重新登录".into())
            }
        };
    if automatic || source == CodexStatisticsSource::Pat {
        if let Some(token) = credentials.pat.as_deref() {
            match direct(http::Credential::Pat(token)) {
                Ok(snapshot) => return Ok(snapshot),
                Err(http::FetchError::Unauthorized) if automatic => {}
                Err(error) => return Err(error.message()),
            }
        } else if !automatic {
            return Err(
                "当前 Codex 登录文件未配置 PAT，请在 Codex 中配置 PAT 或选择其他来源".into(),
            );
        }
    }
    if credentials.is_api_key {
        return Err(
            "当前 Codex 使用 API Key 登录，无法读取 ChatGPT 账号的服务端 Token 活动".into(),
        );
    }
    if let Some(oauth) = credentials.oauth.as_ref() {
        if oauth.needs_refresh(now) {
            return if automatic {
                cli()
            } else {
                Err(OAUTH_REFRESH.into())
            };
        }
        match direct(http::Credential::OAuth(oauth)) {
            Ok(snapshot) => return Ok(snapshot),
            Err(http::FetchError::Unauthorized) if automatic => {}
            Err(error) => return Err(error.message()),
        }
    } else if !automatic {
        return Err("当前 Codex 登录文件没有 OAuth 凭据，请先使用 ChatGPT 登录 Codex".into());
    }
    cli()
}

fn fetch_direct(
    credential: http::Credential<'_>,
    now: DateTime<Utc>,
) -> Result<AccountUsageSnapshot, http::FetchError> {
    let client = http::client()?;
    fetch_direct_with(credential, now, |url, token, account, pat| {
        http::get_json(&client, url, token, account, pat)
    })
}

fn fetch_direct_with(
    credential: http::Credential<'_>,
    now: DateTime<Utc>,
    mut get: impl FnMut(&str, &str, Option<&str>, bool) -> Result<Value, http::FetchError>,
) -> Result<AccountUsageSnapshot, http::FetchError> {
    let (response, source, email, account_id) = match credential {
        http::Credential::Pat(token) => {
            let whoami = get(http::WHOAMI_URL, token, None, true)?;
            // A PAT's scope must be proved by whoami, never inherited from OAuth.
            let account_id = auth::nonempty(whoami.get("chatgpt_account_id"))
                .ok_or(http::FetchError::InvalidResponse)?;
            let response = get(PROFILE_URL, token, Some(&account_id), true)?;
            (
                response,
                CodexStatisticsSource::Pat,
                auth::nonempty(whoami.get("email")),
                Some(account_id),
            )
        }
        http::Credential::OAuth(oauth) => (
            get(
                PROFILE_URL,
                &oauth.access_token,
                oauth.account_id.as_deref(),
                false,
            )?,
            CodexStatisticsSource::Oauth,
            oauth.email.clone(),
            oauth.account_id.clone(),
        ),
    };
    parse_profile(response, source, email, account_id, now)
}

fn fetch_cli(now: DateTime<Utc>, home: Option<&Path>) -> Result<AccountUsageSnapshot, String> {
    let executable =
        super::find_codex().ok_or_else(|| "未找到本机 Codex，请先安装并登录 Codex".to_string())?;
    let mut server = AppServer::start_scoped(executable, REQUEST_TIMEOUT, home)?;
    let mut snapshot = fetch_cli_with(&mut server, now)?;
    // Account/read currently exposes email, not account ID. Only attach the ID
    // from the same scoped native auth after the read, and never from stale OAuth
    // stored alongside a PAT or API key.
    snapshot.account_id = cli_account_id(home, snapshot.account.as_deref());
    Ok(snapshot)
}

fn cli_account_id(home: Option<&Path>, email: Option<&str>) -> Option<String> {
    let credentials = auth::load(home).ok()?;
    if credentials.pat.is_some() || credentials.is_api_key {
        return None;
    }
    let oauth = credentials.oauth?;
    if let (Some(expected), Some(actual)) = (oauth.email.as_deref(), email) {
        if !expected.eq_ignore_ascii_case(actual.trim()) {
            return None;
        }
    }
    oauth.account_id
}

fn fetch_cli_with(
    server: &mut AppServer,
    now: DateTime<Utc>,
) -> Result<AccountUsageSnapshot, String> {
    server.request(
        1,
        "initialize",
        Some(json!({
            "clientInfo": {"name":"agentbar", "version":env!("CARGO_PKG_VERSION")}
        })),
        "无法初始化 Codex 账号读取服务，请更新 Codex 后重试",
    )?;
    server.send(&json!({"method":"initialized"}))?;
    let account = server.request(
        2,
        "account/read",
        Some(json!({"refreshToken":false})),
        "无法读取 Codex 账号，请确认已在本机登录 Codex",
    )?;
    let account = super::parse_account(&account)?;
    if account.status != ProviderStatus::Ready {
        return Ok(empty_snapshot(
            CodexStatisticsSource::Cli,
            ProviderStatus::Unavailable,
            "当前 Codex 未使用可读取服务端活动的 ChatGPT 账号，请先在 Codex 中登录",
        ));
    }
    let response = server.request(
        3,
        "account/usage/read",
        None,
        "无法读取 Codex 服务端 Token 活动，请检查网络、登录状态及 Codex 版本",
    )?;
    parse_cli(response, account.account, now).map_err(|_| INVALID_ACTIVITY.into())
}

#[derive(Deserialize)]
struct ProfileResponse {
    stats: RawStats,
    metadata: Option<ProfileMetadata>,
}

#[derive(Deserialize)]
struct ProfileMetadata {
    stats_as_of: Option<String>,
    stats_error: Option<String>,
}

#[derive(Deserialize)]
struct RawStats {
    #[serde(alias = "lifetimeTokens")]
    lifetime_tokens: Option<u64>,
    #[serde(alias = "peakDailyTokens")]
    peak_daily_tokens: Option<u64>,
    #[serde(alias = "longestRunningTurnSec")]
    longest_running_turn_sec: Option<u64>,
    #[serde(alias = "currentStreakDays")]
    current_streak_days: Option<u64>,
    #[serde(alias = "longestStreakDays")]
    longest_streak_days: Option<u64>,
    daily_usage_buckets: Option<Vec<RawDay>>,
}

#[derive(Deserialize)]
struct RawDay {
    #[serde(alias = "startDate")]
    start_date: String,
    tokens: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliResponse {
    summary: RawStats,
    daily_usage_buckets: Option<Vec<RawDay>>,
}

fn parse_profile(
    value: Value,
    source: CodexStatisticsSource,
    email: Option<String>,
    account_id: Option<String>,
    now: DateTime<Utc>,
) -> Result<AccountUsageSnapshot, http::FetchError> {
    let profile: ProfileResponse =
        serde_json::from_value(value).map_err(|_| http::FetchError::InvalidResponse)?;
    let mut snapshot = map_stats(profile.stats, source, email, account_id, now)?;
    if let Some(metadata) = profile.metadata {
        snapshot.service_updated_at = metadata.stats_as_of.and_then(valid_service_date);
        if metadata
            .stats_error
            .is_some_and(|error| !error.trim().is_empty())
        {
            snapshot.message = Some(PARTIAL_STATS.into());
        }
    }
    Ok(snapshot)
}

fn parse_cli(
    value: Value,
    email: Option<String>,
    now: DateTime<Utc>,
) -> Result<AccountUsageSnapshot, http::FetchError> {
    let mut response: CliResponse =
        serde_json::from_value(value).map_err(|_| http::FetchError::InvalidResponse)?;
    response.summary.daily_usage_buckets = response.daily_usage_buckets;
    map_stats(
        response.summary,
        CodexStatisticsSource::Cli,
        email,
        None,
        now,
    )
}

fn map_stats(
    stats: RawStats,
    source: CodexStatisticsSource,
    email: Option<String>,
    account_id: Option<String>,
    now: DateTime<Utc>,
) -> Result<AccountUsageSnapshot, http::FetchError> {
    let daily_usage = stats
        .daily_usage_buckets
        .map(|buckets| {
            let mut daily = BTreeMap::new();
            for bucket in buckets {
                if !is_day(&bucket.start_date)
                    || daily.insert(bucket.start_date, bucket.tokens).is_some()
                {
                    return Err(http::FetchError::InvalidResponse);
                }
            }
            Ok(daily
                .into_iter()
                .map(|(date, tokens)| DailyTokenUsage { date, tokens })
                .collect())
        })
        .transpose()?;
    let summary = AccountUsageSummary {
        lifetime_tokens: stats.lifetime_tokens,
        peak_daily_tokens: stats.peak_daily_tokens,
        longest_running_turn_sec: stats.longest_running_turn_sec,
        current_streak_days: stats.current_streak_days,
        longest_streak_days: stats.longest_streak_days,
    };
    let has_data = daily_usage.is_some()
        || [
            summary.lifetime_tokens,
            summary.peak_daily_tokens,
            summary.longest_running_turn_sec,
            summary.current_streak_days,
            summary.longest_streak_days,
        ]
        .iter()
        .any(Option::is_some);
    Ok(AccountUsageSnapshot {
        source,
        status: if has_data {
            ProviderStatus::Ready
        } else {
            ProviderStatus::Unavailable
        },
        message: (!has_data).then(|| NO_STATS.into()),
        account: email,
        account_id,
        summary,
        daily_usage,
        service_updated_at: None,
        updated_at: Some(now.to_rfc3339_opts(SecondsFormat::Secs, true)),
    })
}

fn is_day(value: &str) -> bool {
    value.len() == 10
        && NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .is_ok_and(|date| date.format("%Y-%m-%d").to_string() == value)
}

fn valid_service_date(value: String) -> Option<String> {
    if is_day(&value) {
        return Some(value);
    }
    DateTime::parse_from_rfc3339(&value)
        .ok()
        .map(|date| date.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn empty_snapshot(
    source: CodexStatisticsSource,
    status: ProviderStatus,
    message: &str,
) -> AccountUsageSnapshot {
    AccountUsageSnapshot {
        source,
        status,
        message: Some(message.into()),
        account: None,
        account_id: None,
        summary: AccountUsageSummary::default(),
        daily_usage: None,
        service_updated_at: None,
        updated_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::cell::RefCell;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_789_646_400, 0).unwrap()
    }

    fn credentials(pat: bool, expires: i64) -> auth::Credentials {
        let access = format!(
            "e30.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"exp":expires})).unwrap())
        );
        let identity = format!(
            "e30.{}.signature",
            URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&json!({"email":"oauth@example.test"})).unwrap())
        );
        auth::parse(
            &serde_json::to_vec(&json!({
                "personal_access_token":pat.then_some("synthetic-pat"),
                "tokens":{"access_token":access,"id_token":identity,"account_id":"oauth-account"}
            }))
            .unwrap(),
        )
        .unwrap()
    }

    fn ready(source: CodexStatisticsSource) -> AccountUsageSnapshot {
        parse_profile(
            json!({"stats":{"lifetime_tokens":0}}),
            source,
            None,
            None,
            now(),
        )
        .unwrap()
    }

    #[test]
    fn profile_keeps_missing_empty_and_zero_distinct_and_never_fills_dates() {
        let missing = parse_profile(
            json!({"stats":{}}),
            CodexStatisticsSource::Oauth,
            None,
            None,
            now(),
        )
        .unwrap();
        assert_eq!(missing.status, ProviderStatus::Unavailable);
        assert!(missing.summary.lifetime_tokens.is_none());
        assert!(missing.daily_usage.is_none());
        let zero = ready(CodexStatisticsSource::Oauth);
        assert_eq!(zero.status, ProviderStatus::Ready);
        assert_eq!(zero.summary.lifetime_tokens, Some(0));
        let empty = parse_profile(
            json!({"stats":{"daily_usage_buckets":[]}}),
            CodexStatisticsSource::Oauth,
            None,
            None,
            now(),
        )
        .unwrap();
        assert!(empty.daily_usage.unwrap().is_empty());
        let stats = parse_profile(json!({"stats":{
            "lifetime_tokens":100,"peak_daily_tokens":80,"longest_running_turn_sec":3600,
            "current_streak_days":2,"longest_streak_days":4,
            "daily_usage_buckets":[{"start_date":"2026-09-17","tokens":80},{"start_date":"2026-09-15","tokens":20}]
        }}),CodexStatisticsSource::Oauth,Some("user@example.test".into()),Some("account-test".into()),now()).unwrap();
        assert_eq!(stats.source, CodexStatisticsSource::Oauth);
        assert_eq!(stats.account.as_deref(), Some("user@example.test"));
        assert_eq!(stats.account_id.as_deref(), Some("account-test"));
        assert_eq!(stats.summary.longest_running_turn_sec, Some(3600));
        let daily = stats.daily_usage.unwrap();
        assert_eq!(daily.len(), 2);
        assert_eq!(daily[0].date, "2026-09-15");
        assert_eq!(daily[1].date, "2026-09-17");
        assert_eq!(daily[1].tokens, 80);
    }

    #[test]
    fn invalid_counts_dates_and_duplicate_buckets_fail_without_fabricating_totals() {
        for stats in [
            json!({"lifetime_tokens":-1}),
            json!({"lifetime_tokens":1.5}),
            json!({"daily_usage_buckets":[{"start_date":"2026-09-17","tokens":-1}]}),
            json!({"daily_usage_buckets":[{"start_date":"2026-02-30","tokens":1}]}),
            json!({"daily_usage_buckets":[{"start_date":"2026-9-17","tokens":1}]}),
            json!({"daily_usage_buckets":[{"start_date":"2026-09-17","tokens":1},{"start_date":"2026-09-17","tokens":1}]}),
        ] {
            assert_eq!(
                parse_profile(
                    json!({"stats":stats}),
                    CodexStatisticsSource::Pat,
                    None,
                    None,
                    now()
                )
                .unwrap_err(),
                http::FetchError::InvalidResponse
            );
        }
        for value in [json!({}), json!([]), json!({"stats":null})] {
            assert!(parse_profile(value, CodexStatisticsSource::Pat, None, None, now()).is_err());
        }
    }

    #[test]
    fn service_metadata_preserves_valid_dates_but_never_exposes_upstream_error_text() {
        let snapshot = parse_profile(
            json!({"stats":{"lifetime_tokens":12},"metadata":{
                "stats_as_of":"2026-09-09","stats_error":"private response with synthetic-secret"
            }}),
            CodexStatisticsSource::Oauth,
            None,
            None,
            now(),
        )
        .unwrap();
        assert_eq!(snapshot.service_updated_at.as_deref(), Some("2026-09-09"));
        assert_eq!(snapshot.message.as_deref(), Some(PARTIAL_STATS));
        assert!(!serde_json::to_string(&snapshot).unwrap().contains("secret"));
        let snapshot = parse_profile(
            json!({"stats":{"lifetime_tokens":12},"metadata":{
                "stats_as_of":"private upstream string","stats_error":""
            }}),
            CodexStatisticsSource::Oauth,
            None,
            None,
            now(),
        )
        .unwrap();
        assert!(snapshot.service_updated_at.is_none());
        assert!(snapshot.message.is_none());
        assert_eq!(
            valid_service_date("2026-09-09T12:00:00+00:00".into()).as_deref(),
            Some("2026-09-09T12:00:00Z")
        );
    }

    #[test]
    fn direct_pat_and_oauth_requests_use_only_their_own_account_scope() {
        let mut calls = Vec::new();
        let pat = fetch_direct_with(
            http::Credential::Pat("synthetic-pat"),
            now(),
            |url, token, account, pat| {
                assert_eq!(token, "synthetic-pat");
                assert!(pat);
                calls.push(url.to_owned());
                if url == http::WHOAMI_URL {
                    assert!(account.is_none());
                    Ok(json!({"chatgpt_account_id":"pat-account","email":"pat@example.test"}))
                } else {
                    assert_eq!(account, Some("pat-account"));
                    Ok(json!({"stats":{"lifetime_tokens":42}}))
                }
            },
        )
        .unwrap();
        assert_eq!(calls, vec![http::WHOAMI_URL, PROFILE_URL]);
        assert_eq!(pat.source, CodexStatisticsSource::Pat);
        assert_eq!(pat.account_id.as_deref(), Some("pat-account"));
        assert_eq!(pat.account.as_deref(), Some("pat@example.test"));
        let credentials = credentials(false, now().timestamp() + 3600);
        let oauth = credentials.oauth.as_ref().unwrap();
        let snapshot = fetch_direct_with(
            http::Credential::OAuth(oauth),
            now(),
            |url, token, account, pat| {
                assert_eq!(url, PROFILE_URL);
                assert_eq!(token, oauth.access_token);
                assert_eq!(account, Some("oauth-account"));
                assert!(!pat);
                Ok(json!({"stats":{"lifetime_tokens":21}}))
            },
        )
        .unwrap();
        assert_eq!(snapshot.source, CodexStatisticsSource::Oauth);
        assert_eq!(snapshot.account.as_deref(), Some("oauth@example.test"));
        assert_eq!(
            fetch_direct_with(
                http::Credential::Pat("synthetic-pat"),
                now(),
                |url, _, _, _| {
                    assert_eq!(url, http::WHOAMI_URL);
                    Ok(json!({"email":"pat@example.test"}))
                }
            )
            .unwrap_err(),
            http::FetchError::InvalidResponse
        );
    }

    #[test]
    fn auto_retries_only_authorization_failures_and_reports_the_effective_source() {
        let calls = RefCell::new(Vec::new());
        let snapshot = fetch_with_strategies(
            CodexStatisticsSource::Auto,
            Ok(credentials(true, now().timestamp() + 3600)),
            now(),
            |credential| {
                calls.borrow_mut().push(match credential {
                    http::Credential::Pat(_) => "pat",
                    http::Credential::OAuth(_) => "oauth",
                });
                Err(http::FetchError::Unauthorized)
            },
            || {
                calls.borrow_mut().push("cli");
                Ok(ready(CodexStatisticsSource::Cli))
            },
        )
        .unwrap();
        assert_eq!(snapshot.source, CodexStatisticsSource::Cli);
        assert_eq!(*calls.borrow(), vec!["pat", "oauth", "cli"]);
        let snapshot = fetch_with_strategies(
            CodexStatisticsSource::Auto,
            Ok(credentials(true, now().timestamp() + 3600)),
            now(),
            |credential| match credential {
                http::Credential::Pat(_) => Err(http::FetchError::Unauthorized),
                http::Credential::OAuth(_) => Ok(ready(CodexStatisticsSource::Oauth)),
            },
            || panic!("successful direct response must not start CLI"),
        )
        .unwrap();
        assert_eq!(snapshot.source, CodexStatisticsSource::Oauth);
        for error in [
            http::FetchError::Network,
            http::FetchError::Server(403),
            http::FetchError::Server(429),
            http::FetchError::InvalidResponse,
        ] {
            let message = fetch_with_strategies(
                CodexStatisticsSource::Auto,
                Ok(credentials(true, now().timestamp() + 3600)),
                now(),
                |credential| {
                    assert!(matches!(credential, http::Credential::Pat(_)));
                    Err(error)
                },
                || panic!("network, forbidden, throttling and parsing errors must stop"),
            )
            .unwrap_err();
            assert_eq!(message, error.message());
        }
    }

    #[test]
    fn explicit_sources_never_silently_fallback_and_oauth_expiry_is_explicit() {
        for source in [CodexStatisticsSource::Pat, CodexStatisticsSource::Oauth] {
            let error = fetch_with_strategies(
                source,
                Ok(credentials(true, now().timestamp() + 3600)),
                now(),
                |credential| {
                    assert_eq!(
                        matches!(credential, http::Credential::Pat(_)),
                        source == CodexStatisticsSource::Pat
                    );
                    Err(http::FetchError::Unauthorized)
                },
                || panic!("explicit source must not fallback"),
            )
            .unwrap_err();
            assert_eq!(error, http::FetchError::Unauthorized.message());
        }
        assert!(fetch_with_strategies(
            CodexStatisticsSource::Pat,
            Ok(credentials(false, now().timestamp() + 3600)),
            now(),
            |_| panic!("no PAT"),
            || panic!("no fallback")
        )
        .unwrap_err()
        .contains("PAT"));
        assert_eq!(
            fetch_with_strategies(
                CodexStatisticsSource::Oauth,
                Ok(credentials(false, now().timestamp() - 1)),
                now(),
                |_| panic!("expired token must not be used"),
                || panic!("explicit source must not fallback")
            )
            .unwrap_err(),
            OAUTH_REFRESH
        );
        let snapshot = fetch_with_strategies(
            CodexStatisticsSource::Auto,
            Ok(credentials(false, now().timestamp() - 1)),
            now(),
            |_| panic!("expired token must not be used"),
            || Ok(ready(CodexStatisticsSource::Cli)),
        )
        .unwrap();
        assert_eq!(snapshot.source, CodexStatisticsSource::Cli);
    }

    #[test]
    fn missing_invalid_and_api_key_auth_keep_native_scope_and_fail_closed() {
        let snapshot = fetch_with_strategies(
            CodexStatisticsSource::Auto,
            Err(auth::CredentialError::Missing),
            now(),
            |_| panic!("no auth"),
            || Ok(ready(CodexStatisticsSource::Cli)),
        )
        .unwrap();
        assert_eq!(snapshot.source, CodexStatisticsSource::Cli);
        for error in [
            auth::CredentialError::Invalid,
            auth::CredentialError::Unreadable,
            auth::CredentialError::Missing,
        ] {
            assert!(fetch_with_strategies(
                CodexStatisticsSource::Oauth,
                Err(error),
                now(),
                |_| panic!("invalid auth"),
                || panic!("explicit source cannot use CLI")
            )
            .is_err());
        }
        assert!(fetch_with_strategies(
            CodexStatisticsSource::Auto,
            Err(auth::CredentialError::Invalid),
            now(),
            |_| panic!("invalid auth"),
            || panic!("malformed auth must stop")
        )
        .is_err());
        let credentials = auth::parse(br#"{"OPENAI_API_KEY":"synthetic-api-secret"}"#).unwrap();
        assert!(fetch_with_strategies(
            CodexStatisticsSource::Auto,
            Ok(credentials),
            now(),
            |_| panic!("API key cannot query account profile"),
            || panic!("API key auth is already known")
        )
        .unwrap_err()
        .contains("API Key"));
    }

    #[test]
    fn local_and_custom_backend_choices_never_send_credentials_to_other_origins() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("config.toml"),
            "chatgpt_base_url = 'https://custom.example.test'",
        )
        .unwrap();
        let local = collect_scoped(
            CodexStatisticsSource::Local,
            now(),
            Some(directory.path()),
            |_| panic!("local does not query"),
            || panic!("local does not start CLI"),
        )
        .unwrap();
        assert_eq!(local.source, CodexStatisticsSource::Local);
        for source in [CodexStatisticsSource::Oauth, CodexStatisticsSource::Pat] {
            assert!(collect_scoped(
                source,
                now(),
                Some(directory.path()),
                |_| panic!("custom backend rejects direct reads"),
                || panic!("explicit source cannot fallback")
            )
            .is_err());
        }
        let automatic = collect_scoped(
            CodexStatisticsSource::Auto,
            now(),
            Some(directory.path()),
            |_| panic!("custom backend must use its native owner"),
            || Ok(ready(CodexStatisticsSource::Cli)),
        )
        .unwrap();
        assert_eq!(automatic.source, CodexStatisticsSource::Cli);
    }

    #[test]
    fn request_scope_digest_changes_with_account_configuration_without_modifying_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let first = scope_key(Some(directory.path()));
        let contents = br#"{"personal_access_token":"synthetic-secret"}"#;
        std::fs::write(directory.path().join("auth.json"), contents).unwrap();
        let second = scope_key(Some(directory.path()));
        assert_ne!(first, second);
        assert!(!second.contains("secret"));
        assert_eq!(second, scope_key(Some(directory.path())));
        assert_eq!(
            std::fs::read(directory.path().join("auth.json")).unwrap(),
            contents
        );
        std::fs::write(
            directory.path().join("config.toml"),
            "chatgpt_base_url='https://custom.example.test'",
        )
        .unwrap();
        assert_ne!(second, scope_key(Some(directory.path())));
    }

    #[test]
    fn cache_scope_requires_a_readable_identifiable_login_and_configuration() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!scope_keys(None).1);
        assert!(!scope_keys(Some(directory.path())).1);
        for auth in ["", "invalid", "{}", r#"{"OPENAI_API_KEY":"synthetic-key"}"#] {
            std::fs::write(directory.path().join("auth.json"), auth).unwrap();
            assert!(!scope_keys(Some(directory.path())).1);
        }
        std::fs::write(
            directory.path().join("auth.json"),
            br#"{"personal_access_token":"synthetic-token-a"}"#,
        )
        .unwrap();
        let (first, valid) = scope_keys(Some(directory.path()));
        assert!(valid);
        assert!(!first.contains("synthetic-token"));
        std::fs::write(
            directory.path().join("auth.json"),
            br#"{"personal_access_token":"synthetic-token-b"}"#,
        )
        .unwrap();
        let (second, valid) = scope_keys(Some(directory.path()));
        assert!(valid);
        assert_ne!(first, second);
        std::fs::create_dir(directory.path().join("config.toml")).unwrap();
        assert!(!scope_keys(Some(directory.path())).1);
        std::fs::remove_dir(directory.path().join("config.toml")).unwrap();
        std::fs::write(
            directory.path().join("auth.json"),
            vec![b'x'; 256 * 1024 + 1],
        )
        .unwrap();
        assert!(!scope_keys(Some(directory.path())).1);
        std::fs::remove_file(directory.path().join("auth.json")).unwrap();
        std::fs::create_dir(directory.path().join("auth.json")).unwrap();
        assert!(!scope_keys(Some(directory.path())).1);
    }

    #[test]
    fn cli_account_id_does_not_attach_stale_oauth_to_pat_or_another_email() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("auth.json");
        let identity = format!(
            "e30.{}.signature",
            URL_SAFE_NO_PAD.encode(br#"{"email":"oauth@example.test"}"#)
        );
        let oauth = json!({"tokens":{"access_token":"synthetic-oauth","account_id":"oauth-account","id_token":identity}});
        std::fs::write(&file, serde_json::to_vec(&oauth).unwrap()).unwrap();
        assert_eq!(
            cli_account_id(Some(directory.path()), Some("oauth@example.test")).as_deref(),
            Some("oauth-account")
        );
        assert!(cli_account_id(Some(directory.path()), Some("different@example.test")).is_none());
        let mut mixed = oauth;
        mixed["personal_access_token"] = json!("synthetic-pat");
        std::fs::write(&file, serde_json::to_vec(&mixed).unwrap()).unwrap();
        assert!(cli_account_id(Some(directory.path()), Some("oauth@example.test")).is_none());
    }

    #[test]
    fn http_profile_mock_exercises_pat_identity_and_authenticated_profile_transport() {
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
            thread,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for body in [
                r#"{"chatgpt_account_id":"pat-account","email":"pat@example.test"}"#,
                r#"{"stats":{"lifetime_tokens":0,"daily_usage_buckets":[]},"metadata":{"stats_as_of":"2026-09-17"}}"#,
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    request.push_str(&line);
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                requests.push(request.to_lowercase());
            }
            requests
        });
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let snapshot = fetch_direct_with(
            http::Credential::Pat("synthetic-pat"),
            now(),
            |url, token, account, pat| {
                let path = match url {
                    http::WHOAMI_URL => "/whoami",
                    PROFILE_URL => "/backend-api/wham/profiles/me",
                    _ => panic!("only verified identity and profile endpoints are permitted"),
                };
                http::get_json(
                    &client,
                    &format!("http://{address}{path}"),
                    token,
                    account,
                    pat,
                )
            },
        )
        .unwrap();
        assert_eq!(snapshot.summary.lifetime_tokens, Some(0));
        assert!(snapshot.daily_usage.unwrap().is_empty());
        assert_eq!(snapshot.service_updated_at.as_deref(), Some("2026-09-17"));
        let requests = server.join().unwrap();
        assert!(!requests[0].contains("chatgpt-account-id:"));
        assert!(requests[1].starts_with("get /backend-api/wham/profiles/me "));
        assert!(requests[1].contains("chatgpt-account-id: pat-account"));
        for request in requests {
            assert!(request.contains("authorization: bearer synthetic-pat"));
            assert!(request.contains("originator: codex_cli_rs"));
        }
    }

    #[cfg(unix)]
    fn fake_server(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codex");
        std::fs::write(&path, format!("#!/bin/sh\n{contents}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (directory, path)
    }

    #[test]
    #[cfg(unix)]
    fn cli_reads_real_account_activity_rpc_with_the_same_home_and_redacts_errors() {
        let (_directory, path) = fake_server(
            r#"
read -r request
printf '%s\n' '{"id":1,"result":{}}'
read -r notification
read -r request
printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt","email":"cli@example.test"}}}'
read -r request
case "$request" in *'account/usage/read'*) ;; *) exit 3 ;; esac
case "$CODEX_HOME" in '/synthetic/scoped-home') ;; *) exit 4 ;; esac
printf '%s\n' '{"id":3,"result":{"summary":{"lifetimeTokens":42,"peakDailyTokens":40},"dailyUsageBuckets":[{"startDate":"2026-09-17","tokens":40}]}}'
"#,
        );
        let mut server = AppServer::start_scoped(
            path,
            std::time::Duration::from_secs(2),
            Some(Path::new("/synthetic/scoped-home")),
        )
        .unwrap();
        let snapshot = fetch_cli_with(&mut server, now()).unwrap();
        assert_eq!(snapshot.source, CodexStatisticsSource::Cli);
        assert_eq!(snapshot.account.as_deref(), Some("cli@example.test"));
        assert_eq!(snapshot.summary.lifetime_tokens, Some(42));
        assert_eq!(snapshot.daily_usage.unwrap()[0].tokens, 40);
        let (_directory, path) = fake_server(
            r#"
read -r request
printf '%s\n' '{"id":1,"result":{}}'
read -r notification
read -r request
printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt","email":"cli@example.test"}}}'
read -r request
printf '%s\n' '{"id":3,"error":{"message":"private body synthetic-secret"}}'
"#,
        );
        let mut server =
            AppServer::start_scoped(path, std::time::Duration::from_secs(2), None).unwrap();
        let error = fetch_cli_with(&mut server, now()).unwrap_err();
        assert!(error.contains("服务端 Token 活动"));
        assert!(!error.contains("secret"));
    }

    #[test]
    #[ignore = "Reads the current native Codex account and live account activity"]
    fn live_account_statistics() {
        let snapshot = collect_account_statistics(CodexStatisticsSource::Auto);
        let summary_fields = [
            snapshot.summary.lifetime_tokens,
            snapshot.summary.peak_daily_tokens,
            snapshot.summary.longest_running_turn_sec,
            snapshot.summary.current_streak_days,
            snapshot.summary.longest_streak_days,
        ]
        .into_iter()
        .filter(Option::is_some)
        .count();
        println!("status={:?}, source={:?}, summary_fields={summary_fields}, daily_count={:?}, service_updated_at={:?}",
            snapshot.status, snapshot.source, snapshot.daily_usage.as_ref().map(Vec::len), snapshot.service_updated_at);
        assert_eq!(snapshot.status, ProviderStatus::Ready);
        assert!(summary_fields > 0 || snapshot.daily_usage.is_some());
    }
}
