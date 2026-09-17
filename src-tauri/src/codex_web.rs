//! Optional, user-opened ChatGPT session. Remote pages have no Tauri capabilities.
//! Credentials stay inside the isolated webview; only the selected usage DTO leaves it.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use serde::Deserialize;
use serde_json::json;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::{account_statistics::WebUsageSnapshot, models::ProviderStatus};

const LABEL: &str = "codex-web-usage";
const USAGE_URL: &str = "https://chatgpt.com/codex/settings/usage";
const SCRIPT: &str = include_str!("codex_web/read_usage.js");
const CONFIG_MARKER: &str = "/*__AGENTBAR_WEB_CONFIG_JSON__*/null";
const CANCEL_SCRIPT: &str = "(() => { const s = window.__agentbarWebUsageV1; if (s) { s.cancel(); delete window.__agentbarWebUsageV1; } })()";
static GENERATION: AtomicU64 = AtomicU64::new(0);
static REQUEST_ID: AtomicU64 = AtomicU64::new(0);
static READ_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// This is called only by the explicit "open usage page" action, never by refresh.
pub fn open(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LABEL) {
        window.show().map_err(|_| "无法打开 Codex 用量网页。")?;
        window
            .set_focus()
            .map_err(|_| "无法聚焦 Codex 用量网页。")?;
        return Ok(());
    }
    let popup_app = app.clone();
    let window = WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::External(USAGE_URL.parse().expect("fixed HTTPS URL")),
    )
    .title("Codex 用量网页 · AgentBar")
    .inner_size(1100.0, 800.0)
    .min_inner_size(720.0, 540.0)
    // Available on macOS 12. No external browser cookies or disk credentials are imported.
    .incognito(true)
    .on_navigation(|url| {
        let allowed = allowed_navigation(url);
        if allowed {
            GENERATION.fetch_add(1, Ordering::SeqCst);
        }
        allowed
    })
    .on_new_window(move |url, _| {
        // Authentication popups use the same isolated session and never gain a new surface.
        if allowed_navigation(&url) {
            if let Some(window) = popup_app.get_webview_window(LABEL) {
                let _ = window.navigate(url);
            }
        }
        tauri::webview::NewWindowResponse::Deny
    })
    .build()
    .map_err(|_| "无法创建 Codex 用量网页窗口。".to_string())?;
    GENERATION.fetch_add(1, Ordering::SeqCst);
    let event_window = window.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            // Closing the visible page preserves this application's temporary login session.
            api.prevent_close();
            GENERATION.fetch_add(1, Ordering::SeqCst);
            let _ = event_window.eval(CANCEL_SCRIPT);
            let _ = event_window.hide();
        }
        tauri::WindowEvent::Destroyed => {
            GENERATION.fetch_add(1, Ordering::SeqCst);
        }
        _ => {}
    });
    Ok(())
}

/// Disabling the integration destroys its temporary login session and in-flight reads.
pub fn close(app: &AppHandle) {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.eval(CANCEL_SCRIPT);
        let _ = window.clear_all_browsing_data();
        let _ = window.destroy();
    }
}

fn allowed_navigation(url: &tauri::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "chatgpt.com"
                    | "auth.openai.com"
                    | "auth0.openai.com"
                    | "auth.chatgpt.com"
                    | "accounts.google.com"
                    | "login.microsoftonline.com"
                    | "login.live.com"
                    | "appleid.apple.com"
            )
        )
}

fn usage_origin(url: &tauri::Url) -> bool {
    allowed_navigation(url) && url.host_str() == Some("chatgpt.com")
}

fn unavailable(message: &str) -> WebUsageSnapshot {
    WebUsageSnapshot {
        status: ProviderStatus::Unavailable,
        message: Some(message.to_string()),
        account: None,
        credits_remaining: None,
        code_review_remaining_percent: None,
        usage_unit: None,
        usage_breakdown: None,
        credit_events: None,
        updated_at: None,
    }
}

