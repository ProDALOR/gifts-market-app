//! Bounded, checksummed Protobuf frames. Compression is only applied to sealed files.
use crate::wire as w;
use crate::*;
use prost::Message;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
pub(crate) const MAGIC: &[u8; 8] = b"MKTEVENT";
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
pub(crate) fn encode(entry: &ArchiveEntry) -> Result<Vec<u8>, ArchiveError> {
    let entry = w::ArchiveEntry::from(entry.clone());
    if entry.encoded_len() > MAX_FRAME_BYTES {
        return Err(invalid("archive frame exceeds limit"));
    }
    Ok(entry.encode_to_vec())
}
pub(crate) fn write_frame(file: &mut impl Write, bytes: &[u8]) -> Result<(), ArchiveError> {
    let length = bytes.len() as u32;
    file.write_all(&length.to_le_bytes())?;
    file.write_all(&(!length).to_le_bytes())?;
    file.write_all(&Sha256::digest(bytes))?;
    file.write_all(bytes)?;
    Ok(())
}
fn fill(reader: &mut impl Read, bytes: &mut [u8]) -> Result<usize, ArchiveError> {
    let mut n = 0;
    while n < bytes.len() {
        match reader.read(&mut bytes[n..]) {
            Ok(0) => break,
            Ok(count) => n += count,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(n)
}
/// Return the last complete byte offset. Only an unsealed tail may be incomplete.
pub(crate) fn scan(
    reader: &mut impl Read,
    tail: bool,
    mut visit: impl FnMut(ArchiveEntry, [u8; 32]) -> Result<(), ArchiveError>,
) -> Result<u64, ArchiveError> {
    let mut magic = [0; 8];
    let n = fill(reader, &mut magic)?;
    if n < 8 && tail && magic[..n] == MAGIC[..n] {
        return Ok(0);
    }
    if &magic != MAGIC {
        return Err(invalid("invalid archive magic"));
    }
    let mut complete = 8;
    loop {
        let mut header = [0; 40];
        let n = fill(reader, &mut header)?;
        if n == 0 {
            return Ok(complete);
        }
        if n < 40 {
            if tail {
                return Ok(complete);
            }
            return Err(invalid("truncated sealed frame"));
        }
        let length = u32::from_le_bytes(header[..4].try_into().expect("four bytes")) as usize;
        let complement = u32::from_le_bytes(header[4..8].try_into().expect("four bytes"));
        if (length as u32) ^ complement != u32::MAX {
            return Err(invalid("frame length checksum mismatch"));
        }
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(invalid("invalid frame length"));
        }
        let mut bytes = vec![0; length];
        if fill(reader, &mut bytes)? < length {
            if tail {
                return Ok(complete);
            }
            return Err(invalid("truncated sealed payload"));
        }
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if digest != header[8..] {
            return Err(invalid("archive checksum mismatch"));
        }
        let entry = w::ArchiveEntry::decode(bytes.as_slice())?.try_into()?;
        visit(entry, digest)?;
        complete += 40 + length as u64;
    }
}
impl From<ArchiveEntry> for w::ArchiveEntry {
    fn from(v: ArchiveEntry) -> Self {
        use w::archive_entry::Entry;
        Self {
            entry: Some(match v {
                ArchiveEntry::CatalogPage {
                    generation,
                    revision,
                    offset,
                    entries,
                    complete,
                } => Entry::CatalogPage(w::ArchiveCatalogPage {
                    generation,
                    revision,
                    offset: offset as u64,
                    entries: entries.into_iter().map(Into::into).collect(),
                    complete,
                }),
                ArchiveEntry::Gap {
                    after,
                    resumed_after,
                    reason,
                } => Entry::Gap(w::ArchiveGap {
                    after: after.map(Into::into),
                    resumed_after: Some(resumed_after.into()),
                    reason: match reason {
                        GapReason::InitialStart => w::ArchiveGapReason::InitialStart,
                        GapReason::GenerationChanged => w::ArchiveGapReason::GenerationChanged,
                        GapReason::ReplayExpired => w::ArchiveGapReason::ReplayExpired,
                    } as i32,
                }),
                ArchiveEntry::Batch {
                    cursor,
                    catalog_revision,
                    events,
                } => Entry::Batch(w::ArchiveBatch {
                    cursor: Some(cursor.into()),
                    catalog_revision,
                    events: events.into_iter().map(Into::into).collect(),
                }),
            }),
        }
    }
}
impl TryFrom<w::ArchiveEntry> for ArchiveEntry {
    type Error = ArchiveError;
    fn try_from(v: w::ArchiveEntry) -> Result<Self, Self::Error> {
        use w::archive_entry::Entry;
        Ok(
            match v.entry.ok_or_else(|| invalid("missing archive entry"))? {
                Entry::CatalogPage(v) => Self::CatalogPage {
                    generation: v.generation,
                    revision: v.revision,
                    offset: v
                        .offset
                        .try_into()
                        .map_err(|_| invalid("invalid catalog offset"))?,
                    entries: v
                        .entries
                        .into_iter()
                        .map(TryInto::try_into)
                        .collect::<Result<_, _>>()
                        .map_err(|_| invalid("invalid catalog entry"))?,
                    complete: v.complete,
                },
                Entry::Gap(v) => Self::Gap {
                    after: v.after.map(Into::into),
                    resumed_after: v
                        .resumed_after
                        .ok_or_else(|| invalid("missing boundary"))?
                        .into(),
                    reason: match w::ArchiveGapReason::try_from(v.reason)
                        .map_err(|_| invalid("unknown gap reason"))?
                    {
                        w::ArchiveGapReason::InitialStart => GapReason::InitialStart,
                        w::ArchiveGapReason::GenerationChanged => GapReason::GenerationChanged,
                        w::ArchiveGapReason::ReplayExpired => GapReason::ReplayExpired,
                        _ => return Err(invalid("missing gap reason")),
                    },
                },
                Entry::Batch(v) => Self::Batch {
                    cursor: v
                        .cursor
                        .ok_or_else(|| invalid("missing batch cursor"))?
                        .into(),
                    catalog_revision: v.catalog_revision,
                    events: v
                        .events
                        .into_iter()
                        .map(TryInto::try_into)
                        .collect::<Result<_, _>>()
                        .map_err(|_| invalid("invalid market event"))?,
                },
            },
        )
    }
}
