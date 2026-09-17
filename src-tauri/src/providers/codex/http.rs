//! Fixed-origin, read-only Codex PAT/OAuth usage requests. No cookies, redirects,
//! credential writes, token refreshes, or raw response/error messages.

use std::{collections::HashSet, io::Read};

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{
    blocking::Client,
    header::{HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT},
    redirect::Policy,
};
use serde_json::{json, Value};

use super::{
    auth::{nonempty, OAuthCredential},
    parse_window, plan_label, unavailable, ProviderStatus, ProviderUsage, UsageWindow,
    MAX_RESPONSE_BYTES, REQUEST_TIMEOUT,
};

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub(super) const WHOAMI_URL: &str =
    "https://auth.openai.com/api/accounts/v1/user-auth-credential/whoami";

pub(super) enum Credential<'a> {
    Pat(&'a str),
    OAuth(&'a OAuthCredential),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum FetchError {
    Unauthorized,
    InvalidResponse,
    Network,
    Server(u16),
}

impl FetchError {
    pub fn message(self) -> String {
        match self {
            Self::Unauthorized => "Codex 登录凭据已失效，请在 Codex 中重新登录".into(),
            Self::InvalidResponse => super::INVALID_RESPONSE.into(),
            Self::Network => "无法连接 Codex 用量服务，请检查网络后重试".into(),
            Self::Server(429) => "Codex 用量服务请求过于频繁，请稍后重试".into(),
            Self::Server(code) => format!("Codex 用量服务返回错误（HTTP {code}），请稍后重试"),
        }
    }
}

pub(super) fn fetch(
    credential: Credential<'_>,
    now: DateTime<Utc>,
) -> Result<ProviderUsage, FetchError> {
    let client = client()?;
    fetch_with(credential, now, |url, token, account, pat| {
        get_json(&client, url, token, account, pat)
    })
}

pub(super) fn client() -> Result<Client, FetchError> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(Policy::none())
        .build()
        .map_err(|_| FetchError::Network)
}

pub(super) fn get_json(
    client: &Client,
    url: &str,
    token: &str,
    account: Option<&str>,
    pat: bool,
) -> Result<Value, FetchError> {
    let mut bearer =
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| FetchError::Unauthorized)?;
    bearer.set_sensitive(true);
    let mut request = client
        .get(url)
        .header(AUTHORIZATION, bearer)
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, if pat { "codex_cli_rs" } else { "AgentBar" });
    if pat {
        request = request.header("originator", "codex_cli_rs");
    }
    if let Some(account) = account {
        let value = HeaderValue::from_str(account).map_err(|_| FetchError::InvalidResponse)?;
        request = request.header("ChatGPT-Account-Id", value);
    }
    let response = request.send().map_err(|_| FetchError::Network)?;
    match response.status().as_u16() {
        200..=299 => {}
        401 => return Err(FetchError::Unauthorized),
        code => return Err(FetchError::Server(code)),
    }
    let mut body = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| FetchError::Network)?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(FetchError::InvalidResponse);
    }
    serde_json::from_slice(&body).map_err(|_| FetchError::InvalidResponse)
}

fn fetch_with(
    credential: Credential<'_>,
    now: DateTime<Utc>,
    mut get: impl FnMut(&str, &str, Option<&str>, bool) -> Result<Value, FetchError>,
) -> Result<ProviderUsage, FetchError> {
    let (response, email, fallback_plan) = match credential {
        Credential::Pat(token) => {
            let whoami = get(WHOAMI_URL, token, None, true)?;
            if !whoami.is_object() {
                return Err(FetchError::InvalidResponse);
            }
            let account = nonempty(whoami.get("chatgpt_account_id"));
            let response = get(USAGE_URL, token, account.as_deref(), true)?;
            (
                response,
                nonempty(whoami.get("email")),
                nonempty(whoami.get("chatgpt_plan_type")),
            )
        }
        Credential::OAuth(oauth) => (
            get(
                USAGE_URL,
                &oauth.access_token,
                oauth.account_id.as_deref(),
                false,
            )?,
            oauth.email.clone(),
            oauth.plan.clone(),
        ),
    };
    map_usage(&response, email, fallback_plan.as_deref(), now)
}

