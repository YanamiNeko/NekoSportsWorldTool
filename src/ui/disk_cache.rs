//! Bounded application-owned caches. Oldest writes are evicted first, across providers.
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::SystemTime,
};

pub(super) const TILE_LIMIT: u64 = 128 * 1024 * 1024;
pub(super) const ROAD_LIMIT: u64 = 64 * 1024 * 1024;
static LOCK: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

fn files(root: &Path, result: &mut Vec<(SystemTime, PathBuf, u64)>) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        // Never follow symlinks/junctions out of an application cache.
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            files(&entry.path(), result)?;
        } else if kind.is_file() {
            let meta = entry.metadata()?;
            result.push((
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                entry.path(),
                meta.len(),
            ));
        }
    }
    Ok(())
}

fn trim(root: &Path, limit: u64, reserve: u64) -> io::Result<()> {
    if reserve > limit {
        return Err(io::Error::other("缓存内容超过目录容量限制"));
    }
    let mut entries = Vec::new();
    files(root, &mut entries)?;
    entries.sort();
    let mut total: u64 = entries.iter().map(|entry| entry.2).sum();
    for (_, path, size) in entries {
        if total <= limit - reserve {
            break;
        }
        fs::remove_file(path)?;
        total -= size;
    }
    Ok(())
}

pub(super) fn prepare(root: &Path, limit: u64) -> io::Result<()> {
    let mut checked = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fs::create_dir_all(root)?;
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err(io::Error::other("缓存目录不能是链接"));
    }
    if !checked.contains(root) {
        trim(root, limit, 0)?;
        checked.insert(root.to_owned());
    }
    Ok(())
}

// Reserve space for temporary files too. A failure remains an ordinary cache miss/error.
pub(super) fn store(root: &Path, limit: u64, entries: &[(&Path, &[u8])]) -> io::Result<()> {
    prepare(root, limit)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let canonical_root = root.canonicalize()?;
    for (path, _) in entries {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("缓存路径无效"))?;
        if !parent.starts_with(root) {
            return Err(io::Error::other("缓存路径越界"));
        }
        fs::create_dir_all(parent)?;
        if !parent.canonicalize()?.starts_with(&canonical_root) {
            return Err(io::Error::other("缓存路径越界"));
        }
    }
    trim(
        root,
        limit,
        entries.iter().map(|(_, bytes)| bytes.len() as u64).sum(),
    )?;
    for (path, bytes) in entries {
        let temp = root.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            fs::write(&temp, bytes)?;
            if path.exists() {
                fs::remove_file(path)?;
            }
            fs::rename(&temp, path)
        })();
        let _ = fs::remove_file(temp);
        result?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_includes_provider_subdirectories_and_sidecars() {
        let root = std::env::temp_dir().join(format!("cache-quota-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("provider")).unwrap();
        let old = root.join("old.png");
        fs::write(&old, [0; 60]).unwrap();
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        fs::write(root.join("provider/other.png"), [0; 20]).unwrap();
        let new = root.join("new.png");
        let ttl = root.join("new.ttl");
        store(&root, 100, &[(&new, &[1; 60]), (&ttl, b"120")]).unwrap();
        assert!(!old.exists());
        assert_eq!(fs::read(&new).unwrap(), vec![1; 60]);
        let mut entries = Vec::new();
        files(&root, &mut entries).unwrap();
        assert!(entries.iter().map(|e| e.2).sum::<u64>() <= 100);
        assert!(store(&root, 100, &[(&new, &[0; 101])]).is_err());
        assert_eq!(fs::read(&new).unwrap(), vec![1; 60]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_prunes_legacy_over_quota_cache() {
        let root = std::env::temp_dir().join(format!("cache-legacy-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("legacy.osm"), [0; 120]).unwrap();
        prepare(&root, 100).unwrap();
        assert!(!root.join("legacy.osm").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_writers_share_one_quota_and_cannot_write_outside_root() {
        let root = std::env::temp_dir().join(format!("cache-concurrent-{}", uuid::Uuid::new_v4()));
        let workers: Vec<_> = (0..4)
            .map(|index| {
                let root = root.clone();
                std::thread::spawn(move || {
                    store(
                        &root,
                        100,
                        &[(&root.join(format!("{index}.osm")), &[0; 60])],
                    )
                    .unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let mut entries = Vec::new();
        files(&root, &mut entries).unwrap();
        assert!(entries.iter().map(|e| e.2).sum::<u64>() <= 100);
        let outside = root.with_extension("outside");
        assert!(store(&root, 100, &[(&outside, b"data")]).is_err());
        assert!(!outside.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
