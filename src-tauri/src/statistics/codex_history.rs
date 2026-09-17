//! CodexBar-compatible, content-free Codex / pi-family usage normalization.
//! Only complete JSON values advance the parser checkpoint. The persisted state never contains
//! chat text, tool arguments, environment values, or credentials.
use super::{iso, Event, ParsedFile, ProviderId, RawUsage, Turn, Usage};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ForkBaseline {
    pub parent: String,
    pub cutoff: DateTime<Utc>,
    pub total: Usage,
    pub last: Option<Usage>,
    pub previous: Option<Usage>,
}
impl ForkBaseline {
    pub fn adjust(&self, baseline: Usage) -> Usage {
        fn subtract(value: Usage, baseline: Usage) -> Usage {
            Usage {
                input: value.input.saturating_sub(baseline.input),
                cached: value.cached.saturating_sub(baseline.cached),
                cache_write: value.cache_write.saturating_sub(baseline.cache_write),
                output: value.output.saturating_sub(baseline.output),
                ..Usage::default()
            }
        }
        let current = subtract(self.total, baseline);
        self.previous
            .map(|previous| {
                current
                    .difference(subtract(previous, baseline))
                    .or(self.last)
                    .unwrap_or_default()
            })
            .unwrap_or(current)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Snapshot {
    total: Usage,
    previous: Option<Usage>,
    last: Option<Usage>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct ParserState {
    pub offset: u64,
    pub unterminated: bool,
    line_number: u64,
    model: Option<String>,
    turn_models: HashMap<String, Option<String>>,
    current_turn: String,
    previous: Option<Usage>,
    seen_totals: HashSet<Usage>,
    has_turn_format: bool,
    subagent: bool,
    saw_session_meta: bool,
    pub session_id: Option<String>,
    parent: Option<String>,
    fork_time: Option<DateTime<Utc>>,
    owned_ordinal: Option<u64>,
    #[serde(skip)]
    requires_replay: bool,
    owned_started: bool,
    local_baseline: bool,
    marker_event_count: Option<usize>,
    pub snapshots: Vec<(DateTime<Utc>, Usage)>,
    pi_provider: Option<String>,
    pi_model: Option<String>,
    pi_session: Option<String>,
    #[serde(default)]
    pending_tasks: HashMap<String, DateTime<Utc>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Timestamp {
    Text(String),
    Number(f64),
}
impl Timestamp {
    fn utc(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Text(text) => DateTime::parse_from_rfc3339(text)
                .ok()
                .map(|time| time.with_timezone(&Utc))
                .or_else(|| text.parse::<f64>().ok().and_then(numeric_time)),
            Self::Number(value) => numeric_time(*value),
        }
    }
}
fn numeric_time(value: f64) -> Option<DateTime<Utc>> {
    if !value.is_finite() {
        return None;
    }
    let milliseconds = if value > 1_000_000_000_000.0 {
        value
    } else {
        value * 1000.0
    };
    DateTime::from_timestamp_millis(milliseconds as i64)
}

#[derive(Default, Deserialize)]
struct Record {
    #[serde(rename = "type", default)]
    kind: String,
    ordinal: Option<u64>,
    timestamp: Option<Timestamp>,
    id: Option<String>,
    #[serde(alias = "sessionId")]
    session_id: Option<String>,
    model: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
    provider: Option<String>,
    #[serde(default)]
    payload: Payload,
    message: Option<PiMessage>,
}
#[derive(Default, Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: Option<String>,
    id: Option<String>,
    #[serde(alias = "sessionId")]
    session_id: Option<String>,
    #[serde(
        alias = "forkedFromId",
        alias = "parent_session_id",
        alias = "parentSessionId"
    )]
    forked_from_id: Option<String>,
    timestamp: Option<Timestamp>,
    source: Option<super::SessionSource>,
    subagent_history_start_ordinal: Option<u64>,
    model: Option<String>,
    model_name: Option<String>,
    #[serde(alias = "turnId")]
    turn_id: Option<String>,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    duration_ms: Option<f64>,
    trigger_turn: Option<bool>,
    usage: Option<RawUsage>,
    response_id: Option<String>,
    thread_token_usage: Option<RawUsage>,
    info: Option<Info>,
}
#[derive(Default, Deserialize)]
struct Info {
    model: Option<String>,
    model_name: Option<String>,
    total_token_usage: Option<RawUsage>,
    last_token_usage: Option<RawUsage>,
}
#[derive(Default, Deserialize)]
struct PiMessage {
    role: Option<String>,
    model: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
    provider: Option<String>,
    timestamp: Option<Timestamp>,
    usage: Option<PiUsage>,
}
#[derive(Default, Deserialize)]
struct PiUsage {
    #[serde(
        default,
        alias = "inputTokens",
        alias = "input_tokens",
        alias = "promptTokens",
        alias = "prompt_tokens"
    )]
    input: u64,
    #[serde(
        default,
        rename = "cacheRead",
        alias = "cacheReadTokens",
        alias = "cache_read",
        alias = "cache_read_tokens",
        alias = "cacheReadInputTokens",
        alias = "cache_read_input_tokens"
    )]
    cached: u64,
    #[serde(
        default,
        rename = "cacheWrite",
        alias = "cacheWriteTokens",
        alias = "cache_write",
        alias = "cache_write_tokens",
        alias = "cacheCreationTokens",
        alias = "cache_creation_tokens",
        alias = "cacheCreationInputTokens",
        alias = "cache_creation_input_tokens"
    )]
    write: u64,
    #[serde(
        default,
        alias = "outputTokens",
        alias = "output_tokens",
        alias = "completionTokens",
        alias = "completion_tokens"
    )]
    output: u64,
}

