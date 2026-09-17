//! Local activity uses normalized facts only; an open session is never a timed task.
use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Days, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use super::ActivityStatistics;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CompletedTask {
    key: String,
    started_at: Option<DateTime<Utc>>,
    completed_at: DateTime<Utc>,
    duration_sec: u64,
}

impl CompletedTask {
    pub(super) fn new(
        key: String,
        started_at: Option<DateTime<Utc>>,
        completed_at: DateTime<Utc>,
        duration_ms: Option<f64>,
    ) -> Option<Self> {
        if started_at.is_some_and(|start| start > completed_at) {
            return None;
        }
        let duration_ms = match duration_ms {
            Some(value) if value.is_finite() && value >= 0.0 => value,
            Some(_) => return None,
            None => completed_at
                .signed_duration_since(started_at?)
                .num_milliseconds() as f64,
        };
        Some(Self {
            key,
            started_at,
            completed_at,
            duration_sec: (duration_ms / 1000.0).floor() as u64,
        })
    }
}

pub(super) fn summarize<'a, Tz: TimeZone>(
    now: DateTime<Tz>,
    timestamps: impl IntoIterator<Item = DateTime<Utc>>,
    tasks: impl IntoIterator<Item = &'a CompletedTask>,
) -> Option<ActivityStatistics> {
    let end = now.with_timezone(&Utc);
    let zone = now.timezone();
    let today = now.date_naive();
    let mut dates: BTreeSet<_> = timestamps
        .into_iter()
        .filter(|time| *time <= end)
        .map(|time| time.with_timezone(&zone).date_naive())
        .collect();
    let mut completed = HashMap::<&str, u64>::new();
    for task in tasks.into_iter().filter(|task| task.completed_at <= end) {
        completed
            .entry(&task.key)
            .and_modify(|duration| *duration = (*duration).max(task.duration_sec))
            .or_insert(task.duration_sec);
        dates.insert(task.completed_at.with_timezone(&zone).date_naive());
        if let Some(start) = task.started_at {
            dates.insert(start.with_timezone(&zone).date_naive());
        }
    }
    if dates.is_empty() {
        return None;
    }
    let mut previous = None;
    let mut run = 0_u64;
    let mut longest = 0;
    for date in &dates {
        run = if previous.is_some_and(|prior: chrono::NaiveDate| prior.succ_opt() == Some(*date)) {
            run + 1
        } else {
            1
        };
        longest = longest.max(run);
        previous = Some(*date);
    }
    let current = if dates.contains(&today)
        || today
            .checked_sub_days(Days::new(1))
            .is_some_and(|yesterday| dates.contains(&yesterday))
    {
        run
    } else {
        0
    };
    Some(ActivityStatistics {
        longest_running_turn_sec: completed.values().copied().max(),
        current_streak_days: Some(current),
        longest_streak_days: Some(longest),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn streaks_use_local_days_deduplicate_and_ignore_future_records() {
        let now = DateTime::parse_from_rfc3339("2026-09-17T00:30:00+08:00").unwrap();
        let times = [
            "2026-09-12T16:00:00Z",
            "2026-09-13T16:00:00Z", // September 13, 14
            "2026-09-14T16:00:00Z", // historical 3-day run
            "2026-09-14T20:00:00Z", // same local day
            "2026-09-16T16:00:00Z", // September 17 after a gap
            "2026-09-17T16:00:00Z", // future date must not extend the run
        ]
        .map(time);
        let result = summarize(now, times, []).unwrap();
        assert_eq!(result.current_streak_days, Some(1));
        assert_eq!(result.longest_streak_days, Some(3));
        assert_eq!(result.longest_running_turn_sec, None);
        // One UTC hour straddles midnight in China but is still one date west of UTC.
        let boundary = ["2026-09-16T15:59:00Z", "2026-09-16T16:01:00Z"].map(time);
        let east_result = summarize(now, boundary, []).unwrap();
        assert_eq!(east_result.current_streak_days, Some(2));
        let west = now.with_timezone(&chrono::FixedOffset::west_opt(7 * 3600).unwrap());
        let result = summarize(west, boundary, []).unwrap();
        assert_eq!(result.current_streak_days, Some(1));
        assert_eq!(result.longest_streak_days, Some(1));
    }

    #[test]
    fn streak_ending_yesterday_survives_until_tomorrow_and_empty_is_unknown() {
        let times = [
            "2026-09-14T09:00:00Z",
            "2026-09-15T09:00:00Z",
            "2026-09-16T09:00:00Z",
        ]
        .map(time);
        let result = summarize(time("2026-09-17T09:00:00Z"), times, []).unwrap();
        assert_eq!(result.current_streak_days, Some(3));
        let result = summarize(time("2026-09-18T09:00:00Z"), times, []).unwrap();
        assert_eq!(result.current_streak_days, Some(0));
        assert_eq!(result.longest_streak_days, Some(3));
        assert!(summarize(time("2026-09-17T09:00:00Z"), [], []).is_none());
    }

    #[test]
    fn explicit_duration_is_authoritative_and_only_completed_past_tasks_count() {
        let start = time("2026-09-16T09:00:00Z");
        let end = time("2026-09-16T09:02:00Z");
        let task = CompletedTask::new("task".into(), Some(start), end, Some(1500.0)).unwrap();
        assert_eq!(task.duration_sec, 1);
        let same_task = task.clone();
        let future = CompletedTask::new(
            "future".into(),
            Some(start),
            time("2026-09-18T09:00:00Z"),
            None,
        )
        .unwrap();
        let result = summarize(
            time("2026-09-17T09:00:00Z"),
            [],
            [&task, &same_task, &future],
        )
        .unwrap();
        assert_eq!(result.longest_running_turn_sec, Some(1));
        assert_eq!(result.current_streak_days, Some(1));
        assert!(CompletedTask::new("bad".into(), Some(end), start, Some(1.0)).is_none());
        assert!(CompletedTask::new("bad".into(), None, end, Some(-1.0)).is_none());
        assert!(CompletedTask::new("bad".into(), None, end, Some(f64::NAN)).is_none());
        assert!(CompletedTask::new("unknown".into(), None, end, None).is_none());
    }
}
