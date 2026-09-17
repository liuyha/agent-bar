use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use chrono::Utc;
use tokio::sync::Notify;

use crate::{
    models::{AppSettings, DashboardSnapshot, ProviderId},
    providers::{
        collect_snapshot, initial_snapshot, local_providers, pending_provider, UsageProvider,
    },
    storage,
};

struct CachedState {
    settings: AppSettings,
    snapshot: DashboardSnapshot,
    settings_version: u64,
}

pub struct AppState {
    cache: Mutex<CachedState>,
    refresh_lock: Mutex<()>,
    providers: Vec<Box<dyn UsageProvider>>,
    settings_path: PathBuf,
    snapshot_path: PathBuf,
    pub settings_changed: Notify,
}

impl AppState {
    pub fn load(settings_path: PathBuf, data_dir: PathBuf) -> Result<Self, String> {
        Self::load_with_paths(settings_path, data_dir, local_providers())
    }

    #[cfg(test)]
    fn load_with_providers(
        settings_path: PathBuf,
        providers: Vec<Box<dyn UsageProvider>>,
    ) -> Result<Self, String> {
        let data_dir = storage::data_dir(settings_path.parent().unwrap());
        Self::load_with_paths(settings_path, data_dir, providers)
    }

    fn load_with_paths(
        settings_path: PathBuf,
        data_dir: PathBuf,
        providers: Vec<Box<dyn UsageProvider>>,
    ) -> Result<Self, String> {
        let settings = load_settings(&settings_path)?;
        let snapshot_path = data_dir.join("dashboard.json");
        // Restore the last saved data immediately; credentials and HTTP stay on a worker.
        let snapshot = restore_snapshot(&snapshot_path, &settings);
        Ok(Self {
            cache: Mutex::new(CachedState {
                settings,
                snapshot,
                settings_version: 0,
            }),
            refresh_lock: Mutex::new(()),
            providers,
            settings_path,
            snapshot_path,
            settings_changed: Notify::new(),
        })
    }

    fn cache(&self) -> Result<MutexGuard<'_, CachedState>, String> {
        self.cache
            .lock()
            .map_err(|_| "应用状态已不可用，请重启 AgentBar".into())
    }

    pub fn settings(&self) -> Result<AppSettings, String> {
        Ok(self.cache()?.settings.clone())
    }

    pub fn snapshot(&self) -> Result<DashboardSnapshot, String> {
        Ok(self.cache()?.snapshot.clone())
    }

    pub fn refresh(&self) -> Result<DashboardSnapshot, String> {
        self.refresh_selection(None)
    }

    pub fn refresh_provider(&self, provider: ProviderId) -> Result<DashboardSnapshot, String> {
        self.refresh_selection(Some(provider))
    }

    fn refresh_selection(&self, provider: Option<ProviderId>) -> Result<DashboardSnapshot, String> {
        // Manual card refreshes and the background refresh share this lock, so
        // overlapping requests cannot overwrite each other's provider results.
        let _refresh = self
            .refresh_lock
            .lock()
            .map_err(|_| "用量刷新已不可用，请重启 AgentBar".to_string())?;
        let (mut settings, version) = {
            let cache = self.cache()?;
            (cache.settings.clone(), cache.settings_version)
        };
        if let Some(id) = provider {
            if !settings.enabled_providers.contains(&id) {
                return Err("该服务已停用，无法刷新用量".into());
            }
            if !self.providers.iter().any(|candidate| candidate.id() == id) {
                return Err("未找到该服务，无法刷新用量".into());
            }
            settings.enabled_providers = vec![id];
        }
        let collected = collect_snapshot(&settings, &self.providers, Utc::now());
        let mut cache = self.cache()?;
        // A save can complete while requests are in flight. Its filtered snapshot
        // wins; the settings notification schedules collection with the new selection.
        if cache.settings_version != version {
            return Ok(cache.snapshot.clone());
        }
        let mut snapshot = if let Some(id) = provider {
            let mut snapshot = cache.snapshot.clone();
            let usage = collected
                .providers
                .into_iter()
                .next()
                .ok_or_else(|| "未能读取该服务用量，请重试".to_string())?;
            let current = snapshot
                .providers
                .iter_mut()
                .find(|candidate| candidate.id == id)
                .ok_or_else(|| "该服务用量卡片已不可用，请重试".to_string())?;
            *current = usage;
            snapshot.updated_at = collected.updated_at;
            snapshot
        } else {
            collected
        };
        snapshot.revision = cache
            .snapshot
            .revision
            .checked_add(1)
            .ok_or_else(|| "快照版本已达到上限，请重启 AgentBar".to_string())?;
        // Publish only data successfully stored in the user's .agent-bar directory.
        // Hold the state lock through commit so settings changes cannot be overwritten.
        storage::write_json(&self.snapshot_path, &snapshot)?;
        let snapshot = storage::read_json::<DashboardSnapshot>(&self.snapshot_path)?
            .ok_or("保存后未找到本地用量数据，请重试")?;
        cache.snapshot = snapshot.clone();
        Ok(snapshot)
    }

    pub fn save_settings(&self, settings: AppSettings) -> Result<AppSettings, String> {
        let settings = settings.validated()?;
        // Saving settings never waits for credential/network IO. A failed write
        // leaves both cached settings and the dashboard untouched.
        let mut cache = self.cache()?;
        let mut snapshot = cache.snapshot.clone();
        snapshot.providers = settings
            .enabled_providers
            .iter()
            .map(|id| {
                cache
                    .snapshot
                    .providers
                    .iter()
                    .find(|provider| provider.id == *id)
                    .cloned()
                    .unwrap_or_else(|| pending_provider(*id))
            })
            .collect();
        snapshot.revision = cache
            .snapshot
            .revision
            .checked_add(1)
            .ok_or_else(|| "快照版本已达到上限，请重启 AgentBar".to_string())?;
        persist_settings(&self.settings_path, &settings)?;
        cache.settings_version = snapshot.revision;
        cache.settings = settings.clone();
        cache.snapshot = snapshot;
        drop(cache);
        self.settings_changed.notify_one();
        Ok(settings)
    }
}

