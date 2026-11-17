//! Serialized atomic writes and quota maintenance. Work completes on blocking workers even
//! if its async caller is cancelled, so index updates cannot be separated from a rename.
use crate::{DocumentId, MediaAsset, MediaError, MediaKey, MediaLimits, MediaResult};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};
struct Entry {
    bytes: u64,
    accessed: SystemTime,
}
struct Index {
    files: HashMap<MediaKey, Entry>,
    bytes: u64,
    scanned: Instant,
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
                scanned: Instant::now(),
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
            scanned: Instant::now(),
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
        while index
            .bytes
            .saturating_sub(previous)
            .saturating_add(incoming)
            > self.limits.disk_bytes
        {
            let Some(key) = index
                .files
                .iter()
                .filter(|(key, _)| Some(**key) != replacing)
                .min_by_key(|(_, entry)| entry.accessed)
                .map(|(key, _)| *key)
            else {
                break;
            };
            match fs::remove_file(self.path(key)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            if let Some(entry) = index.files.remove(&key) {
                index.bytes = index.bytes.saturating_sub(entry.bytes);
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
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let file = match fs::File::open(self.path(key)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(MediaError::Unavailable),
        };
        let mut bytes = Vec::new();
        file.take(key.max_bytes() as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| MediaError::Unavailable)?;
        match MediaAsset::parse(key.kind(), bytes) {
            Ok(asset) => {
                let entry = Entry {
                    bytes: asset.bytes().len() as u64,
                    accessed: SystemTime::now(),
                };
                if let Some(old) = index.files.insert(key, entry) {
                    index.bytes = index.bytes.saturating_sub(old.bytes);
                }
                index.bytes += asset.bytes().len() as u64;
                Ok(Some(asset))
            }
            Err(_) => {
                let _ = fs::remove_file(self.path(key));
                if let Some(old) = index.files.remove(&key) {
                    index.bytes = index.bytes.saturating_sub(old.bytes);
                }
                Ok(None)
            }
        }
    }
    pub fn write(&self, key: MediaKey, asset: &MediaAsset) -> MediaResult {
        if asset.bytes().len() as u64 > self.limits.disk_bytes {
            return Err(MediaError::Unavailable);
        }
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if index.scanned.elapsed() >= Duration::from_secs(60) {
            self.scan(&mut index).map_err(|_| MediaError::Unavailable)?;
        }
        self.trim(&mut index, asset.bytes().len() as u64, Some(key))
            .map_err(|_| MediaError::Unavailable)?;
        let temporary = Temporary(self.directory.join(format!(
            "{}.{}.part",
            key.document().get(),
            uuid::Uuid::new_v4()
        )));
        fs::write(&temporary.0, asset.bytes()).map_err(|_| MediaError::Unavailable)?;
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
