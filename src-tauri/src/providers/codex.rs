//! Codex quota collection follows CodexBar's PAT → OAuth → native CLI strategy.
//! Independently implemented from the public protocol and CodexBar behavior at
//! https://github.com/steipete/CodexBar/tree/2d9334237e7c48cf18799c7302ce3229069f3a81
//! Credential files are read-only; native Codex owns token refresh. Only mapped
//! account metadata and quota fields leave this module, never raw HTTP errors.

mod activity;
mod auth;
mod http;

pub(crate) use activity::{
    account_statistics_cache_scope_key, account_statistics_scope_key, collect_account_statistics,
};

use std::{
    env,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};

use crate::models::{DataMode, ProviderId, ProviderStatus, ProviderUsage, UsageWindow};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE_BYTES: u64 = 512 * 1024;
const INVALID_RESPONSE: &str = "Codex 返回的数据格式无法识别，请更新 Codex 后重试";

pub struct CodexProvider;

impl super::UsageProvider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn fetch_usage(&self, now: DateTime<Utc>) -> Result<ProviderUsage, String> {
        let home = auth::resolve_home(env::var_os("CODEX_HOME"), env::var_os("HOME"));
        let scope = activity::cache_scope_key(home.as_deref());
        let credentials = auth::load(home.as_deref());
        let mut authentication_failed = false;
        let usable_credentials = credentials.as_ref().is_ok_and(|credentials| {
            credentials.pat.is_some()
                || credentials
                    .oauth
                    .as_ref()
                    .is_some_and(|oauth| !oauth.needs_refresh(now))
        });
        // Preserve the CLI's custom backend configuration without forwarding a
        // native OpenAI token to a configurable destination in this process.
        let result = if auth::uses_custom_backend(home.as_deref()) {
            fetch_cli_usage(now, home.as_deref())
        } else {
            fetch_with_strategies(
                credentials,
                now,
                |credential| {
                    let result = http::fetch(credential, now);
                    authentication_failed |= matches!(result, Err(http::FetchError::Unauthorized));
                    result
                },
                || fetch_cli_usage(now, home.as_deref()),
            )
        };
        let mut usage = result.unwrap_or_else(|message| ProviderUsage {
            status: ProviderStatus::Error,
            message: Some(message),
            ..unavailable("")
        });
        let current_scope = activity::cache_scope_key(home.as_deref());
        // If the login changes in flight, do not bind this result to the new login.
        // A later successful collection will establish its own stable scope.
        usage.cache_scope = if scope == current_scope
            && (usage.status == ProviderStatus::Ready
                || (usable_credentials && !authentication_failed))
        {
            current_scope
        } else {
            None
        };
        Ok(usage)
    }
}

