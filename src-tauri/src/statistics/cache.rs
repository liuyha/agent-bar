//! Persistent normalized usage only. Callers must never put conversation text or credentials
//! into a cache blob. Checkpoints retain hashes of small file regions, never source bytes.

use std::{
    collections::HashSet,
    fs::{self, File, Metadata},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: i64 = 1;
const FINGERPRINT_BYTES: u64 = 8192;

pub(super) struct Store {
    connection: Connection,
}

impl Store {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(cache_error)?;
        }
        let mut connection = Connection::open(path).map_err(cache_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(cache_error)?;
        }
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(cache_error)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(cache_error)?;
        let transaction = connection.transaction().map_err(cache_error)?;
        let version: i64 = transaction
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(cache_error)?;
        if version > SCHEMA_VERSION {
            return Err(cache_error(format!(
                "缓存版本 {version} 高于支持版本 {SCHEMA_VERSION}，已保留原缓存"
            )));
        }
        if version != SCHEMA_VERSION {
            transaction
                .execute_batch("DROP TABLE IF EXISTS normalized_usage;")
                .map_err(cache_error)?;
        }
        transaction
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS normalized_usage (
                    path BLOB PRIMARY KEY NOT NULL,
                    parsed BLOB NOT NULL
                );",
            )
            .map_err(cache_error)?;
        transaction
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(cache_error)?;
        transaction.commit().map_err(cache_error)?;
        Ok(Self { connection })
    }

    pub(super) fn load(&self, path: &Path) -> Result<Option<Vec<u8>>, String> {
        self.connection
            .query_row(
                "SELECT parsed FROM normalized_usage WHERE path = ?1",
                [path_key(path)],
                |row| row.get(0),
            )
            .optional()
            .map_err(cache_error)
    }

    pub(super) fn save(&self, path: &Path, data: &[u8]) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO normalized_usage (path, parsed) VALUES (?1, ?2)
                 ON CONFLICT(path) DO UPDATE SET parsed = excluded.parsed",
                params![path_key(path), data],
            )
            .map_err(cache_error)?;
        Ok(())
    }

    /// The caller should retain only after successful discovery of every configured root.
    pub(super) fn retain(&self, paths: &HashSet<PathBuf>) -> Result<(), String> {
        let live: HashSet<_> = paths.iter().map(|path| path_key(path)).collect();
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(cache_error)?;
        {
            let mut statement = transaction
                .prepare("SELECT path FROM normalized_usage")
                .map_err(cache_error)?;
            let cached = statement
                .query_map([], |row| row.get::<_, Vec<u8>>(0))
                .map_err(cache_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(cache_error)?;
            for key in cached {
                if !live.contains(&key) {
                    transaction
                        .execute("DELETE FROM normalized_usage WHERE path = ?1", [key])
                        .map_err(cache_error)?;
                }
            }
        }
        transaction.commit().map_err(cache_error)
    }
}

fn cache_error(error: impl std::fmt::Display) -> String {
    format!("本机 Token 统计持久化存储不可用：{error}")
}

// Preserve non-UTF-8 filenames instead of allowing lossy path collisions.
#[cfg(unix)]
fn path_key(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn path_key(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(not(any(unix, windows)))]
fn path_key(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Checkpoint {
    /// The next unprocessed byte, at a complete JSONL record boundary.
    pub(super) offset: u64,
    /// Includes any incomplete trailing record that has not yet been processed.
    pub(super) len: u64,
    identity: Identity,
    modified_seconds: u64,
    modified_nanos: u32,
    prefix_hash: [u8; 32],
    tail_hash: [u8; 32],
    interior_hashes: [[u8; 32]; 3],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Identity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn identity(metadata: &Metadata) -> Identity {
    use std::os::unix::fs::MetadataExt;
    Identity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

// Without a stable file identity, conservatively reparse instead of assuming appends are safe.
#[cfg(not(unix))]
fn identity(_: &Metadata) -> Identity {
    Identity {
        device: 0,
        inode: 0,
    }
}

fn modification(metadata: &Metadata) -> io::Result<(u64, u32)> {
    let time = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid file modification time: {error}"),
            )
        })?;
    Ok((time.as_secs(), time.subsec_nanos()))
}

fn fingerprint(file: &mut File, start: u64, end: u64) -> io::Result<[u8; 32]> {
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; (end - start) as usize];
    file.read_exact(&mut bytes)?;
    Ok(Sha256::digest(bytes).into())
}

