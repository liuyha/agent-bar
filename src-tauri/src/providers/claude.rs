//! Read-only integration with Claude Code's local subscription login.
//!
//! Credential storage is documented at https://code.claude.com/docs/en/authentication.
//! The usage/profile endpoints are internal Claude Code APIs, so an unrecognized
//! response is reported as unavailable instead of being interpreted as zero usage.

use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{blocking::Client, redirect::Policy, StatusCode};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::models::{DataMode, ProviderId, ProviderStatus, ProviderUsage, UsageWindow};

const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const MAX_JSON_BYTES: u64 = 1024 * 1024;

pub struct ClaudeProvider;

// Deliberately does not implement Debug: this type contains a bearer credential.
struct Credential {
    token: String,
    plan: String,
    expires_at: Option<i64>,
    profile_scope: bool,
}

impl super::UsageProvider for ClaudeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    fn fetch_usage(&self, now: DateTime<Utc>) -> Result<ProviderUsage, String> {
        let mut usage = empty_usage();
        match fetch(&mut usage, now) {
            Ok(()) => {}
            Err(Failure::Unavailable(message)) => {
                usage.status = ProviderStatus::Unavailable;
                usage.message = Some(message);
            }
            Err(Failure::Error(message)) => {
                usage.status = ProviderStatus::Error;
                usage.message = Some(message);
            }
        }
        Ok(usage)
    }
}

enum Failure {
    Unavailable(String),
    Error(String),
}

fn empty_usage() -> ProviderUsage {
    ProviderUsage {
        id: ProviderId::Claude,
        name: "Claude".into(),
        plan: "订阅未确认".into(),
        source: DataMode::Local,
        status: ProviderStatus::Unavailable,
        account: None,
        message: None,
        windows: Vec::new(),
        updated_at: None,
        cache_scope: None,
    }
}

fn fetch(usage: &mut ProviderUsage, now: DateTime<Utc>) -> Result<(), Failure> {
    let (config_dir, custom_config_dir) = config_dir()?;
    let settings = read_json_file(&config_dir.join("settings.json"))?;
    if let Some(message) = unsupported_auth(settings.as_ref(), |key| env::var(key).ok()) {
        return Err(Failure::Unavailable(message.into()));
    }

    let stored = read_credentials(&config_dir, custom_config_dir)?.ok_or_else(|| {
        Failure::Unavailable("未检测到 Claude Code 订阅登录，请先在 Claude Code 中登录。".into())
    })?;
    let credential = parse_credential(&stored)?.ok_or_else(|| {
        Failure::Unavailable("本机未保存 Claude 订阅登录；API Key 账号没有订阅额度数据。".into())
    })?;
    usage.plan = credential.plan.clone();
    if !credential.profile_scope {
        return Err(Failure::Unavailable(
            "当前 Claude 登录缺少账号与用量读取权限，请在 Claude Code 中重新登录。".into(),
        ));
    }
    if credential
        .expires_at
        .is_some_and(|expiry| expiry <= now.timestamp_millis())
    {
        return Err(Failure::Unavailable(
            "Claude 登录已过期，请打开 Claude Code 刷新登录后重试。".into(),
        ));
    }

    // Compare the selected login even when the profile request itself cannot connect.
    // Only a one-way digest is retained; bearer credentials never enter the snapshot.
    let mut digest = Sha256::new();
    digest.update(config_dir.as_os_str().as_encoded_bytes());
    digest.update([0]);
    digest.update(credential.token.as_bytes());
    usage.cache_scope = Some(format!("{:x}", digest.finalize()));

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .redirect(Policy::none())
        .https_only(true)
        .user_agent("AgentBar/0.1")
        .build()
        .map_err(|_| Failure::Error("无法初始化 Claude 用量连接。".into()))?;

    let profile = fetch_json(&client, PROFILE_URL, &credential.token)?;
    usage.account = profile_account(&profile);
    if let Some(plan) = profile
        .pointer("/organization/organization_type")
        .and_then(Value::as_str)
    {
        usage.plan = plan_name(plan).into();
    }
    let response = fetch_json(&client, USAGE_URL, &credential.token)?;
    usage.windows = parse_windows(&response)?;
    if usage.windows.is_empty() {
        return Err(Failure::Unavailable(
            "Claude 已登录，但服务未返回可显示的订阅额度。".into(),
        ));
    }
    usage.status = ProviderStatus::Ready;
    usage.updated_at = Some(now.to_rfc3339_opts(SecondsFormat::Secs, true));
    Ok(())
}