fn map_usage(
    response: &Value,
    email: Option<String>,
    fallback_plan: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ProviderUsage, FetchError> {
    if !response.is_object() {
        return Err(FetchError::InvalidResponse);
    }
    let mut usage = unavailable("已读取本机 Codex 账号，服务未返回可展示的额度窗口");
    usage.plan = plan_label(
        response
            .get("plan_type")
            .and_then(Value::as_str)
            .or(fallback_plan),
    );
    usage.account = email;
    let mut malformed = false;
    let mut core = Vec::new();
    if let Some(limit) = response.get("rate_limit").filter(|value| !value.is_null()) {
        read_windows(limit, "", &mut core, &mut malformed);
    }
    // Upstream also gates model-specific windows on a usable core snapshot.
    // An inline monthly credit limit is independently meaningful when present.
    let monthly = response
        .get("individual_limit")
        .or_else(|| response.get("individualLimit"))
        .filter(|value| !value.is_null())
        .or_else(|| {
            response
                .get("rate_limit")
                .and_then(|value| {
                    value
                        .get("individual_limit")
                        .or_else(|| value.get("individualLimit"))
                })
                .filter(|value| !value.is_null())
        })
        .or_else(|| {
            response
                .get("spend_control")
                .or_else(|| response.get("spendControl"))
                .and_then(|value| {
                    value
                        .get("individual_limit")
                        .or_else(|| value.get("individualLimit"))
                })
                .filter(|value| !value.is_null())
        });
    if let Some(window) = monthly.and_then(monthly_window) {
        core.push(window);
    }
    if core.is_empty() {
        return if malformed {
            Err(FetchError::InvalidResponse)
        } else {
            Ok(usage)
        };
    }
    let mut seen = HashSet::new();
    if let Some(extras) = response
        .get("additional_rate_limits")
        .filter(|value| !value.is_null())
    {
        if let Some(extras) = extras.as_array() {
            for extra in extras {
                let id = nonempty(extra.get("metered_feature"))
                    .or_else(|| nonempty(extra.get("limit_name")));
                let Some(id) = id else {
                    malformed = true;
                    continue;
                };
                if !seen.insert(id) {
                    continue;
                }
                let name = nonempty(extra.get("limit_name"))
                    .or_else(|| nonempty(extra.get("metered_feature")))
                    .unwrap();
                let prefix = format!(
                    "{} · ",
                    name.chars()
                        .filter(|ch| !ch.is_control())
                        .take(80)
                        .collect::<String>()
                );
                if let Some(limit) = extra.get("rate_limit").filter(|value| !value.is_null()) {
                    read_windows(limit, &prefix, &mut core, &mut malformed);
                }
            }
        } else {
            malformed = true;
        }
    }
    usage.status = ProviderStatus::Ready;
    usage.windows = core;
    usage.message = malformed.then(|| "部分 Codex 额度窗口格式无法识别，已展示可用额度".into());
    usage.updated_at = Some(now.to_rfc3339_opts(SecondsFormat::Secs, true));
    Ok(usage)
}

fn read_windows(limit: &Value, prefix: &str, windows: &mut Vec<UsageWindow>, malformed: &mut bool) {
    if !limit.is_object() {
        *malformed = true;
        return;
    }
    for (key, fallback) in [
        ("primary_window", "主要额度"),
        ("secondary_window", "次要额度"),
    ] {
        if let Some(value) = limit.get(key).filter(|value| !value.is_null()) {
            let duration = value
                .get("limit_window_seconds")
                .and_then(Value::as_i64)
                .filter(|seconds| *seconds > 0)
                .map(|seconds| seconds / 60);
            let reset = value
                .get("reset_at")
                .and_then(Value::as_i64)
                .filter(|timestamp| *timestamp > 0);
            let normalized = json!({"usedPercent":value.get("used_percent"), "windowDurationMins":duration, "resetsAt":reset});
            match parse_window(&normalized, prefix, fallback) {
                Ok(window) => windows.push(window),
                Err(_) => *malformed = true,
            }
        }
    }
}

fn flexible_number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str()?.trim().parse().ok())
        })
        .filter(|value| value.is_finite())
}