fn interior_fingerprints(file: &mut File, len: u64) -> io::Result<[[u8; 32]; 3]> {
    let mut hashes = [[0; 32]; 3];
    for (index, hash) in hashes.iter_mut().enumerate() {
        let center = (len / 4) * (index as u64 + 1);
        let start = center.saturating_sub(FINGERPRINT_BYTES / 2);
        *hash = fingerprint(
            file,
            start,
            start.saturating_add(FINGERPRINT_BYTES).min(len),
        )?;
    }
    Ok(hashes)
}

fn stable_metadata(before: &Metadata, after: &Metadata) -> io::Result<bool> {
    Ok(identity(before) == identity(after)
        && before.len() == after.len()
        && modification(before)? == modification(after)?)
}

pub(super) fn checkpoint(path: &Path, offset: u64) -> io::Result<Checkpoint> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let len = metadata.len();
    if offset > len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "checkpoint exceeds file length",
        ));
    }
    let (modified_seconds, modified_nanos) = modification(&metadata)?;
    let result = Checkpoint {
        offset,
        len,
        identity: identity(&metadata),
        modified_seconds,
        modified_nanos,
        prefix_hash: fingerprint(&mut file, 0, len.min(FINGERPRINT_BYTES))?,
        tail_hash: fingerprint(&mut file, len.saturating_sub(FINGERPRINT_BYTES), len)?,
        interior_hashes: interior_fingerprints(&mut file, len)?,
    };
    if !stable_metadata(&metadata, &file.metadata()?)?
        || !stable_metadata(&metadata, &fs::metadata(path)?)?
    {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "file changed while creating checkpoint",
        ));
    }
    Ok(result)
}

