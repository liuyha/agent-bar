use super::*;
use serde_json::{json, Value};
use std::io::{Cursor, Write};
use tempfile::tempdir;

fn time(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn records(values: &[Value]) -> String {
    values
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn parse(provider: ProviderId, values: &[Value]) -> ParsedFile {
    parse_reader(
        Cursor::new(records(values)),
        provider,
        Path::new("activity.jsonl"),
    )
    .unwrap()
}

fn task(kind: &str, id: &str, timestamp: &str) -> Value {
    json!({"type":"event_msg","timestamp":timestamp,"payload":{"type":kind,"turn_id":id}})
}

fn summary(parsed: &ParsedFile) -> Option<ActivityStatistics> {
    activity::summarize(
        time("2026-09-17T12:00:00Z"),
        parsed.turns.iter().map(|turn| turn.timestamp),
        &parsed.completed_tasks,
    )
}

#[test]
fn codex_pairs_turn_ids_and_ignores_unfinished_aborted_or_unmatched_tasks() {
    let parsed = parse(
        ProviderId::Codex,
        &[
            task("task_started", "done", "2026-09-16T09:00:00Z"),
            task("task_started", "done", "2026-09-16T09:00:10Z"), // repeated start
            task("task_complete", "done", "2026-09-16T09:02:30Z"),
            task("task_started", "alias", "2026-09-16T09:04:00Z"),
            task("task_completed", "alias", "2026-09-16T09:05:00Z"),
            task("task_started", "open", "2026-09-01T09:00:00Z"),
            task("task_complete", "unmatched", "2026-09-16T09:09:00Z"),
            task("task_started", "abort", "2026-09-01T09:00:00Z"),
            task("turn_aborted", "abort", "2026-09-16T09:00:00Z"),
            task("task_complete", "abort", "2026-09-16T09:10:00Z"),
        ],
    );
    assert_eq!(parsed.completed_tasks.len(), 2);
    assert_eq!(
        summary(&parsed).unwrap().longest_running_turn_sec,
        Some(150)
    );
    let open = parse(
        ProviderId::Codex,
        &[task("task_started", "open", "2026-09-01T09:00:00Z")],
    );
    assert_eq!(summary(&open).unwrap().longest_running_turn_sec, None);
}

#[test]
fn codex_explicit_metadata_survives_rewritten_envelope_times() {
    let start = time("2026-09-16T09:00:00Z").timestamp();
    let mut explicit = task("task_complete", "explicit", "2026-09-19T09:00:00Z");
    explicit["payload"]["started_at"] = json!(start);
    explicit["payload"]["completed_at"] = json!(start + 600);
    explicit["payload"]["duration_ms"] = json!(62_900);
    let mut fallback = task("task_complete", "metadata", "2026-09-19T09:00:00Z");
    fallback["payload"]["started_at"] = json!(start);
    fallback["payload"]["completed_at"] = json!(start + 60);
    let parsed = parse(ProviderId::Codex, &[explicit, fallback]);
    let result = summary(&parsed).unwrap();
    assert_eq!(result.longest_running_turn_sec, Some(62));
    assert_eq!(result.current_streak_days, Some(1));
}

#[test]
fn fork_ownership_and_subagent_boundaries_apply_to_completed_tasks() {
    let meta = json!({"type":"session_meta","timestamp":"2026-09-16T09:00:00Z","payload":{
        "id":"fork","forked_from_id":"parent","subagent_history_start_ordinal":10,"source":"vscode"
    }});
    let mut inherited = task("task_complete", "parent-task", "2026-09-16T09:00:00Z");
    inherited["ordinal"] = json!(9);
    inherited["payload"]["duration_ms"] = json!(9_999_000);
    let mut own = task("task_complete", "own-task", "2026-09-16T09:02:00Z");
    own["ordinal"] = json!(11);
    own["payload"]["duration_ms"] = json!(60_000);
    let parsed = parse(
        ProviderId::Codex,
        &[meta.clone(), inherited.clone(), own.clone()],
    );
    assert_eq!(parsed.completed_tasks.len(), 1);
    assert_eq!(summary(&parsed).unwrap().longest_running_turn_sec, Some(60));
    let mut child = meta;
    child["payload"]["source"] = json!({"subagent":{"thread_spawn":{}}});
    let parsed = parse(ProviderId::Codex, &[child, inherited, own]);
    assert!(parsed.completed_tasks.is_empty());
    assert!(summary(&parsed).is_none());
}

#[test]
fn fork_timestamp_boundary_uses_original_completion_and_late_ordinal_replays_activity() {
    let mut meta = json!({"type":"session_meta","timestamp":"2026-09-16T09:00:00Z","payload":{
        "id":"fork","forked_from_id":"parent","source":"vscode"
    }});
    let mut inherited = task("task_complete", "parent-task", "2026-09-16T10:00:00Z");
    inherited["payload"]["completed_at"] = json!(time("2026-09-16T08:00:00Z").timestamp());
    inherited["payload"]["duration_ms"] = json!(9_999_000);
    let parsed = parse(ProviderId::Codex, &[meta.clone(), inherited.clone()]);
    assert!(parsed.completed_tasks.is_empty());

    // A later authoritative ordinal must erase even already-parsed timing facts.
    meta["payload"]["forked_from_id"] = Value::Null;
    let mut owned = task("task_complete", "own-task", "2026-09-16T10:00:00Z");
    owned["ordinal"] = json!(11);
    owned["payload"]["duration_ms"] = json!(60_000);
    inherited["ordinal"] = json!(9);
    let mut late_meta = meta.clone();
    late_meta["payload"]["subagent_history_start_ordinal"] = json!(10);
    let parsed = parse(ProviderId::Codex, &[meta, inherited, late_meta, owned]);
    assert_eq!(parsed.completed_tasks.len(), 1);
    assert_eq!(summary(&parsed).unwrap().longest_running_turn_sec, Some(60));
}

#[test]
fn claude_only_uses_explicit_turn_duration_and_real_human_activity() {
    let parsed = parse(
        ProviderId::Claude,
        &[
            json!({"type":"user","timestamp":"2026-09-01T09:00:00Z","uuid":"meta","isMeta":true,"message":{"content":"metadata"}}),
            json!({"type":"user","timestamp":"2026-09-16T09:00:00Z","uuid":"human","message":{"content":"hello"}}),
            json!({"type":"system","subtype":"turn_duration","timestamp":"2026-09-16T09:01:00Z","uuid":"duration","durationMs":62_990}),
            json!({"type":"system","subtype":"turn_duration","timestamp":"2026-09-16T09:02:00Z","uuid":"child-duration","durationMs":900_000,"isSidechain":true}),
            json!({"type":"system","subtype":"turn_duration","timestamp":"2026-09-16T09:02:00Z","uuid":"bad-duration","durationMs":-1}),
        ],
    );
    assert_eq!(parsed.turns.len(), 1);
    assert_eq!(parsed.completed_tasks.len(), 1);
    assert_eq!(summary(&parsed).unwrap().longest_running_turn_sec, Some(62));
}

#[test]
fn completed_activity_is_persisted_and_open_pairing_survives_incremental_restart() {
    let directory = tempdir().unwrap();
    let logs = directory.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let path = logs.join("session.jsonl");
    let windows = calendar_windows(time("2026-09-17T12:00:00Z")).unwrap();
    let data = directory.path().join("data");
    fs::write(
        &path,
        records(&[task("task_started", "durable", "2026-09-16T09:00:00Z")]),
    )
    .unwrap();
    let mut cache = FileCache::default();
    let first = collect_persisted(
        ProviderId::Codex,
        std::slice::from_ref(&logs),
        &windows,
        &data,
        &mut cache,
    )
    .unwrap();
    assert_eq!(first.activity.unwrap().longest_running_turn_sec, None);
    drop(cache);
    let completion = task("task_complete", "durable", "2026-09-16T09:03:00Z");
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(records(&[completion]).as_bytes())
        .unwrap();
    let mut cache = FileCache::default();
    let result = collect_persisted(
        ProviderId::Codex,
        std::slice::from_ref(&logs),
        &windows,
        &data,
        &mut cache,
    )
    .unwrap();
    assert_eq!(result.activity.unwrap().longest_running_turn_sec, Some(180));
    drop(cache);
    fs::copy(&path, logs.join("archived-copy.jsonl")).unwrap();
    let result = collect_persisted(
        ProviderId::Codex,
        &[logs],
        &windows,
        &data,
        &mut FileCache::default(),
    )
    .unwrap();
    let activity = result.activity.unwrap();
    assert_eq!(activity.longest_running_turn_sec, Some(180));
    assert_eq!(activity.current_streak_days, Some(1));
    assert_eq!(activity.longest_streak_days, Some(1));
    assert_eq!(
        read_cached(ProviderId::Codex, &data)
            .unwrap()
            .unwrap()
            .activity
            .unwrap()
            .longest_running_turn_sec,
        Some(180)
    );
}

#[test]
fn old_summaries_read_and_old_parser_cache_rebuilds_activity() {
    let old: TokenStatistics = serde_json::from_value(json!({
        "status":"ready","message":null,"periods":[],"updatedAt":"2026-09-16T09:00:00Z"
    }))
    .unwrap();
    assert!(old.activity.is_none());
    for provider in [ProviderId::Codex, ProviderId::Claude] {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let path = logs.join("session.jsonl");
        let completion = if provider == ProviderId::Codex {
            json!({"type":"event_msg","timestamp":"2026-09-16T09:00:00Z","payload":{"type":"task_complete","turn_id":"completed","duration_ms":60000}})
        } else {
            json!({"type":"system","subtype":"turn_duration","timestamp":"2026-09-16T09:00:00Z","uuid":"completed","durationMs":60000})
        };
        fs::write(&path, records(&[completion])).unwrap();
        let data = directory.path().join("data");
        let windows = calendar_windows(time("2026-09-17T12:00:00Z")).unwrap();
        let mut cache = FileCache::default();
        collect_persisted(
            provider,
            std::slice::from_ref(&logs),
            &windows,
            &data,
            &mut cache,
        )
        .unwrap();
        let entry = cache.files.get_mut(&path).unwrap();
        entry.parser_version = parser_version(provider) - 1;
        entry.parsed.completed_tasks.clear();
        cache
            .store
            .as_ref()
            .unwrap()
            .save(&path, &serde_json::to_vec(entry).unwrap())
            .unwrap();
        drop(cache);
        let result = collect_persisted(
            provider,
            &[logs],
            &windows,
            &data,
            &mut FileCache::default(),
        )
        .unwrap();
        assert_eq!(result.activity.unwrap().longest_running_turn_sec, Some(60));
    }
}
