use crate::model::{Record, Resource};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct Storage(Option<Arc<Directory>>);
struct Directory {
    path: PathBuf,
    _lock: File,
    #[cfg(test)]
    fail_after_rename: std::sync::atomic::AtomicBool,
}
impl Storage {
    /// Hold an exclusive advisory lock for this server's lifetime.
    pub fn open(path: Option<PathBuf>) -> io::Result<Self> {
        let Some(path) = path else {
            return Ok(Self(None));
        };
        if !path.is_dir() {
            return Err(io::Error::other("storage path is not a directory"));
        }
        let lock_path = path.join(".recency-tracker.lock");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o666);
        }
        let lock = match options.open(&lock_path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // Unix advisory locking works with a read-only descriptor. This lets
                // an explicitly changed UID lock readable state left by the old UID.
                #[cfg(unix)]
                {
                    File::open(&lock_path)?
                }
                #[cfg(not(unix))]
                {
                    std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(&lock_path)?
                }
            }
            Err(e) => return Err(e),
        };
        lock.try_lock().map_err(|_| {
            io::Error::other("storage is already locked or does not support locking")
        })?;
        Ok(Self(Some(Arc::new(Directory {
            path,
            _lock: lock,
            #[cfg(test)]
            fail_after_rename: std::sync::atomic::AtomicBool::new(false),
        }))))
    }
    pub fn filename(directory: &Path, resource: &Resource) -> PathBuf {
        let hash = Sha256::digest(resource.as_str().as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        directory.join(format!("{hash}.json"))
    }
    pub async fn read(&self, resource: &Resource) -> io::Result<Option<Record>> {
        let Some(directory) = &self.0 else {
            return Ok(None);
        };
        // A publisher body is at most 100 KiB. Bound reads of corrupt/untrusted files too.
        let path = Self::filename(&directory.path, resource);
        tokio::task::spawn_blocking(move || {
            #[cfg(unix)]
            let opened = {
                use rustix::fs::{Mode, OFlags, open};
                // Nonblocking open rejects FIFOs without waiting for a writer.
                // Check the opened inode, avoiding a metadata/open race.
                open(
                    &path,
                    OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map(File::from)
                .map_err(io::Error::from)
            };
            #[cfg(not(unix))]
            let opened = File::open(&path);
            let file = match opened {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            if !file.metadata()?.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "persisted resource is not a regular file",
                ));
            }
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 1024 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "persisted resource exceeds 1 MiB",
                ));
            }
            serde_json::from_slice(&bytes).map(Some).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid persisted resource JSON",
                )
            })
        })
        .await
        .map_err(io::Error::other)?
    }
    #[cfg(test)]
    pub(crate) fn fail_next_write_after_rename(&self) {
        self.0
            .as_ref()
            .unwrap()
            .fail_after_rename
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    /// The tracker actor orders writes. Sync before acknowledging success.
    pub async fn write(&self, resource: &Resource, record: &Record) -> io::Result<()> {
        let Some(directory) = &self.0 else {
            return Ok(());
        };
        let directory = directory.clone();
        let destination = Self::filename(&directory.path, resource);
        let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
        tokio::task::spawn_blocking(move || {
            let mut builder = tempfile::Builder::new();
            builder.prefix(".recency-").suffix(".tmp");
            // Match v2's ordinary file creation permissions, subject to the process
            // umask. Preserve an existing record's explicit permissions on replacement.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                builder.permissions(std::fs::Permissions::from_mode(0o666));
            }
            let mut temporary = builder.tempfile_in(&directory.path)?;
            if let Ok(metadata) = std::fs::metadata(&destination) {
                temporary
                    .as_file()
                    .set_permissions(metadata.permissions())?;
            }
            temporary.write_all(&bytes)?;
            temporary.as_file().sync_all()?;
            temporary.persist(destination).map_err(|e| e.error)?;
            #[cfg(test)]
            if directory
                .fail_after_rename
                .swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                return Err(io::Error::other("injected failure after rename"));
            }
            #[cfg(unix)]
            File::open(&directory.path)?.sync_all()?;
            Ok(())
        })
        .await
        .map_err(io::Error::other)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Version;
    use serde_json::json;
    fn resource() -> Resource {
        Resource::try_from("/example/😀".to_owned()).unwrap()
    }
    #[tokio::test]
    async fn replaces_legacy_json_atomically_and_preserves_the_format() {
        let dir = tempfile::tempdir().unwrap();
        let r = resource();
        let path = Storage::filename(dir.path(), &r);
        // Fixture has the same format and hash convention as the old server.
        std::fs::write(&path, b"{\"version\":\"1\",\"data\":{\"old\":true}}").unwrap();
        let storage = Storage::open(Some(dir.path().to_owned())).unwrap();
        assert_eq!(storage.read(&r).await.unwrap().unwrap().version.0, "1");
        let next = Record {
            version: Version("2".into()),
            data: json!({"nested":[null,false,"β"]}),
        };
        storage.write(&r, &next).await.unwrap();
        assert_eq!(storage.read(&r).await.unwrap(), Some(next));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2); // record and lock only
        assert!(Storage::open(Some(dir.path().to_owned())).is_err());
        drop(storage);
        assert!(Storage::open(Some(dir.path().to_owned())).is_ok());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_nonregular_records_without_blocking() {
        use rustix::fs::{CWD, Mode, mkfifoat};
        let dir = tempfile::tempdir().unwrap();
        let r = resource();
        let path = Storage::filename(dir.path(), &r);
        let storage = Storage::open(Some(dir.path().to_owned())).unwrap();
        mkfifoat(CWD, &path, Mode::RUSR | Mode::WUSR).unwrap();
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(1), storage.read(&r)).await;
        if result.is_err() {
            // Unblock a regressed blocking open before the test runtime is dropped.
            // Otherwise a failed assertion could leave CI waiting forever.
            if let Ok(fd) = rustix::fs::open(
                &path,
                rustix::fs::OFlags::WRONLY | rustix::fs::OFlags::NONBLOCK,
                Mode::empty(),
            ) {
                let _ = File::from(fd).write_all(b"{}");
            }
        }
        assert_eq!(
            result.unwrap().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            storage.read(&r).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        std::fs::remove_dir(&path).unwrap();
        std::os::unix::fs::symlink("/dev/zero", &path).unwrap();
        assert!(storage.read(&r).await.is_err());
    }
    #[tokio::test]
    async fn corrupt_data_is_an_error_not_an_unknown_version() {
        let dir = tempfile::tempdir().unwrap();
        let r = resource();
        std::fs::write(Storage::filename(dir.path(), &r), b"{").unwrap();
        let storage = Storage::open(Some(dir.path().to_owned())).unwrap();
        assert_eq!(
            storage.read(&r).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[tokio::test]
    async fn failed_write_keeps_the_previous_record() {
        let dir = tempfile::tempdir().unwrap();
        let r = resource();
        let storage = Storage::open(Some(dir.path().to_owned())).unwrap();
        let old = Record {
            version: Version("1".into()),
            data: json!({}),
        };
        storage.write(&r, &old).await.unwrap();
        // Moving the directory makes the configured destination unavailable without chmod/root assumptions.
        let moved = dir.path().with_extension("moved");
        std::fs::rename(dir.path(), &moved).unwrap();
        assert!(
            storage
                .write(
                    &r,
                    &Record {
                        version: Version("2".into()),
                        data: json!({})
                    }
                )
                .await
                .is_err()
        );
        let bytes = std::fs::read(Storage::filename(&moved, &r)).unwrap();
        assert_eq!(serde_json::from_slice::<Record>(&bytes).unwrap(), old);
        std::fs::rename(&moved, dir.path()).unwrap();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn replacement_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let r = resource();
        let path = Storage::filename(dir.path(), &r);
        std::fs::write(&path, b"{\"version\":\"1\",\"data\":{}}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let storage = Storage::open(Some(dir.path().to_owned())).unwrap();
        storage
            .write(
                &r,
                &Record {
                    version: Version("2".into()),
                    data: json!({}),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
