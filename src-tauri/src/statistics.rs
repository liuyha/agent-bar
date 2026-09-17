//! Local token accounting. Source logs are read-only; normalized usage is persisted separately.
//! Conversation bodies and credentials are never retained.
//!
//! Codex records request usage and also repeats cumulative snapshots; Claude can emit several
//! chunks for one message. Normalize these into unique requests before calendar aggregation.

mod cache;
mod codex_history;
mod pricing;

use std::{
    collections::{HashMap, HashSet},
    env,
    fs::{self, File},
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::models::{ProviderId, ProviderStatus};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenStatistics {
    pub status: ProviderStatus,
    pub message: Option<String>,
    pub periods: Vec<TokenPeriod>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenPeriod {
    pub period: String,
    pub start_at: String,
    pub end_at: String,
    /// Includes cache reads and writes, consistently across both providers.
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub request_count: Option<u64>,
    pub conversation_turns: Option<u64>,
    /// Known-price subtotal. Absent if all nonzero usage is unpriced; `unpriced_tokens` makes
    /// a partial estimate explicit rather than silently treating unknown models as free.
    pub estimated_cost_usd: Option<f64>,
    pub unpriced_tokens: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
struct Usage {
    input: u64,
    cached: u64,
    cache_write: u64,
    output: u64,
    write_5m: u64,
    write_1h: u64,
}

impl Usage {
    fn total(self) -> u64 {
        self.input.saturating_add(self.output)
    }

    fn difference(self, previous: Self) -> Option<Self> {
        Some(Self {
            input: self.input.checked_sub(previous.input)?,
            cached: self.cached.checked_sub(previous.cached)?,
            cache_write: self.cache_write.checked_sub(previous.cache_write)?,
            output: self.output.checked_sub(previous.output)?,
            write_5m: self.write_5m.checked_sub(previous.write_5m)?,
            write_1h: self.write_1h.checked_sub(previous.write_1h)?,
        })
    }

    fn merge_max(&mut self, other: Self) {
        self.input = self.input.max(other.input);
        self.cached = self.cached.max(other.cached);
        self.cache_write = self.cache_write.max(other.cache_write);
        self.output = self.output.max(other.output);
        self.write_5m = self.write_5m.max(other.write_5m);
        self.write_1h = self.write_1h.max(other.write_1h);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Event {
    key: String,
    timestamp: DateTime<Utc>,
    model: Option<String>,
    usage: Usage,
    request_count: Option<u64>,
    legacy_turn: Option<String>,
    #[serde(default)]
    fork_baseline: Option<codex_history::ForkBaseline>,
    #[serde(default)]
    codex_snapshot: Option<codex_history::Snapshot>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Turn {
    key: String,
    timestamp: DateTime<Utc>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct ParsedFile {
    events: Vec<Event>,
    turns: Vec<Turn>,
    unknown_turns: Vec<DateTime<Utc>>,
    modern_turns: HashMap<String, DateTime<Utc>>,
    first_usage: Option<DateTime<Utc>>,
    malformed: bool,
    #[serde(default)]
    codex_state: Option<codex_history::ParserState>,
}

#[derive(Serialize, Deserialize)]
struct CachedFile {
    parser_version: u32,
    modified: SystemTime,
    len: u64,
    parsed: ParsedFile,
    checkpoint: Option<cache::Checkpoint>,
}

#[derive(Default)]
struct FileCache {
    // Retain all normalized history, independently of calendar boundaries and collection time.
    files: HashMap<PathBuf, CachedFile>,
    store: Option<cache::Store>,
    store_path: Option<PathBuf>,
    cache_error: Option<String>,
}

const CODEX_PARSER_VERSION: u32 = 2;
const CLAUDE_PARSER_VERSION: u32 = 1;

fn parser_version(provider: ProviderId) -> u32 {
    match provider {
        ProviderId::Codex => CODEX_PARSER_VERSION,
        ProviderId::Claude => CLAUDE_PARSER_VERSION,
    }
}

static CODEX_CACHE: OnceLock<Mutex<FileCache>> = OnceLock::new();
static CLAUDE_CACHE: OnceLock<Mutex<FileCache>> = OnceLock::new();

fn persisted_file_names(provider: ProviderId) -> (&'static str, &'static str) {
    match provider {
        ProviderId::Codex => ("codex-token-history.sqlite3", "codex-token-statistics.json"),
        ProviderId::Claude => (
            "claude-token-history.sqlite3",
            "claude-token-statistics.json",
        ),
    }
}

/// Read the last saved result without inspecting source logs or updating the history store.
pub fn read_cached(
    provider: ProviderId,
    data_dir: &Path,
) -> Result<Option<TokenStatistics>, String> {
    let (_, summary_name) = persisted_file_names(provider);
    crate::storage::read_json(&data_dir.join(summary_name))
}

/// Synchronous filesystem work: call on a blocking worker, never from the UI event loop.
pub fn collect(provider: ProviderId, cache_dir: Option<&Path>) -> Result<TokenStatistics, String> {
    let local_now = Local::now();
    let windows = calendar_windows(local_now)?;
    let home = crate::user_paths::home_dir();
    let roots = match provider {
        ProviderId::Codex => {
            let root =
                crate::user_paths::config_dir(env::var_os("CODEX_HOME"), home.clone(), ".codex")
                    .ok_or("无法定位本机 Codex 日志目录")?;
            let mut roots = vec![root.join("sessions"), root.join("archived_sessions")];
            if let Some(home) = &home {
                roots.extend(codex_history::additional_roots(home));
            }
            roots
        }
        ProviderId::Claude => {
            let root = crate::user_paths::config_dir(
                env::var_os("CLAUDE_CONFIG_DIR"),
                home.clone(),
                ".claude",
            )
            .ok_or("无法定位本机 Claude 日志目录")?;
            vec![root.join("projects")]
        }
    };
    let cache = match provider {
        ProviderId::Codex => &CODEX_CACHE,
        ProviderId::Claude => &CLAUDE_CACHE,
    };
    let mut cache = cache
        .get_or_init(|| Mutex::new(FileCache::default()))
        .lock()
        .map_err(|_| "本机统计缓存暂不可用，请重启 AgentBar")?;
    let data_dir = cache_dir.ok_or("无法定位 AgentBar 数据目录，Token 统计未保存")?;
    collect_persisted(provider, &roots, &windows, data_dir, &mut cache)
}

fn collect_persisted(
    provider: ProviderId,
    roots: &[PathBuf],
    windows: &[Window],
    data_dir: &Path,
    cache: &mut FileCache,
) -> Result<TokenStatistics, String> {
    crate::storage::ensure_data_dir(data_dir)?;
    let (history_name, summary_name) = persisted_file_names(provider);
    let store_path = data_dir.join(history_name);
    if cache.store_path.as_ref() != Some(&store_path) {
        cache.store = None;
        cache.store_path = Some(store_path.clone());
    }
    if cache.store.is_none() {
        cache.store = Some(cache::Store::open(&store_path)?);
    }
    // Restore persisted records on every refresh, including after a process restart.
    // Reuse unchanged logs only where their identity is reliable; otherwise rescan safely.
    cache.files.clear();
    let statistics = collect_roots(provider, roots, windows, cache);
    if let Some(error) = &cache.cache_error {
        return Err(format!("Token 统计未完整保存，请重试：{error}"));
    }
    let summary_path = data_dir.join(summary_name);
    crate::storage::write_json(&summary_path, &statistics)?;
    crate::storage::read_json(&summary_path)?
        .ok_or_else(|| "已保存的 Token 统计无法读取，请重试".into())
}

struct Window {
    label: &'static str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

fn calendar_windows<Tz: TimeZone>(now: DateTime<Tz>) -> Result<Vec<Window>, String> {
    let today = now.date_naive();
    let monday = today
        .checked_sub_days(Days::new(today.weekday().num_days_from_monday().into()))
        .ok_or("无法计算本周统计时间")?;
    let first = today.with_day(1).ok_or("无法计算本月统计时间")?;
    let first_of_year = today.with_ordinal(1).ok_or("无法计算本年统计时间")?;
    let mut windows = [
        ("day", today),
        ("week", monday),
        ("month", first),
        ("year", first_of_year),
    ]
    .into_iter()
    .map(|(label, date)| {
        Ok(Window {
            label,
            start: day_start(&now.timezone(), date)?,
            end: now.with_timezone(&Utc),
        })
    })
    .collect::<Result<Vec<_>, String>>()?;
    windows.push(Window {
        label: "all",
        // Unbounded internally; the public startAt is the earliest retained valid record.
        start: DateTime::<Utc>::MIN_UTC,
        end: now.with_timezone(&Utc),
    });
    Ok(windows)
}

fn day_start<Tz: TimeZone>(zone: &Tz, date: NaiveDate) -> Result<DateTime<Utc>, String> {
    // A DST transition can make local midnight nonexistent or ambiguous. Choose the first
    // valid local minute, and the earlier occurrence when the clock repeats.
    for minute in 0..24 * 60 {
        let local = date.and_hms_opt(minute / 60, minute % 60, 0).unwrap();
        if let Some(time) = zone.from_local_datetime(&local).earliest() {
            return Ok(time.with_timezone(&Utc));
        }
    }
    Err("无法计算本地日期边界".into())
}

fn collect_roots(
    provider: ProviderId,
    roots: &[PathBuf],
    windows: &[Window],
    cache: &mut FileCache,
) -> TokenStatistics {
    let end = windows[0].end;
    cache.cache_error = None;
    let mut paths = Vec::new();
    let mut read_error = false;
    for root in roots {
        discover_logs(root, &mut paths, &mut read_error);
    }
    paths.sort();
    paths.dedup();
    let current: HashSet<_> = paths.iter().cloned().collect();
    cache.files.retain(|path, _| current.contains(path));
    if !read_error {
        if let Some(store) = &cache.store {
            if let Err(error) = store.retain(&current) {
                cache.cache_error = Some(error);
            }
        }
    }
    for path in paths {
        let Ok(metadata) = fs::metadata(&path) else {
            read_error = true;
            cache.files.remove(&path);
            continue;
        };
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let len = metadata.len();
        if !cache.files.contains_key(&path) {
            if let Some(store) = &cache.store {
                match store.load(&path) {
                    Ok(Some(bytes)) => match serde_json::from_slice::<CachedFile>(&bytes) {
                        Ok(entry) if entry.parser_version == parser_version(provider) => {
                            cache.files.insert(path.clone(), entry);
                        }
                        _ => {}
                    },
                    Ok(None) => {}
                    Err(error) => cache.cache_error = Some(error),
                }
            }
        }
        let reusable = cache.files.get(&path).is_some_and(|entry| {
            if provider == ProviderId::Codex {
                !entry
                    .parsed
                    .codex_state
                    .as_ref()
                    .is_some_and(|state| state.unterminated)
                    && entry.checkpoint.as_ref().is_some_and(|checkpoint| {
                        cache::can_append(&path, checkpoint).unwrap_or(false)
                    })
            } else {
                entry.modified == modified && entry.len == len
            }
        });
        if reusable
            && cache.files.get(&path).is_some_and(|entry| {
                entry.modified == modified
                    && entry.len == len
                    && (provider != ProviderId::Codex
                        || entry
                            .parsed
                            .codex_state
                            .as_ref()
                            .is_some_and(|state| state.offset == len))
            })
        {
            continue;
        }
        let parsed_result = if provider == ProviderId::Codex {
            let prior = if reusable {
                cache.files.remove(&path)
            } else {
                None
            };
            parse_incremental_codex(&path, prior)
        } else {
            parse_file(&path, provider).map(|parsed| CachedFile {
                parser_version: parser_version(provider),
                modified,
                len,
                parsed,
                checkpoint: None,
            })
        };
        match parsed_result {
            Ok(entry) => {
                if let Some(store) = &cache.store {
                    match serde_json::to_vec(&entry) {
                        Ok(bytes) => {
                            if let Err(error) = store.save(&path, &bytes) {
                                cache.cache_error = Some(error);
                            }
                        }
                        Err(error) => cache.cache_error = Some(error.to_string()),
                    }
                }
                cache.files.insert(path, entry);
            }
            Err(_) => {
                read_error = true;
                cache.files.remove(&path);
            }
        }
    }
    let first_usage = cache
        .files
        .values()
        .filter_map(|file| file.parsed.first_usage)
        .filter(|time| *time <= end)
        .min();
    let has_usage = first_usage.is_some();
    let malformed = cache.files.values().any(|file| file.parsed.malformed);
    let mut unique = HashMap::<String, Event>::new();
    let mut turns = HashMap::<&str, DateTime<Utc>>::new();
    let modern_turns: HashSet<_> = cache
        .files
        .values()
        .flat_map(|cached| cached.parsed.modern_turns.iter())
        .filter(|(_, time)| **time <= end)
        .map(|(turn, _)| turn)
        .collect();
    for cached in cache.files.values() {
        for original in &cached.parsed.events {
            let mut adjusted;
            let event = if let Some(fork) = &original.fork_baseline {
                let baseline = cache
                    .files
                    .values()
                    .filter_map(|file| file.parsed.codex_state.as_ref())
                    .filter(|state| state.session_id.as_deref() == Some(&fork.parent))
                    .flat_map(|state| state.snapshots.iter())
                    .filter(|(time, _)| *time <= fork.cutoff)
                    .max_by_key(|(time, _)| *time)
                    .map(|(_, usage)| *usage);
                adjusted = original.clone();
                if let Some(baseline) = baseline {
                    adjusted.usage = fork.adjust(baseline);
                    adjusted.request_count = (fork.last == Some(adjusted.usage)).then_some(1);
                } else {
                    // A missing parent makes the cumulative baseline unprovable. Expose partial
                    // data instead of silently billing inherited totals as child-owned requests.
                    read_error = true;
                    continue;
                }
                if adjusted.usage.total() == 0 {
                    continue;
                }
                &adjusted
            } else {
                original
            };
            // Filter before merging: a future chunk must not inflate a past request.
            if event.timestamp > end {
                continue;
            }
            // A copied/archived log may contain only legacy snapshots of a turn whose
            // request records survive elsewhere. Always prefer the request records.
            if event
                .legacy_turn
                .as_ref()
                .is_some_and(|turn| modern_turns.contains(turn))
            {
                continue;
            }
            unique
                .entry(event.key.clone())
                .and_modify(|existing| {
                    // Claude message chunks and copied logs are cumulative, not new usage.
                    existing.usage.merge_max(event.usage);
                    existing.timestamp = existing.timestamp.min(event.timestamp);
                    if existing.model.is_none() {
                        existing.model = event.model.clone();
                    }
                    if event.request_count.is_none() {
                        existing.request_count = None;
                    }
                })
                .or_insert_with(|| event.clone());
        }
        for turn in &cached.parsed.turns {
            if turn.timestamp > end {
                continue;
            }
            turns
                .entry(&turn.key)
                .and_modify(|time| *time = (*time).min(turn.timestamp))
                .or_insert(turn.timestamp);
        }
    }
    let status = if has_usage {
        ProviderStatus::Ready
    } else if read_error || malformed {
        ProviderStatus::Error
    } else {
        ProviderStatus::Unavailable
    };
    let message = match status {
        ProviderStatus::Unavailable => Some("未找到本机 Token 用量记录；使用后即可统计".into()),
        ProviderStatus::Error => {
            Some("无法读取本机 Token 用量记录，请检查日志目录权限及格式".into())
        }
        ProviderStatus::Ready if read_error || malformed => {
            Some("部分本机日志无法读取或格式不完整，当前统计可能不完整".into())
        }
        ProviderStatus::Ready => None,
    };
    let first_record = first_usage
        .into_iter()
        .chain(turns.values().copied())
        .chain(
            cache
                .files
                .values()
                .flat_map(|cached| cached.parsed.unknown_turns.iter().copied())
                .filter(|time| *time <= end),
        )
        .min()
        .unwrap_or(end);
    let mut periods: Vec<_> = windows
        .iter()
        .map(|window| TokenPeriod {
            period: window.label.into(),
            start_at: iso(if window.label == "all" {
                first_record
            } else {
                window.start
            }),
            end_at: iso(window.end),
            input_tokens: 0,
            cached_input_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            request_count: (status == ProviderStatus::Ready).then_some(0),
            conversation_turns: (status == ProviderStatus::Ready).then_some(0),
            estimated_cost_usd: (status == ProviderStatus::Ready).then_some(0.0),
            unpriced_tokens: 0,
        })
        .collect();
    for event in unique.values() {
        for (window, period) in windows.iter().zip(periods.iter_mut()) {
            if event.timestamp < window.start || event.timestamp > window.end {
                continue;
            }
            let usage = event.usage;
            period.input_tokens = period.input_tokens.saturating_add(usage.input);
            period.cached_input_tokens = period.cached_input_tokens.saturating_add(usage.cached);
            period.cache_write_tokens = period.cache_write_tokens.saturating_add(usage.cache_write);
            period.output_tokens = period.output_tokens.saturating_add(usage.output);
            period.total_tokens = period.input_tokens.saturating_add(period.output_tokens);
            period.request_count = period
                .request_count
                .zip(event.request_count)
                .map(|(count, more)| count.saturating_add(more));
            if let Some(cost) = pricing::estimate(provider, event.model.as_deref(), usage) {
                if let Some(total_cost) = period.estimated_cost_usd.as_mut() {
                    *total_cost += cost;
                }
            } else if usage.total() > 0 {
                period.unpriced_tokens = period.unpriced_tokens.saturating_add(usage.total());
            }
        }
    }
    for (window, period) in windows.iter().zip(periods.iter_mut()) {
        if period.total_tokens > 0 && period.unpriced_tokens == period.total_tokens {
            period.estimated_cost_usd = None;
        }
        if cache.files.values().any(|cached| {
            cached
                .parsed
                .unknown_turns
                .iter()
                .any(|time| *time >= window.start && *time <= window.end)
        }) {
            period.conversation_turns = None;
        } else if status == ProviderStatus::Ready {
            period.conversation_turns = Some(
                turns
                    .values()
                    .filter(|time| **time >= window.start && **time <= window.end)
                    .count() as u64,
            );
        }
    }
    let message = if status == ProviderStatus::Ready
        && periods
            .iter()
            .any(|period| period.request_count.is_none() || period.conversation_turns.is_none())
    {
        Some(match message {
            Some(message) => format!("{message}；部分记录无法还原请求次数或会话轮次"),
            None => "部分记录无法还原请求次数或会话轮次，对应统计暂不可用".into(),
        })
    } else {
        message
    };
    let message = if cache.cache_error.is_some() {
        Some(match message {
            Some(message) => format!("{message}；Token 历史数据未完整保存，请重试"),
            None => "Token 历史数据未完整保存，请重试".into(),
        })
    } else {
        message
    };
    TokenStatistics {
        status,
        message,
        periods,
        updated_at: iso(windows[0].end),
    }
}

fn parse_incremental_codex(path: &Path, prior: Option<CachedFile>) -> std::io::Result<CachedFile> {
    let mut parsed = prior.map(|entry| entry.parsed).unwrap_or_default();
    let offset = parsed.codex_state.as_ref().map_or(0, |state| state.offset);
    let before = cache::checkpoint(path, offset)?;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    codex_history::parse_append(&mut BufReader::new(file), path, &mut parsed)?;
    let offset = parsed.codex_state.as_ref().map_or(0, |state| state.offset);
    // A full scan must also work on platforms where cached-prefix reuse is
    // disabled. Validate this read independently of the reuse policy.
    if !cache::unchanged_prefix(path, &before)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "日志在扫描期间被重写",
        ));
    }
    let checkpoint = cache::checkpoint(path, offset)?;
    let metadata = fs::metadata(path)?;
    Ok(CachedFile {
        parser_version: CODEX_PARSER_VERSION,
        modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        len: metadata.len(),
        parsed,
        checkpoint: Some(checkpoint),
    })
}

fn discover_logs(root: &Path, paths: &mut Vec<PathBuf>, failed: &mut bool) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            *failed = true;
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            *failed = true;
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            *failed = true;
            continue;
        };
        // Do not follow links outside the configured log tree or recurse through link cycles.
        if kind.is_dir() {
            discover_logs(&entry.path(), paths, failed);
        } else if kind.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
        {
            paths.push(entry.path());
        }
    }
}

fn iso(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

// Typed deserialization deliberately discards every conversation/content/credential field.
#[derive(Default, Deserialize)]
struct Record {
    #[serde(rename = "type", default)]
    kind: String,
    timestamp: Option<String>,
    message: Option<Message>,
    uuid: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    #[serde(rename = "isMeta", default)]
    is_meta: bool,
    #[serde(rename = "isCompactSummary", default)]
    is_compact_summary: bool,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: bool,
    #[serde(rename = "isSynthetic", default)]
    is_synthetic: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SessionSource {
    Name(String),
    Details {
        subagent: Option<serde::de::IgnoredAny>,
    },
}

impl SessionSource {
    fn is_subagent(&self) -> bool {
        matches!(self, Self::Details { subagent: Some(_) })
            || matches!(self, Self::Name(name) if name == "subagent")
    }
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<RawUsage>,
    #[serde(default)]
    content: HumanContent,
}

#[derive(Default)]
struct HumanContent(bool);

impl<'de> Deserialize<'de> for HumanContent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ContentVisitor;
        impl<'de> serde::de::Visitor<'de> for ContentVisitor {
            type Value = HumanContent;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("message content")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(HumanContent(!value.trim().is_empty()))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                #[derive(Deserialize)]
                struct Block {
                    #[serde(rename = "type")]
                    kind: String,
                }
                let mut human = false;
                while let Some(block) = sequence.next_element::<Block>()? {
                    human |= matches!(block.kind.as_str(), "text" | "image" | "document");
                }
                Ok(HumanContent(human))
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(HumanContent(false))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(HumanContent(false))
            }
        }
        deserializer.deserialize_any(ContentVisitor)
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
struct RawUsage {
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    #[serde(default)]
    cache_write_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    output_tokens: u64,
    #[serde(default)]
    cache_creation: CacheCreation,
}

#[derive(Clone, Copy, Default, Deserialize)]
struct CacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

impl RawUsage {
    fn normalize(self, provider: ProviderId) -> Usage {
        match provider {
            ProviderId::Codex => Usage {
                input: self.input_tokens,
                cached: self
                    .cached_input_tokens
                    .max(self.cache_read_input_tokens)
                    .min(self.input_tokens),
                cache_write: self.cache_write_input_tokens,
                output: self.output_tokens,
                ..Usage::default()
            },
            ProviderId::Claude => Usage {
                input: self
                    .input_tokens
                    .saturating_add(self.cache_read_input_tokens)
                    .saturating_add(self.cache_creation_input_tokens),
                cached: self.cache_read_input_tokens,
                cache_write: self.cache_creation_input_tokens,
                output: self.output_tokens,
                write_5m: self.cache_creation.ephemeral_5m_input_tokens,
                write_1h: self.cache_creation.ephemeral_1h_input_tokens,
            },
        }
    }
}

fn parse_file(path: &Path, provider: ProviderId) -> std::io::Result<ParsedFile> {
    parse_reader(BufReader::new(File::open(path)?), provider, path)
}

fn parse_reader(
    mut reader: impl BufRead + Seek,
    provider: ProviderId,
    path: &Path,
) -> std::io::Result<ParsedFile> {
    if provider == ProviderId::Codex {
        let mut parsed = ParsedFile::default();
        codex_history::parse_append(&mut reader, path, &mut parsed)?;
        return Ok(parsed);
    }
    let mut parsed = ParsedFile::default();
    let mut has_turn_format = false;
    let mut line = String::new();
    let mut line_number = 0_u64;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        line_number += 1;
        if !((line.contains("\"assistant\"") && line.contains("\"usage\""))
            || line.contains("\"user\""))
        {
            continue;
        }
        let record: Record = match serde_json::from_str(&line) {
            Ok(record) => record,
            Err(_) => {
                parsed.malformed |= line.ends_with('\n');
                continue;
            }
        };
        let Some(timestamp) = record
            .timestamp
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
        else {
            parsed.malformed = true;
            continue;
        };
        if record.kind == "user" {
            has_turn_format = true;
            if !record.is_meta
                && !record.is_compact_summary
                && !record.is_sidechain
                && !record.is_synthetic
                && record
                    .message
                    .as_ref()
                    .is_some_and(|message| message.content.0)
            {
                if let Some(id) = record.uuid {
                    parsed.turns.push(Turn {
                        key: format!("claude-turn:{id}"),
                        timestamp,
                    });
                } else {
                    parsed.unknown_turns.push(timestamp);
                }
            }
            continue;
        }
        if record.kind != "assistant" {
            continue;
        }
        let Some(message) = record.message else {
            continue;
        };
        let Some(raw) = message.usage else {
            continue;
        };
        let request_id = record
            .request_id
            .filter(|id| !id.is_empty())
            .or_else(|| message.id.filter(|id| !id.is_empty()));
        let request_count = request_id.as_ref().map(|_| 1);
        parsed.events.push(Event {
            key: request_id
                .or(record.uuid)
                .map(|id| format!("claude-message:{id}"))
                .unwrap_or_else(|| format!("claude-line:{}:{line_number}", path.display())),
            timestamp,
            model: message.model,
            usage: raw.normalize(provider),
            request_count,
            legacy_turn: None,
            fork_baseline: None,
            codex_snapshot: None,
        });
        parsed.first_usage = Some(parsed.first_usage.unwrap_or(timestamp).min(timestamp));
    }
    if !has_turn_format {
        parsed
            .unknown_turns
            .extend(parsed.events.iter().map(|event| event.timestamp));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use serde_json::{json, Value};
    use std::{io::Cursor, time::Instant};
    use tempfile::tempdir;

    fn time(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn windows() -> Vec<Window> {
        calendar_windows(DateTime::parse_from_rfc3339("2026-09-16T20:00:00+08:00").unwrap())
            .unwrap()
    }

    fn write_log(path: &Path, records: &[Value]) {
        let text = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(path, text).unwrap();
    }

    fn parsed(provider: ProviderId, records: &[Value]) -> ParsedFile {
        let text = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        parse_reader(Cursor::new(text), provider, Path::new("test.jsonl")).unwrap()
    }

    fn usage(input: u64, cached: u64, output: u64) -> Value {
        json!({"input_tokens":input,"cached_input_tokens":cached,"output_tokens":output})
    }

    fn context(model: &str) -> Value {
        json!({"type":"turn_context","timestamp":"2026-09-16T09:00:00Z","payload":{"turn_id":"turn-1","model":model}})
    }

    fn started() -> Value {
        json!({"type":"event_msg","timestamp":"2026-09-16T09:00:00Z","payload":{"type":"task_started","turn_id":"turn-1"}})
    }

    fn legacy(timestamp: &str, total: Value, last: Value) -> Value {
        json!({"type":"event_msg","timestamp":timestamp,"payload":{"type":"token_count","info":{"total_token_usage":total,"last_token_usage":last}}})
    }

    fn modern(timestamp: &str, id: &str, count: Value) -> Value {
        json!({"type":"token_usage_record","timestamp":timestamp,"payload":{"turn_id":"turn-1","response_id":id,"usage":count,"thread_token_usage":count}})
    }

    fn known_turn(provider: ProviderId, timestamp: &str, id: &str) -> Vec<Value> {
        match provider {
            ProviderId::Codex => vec![
                json!({"type":"turn_context","timestamp":timestamp,"payload":{"turn_id":id,"model":"gpt-6-astra"}}),
                json!({"type":"event_msg","timestamp":timestamp,"payload":{"type":"task_started","turn_id":id}}),
                json!({"type":"token_usage_record","timestamp":timestamp,"payload":{"turn_id":id,"response_id":id,"usage":{"input_tokens":100,"cached_input_tokens":40,"cache_write_input_tokens":20,"output_tokens":10}}}),
            ],
            ProviderId::Claude => vec![
                json!({"type":"user","timestamp":timestamp,"uuid":id,"message":{"content":"hello"}}),
                json!({"type":"assistant","timestamp":timestamp,"message":{"id":id,"model":"claude-sonnet-4-6","usage":{"input_tokens":40,"cache_read_input_tokens":40,"cache_creation_input_tokens":20,"output_tokens":10}}}),
            ],
        }
    }

    fn assert_reused_when_supported(
        provider: ProviderId,
        previous: *const Event,
        current: *const Event,
    ) {
        // Codex conservatively rescans on platforms without stable file identity.
        // All platforms still exercise the surrounding accounting assertions.
        if provider != ProviderId::Codex || cfg!(unix) {
            assert_eq!(previous, current);
        }
    }

    #[test]
    fn codex_full_scan_validates_reads_independently_of_cache_reuse() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        write_log(
            &path,
            &known_turn(ProviderId::Codex, "2026-09-16T09:01:00Z", "first"),
        );

        let first = parse_incremental_codex(&path, None)
            .expect("a stable Codex file must support a complete scan on every platform");
        let checkpoint = first.checkpoint.as_ref().unwrap();
        assert!(cache::unchanged_prefix(&path, checkpoint).unwrap());
        #[cfg(not(unix))]
        assert!(!cache::can_append(&path, checkpoint).unwrap());
        assert_eq!(first.parsed.events[0].usage.total(), 110);

        write_log(
            &path,
            &known_turn(ProviderId::Codex, "2026-09-16T09:02:00Z", "replacement"),
        );
        let mut cache = FileCache::default();
        for _ in 0..2 {
            let result = collect_roots(
                ProviderId::Codex,
                &[directory.path().into()],
                &windows(),
                &mut cache,
            );
            assert_eq!(result.status, ProviderStatus::Ready, "{:?}", result.message);
            assert_eq!(result.periods[0].total_tokens, 110);
            assert_eq!(result.periods[0].request_count, Some(1));
        }
    }

    #[test]
    fn calendar_periods_use_local_dates_monday_and_cross_year_boundaries() {
        let now = DateTime::parse_from_rfc3339("2027-01-01T00:30:00+08:00").unwrap();
        let result = calendar_windows(now).unwrap();
        assert_eq!(result[0].start, time("2026-12-31T16:00:00Z"));
        assert_eq!(result[1].start, time("2026-12-27T16:00:00Z"));
        assert_eq!(result[2].start, time("2026-12-31T16:00:00Z"));
        assert_eq!(result[3].start, time("2026-12-31T16:00:00Z"));
        assert_eq!(
            result.iter().map(|window| window.label).collect::<Vec<_>>(),
            ["day", "week", "month", "year", "all"]
        );
        assert!(result
            .iter()
            .all(|window| window.end == now.with_timezone(&Utc)));
        let west = FixedOffset::west_opt(7 * 3600).unwrap();
        let same_instant = now.with_timezone(&west);
        let result = calendar_windows(same_instant).unwrap();
        assert_eq!(result[0].start, time("2026-12-31T07:00:00Z"));
        assert_eq!(result[2].start, time("2026-12-01T07:00:00Z"));
        assert_eq!(result[3].start, time("2026-01-01T07:00:00Z"));
    }

    #[test]
    fn year_and_all_include_old_archived_logs_deduplicate_and_exclude_future_metrics() {
        for provider in [ProviderId::Codex, ProviderId::Claude] {
            let directory = tempdir().unwrap();
            let sessions = directory.path().join("sessions");
            let archives = directory.path().join("archived_sessions");
            fs::create_dir(&sessions).unwrap();
            fs::create_dir(&archives).unwrap();
            let timestamps = [
                ("2025-12-31T15:59:59.999Z", "previous-year"),
                ("2025-12-31T16:00:00Z", "year-boundary"),
                ("2026-08-01T00:00:00Z", "previous-month"),
                ("2026-09-16T12:00:00Z", "exact-now"),
                ("2026-09-16T12:00:00.001Z", "future"),
            ];
            for (index, (timestamp, id)) in timestamps.iter().enumerate() {
                let records = known_turn(provider, timestamp, id);
                write_log(&archives.join(format!("{index}.jsonl")), &records);
                write_log(&sessions.join(format!("copy-{index}.jsonl")), &records);
            }
            let result = collect_roots(
                provider,
                &[sessions, archives],
                &windows(),
                &mut FileCache::default(),
            );
            assert_eq!(result.status, ProviderStatus::Ready);
            let cost = match provider {
                ProviderId::Codex => 0.00119,
                ProviderId::Claude => 0.000357,
            };
            for (period, count) in result.periods.iter().zip([1, 1, 1, 3, 4]) {
                assert_eq!(period.input_tokens, 100 * count, "{}", period.period);
                assert_eq!(period.cached_input_tokens, 40 * count);
                assert_eq!(period.cache_write_tokens, 20 * count);
                assert_eq!(period.output_tokens, 10 * count);
                assert_eq!(period.total_tokens, 110 * count);
                assert_eq!(period.request_count, Some(count));
                assert_eq!(period.conversation_turns, Some(count));
                assert_eq!(period.unpriced_tokens, 0);
                assert!((period.estimated_cost_usd.unwrap() - cost * count as f64).abs() < 1e-9);
                assert_eq!(period.end_at, "2026-09-16T12:00:00.000Z");
            }
            assert_eq!(result.periods[3].start_at, "2025-12-31T16:00:00.000Z");
            assert_eq!(result.periods[4].start_at, timestamps[0].0);
        }
    }

    #[test]
    fn old_unknown_metrics_propagate_only_to_periods_containing_them() {
        for provider in [ProviderId::Codex, ProviderId::Claude] {
            let directory = tempdir().unwrap();
            let old_path = directory.path().join("old.jsonl");
            let unknown = |timestamp| match provider {
                ProviderId::Codex => vec![legacy(timestamp, usage(300, 40, 30), Value::Null)],
                ProviderId::Claude => vec![
                    json!({"type":"assistant","timestamp":timestamp,"message":{"model":"unknown","usage":{"input_tokens":300,"output_tokens":30}}}),
                ],
            };
            write_log(&old_path, &unknown("2025-12-01T00:00:00Z"));
            let mut cache = FileCache::default();
            let collect = |cache: &mut FileCache| {
                collect_roots(provider, &[directory.path().into()], &windows(), cache)
            };
            let result = collect(&mut cache);
            let all = &result.periods[4];
            assert_eq!(all.total_tokens, 330);
            assert_eq!(all.unpriced_tokens, 330);
            assert_eq!(all.estimated_cost_usd, None);
            assert_eq!(all.request_count, None);
            assert_eq!(all.conversation_turns, None);
            assert_eq!(result.periods[3].total_tokens, 0);
            assert_eq!(result.periods[3].request_count, Some(0));
            assert_eq!(result.periods[3].conversation_turns, Some(0));
            assert_eq!(result.periods[3].estimated_cost_usd, Some(0.0));

            write_log(
                &directory.path().join("current.jsonl"),
                &known_turn(provider, "2026-09-16T09:00:00Z", "known"),
            );
            write_log(
                &directory.path().join("earlier-this-year.jsonl"),
                &unknown("2026-08-01T00:00:00Z"),
            );
            let result = collect(&mut cache);
            let day = &result.periods[0];
            assert_eq!(day.request_count, Some(1));
            assert_eq!(day.conversation_turns, Some(1));
            for (period, unpriced) in result.periods[3..].iter().zip([330, 660]) {
                assert_eq!(period.total_tokens, 110 + unpriced);
                assert_eq!(period.unpriced_tokens, unpriced);
                assert_eq!(period.estimated_cost_usd, day.estimated_cost_usd);
                assert_eq!(period.request_count, None);
                assert_eq!(period.conversation_turns, None);
            }
        }
    }

    #[test]
    fn codex_repeated_snapshots_and_lagging_legacy_totals_are_not_new_requests() {
        let records = [
            context("gpt-6-astra"),
            started(),
            modern("2026-09-16T09:01:00Z", "response-1", usage(100, 40, 10)),
            // A real schema variant has a legacy total smaller than modern thread usage.
            legacy(
                "2026-09-16T09:01:00.001Z",
                usage(90, 30, 10),
                usage(100, 40, 10),
            ),
            legacy(
                "2026-09-16T09:01:10Z",
                usage(90, 30, 10),
                usage(100, 40, 10),
            ),
            // Real logs can delay the mirror by thirty seconds and repeat zero resets.
            legacy(
                "2026-09-16T09:01:30Z",
                usage(80, 20, 10),
                usage(100, 40, 10),
            ),
            legacy("2026-09-16T09:01:31Z", usage(0, 0, 0), usage(0, 0, 0)),
            modern("2026-09-16T09:02:00Z", "response-2", usage(150, 60, 15)),
            legacy(
                "2026-09-16T09:02:00.001Z",
                usage(250, 100, 25),
                usage(150, 60, 15),
            ),
        ];
        let result = parsed(ProviderId::Codex, &records);
        assert_eq!(result.events.len(), 2);
        assert_eq!(
            result
                .events
                .iter()
                .map(|event| event.usage.total())
                .sum::<u64>(),
            275
        );
        assert!(result
            .events
            .iter()
            .all(|event| event.request_count == Some(1)));
        assert_eq!(result.turns.len(), 1);
    }

    #[test]
    fn legacy_counts_deltas_handles_reset_and_marks_missing_requests_unknown() {
        let records = [
            context("gpt-5.6-sol"),
            started(),
            legacy(
                "2026-09-16T09:01:00Z",
                usage(100, 20, 10),
                usage(100, 20, 10),
            ),
            legacy(
                "2026-09-16T09:02:00Z",
                usage(400, 100, 50),
                usage(150, 40, 20),
            ),
            legacy("2026-09-16T09:03:00Z", usage(50, 0, 5), usage(50, 0, 5)),
        ];
        let result = parsed(ProviderId::Codex, &records);
        assert_eq!(result.events.len(), 3);
        assert_eq!(result.events[1].usage.total(), 340);
        assert_eq!(result.events[1].request_count, None);
        assert_eq!(result.events[2].usage.total(), 55);
        assert_eq!(result.events[2].request_count, Some(1));
    }

    #[test]
    fn duplicate_files_fork_copies_and_model_contexts_do_not_double_bill() {
        let directory = tempdir().unwrap();
        let records = [
            context("gpt-6-astra"),
            started(),
            modern("2026-09-16T09:01:00Z", "response-1", usage(100, 40, 10)),
        ];
        write_log(&directory.path().join("original.jsonl"), &records);
        write_log(&directory.path().join("fork-copy.jsonl"), &records);
        let mut cache = FileCache::default();
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut cache,
        );
        assert_eq!(result.status, ProviderStatus::Ready);
        assert_eq!(result.periods[0].total_tokens, 110);
        assert_eq!(result.periods[0].request_count, Some(1));
        assert_eq!(result.periods[0].conversation_turns, Some(1));
        assert!((result.periods[0].estimated_cost_usd.unwrap() - 0.00114).abs() < 1e-9);
        let mut records = records.to_vec();
        records.insert(2, json!({"type":"turn_context","payload":{"turn_id":"unrelated","model":"unknown-model"}}));
        assert_eq!(
            parsed(ProviderId::Codex, &records).events[0]
                .model
                .as_deref(),
            Some("gpt-6-astra")
        );
    }

    #[test]
    fn subagent_metadata_is_first_record_and_does_not_create_user_turns() {
        let records = [
            json!({"type":"session_meta","payload":{"source":{"subagent":{"thread_spawn":{}}}}}),
            json!({"type":"session_meta","payload":{"source":"vscode"}}),
            context("gpt-6-astra"),
            started(),
            modern("2026-09-16T09:01:00Z", "response-1", usage(100, 40, 10)),
        ];
        let result = parsed(ProviderId::Codex, &records);
        assert_eq!(result.events.len(), 1);
        assert!(result.turns.is_empty());
        assert!(result.unknown_turns.is_empty());
    }

    #[test]
    fn claude_merges_message_chunks_includes_caches_and_excludes_tool_results_and_meta_turns() {
        let directory = tempdir().unwrap();
        let human = |id: &str, content: Value, meta: bool| json!({"type":"user","uuid":id,"timestamp":"2026-09-16T08:00:00Z","isMeta":meta,"message":{"content":content}});
        let message = |output| json!({"type":"assistant","uuid":format!("chunk-{output}"),"timestamp":"2026-09-16T09:01:00Z","message":{"id":"message-1","model":"claude-sonnet-4-6","usage":{"input_tokens":100,"cache_read_input_tokens":50,"cache_creation_input_tokens":20,"output_tokens":output}}});
        let records = [
            human("human-1", json!("hello"), false),
            human("human-1", json!("copied history"), false),
            human(
                "tool-1",
                json!([{"type":"tool_result","content":"result"}]),
                false,
            ),
            human("meta-1", json!("system context"), true),
            message(5),
            message(20),
        ];
        write_log(&directory.path().join("session.jsonl"), &records);
        let result = collect_roots(
            ProviderId::Claude,
            &[directory.path().into()],
            &windows(),
            &mut FileCache::default(),
        );
        let day = &result.periods[0];
        assert_eq!(day.input_tokens, 170);
        assert_eq!(day.cached_input_tokens, 50);
        assert_eq!(day.cache_write_tokens, 20);
        assert_eq!(day.output_tokens, 20);
        assert_eq!(day.total_tokens, 190);
        assert_eq!(day.request_count, Some(1));
        assert_eq!(day.conversation_turns, Some(1));
        assert!((day.estimated_cost_usd.unwrap() - 0.00069).abs() < 1e-9);
    }

    #[test]
    fn missing_logs_are_unavailable_but_old_usage_makes_current_period_zero() {
        let directory = tempdir().unwrap();
        let mut cache = FileCache::default();
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().join("missing")],
            &windows(),
            &mut cache,
        );
        assert_eq!(result.status, ProviderStatus::Unavailable);
        assert_eq!(result.periods[0].estimated_cost_usd, None);
        assert_eq!(result.periods[0].request_count, None);
        write_log(
            &directory.path().join("old.jsonl"),
            &[
                context("gpt-6-astra"),
                modern("2026-08-01T00:00:00Z", "old-response", usage(100, 40, 10)),
            ],
        );
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut cache,
        );
        assert_eq!(result.status, ProviderStatus::Ready);
        assert_eq!(result.periods[0].total_tokens, 0);
        assert_eq!(result.periods[0].estimated_cost_usd, Some(0.0));
        assert_eq!(result.periods[0].request_count, Some(0));
    }

    #[test]
    fn partially_priced_period_keeps_known_subtotal_and_exposes_unpriced_tokens() {
        let directory = tempdir().unwrap();
        write_log(
            &directory.path().join("mixed.jsonl"),
            &[
                context("gpt-6-astra"),
                started(),
                modern("2026-09-16T09:01:00Z", "known-price", usage(100, 40, 10)),
                context("codex-auto-review"),
                modern("2026-09-16T09:02:00Z", "unknown-price", usage(200, 0, 20)),
            ],
        );
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut FileCache::default(),
        );
        let day = &result.periods[0];
        assert_eq!(day.total_tokens, 330);
        assert_eq!(day.unpriced_tokens, 220);
        assert!((day.estimated_cost_usd.unwrap() - 0.00114).abs() < 1e-9);
    }

    #[test]
    fn exact_calendar_boundaries_unknown_prices_and_missing_turns_are_visible() {
        let directory = tempdir().unwrap();
        write_log(
            &directory.path().join("calendar.jsonl"),
            &[
                context("unknown-model"),
                modern("2026-09-15T15:59:59.999Z", "before-day", usage(10, 0, 1)),
                modern("2026-09-15T16:00:00Z", "day-boundary", usage(20, 0, 2)),
                modern("2026-09-01T00:00:00Z", "month-only", usage(30, 0, 3)),
                modern("2026-09-16T12:00:00.001Z", "future", usage(40, 0, 4)),
            ],
        );
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut FileCache::default(),
        );
        assert_eq!(
            result
                .periods
                .iter()
                .map(|period| period.total_tokens)
                .collect::<Vec<_>>(),
            [22, 33, 66, 66, 66]
        );
        assert_eq!(result.periods[0].estimated_cost_usd, None);
        assert_eq!(result.periods[0].unpriced_tokens, 22);
        assert_eq!(result.periods[0].conversation_turns, None);
        assert!(result.message.unwrap().contains("无法还原"));
    }

    #[test]
    fn cache_refreshes_appended_files_and_discards_deleted_files() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        let mut records = vec![
            context("gpt-6-astra"),
            started(),
            modern("2026-09-16T09:01:00Z", "response-1", usage(100, 0, 10)),
        ];
        write_log(&path, &records);
        let mut cache = FileCache::default();
        let collect = |cache: &mut FileCache| {
            collect_roots(
                ProviderId::Codex,
                &[directory.path().into()],
                &windows(),
                cache,
            )
        };
        assert_eq!(collect(&mut cache).periods[0].total_tokens, 110);
        let original_events = cache.files[&path].parsed.events.as_ptr();
        assert_eq!(collect(&mut cache).periods[0].total_tokens, 110);
        assert_reused_when_supported(
            ProviderId::Codex,
            original_events,
            cache.files[&path].parsed.events.as_ptr(),
        );
        records.push(modern(
            "2026-09-16T09:02:00Z",
            "response-2",
            usage(200, 0, 20),
        ));
        write_log(&path, &records);
        assert_eq!(collect(&mut cache).periods[0].total_tokens, 330);
        fs::remove_file(path).unwrap();
        assert_eq!(collect(&mut cache).status, ProviderStatus::Unavailable);
    }

    #[test]
    fn full_history_cache_survives_year_rollover_and_refreshes_edits_and_deletions() {
        let directory = tempdir().unwrap();
        let old_path = directory.path().join("old.jsonl");
        let recent_path = directory.path().join("recent.jsonl");
        let mut old = known_turn(ProviderId::Codex, "2025-08-01T00:00:00Z", "old");
        write_log(&old_path, &old);
        write_log(
            &recent_path,
            &known_turn(ProviderId::Codex, "2026-12-31T15:00:00Z", "recent"),
        );
        let mut cache = FileCache::default();
        let collect = |now: &str, cache: &mut FileCache| {
            collect_roots(
                ProviderId::Codex,
                &[directory.path().into()],
                &calendar_windows(DateTime::parse_from_rfc3339(now).unwrap()).unwrap(),
                cache,
            )
        };
        let result = collect("2026-12-31T23:59:59+08:00", &mut cache);
        assert_eq!(result.periods[3].total_tokens, 110);
        assert_eq!(result.periods[4].total_tokens, 220);
        assert_eq!(result.periods[4].start_at, "2025-08-01T00:00:00.000Z");
        let old_events = cache.files[&old_path].parsed.events.as_ptr();
        let recent_events = cache.files[&recent_path].parsed.events.as_ptr();
        let result = collect("2027-01-01T00:00:01+08:00", &mut cache);
        assert_eq!(result.periods[3].total_tokens, 0);
        assert_eq!(result.periods[4].total_tokens, 220);
        assert_reused_when_supported(
            ProviderId::Codex,
            old_events,
            cache.files[&old_path].parsed.events.as_ptr(),
        );
        assert_reused_when_supported(
            ProviderId::Codex,
            recent_events,
            cache.files[&recent_path].parsed.events.as_ptr(),
        );

        // A rewrite can keep the same length; force a distinct mtime without a flaky sleep.
        let old_len = cache.files[&old_path].len;
        let modified = cache.files[&old_path].modified + std::time::Duration::from_secs(1);
        old[2]["payload"]["usage"]["input_tokens"] = json!(200);
        write_log(&old_path, &old);
        File::options()
            .write(true)
            .open(&old_path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert_eq!(fs::metadata(&old_path).unwrap().len(), old_len);
        let result = collect("2027-01-01T00:00:02+08:00", &mut cache);
        assert_eq!(result.periods[3].total_tokens, 0);
        assert_eq!(result.periods[4].total_tokens, 320);
        assert_reused_when_supported(
            ProviderId::Codex,
            recent_events,
            cache.files[&recent_path].parsed.events.as_ptr(),
        );

        fs::remove_file(&old_path).unwrap();
        let result = collect("2027-01-01T00:00:03+08:00", &mut cache);
        assert_eq!(cache.files.len(), 1);
        assert_eq!(result.periods[4].total_tokens, 110);
        assert_eq!(result.periods[4].start_at, "2026-12-31T15:00:00.000Z");
        fs::remove_file(&recent_path).unwrap();
        let result = collect("2027-01-01T00:00:04+08:00", &mut cache);
        assert!(cache.files.is_empty());
        assert_eq!(result.status, ProviderStatus::Unavailable);
        assert_eq!(result.periods[4].start_at, result.periods[4].end_at);
    }

    #[test]
    fn future_only_logs_become_eligible_and_empty_all_starts_at_now() {
        for provider in [ProviderId::Codex, ProviderId::Claude] {
            let directory = tempdir().unwrap();
            let path = directory.path().join("future.jsonl");
            write_log(
                &path,
                &known_turn(provider, "2026-09-17T00:00:00Z", "future"),
            );
            let mut cache = FileCache::default();
            let result =
                collect_roots(provider, &[directory.path().into()], &windows(), &mut cache);
            assert_eq!(result.status, ProviderStatus::Unavailable);
            assert_eq!(result.periods[4].start_at, result.periods[4].end_at);
            for period in &result.periods {
                assert_eq!(period.total_tokens, 0);
                assert_eq!(period.request_count, None);
                assert_eq!(period.conversation_turns, None);
                assert_eq!(period.estimated_cost_usd, None);
            }
            let events = cache.files[&path].parsed.events.as_ptr();
            let result = collect_roots(
                provider,
                &[directory.path().into()],
                &calendar_windows(time("2026-09-17T00:00:00Z")).unwrap(),
                &mut cache,
            );
            assert_reused_when_supported(
                provider,
                events,
                cache.files[&path].parsed.events.as_ptr(),
            );
            assert_eq!(result.status, ProviderStatus::Ready);
            for period in &result.periods {
                assert_eq!(period.total_tokens, 110);
                assert_eq!(period.request_count, Some(1));
                assert_eq!(period.conversation_turns, Some(1));
            }
            assert_eq!(result.periods[4].start_at, "2026-09-17T00:00:00.000Z");
        }
    }

    #[test]
    fn future_claude_chunks_and_unknown_turns_cannot_change_current_metrics() {
        let directory = tempdir().unwrap();
        let mut records = known_turn(ProviderId::Claude, "2026-09-16T09:00:00Z", "message");
        records.push(json!({"type":"assistant","timestamp":"2026-09-16T12:00:00.001Z","uuid":"message","message":{"model":"unknown","usage":{"input_tokens":1000,"output_tokens":900}}}));
        records.push(json!({"type":"user","timestamp":"2026-09-16T12:00:00.001Z","message":{"content":"future user without id"}}));
        write_log(&directory.path().join("session.jsonl"), &records);
        let result = collect_roots(
            ProviderId::Claude,
            &[directory.path().into()],
            &windows(),
            &mut FileCache::default(),
        );
        for period in &result.periods {
            assert_eq!(period.total_tokens, 110);
            assert_eq!(period.request_count, Some(1));
            assert_eq!(period.conversation_turns, Some(1));
            assert_eq!(period.unpriced_tokens, 0);
            assert!((period.estimated_cost_usd.unwrap() - 0.000357).abs() < 1e-9);
        }
    }

    #[test]
    fn future_modern_codex_record_does_not_suppress_retained_legacy_usage() {
        for separate_files in [false, true] {
            let directory = tempdir().unwrap();
            let mut records = vec![
                context("gpt-6-astra"),
                started(),
                legacy(
                    "2026-09-16T09:01:00Z",
                    usage(100, 40, 10),
                    usage(100, 40, 10),
                ),
            ];
            let future = modern("2026-09-16T12:00:00.001Z", "future", usage(200, 40, 20));
            if separate_files {
                write_log(
                    &directory.path().join("future.jsonl"),
                    &[context("gpt-6-astra"), started(), future],
                );
            } else {
                records.push(future);
            }
            write_log(&directory.path().join("legacy.jsonl"), &records);
            let mut cache = FileCache::default();
            let result = collect_roots(
                ProviderId::Codex,
                &[directory.path().into()],
                &windows(),
                &mut cache,
            );
            for period in &result.periods {
                assert_eq!(period.total_tokens, 110);
                assert_eq!(period.request_count, Some(1));
                assert_eq!(period.conversation_turns, Some(1));
            }
            let result = collect_roots(
                ProviderId::Codex,
                &[directory.path().into()],
                &calendar_windows(time("2026-09-16T12:00:01Z")).unwrap(),
                &mut cache,
            );
            for period in &result.periods {
                assert_eq!(period.total_tokens, 220);
                assert_eq!(period.request_count, Some(1));
            }
        }
    }

    #[test]
    fn truncated_active_line_is_ignored_and_bad_complete_line_reports_partial_data() {
        let text = context("gpt-6-astra").to_string()
            + "\n"
            + &modern("2026-09-16T09:01:00Z", "response-1", usage(100, 0, 10)).to_string()
            + "\n{\"type\":\"token_usage_record\",";
        let result =
            parse_reader(Cursor::new(&text), ProviderId::Codex, Path::new("test")).unwrap();
        assert!(!result.malformed);
        assert_eq!(result.events.len(), 1);
        let result = parse_reader(
            Cursor::new(text + "\n"),
            ProviderId::Codex,
            Path::new("test"),
        )
        .unwrap();
        assert!(result.malformed);
    }

    #[test]
    fn serialized_contract_uses_camel_case_and_null_for_unknown_values() {
        let directory = tempdir().unwrap();
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut FileCache::default(),
        );
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["status"], "unavailable");
        assert!(value["updatedAt"].is_string());
        assert!(value["periods"][0]["requestCount"].is_null());
        assert!(value["periods"][0]["conversationTurns"].is_null());
        assert_eq!(value["periods"][0]["unpricedTokens"], 0);
        let periods = value["periods"].as_array().unwrap();
        assert_eq!(periods.len(), 5);
        for (period, label) in periods.iter().zip(["day", "week", "month", "year", "all"]) {
            assert_eq!(period["period"], label);
            assert!(period["startAt"].is_string());
            assert!(period["endAt"].is_string());
            assert!(period["requestCount"].is_null());
            assert!(period["conversationTurns"].is_null());
            assert!(period["estimatedCostUsd"].is_null());
        }
        assert_eq!(periods[4]["startAt"], periods[4]["endAt"]);
    }

    #[test]
    fn missing_usage_fields_are_not_reported_as_zero_usage_or_a_request() {
        for invalid in [
            json!({}),
            json!({"input_tokens": 100}),
            json!({"output_tokens": 100}),
        ] {
            let result = parsed(
                ProviderId::Codex,
                &[
                    context("gpt-6-astra"),
                    modern("2026-09-16T09:01:00Z", "invalid", invalid.clone()),
                ],
            );
            assert!(result.first_usage.is_none());
            assert!(result.events.is_empty());
            assert!(result.malformed);
            let result = parsed(
                ProviderId::Claude,
                &[
                    json!({"type":"assistant","timestamp":"2026-09-16T09:01:00Z","message":{"id":"invalid","usage":invalid}}),
                ],
            );
            assert!(result.first_usage.is_none());
            assert!(result.events.is_empty());
            assert!(result.malformed);
        }
    }

    #[test]
    fn pure_legacy_zero_resets_are_not_requests() {
        let result = parsed(
            ProviderId::Codex,
            &[
                context("gpt-6-astra"),
                started(),
                legacy("2026-09-16T09:01:00Z", usage(100, 0, 10), usage(100, 0, 10)),
                legacy("2026-09-16T09:01:30Z", usage(0, 0, 0), usage(0, 0, 0)),
            ],
        );
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].request_count, Some(1));
    }

    #[test]
    fn request_records_override_legacy_copies_of_the_same_turn_across_files() {
        let directory = tempdir().unwrap();
        write_log(
            &directory.path().join("modern.jsonl"),
            &[
                context("gpt-6-astra"),
                started(),
                modern("2026-09-16T09:01:00Z", "response-1", usage(100, 0, 10)),
            ],
        );
        write_log(
            &directory.path().join("legacy-copy.jsonl"),
            &[
                context("gpt-6-astra"),
                started(),
                legacy("2026-09-16T09:01:30Z", usage(100, 0, 10), usage(100, 0, 10)),
            ],
        );
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut FileCache::default(),
        );
        assert_eq!(result.periods[0].request_count, Some(1));
        assert_eq!(result.periods[0].total_tokens, 110);
    }

    fn persistent_cache(database: &Path) -> FileCache {
        FileCache {
            store: Some(cache::Store::open(database).unwrap()),
            ..FileCache::default()
        }
    }

    fn append_log(path: &Path, text: &str) {
        use std::io::Write;
        File::options()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    #[test]
    fn cached_statistics_read_saved_results_without_collecting_new_logs() {
        for provider in [ProviderId::Codex, ProviderId::Claude] {
            let directory = tempdir().unwrap();
            let logs = directory.path().join("logs");
            let data = directory.path().join(".agent-bar");
            fs::create_dir(&logs).unwrap();
            write_log(
                &logs.join("first.jsonl"),
                &known_turn(provider, "2026-09-16T09:01:00Z", "first"),
            );
            let mut cache = FileCache::default();
            let saved = collect_persisted(
                provider,
                std::slice::from_ref(&logs),
                &windows(),
                &data,
                &mut cache,
            )
            .unwrap();
            assert_eq!(saved.periods[4].total_tokens, 110);
            let (_, summary_name) = persisted_file_names(provider);
            let summary = data.join(summary_name);
            let saved_bytes = fs::read(&summary).unwrap();
            write_log(
                &logs.join("second.jsonl"),
                &known_turn(provider, "2026-09-16T09:02:00Z", "second"),
            );

            let cached = read_cached(provider, &data).unwrap().unwrap();
            assert_eq!(
                serde_json::to_value(&cached).unwrap(),
                serde_json::to_value(&saved).unwrap()
            );
            assert_eq!(fs::read(&summary).unwrap(), saved_bytes);

            let refreshed = collect_persisted(
                provider,
                std::slice::from_ref(&logs),
                &windows(),
                &data,
                &mut cache,
            )
            .unwrap();
            assert_eq!(refreshed.periods[4].total_tokens, 220);
            assert_eq!(
                read_cached(provider, &data).unwrap().unwrap().periods[4].total_tokens,
                220
            );
        }
    }

    #[test]
    fn cached_statistics_return_none_without_creating_missing_data() {
        let directory = tempdir().unwrap();
        let data = directory.path().join(".agent-bar");
        for provider in [ProviderId::Codex, ProviderId::Claude] {
            assert!(read_cached(provider, &data).unwrap().is_none());
        }
        assert!(!data.exists());
    }

    #[test]
    fn cached_statistics_report_corrupt_summaries_without_modifying_them() {
        let directory = tempdir().unwrap();
        for provider in [ProviderId::Codex, ProviderId::Claude] {
            let (_, summary_name) = persisted_file_names(provider);
            let summary = directory.path().join(summary_name);
            let invalid = b"{incomplete saved statistics";
            fs::write(&summary, invalid).unwrap();
            assert!(read_cached(provider, directory.path())
                .unwrap_err()
                .contains("统计数据格式错误"));
            assert_eq!(fs::read(&summary).unwrap(), invalid);
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn persisted_statistics_restore_both_providers_with_platform_appropriate_reuse() {
        for (provider, name) in [(ProviderId::Codex, "codex"), (ProviderId::Claude, "claude")] {
            let directory = tempdir().unwrap();
            let logs = directory.path().join("logs");
            let data = directory.path().join(".agent-bar");
            fs::create_dir(&logs).unwrap();
            let source = logs.join("session.jsonl");
            write_log(
                &source,
                &known_turn(provider, "2026-09-16T09:01:00Z", "first"),
            );
            let collect = |cache: &mut FileCache| {
                collect_persisted(
                    provider,
                    std::slice::from_ref(&logs),
                    &windows(),
                    &data,
                    cache,
                )
            };
            let mut cache = FileCache::default();
            let result = collect(&mut cache).unwrap();
            assert_eq!(result.periods[4].total_tokens, 110);
            let summary = data.join(format!("{name}-token-statistics.json"));
            let saved: TokenStatistics = crate::storage::read_json(&summary).unwrap().unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                serde_json::to_value(&saved).unwrap()
            );
            let connection =
                rusqlite::Connection::open(data.join(format!("{name}-token-history.sqlite3")))
                    .unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE refresh_writes (path BLOB NOT NULL);
                     CREATE TRIGGER track_reparse BEFORE INSERT ON normalized_usage
                     BEGIN INSERT INTO refresh_writes (path) VALUES (NEW.path); END;",
                )
                .unwrap();
            // A memory-only mutation must not survive the next read from the data directory.
            cache.files.get_mut(&source).unwrap().parsed.events[0]
                .usage
                .input = 99_999;
            assert_eq!(collect(&mut cache).unwrap().periods[4].total_tokens, 110);
            drop(cache);
            assert_eq!(
                collect(&mut FileCache::default()).unwrap().periods[4].total_tokens,
                110
            );
            let writes: i64 = connection
                .query_row("SELECT COUNT(*) FROM refresh_writes", [], |row| row.get(0))
                .unwrap();
            let expected_writes = if provider == ProviderId::Codex && !cfg!(unix) {
                // Without a stable file identity, both refreshes intentionally rescan.
                2
            } else {
                0
            };
            assert_eq!(writes, expected_writes);
            let bytes: Vec<u8> = connection
                .query_row("SELECT parsed FROM normalized_usage", [], |row| row.get(0))
                .unwrap();
            assert!(!String::from_utf8(bytes).unwrap().contains("hello"));
        }
    }

    #[test]
    fn claude_persisted_refresh_tracks_chunks_new_logs_rewrites_and_deletions() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        let data = directory.path().join(".agent-bar");
        fs::create_dir(&logs).unwrap();
        let source = logs.join("session.jsonl");
        write_log(
            &source,
            &known_turn(ProviderId::Claude, "2026-09-16T09:01:00Z", "first"),
        );
        let collect = |cache: &mut FileCache| {
            collect_persisted(
                ProviderId::Claude,
                std::slice::from_ref(&logs),
                &windows(),
                &data,
                cache,
            )
            .unwrap()
        };
        let mut cache = FileCache::default();
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 110);
        let chunk = json!({"type":"assistant","timestamp":"2026-09-16T09:02:00Z",
            "message":{"id":"first","model":"claude-sonnet-4-6","usage":{
                "input_tokens":40,"cache_read_input_tokens":40,
                "cache_creation_input_tokens":20,"output_tokens":30}}});
        append_log(&source, &(chunk.to_string() + "\n"));
        let refreshed = collect(&mut cache);
        assert_eq!(refreshed.periods[4].total_tokens, 130);
        assert_eq!(refreshed.periods[4].request_count, Some(1));
        let second = logs.join("second.jsonl");
        write_log(
            &second,
            &known_turn(ProviderId::Claude, "2026-09-16T09:03:00Z", "second"),
        );
        drop(cache);
        let mut cache = FileCache::default();
        let restarted = collect(&mut cache);
        assert_eq!(restarted.periods[4].total_tokens, 240);
        assert_eq!(restarted.periods[4].request_count, Some(2));
        assert_eq!(restarted.periods[4].conversation_turns, Some(2));
        write_log(
            &source,
            &known_turn(ProviderId::Claude, "2026-09-16T09:04:00Z", "replacement"),
        );
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 220);
        fs::remove_file(&second).unwrap();
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 110);
        drop(cache);
        assert_eq!(
            collect(&mut FileCache::default()).periods[4].total_tokens,
            110
        );
    }

    #[test]
    fn history_write_failure_keeps_prior_summary_and_retries_unsaved_usage() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        let data = directory.path().join(".agent-bar");
        fs::create_dir(&logs).unwrap();
        let source = logs.join("session.jsonl");
        write_log(
            &source,
            &known_turn(ProviderId::Claude, "2026-09-16T09:01:00Z", "first"),
        );
        let collect = |cache: &mut FileCache| {
            collect_persisted(
                ProviderId::Claude,
                std::slice::from_ref(&logs),
                &windows(),
                &data,
                cache,
            )
        };
        let mut cache = FileCache::default();
        collect(&mut cache).unwrap();
        let summary = data.join("claude-token-statistics.json");
        let previous = fs::read(&summary).unwrap();
        let connection =
            rusqlite::Connection::open(data.join("claude-token-history.sqlite3")).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_save BEFORE INSERT ON normalized_usage
                 BEGIN SELECT RAISE(ABORT, 'storage unavailable'); END;",
            )
            .unwrap();
        for record in known_turn(ProviderId::Claude, "2026-09-16T09:02:00Z", "second") {
            append_log(&source, &(record.to_string() + "\n"));
        }
        let error = collect(&mut cache).unwrap_err();
        assert!(error.contains("未完整保存"), "{error}");
        assert_eq!(fs::read(&summary).unwrap(), previous);
        connection.execute_batch("DROP TRIGGER fail_save").unwrap();
        assert_eq!(collect(&mut cache).unwrap().periods[4].total_tokens, 220);
        drop(cache);
        assert_eq!(
            collect(&mut FileCache::default()).unwrap().periods[4].total_tokens,
            220
        );
    }

    #[test]
    fn invalid_data_directory_and_summary_write_failure_are_reported() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let invalid_data = directory.path().join("not-a-directory");
        fs::write(&invalid_data, "occupied").unwrap();
        assert!(collect_persisted(
            ProviderId::Claude,
            std::slice::from_ref(&logs),
            &windows(),
            &invalid_data,
            &mut FileCache::default(),
        )
        .is_err());
        let data = directory.path().join(".agent-bar");
        fs::create_dir_all(data.join("claude-token-statistics.json")).unwrap();
        assert!(collect_persisted(
            ProviderId::Claude,
            &[logs],
            &windows(),
            &data,
            &mut FileCache::default(),
        )
        .is_err());
    }

    #[test]
    fn failed_history_open_can_retry_and_switch_storage_directories() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        let first_data = directory.path().join("first-data");
        let second_data = directory.path().join("second-data");
        fs::create_dir(&logs).unwrap();
        fs::create_dir(&first_data).unwrap();
        write_log(
            &logs.join("session.jsonl"),
            &known_turn(ProviderId::Claude, "2026-09-16T09:01:00Z", "first"),
        );
        let database = first_data.join("claude-token-history.sqlite3");
        fs::write(&database, "damaged database").unwrap();
        let mut cache = FileCache::default();
        let collect = |data: &Path, cache: &mut FileCache| {
            collect_persisted(
                ProviderId::Claude,
                std::slice::from_ref(&logs),
                &windows(),
                data,
                cache,
            )
        };
        assert!(collect(&first_data, &mut cache).is_err());
        assert_eq!(fs::read_to_string(&database).unwrap(), "damaged database");
        fs::remove_file(&database).unwrap();
        assert_eq!(
            collect(&first_data, &mut cache).unwrap().periods[4].total_tokens,
            110
        );
        assert_eq!(
            collect(&second_data, &mut cache).unwrap().periods[4].total_tokens,
            110
        );
        assert!(second_data.join("claude-token-statistics.json").is_file());
        assert!(cache.cache_error.is_none());
    }

    #[test]
    fn codex_incremental_history_survives_restart_partial_lines_and_rewrites() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let path = logs.join("session.jsonl");
        let database = directory.path().join("history.sqlite3");
        write_log(
            &path,
            &[
                context("gpt-6-astra"),
                started(),
                modern("2026-09-16T09:01:00Z", "first", usage(100, 0, 10)),
            ],
        );
        let collect = |cache: &mut FileCache| {
            collect_roots(
                ProviderId::Codex,
                std::slice::from_ref(&logs),
                &windows(),
                cache,
            )
        };
        let mut cache = persistent_cache(&database);
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 110);
        let boundary = cache.files[&path]
            .parsed
            .codex_state
            .as_ref()
            .unwrap()
            .offset;
        let record = modern("2026-09-16T09:02:00Z", "second", usage(200, 0, 20)).to_string();
        append_log(&path, &record[..record.len() / 2]);
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 110);
        assert_eq!(
            cache.files[&path]
                .parsed
                .codex_state
                .as_ref()
                .unwrap()
                .offset,
            boundary
        );
        drop(cache);
        let mut cache = persistent_cache(&database);
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 110);
        append_log(&path, &(record[record.len() / 2..].to_string() + "\n"));
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 330);
        assert_eq!(cache.files[&path].parsed.events.len(), 2);
        drop(cache);
        let mut cache = persistent_cache(&database);
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 330);
        assert_eq!(cache.files[&path].parsed.events.len(), 2);
        write_log(
            &path,
            &[
                context("gpt-6-astra"),
                modern("2026-09-16T09:03:00Z", "replacement", usage(10, 0, 1)),
            ],
        );
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 11);
        drop(cache);
        let mut cache = persistent_cache(&database);
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 11);
    }

    #[test]
    fn complete_unterminated_record_is_counted_then_safely_reparsed_on_append() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        let record = modern("2026-09-16T09:01:00Z", "first", usage(100, 0, 10));
        fs::write(&path, record.to_string()).unwrap();
        let mut cache = FileCache::default();
        let collect = |cache: &mut FileCache| {
            collect_roots(
                ProviderId::Codex,
                &[directory.path().into()],
                &windows(),
                cache,
            )
        };
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 110);
        append_log(
            &path,
            &("\n".to_string()
                + &modern("2026-09-16T09:02:00Z", "second", usage(20, 0, 2)).to_string()
                + "\n"),
        );
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 132);
    }

    #[test]
    fn parser_revision_invalidates_unchanged_persisted_counts() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let path = logs.join("session.jsonl");
        let database = directory.path().join("history.sqlite3");
        write_log(
            &path,
            &[modern("2026-09-16T09:01:00Z", "first", usage(100, 0, 10))],
        );
        let mut cache = persistent_cache(&database);
        collect_roots(
            ProviderId::Codex,
            std::slice::from_ref(&logs),
            &windows(),
            &mut cache,
        );
        let entry = cache.files.get_mut(&path).unwrap();
        entry.parser_version = 0;
        entry.parsed.events[0].usage.input = 99_999;
        cache
            .store
            .as_ref()
            .unwrap()
            .save(&path, &serde_json::to_vec(entry).unwrap())
            .unwrap();
        drop(cache);
        let mut cache = persistent_cache(&database);
        let result = collect_roots(ProviderId::Codex, &[logs], &windows(), &mut cache);
        assert_eq!(result.periods[4].total_tokens, 110);
        assert!(cache.cache_error.is_none());
    }

    fn codex_meta(id: &str, parent: Option<&str>, ordinal: Option<u64>) -> Value {
        json!({"type":"session_meta", "ordinal":0,"timestamp":"2026-09-16T09:00:00Z", "payload":{
            "id":id,"forked_from_id":parent,"subagent_history_start_ordinal":ordinal,
            "source":if parent.is_some() {json!({"subagent":{"thread_spawn":{}}})} else {json!("vscode")}
        }})
    }

    #[test]
    fn explicit_subagent_ordinal_discards_inherited_prefix_across_restarts() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let path = logs.join("child.jsonl");
        let database = directory.path().join("history.sqlite3");
        let mut copied = legacy(
            "2026-09-16T09:01:00Z",
            usage(1070, 915, 110),
            usage(20, 5, 5),
        );
        copied["ordinal"] = json!(208);
        write_log(
            &path,
            &[
                codex_meta("child", Some("absent-parent"), Some(210)),
                context("parent-model"),
                json!({"type":"inter_agent_communication_metadata","ordinal":11,"payload":{"trigger_turn":true}}),
                copied,
            ],
        );
        let mut cache = persistent_cache(&database);
        let result = collect_roots(
            ProviderId::Codex,
            std::slice::from_ref(&logs),
            &windows(),
            &mut cache,
        );
        assert_eq!(result.periods[4].total_tokens, 0);
        drop(cache);
        let mut own_context = context("gpt-6-astra");
        own_context["ordinal"] = json!(210);
        let mut own = legacy(
            "2026-09-16T09:02:00Z",
            usage(1100, 930, 115),
            usage(30, 15, 5),
        );
        own["ordinal"] = json!(211);
        append_log(
            &path,
            &(own_context.to_string() + "\n" + &own.to_string() + "\n"),
        );
        let mut cache = persistent_cache(&database);
        let result = collect_roots(
            ProviderId::Codex,
            std::slice::from_ref(&logs),
            &windows(),
            &mut cache,
        );
        assert_eq!(result.periods[4].total_tokens, 35);
        assert_eq!(result.periods[4].cached_input_tokens, 15);
        assert_eq!(result.periods[4].conversation_turns, Some(0));
        assert_eq!(
            cache.files[&path].parsed.events[0].model.as_deref(),
            Some("gpt-6-astra")
        );
        drop(cache);
        let result = collect_roots(
            ProviderId::Codex,
            &[logs],
            &windows(),
            &mut persistent_cache(&database),
        );
        assert_eq!(result.periods[4].total_tokens, 35);
    }

    #[test]
    fn late_ordinal_replays_cached_prefix_once_and_survives_restart() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let path = logs.join("late-ordinal.jsonl");
        let database = directory.path().join("history.sqlite3");
        let mut leaf = codex_meta("child", None, None);
        leaf["payload"]["source"] = json!({"subagent":{"thread_spawn":{}}});
        let mut prefix = legacy(
            "2026-09-16T09:01:00Z",
            usage(1000, 900, 100),
            usage(50, 10, 5),
        );
        prefix["ordinal"] = json!(9);
        write_log(&path, &[leaf, prefix]);
        let collect = |cache: &mut FileCache| {
            collect_roots(
                ProviderId::Codex,
                std::slice::from_ref(&logs),
                &windows(),
                cache,
            )
        };
        let mut cache = persistent_cache(&database);
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 55);
        drop(cache);
        let mut metadata = codex_meta("child", Some("missing-parent"), Some(10));
        metadata["ordinal"] = json!(10);
        let mut context = context("gpt-6-astra");
        context["ordinal"] = json!(11);
        let mut owned = legacy(
            "2026-09-16T09:02:00Z",
            usage(1050, 910, 105),
            usage(50, 10, 5),
        );
        owned["ordinal"] = json!(12);
        append_log(&path, &(metadata.to_string() + "\n"));
        let mut cache = persistent_cache(&database);
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 0);
        assert!(cache.files[&path].parsed.events.is_empty());
        drop(cache);
        append_log(
            &path,
            &(context.to_string() + "\n" + &owned.to_string() + "\n"),
        );
        let mut cache = persistent_cache(&database);
        let result = collect(&mut cache);
        assert_eq!(result.periods[4].total_tokens, 55);
        assert_eq!(result.periods[4].cached_input_tokens, 10);
        assert_eq!(result.periods[4].unpriced_tokens, 0);
        assert_eq!(cache.files[&path].parsed.events.len(), 1);
        assert_eq!(
            cache.files[&path].parsed.events[0].timestamp,
            time("2026-09-16T09:02:00Z")
        );
        drop(cache);
        let mut cache = persistent_cache(&database);
        assert_eq!(collect(&mut cache).periods[4].total_tokens, 55);
        assert_eq!(cache.files[&path].parsed.events.len(), 1);
        // Cold parsing the completed file must converge to the same classification.
        let result = collect(&mut FileCache::default());
        assert_eq!(result.periods[4].total_tokens, 55);
    }

    #[test]
    fn fork_baseline_and_late_lineage_remove_parent_prefix_and_keep_owned_delta() {
        let directory = tempdir().unwrap();
        let parent = [
            codex_meta("parent", None, None),
            context("gpt-6-astra"),
            legacy(
                "2026-09-16T08:59:59Z",
                usage(1000, 900, 100),
                usage(1000, 900, 100),
            ),
        ];
        write_log(&directory.path().join("parent.jsonl"), &parent);
        let mut leaf = codex_meta("child", None, None);
        leaf["payload"]["source"] = json!({"subagent":{"thread_spawn":{}}});
        write_log(
            &directory.path().join("child.jsonl"),
            &[
                leaf,
                context("parent-model"),
                legacy(
                    "2026-09-16T09:01:00Z",
                    usage(1000, 900, 100),
                    usage(50, 10, 5),
                ),
                codex_meta("child", Some("parent"), None),
                codex_meta("ancestor", None, None),
                context("gpt-6-astra"),
                legacy(
                    "2026-09-16T09:02:00Z",
                    usage(1050, 910, 105),
                    usage(50, 10, 5),
                ),
            ],
        );
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut FileCache::default(),
        );
        assert_eq!(result.periods[4].total_tokens, 1155);
        assert_eq!(result.periods[4].unpriced_tokens, 0);
        // The baseline is recomputed from normalized parent snapshots on every collection;
        // a missing parent is visible as incomplete rather than billing copied history.
        fs::remove_file(directory.path().join("parent.jsonl")).unwrap();
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut FileCache::default(),
        );
        assert_eq!(result.periods[4].total_tokens, 0);
        assert!(result.message.unwrap().contains("不完整"));
    }

    #[test]
    fn local_subagent_marker_proves_owned_suffix_and_model_context_is_authoritative() {
        let records = [
            codex_meta("child", Some("missing-parent"), None),
            legacy("2026-09-16T09:01:00Z", usage(1000, 900, 100), Value::Null),
            context("gpt-6-astra"),
            json!({"type":"inter_agent_communication_metadata","timestamp":"2026-09-16T09:01:00Z","payload":{"trigger_turn":true}}),
            legacy(
                "2026-09-16T09:02:00Z",
                usage(1050, 910, 105),
                usage(50, 10, 5),
            ),
            legacy(
                "2026-09-16T09:03:00Z",
                usage(1070, 915, 110),
                usage(20, 5, 5),
            ),
        ];
        let result = parsed(ProviderId::Codex, &records);
        assert_eq!(
            result
                .events
                .iter()
                .map(|event| event.usage.total())
                .sum::<u64>(),
            80
        );
        assert!(result
            .events
            .iter()
            .all(|event| event.fork_baseline.is_none()));
        let mut request = modern("2026-09-16T09:04:00Z", "model-authority", usage(10, 0, 1));
        request["payload"]["model"] = json!("stale-model");
        let result = parsed(ProviderId::Codex, &[context("gpt-6-astra"), request]);
        assert_eq!(result.events[0].model.as_deref(), Some("gpt-6-astra"));
    }

    #[test]
    fn independent_sessions_with_identical_legacy_usage_do_not_collide() {
        let directory = tempdir().unwrap();
        for id in ["one", "two"] {
            write_log(
                &directory.path().join(format!("{id}.jsonl")),
                &[
                    codex_meta(id, None, None),
                    legacy("2026-09-16T09:01:00Z", usage(100, 0, 10), usage(100, 0, 10)),
                ],
            );
        }
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut FileCache::default(),
        );
        assert_eq!(result.periods[4].total_tokens, 220);
    }

    #[test]
    fn pi_omp_filter_provider_combine_disjoint_caches_and_deduplicate_across_days() {
        let directory = tempdir().unwrap();
        let records = [
            json!({"type":"session","id":"pi-session"}),
            json!({"type":"model_change","provider":"openai-codex","modelId":"gpt-6-astra"}),
            json!({"type":"message","id":"user","timestamp":"2026-09-15T08:00:00Z","message":{"role":"user"}}),
            json!({"type":"message","id":"old","timestamp":"2026-09-15T09:00:00Z","message":{"role":"assistant","usage":{"input":40,"cacheRead":30,"cacheWrite":20,"output":10}}}),
            json!({"type":"message","id":"ignored","timestamp":"2026-09-16T09:00:00Z","message":{"role":"assistant","provider":"openai","model":"gpt-6-astra","usage":{"input":9000,"output":1000}}}),
            json!({"type":"message","id":"today","timestamp":"2026-09-16T09:00:00Z","message":{"role":"assistant","usage":{"input_tokens":100,"output_tokens":10}}}),
            json!({"type":"model_change","provider":"anthropic","modelId":"claude-sonnet-4-6"}),
            json!({"type":"message","id":"claude","timestamp":"2026-09-16T09:00:00Z","message":{"role":"assistant","usage":{"input":8000,"output":1000}}}),
        ];
        write_log(&directory.path().join("pi.jsonl"), &records);
        write_log(&directory.path().join("omp-copy.jsonl"), &records);
        let mut cache = FileCache::default();
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut cache,
        );
        assert_eq!(result.periods[0].total_tokens, 110);
        assert_eq!(result.periods[4].total_tokens, 210);
        assert_eq!(result.periods[4].cached_input_tokens, 30);
        assert_eq!(result.periods[4].cache_write_tokens, 20);
        assert_eq!(result.periods[4].request_count, Some(2));
        assert_eq!(result.periods[4].conversation_turns, Some(1));
        let appended = json!({"type":"message","id":"new-explicit","timestamp":"2026-09-16T10:00:00Z","message":{"role":"assistant","provider":"openai-codex","model":"gpt-6-astra","usage":{"input":20,"output":2}}});
        append_log(
            &directory.path().join("pi.jsonl"),
            &(appended.to_string() + "\n"),
        );
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut cache,
        );
        assert_eq!(result.periods[4].total_tokens, 232);
        assert_eq!(result.periods[4].request_count, Some(3));
    }

    #[test]
    fn appended_after_parser_eof_is_not_hidden_by_a_newer_file_checkpoint() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        write_log(
            &path,
            &[modern("2026-09-16T09:01:00Z", "first", usage(100, 0, 10))],
        );
        let mut entry = parse_incremental_codex(&path, None).unwrap();
        let boundary = entry.parsed.codex_state.as_ref().unwrap().offset;
        // Model the writer racing after read_line returned EOF but before final metadata.
        append_log(
            &path,
            &(modern("2026-09-16T09:02:00Z", "second", usage(20, 0, 2)).to_string() + "\n"),
        );
        let metadata = fs::metadata(&path).unwrap();
        entry.modified = metadata.modified().unwrap();
        entry.len = metadata.len();
        entry.checkpoint = Some(cache::checkpoint(&path, boundary).unwrap());
        let mut cache = FileCache::default();
        cache.files.insert(path, entry);
        let result = collect_roots(
            ProviderId::Codex,
            &[directory.path().into()],
            &windows(),
            &mut cache,
        );
        assert_eq!(result.periods[4].total_tokens, 132);
    }

    #[test]
    fn confirmed_legacy_reset_starts_a_new_deduplication_epoch() {
        let result = parsed(
            ProviderId::Codex,
            &[
                context("gpt-6-astra"),
                started(),
                legacy("2026-09-16T09:01:00Z", usage(100, 0, 10), usage(100, 0, 10)),
                legacy("2026-09-16T09:02:00Z", usage(0, 0, 0), usage(0, 0, 0)),
                legacy("2026-09-16T09:03:00Z", usage(100, 0, 10), usage(100, 0, 10)),
                legacy("2026-09-16T09:04:00Z", usage(100, 0, 10), usage(100, 0, 10)),
            ],
        );
        assert_eq!(result.events.len(), 2);
        assert_eq!(
            result
                .events
                .iter()
                .map(|event| event.usage.total())
                .sum::<u64>(),
            220
        );
    }

    #[test]
    #[ignore = "read-only smoke check against this machine's local usage logs"]
    fn local_statistics_smoke() {
        let directory = tempdir().unwrap();
        for provider in [ProviderId::Codex, ProviderId::Claude] {
            let start = Instant::now();
            let result = collect(provider, Some(directory.path())).unwrap();
            eprintln!(
                "{provider:?}: status={:?}; elapsed={:?}; periods={}; message={:?}",
                result.status,
                start.elapsed(),
                result.periods.len(),
                result.message
            );
            if provider == ProviderId::Codex {
                assert_eq!(result.status, ProviderStatus::Ready);
                assert!(!result.periods.is_empty());
            }
            let cached = Instant::now();
            let refreshed = collect(provider, Some(directory.path())).unwrap();
            eprintln!(
                "{provider:?}: cached elapsed={:?}; status={:?}",
                cached.elapsed(),
                refreshed.status
            );
        }
    }
}
