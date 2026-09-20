//! Hourly history uses the same normalized requests as the summary and daily history.
use super::{
    add_event, day_start, empty_period, iso, Event, ProviderId, ProviderStatus, TokenPeriod,
};
use chrono::{DateTime, Duration, TimeZone, Timelike, Utc};
use std::collections::BTreeMap;

pub(super) fn summarize<'a, Tz: TimeZone>(
    provider: ProviderId,
    status: ProviderStatus,
    zone: &Tz,
    end: DateTime<Utc>,
    events: impl IntoIterator<Item = &'a Event>,
    turns: impl IntoIterator<Item = DateTime<Utc>>,
    unknown_turns: impl IntoIterator<Item = DateTime<Utc>>,
) -> Result<Vec<TokenPeriod>, String> {
    let yesterday = end
        .with_timezone(zone)
        .date_naive()
        .pred_opt()
        .ok_or("无法计算本地小时统计边界")?;
    let start = day_start(zone, yesterday)?;
    let in_range = |timestamp: DateTime<Utc>| timestamp >= start && timestamp <= end;
    let hour_start = |timestamp: DateTime<Utc>| {
        let local = timestamp.with_timezone(zone);
        // Subtract elapsed minutes instead of resolving an ambiguous wall-clock time.
        // Repeated hours at a daylight-saving transition retain separate UTC buckets.
        timestamp
            - Duration::minutes(i64::from(local.minute()))
            - Duration::seconds(i64::from(local.second()))
            - Duration::nanoseconds(i64::from(local.nanosecond()))
    };
    let mut buckets = BTreeMap::new();
    for event in events.into_iter().filter(|event| in_range(event.timestamp)) {
        let period = buckets
            .entry(hour_start(event.timestamp))
            .or_insert_with(|| empty_period("all", end, end, status));
        add_event(period, provider, event);
    }
    for timestamp in turns.into_iter().filter(|timestamp| in_range(*timestamp)) {
        let period = buckets
            .entry(hour_start(timestamp))
            .or_insert_with(|| empty_period("all", end, end, status));
        period.conversation_turns = period
            .conversation_turns
            .map(|count| count.saturating_add(1));
    }
    for timestamp in unknown_turns
        .into_iter()
        .filter(|timestamp| in_range(*timestamp))
    {
        buckets
            .entry(hour_start(timestamp))
            .or_insert_with(|| empty_period("all", end, end, status))
            .conversation_turns = None;
    }
    Ok(buckets
        .into_iter()
        .map(|(start, mut period)| {
            period.start_at = iso(start);
            period.end_at = iso((start + Duration::hours(1) - Duration::milliseconds(1)).min(end));
            if period.total_tokens > 0 && period.unpriced_tokens == period.total_tokens {
                period.estimated_cost_usd = None;
            }
            period
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statistics::Usage;
    use chrono::FixedOffset;

    fn time(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn local_hours_filter_range_preserve_unknowns_and_keep_turn_only_hours() {
        let zone = FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap();
        let end = time("2026-01-02T04:45:00Z");
        let event = |timestamp: &str, model: &str, requests| Event {
            key: timestamp.into(),
            timestamp: time(timestamp),
            model: Some(model.into()),
            usage: Usage {
                input: 100,
                output: 10,
                ..Usage::default()
            },
            request_count: requests,
            legacy_turn: None,
            fork_baseline: None,
            codex_snapshot: None,
        };
        let events = [
            event("2025-12-31T18:29:59.999Z", "gpt-6-astra", Some(1)),
            event("2025-12-31T18:30:00Z", "gpt-6-astra", Some(1)),
            event("2025-12-31T19:29:59.999Z", "gpt-6-astra", Some(1)),
            event("2025-12-31T19:30:00Z", "unpriced-model", None),
            event("2026-01-02T04:45:00Z", "gpt-6-astra", Some(1)),
            event("2026-01-02T04:45:00.001Z", "gpt-6-astra", Some(1)),
        ];
        let hourly = summarize(
            ProviderId::Codex,
            ProviderStatus::Ready,
            &zone,
            end,
            &events,
            [time("2026-01-02T03:50:00Z")],
            [
                time("2025-12-31T19:35:00Z"),
                time("2026-01-02T04:45:00.001Z"),
            ],
        )
        .unwrap();
        assert_eq!(hourly.len(), 4);
        assert!(hourly.iter().all(|hour| hour.period == "all"));
        assert_eq!(hourly[0].start_at, "2025-12-31T18:30:00.000Z");
        assert_eq!(hourly[0].end_at, "2025-12-31T19:29:59.999Z");
        assert_eq!(hourly[0].total_tokens, 220);
        assert_eq!(hourly[0].request_count, Some(2));
        assert!(hourly[0].estimated_cost_usd.unwrap() > 0.0);
        assert_eq!(hourly[1].unpriced_tokens, 110);
        assert_eq!(hourly[1].estimated_cost_usd, None);
        assert_eq!(hourly[1].request_count, None);
        assert_eq!(hourly[1].conversation_turns, None);
        assert_eq!(hourly[2].total_tokens, 0);
        assert_eq!(hourly[2].conversation_turns, Some(1));
        assert_eq!(hourly[3].start_at, "2026-01-02T04:30:00.000Z");
        assert_eq!(hourly[3].end_at, "2026-01-02T04:45:00.000Z");
        assert_eq!(hourly[3].total_tokens, 110);
        assert_eq!(hourly[3].conversation_turns, Some(0));
    }
}