fn fetch_with_strategies(
    credentials: Result<auth::Credentials, auth::CredentialError>,
    now: DateTime<Utc>,
    mut direct: impl FnMut(http::Credential<'_>) -> Result<ProviderUsage, http::FetchError>,
    cli: impl FnOnce() -> Result<ProviderUsage, String>,
) -> Result<ProviderUsage, String> {
    let credentials = match credentials {
        Ok(value) => value,
        Err(auth::CredentialError::Invalid) => {
            return Err("Codex 登录文件格式无法识别，请在 Codex 中重新登录".into())
        }
        Err(auth::CredentialError::Missing) => return cli(),
        Err(auth::CredentialError::Unreadable) => return match cli() {
            Ok(usage) if usage.status == ProviderStatus::Ready => Ok(usage),
            _ => Err(
                "无法读取 Codex 登录文件，且本机 Codex 未能恢复用量；请检查登录文件权限或重新登录"
                    .into(),
            ),
        },
    };
    if let Some(token) = credentials.pat.as_deref() {
        match direct(http::Credential::Pat(token)) {
            Ok(usage) => return Ok(usage),
            Err(http::FetchError::Unauthorized) => {}
            Err(error) => return Err(error.message()),
        }
    }
    if credentials.is_api_key {
        return parse_account(&json!({"account": {"type": "apiKey"}}));
    }
    if let Some(oauth) = credentials.oauth.as_ref() {
        if oauth.needs_refresh(now) {
            return cli();
        }
        match direct(http::Credential::OAuth(oauth)) {
            Ok(usage) => return Ok(usage),
            Err(http::FetchError::Unauthorized) => {}
            Err(error) => return Err(error.message()),
        }
    }
    cli()
}

fn fetch_cli_usage(now: DateTime<Utc>, home: Option<&Path>) -> Result<ProviderUsage, String> {
    let Some(executable) = find_codex() else {
        return Ok(unavailable("未找到本机 Codex，请先安装并登录 Codex"));
    };
    let mut server = AppServer::start_scoped(executable, REQUEST_TIMEOUT, home)?;
    server.request(
        1,
        "initialize",
        Some(json!({
            "clientInfo": { "name": "agentbar", "version": env!("CARGO_PKG_VERSION") }
        })),
        "无法初始化 Codex 账号读取服务，请更新 Codex 后重试",
    )?;
    server.send(&json!({ "method": "initialized" }))?;
    // Do not explicitly request login, logout, or quota reset. The native CLI
    // alone owns any refresh required while fetching its own account limits.
    let account = server.request(
        2,
        "account/read",
        Some(json!({ "refreshToken": false })),
        "无法读取 Codex 账号，请确认已在本机登录 Codex",
    )?;
    let mut usage = parse_account(&account)?;
    if usage.status != ProviderStatus::Ready {
        return Ok(usage);
    }
    match server.request(
        3,
        "account/rateLimits/read",
        None,
        "无法读取 Codex 用量，请检查网络及 Codex 登录状态后重试",
    ) {
        Ok(limits) => match apply_limits(&mut usage, &limits, now) {
            Ok(()) => {}
            Err(message) => {
                usage.status = ProviderStatus::Error;
                usage.message = Some(message);
            }
        },
        Err(message) => {
            usage.status = ProviderStatus::Error;
            usage.message = Some(message);
        }
    }
    Ok(usage)
}

fn unavailable(message: &str) -> ProviderUsage {
    ProviderUsage {
        id: ProviderId::Codex,
        name: "Codex".into(),
        plan: "未连接".into(),
        source: DataMode::Local,
        status: ProviderStatus::Unavailable,
        account: None,
        message: Some(message.into()),
        windows: vec![],
        updated_at: None,
        cache_scope: None,
    }
}

fn parse_account(result: &Value) -> Result<ProviderUsage, String> {
    let mut usage = unavailable("未找到本机 Codex 登录账号，请先在 Codex 中登录");
    let Some(account) = result.get("account").filter(|value| !value.is_null()) else {
        return Ok(usage);
    };
    match account.get("type").and_then(Value::as_str) {
        Some("chatgpt") => {
            usage.status = ProviderStatus::Ready;
            usage.plan = plan_label(account.get("planType").and_then(Value::as_str));
            usage.account = account
                .get("email")
                .and_then(Value::as_str)
                .filter(|email| !email.trim().is_empty())
                .map(str::to_owned);
            usage.message = None;
        }
        Some("apiKey") => {
            usage.plan = "API Key".into();
            usage.message = Some("当前 Codex 使用 API Key 登录，无法读取 ChatGPT 订阅额度".into());
        }
        Some(_) => {
            usage.message = Some("当前 Codex 登录方式不提供 ChatGPT 订阅额度".into());
        }
        None => return Err(INVALID_RESPONSE.into()),
    }
    Ok(usage)
}

fn apply_limits(
    usage: &mut ProviderUsage,
    result: &Value,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let mut buckets = Vec::new();
    if let Some(by_id) = result.get("rateLimitsByLimitId").and_then(Value::as_object) {
        // Prefer the current multi-bucket response over the legacy duplicate.
        if let Some(core) = by_id.get("codex") {
            buckets.push(("codex", core));
        }
        buckets.extend(
            by_id
                .iter()
                .filter(|(id, _)| id.as_str() != "codex")
                .map(|(id, bucket)| (id.as_str(), bucket)),
        );
    }
    if buckets.is_empty() {
        if let Some(legacy) = result.get("rateLimits").filter(|value| !value.is_null()) {
            let id = legacy
                .get("limitId")
                .and_then(Value::as_str)
                .unwrap_or("codex");
            buckets.push((id, legacy));
        }
    }

    let mut windows = Vec::new();
    for (id, bucket) in buckets {
        if !bucket.is_object() {
            return Err(INVALID_RESPONSE.into());
        }
        if id == "codex" {
            if let Some(plan) = bucket.get("planType").and_then(Value::as_str) {
                usage.plan = plan_label(Some(plan));
            }
        }
        let prefix = if id == "codex" {
            String::new()
        } else {
            let name = bucket
                .get("limitName")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(id);
            format!("{} · ", name.chars().take(80).collect::<String>())
        };
        for (key, fallback) in [("primary", "主要额度"), ("secondary", "次要额度")] {
            if let Some(window) = bucket.get(key).filter(|value| !value.is_null()) {
                windows.push(parse_window(window, &prefix, fallback)?);
            }
        }
        if let Some(window) = bucket.get("individualLimit").and_then(http::monthly_window) {
            windows.push(window);
        }
    }
    if windows.is_empty() {
        usage.status = ProviderStatus::Unavailable;
        usage.message = Some("已读取本机 Codex 账号，服务未返回可展示的额度窗口".into());
        return Ok(());
    }
    usage.windows = windows;
    usage.updated_at = Some(now.to_rfc3339_opts(SecondsFormat::Secs, true));
    Ok(())
}

fn parse_window(value: &Value, prefix: &str, fallback: &str) -> Result<UsageWindow, String> {
    let used = value
        .get("usedPercent")
        .and_then(Value::as_f64)
        .filter(|used| used.is_finite())
        .ok_or_else(|| INVALID_RESPONSE.to_string())?;
    let duration = value.get("windowDurationMins").and_then(Value::as_i64);
    let label = match duration {
        Some(10_080) => "每周用量".into(),
        Some(minutes) if minutes > 0 && minutes % 60 == 0 => {
            format!("{} 小时用量", minutes / 60)
        }
        Some(minutes) if minutes > 0 => format!("{minutes} 分钟用量"),
        _ => fallback.into(),
    };
    let resets_at = value
        .get("resetsAt")
        .and_then(Value::as_i64)
        .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
        .map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, true));
    Ok(UsageWindow {
        label: format!("{prefix}{label}"),
        used_percent: used.clamp(0.0, 100.0),
        resets_at,
    })
}