/// The caller supplies the identity verified by its current OAuth/PAT backend request.
/// Missing identity never falls back to whichever browser account happens to be logged in.
pub async fn read(
    app: &AppHandle,
    expected_account: Option<&str>,
    expected_account_id: Option<&str>,
) -> WebUsageSnapshot {
    let Some(account) = expected_account.map(str::trim).filter(|s| !s.is_empty()) else {
        return unavailable("服务端尚未确认当前账号，暂不合并网页补充数据。");
    };
    let Some(account_id) = expected_account_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return unavailable("服务端尚未确认当前工作区，暂不合并网页补充数据。");
    };
    let _lock = READ_LOCK.lock().await;
    let Some(window) = app.get_webview_window(LABEL) else {
        return unavailable("请先打开 Codex 用量网页，并登录与当前数据源相同的账号。");
    };
    if !window.url().is_ok_and(|url| usage_origin(&url)) {
        return unavailable("请在 Codex 用量网页完成登录，然后刷新使用统计。");
    }
    let generation = GENERATION.load(Ordering::SeqCst);
    let request_id = REQUEST_ID.fetch_add(1, Ordering::SeqCst) + 1;
    let config = json!({
        "requestId": request_id,
        "expectedAccount": account,
        "expectedAccountId": account_id,
    });
    let script = SCRIPT.replace(CONFIG_MARKER, &config.to_string());
    if window.eval(script).is_err() {
        return unavailable("Codex 用量网页尚未就绪，请完成登录后刷新。");
    }

    let result = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if GENERATION.load(Ordering::SeqCst) != generation
                || !window.url().is_ok_and(|url| usage_origin(&url))
            {
                return Err("网页读取已取消，请刷新使用统计。");
            }
            let raw = evaluate(
                &window,
                format!(
                    "(() => {{ if (location.origin !== 'https://chatgpt.com') return null; const s = window.__agentbarWebUsageV1; return s && s.id === {request_id} ? {{ id: s.id, done: s.done, snapshot: s.snapshot }} : null; }})()"
                ),
            )
            .await?;
            if let Some(result) = parse_result(&raw, request_id, account)? {
                // A close/navigation while WebKit was evaluating also invalidates its result.
                if GENERATION.load(Ordering::SeqCst) != generation
                    || !window.url().is_ok_and(|url| usage_origin(&url))
                {
                    return Err("网页读取已取消，请刷新使用统计。");
                }
                return Ok(result);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await;
    // The request id prevents an old read from deleting a newer read's state.
    let _ = window.eval(format!(
        "(() => {{ const s = window.__agentbarWebUsageV1; if (s && s.id === {request_id}) {{ s.cancel(); delete window.__agentbarWebUsageV1; }} }})()"
    ));
    match result {
        Ok(Ok(snapshot)) => snapshot,
        Ok(Err(message)) => unavailable(message),
        Err(_) => unavailable("Codex 用量网页读取超时，请完成登录后刷新。"),
    }
}

async fn evaluate(window: &WebviewWindow, script: String) -> Result<String, &'static str> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = std::sync::Mutex::new(Some(sender));
    window
        .eval_with_callback(script, move |result| {
            if let Ok(mut sender) = sender.lock() {
                if let Some(sender) = sender.take() {
                    let _ = sender.send(result);
                }
            }
        })
        .map_err(|_| "Codex 用量网页已关闭，请重新打开网页。")?;
    tokio::time::timeout(Duration::from_secs(3), receiver)
        .await
        .map_err(|_| "Codex 用量网页暂未响应，请刷新使用统计。")?
        .map_err(|_| "Codex 用量网页已关闭，请重新打开网页。")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptResult {
    id: u64,
    done: bool,
    snapshot: Option<serde_json::Value>,
}

fn parse_result(
    raw: &str,
    request_id: u64,
    expected_account: &str,
) -> Result<Option<WebUsageSnapshot>, &'static str> {
    // Bound the callback payload and never include webpage data in diagnostics.
    if raw.len() > 512 * 1024 {
        return Err("Codex 网页补充数据格式异常。");
    }
    let result: Option<ScriptResult> =
        serde_json::from_str(raw).map_err(|_| "Codex 网页补充数据格式异常。")?;
    match result {
        Some(result) if result.id == request_id && result.done => {
            let raw_snapshot = result.snapshot.ok_or("Codex 网页补充数据格式异常。")?;
            if !allowed_fields(&raw_snapshot) {
                return Err("Codex 网页补充数据格式异常。");
            }
            let snapshot =
                serde_json::from_value(raw_snapshot).map_err(|_| "Codex 网页补充数据格式异常。")?;
            if !valid_snapshot(&snapshot, expected_account) {
                return Err("Codex 网页补充数据校验失败，请确认账号后刷新。");
            }
            Ok(Some(snapshot))
        }
        Some(result) if result.id == request_id => Ok(None),
        _ => Err("网页已切换或重新加载，请刷新使用统计。"),
    }
}

fn object_fields(value: &serde_json::Value, fields: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.keys().all(|key| fields.contains(&key.as_str())))
}

