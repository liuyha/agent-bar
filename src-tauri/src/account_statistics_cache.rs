//! Disk cache of normalized successful account activity. No collector, HTTP
//! client, CLI process, credential, cookie, or raw service response belongs here.

use std::{path::Path, sync::Mutex};

use serde::{Deserialize, Serialize};

use crate::{
    account_statistics::AccountUsageSnapshot,
    models::{CodexStatisticsSource, ProviderStatus},
    storage,
};

const FILE_NAME: &str = "codex-account-statistics.json";
const VERSION: u32 = 1;
const SUPERSEDED: &str = "已有更新的服务端统计请求，请读取最新缓存";

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    version: u32,
    scope: String,
    requested_source: CodexStatisticsSource,
    snapshot: AccountUsageSnapshot,
}

/// A new request invalidates older in-flight results even when it eventually
/// fails. Serialize the check and atomic replacement so completion order cannot
/// roll the cache backwards.
pub(crate) struct RequestOrder(Mutex<u64>);

pub(crate) static REQUESTS: RequestOrder = RequestOrder(Mutex::new(0));

impl RequestOrder {
    pub(crate) fn begin(&self) -> Result<u64, String> {
        let mut latest = self.0.lock().map_err(|_| "统计请求状态不可用")?;
        *latest += 1;
        Ok(*latest)
    }

    pub(crate) fn finish(
        &self,
        ticket: u64,
        directory: &Path,
        source: CodexStatisticsSource,
        scope: Option<&str>,
        mut snapshot: AccountUsageSnapshot,
        validate_current: impl Fn() -> Result<bool, String>,
    ) -> Result<AccountUsageSnapshot, String> {
        let latest = self.0.lock().map_err(|_| "统计请求状态不可用")?;
        if ticket != *latest {
            return Err(SUPERSEDED.into());
        }
        let web_enabled = validate_current()?;
        if !web_enabled {
            snapshot.web = None;
        }
        if snapshot.status != ProviderStatus::Ready {
            return Ok(snapshot);
        }
        let Some(scope) = scope else {
            let message = "无法确认本机登录范围，本次服务端数据未缓存";
            snapshot.message = Some(match snapshot.message {
                Some(existing) => format!("{existing}；{message}"),
                None => message.into(),
            });
            return Ok(snapshot);
        };

        // A transient web login/error hint may be shown for the live request,
        // but only normalized successful web data is written to disk.
        let transient_web = snapshot
            .web
            .as_ref()
            .filter(|web| web.status != ProviderStatus::Ready)
            .cloned();
        snapshot.web = snapshot
            .web
            .filter(|web| web.status == ProviderStatus::Ready);
        storage::write_json(
            &directory.join(FILE_NAME),
            &Envelope {
                version: VERSION,
                scope: scope.into(),
                requested_source: source,
                snapshot,
            },
        )?;
        let mut restored = read_cached(directory, source, Some(scope), web_enabled)?
            .ok_or("无法确认服务端统计缓存已保存，请重试")?;
        let web_enabled = validate_current()?;
        if web_enabled {
            if transient_web.is_some() {
                restored.web = transient_web;
            }
        } else {
            restored.web = None;
        }
        Ok(restored)
    }
}