pub(super) fn additional_roots(home: &Path) -> Vec<PathBuf> {
    // CodexBar's PiSessionCostScanner uses these two roots, without a date filter here because
    // AgentBar exposes all-time history. A native CODEX_HOME does not relocate pi / OMP logs.
    vec![
        home.join(".pi/agent/sessions"),
        home.join(".omp/agent/sessions"),
    ]
}
fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
fn model_evidence<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    values.into_iter().find_map(clean)
}
fn normalized(raw: RawUsage) -> Usage {
    raw.normalize(ProviderId::Codex)
}

pub(super) fn parse_append(
    reader: &mut (impl BufRead + Seek),
    path: &Path,
    parsed: &mut ParsedFile,
) -> std::io::Result<()> {
    parse_pass(reader, path, parsed)?;
    if let Some(state) = parsed
        .codex_state
        .as_ref()
        .filter(|state| state.requires_replay)
    {
        // A later same-leaf metadata record can establish the ownership boundary after its
        // prefix was already cached. Replay once with the authoritative metadata in place;
        // retaining only metadata here avoids buffering any conversation text.
        let seed = ParserState {
            current_turn: format!("file:{}", path.display()),
            saw_session_meta: true,
            session_id: state.session_id.clone(),
            parent: state.parent.clone(),
            fork_time: state.fork_time,
            subagent: state.subagent,
            owned_ordinal: state.owned_ordinal,
            ..ParserState::default()
        };
        *parsed = ParsedFile {
            codex_state: Some(seed),
            ..ParsedFile::default()
        };
        reader.seek(SeekFrom::Start(0))?;
        parse_pass(reader, path, parsed)?;
    }
    Ok(())
}

fn parse_pass(
    reader: &mut impl BufRead,
    path: &Path,
    parsed: &mut ParsedFile,
) -> std::io::Result<()> {
    let mut state = parsed.codex_state.take().unwrap_or_else(|| ParserState {
        current_turn: format!("file:{}", path.display()),
        ..ParserState::default()
    });
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            break;
        }
        let terminated = line.ends_with('\n');
        if line.trim().is_empty() {
            state.offset += read as u64;
            state.line_number += 1;
            continue;
        }
        let record: Record = match serde_json::from_str(&line) {
            Ok(record) => record,
            Err(_) if !terminated => break,
            Err(_) => {
                parsed.malformed = true;
                state.offset += read as u64;
                state.line_number += 1;
                continue;
            }
        };
        state.line_number += 1;
        state.offset += read as u64;
        state.unterminated = !terminated;
        handle_record(record, &mut state, parsed, path);
    }
    if state.owned_ordinal.is_none() && !state.local_baseline {
        if let Some((parent, cutoff)) = state.parent.as_ref().zip(state.fork_time) {
            for event in &mut parsed.events {
                if let Some(snapshot) = &event.codex_snapshot {
                    event.fork_baseline = Some(ForkBaseline {
                        parent: parent.clone(),
                        cutoff,
                        total: snapshot.total,
                        previous: snapshot.previous,
                        last: snapshot.last,
                    });
                }
            }
        }
    }
    parsed.first_usage = parsed.events.iter().map(|event| event.timestamp).min();
    // Recompute this small derived view so appends containing task_started can establish the
    // turn format without leaving historical "unknown" markers behind.
    parsed.unknown_turns.clear();
    if !state.has_turn_format && !state.subagent {
        parsed
            .unknown_turns
            .extend(parsed.events.iter().map(|event| event.timestamp));
    }
    // Preserve legacy records before modern ones for future-time filtering during aggregation.
    parsed.events.retain(|event| {
        event
            .legacy_turn
            .as_ref()
            .and_then(|turn| parsed.modern_turns.get(turn))
            .is_none_or(|time| *time > event.timestamp)
    });
    parsed.codex_state = Some(state);
    Ok(())
}