fn allowed_fields(value: &serde_json::Value) -> bool {
    if !object_fields(
        value,
        &[
            "status",
            "message",
            "account",
            "creditsRemaining",
            "codeReviewRemainingPercent",
            "usageUnit",
            "usageBreakdown",
            "creditEvents",
            "updatedAt",
        ],
    ) {
        return false;
    }
    if let Some(days) = value.get("usageBreakdown").and_then(|v| v.as_array()) {
        if days.iter().any(|day| {
            !object_fields(day, &["date", "amounts"])
                || day
                    .get("amounts")
                    .and_then(|v| v.as_array())
                    .is_some_and(|amounts| {
                        amounts
                            .iter()
                            .any(|amount| !object_fields(amount, &["service", "amount"]))
                    })
        }) {
            return false;
        }
    }
    !value
        .get("creditEvents")
        .and_then(|v| v.as_array())
        .is_some_and(|events| {
            events
                .iter()
                .any(|event| !object_fields(event, &["date", "service", "credits"]))
        })
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.: -".contains(&c))
}

fn valid_date(value: &str) -> bool {
    value.len() <= 40
        && (chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
            || chrono::DateTime::parse_from_rfc3339(value).is_ok())
}

fn partial_message(message: &str) -> bool {
    let Some(missing) = message
        .strip_prefix("部分网页补充数据暂不可用：")
        .and_then(|s| s.strip_suffix('。'))
    else {
        return false;
    };
    matches!(
        missing,
        "额度补充"
            | "每日用量"
            | "Credit 记录"
            | "额度补充、每日用量"
            | "额度补充、Credit 记录"
            | "每日用量、Credit 记录"
    )
}