fn plan_label(plan: Option<&str>) -> String {
    match plan {
        Some("guest") => "Guest",
        Some("free") => "Free",
        Some("free_workspace") => "Free Workspace",
        Some("go") => "Go",
        Some("plus") => "Plus",
        Some("pro") => "Pro",
        Some("prolite") => "Pro Lite",
        Some("team") => "Team",
        Some("business" | "self_serve_business_prolite" | "self_serve_business_usage_based") => {
            "Business"
        }
        Some(
            "enterprise" | "ent26" | "enterprise_cbp_automation" | "enterprise_cbp_usage_based",
        ) => "Enterprise",
        Some("edu" | "education" | "edu_plus" | "edu_pro") => "Edu",
        Some("quorum") => "Quorum",
        Some("k12") => "K12",
        _ => "ChatGPT",
    }
    .into()
}

fn find_codex() -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
        PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex"),
    ];
    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.extend([
            home.join("Applications/Codex.app/Contents/Resources/codex"),
            home.join("Applications/ChatGPT.app/Contents/Resources/codex"),
            home.join(".local/bin/codex"),
        ]);
    }
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|directory| directory.join("codex")));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
    ]);
    candidates.into_iter().find(|candidate| candidate.is_file())
}

struct AppServer {
    child: Child,
    input: ChildStdin,
    messages: Receiver<Result<Value, &'static str>>,
    deadline: Instant,
}

impl AppServer {
    #[cfg(test)]
    fn start(executable: PathBuf, timeout: Duration) -> Result<Self, String> {
        Self::start_scoped(executable, timeout, None)
    }