fn config_dir() -> Result<(PathBuf, bool), Failure> {
    if let Some(directory) = env::var_os("CLAUDE_CONFIG_DIR").filter(|value| !value.is_empty()) {
        return Ok((PathBuf::from(directory), true));
    }
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or_else(|| Failure::Error("无法定位本机 Claude 配置目录。".into()))?;
    Ok((PathBuf::from(home).join(".claude"), false))
}

fn unsupported_auth(
    settings: Option<&Value>,
    environment: impl Fn(&str) -> Option<String>,
) -> Option<&'static str> {
    let configured = |key: &str| {
        environment(key)
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                settings?
                    .get("env")?
                    .get(key)?
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_owned)
            })
    };
    if [
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
    ]
    .iter()
    .any(|key| configured(key).is_some_and(|value| matches!(value.as_str(), "1" | "true")))
    {
        return Some("本机 Claude 使用云服务商认证，无法读取 Claude 订阅额度。");
    }
    if configured("ANTHROPIC_BASE_URL")
        .is_some_and(|url| url.trim_end_matches('/') != "https://api.anthropic.com")
        || configured("CLAUDE_CODE_CUSTOM_OAUTH_URL").is_some()
        || settings
            .and_then(|value| value.get("forceLoginMethod"))
            .and_then(Value::as_str)
            == Some("gateway")
        || settings
            .and_then(|value| value.get("forceLoginGatewayUrl"))
            .is_some_and(|value| !value.is_null())
    {
        return Some("本机 Claude 配置了第三方代理或网关，无法读取官方订阅额度。");
    }
    if [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_PROFILE",
    ]
    .iter()
    .any(|key| configured(key).is_some())
        || settings
            .and_then(|value| value.get("apiKeyHelper"))
            .is_some_and(|value| !value.is_null())
        || (configured("ANTHROPIC_FEDERATION_RULE_ID").is_some()
            && configured("ANTHROPIC_ORGANIZATION_ID").is_some())
    {
        return Some("本机 Claude 使用 API Key、外部认证或 Console 配置，无法读取订阅额度。");
    }
    if configured("CLAUDE_CODE_OAUTH_TOKEN").is_some() {
        return Some("本机 Claude 使用环境变量令牌，通常仅有推理权限；请使用 Claude Code 的订阅登录读取用量。");
    }
    None
}

fn read_json_file(path: &Path) -> Result<Option<Value>, Failure> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(Failure::Error(
                "无法读取本机 Claude 配置，请检查文件权限。".into(),
            ))
        }
    };
    let mut bytes = Vec::new();
    file.take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Error("无法读取本机 Claude 配置。".into()))?;
    parse_stored_json(&bytes).map(Some)
}

fn parse_stored_json(bytes: &[u8]) -> Result<Value, Failure> {
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err(Failure::Error(
            "本机 Claude 配置文件超出支持的大小。".into(),
        ));
    }
    serde_json::from_slice(bytes).map_err(|_| {
        Failure::Error("本机 Claude 配置格式无效，请在 Claude Code 中检查登录状态。".into())
    })
}

