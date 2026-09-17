use chrono::{DateTime, Duration, Utc};

use crate::models::{DashboardSnapshot, ProviderId, ProviderStatus, ProviderUsage, UsageWindow};

/// Providers already order their primary quota before secondary or model-specific quotas.
fn selected_window(snapshot: &DashboardSnapshot) -> Option<(&ProviderUsage, &UsageWindow)> {
    [ProviderId::Codex, ProviderId::Claude]
        .into_iter()
        .find_map(|id| {
            snapshot
                .providers
                .iter()
                .filter(|provider| provider.id == id && provider.status == ProviderStatus::Ready)
                .find_map(|provider| {
                    provider
                        .windows
                        .iter()
                        .find(|window| window.used_percent.is_finite())
                        .map(|window| (provider, window))
                })
        })
}

fn remaining_percent(window: &UsageWindow) -> u8 {
    (100.0 - window.used_percent).clamp(0.0, 100.0).round() as u8
}

fn reset_countdown(window: &UsageWindow, now: DateTime<Utc>) -> String {
    let Some(resets_at) = window
        .resets_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
    else {
        return "重置时间未知".into();
    };
    let remaining = resets_at.signed_duration_since(now);
    if remaining <= Duration::zero() {
        return "等待重置".into();
    }

    let whole_minutes = remaining.num_minutes();
    let minutes = whole_minutes + i64::from(remaining > Duration::minutes(whole_minutes));
    if minutes >= 24 * 60 {
        let days = minutes / (24 * 60);
        let hours = minutes % (24 * 60) / 60;
        if hours == 0 {
            format!("{days}天")
        } else {
            format!("{days}天 {hours}小时")
        }
    } else if minutes >= 60 {
        let hours = minutes / 60;
        let minutes = minutes % 60;
        if minutes == 0 {
            format!("{hours}小时")
        } else {
            format!("{hours}小时 {minutes}分钟")
        }
    } else {
        format!("{minutes}分钟")
    }
}

pub fn title(snapshot: &DashboardSnapshot, now: DateTime<Utc>) -> Option<String> {
    let (_, window) = selected_window(snapshot)?;
    Some(format!(
        "{}% {}",
        remaining_percent(window),
        reset_countdown(window, now)
    ))
}