fn handle_record(record: Record, state: &mut ParserState, parsed: &mut ParsedFile, path: &Path) {
    if matches!(record.kind.as_str(), "session" | "model_change" | "message") {
        handle_pi(record, state, parsed, path);
        return;
    }
    let payload = record.payload;
    if record.kind == "session_meta" {
        let id = clean(payload.id.as_deref())
            .or_else(|| clean(payload.session_id.as_deref()))
            .or_else(|| clean(record.id.as_deref()))
            .or_else(|| clean(record.session_id.as_deref()));
        if !state.saw_session_meta {
            state.saw_session_meta = true;
            state.session_id = id;
            state.subagent = payload
                .source
                .as_ref()
                .is_some_and(super::SessionSource::is_subagent);
            state.parent = clean(payload.forked_from_id.as_deref());
            state.fork_time = payload
                .timestamp
                .as_ref()
                .and_then(Timestamp::utc)
                .or_else(|| record.timestamp.as_ref().and_then(Timestamp::utc));
            state.owned_ordinal = payload.subagent_history_start_ordinal;
        } else if id.is_some() && id == state.session_id {
            if state.parent.is_none() {
                state.parent = clean(payload.forked_from_id.as_deref());
            }
            if state.owned_ordinal.is_none() {
                state.owned_ordinal = payload.subagent_history_start_ordinal;
                state.requires_replay = state.owned_ordinal.is_some();
            }
        }
        return;
    }
    let timestamp = record.timestamp.as_ref().and_then(Timestamp::utc);
    let activity_timestamp = if record.kind == "event_msg" {
        match payload.kind.as_deref() {
            Some("task_started") => payload.started_at,
            Some("task_complete" | "task_completed" | "turn_aborted") => payload.completed_at,
            _ => None,
        }
        .and_then(|time| DateTime::from_timestamp(time, 0))
    } else {
        None
    };
    let inherited = if let Some(boundary) = state.owned_ordinal {
        record.ordinal.is_none_or(|ordinal| ordinal < boundary)
    } else {
        state.parent.is_some()
            && activity_timestamp
                .or(timestamp)
                .zip(state.fork_time)
                .is_some_and(|(time, fork)| time < fork)
    };
    if inherited {
        // Copied history seeds a baseline, but cannot supply model/turn authority to the child.
        if let Some(total) = payload
            .thread_token_usage
            .or_else(|| payload.info.and_then(|info| info.total_token_usage))
        {
            state.previous = Some(normalized(total));
            state.local_baseline = true;
        }
        return;
    }
    if !state.owned_started {
        state.owned_started = true;
        if state.owned_ordinal.is_some() {
            state.model = None;
            state.turn_models.clear();
        }
    }
    if record.kind == "inter_agent_communication_metadata"
        && payload.trigger_turn == Some(true)
        && state.subagent
        && state.owned_ordinal.is_none()
    {
        state.marker_event_count = Some(parsed.events.len());
        return;
    }
    if record.kind == "turn_context" {
        let info = payload.info.as_ref();
        let fields = [
            payload.model.as_deref(),
            payload.model_name.as_deref(),
            info.and_then(|info| info.model.as_deref()),
            info.and_then(|info| info.model_name.as_deref()),
        ];
        if fields.iter().any(Option::is_some) {
            state.model = model_evidence(fields);
        }
        if let Some(turn) = clean(payload.turn_id.as_deref()) {
            state.current_turn = turn.clone();
            state.turn_models.insert(turn, state.model.clone());
        }
        return;
    }
    let is_started = record.kind == "event_msg" && payload.kind.as_deref() == Some("task_started");
    let is_completed = record.kind == "event_msg"
        && matches!(
            payload.kind.as_deref(),
            Some("task_complete" | "task_completed")
        );
    let is_aborted = record.kind == "event_msg" && payload.kind.as_deref() == Some("turn_aborted");
    let is_legacy = record.kind == "event_msg" && payload.kind.as_deref() == Some("token_count");
    let is_modern = record.kind == "token_usage_record";
    if is_completed || is_aborted {
        if state.subagent {
            return;
        }
        let Some(id) = clean(payload.turn_id.as_deref()) else {
            return;
        };
        let pending = state.pending_tasks.remove(&id);
        if is_aborted {
            return;
        }
        // Forks can rewrite the envelope timestamp. Explicit task metadata preserves the
        // actual execution instants; use paired envelopes only for older log formats.
        let started_at = payload
            .started_at
            .and_then(|time| DateTime::from_timestamp(time, 0))
            .or(pending);
        let Some(completed_at) = payload
            .completed_at
            .and_then(|time| DateTime::from_timestamp(time, 0))
            .or(timestamp)
        else {
            return;
        };
        if let Some(task) = super::activity::CompletedTask::new(
            format!("codex-task:{id}"),
            started_at,
            completed_at,
            payload.duration_ms,
        ) {
            parsed.completed_tasks.push(task);
        }
        return;
    }
    if !is_started && !is_legacy && !is_modern {
        return;
    }
    let Some(timestamp) = timestamp else {
        parsed.malformed = true;
        return;
    };
    if is_started {
        state.has_turn_format = true;
        if !state.subagent {
            if let Some(id) = clean(payload.turn_id.as_deref()) {
                state.current_turn = id.clone();
                let started_at = payload
                    .started_at
                    .and_then(|time| DateTime::from_timestamp(time, 0))
                    .unwrap_or(timestamp);
                // Repeated/copied start notifications must not restart a running task.
                state.pending_tasks.entry(id.clone()).or_insert(started_at);
                parsed.turns.push(Turn {
                    key: format!("codex-turn:{id}"),
                    timestamp: started_at,
                });
            } else {
                state.has_turn_format = false;
            }
        }
        return;
    }
    let turn = clean(payload.turn_id.as_deref()).unwrap_or_else(|| state.current_turn.clone());
    let context_model = state
        .turn_models
        .get(&turn)
        .cloned()
        .unwrap_or_else(|| state.model.clone());
    if is_modern {
        let Some(raw) = payload.usage else {
            return;
        };
        let usage = normalized(raw);
        parsed
            .modern_turns
            .entry(turn)
            .and_modify(|time| *time = (*time).min(timestamp))
            .or_insert(timestamp);
        if let Some(total) = payload.thread_token_usage.map(normalized) {
            state.previous = Some(total);
            state.snapshots.push((timestamp, total));
        }
        add_event(
            parsed,
            Event {
                key: clean(payload.response_id.as_deref())
                    .map(|id| format!("codex-response:{id}"))
                    .unwrap_or_else(|| {
                        format!(
                            "codex-record:{}:{}:{usage:?}",
                            state.session_id.as_deref().unwrap_or("unknown"),
                            iso(timestamp)
                        )
                    }),
                timestamp,
                model: context_model.or_else(|| clean(payload.model.as_deref())),
                usage,
                request_count: Some(1),
                legacy_turn: None,
                fork_baseline: None,
                codex_snapshot: None,
            },
        );
        return;
    }
    let Some(info) = payload.info else {
        return;
    };
    let total = info.total_token_usage.map(normalized);
    let last = info.last_token_usage.map(normalized);
    if total.is_none() && last.is_none() {
        return;
    }
    if let Some(total) = total {
        state.snapshots.push((timestamp, total));
    }
    if let Some(prefix_count) = state.marker_event_count {
        if total
            .zip(last)
            .and_then(|(total, last)| total.difference(last))
            == state.previous
            && total != state.previous
        {
            parsed.events.drain(..prefix_count);
            parsed.modern_turns.clear();
            state.local_baseline = true;
            state.marker_event_count = None;
        }
    }
    if parsed
        .modern_turns
        .get(&turn)
        .is_some_and(|time| *time <= timestamp)
    {
        state.previous = total.or(state.previous);
        return;
    }
    if total.zip(state.previous).is_some_and(|(total, previous)| {
        total.difference(previous).is_none() && last == Some(total)
    }) {
        // A confirmed cumulative reset starts a new counter epoch; later requests can reach
        // totals already observed in the old epoch and must still count.
        state.seen_totals.clear();
    }
    if total.is_some_and(|total| state.seen_totals.contains(&total)) {
        return;
    }
    if let Some(total) = total {
        state.seen_totals.insert(total);
    }
    let snapshot = total.map(|total| Snapshot {
        total,
        previous: state.previous,
        last,
    });
    let usage = match (total, state.previous) {
        (Some(total), Some(previous)) => {
            // A repeated older cumulative row cannot reset an already-counted watermark.
            // Genuine reset rows have a total equal to their latest request.
            if total.difference(previous).is_none() && last != Some(total) {
                return;
            }
            total.difference(previous).or(last).unwrap_or(total)
        }
        (Some(total), None) => last.unwrap_or(total),
        (None, _) => last.unwrap(),
    };
    state.previous = total.or(state.previous);
    if usage.total() == 0 {
        return;
    }
    let fallback_model = model_evidence([
        info.model.as_deref(),
        info.model_name.as_deref(),
        payload.model.as_deref(),
        record.model.as_deref(),
    ]);
    add_event(
        parsed,
        Event {
            key: format!(
                "codex-snapshot:{}:{}:{total:?}:{last:?}",
                state.session_id.as_deref().unwrap_or("unknown"),
                iso(timestamp)
            ),
            timestamp,
            model: context_model.or(fallback_model),
            usage,
            request_count: (last == Some(usage)).then_some(1),
            legacy_turn: Some(turn),
            fork_baseline: None,
            codex_snapshot: snapshot,
        },
    );
}