/// Missing, unreadable, incompatible, or corrupt caches are safe cache misses.
/// This function only reads the cache file and never initiates collection.
pub(crate) fn read_cached(
    directory: &Path,
    source: CodexStatisticsSource,
    scope: Option<&str>,
    web_enabled: bool,
) -> Result<Option<AccountUsageSnapshot>, String> {
    let Some(scope) = scope else {
        return Ok(None);
    };
    let Ok(Some(envelope)) = storage::read_json::<Envelope>(&directory.join(FILE_NAME)) else {
        return Ok(None);
    };
    if envelope.version != VERSION
        || envelope.scope != scope
        || envelope.requested_source != source
        || envelope.snapshot.status != ProviderStatus::Ready
        || envelope.snapshot.source == CodexStatisticsSource::Local
    {
        return Ok(None);
    }
    let mut snapshot = envelope.snapshot;
    snapshot.web = snapshot
        .web
        .filter(|web| web_enabled && web.status == ProviderStatus::Ready);
    Ok(Some(snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_statistics::{AccountUsageSummary, WebUsageSnapshot};
    use std::{cell::Cell, fs};

    const SOURCE: CodexStatisticsSource = CodexStatisticsSource::Auto;
    const SCOPE: &str = "synthetic-scope-a";

    fn ready(tokens: u64) -> AccountUsageSnapshot {
        AccountUsageSnapshot {
            source: CodexStatisticsSource::Oauth,
            status: ProviderStatus::Ready,
            message: None,
            account: Some("synthetic@example.test".into()),
            account_id: Some("synthetic-account-a".into()),
            summary: AccountUsageSummary {
                lifetime_tokens: Some(tokens),
                ..Default::default()
            },
            daily_usage: None,
            service_updated_at: None,
            updated_at: Some("2026-09-17T00:00:00Z".into()),
            web: None,
        }
    }

    fn web(status: ProviderStatus) -> WebUsageSnapshot {
        WebUsageSnapshot {
            status,
            message: Some("synthetic-web-status".into()),
            account: Some("synthetic@example.test".into()),
            credits_remaining: Some(12.0),
            code_review_remaining_percent: None,
            usage_unit: None,
            usage_breakdown: None,
            credit_events: None,
            updated_at: None,
        }
    }

    fn save(order: &RequestOrder, directory: &Path, snapshot: AccountUsageSnapshot) {
        order
            .finish(
                order.begin().unwrap(),
                directory,
                SOURCE,
                Some(SCOPE),
                snapshot,
                || Ok(true),
            )
            .unwrap();
    }

    #[test]
    fn successful_cache_survives_new_instance_with_private_permissions() {
        let home = tempfile::tempdir().unwrap();
        let directory = storage::data_dir(home.path());
        let order = RequestOrder(Mutex::new(0));
        save(&order, &directory, ready(41));
        drop(order);
        let restored = read_cached(&directory, SOURCE, Some(SCOPE), false)
            .unwrap()
            .unwrap();
        assert_eq!(restored.summary.lifetime_tokens, Some(41));
        assert_eq!(restored.source, CodexStatisticsSource::Oauth);
        let next_instance = RequestOrder(Mutex::new(0));
        save(&next_instance, &directory, ready(42));
        assert_eq!(
            read_cached(&directory, SOURCE, Some(SCOPE), false)
                .unwrap()
                .unwrap()
                .summary
                .lifetime_tokens,
            Some(42)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(directory.join(FILE_NAME))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn reads_are_disk_only_and_missing_corrupt_or_foreign_caches_are_misses() {
        let directory = tempfile::tempdir().unwrap();
        assert!(read_cached(directory.path(), SOURCE, Some(SCOPE), true)
            .unwrap()
            .is_none());
        // A read never creates a directory/file or starts collection.
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        let order = RequestOrder(Mutex::new(0));
        save(&order, directory.path(), ready(41));
        let original = fs::read(directory.path().join(FILE_NAME)).unwrap();
        for scope in [None, Some("different-account")] {
            assert!(read_cached(directory.path(), SOURCE, scope, true)
                .unwrap()
                .is_none());
        }
        assert!(read_cached(
            directory.path(),
            CodexStatisticsSource::Cli,
            Some(SCOPE),
            true
        )
        .unwrap()
        .is_none());
        assert_eq!(
            fs::read(directory.path().join(FILE_NAME)).unwrap(),
            original
        );
        fs::write(directory.path().join(FILE_NAME), b"{broken").unwrap();
        assert!(read_cached(directory.path(), SOURCE, Some(SCOPE), true)
            .unwrap()
            .is_none());
        assert_eq!(
            fs::read(directory.path().join(FILE_NAME)).unwrap(),
            b"{broken"
        );
    }

    #[test]
    fn errors_and_unavailable_results_preserve_the_last_success() {
        let directory = tempfile::tempdir().unwrap();
        let order = RequestOrder(Mutex::new(0));
        save(&order, directory.path(), ready(41));
        let original = fs::read(directory.path().join(FILE_NAME)).unwrap();
        for status in [ProviderStatus::Error, ProviderStatus::Unavailable] {
            let mut snapshot = ready(0);
            snapshot.status = status;
            let result = order
                .finish(
                    order.begin().unwrap(),
                    directory.path(),
                    SOURCE,
                    Some(SCOPE),
                    snapshot,
                    || Ok(true),
                )
                .unwrap();
            assert_eq!(result.status, status);
            assert_eq!(
                fs::read(directory.path().join(FILE_NAME)).unwrap(),
                original
            );
        }
    }

    #[test]
    fn incompatible_and_unsuccessful_envelopes_are_not_restored() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        let mut envelope = Envelope {
            version: VERSION + 1,
            scope: SCOPE.into(),
            requested_source: SOURCE,
            snapshot: ready(41),
        };
        storage::write_json(&path, &envelope).unwrap();
        assert!(read_cached(directory.path(), SOURCE, Some(SCOPE), true)
            .unwrap()
            .is_none());
        envelope.version = VERSION;
        envelope.snapshot.status = ProviderStatus::Error;
        storage::write_json(&path, &envelope).unwrap();
        assert!(read_cached(directory.path(), SOURCE, Some(SCOPE), true)
            .unwrap()
            .is_none());
        envelope.snapshot.status = ProviderStatus::Ready;
        envelope.snapshot.source = CodexStatisticsSource::Local;
        storage::write_json(&path, &envelope).unwrap();
        assert!(read_cached(directory.path(), SOURCE, Some(SCOPE), true)
            .unwrap()
            .is_none());
    }

    #[test]
    fn failed_atomic_write_does_not_report_success_or_damage_the_destination() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join(FILE_NAME);
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("keep"), "original").unwrap();
        let order = RequestOrder(Mutex::new(0));
        assert!(order
            .finish(
                order.begin().unwrap(),
                directory.path(),
                SOURCE,
                Some(SCOPE),
                ready(41),
                || Ok(true)
            )
            .is_err());
        assert_eq!(
            fs::read_to_string(destination.join("keep")).unwrap(),
            "original"
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn web_cache_requires_success_and_respects_the_current_switch() {
        let directory = tempfile::tempdir().unwrap();
        let order = RequestOrder(Mutex::new(0));
        let mut snapshot = ready(41);
        snapshot.web = Some(web(ProviderStatus::Ready));
        save(&order, directory.path(), snapshot.clone());
        assert!(read_cached(directory.path(), SOURCE, Some(SCOPE), false)
            .unwrap()
            .unwrap()
            .web
            .is_none());
        assert!(read_cached(directory.path(), SOURCE, Some(SCOPE), true)
            .unwrap()
            .unwrap()
            .web
            .is_some());
        for status in [ProviderStatus::Error, ProviderStatus::Unavailable] {
            snapshot.web = Some(web(status));
            let result = order
                .finish(
                    order.begin().unwrap(),
                    directory.path(),
                    SOURCE,
                    Some(SCOPE),
                    snapshot.clone(),
                    || Ok(true),
                )
                .unwrap();
            assert_eq!(result.web.unwrap().status, status);
            assert!(read_cached(directory.path(), SOURCE, Some(SCOPE), true)
                .unwrap()
                .unwrap()
                .web
                .is_none());
            assert!(!fs::read_to_string(directory.path().join(FILE_NAME))
                .unwrap()
                .contains("synthetic-web-status"));
        }
        snapshot.web = Some(web(ProviderStatus::Ready));
        order
            .finish(
                order.begin().unwrap(),
                directory.path(),
                SOURCE,
                Some(SCOPE),
                snapshot,
                || Ok(false),
            )
            .unwrap();
        assert!(read_cached(directory.path(), SOURCE, Some(SCOPE), true)
            .unwrap()
            .unwrap()
            .web
            .is_none());
    }

    #[test]
    fn older_in_flight_results_cannot_overwrite_a_newer_request() {
        let directory = tempfile::tempdir().unwrap();
        let order = RequestOrder(Mutex::new(0));
        let older = order.begin().unwrap();
        let newer = order.begin().unwrap();
        order
            .finish(
                newer,
                directory.path(),
                SOURCE,
                Some(SCOPE),
                ready(42),
                || Ok(true),
            )
            .unwrap();
        assert_eq!(
            order
                .finish(
                    older,
                    directory.path(),
                    SOURCE,
                    Some(SCOPE),
                    ready(41),
                    || Ok(true)
                )
                .unwrap_err(),
            SUPERSEDED
        );
        assert_eq!(
            read_cached(directory.path(), SOURCE, Some(SCOPE), false)
                .unwrap()
                .unwrap()
                .summary
                .lifetime_tokens,
            Some(42)
        );
        let older = order.begin().unwrap();
        let newer = order.begin().unwrap();
        let mut failed = ready(0);
        failed.status = ProviderStatus::Error;
        order
            .finish(newer, directory.path(), SOURCE, Some(SCOPE), failed, || {
                Ok(true)
            })
            .unwrap();
        assert!(order
            .finish(
                older,
                directory.path(),
                SOURCE,
                Some(SCOPE),
                ready(43),
                || Ok(true)
            )
            .is_err());
        assert_eq!(
            read_cached(directory.path(), SOURCE, Some(SCOPE), false)
                .unwrap()
                .unwrap()
                .summary
                .lifetime_tokens,
            Some(42)
        );
    }

    #[test]
    fn changed_scope_or_settings_cannot_publish_or_return_an_old_result() {
        let directory = tempfile::tempdir().unwrap();
        let order = RequestOrder(Mutex::new(0));
        save(&order, directory.path(), ready(41));
        let original = fs::read(directory.path().join(FILE_NAME)).unwrap();
        assert!(order
            .finish(
                order.begin().unwrap(),
                directory.path(),
                SOURCE,
                Some(SCOPE),
                ready(42),
                || Err("scope changed".into())
            )
            .is_err());
        assert_eq!(
            fs::read(directory.path().join(FILE_NAME)).unwrap(),
            original
        );
        let calls = Cell::new(0);
        assert!(order
            .finish(
                order.begin().unwrap(),
                directory.path(),
                SOURCE,
                Some(SCOPE),
                ready(43),
                || {
                    calls.set(calls.get() + 1);
                    if calls.get() > 1 {
                        Err("scope changed after write".into())
                    } else {
                        Ok(true)
                    }
                }
            )
            .is_err());
        assert_eq!(calls.get(), 2);
        assert!(
            read_cached(directory.path(), SOURCE, Some("new-scope"), true)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn missing_cache_scope_keeps_live_cli_results_without_touching_disk() {
        let directory = tempfile::tempdir().unwrap();
        let order = RequestOrder(Mutex::new(0));
        save(&order, directory.path(), ready(41));
        let original = fs::read(directory.path().join(FILE_NAME)).unwrap();
        let mut snapshot = ready(42);
        snapshot.source = CodexStatisticsSource::Cli;
        snapshot.message = Some("partial data".into());
        let result = order
            .finish(
                order.begin().unwrap(),
                directory.path(),
                SOURCE,
                None,
                snapshot,
                || Ok(true),
            )
            .unwrap();
        assert_eq!(result.status, ProviderStatus::Ready);
        assert_eq!(result.summary.lifetime_tokens, Some(42));
        assert!(result.message.unwrap().contains("未缓存"));
        assert_eq!(
            fs::read(directory.path().join(FILE_NAME)).unwrap(),
            original
        );
        assert!(read_cached(directory.path(), SOURCE, None, true)
            .unwrap()
            .is_none());
    }
}