fn restore_snapshot(path: &Path, settings: &AppSettings) -> DashboardSnapshot {
    match storage::read_json::<DashboardSnapshot>(path) {
        Ok(Some(mut snapshot)) => {
            // Settings are authoritative even when saved after the last collection.
            snapshot.providers = settings
                .enabled_providers
                .iter()
                .map(|id| {
                    snapshot
                        .providers
                        .iter()
                        .find(|provider| provider.id == *id)
                        .cloned()
                        .unwrap_or_else(|| pending_provider(*id))
                })
                .collect();
            snapshot
        }
        Ok(None) => initial_snapshot(settings),
        Err(error) => {
            eprintln!("{error}");
            let mut snapshot = initial_snapshot(settings);
            for provider in &mut snapshot.providers {
                provider.message = Some("本地用量数据无法读取，正在重新采集…".into());
            }
            snapshot
        }
    }
}

fn load_settings(path: &Path) -> Result<AppSettings, String> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(AppSettings::default()),
        Err(error) => return Err(format!("读取设置失败（{}）：{error}", path.display())),
    };
    let settings: AppSettings = serde_json::from_slice(&data)
        .map_err(|error| format!("设置文件格式错误（{}）：{error}", path.display()))?;
    settings.validated()
}

fn persist_settings(path: &Path, settings: &AppSettings) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or_else(|| "设置文件路径无效".to_string())?;
    fs::create_dir_all(directory).map_err(|error| format!("无法创建设置目录：{error}"))?;
    let bytes =
        serde_json::to_vec_pretty(settings).map_err(|error| format!("无法序列化设置：{error}"))?;
    let temporary_path = path.with_extension("json.tmp");
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary_path)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary_path, path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("保存设置失败（{}）：{error}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProviderStatus, ProviderUsage, Theme};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct CountingProvider {
        id: ProviderId,
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl UsageProvider for CountingProvider {
        fn id(&self) -> ProviderId {
            self.id
        }

        fn fetch_usage(&self, _: chrono::DateTime<Utc>) -> Result<ProviderUsage, String> {
            let count = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail {
                return Err("服务暂时不可用".into());
            }
            Ok(ProviderUsage {
                status: ProviderStatus::Ready,
                plan: format!("第 {count} 次读取"),
                updated_at: Some(format!("2026-09-16T00:00:{count:02}Z")),
                message: None,
                ..pending_provider(self.id)
            })
        }
    }

    fn counting_providers(
        codex_calls: &Arc<AtomicUsize>,
        claude_calls: &Arc<AtomicUsize>,
    ) -> Vec<Box<dyn UsageProvider>> {
        vec![
            Box::new(CountingProvider {
                id: ProviderId::Codex,
                calls: Arc::clone(codex_calls),
                fail: false,
            }),
            Box::new(CountingProvider {
                id: ProviderId::Claude,
                calls: Arc::clone(claude_calls),
                fail: false,
            }),
        ]
    }

    #[test]
    fn dashboard_reloads_from_user_storage_without_collection_and_keeps_revision() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("config/settings.json");
        let data_dir = storage::data_dir(directory.path());
        let codex_calls = Arc::new(AtomicUsize::new(0));
        let claude_calls = Arc::new(AtomicUsize::new(0));
        let state = AppState::load_with_paths(
            settings_path.clone(),
            data_dir.clone(),
            counting_providers(&codex_calls, &claude_calls),
        )
        .unwrap();
        state.refresh().unwrap();
        let saved = state.refresh_provider(ProviderId::Claude).unwrap();
        assert_eq!(
            storage::read_json::<DashboardSnapshot>(&data_dir.join("dashboard.json")).unwrap(),
            Some(saved.clone())
        );
        drop(state);
        let restored = AppState::load_with_paths(
            settings_path,
            data_dir,
            counting_providers(&codex_calls, &claude_calls),
        )
        .unwrap();
        assert_eq!(restored.snapshot().unwrap(), saved);
        assert_eq!(codex_calls.load(Ordering::SeqCst), 1);
        assert_eq!(claude_calls.load(Ordering::SeqCst), 2);
        let updated = restored.refresh_provider(ProviderId::Codex).unwrap();
        assert_eq!(updated.revision, saved.revision + 1);
        assert_eq!(updated.providers[1], saved.providers[1]);
    }

    #[test]
    fn restored_dashboard_obeys_settings_saved_after_last_refresh() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let calls = Arc::new(AtomicUsize::new(0));
        let state = AppState::load_with_providers(path.clone(), counting_providers(&calls, &calls))
            .unwrap();
        let saved = state.refresh().unwrap();
        state
            .save_settings(AppSettings {
                enabled_providers: vec![ProviderId::Claude],
                ..AppSettings::default()
            })
            .unwrap();
        drop(state);
        let restored = AppState::load_with_providers(path, vec![]).unwrap();
        assert_eq!(
            restored.snapshot().unwrap().providers,
            vec![saved.providers[1].clone()]
        );
    }

    #[test]
    fn failed_dashboard_write_does_not_publish_new_data() {
        let directory = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let state = AppState::load_with_providers(
            directory.path().join("settings.json"),
            counting_providers(&calls, &calls),
        )
        .unwrap();
        let saved = state.refresh().unwrap();
        fs::remove_file(&state.snapshot_path).unwrap();
        fs::create_dir(&state.snapshot_path).unwrap();
        let error = state.refresh_provider(ProviderId::Codex).unwrap_err();
        assert!(error.contains("保存统计数据失败"));
        assert_eq!(state.snapshot().unwrap(), saved);
        assert_eq!(
            fs::read_dir(state.snapshot_path.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn corrupt_dashboard_is_reported_then_rebuilt_by_successful_refresh() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = storage::data_dir(directory.path());
        storage::ensure_data_dir(&data_dir).unwrap();
        let snapshot_path = data_dir.join("dashboard.json");
        fs::write(&snapshot_path, "invalid-json").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let state = AppState::load_with_providers(
            directory.path().join("settings.json"),
            counting_providers(&calls, &calls),
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(state.snapshot().unwrap().providers[0]
            .message
            .as_ref()
            .unwrap()
            .contains("无法读取"));
        assert_eq!(fs::read_to_string(&snapshot_path).unwrap(), "invalid-json");
        let refreshed = state.refresh().unwrap();
        assert_eq!(
            storage::read_json::<DashboardSnapshot>(&snapshot_path).unwrap(),
            Some(refreshed)
        );
    }

    #[test]
    fn card_refresh_only_collects_target_and_preserves_other_provider_in_full() {
        let directory = tempfile::tempdir().unwrap();
        let codex_calls = Arc::new(AtomicUsize::new(0));
        let claude_calls = Arc::new(AtomicUsize::new(0));
        let state = AppState::load_with_providers(
            directory.path().join("settings.json"),
            counting_providers(&codex_calls, &claude_calls),
        )
        .unwrap();
        let initial = state.refresh().unwrap();
        assert_eq!(initial.revision, 1);

        let codex_refreshed = state.refresh_provider(ProviderId::Codex).unwrap();
        assert_eq!(codex_calls.load(Ordering::SeqCst), 2);
        assert_eq!(claude_calls.load(Ordering::SeqCst), 1);
        assert_eq!(codex_refreshed.providers[1], initial.providers[1]);
        assert_ne!(codex_refreshed.providers[0], initial.providers[0]);
        assert_eq!(codex_refreshed.revision, 2);

        let claude_refreshed = state.refresh_provider(ProviderId::Claude).unwrap();
        assert_eq!(codex_calls.load(Ordering::SeqCst), 2);
        assert_eq!(claude_calls.load(Ordering::SeqCst), 2);
        assert_eq!(claude_refreshed.providers[0], codex_refreshed.providers[0]);
        assert_eq!(claude_refreshed.revision, 3);

        state.save_settings(AppSettings::default()).unwrap();
        let background = state.refresh().unwrap();
        assert_eq!(background.revision, 5);
        assert_eq!(codex_calls.load(Ordering::SeqCst), 3);
        assert_eq!(claude_calls.load(Ordering::SeqCst), 3);
        assert_eq!(background.providers.len(), 2);
        assert_eq!(state.snapshot().unwrap(), background);
    }

    #[test]
    fn card_refresh_rejects_disabled_and_missing_providers_without_collecting() {
        let directory = tempfile::tempdir().unwrap();
        let codex_calls = Arc::new(AtomicUsize::new(0));
        let claude_calls = Arc::new(AtomicUsize::new(0));
        let state = AppState::load_with_providers(
            directory.path().join("settings.json"),
            counting_providers(&codex_calls, &claude_calls),
        )
        .unwrap();
        state
            .save_settings(AppSettings {
                enabled_providers: vec![ProviderId::Claude],
                ..AppSettings::default()
            })
            .unwrap();
        let before = state.snapshot().unwrap();
        assert!(state
            .refresh_provider(ProviderId::Codex)
            .unwrap_err()
            .contains("已停用"));
        assert_eq!(codex_calls.load(Ordering::SeqCst), 0);
        assert_eq!(claude_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.snapshot().unwrap(), before);

        let missing =
            AppState::load_with_providers(directory.path().join("missing-settings.json"), vec![])
                .unwrap();
        assert!(missing
            .refresh_provider(ProviderId::Codex)
            .unwrap_err()
            .contains("未找到"));
        assert_eq!(missing.snapshot().unwrap().revision, 0);
    }

    #[test]
    fn failed_card_refresh_only_updates_target_error_state() {
        let directory = tempfile::tempdir().unwrap();
        let codex_calls = Arc::new(AtomicUsize::new(0));
        let claude_calls = Arc::new(AtomicUsize::new(0));
        let mut providers = counting_providers(&codex_calls, &claude_calls);
        providers[0] = Box::new(CountingProvider {
            id: ProviderId::Codex,
            calls: Arc::clone(&codex_calls),
            fail: true,
        });
        let state =
            AppState::load_with_providers(directory.path().join("settings.json"), providers)
                .unwrap();
        let before = state.refresh().unwrap();
        let result = state.refresh_provider(ProviderId::Codex).unwrap();
        assert_eq!(result.providers[0].status, ProviderStatus::Error);
        assert_eq!(
            result.providers[0].message.as_deref(),
            Some("服务暂时不可用")
        );
        assert_eq!(result.providers[1], before.providers[1]);
        assert_eq!(codex_calls.load(Ordering::SeqCst), 2);
        assert_eq!(claude_calls.load(Ordering::SeqCst), 1);
        assert_eq!(result.revision, 2);
    }

    #[test]
    fn concurrent_card_refreshes_keep_both_results_and_monotonic_revisions() {
        let directory = tempfile::tempdir().unwrap();
        let codex_calls = Arc::new(AtomicUsize::new(0));
        let claude_calls = Arc::new(AtomicUsize::new(0));
        let state = Arc::new(
            AppState::load_with_providers(
                directory.path().join("settings.json"),
                counting_providers(&codex_calls, &claude_calls),
            )
            .unwrap(),
        );
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let workers = [ProviderId::Codex, ProviderId::Claude].map(|id| {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                state.refresh_provider(id).unwrap()
            })
        });
        barrier.wait();
        let mut revisions = workers.map(|worker| worker.join().unwrap().revision);
        revisions.sort_unstable();
        assert_eq!(revisions, [1, 2]);
        let result = state.snapshot().unwrap();
        assert_eq!(result.revision, 2);
        assert_eq!(codex_calls.load(Ordering::SeqCst), 1);
        assert_eq!(claude_calls.load(Ordering::SeqCst), 1);
        assert!(result
            .providers
            .iter()
            .all(|provider| provider.status == ProviderStatus::Ready));
    }

    #[test]
    fn settings_survive_restart_and_update_cached_provider_filter() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let state = AppState::load_with_providers(path.clone(), vec![]).unwrap();
        let settings = AppSettings {
            refresh_interval_seconds: 60,
            enabled_providers: vec![ProviderId::Claude],
            theme: Theme::Dark,
            codex_statistics_source: crate::models::CodexStatisticsSource::Auto,
        };
        assert_eq!(state.save_settings(settings.clone()).unwrap(), settings);
        assert_eq!(
            state.snapshot().unwrap().providers[0].id,
            ProviderId::Claude
        );
        assert_eq!(
            AppState::load_with_providers(path, vec![])
                .unwrap()
                .settings()
                .unwrap(),
            settings
        );
    }

    #[test]
    fn refresh_and_successful_saves_share_one_monotonic_revision() {
        let directory = tempfile::tempdir().unwrap();
        let state =
            AppState::load_with_providers(directory.path().join("settings.json"), vec![]).unwrap();
        assert_eq!(state.snapshot().unwrap().revision, 0);
        assert_eq!(state.refresh().unwrap().revision, 1);
        assert_eq!(state.refresh().unwrap().revision, 2);

        state.save_settings(AppSettings::default()).unwrap();
        assert_eq!(state.snapshot().unwrap().revision, 3);
        state.save_settings(AppSettings::default()).unwrap();
        assert_eq!(state.snapshot().unwrap().revision, 4);
        assert_eq!(state.refresh().unwrap().revision, 5);

        assert!(state
            .save_settings(AppSettings {
                refresh_interval_seconds: 0,
                ..AppSettings::default()
            })
            .is_err());
        assert_eq!(state.snapshot().unwrap().revision, 5);
    }

    #[test]
    fn failed_save_does_not_modify_settings_or_cached_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let state = AppState::load_with_providers(path.clone(), vec![]).unwrap();
        assert_eq!(state.refresh().unwrap().revision, 1);
        let original_settings = state.settings().unwrap();
        let original_snapshot = state.snapshot().unwrap();
        // A directory at the destination reliably causes atomic replacement to fail.
        fs::create_dir(&path).unwrap();
        let result = state.save_settings(AppSettings {
            theme: Theme::Dark,
            ..AppSettings::default()
        });
        assert!(result.is_err());
        assert_eq!(state.settings().unwrap(), original_settings);
        assert_eq!(state.snapshot().unwrap(), original_snapshot);
        assert_eq!(state.snapshot().unwrap().revision, 1);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn saving_during_collection_does_not_block_and_discards_obsolete_results() {
        use crate::models::ProviderStatus;
        use std::sync::{mpsc, Arc};

        struct SlowProvider {
            started: mpsc::SyncSender<()>,
            resume: Mutex<mpsc::Receiver<()>>,
        }
        impl UsageProvider for SlowProvider {
            fn id(&self) -> ProviderId {
                ProviderId::Codex
            }
            fn fetch_usage(
                &self,
                _: chrono::DateTime<Utc>,
            ) -> Result<crate::models::ProviderUsage, String> {
                self.started.send(()).unwrap();
                self.resume
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                Ok(crate::models::ProviderUsage {
                    status: ProviderStatus::Ready,
                    message: None,
                    ..pending_provider(ProviderId::Codex)
                })
            }
        }
        for single_provider in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (started_tx, started_rx) = mpsc::sync_channel(1);
            let (resume_tx, resume_rx) = mpsc::sync_channel(1);
            let state = Arc::new(
                AppState::load_with_providers(
                    directory.path().join("settings.json"),
                    vec![Box::new(SlowProvider {
                        started: started_tx,
                        resume: Mutex::new(resume_rx),
                    })],
                )
                .unwrap(),
            );
            let worker_state = Arc::clone(&state);
            let worker = std::thread::spawn(move || {
                if single_provider {
                    worker_state.refresh_provider(ProviderId::Codex).unwrap()
                } else {
                    worker_state.refresh().unwrap()
                }
            });
            started_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            state
                .save_settings(AppSettings {
                    enabled_providers: vec![ProviderId::Claude],
                    ..AppSettings::default()
                })
                .unwrap();
            assert_eq!(
                state.snapshot().unwrap().providers[0].id,
                ProviderId::Claude
            );
            resume_tx.send(()).unwrap();
            let result = worker.join().unwrap();
            assert_eq!(result.revision, 1);
            assert_eq!(result.providers.len(), 1);
            assert_eq!(result.providers[0].id, ProviderId::Claude);
            // The obsolete result must not reach disk either.
            assert!(!state.snapshot_path.exists());
        }
    }
}