fn add_event(parsed: &mut ParsedFile, event: Event) {
    parsed.first_usage = Some(
        parsed
            .first_usage
            .unwrap_or(event.timestamp)
            .min(event.timestamp),
    );
    parsed.events.push(event);
}
fn handle_pi(record: Record, state: &mut ParserState, parsed: &mut ParsedFile, path: &Path) {
    if record.kind == "session" {
        if state.pi_session.is_none() {
            state.pi_session =
                clean(record.id.as_deref()).or_else(|| clean(record.session_id.as_deref()));
        }
        return;
    }
    if record.kind == "model_change" {
        state.pi_provider =
            clean(record.provider.as_deref()).map(|value| value.to_ascii_lowercase());
        state.pi_model =
            clean(record.model_id.as_deref()).or_else(|| clean(record.model.as_deref()));
        return;
    }
    let Some(message) = record.message else {
        return;
    };
    let explicit = clean(message.provider.as_deref())
        .or_else(|| clean(record.provider.as_deref()))
        .map(|value| value.to_ascii_lowercase());
    let provider = explicit.as_deref().or(state.pi_provider.as_deref());
    if provider != Some("openai-codex") {
        return;
    }
    let Some(timestamp) = record
        .timestamp
        .as_ref()
        .and_then(Timestamp::utc)
        .or_else(|| message.timestamp.as_ref().and_then(Timestamp::utc))
    else {
        if message.usage.is_some() {
            parsed.malformed = true;
        }
        return;
    };
    let identity = match (state.pi_session.as_ref(), clean(record.id.as_deref())) {
        (Some(session), Some(id)) => format!("pi:{session}:{id}"),
        _ => format!("pi-file:{}:{}", path.display(), state.line_number),
    };
    if message.role.as_deref() == Some("user") {
        state.has_turn_format = true;
        parsed.turns.push(Turn {
            key: format!("{identity}:turn"),
            timestamp,
        });
        return;
    }
    if message.role.as_deref() != Some("assistant") {
        return;
    }
    let Some(raw) = message.usage else {
        return;
    };
    let model = model_evidence([
        message.model.as_deref(),
        record.model.as_deref(),
        message.model_id.as_deref(),
        record.model_id.as_deref(),
    ])
    .or_else(|| {
        (state.pi_provider.as_deref() == Some("openai-codex"))
            .then(|| state.pi_model.clone())
            .flatten()
    });
    let usage = Usage {
        input: raw
            .input
            .saturating_add(raw.cached)
            .saturating_add(raw.write),
        cached: raw.cached,
        cache_write: raw.write,
        output: raw.output,
        ..Usage::default()
    };
    if usage.total() == 0 {
        return;
    }
    add_event(
        parsed,
        Event {
            key: identity,
            timestamp,
            model,
            usage,
            request_count: Some(1),
            legacy_turn: None,
            fork_baseline: None,
            codex_snapshot: None,
        },
    );
}