    fn start_scoped(
        executable: PathBuf,
        timeout: Duration,
        home: Option<&Path>,
    ) -> Result<Self, String> {
        let mut command = Command::new(executable);
        command
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Do not forward Codex logs or upstream error bodies into application logs.
            .stderr(Stdio::null());
        if let Some(home) = home {
            command.env("CODEX_HOME", home);
        }
        // If resolving an absolute home failed (e.g. deleted launch directory),
        // keep the inherited directory rather than changing relative auth scope.
        if home.is_none_or(Path::is_absolute) {
            if let Some(home) = env::var_os("HOME") {
                command.current_dir(home);
            }
        }
        let mut child = command
            .spawn()
            .map_err(|_| "无法启动本机 Codex，请检查安装是否完整".to_string())?;
        let Some(input) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("无法连接 Codex 账号读取服务".into());
        };
        let Some(output) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("无法连接 Codex 账号读取服务".into());
        };
        let (sender, messages) = mpsc::sync_channel(16);
        thread::spawn(move || {
            let reader = BufReader::new(output.take(MAX_RESPONSE_BYTES));
            for line in reader.lines() {
                let message = line
                    .map_err(|_| "Codex 账号读取服务已断开")
                    .and_then(|line| serde_json::from_str(&line).map_err(|_| INVALID_RESPONSE));
                let failed = message.is_err();
                if sender.send(message).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            messages,
            deadline: Instant::now() + timeout,
        })
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.input, message)
            .map_err(|_| "无法向 Codex 发送读取请求".to_string())?;
        self.input
            .write_all(b"\n")
            .and_then(|_| self.input.flush())
            .map_err(|_| "无法向 Codex 发送读取请求".to_string())
    }

    fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Option<Value>,
        error_message: &'static str,
    ) -> Result<Value, String> {
        let mut request = json!({ "id": id, "method": method });
        if let Some(params) = params {
            request["params"] = params;
        }
        self.send(&request)?;
        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("读取 Codex 账号或用量超时，请稍后重试".into());
            }
            let message = self
                .messages
                .recv_timeout(remaining)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => "读取 Codex 账号或用量超时，请稍后重试",
                    mpsc::RecvTimeoutError::Disconnected => {
                        "Codex 账号读取服务已退出，请更新 Codex 后重试"
                    }
                })?
                .map_err(str::to_string)?;
            // Notifications and server requests are not responses to our read request.
            if message.get("method").is_some()
                || message.get("id").and_then(Value::as_u64) != Some(id)
            {
                continue;
            }
            if message.get("error").is_some() {
                return Err(error_message.into());
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| INVALID_RESPONSE.into());
        }
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        // Also runs on timeout and parse errors; do not leave a background server per refresh.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::UsageProvider;
    use std::cell::RefCell;

    fn account() -> Value {
        json!({ "account": { "type": "chatgpt", "email": "user@example.com", "planType": "pro" } })
    }

    #[test]
    fn missing_account_and_api_keys_do_not_fabricate_usage() {
        for response in [
            json!({"account": null}),
            json!({"account": {"type": "apiKey"}}),
        ] {
            let usage = parse_account(&response).unwrap();
            assert_eq!(usage.status, ProviderStatus::Unavailable);
            assert!(usage.windows.is_empty());
            assert!(usage.updated_at.is_none());
        }
    }

    fn synthetic_credentials(pat: bool, expired: bool) -> auth::Credentials {
        auth::parse(&serde_json::to_vec(&json!({
            "personal_access_token": pat.then_some("synthetic-pat"),
            "tokens": {"access_token":"synthetic-oauth","account_id":"synthetic-account"},
            "last_refresh": if expired { "2000-01-01T00:00:00Z".to_owned() } else { Utc::now().to_rfc3339() }
        })).unwrap()).unwrap()
    }

    #[test]
    fn auto_strategy_prefers_pat_then_oauth_and_repairs_only_unauthorized_with_cli() {
        let calls = RefCell::new(Vec::new());
        let usage = fetch_with_strategies(
            Ok(synthetic_credentials(true, false)),
            Utc::now(),
            |credential| match credential {
                http::Credential::Pat(_) => {
                    calls.borrow_mut().push("pat");
                    Err(http::FetchError::Unauthorized)
                }
                http::Credential::OAuth(_) => {
                    calls.borrow_mut().push("oauth");
                    parse_account(&account()).map_err(|_| http::FetchError::InvalidResponse)
                }
            },
            || {
                calls.borrow_mut().push("cli");
                Ok(unavailable("cli"))
            },
        )
        .unwrap();
        assert_eq!(usage.status, ProviderStatus::Ready);
        assert_eq!(*calls.borrow(), vec!["pat", "oauth"]);
        calls.borrow_mut().clear();
        fetch_with_strategies(
            Ok(synthetic_credentials(true, false)),
            Utc::now(),
            |credential| {
                calls.borrow_mut().push(match credential {
                    http::Credential::Pat(_) => "pat",
                    http::Credential::OAuth(_) => "oauth",
                });
                Err(http::FetchError::Unauthorized)
            },
            || {
                calls.borrow_mut().push("cli");
                Ok(unavailable("cli"))
            },
        )
        .unwrap();
        assert_eq!(*calls.borrow(), vec!["pat", "oauth", "cli"]);
        calls.borrow_mut().clear();
        fetch_with_strategies(
            Ok(synthetic_credentials(true, false)),
            Utc::now(),
            |_| {
                calls.borrow_mut().push("pat");
                Ok(unavailable("direct"))
            },
            || panic!("successful PAT must never launch CLI"),
        )
        .unwrap();
        assert_eq!(*calls.borrow(), vec!["pat"]);
    }

    #[test]
    fn network_server_and_decode_failures_never_launch_cli_or_switch_identity() {
        for pat in [true, false] {
            for error in [
                http::FetchError::Network,
                http::FetchError::InvalidResponse,
                http::FetchError::Server(429),
                http::FetchError::Server(403),
                http::FetchError::Server(500),
            ] {
                let mut calls = 0;
                let actual = fetch_with_strategies(
                    Ok(synthetic_credentials(pat, false)),
                    Utc::now(),
                    |_| {
                        calls += 1;
                        Err(error)
                    },
                    || panic!("transient failure must not launch CLI"),
                )
                .unwrap_err();
                assert_eq!(calls, 1);
                assert_eq!(actual, error.message());
            }
        }
    }

    #[test]
    fn expired_or_missing_native_credentials_delegate_refresh_to_cli_and_invalid_json_stops() {
        for credentials in [
            Ok(synthetic_credentials(false, true)),
            Err(auth::CredentialError::Missing),
            Ok(auth::Credentials::default()),
        ] {
            let usage = fetch_with_strategies(
                credentials,
                Utc::now(),
                |_| panic!("credentials cannot be used directly"),
                || Ok(unavailable("native-owner")),
            )
            .unwrap();
            assert_eq!(usage.message.as_deref(), Some("native-owner"));
        }
        let error = fetch_with_strategies(
            Err(auth::CredentialError::Invalid),
            Utc::now(),
            |_| panic!("invalid JSON must stop"),
            || panic!("invalid JSON must not launch CLI"),
        )
        .unwrap_err();
        assert!(error.contains("格式"));
        let error = fetch_with_strategies(
            Err(auth::CredentialError::Unreadable),
            Utc::now(),
            |_| unreachable!(),
            || Ok(unavailable("no-account")),
        )
        .unwrap_err();
        assert!(error.contains("权限"));
        let api_key = auth::parse(br#"{"OPENAI_API_KEY":"synthetic-api-key"}"#).unwrap();
        let usage = fetch_with_strategies(
            Ok(api_key),
            Utc::now(),
            |_| panic!("API keys are not subscription credentials"),
            || panic!("API key mode is already known"),
        )
        .unwrap();
        assert_eq!(usage.status, ProviderStatus::Unavailable);
    }

    #[test]
    fn native_pat_account_is_reported_as_chatgpt_with_nullable_email() {
        // app-server Account (v2) normalizes PAT and OAuth to `chatgpt`;
        // personalAccessToken/chatgptAuthTokens are auth/login modes, not Account variants.
        let usage = parse_account(
            &json!({"account":{"type":"chatgpt","email":null,"planType":"business"}}),
        )
        .unwrap();
        assert_eq!(usage.status, ProviderStatus::Ready);
        assert_eq!(usage.plan, "Business");
        assert!(usage.account.is_none());
    }

    #[test]
    fn prefers_all_current_buckets_and_actual_window_durations() {
        let mut usage = parse_account(&account()).unwrap();
        let limits = json!({
            "rateLimits": { "primary": { "usedPercent": 99, "windowDurationMins": 300 } },
            "rateLimitsByLimitId": {
                "codex": { "primary": { "usedPercent": 38, "windowDurationMins": 10080, "resetsAt": 1789805400 } },
                "spark": { "limitName": "Spark", "primary": { "usedPercent": 0, "windowDurationMins": 300, "resetsAt": null } }
            }
        });
        apply_limits(&mut usage, &limits, Utc::now()).unwrap();
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].label, "每周用量");
        assert_eq!(usage.windows[0].used_percent, 38.0);
        assert_eq!(
            usage.windows[0].resets_at.as_deref(),
            Some("2026-09-19T08:10:00Z")
        );
        assert_eq!(usage.windows[1].label, "Spark · 5 小时用量");
        assert_eq!(usage.windows[1].used_percent, 0.0);
        assert!(usage.windows[1].resets_at.is_none());
    }

    #[test]
    fn accepts_legacy_limits_without_inventing_missing_percent_or_reset() {
        let mut usage = parse_account(&account()).unwrap();
        apply_limits(
            &mut usage,
            &json!({ "rateLimits": { "primary": { "usedPercent": 25 } } }),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(usage.windows[0].label, "主要额度");
        assert!(usage.windows[0].resets_at.is_none());
        assert!(parse_window(&json!({"resetsAt": 123}), "", "主要额度").is_err());
        assert!(parse_window(&json!({"usedPercent": null}), "", "主要额度").is_err());
    }

    #[test]
    fn absent_limits_are_unavailable_and_do_not_become_zero_usage() {
        let mut usage = parse_account(&account()).unwrap();
        apply_limits(&mut usage, &json!({ "rateLimits": {} }), Utc::now()).unwrap();
        assert_eq!(usage.status, ProviderStatus::Unavailable);
        assert!(usage.windows.is_empty());
        assert!(usage.updated_at.is_none());
    }

    #[test]
    fn cli_monthly_limits_preserve_service_reported_percentage_and_reset() {
        let mut usage = parse_account(&account()).unwrap();
        apply_limits(&mut usage,&json!({"rateLimits":{"individualLimit":{"limit":100,"used":15,"remainingPercent":85,"resetsAt":1789805400}}}),Utc::now()).unwrap();
        assert_eq!(usage.status, ProviderStatus::Ready);
        assert_eq!(usage.windows[0].label, "每月额度");
        assert_eq!(usage.windows[0].used_percent, 15.0);
    }

    #[cfg(unix)]
    fn fake_server(contents: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codex");
        std::fs::write(&path, format!("#!/bin/sh\n{contents}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (directory, path)
    }

    #[test]
    #[cfg(unix)]
    fn ignores_server_requests_and_redacts_upstream_errors() {
        let (_directory, path) = fake_server(
            "read -r request\nprintf '%s\\n' '{\"method\":\"account/chatgptAuthTokens/refresh\",\"id\":1}' '{\"id\":1,\"error\":{\"message\":\"secret error body\"}}'",
        );
        let mut server = AppServer::start(path, Duration::from_secs(2)).unwrap();
        let error = server
            .request(1, "account/read", None, "读取失败")
            .unwrap_err();
        assert_eq!(error, "读取失败");
        assert!(!error.contains("secret"));
    }

    #[test]
    #[cfg(unix)]
    fn fallback_cli_receives_the_same_explicit_codex_home() {
        let (_directory, path) = fake_server(
            "read -r request\nprintf '{\"id\":1,\"result\":{\"home\":\"%s\"}}\\n' \"$CODEX_HOME\"",
        );
        let mut server = AppServer::start_scoped(
            path,
            Duration::from_secs(2),
            Some(Path::new("/synthetic/scoped-home")),
        )
        .unwrap();
        let response = server.request(1, "account/read", None, "读取失败").unwrap();
        assert_eq!(response["home"], "/synthetic/scoped-home");
    }

    #[test]
    #[cfg(unix)]
    fn timeout_kills_and_reaps_the_owned_server() {
        let (_directory, path) = fake_server("exec /bin/sleep 10");
        let mut server = AppServer::start(path, Duration::from_millis(50)).unwrap();
        let pid = server.child.id();
        let started = Instant::now();
        let error = server
            .request(1, "initialize", None, "初始化失败")
            .unwrap_err();
        assert!(error.contains("超时"));
        drop(server);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success());
    }

    #[test]
    #[ignore = "Reads the current local Codex account and live subscription limits"]
    fn live_codex_account_and_usage() {
        let usage = CodexProvider.fetch_usage(Utc::now()).unwrap();
        let account = usage.account.as_deref().and_then(|email| {
            email
                .split_once('@')
                .map(|(name, domain)| format!("{}***@{domain}", name.chars().next().unwrap_or('*')))
        });
        println!(
            "Codex status={:?}, plan={}, account={account:?}, windows={:?}",
            usage.status, usage.plan, usage.windows
        );
        assert_eq!(
            usage.status,
            ProviderStatus::Ready,
            "Codex live usage unavailable"
        );
        assert!(!usage.windows.is_empty());
    }
}
