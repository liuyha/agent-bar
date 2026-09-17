//! Shared storage for normalized statistics and dashboard snapshots, never credentials.

use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{de::DeserializeOwned, Serialize};

static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);

pub fn data_dir(home: &Path) -> PathBuf {
    home.join(".agent-bar")
}

pub fn ensure_data_dir(path: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|error| format!("无法创建统计数据目录（{}）：{error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("无法设置统计数据目录权限：{error}"))?;
    }
    Ok(())
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取统计数据失败（{}）：{error}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("统计数据格式错误（{}）：{error}", path.display()))
}

pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path.parent().ok_or("统计数据路径无效")?;
    ensure_data_dir(parent)?;
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("无法序列化统计数据：{error}"))?;
    let sequence = NEXT_WRITE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".statistics-{}-{sequence}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("创建统计数据临时文件失败：{error}"))?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("保存统计数据失败（{}）：{error}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn statistics_use_the_user_home_and_survive_reloading() {
        let home = tempfile::tempdir().unwrap();
        let directory = data_dir(home.path());
        assert_eq!(directory, home.path().join(".agent-bar"));
        let path = directory.join("dashboard.json");
        assert!(read_json::<Value>(&path).unwrap().is_none());
        write_json(&path, &json!({"revision": 1})).unwrap();
        write_json(&path, &json!({"revision": 2})).unwrap();
        assert_eq!(
            read_json::<Value>(&path).unwrap(),
            Some(json!({"revision": 2}))
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn failed_atomic_replace_cleans_up_and_preserves_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dashboard.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), "original").unwrap();
        assert!(write_json(&path, &json!({"revision": 2})).is_err());
        assert_eq!(fs::read_to_string(path.join("keep")).unwrap(), "original");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