fn read_credentials(config_dir: &Path, custom_config_dir: bool) -> Result<Option<Value>, Failure> {
    #[cfg(target_os = "macos")]
    {
        // Claude Code prefers Keychain, then falls back to its credentials file.
        let keychain = read_keychain(config_dir, custom_config_dir);
        match keychain {
            Ok(Some(value)) => return Ok(Some(value)),
            Ok(None) => {}
            Err(error) => {
                return read_json_file(&config_dir.join(".credentials.json"))?
                    .map(Some)
                    .ok_or(error)
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = custom_config_dir;
    read_json_file(&config_dir.join(".credentials.json"))
}

#[cfg(target_os = "macos")]
fn read_keychain(config_dir: &Path, custom_config_dir: bool) -> Result<Option<Value>, Failure> {
    use sha2::{Digest, Sha256};
    use std::{
        process::{Command, Stdio},
        thread,
        time::Instant,
    };

    let service = if custom_config_dir {
        let digest = format!(
            "{:x}",
            Sha256::digest(config_dir.to_string_lossy().as_bytes())
        );
        format!("Claude Code-credentials-{}", &digest[..8])
    } else {
        "Claude Code-credentials".into()
    };
    let mut command = Command::new("/usr/bin/security");
    command.args(["find-generic-password", "-s", &service, "-w"]);
    if let Ok(user) = env::var("USER") {
        if !user.is_empty() {
            command.args(["-a", &user]);
        }
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Failure::Error("无法读取 macOS 钥匙串中的 Claude 登录。".into()))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.code() == Some(44) {
                    return Ok(None);
                }
                if !status.success() {
                    return Err(Failure::Error(
                        "无法访问 Claude 登录钥匙串，请解锁后重试。".into(),
                    ));
                }
                let output = child
                    .wait_with_output()
                    .map_err(|_| Failure::Error("无法读取 Claude 登录钥匙串。".into()))?;
                return parse_stored_json(&output.stdout).map(Some);
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Error(
                    "读取 Claude 登录钥匙串超时，请解锁后重试。".into(),
                ));
            }
        }
    }
}

fn parse_credential(value: &Value) -> Result<Option<Credential>, Failure> {
    let Some(oauth) = value.get("claudeAiOauth").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            Failure::Unavailable("本机 Claude 订阅登录不完整，请在 Claude Code 中重新登录。".into())
        })?;
    let scopes = oauth.get("scopes").and_then(Value::as_array);
    Ok(Some(Credential {
        token: token.into(),
        plan: plan_name(
            oauth
                .get("subscriptionType")
                .and_then(Value::as_str)
                .unwrap_or(""),
        )
        .into(),
        expires_at: oauth.get("expiresAt").and_then(Value::as_i64),
        profile_scope: scopes.is_some_and(|items| {
            items
                .iter()
                .any(|value| value.as_str() == Some("user:profile"))
        }),
    }))
}

fn plan_name(value: &str) -> &'static str {
    match value {
        "pro" | "claude_pro" => "Pro",
        "max" | "claude_max" => "Max",
        "team" | "claude_team" => "Team",
        "enterprise" | "claude_enterprise" => "Enterprise",
        _ => "订阅未确认",
    }
}

