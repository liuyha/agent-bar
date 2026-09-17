//! Read-only reset balances. Wire fields follow the native Codex app and its
//! `account/rateLimits/read` schema; detail lists may be capped by the service.

use std::collections::HashSet;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;

use crate::models::{ResetCredit, ResetCredits};

const UNAVAILABLE_DETAILS: &str = "暂时无法读取重置次数明细，请稍后刷新";

#[derive(Clone, Copy)]
pub(super) enum WireFormat {
    Http,
    Cli,
}

pub(super) fn summary(
    value: Option<&Value>,
    format: WireFormat,
    now: DateTime<Utc>,
) -> Option<ResetCredits> {
    let value = value.filter(|value| !value.is_null())?;
    let count_key = match format {
        WireFormat::Http => "available_count",
        WireFormat::Cli => "availableCount",
    };
    let remaining = value.get(count_key).and_then(Value::as_u64);
    let mut result = ResetCredits {
        remaining,
        credits: None,
        updated_at: remaining.map(|_| now.to_rfc3339_opts(SecondsFormat::Secs, true)),
        message: Some(UNAVAILABLE_DETAILS.into()),
    };
    if let Some(credits) = value.get("credits").filter(|credits| !credits.is_null()) {
        if let Some(credits) = parse_details(credits, format, now) {
            result.credits = Some(credits);
            result.updated_at = Some(now.to_rfc3339_opts(SecondsFormat::Secs, true));
            result.message = remaining.is_none().then(|| "服务未返回重置剩余数量".into());
        }
    }
    Some(result)
}

pub(super) fn apply_http_details(
    current: &mut Option<ResetCredits>,
    value: &Value,
    now: DateTime<Utc>,
) {
    let Some(mut details) = summary(Some(value), WireFormat::Http, now) else {
        return;
    };
    // A missing details count does not erase a confirmed usage count. Never
    // derive a balance from list length: native Codex documents capped lists.
    if details.remaining.is_none() {
        details.remaining = current.as_ref().and_then(|summary| summary.remaining);
        if details.updated_at.is_none() {
            details.updated_at = current
                .as_ref()
                .and_then(|summary| summary.updated_at.clone());
        }
        if details.credits.is_some() && details.remaining.is_some() {
            details.message = None;
        }
    }
    *current = Some(details);
}

pub(super) fn mark_details_unavailable(current: &mut Option<ResetCredits>) {
    let current = current.get_or_insert_with(|| ResetCredits {
        remaining: None,
        credits: None,
        updated_at: None,
        message: None,
    });
    current.message = Some(UNAVAILABLE_DETAILS.into());
}

fn parse_details(
    value: &Value,
    format: WireFormat,
    now: DateTime<Utc>,
) -> Option<Vec<ResetCredit>> {
    let mut credits = Vec::new();
    let mut seen = HashSet::new();
    for entry in value.as_array()? {
        let status = entry.get("status")?.as_str()?;
        if status != "available" {
            continue;
        }
        let id = entry.get("id")?.as_str()?.trim();
        if id.is_empty() {
            return None;
        }
        let expires = match format {
            WireFormat::Http => match entry.get("expires_at") {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    DateTime::parse_from_rfc3339(value.as_str()?)
                        .ok()?
                        .with_timezone(&Utc),
                ),
            },
            WireFormat::Cli => match entry.get("expiresAt") {
                None | Some(Value::Null) => None,
                Some(value) => Some(DateTime::<Utc>::from_timestamp(value.as_i64()?, 0)?),
            },
        };
        if expires.is_some_and(|expires| expires <= now) || !seen.insert(id.to_owned()) {
            continue;
        }
        credits.push(ResetCredit {
            id: id.to_owned(),
            remaining: 1,
            expires_at: expires.map(|expires| expires.to_rfc3339_opts(SecondsFormat::Secs, true)),
        });
    }
    credits.sort_by(|a, b| match (&a.expires_at, &b.expires_at) {
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.id.cmp(&b.id),
    });
    Some(credits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-17T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn missing_balances_and_details_are_distinct_from_confirmed_zero() {
        assert!(summary(None, WireFormat::Http, now()).is_none());
        let unknown = summary(
            Some(&json!({"available_count":null})),
            WireFormat::Http,
            now(),
        )
        .unwrap();
        assert_eq!(unknown.remaining, None);
        assert_eq!(unknown.credits, None);
        let zero = summary(
            Some(&json!({"available_count":0,"credits":[]})),
            WireFormat::Http,
            now(),
        )
        .unwrap();
        assert_eq!(zero.remaining, Some(0));
        assert_eq!(zero.credits, Some(vec![]));
        assert!(zero.message.is_none());
        assert_eq!(
            summary(
                Some(&json!({"available_count":-1})),
                WireFormat::Http,
                now()
            )
            .unwrap()
            .remaining,
            None
        );
    }

    #[test]
    fn http_uses_available_balance_and_only_available_unexpired_distinct_details() {
        let result = summary(
            Some(
                &json!({"available_count":5,"applicable_available_count":0,"credits":[
                    {"id":"later","status":"available","expires_at":"2026-10-05T04:19:47.093933Z"},
                    {"id":"first","status":"available","expires_at":"2026-10-04T02:33:50.795488Z"},
                    {"id":"first","status":"available","expires_at":"2026-10-04T02:33:50.795488Z"},
                    {"id":"used","status":"redeemed","expires_at":null},
                    {"id":"expired","status":"available","expires_at":"2026-09-17T08:00:00Z"},
                    {"id":"forever","status":"available","expires_at":null}
                ]}),
            ),
            WireFormat::Http,
            now(),
        )
        .unwrap();
        assert_eq!(result.remaining, Some(5));
        let credits = result.credits.unwrap();
        assert_eq!(
            credits
                .iter()
                .map(|credit| credit.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "later", "forever"]
        );
        assert_eq!(
            credits[0].expires_at.as_deref(),
            Some("2026-10-04T02:33:50Z")
        );
        assert!(credits.iter().all(|credit| credit.remaining == 1));
    }

    #[test]
    fn cli_uses_epoch_expiry_and_preserves_capped_balance() {
        let result = summary(Some(&json!({"availableCount":2,"credits":[{"id":"credit","status":"available","expiresAt":1789805400}]})), WireFormat::Cli, now()).unwrap();
        assert_eq!(result.remaining, Some(2));
        let credits = result.credits.unwrap();
        assert_eq!(credits.len(), 1);
        assert_eq!(
            credits[0].expires_at.as_deref(),
            Some("2026-09-19T08:10:00Z")
        );
    }

    #[test]
    fn detail_failure_does_not_erase_known_count_or_invent_empty_list() {
        let mut current = summary(Some(&json!({"available_count":2})), WireFormat::Http, now());
        apply_http_details(
            &mut current,
            &json!({"credits":[{"id":"bad","status":"available","expires_at":"invalid"}]}),
            now(),
        );
        let current = current.unwrap();
        assert_eq!(current.remaining, Some(2));
        assert!(current.credits.is_none());
        assert!(current.message.is_some());
    }
}
