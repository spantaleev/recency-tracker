use crate::model::{Record, Resource};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct Storage(Option<Arc<Directory>>);
struct Directory {
    path: PathBuf,
    _lock: File,
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
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(path.join(".recency-tracker.lock"))?;
        lock.try_lock().map_err(|_| {
            io::Error::other("storage is already locked or does not support locking")
        })?;
        Ok(Self(Some(Arc::new(Directory { path, _lock: lock }))))
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
        let file = match tokio::fs::File::open(path).await {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        use tokio::io::AsyncReadExt;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes).await?;
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
            let mut temporary = tempfile::Builder::new()
                .prefix(".recency-")
                .suffix(".tmp")
                .tempfile_in(&directory.path)?;
            temporary.write_all(&bytes)?;
            temporary.as_file().sync_all()?;
            temporary.persist(destination).map_err(|e| e.error)?;
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
}