pub(super) fn monthly_window(value: &Value) -> Option<UsageWindow> {
    let remaining = flexible_number(
        value
            .get("remaining_percent")
            .or_else(|| value.get("remainingPercent")),
    );
    let percent = remaining.map(|remaining| 100.0 - remaining).or_else(|| {
        let limit = flexible_number(value.get("limit")).filter(|limit| *limit > 0.0)?;
        let used = flexible_number(value.get("used"))?;
        Some(used / limit * 100.0)
    })?;
    let reset = value
        .get("reset_at")
        .or_else(|| value.get("resets_at"))
        .or_else(|| value.get("resetsAt"))
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .filter(|timestamp| *timestamp > 0)
        .and_then(|timestamp| DateTime::from_timestamp(timestamp, 0))
        .map(|date| date.to_rfc3339_opts(SecondsFormat::Secs, true));
    Some(UsageWindow {
        label: "每月额度".into(),
        used_percent: percent.clamp(0.0, 100.0),
        resets_at: reset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        thread,
    };

    fn response() -> Value {
        json!({"plan_type":"pro","rate_limit":{"primary_window":{"used_percent":21,"limit_window_seconds":18000,"reset_at":1789805400},"secondary_window":{"used_percent":9,"limit_window_seconds":604800}},"additional_rate_limits":[{"limit_name":"Spark","metered_feature":"spark","rate_limit":{"primary_window":{"used_percent":2,"limit_window_seconds":18000},"secondary_window":{"used_percent":5,"limit_window_seconds":604800}}}]})
    }

    #[test]
    fn pat_identity_and_scope_come_only_from_whoami() {
        let mut calls = Vec::new();
        let usage = fetch_with(Credential::Pat("synthetic-pat"), Utc::now(), |url, token, account, pat| {
            assert_eq!(token, "synthetic-pat"); assert!(pat);
            calls.push(url.to_owned());
            if url == WHOAMI_URL {
                assert!(account.is_none());
                Ok(json!({"chatgpt_account_id":"pat-account","email":"pat@example.test","chatgpt_plan_type":"plus"}))
            } else {
                assert_eq!(account, Some("pat-account")); Ok(response())
            }
        }).unwrap();
        assert_eq!(calls, vec![WHOAMI_URL, USAGE_URL]);
        assert_eq!(usage.account.as_deref(), Some("pat@example.test"));
        assert_eq!(usage.plan, "Pro");
        assert_eq!(usage.windows.len(), 4);
        assert_eq!(usage.windows[0].label, "5 小时用量");
        assert_eq!(usage.windows[1].label, "每周用量");
        assert_eq!(usage.windows[3].label, "Spark · 每周用量");
        assert_eq!(
            usage.windows[0].resets_at.as_deref(),
            Some("2026-09-19T08:10:00Z")
        );
    }

    #[test]
    fn malformed_additive_windows_preserve_valid_core_without_inventing_values() {
        let usage = map_usage(&json!({"rate_limit":{"primary_window":{"used_percent":0},"secondary_window":{"used_percent":null}},"additional_rate_limits":[{"limit_name":"broken","rate_limit":{"primary_window":{}}},{"limit_name":"valid","rate_limit":{"primary_window":{"used_percent":27}}},42]}),None,None,Utc::now()).unwrap();
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].used_percent, 0.0);
        assert!(usage.windows[0].resets_at.is_none());
        assert!(usage.message.is_some());
        let extras_only = map_usage(
            &json!({"additional_rate_limits":response()["additional_rate_limits"]}),
            None,
            None,
            Utc::now(),
        )
        .unwrap();
        assert_eq!(extras_only.status, ProviderStatus::Unavailable);
        assert!(extras_only.windows.is_empty());
        assert_eq!(
            map_usage(
                &json!({"rate_limit":{"primary_window":{}}}),
                None,
                None,
                Utc::now()
            )
            .err(),
            Some(FetchError::InvalidResponse)
        );
    }

    #[test]
    fn inline_monthly_limits_use_reported_percent_and_reset_without_assuming_zero() {
        let usage = map_usage(&json!({"spend_control":{"individual_limit":{"remaining_percent":"75","reset_at":1789805400}}}),None,None,Utc::now()).unwrap();
        assert_eq!(usage.windows[0].label, "每月额度");
        assert_eq!(usage.windows[0].used_percent, 25.0);
        assert_eq!(
            usage.windows[0].resets_at.as_deref(),
            Some("2026-09-19T08:10:00Z")
        );
        assert!(monthly_window(&json!({"limit":100})).is_none());
        assert_eq!(
            monthly_window(&json!({"limit":"100","used":"20"}))
                .unwrap()
                .used_percent,
            20.0
        );
    }

    fn mock_server(
        status: u16,
        body: &'static str,
        location: Option<&str>,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let location = location.map(str::to_owned);
        let handle = thread::spawn(move || {
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
            let extra = location
                .map(|url| format!("Location: {url}\r\n"))
                .unwrap_or_default();
            write!(socket,"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",body.len()).unwrap();
            request
        });
        (format!("http://{address}/usage"), handle)
    }

    #[test]
    fn local_http_mock_verifies_bearer_identity_headers_and_redacted_errors() {
        let client = Client::builder().redirect(Policy::none()).build().unwrap();
        let (url, server) = mock_server(200, "{}", None);
        assert_eq!(
            get_json(
                &client,
                &url,
                "synthetic-secret",
                Some("account-test"),
                true
            )
            .unwrap(),
            json!({})
        );
        let request = server.join().unwrap().to_lowercase();
        assert!(request.contains("authorization: bearer synthetic-secret"));
        assert!(request.contains("chatgpt-account-id: account-test"));
        assert!(request.contains("originator: codex_cli_rs"));
        for (status, error) in [
            (401, FetchError::Unauthorized),
            (403, FetchError::Server(403)),
            (429, FetchError::Server(429)),
            (500, FetchError::Server(500)),
        ] {
            let (url, server) = mock_server(status, "secret-error-body", None);
            let actual = get_json(&client, &url, "synthetic-secret", None, false).unwrap_err();
            assert_eq!(actual, error);
            assert!(!actual.message().contains("secret"));
            server.join().unwrap();
        }
        let (url, server) = mock_server(200, "secret-invalid-json", None);
        assert_eq!(
            get_json(&client, &url, "synthetic-secret", None, false).unwrap_err(),
            FetchError::InvalidResponse
        );
        server.join().unwrap();
    }

    #[test]
    fn authenticated_http_does_not_follow_redirects() {
        let client = Client::builder().redirect(Policy::none()).build().unwrap();
        let (url, server) = mock_server(302, "", Some("http://127.0.0.1:1/token-exfiltration"));
        assert_eq!(
            get_json(&client, &url, "synthetic-secret", None, false).unwrap_err(),
            FetchError::Server(302)
        );
        server.join().unwrap();
    }
}