fn profile_account(profile: &Value) -> Option<String> {
    ["/account/email", "/account/display_name"]
        .iter()
        .filter_map(|pointer| profile.pointer(pointer).and_then(Value::as_str))
        .find(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn fetch_json(client: &Client, url: &'static str, token: &str) -> Result<Value, Failure> {
    // Never use configured endpoint overrides, follow redirects, or expose error bodies.
    let response = client
        .get(url)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .send()
        .map_err(|error| {
            Failure::Error(
                if error.is_timeout() {
                    "读取 Claude 账号用量超时。"
                } else {
                    "无法连接 Claude 官方用量服务。"
                }
                .into(),
            )
        })?;
    match response.status() {
        StatusCode::UNAUTHORIZED => {
            return Err(Failure::Unavailable(
                "Claude 登录已失效，请在 Claude Code 中重新登录。".into(),
            ))
        }
        StatusCode::FORBIDDEN => {
            return Err(Failure::Unavailable(
                "当前 Claude 登录没有读取订阅用量的权限。".into(),
            ))
        }
        StatusCode::TOO_MANY_REQUESTS => {
            return Err(Failure::Error(
                "Claude 用量查询暂时受到频率限制，请稍后刷新。".into(),
            ))
        }
        status if !status.is_success() => {
            return Err(Failure::Error(format!(
                "Claude 官方用量服务暂不可用（HTTP {}）。",
                status.as_u16()
            )))
        }
        _ => {}
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Error("无法读取 Claude 用量响应。".into()))?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err(Failure::Error("Claude 用量响应超出支持的大小。".into()));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| Failure::Error("Claude 返回了无法识别的用量响应。".into()))
}

fn parse_windows(value: &Value) -> Result<Vec<UsageWindow>, Failure> {
    // The current official CLI (2.1.273) consumes server-defined limits[] rows.
    // An explicit empty array is authoritative; do not resurrect legacy meters.
    if let Some(limits) = value.get("limits").filter(|value| !value.is_null()) {
        let limits = limits
            .as_array()
            .ok_or_else(|| Failure::Error("Claude 返回了无法识别的用量列表。".into()))?;
        return limits
            .iter()
            .map(|window| {
                let kind = window.get("kind").and_then(Value::as_str).unwrap_or("");
                let label = match kind {
                    "session" => "5 小时用量",
                    "weekly_all" => "每周用量",
                    "weekly_scoped" => "每周用量",
                    _ => "订阅用量",
                };
                let scope = ["/scope/model/display_name", "/scope/surface/display_name"]
                    .iter()
                    .filter_map(|pointer| window.pointer(pointer).and_then(Value::as_str))
                    .find(|name| !name.trim().is_empty());
                let label = scope.map_or_else(
                    || label.to_owned(),
                    |name| format!("{} · {label}", name.chars().take(80).collect::<String>()),
                );
                parse_window(window, "percent", &label)
            })
            .collect();
    }
    let mut windows = Vec::new();
    for (key, label) in [
        ("five_hour", "5 小时用量"),
        ("seven_day", "每周用量"),
        ("seven_day_sonnet", "Sonnet 每周用量"),
        ("seven_day_opus", "Opus 每周用量"),
    ] {
        let Some(window) = value.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        if window.get("utilization").is_none_or(Value::is_null) {
            continue;
        }
        windows.push(parse_window(window, "utilization", label)?);
    }
    Ok(windows)
}

fn parse_window(window: &Value, percent_key: &str, label: &str) -> Result<UsageWindow, Failure> {
    let used_percent = window
        .get(percent_key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| Failure::Error("Claude 返回了无效的用量百分比。".into()))?;
    let resets_at = match window.get("resets_at") {
        None | Some(Value::Null) => None,
        Some(Value::String(reset)) => Some(
            DateTime::parse_from_rfc3339(reset)
                .map_err(|_| Failure::Error("Claude 返回了无效的重置时间。".into()))?
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        ),
        Some(_) => return Err(Failure::Error("Claude 返回了无效的重置时间。".into())),
    };
    Ok(UsageWindow {
        label: label.into(),
        used_percent,
        resets_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn credentials_only_accept_subscription_oauth_and_require_profile_scope() {
        assert!(matches!(
            parse_credential(&json!({"apiKey":"synthetic-key"})),
            Ok(None)
        ));
        let value = json!({"claudeAiOauth": {
            "accessToken":"synthetic-token", "expiresAt":1800000000000_i64,
            "subscriptionType":"max", "scopes":["user:profile", "user:inference"]
        }});
        let credential = parse_credential(&value).ok().flatten().unwrap();
        assert_eq!(credential.plan, "Max");
        assert!(credential.profile_scope);
        assert_eq!(credential.expires_at, Some(1800000000000));
        let missing_scope =
            json!({"claudeAiOauth":{"accessToken":"synthetic-token","scopes":["user:inference"]}});
        assert!(
            !parse_credential(&missing_scope)
                .ok()
                .flatten()
                .unwrap()
                .profile_scope
        );
    }

    #[test]
    fn subscription_windows_preserve_real_values_and_missing_resets() {
        let windows = parse_windows(&json!({
            "five_hour":{"utilization":12.5,"resets_at":"2026-09-16T18:00:00+08:00"},
            "seven_day":{"utilization":104.0,"resets_at":null},
            "seven_day_sonnet":null
        }))
        .ok()
        .unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].used_percent, 12.5);
        assert_eq!(
            windows[0].resets_at.as_deref(),
            Some("2026-09-16T10:00:00Z")
        );
        assert_eq!(windows[1].used_percent, 104.0);
        assert_eq!(windows[1].resets_at, None);
    }

    #[test]
    fn current_server_rows_take_precedence_and_preserve_scope_and_order() {
        let windows = parse_windows(&json!({
            "limits":[
                {"kind":"session","group":"session","percent":4.0,"resets_at":null},
                {"kind":"weekly_scoped","group":"weekly","percent":16.0,"resets_at":null,"scope":{"model":{"display_name":"Fable"}}}
            ],
            "five_hour":{"utilization":99.0}
        })).ok().unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].used_percent, 4.0);
        assert_eq!(windows[1].label, "Fable · 每周用量");
        assert!(
            parse_windows(&json!({"limits":[],"five_hour":{"utilization":99.0}}))
                .ok()
                .unwrap()
                .is_empty()
        );
        assert!(parse_windows(&json!({"limits":[{"kind":"session","percent":null}]})).is_err());
    }

    #[test]
    fn absent_or_changed_usage_schema_never_means_zero() {
        assert!(
            parse_windows(&json!({"five_hour":null,"seven_day":{"utilization":null}}))
                .ok()
                .unwrap()
                .is_empty()
        );
        assert!(parse_windows(&json!({"different_schema":{}}))
            .ok()
            .unwrap()
            .is_empty());
        assert!(parse_windows(&json!({"five_hour":{"utilization":"12"}})).is_err());
        assert!(parse_windows(&json!({"five_hour":{"utilization":-1}})).is_err());
        assert!(
            parse_windows(&json!({"five_hour":{"utilization":12,"resets_at":"invalid"}})).is_err()
        );
    }

    #[test]
    fn custom_auth_does_not_contact_subscription_service() {
        for settings in [
            json!({"env":{"ANTHROPIC_API_KEY":"synthetic-key"}}),
            json!({"env":{"ANTHROPIC_BASE_URL":"https://proxy.invalid"}}),
            json!({"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}),
            json!({"apiKeyHelper":"must never execute"}),
        ] {
            assert!(unsupported_auth(Some(&settings), |_| None).is_some());
        }
        assert!(unsupported_auth(
            Some(&json!({"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com/"}})),
            |_| None
        )
        .is_none());
        assert!(unsupported_auth(None, |key| (key == "ANTHROPIC_AUTH_TOKEN")
            .then(|| "synthetic-token".into()))
        .is_some());
    }

    #[test]
    fn profile_uses_only_allowlisted_identity_fields() {
        assert_eq!(
            profile_account(
                &json!({"account":{"email":"person@example.test","accessToken":"never expose"}})
            )
            .as_deref(),
            Some("person@example.test")
        );
        assert_eq!(
            profile_account(&json!({"accessToken":"never expose"})),
            None
        );
    }

    #[test]
    #[ignore = "Reads this machine's Claude login and calls the official account/usage endpoints"]
    fn local_claude_read_only_smoke() {
        use super::super::UsageProvider;
        let usage = ClaudeProvider.fetch_usage(Utc::now()).unwrap();
        println!(
            "Claude status={:?}; account_present={}; windows={}; message={}",
            usage.status,
            usage.account.is_some(),
            usage.windows.len(),
            usage.message.as_deref().unwrap_or("")
        );
        assert_eq!(usage.source, DataMode::Local);
        if usage.status == ProviderStatus::Ready {
            assert!(!usage.windows.is_empty());
            assert!(usage.updated_at.is_some());
        } else {
            assert!(usage.windows.is_empty());
        }
    }
}