pub fn tooltip(snapshot: &DashboardSnapshot, now: DateTime<Utc>) -> String {
    let Some((provider, window)) = selected_window(snapshot) else {
        return "AgentBar · 暂无可用用量，打开面板查看账号状态".into();
    };
    format!(
        "AgentBar · {} · {}\n剩余 {}% · {}",
        provider.name,
        window.label,
        remaining_percent(window),
        reset_countdown(window, now)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DataMode;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-16T04:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn window(used_percent: f64, remaining: Duration) -> UsageWindow {
        UsageWindow {
            label: "每周用量".into(),
            used_percent,
            resets_at: Some((now() + remaining).to_rfc3339()),
        }
    }

    fn provider(id: ProviderId, windows: Vec<UsageWindow>) -> ProviderUsage {
        ProviderUsage {
            id,
            name: match id {
                ProviderId::Codex => "Codex",
                ProviderId::Claude => "Claude",
            }
            .into(),
            plan: "Pro".into(),
            source: DataMode::Local,
            status: ProviderStatus::Ready,
            account: None,
            message: None,
            windows,
            updated_at: None,
            cache_scope: None,
        }
    }

    fn snapshot(providers: Vec<ProviderUsage>) -> DashboardSnapshot {
        DashboardSnapshot {
            providers,
            updated_at: now().to_rfc3339(),
            mode: DataMode::Live,
            revision: 1,
        }
    }

    #[test]
    fn title_uses_remaining_percent_and_requested_day_hour_format() {
        let snapshot = snapshot(vec![provider(
            ProviderId::Codex,
            vec![window(40.0, Duration::hours(68))],
        )]);
        assert_eq!(title(&snapshot, now()).as_deref(), Some("60% 2天 20小时"));
        assert_eq!(
            tooltip(&snapshot, now()),
            "AgentBar · Codex · 每周用量\n剩余 60% · 2天 20小时"
        );
    }

    #[test]
    fn countdown_rounds_up_to_minutes_and_handles_boundaries() {
        for (remaining, expected) in [
            (Duration::nanoseconds(1), "1分钟"),
            (Duration::seconds(60), "1分钟"),
            (Duration::seconds(61), "2分钟"),
            (Duration::minutes(59), "59分钟"),
            (Duration::seconds(3_541), "1小时"),
            (Duration::hours(1) + Duration::seconds(1), "1小时 1分钟"),
            (Duration::minutes(1439), "23小时 59分钟"),
            (Duration::seconds(86_341), "1天"),
            (Duration::days(2), "2天"),
            (Duration::hours(49) + Duration::minutes(20), "2天 1小时"),
            (Duration::zero(), "等待重置"),
            (Duration::seconds(-1), "等待重置"),
        ] {
            assert_eq!(reset_countdown(&window(40.0, remaining), now()), expected);
        }
    }

    #[test]
    fn missing_or_invalid_reset_keeps_known_usage() {
        for resets_at in [None, Some("invalid".into())] {
            let mut window = window(40.0, Duration::zero());
            window.resets_at = resets_at;
            let snapshot = snapshot(vec![provider(ProviderId::Codex, vec![window])]);
            assert_eq!(title(&snapshot, now()).as_deref(), Some("60% 重置时间未知"));
        }
    }

    #[test]
    fn reset_timestamps_respect_timezone_offsets() {
        let mut window = window(40.0, Duration::zero());
        window.resets_at = Some("2026-09-16T13:30:00+08:00".into());
        assert_eq!(reset_countdown(&window, now()), "1小时 30分钟");
    }

    #[test]
    fn remaining_percent_is_rounded_and_clamped() {
        for (used, expected) in [(40.4, 60), (40.5, 60), (40.6, 59), (-1.0, 100), (120.0, 0)] {
            assert_eq!(remaining_percent(&window(used, Duration::zero())), expected);
        }
    }

    #[test]
    fn codex_primary_window_wins_over_claude_and_other_windows() {
        let mut primary = window(40.0, Duration::hours(5));
        primary.label = "5 小时用量".into();
        let mut spark = window(90.0, Duration::hours(1));
        spark.label = "Spark · 5 小时用量".into();
        let snapshot = snapshot(vec![
            provider(ProviderId::Claude, vec![window(10.0, Duration::hours(1))]),
            provider(
                ProviderId::Codex,
                vec![primary, window(80.0, Duration::days(3)), spark],
            ),
        ]);
        assert_eq!(title(&snapshot, now()).as_deref(), Some("60% 5小时"));
        assert!(tooltip(&snapshot, now()).contains("Codex · 5 小时用量"));
    }

    #[test]
    fn unavailable_or_invalid_codex_usage_falls_back_to_claude() {
        for (status, windows) in [
            (
                ProviderStatus::Unavailable,
                vec![window(40.0, Duration::hours(5))],
            ),
            (
                ProviderStatus::Error,
                vec![window(40.0, Duration::hours(5))],
            ),
            (ProviderStatus::Ready, vec![]),
            (
                ProviderStatus::Ready,
                vec![window(f64::NAN, Duration::hours(5))],
            ),
        ] {
            let mut codex = provider(ProviderId::Codex, windows);
            codex.status = status;
            let snapshot = snapshot(vec![
                codex,
                provider(ProviderId::Claude, vec![window(10.0, Duration::hours(1))]),
            ]);
            assert_eq!(title(&snapshot, now()).as_deref(), Some("90% 1小时"));
            assert!(tooltip(&snapshot, now()).contains("Claude"));
        }
    }

    #[test]
    fn invalid_window_is_skipped_without_fabricating_usage() {
        let invalid = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY]
            .into_iter()
            .map(|used| window(used, Duration::hours(1)))
            .collect::<Vec<_>>();
        let mut provider = provider(ProviderId::Codex, invalid);
        assert_eq!(title(&snapshot(vec![provider.clone()]), now()), None);
        provider.windows.push(window(100.0, Duration::hours(1)));
        assert_eq!(
            title(&snapshot(vec![provider]), now()).as_deref(),
            Some("0% 1小时")
        );
        let empty = snapshot(vec![]);
        assert_eq!(title(&empty, now()), None);
        assert!(tooltip(&empty, now()).contains("暂无可用用量"));
    }
}