/// Reuse requires stable file identity and unchanged sampled content from the previous file.
/// Sampling includes the head, tail and quarter points; it does not reread the entire log.
/// Equal-length rewrites with changed modification time always force a complete reparse.
pub(super) fn can_append(path: &Path, previous: &Checkpoint) -> io::Result<bool> {
    if !cfg!(unix) {
        return Ok(false);
    }
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if previous.offset > previous.len
        || identity(&metadata) != previous.identity
        || metadata.len() < previous.len
    {
        return Ok(false);
    }
    if metadata.len() == previous.len
        && modification(&metadata)? != (previous.modified_seconds, previous.modified_nanos)
    {
        return Ok(false);
    }
    if fingerprint(&mut file, 0, previous.len.min(FINGERPRINT_BYTES))? != previous.prefix_hash
        || fingerprint(
            &mut file,
            previous.len.saturating_sub(FINGERPRINT_BYTES),
            previous.len,
        )? != previous.tail_hash
        || interior_fingerprints(&mut file, previous.len)? != previous.interior_hashes
    {
        return Ok(false);
    }
    Ok(stable_metadata(&metadata, &file.metadata()?)?
        && stable_metadata(&metadata, &fs::metadata(path)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write;

    #[test]
    fn usage_survives_reopening_and_is_replaced_by_path() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("nested/usage.sqlite");
        let path = directory.path().join("session.jsonl");
        let store = Store::open(&database).unwrap();
        store.save(&path, b"{\"tokens\":42}").unwrap();
        drop(store);
        let store = Store::open(&database).unwrap();
        assert_eq!(
            store.load(&path).unwrap().as_deref(),
            Some(&b"{\"tokens\":42}"[..])
        );
        store.save(&path, b"{\"tokens\":99}").unwrap();
        assert_eq!(
            store.load(&path).unwrap().as_deref(),
            Some(&b"{\"tokens\":99}"[..])
        );
    }

    #[test]
    fn retain_removes_only_absent_files_and_persists_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("usage.sqlite");
        let live = PathBuf::from("live.jsonl");
        let removed = PathBuf::from("removed.jsonl");
        let store = Store::open(&database).unwrap();
        store.save(&live, b"live").unwrap();
        store.save(&removed, b"removed").unwrap();
        store.retain(&HashSet::from([live.clone()])).unwrap();
        drop(store);
        let store = Store::open(&database).unwrap();
        assert_eq!(store.load(&live).unwrap().as_deref(), Some(&b"live"[..]));
        assert!(store.load(&removed).unwrap().is_none());
        store.retain(&HashSet::new()).unwrap();
        assert!(store.load(&live).unwrap().is_none());
    }

    #[test]
    fn older_schema_invalidates_old_normalized_data() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("usage.sqlite");
        let path = Path::new("session.jsonl");
        let store = Store::open(&database).unwrap();
        store.save(path, b"old-parser-data").unwrap();
        store
            .connection
            .pragma_update(None, "user_version", 0)
            .unwrap();
        drop(store);
        let store = Store::open(&database).unwrap();
        assert!(store.load(path).unwrap().is_none());
        store.save(path, b"new-parser-data").unwrap();
        assert_eq!(
            store.load(path).unwrap().as_deref(),
            Some(&b"new-parser-data"[..])
        );
    }

    #[test]
    fn newer_schema_is_rejected_without_discarding_data() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("usage.sqlite");
        let path = Path::new("session.jsonl");
        let store = Store::open(&database).unwrap();
        store.save(path, b"newer-parser-data").unwrap();
        store
            .connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(store);
        assert!(Store::open(&database)
            .err()
            .unwrap()
            .contains("已保留原缓存"));
        let connection = Connection::open(&database).unwrap();
        let bytes: Vec<u8> = connection
            .query_row("SELECT parsed FROM normalized_usage", [], |row| row.get(0))
            .unwrap();
        assert_eq!(bytes, b"newer-parser-data");
    }

    #[test]
    #[cfg(unix)]
    fn database_uses_wal_and_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("usage.sqlite");
        let store = Store::open(&database).unwrap();
        let mode: String = store
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(
            fs::metadata(&database).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn corrupt_database_is_reported_without_deleting_it() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("usage.sqlite");
        let damaged = b"not a SQLite database";
        fs::write(&database, damaged).unwrap();
        assert!(Store::open(&database).err().unwrap().contains("存储不可用"));
        assert_eq!(fs::read(&database).unwrap(), damaged);
    }

    #[test]
    #[cfg(unix)]
    fn checkpoint_survives_serialization_and_allows_append_after_partial_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        fs::write(&path, b"{\"n\":1}\n{\"n\":").unwrap();
        let before = checkpoint(&path, 8).unwrap();
        let before: Checkpoint =
            serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
        assert!(can_append(&path, &before).unwrap());
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"2}\n")
            .unwrap();
        assert!(can_append(&path, &before).unwrap());
        assert_eq!(before.offset, 8);
    }

    #[test]
    #[cfg(unix)]
    fn truncation_replacement_and_rewrites_force_reparse() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        fs::write(&path, b"first record\n").unwrap();
        let before = checkpoint(&path, 13).unwrap();
        fs::write(&path, b"short\n").unwrap();
        assert!(!can_append(&path, &before).unwrap());
        fs::write(&path, b"other record\nmore\n").unwrap();
        assert!(!can_append(&path, &before).unwrap());
        let replacement = directory.path().join("replacement.jsonl");
        fs::write(&replacement, b"first record\nmore\n").unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert!(!can_append(&path, &before).unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn changed_middle_with_same_length_forces_reparse_via_mtime() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        fs::write(&path, vec![b'x'; 32_768]).unwrap();
        let mut before = checkpoint(&path, 32_768).unwrap();
        // Explicitly make the saved time older, avoiding filesystem clock-granularity flakiness.
        before.modified_seconds = before.modified_seconds.saturating_sub(1);
        let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(16_384)).unwrap();
        file.write_all(b"y").unwrap();
        assert!(!can_append(&path, &before).unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn interior_rewrite_followed_by_append_forces_reparse() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        fs::write(&path, vec![b'x'; 131_072]).unwrap();
        let before = checkpoint(&path, 131_072).unwrap();
        let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(65_536)).unwrap();
        file.write_all(b"y").unwrap();
        file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(b"new record\n").unwrap();
        assert!(!can_append(&path, &before).unwrap());
    }

    #[test]
    fn checkpoint_rejects_offset_beyond_eof() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        fs::write(&path, b"{}\n").unwrap();
        assert_eq!(
            checkpoint(&path, 4).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