/// The page can mutate its JS globals. Recheck identity, limits and allowed text natively.
fn valid_snapshot(snapshot: &WebUsageSnapshot, expected_account: &str) -> bool {
    if snapshot.status != ProviderStatus::Ready {
        return snapshot.account.is_none()
            && snapshot.credits_remaining.is_none()
            && snapshot.code_review_remaining_percent.is_none()
            && snapshot.usage_unit.is_none()
            && snapshot.usage_breakdown.is_none()
            && snapshot.credit_events.is_none()
            && snapshot.updated_at.is_none()
            && matches!(
                snapshot.message.as_deref(),
                Some(
                    "服务端尚未确认当前账号和工作区，暂不合并网页补充数据。"
                        | "请在 Codex 用量网页完成登录，然后刷新使用统计。"
                        | "网页登录账号与当前数据源不一致，请切换为相同账号后刷新。"
                        | "网页登录已失效，请重新登录后刷新使用统计。"
                        | "网页返回的工作区与当前数据源不一致，已停止合并网页补充数据。"
                        | "Codex 用量网页读取超时，请完成登录后刷新。"
                        | "网页补充接口不可用，请在用量网页检查登录状态和工作区权限。"
                        | "Codex 网页补充读取失败，请刷新使用统计。"
                )
            );
    }
    let account = expected_account.trim().to_lowercase();
    snapshot.account.as_deref() == Some(account.as_str())
        && snapshot.message.as_deref().is_none_or(partial_message)
        && snapshot
            .updated_at
            .as_deref()
            .is_some_and(|v| chrono::DateTime::parse_from_rfc3339(v).is_ok())
        && snapshot
            .credits_remaining
            .is_none_or(|v| v.is_finite() && v >= 0.0)
        && snapshot
            .code_review_remaining_percent
            .is_none_or(|v| v.is_finite() && (0.0..=100.0).contains(&v))
        && snapshot.usage_unit.as_deref().is_none_or(valid_name)
        && (snapshot.usage_unit.is_none() || snapshot.usage_breakdown.is_some())
        && snapshot.usage_breakdown.as_ref().is_none_or(|days| {
            days.len() <= 62
                && days.iter().all(|day| {
                    valid_date(&day.date)
                        && day.amounts.len() <= 64
                        && day
                            .amounts
                            .iter()
                            .all(|amount| valid_name(&amount.service) && amount.amount.is_finite())
                })
        })
        && snapshot.credit_events.as_ref().is_none_or(|events| {
            events.len() <= 1000
                && events.iter().all(|event| {
                    valid_date(&event.date)
                        && valid_name(&event.service)
                        && event.credits.is_finite()
                })
        })
        && (snapshot.credits_remaining.is_some()
            || snapshot.code_review_remaining_percent.is_some()
            || snapshot.usage_breakdown.is_some()
            || snapshot.credit_events.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_is_limited_to_https_login_hosts() {
        for url in [
            USAGE_URL,
            "https://auth.openai.com/log-in",
            "https://accounts.google.com/signin",
        ] {
            assert!(allowed_navigation(&url.parse().unwrap()), "{url}");
        }
        for url in [
            "http://chatgpt.com/codex",
            "https://chatgpt.com.evil.test/",
            "https://evil.test/",
            "https://chatgpt.com:444/",
            "https://username@chatgpt.com/",
            "tauri://localhost/",
            "file:///tmp/test.html",
            "https://localhost/",
            "javascript:alert(1)",
        ] {
            assert!(!allowed_navigation(&url.parse().unwrap()), "{url}");
        }
        assert!(!usage_origin(&"https://auth.openai.com/".parse().unwrap()));
    }

    #[test]
    fn results_require_current_request_and_finished_snapshot() {
        assert!(
            parse_result(r#"{"id":1,"done":false,"snapshot":null}"#, 1, "a@b.test")
                .unwrap()
                .is_none()
        );
        assert!(parse_result(r#"{"id":1,"done":false,"snapshot":null}"#, 2, "a@b.test").is_err());
        assert!(parse_result(r#"{"id":1,"done":true,"snapshot":null}"#, 1, "a@b.test").is_err());
        assert!(parse_result("null", 1, "a@b.test").is_err());
        assert!(parse_result("secret malformed response", 1, "a@b.test")
            .unwrap_err()
            .contains("格式异常"));
    }

    fn valid_wire_result() -> serde_json::Value {
        json!({"id":1,"done":true,"snapshot":{
            "status":"ready","message":null,"account":"a@b.test","creditsRemaining":0,
            "codeReviewRemainingPercent":100,"usageUnit":"credits",
            "usageBreakdown":[{"date":"2026-09-17","amounts":[{"service":"codex_cli","amount":1.5}]}],
            "creditEvents":[{"date":"2026-09-17T00:00:00Z","service":"codex_cli","credits":-1.5}],
            "updatedAt":"2026-09-17T12:00:00Z"
        }})
    }

    #[test]
    fn callback_round_trip_preserves_units_and_signed_credits() {
        let snapshot = parse_result(&valid_wire_result().to_string(), 1, " A@B.test ")
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.usage_unit.as_deref(), Some("credits"));
        assert_eq!(snapshot.credit_events.unwrap()[0].credits, -1.5);
        // Returning an object from JS produces a JSON object in Wry's callback, not a JSON string.
        assert!(parse_result(
            &json!(valid_wire_result().to_string()).to_string(),
            1,
            "a@b.test"
        )
        .is_err());
    }

    #[test]
    fn altered_callback_cannot_bypass_native_identity_amount_or_field_checks() {
        for (field, value) in [
            ("account", json!("other@example.test")),
            ("creditsRemaining", json!(-1)),
            ("codeReviewRemainingPercent", json!(101)),
            ("updatedAt", json!("not-a-date")),
            ("message", json!("raw secret response")),
            ("accessToken", json!("unexpected secret")),
        ] {
            let mut raw = valid_wire_result();
            raw["snapshot"][field] = value;
            assert!(
                parse_result(&raw.to_string(), 1, "a@b.test").is_err(),
                "{field}"
            );
        }
        let mut raw = valid_wire_result();
        raw["snapshot"]["creditEvents"][0]["date"] = json!("2026-02-31");
        assert!(parse_result(&raw.to_string(), 1, "a@b.test").is_err());
        raw = valid_wire_result();
        raw["snapshot"]["creditEvents"][0]["raw"] = json!("unexpected");
        assert!(parse_result(&raw.to_string(), 1, "a@b.test").is_err());
        let mut snapshot: WebUsageSnapshot =
            serde_json::from_value(valid_wire_result()["snapshot"].clone()).unwrap();
        snapshot.credits_remaining = Some(f64::INFINITY);
        assert!(!valid_snapshot(&snapshot, "a@b.test"));
    }

    #[test]
    fn altered_callback_is_bounded_and_unavailable_cannot_carry_data() {
        let mut raw = valid_wire_result();
        raw["snapshot"]["usageBreakdown"] =
            json!(vec![raw["snapshot"]["usageBreakdown"][0].clone(); 63]);
        assert!(parse_result(&raw.to_string(), 1, "a@b.test").is_err());
        raw = valid_wire_result();
        raw["snapshot"]["status"] = json!("unavailable");
        raw["snapshot"]["message"] = json!("请在 Codex 用量网页完成登录，然后刷新使用统计。");
        assert!(parse_result(&raw.to_string(), 1, "a@b.test").is_err());
        assert!(parse_result(&"x".repeat(512 * 1024 + 1), 1, "a@b.test").is_err());
    }

    #[test]
    fn unavailable_never_contains_previous_account_or_amounts() {
        let snapshot = unavailable("未连接");
        assert_eq!(snapshot.status, ProviderStatus::Unavailable);
        assert!(snapshot.account.is_none());
        assert!(snapshot.credits_remaining.is_none());
        assert!(snapshot.usage_breakdown.is_none());
        assert!(snapshot.updated_at.is_none());
        assert_eq!(SCRIPT.matches(CONFIG_MARKER).count(), 1);
    }
}
