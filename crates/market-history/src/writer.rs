use crate::{
    format::{MAGIC, MAX_FRAME_BYTES, encode, scan, write_frame},
    recovery::State,
    *,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
const MAX_SEGMENT_BYTES: u64 = 256 * 1024 * 1024;
#[derive(Debug, Clone, Copy)]
pub struct ArchiveOptions {
    pub segment_bytes: u64,
}
impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            segment_bytes: 8 * 1024 * 1024,
        }
    }
}
pub struct HistoryArchive {
    directory: PathBuf,
    _lock: File,
    file: File,
    index: u64,
    bytes: u64,
    state: State,
    options: ArchiveOptions,
    poisoned: bool,
}
fn sync_dir(path: &Path) -> Result<(), ArchiveError> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    Ok(())
}
fn path(directory: &Path, index: u64, compressed: bool) -> PathBuf {
    directory.join("segments").join(format!(
        "{index:020}.binpb{}",
        if compressed { ".zst" } else { "" }
    ))
}
fn segments(directory: &Path) -> Result<BTreeMap<u64, (bool, bool)>, ArchiveError> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(directory.join("segments"))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| invalid("invalid segment filename"))?;
        if name.ends_with(".tmp") {
            continue;
        }
        let (number, compressed) = if let Some(n) = name.strip_suffix(".binpb.zst") {
            (n, true)
        } else if let Some(n) = name.strip_suffix(".binpb") {
            (n, false)
        } else {
            return Err(invalid("unexpected file in segment directory"));
        };
        if number.len() != 20 {
            return Err(invalid("invalid segment number"));
        }
        let index = number
            .parse::<u64>()
            .map_err(|_| invalid("invalid segment number"))?;
        let entry = files.entry(index).or_insert((false, false));
        if compressed {
            entry.1 = true;
        } else {
            entry.0 = true;
        }
    }
    Ok(files)
}
fn reader(directory: &Path, index: u64, compressed: bool) -> Result<Box<dyn Read>, ArchiveError> {
    let file = File::open(path(directory, index, compressed))?;
    let reader: Box<dyn Read> = if compressed {
        let mut decoder = zstd::stream::read::Decoder::new(file)?;
        decoder.window_log_max(23)?;
        Box::new(decoder)
    } else {
        Box::new(file)
    };
    Ok(Box::new(BoundedReader {
        reader,
        remaining: MAX_SEGMENT_BYTES + MAX_FRAME_BYTES as u64 + 49,
    }))
}
struct BoundedReader {
    reader: Box<dyn Read>,
    remaining: u64,
}
impl Read for BoundedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut probe = [0];
            return if self.reader.read(&mut probe)? == 0 {
                Ok(0)
            } else {
                Err(io::Error::other("decoded segment exceeds limit"))
            };
        }
        let length = buffer.len().min(self.remaining as usize);
        let n = self.reader.read(&mut buffer[..length])?;
        self.remaining -= n as u64;
        Ok(n)
    }
}
fn hash(mut reader: impl Read) -> Result<[u8; 32], ArchiveError> {
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.finalize().into())
}
fn create(directory: &Path, index: u64) -> Result<File, ArchiveError> {
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create_new(true)
        .open(path(directory, index, false))?;
    file.write_all(MAGIC)?;
    file.sync_all()?;
    sync_dir(&directory.join("segments"))?;
    Ok(file)
}
impl HistoryArchive {
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        Self::with_options(directory, ArchiveOptions::default())
    }
    pub fn with_options(
        directory: impl AsRef<Path>,
        options: ArchiveOptions,
    ) -> Result<Self, ArchiveError> {
        if options.segment_bytes < 64 || options.segment_bytes > MAX_SEGMENT_BYTES {
            return Err(invalid("invalid segment size"));
        }
        let directory = directory.as_ref().to_path_buf();
        fs::create_dir_all(directory.join("segments"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("writer.lock"))?;
        lock.try_lock()
            .map_err(|e| io::Error::other(e.to_string()))?;
        lock.sync_all()?;
        sync_dir(&directory)?;
        if let Some(parent) = directory.parent().filter(|p| !p.as_os_str().is_empty()) {
            sync_dir(parent)?;
        }
        let files = segments(&directory)?;
        let last = files.last_key_value().map(|(&n, _)| n);
        let mut state = State::default();
        let mut active = None;
        for (position, (&index, &(raw, compressed))) in files.iter().enumerate() {
            if index != position as u64 + 1 {
                return Err(invalid("archive segment gap"));
            }
            if raw
                && compressed
                && hash(reader(&directory, index, false)?)?
                    != hash(reader(&directory, index, true)?)?
            {
                return Err(invalid("conflicting segment copies"));
            }
            let mut source = reader(&directory, index, compressed)?;
            let tail = last == Some(index) && !compressed;
            let complete = scan(&mut source, tail, |entry, digest| {
                state.apply(&entry, digest)?;
                Ok(())
            })?;
            if compressed && raw {
                fs::remove_file(path(&directory, index, false))?;
                sync_dir(&directory.join("segments"))?;
            }
            if tail {
                let mut file = OpenOptions::new()
                    .read(true)
                    .append(true)
                    .open(path(&directory, index, false))?;
                if complete < file.metadata()?.len() {
                    file.set_len(complete)?;
                    file.sync_all()?;
                }
                if complete == 0 {
                    file.write_all(MAGIC)?;
                    file.sync_all()?;
                }
                file.seek(SeekFrom::End(0))?;
                active = Some((index, file, complete.max(8)));
            }
        }
        let (index, file, bytes) = if let Some(active) = active {
            active
        } else {
            let index = last
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| invalid("segment number exhausted"))?;
            (index, create(&directory, index)?, 8)
        };
        Ok(Self {
            directory,
            _lock: lock,
            file,
            index,
            bytes,
            state,
            options,
            poisoned: false,
        })
    }
    pub fn cursor(&self) -> Option<&Cursor> {
        self.state.cursor.as_ref()
    }
    pub fn catalog(&self) -> Option<(&str, &str)> {
        self.state
            .catalog
            .as_ref()
            .map(|(g, r)| (g.as_str(), r.as_str()))
    }
    pub fn catalog_snapshot(
        &self,
    ) -> Result<Option<std::sync::Arc<market_view::protocol::catalog::CatalogSnapshot>>, ArchiveError>
    {
        let Some((generation, revision)) = &self.state.catalog else {
            return Ok(None);
        };
        let catalog = market_view::Catalog::from_entries(self.state.entries.iter().cloned())
            .map_err(|_| invalid("invalid archived catalog"))?;
        let snapshot = market_view::protocol::catalog::CatalogSnapshot::received(
            generation.clone(),
            revision.clone(),
            std::sync::Arc::new(catalog),
        );
        Ok(Some(std::sync::Arc::new(snapshot)))
    }
    pub fn active_path(&self) -> PathBuf {
        path(&self.directory, self.index, false)
    }
    pub fn visit(
        &self,
        mut visitor: impl FnMut(ArchiveEntry) -> Result<(), ArchiveError>,
    ) -> Result<(), ArchiveError> {
        for (&index, &(_, compressed)) in &segments(&self.directory)? {
            scan(
                &mut reader(&self.directory, index, compressed)?,
                false,
                |entry, _| visitor(entry),
            )?;
        }
        Ok(())
    }
    fn rotate(&mut self) -> Result<(), ArchiveError> {
        self.file.sync_all()?;
        let raw = path(&self.directory, self.index, false);
        let compressed = path(&self.directory, self.index, true);
        let temp = compressed.with_extension("zst.tmp");
        let target = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)?;
        let mut encoder = zstd::stream::write::Encoder::new(target, 3)?;
        encoder.include_checksum(true)?;
        std::io::copy(&mut File::open(&raw)?, &mut encoder)?;
        encoder.finish()?.sync_all()?;
        fs::rename(temp, compressed)?;
        sync_dir(&self.directory.join("segments"))?;
        fs::remove_file(raw)?;
        sync_dir(&self.directory.join("segments"))?;
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| invalid("segment number exhausted"))?;
        self.file = create(&self.directory, self.index)?;
        self.bytes = 8;
        Ok(())
    }
    pub fn append(&mut self, entry: ArchiveEntry) -> Result<bool, ArchiveError> {
        if self.poisoned {
            return Err(invalid("reopen the archive after a write failure"));
        }
        let bytes = encode(&entry)?;
        let mut next = self.state.clone();
        if !next.apply(&entry, Sha256::digest(&bytes).into())? {
            return Ok(false);
        }
        self.poisoned = true;
        if self.bytes > 8 && self.bytes + 40 + bytes.len() as u64 > self.options.segment_bytes {
            self.rotate()?;
        }
        write_frame(&mut self.file, &bytes)?;
        self.file.sync_all()?;
        self.bytes += 40 + bytes.len() as u64;
        self.state = next;
        self.poisoned = false;
        Ok(true)
    }
}
