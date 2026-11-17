//! Concurrent validated reads, atomic writes and background quota maintenance. Work completes on blocking workers even
//! if its async caller is cancelled, so index updates cannot be separated from a rename.
use crate::{DocumentId, MediaAsset, MediaError, MediaKey, MediaLimits, MediaResult};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, SystemTime},
};
struct Entry {
    bytes: u64,
    accessed: SystemTime,
}
struct Index {
    files: HashMap<MediaKey, Entry>,
    bytes: u64,
    generation: u64,
}
pub(crate) struct DiskStore {
    directory: PathBuf,
    limits: MediaLimits,
    index: Mutex<Index>,
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
impl DiskStore {
    pub fn open(directory: PathBuf, limits: MediaLimits) -> std::io::Result<Self> {
        fs::create_dir_all(&directory)?;
        let store = Self {
            directory,
            limits,
            index: Mutex::new(Index {
                files: HashMap::new(),
                bytes: 0,
                generation: 0,
            }),
        };
        let mut index = store.index.lock().unwrap();
        store.scan(&mut index)?;
        store.trim(&mut index, 0, None)?;
        drop(index);
        Ok(store)
    }
    fn path(&self, key: MediaKey) -> PathBuf {
        self.directory
            .join(format!("{}.{}", key.document().get(), key.extension()))
    }
    fn scan(&self, index: &mut Index) -> std::io::Result<()> {
        let mut files = HashMap::new();
        let mut bytes = 0;
        for item in fs::read_dir(&self.directory)? {
            let item = item?;
            let metadata = match item.metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if !item.file_type()?.is_file() {
                continue;
            }
            let name = item.file_name();
            let name = name.to_string_lossy();
            if let Some(key) = parse_name(&name) {
                bytes += metadata.len();
                let accessed = index
                    .files
                    .get(&key)
                    .map(|entry| entry.accessed)
                    .unwrap_or_else(|| metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));
                files.insert(
                    key,
                    Entry {
                        bytes: metadata.len(),
                        accessed,
                    },
                );
            } else if stale_temporary(&name, &metadata, self.limits.temporary_ttl) {
                let _ = fs::remove_file(item.path());
            }
        }
        *index = Index {
            files,
            bytes,
            generation: index.generation + 1,
        };
        Ok(())
    }
    fn trim(
        &self,
        index: &mut Index,
        incoming: u64,
        replacing: Option<MediaKey>,
    ) -> std::io::Result<()> {
        let previous = replacing
            .and_then(|key| index.files.get(&key))
            .map_or(0, |entry| entry.bytes);
        let over_budget = |bytes: u64| {
            bytes.saturating_sub(previous).saturating_add(incoming) > self.limits.disk_bytes
        };
        if !over_budget(index.bytes) {
            return Ok(());
        }
        let mut victims: Vec<_> = index
            .files
            .iter()
            .filter(|(key, _)| Some(**key) != replacing)
            .map(|(key, entry)| (*key, entry.accessed))
            .collect();
        victims.sort_unstable_by_key(|(_, accessed)| *accessed);
        for (key, _) in victims {
            if !over_budget(index.bytes) {
                break;
            }
            match fs::remove_file(self.path(key)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            if let Some(entry) = index.files.remove(&key) {
                index.bytes = index.bytes.saturating_sub(entry.bytes);
                index.generation += 1;
            }
        }
        Ok(())
    }
    pub fn touch(&self, key: MediaKey) {
        if let Ok(mut index) = self.index.try_lock()
            && let Some(entry) = index.files.get_mut(&key)
        {
            entry.accessed = SystemTime::now();
        }
    }
    pub fn read(&self, key: MediaKey) -> Result<Option<MediaAsset>, MediaError> {
        self.read_validated(key, |bytes| MediaAsset::parse(key.kind(), bytes).ok())
    }
    fn read_validated(
        &self,
        key: MediaKey,
        validate: impl FnOnce(Vec<u8>) -> Option<MediaAsset>,
    ) -> Result<Option<MediaAsset>, MediaError> {
        // Validation and decompression must not serialize unrelated cache hits.
        let generation = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation;
        let file = match fs::File::open(self.path(key)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(MediaError::Unavailable),
        };
        let mut bytes = Vec::new();
        file.take(key.max_bytes() as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| MediaError::Unavailable)?;
        let asset = validate(bytes);
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if generation == index.generation {
            match &asset {
                Some(asset) => {
                    if let Some(old) = index.files.insert(
                        key,
                        Entry {
                            bytes: asset.bytes().len() as u64,
                            accessed: SystemTime::now(),
                        },
                    ) {
                        index.bytes = index.bytes.saturating_sub(old.bytes);
                    }
                    index.bytes += asset.bytes().len() as u64;
                }
                None => {
                    // A corrupt read cannot delete a concurrently replaced file.
                    let _ = fs::remove_file(self.path(key));
                    if let Some(old) = index.files.remove(&key) {
                        index.bytes = index.bytes.saturating_sub(old.bytes);
                    }
                    index.generation += 1;
                }
            }
        }
        Ok(asset)
    }
    pub fn maintain(&self) -> Result<(), MediaError> {
        let generation = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation;
        let mut scanned = Index {
            files: HashMap::new(),
            bytes: 0,
            generation,
        };
        self.scan(&mut scanned)
            .map_err(|_| MediaError::Unavailable)?;
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        // Retry on the next maintenance tick if a write raced the directory walk.
        if index.generation != generation {
            return Ok(());
        }
        for (key, entry) in &mut scanned.files {
            if let Some(old) = index.files.get(key) {
                entry.accessed = old.accessed;
            }
        }
        *index = scanned;
        self.trim(&mut index, 0, None)
            .map_err(|_| MediaError::Unavailable)
    }
    pub fn write(&self, key: MediaKey, asset: &MediaAsset) -> MediaResult {
        if asset.bytes().len() as u64 > self.limits.disk_bytes {
            return Err(MediaError::Unavailable);
        }
        let temporary = Temporary(self.directory.join(format!(
            "{}.{}.part",
            key.document().get(),
            uuid::Uuid::new_v4()
        )));
        fs::write(&temporary.0, asset.bytes()).map_err(|_| MediaError::Unavailable)?;
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        self.trim(&mut index, asset.bytes().len() as u64, Some(key))
            .map_err(|_| MediaError::Unavailable)?;
        fs::rename(&temporary.0, self.path(key)).map_err(|_| MediaError::Unavailable)?;
        if let Some(previous) = index.files.insert(
            key,
            Entry {
                bytes: asset.bytes().len() as u64,
                accessed: SystemTime::now(),
            },
        ) {
            index.bytes = index.bytes.saturating_sub(previous.bytes);
        }
        index.bytes += asset.bytes().len() as u64;
        index.generation += 1;
        Ok(asset.clone())
    }
}
fn parse_name(name: &str) -> Option<MediaKey> {
    let (id, extension) = name.split_once('.')?;
    let id = DocumentId::new(id.parse().ok()?)?;
    match extension {
        "thumb" => Some(MediaKey::thumbnail(id)),
        "tgs" => Some(MediaKey::animation(id)),
        _ => None,
    }
}
fn stale_temporary(name: &str, metadata: &fs::Metadata, ttl: Duration) -> bool {
    let mut parts = name.split('.');
    matches!((parts.next(),parts.next(),parts.next(),parts.next()),(Some(id),Some(uuid),Some("part"),None) if id.parse::<i64>().is_ok_and(|id|id>0) && uuid::Uuid::parse_str(uuid).is_ok())
        && metadata
            .modified()
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age >= ttl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};
    fn key(id: i64) -> MediaKey {
        MediaKey::thumbnail(DocumentId::new(id).unwrap())
    }
    fn image() -> MediaAsset {
        MediaAsset::thumbnail(b"\x89PNG\r\n\x1a\nsynthetic".to_vec()).unwrap()
    }
    #[test]
    fn validation_runs_concurrently_and_corrupt_reads_cannot_delete_replacements() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(DiskStore::open(dir.path().into(), MediaLimits::default()).unwrap());
        store.write(key(1), &image()).unwrap();
        store.write(key(2), &image()).unwrap();
        let (entered, wait) = mpsc::channel();
        let (resume, blocked) = mpsc::channel();
        let reading = store.clone();
        let first = std::thread::spawn(move || {
            reading.read_validated(key(1), |_| {
                entered.send(()).unwrap();
                blocked.recv_timeout(Duration::from_secs(3)).unwrap();
                None // the old file failed validation
            })
        });
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        let (done, completion) = mpsc::channel();
        let other = store.clone();
        let second = std::thread::spawn(move || {
            assert!(other.read(key(2)).unwrap().is_some());
            other.write(key(1), &image()).unwrap();
            done.send(()).unwrap();
        });
        let concurrent = completion.recv_timeout(Duration::from_secs(1));
        resume.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap();
        concurrent.expect("an unrelated read and atomic replacement must not wait on validation");
        assert_eq!(store.read(key(1)).unwrap().unwrap().etag(), image().etag());
        let index = store.index.lock().unwrap();
        assert_eq!(
            index.bytes,
            index.files.values().map(|entry| entry.bytes).sum::<u64>()
        );
        assert_eq!(index.files.len(), 2);
    }
    #[test]
    fn maintenance_reconciles_disk_quota_and_cleans_temporary_files_without_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let limits = MediaLimits {
            disk_bytes: image().bytes().len() as u64 * 2,
            temporary_ttl: Duration::ZERO,
            ..Default::default()
        };
        let store = DiskStore::open(dir.path().into(), limits).unwrap();
        store.write(key(1), &image()).unwrap();
        for id in 2..=4 {
            fs::write(store.path(key(id)), image().bytes()).unwrap();
        }
        let partial = dir.path().join(format!("5.{}.part", uuid::Uuid::new_v4()));
        fs::write(&partial, b"stale").unwrap();
        fs::write(dir.path().join("keep.txt"), b"unrelated").unwrap();
        store.maintain().unwrap();
        let index = store.index.lock().unwrap();
        assert!(index.bytes <= store.limits.disk_bytes);
        assert_eq!(index.files.len(), 2);
        assert!(!partial.exists());
        assert!(dir.path().join("keep.txt").exists());
    }
}
