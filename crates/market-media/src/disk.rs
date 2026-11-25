//! Concurrent validated reads, atomic writes and background quota maintenance. Work completes on blocking workers even
//! if its async caller is cancelled, so index updates cannot be separated from a rename.
use crate::{DocumentId, MediaAsset, MediaError, MediaKey, MediaKind, MediaLimits, MediaResult};
use sha2::{Digest, Sha256};
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
    sources: HashMap<DocumentId, Source>,
    bytes: u64,
    generation: u64,
}
#[derive(Clone, Copy)]
struct Source {
    digest: [u8; 32],
    stamp: Option<(SystemTime, u64)>,
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
    #[cfg(test)]
    pub fn open(directory: PathBuf, limits: MediaLimits) -> std::io::Result<Self> {
        Self::open_checked(directory, limits, &|| Ok(()))
    }
    pub(crate) fn open_checked(
        directory: PathBuf,
        limits: MediaLimits,
        check: &impl Fn() -> std::io::Result<()>,
    ) -> std::io::Result<Self> {
        check()?;
        fs::create_dir_all(&directory)?;
        let store = Self {
            directory,
            limits,
            index: Mutex::new(Index {
                files: HashMap::new(),
                sources: HashMap::new(),
                bytes: 0,
                generation: 0,
            }),
        };
        let mut index = store.index.lock().unwrap();
        store.scan_checked(&mut index, check)?;
        store.prune_previews_checked(&mut index, check)?;
        store.trim_checked(&mut index, 0, None, check)?;
        check()?;
        drop(index);
        Ok(store)
    }
    fn path(&self, key: MediaKey) -> PathBuf {
        self.directory
            .join(format!("{}.{}", key.document().get(), key.extension()))
    }
    fn scan(&self, index: &mut Index) -> std::io::Result<()> {
        self.scan_checked(index, &|| Ok(()))
    }
    fn scan_checked(
        &self,
        index: &mut Index,
        check: &impl Fn() -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        check()?;
        let mut files = HashMap::new();
        let mut previews = HashMap::<DocumentId, (SystemTime, MediaKey)>::new();
        let mut originals = HashMap::new();
        let mut bytes = 0;
        for item in fs::read_dir(&self.directory)? {
            check()?;
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
                let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                if key.kind() == MediaKind::Preview {
                    let candidate = (modified, key);
                    let entry = previews.entry(key.document()).or_insert(candidate);
                    // A stable tie-break also handles old caches with multiple derivatives.
                    if (modified, key.source()) > (entry.0, entry.1.source()) {
                        *entry = candidate;
                    }
                } else if key.kind() == MediaKind::Animation {
                    originals.insert(key.document(), (modified, metadata.len()));
                }
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
            } else if retired_placeholder(&name)
                || obsolete_derivative(&name)
                || stale_temporary(&name, &metadata, self.limits.temporary_ttl)
            {
                let _ = fs::remove_file(item.path());
            }
        }
        let mut sources = HashMap::new();
        for (document, (_, preview)) in &previews {
            check()?;
            sources.insert(
                *document,
                Source {
                    digest: preview.source().unwrap(),
                    stamp: None,
                },
            );
        }
        for (document, stamp) in originals {
            check()?;
            let previous = index.sources.get(&document).copied();
            if previous.is_none() && !previews.contains_key(&document) {
                continue;
            }
            let source =
                if let Some(previous) = previous.filter(|source| source.stamp == Some(stamp)) {
                    previous
                } else {
                    // Only changed originals with known derivatives need hashing. No decompression
                    // or JSON parsing is needed to restore source identity after a restart.
                    let mut bytes = Vec::new();
                    match fs::File::open(self.path(MediaKey::animation(document))) {
                        Ok(file) => {
                            file.take(crate::MAX_ANIMATION_BYTES as u64 + 1)
                                .read_to_end(&mut bytes)?;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => return Err(error),
                    }
                    if bytes.len() > crate::MAX_ANIMATION_BYTES {
                        sources.remove(&document);
                        continue;
                    }
                    Source {
                        digest: Sha256::digest(&bytes).into(),
                        stamp: Some(stamp),
                    }
                };
            sources.insert(document, source);
        }
        *index = Index {
            files,
            sources,
            bytes,
            generation: index.generation + 1,
        };
        Ok(())
    }
    pub fn preview_key(&self, document: DocumentId) -> Option<MediaKey> {
        self.index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sources
            .get(&document)
            .map(|source| MediaKey::preview(document, source.digest))
    }
    pub fn current_source(&self, document: DocumentId, digest: [u8; 32]) -> bool {
        self.index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sources
            .get(&document)
            .is_none_or(|source| source.digest == digest)
    }
    fn prune_previews(&self, index: &mut Index) -> std::io::Result<()> {
        self.prune_previews_checked(index, &|| Ok(()))
    }
    fn prune_previews_checked(
        &self,
        index: &mut Index,
        check: &impl Fn() -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        check()?;
        let obsolete: Vec<_> = index
            .files
            .keys()
            .copied()
            .filter(|key| {
                key.kind() == MediaKind::Preview
                    && index
                        .sources
                        .get(&key.document())
                        .is_some_and(|source| key.source() != Some(source.digest))
            })
            .collect();
        for key in obsolete {
            check()?;
            match fs::remove_file(self.path(key)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            if let Some(old) = index.files.remove(&key) {
                index.bytes = index.bytes.saturating_sub(old.bytes);
                index.generation += 1;
            }
        }
        Ok(())
    }
    fn trim(
        &self,
        index: &mut Index,
        incoming: u64,
        replacing: Option<MediaKey>,
    ) -> std::io::Result<()> {
        self.trim_checked(index, incoming, replacing, &|| Ok(()))
    }
    fn trim_checked(
        &self,
        index: &mut Index,
        incoming: u64,
        replacing: Option<MediaKey>,
        check: &impl Fn() -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        check()?;
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
            check()?;
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
        let stamp = file
            .metadata()
            .ok()
            .and_then(|metadata| Some((metadata.modified().ok()?, metadata.len())));
        let mut bytes = Vec::new();
        file.take(key.max_bytes() as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| MediaError::Unavailable)?;
        let asset = validate(bytes);
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if generation != index.generation
            && key.kind() == MediaKind::Animation
            && let Some(asset) = &asset
        {
            if index
                .sources
                .get(&key.document())
                .is_some_and(|source| source.digest != asset.digest)
            {
                return Err(MediaError::Busy);
            }
            // Unrelated writes must not prevent an original read from registering identity.
            if let std::collections::hash_map::Entry::Vacant(entry) =
                index.sources.entry(key.document())
            {
                entry.insert(Source {
                    digest: asset.digest,
                    stamp,
                });
                index.generation += 1;
            }
        }
        if generation == index.generation {
            match &asset {
                Some(asset) => {
                    if key.kind() == MediaKind::Animation {
                        index.sources.insert(
                            key.document(),
                            Source {
                                digest: asset.digest,
                                stamp,
                            },
                        );
                        index.generation += 1;
                    }
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
            sources: self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .sources
                .clone(),
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
        self.prune_previews(&mut index)
            .map_err(|_| MediaError::Unavailable)?;
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
        if key.kind() == MediaKind::Preview
            && index
                .sources
                .get(&key.document())
                .is_none_or(|source| key.source() != Some(source.digest))
        {
            // An older conversion cannot restore a derivative after its original was replaced.
            return Err(MediaError::Unavailable);
        }
        let previous_source = index.sources.get(&key.document()).copied();
        let replacing_source = key.kind() == MediaKind::Animation;
        let write = (|| -> std::io::Result<()> {
            if replacing_source {
                index.sources.insert(
                    key.document(),
                    Source {
                        digest: asset.digest,
                        stamp: None,
                    },
                );
                // Invalidate old derivatives before replacing the source, even across a crash.
                self.prune_previews(&mut index)?;
            }
            self.trim(&mut index, asset.bytes().len() as u64, Some(key))?;
            fs::rename(&temporary.0, self.path(key))
        })();
        if write.is_err() {
            if replacing_source {
                match previous_source {
                    Some(source) => {
                        index.sources.insert(key.document(), source);
                    }
                    None => {
                        index.sources.remove(&key.document());
                    }
                }
            }
            return Err(MediaError::Unavailable);
        }
        if key.kind() == MediaKind::Animation {
            let stamp = fs::metadata(self.path(key))
                .ok()
                .and_then(|metadata| Some((metadata.modified().ok()?, metadata.len())));
            index.sources.insert(
                key.document(),
                Source {
                    digest: asset.digest,
                    stamp,
                },
            );
        }
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
        _ if extension.starts_with("gallery-") => {
            let (version, hash) = derivative_extension(extension, "gallery-", ".png")?;
            (version == crate::asset::GALLERY_VERSION).then(|| MediaKey::gallery(id, hash))
        }
        _ if extension.starts_with("cover-") => {
            let (version, hash) = derivative_extension(extension, "cover-", ".png")?;
            (version == crate::asset::COVER_VERSION).then(|| MediaKey::cover(id, hash))
        }
        _ => {
            let (version, hash) = preview_extension(extension)?;
            (version == crate::asset::PREVIEW_VERSION).then(|| MediaKey::preview(id, hash))
        }
    }
}
fn preview_extension(extension: &str) -> Option<(&str, [u8; 32])> {
    derivative_extension(extension, "preview-", ".svg")
}
fn derivative_extension<'a>(
    extension: &'a str,
    prefix: &str,
    suffix: &str,
) -> Option<(&'a str, [u8; 32])> {
    let (version, encoded) = extension
        .strip_prefix(prefix)?
        .strip_suffix(suffix)?
        .split_once('-')?;
    if version.is_empty() || !version.bytes().all(|c| c.is_ascii_digit()) || encoded.len() != 64 {
        return None;
    }
    let mut hash = [0; 32];
    for (index, byte) in hash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(encoded.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some((version, hash))
}
// Remove only our retired cache filename shape, leaving unrelated SVGs intact.
fn retired_placeholder(name: &str) -> bool {
    name.strip_suffix(".placeholder.svg")
        .and_then(|id| id.parse().ok())
        .and_then(DocumentId::new)
        .is_some()
}
fn obsolete_derivative(name: &str) -> bool {
    let Some((id, extension)) = name.split_once('.') else {
        return false;
    };
    if id.parse::<i64>().ok().and_then(DocumentId::new).is_none() {
        return false;
    }
    preview_extension(extension).is_some_and(|(version, _)| version != crate::PREVIEW_VERSION)
        || derivative_extension(extension, "cover-", ".png")
            .is_some_and(|(version, _)| version != crate::asset::COVER_VERSION)
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
    fn interrupted_open_stops_scanning_and_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        for id in 1..=64 {
            fs::write(dir.path().join(format!("{id}.tgs")), b"cached original").unwrap();
        }
        let calls = std::cell::Cell::new(0);
        let error = DiskStore::open_checked(dir.path().into(), MediaLimits::default(), &|| {
            let n = calls.get();
            calls.set(n + 1);
            if n == 10 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "test stop",
                ))
            } else {
                Ok(())
            }
        })
        .err()
        .unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(calls.get(), 11);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 64);
        let reopened = DiskStore::open(dir.path().into(), MediaLimits::default()).unwrap();
        assert_eq!(reopened.index.lock().unwrap().files.len(), 64);
    }
    #[test]
    fn retired_outlines_are_removed_on_open_and_maintenance_without_touching_other_assets() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "42.placeholder.svg",
            "0.placeholder.svg",
            "-1.placeholder.svg",
            "logo.placeholder.svg",
            "42.svg",
            "43.thumb",
        ] {
            fs::write(dir.path().join(name), b"cached").unwrap();
        }
        let store = DiskStore::open(dir.path().into(), MediaLimits::default()).unwrap();
        assert!(!dir.path().join("42.placeholder.svg").exists());
        for name in [
            "0.placeholder.svg",
            "-1.placeholder.svg",
            "logo.placeholder.svg",
            "42.svg",
            "43.thumb",
        ] {
            assert!(dir.path().join(name).exists(), "{name}");
        }
        fs::write(dir.path().join("44.placeholder.svg"), b"retired").unwrap();
        store.maintain().unwrap();
        assert!(!dir.path().join("44.placeholder.svg").exists());
        assert_eq!(store.index.lock().unwrap().bytes, 6);
    }
    #[test]
    fn obsolete_preview_versions_are_removed_without_touching_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let old = format!("42.preview-0-{}.svg", "ab".repeat(32));
        let current = format!(
            "42.preview-{}-{}.svg",
            crate::asset::PREVIEW_VERSION,
            "cd".repeat(32)
        );
        for name in [&old, &current, "keep.svg", "42.preview-not-a-cache.svg"] {
            fs::write(dir.path().join(name), b"cached").unwrap();
        }
        let store = DiskStore::open(dir.path().into(), MediaLimits::default()).unwrap();
        assert!(!dir.path().join(old).exists());
        assert!(dir.path().join(current).exists());
        assert!(dir.path().join("keep.svg").exists());
        assert!(dir.path().join("42.preview-not-a-cache.svg").exists());
        assert_eq!(store.index.lock().unwrap().bytes, 6);
    }
    #[test]
    fn corrupt_preview_is_discarded_and_can_be_atomically_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let store = DiskStore::open(dir.path().into(), MediaLimits::default()).unwrap();
        let key = MediaKey::preview(DocumentId::new(1).unwrap(), [7; 32]);
        fs::write(store.path(key), b"<svg><script/></svg>").unwrap();
        store.maintain().unwrap();
        assert!(store.read(key).unwrap().is_none());
        assert!(!store.path(key).exists());
        assert_eq!(store.index.lock().unwrap().bytes, 0);
        let asset = MediaAsset::preview(
            br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"/>"##.to_vec(),
        )
        .unwrap();
        store.write(key, &asset).unwrap();
        assert_eq!(store.read(key).unwrap().unwrap().etag(), asset.etag());
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
    fn unrelated_writes_do_not_lose_an_originals_source_identity() {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let store =
            Arc::new(DiskStore::open(directory.path().into(), MediaLimits::default()).unwrap());
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(br#"{"v":"5.7.4","w":512,"h":512,"fr":60,"ip":0,"op":60,"layers":[]}"#)
            .unwrap();
        let original = MediaAsset::animation(gzip.finish().unwrap()).unwrap();
        let animation = MediaKey::animation(DocumentId::new(1).unwrap());
        fs::write(store.path(animation), original.bytes()).unwrap();
        let (entered, waiting) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let reading = store.clone();
        let thread = std::thread::spawn(move || {
            reading.read_validated(animation, |bytes| {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(3)).unwrap();
                MediaAsset::animation(bytes).ok()
            })
        });
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        store.write(key(2), &image()).unwrap();
        release.send(()).unwrap();
        thread.join().unwrap().unwrap().unwrap();
        assert_eq!(
            store.preview_key(animation.document()),
            Some(MediaKey::preview(animation.document(), original.digest))
        );
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
