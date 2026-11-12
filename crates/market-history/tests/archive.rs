#[path = "../../../tests/support/mod.rs"]
mod test_support;
use market_history::{ArchiveEntry, GapReason, HistoryArchive};
use market_view::Cursor;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "market-history-{}-{}.archive",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn cursor(generation: &str, revision: u64) -> Cursor {
    Cursor {
        generation: generation.into(),
        revision,
    }
}
fn catalog(generation: &str) -> ArchiveEntry {
    ArchiveEntry::CatalogPage {
        generation: generation.into(),
        revision: "catalog-1".into(),
        offset: 0,
        entries: Vec::new(),
        complete: true,
    }
}
fn start(archive: &mut HistoryArchive) {
    archive.append(catalog("generation-a")).unwrap();
    archive
        .append(ArchiveEntry::Gap {
            after: None,
            resumed_after: cursor("generation-a", 10),
            reason: GapReason::InitialStart,
        })
        .unwrap();
}
fn batch(revision: u64) -> ArchiveEntry {
    ArchiveEntry::Batch {
        cursor: cursor("generation-a", revision),
        catalog_revision: "catalog-1".into(),
        events: test_support::sources()
            .into_iter()
            .map(Into::into)
            .collect(),
    }
}
#[test]
fn exclusive_writer_durable_resume_and_exact_batch_retry() {
    let file = Temp::new();
    let mut archive = HistoryArchive::open(&file.0).unwrap();
    assert!(HistoryArchive::open(&file.0).is_err());
    start(&mut archive);
    assert!(archive.append(batch(11)).unwrap());
    assert!(!archive.append(batch(11)).unwrap());
    let length = fs::metadata(archive.active_path()).unwrap().len();
    drop(archive);
    let mut archive = HistoryArchive::open(&file.0).unwrap();
    assert_eq!(archive.cursor(), Some(&cursor("generation-a", 11)));
    assert!(!archive.append(batch(11)).unwrap());
    assert_eq!(fs::metadata(archive.active_path()).unwrap().len(), length);
    let ArchiveEntry::Batch {
        cursor,
        catalog_revision,
        ..
    } = batch(11)
    else {
        unreachable!()
    };
    assert!(
        archive
            .append(ArchiveEntry::Batch {
                cursor,
                catalog_revision,
                events: Vec::new()
            })
            .is_err()
    );
    archive.append(batch(12)).unwrap();
}
#[test]
fn torn_final_write_is_truncated_without_losing_sealed_records() {
    let file = Temp::new();
    let mut archive = HistoryArchive::open(&file.0).unwrap();
    start(&mut archive);
    archive.append(batch(11)).unwrap();
    let active = archive.active_path();
    let length = fs::metadata(&active).unwrap().len();
    drop(archive);
    fs::OpenOptions::new()
        .append(true)
        .open(&active)
        .unwrap()
        .write_all(b"{\"type\":\"batch\",\"cursor\":")
        .unwrap();
    let mut archive = HistoryArchive::open(&file.0).unwrap();
    assert_eq!(fs::metadata(archive.active_path()).unwrap().len(), length);
    archive.append(batch(12)).unwrap();
    drop(archive);
    assert_eq!(
        HistoryArchive::open(&file.0).unwrap().cursor(),
        Some(&cursor("generation-a", 12))
    );
}
#[test]
fn complete_corruption_is_never_discarded_as_a_torn_tail() {
    let file = Temp::new();
    let mut archive = HistoryArchive::open(&file.0).unwrap();
    start(&mut archive);
    let active = archive.active_path();
    drop(archive);
    use std::io::{Read, Seek, SeekFrom};
    let mut handle = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(active)
        .unwrap();
    let _length = handle.metadata().unwrap().len();
    handle.seek(SeekFrom::End(-1)).unwrap();
    let mut byte = [0];
    handle.read_exact(&mut byte).unwrap();
    handle.seek(SeekFrom::End(-1)).unwrap();
    byte[0] ^= 1;
    handle.write_all(&byte).unwrap();
    drop(handle);
    assert!(HistoryArchive::open(&file.0).is_err());
}
#[test]
fn missing_replay_and_restarts_require_explicit_gap_boundaries() {
    let file = Temp::new();
    let mut archive = HistoryArchive::open(&file.0).unwrap();
    start(&mut archive);
    assert!(archive.append(batch(15)).is_err());
    archive
        .append(ArchiveEntry::Gap {
            after: archive.cursor().cloned(),
            resumed_after: cursor("generation-a", 14),
            reason: GapReason::ReplayExpired,
        })
        .unwrap();
    archive.append(batch(15)).unwrap();
    assert!(
        archive
            .append(ArchiveEntry::Gap {
                after: archive.cursor().cloned(),
                resumed_after: cursor("generation-a", 1),
                reason: GapReason::ReplayExpired
            })
            .is_err()
    );
    archive.append(catalog("generation-b")).unwrap();
    let new = ArchiveEntry::Batch {
        cursor: cursor("generation-b", 1),
        catalog_revision: "catalog-1".into(),
        events: Vec::new(),
    };
    assert!(archive.append(new.clone()).is_err());
    archive
        .append(ArchiveEntry::Gap {
            after: archive.cursor().cloned(),
            resumed_after: cursor("generation-b", 0),
            reason: GapReason::GenerationChanged,
        })
        .unwrap();
    archive.append(new).unwrap();
    drop(archive);
    assert_eq!(
        HistoryArchive::open(&file.0).unwrap().cursor(),
        Some(&cursor("generation-b", 1))
    );
}
#[test]
fn catalogs_and_initial_boundaries_must_precede_batches() {
    let file = Temp::new();
    let mut archive = HistoryArchive::open(&file.0).unwrap();
    assert!(archive.append(batch(1)).is_err());
    archive.append(catalog("generation-a")).unwrap();
    assert!(archive.append(batch(1)).is_err());
    archive
        .append(ArchiveEntry::Gap {
            after: None,
            resumed_after: cursor("generation-a", 0),
            reason: GapReason::InitialStart,
        })
        .unwrap();
    assert!(
        archive
            .append(ArchiveEntry::CatalogPage {
                generation: "generation-a".into(),
                revision: "catalog-2".into(),
                offset: 1,
                entries: Vec::new(),
                complete: true
            })
            .is_err()
    );
    archive.append(batch(1)).unwrap();
}

#[test]
fn rotation_compresses_sealed_segments_and_reopens_all_events_in_order() {
    let file = Temp::new();
    let mut archive = HistoryArchive::with_options(
        &file.0,
        market_history::ArchiveOptions {
            segment_bytes: 1024,
        },
    )
    .unwrap();
    start(&mut archive);
    let mut expected = Vec::new();
    for revision in 11..=40 {
        let entry = batch(revision);
        expected.push(entry.clone());
        archive.append(entry).unwrap();
    }
    let mut actual = Vec::new();
    archive
        .visit(|entry| {
            if matches!(entry, ArchiveEntry::Batch { .. }) {
                actual.push(entry);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(actual, expected);
    let compressed: Vec<_> = fs::read_dir(file.0.join("segments"))
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.path().extension().is_some_and(|x| x == "zst"))
        .collect();
    assert!(!compressed.is_empty());
    let disk: u64 = fs::read_dir(file.0.join("segments"))
        .unwrap()
        .map(Result::unwrap)
        .map(|e| e.metadata().unwrap().len())
        .sum();
    let json: usize = expected
        .iter()
        .map(|e| serde_json::to_vec(e).unwrap().len() + 1)
        .sum();
    assert!(disk < (json as u64));
    println!("Archive test data: {disk} bytes on disk versus {json} bytes of compact JSONL");
    drop(archive);
    let mut archive = HistoryArchive::open(&file.0).unwrap();
    assert_eq!(archive.cursor(), Some(&cursor("generation-a", 40)));
    assert!(!archive.append(batch(40)).unwrap());
    archive.append(batch(41)).unwrap();
}
#[test]
fn compression_crash_with_both_copies_recovers_only_after_content_agrees() {
    let file = Temp::new();
    let mut archive = HistoryArchive::with_options(
        &file.0,
        market_history::ArchiveOptions { segment_bytes: 128 },
    )
    .unwrap();
    start(&mut archive);
    archive.append(batch(11)).unwrap();
    drop(archive);
    let sealed = fs::read_dir(file.0.join("segments"))
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.path().extension().is_some_and(|x| x == "zst"))
        .unwrap()
        .path();
    let raw = sealed.with_extension("");
    let bytes = zstd::stream::decode_all(fs::File::open(&sealed).unwrap()).unwrap();
    fs::write(&raw, &bytes).unwrap();
    let archive = HistoryArchive::open(&file.0).unwrap();
    assert!(!raw.exists());
    assert_eq!(archive.cursor(), Some(&cursor("generation-a", 11)));
    drop(archive);
    let mut bad = bytes;
    bad[0] ^= 1;
    fs::write(raw, &bad).unwrap();
    assert!(HistoryArchive::open(&file.0).is_err());
}
#[test]
fn sealed_segment_truncation_and_missing_segments_fail_instead_of_hiding_history() {
    let file = Temp::new();
    let mut archive = HistoryArchive::with_options(
        &file.0,
        market_history::ArchiveOptions { segment_bytes: 128 },
    )
    .unwrap();
    start(&mut archive);
    archive.append(batch(11)).unwrap();
    drop(archive);
    let sealed = fs::read_dir(file.0.join("segments"))
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.path().extension().is_some_and(|x| x == "zst"))
        .unwrap()
        .path();
    let bytes = fs::read(&sealed).unwrap();
    fs::write(&sealed, &bytes[..bytes.len() - 1]).unwrap();
    assert!(HistoryArchive::open(&file.0).is_err());
    fs::write(&sealed, bytes).unwrap();
    fs::remove_file(sealed).unwrap();
    assert!(HistoryArchive::open(&file.0).is_err());
}
#[test]
fn frame_length_corruption_is_rejected_without_truncating_the_segment() {
    let file = Temp::new();
    let mut archive = HistoryArchive::open(&file.0).unwrap();
    start(&mut archive);
    let active = archive.active_path();
    drop(archive);
    let mut bytes = fs::read(&active).unwrap();
    bytes[8] ^= 64;
    let length = bytes.len();
    fs::write(&active, bytes).unwrap();
    assert!(HistoryArchive::open(&file.0).is_err());
    assert_eq!(fs::metadata(active).unwrap().len(), length as u64);
}

#[test]
fn order_metadata_survives_binary_archive_reopen_and_export() {
    let file = Temp::new();
    let mut archive = HistoryArchive::open(&file.0).unwrap();
    start(&mut archive);
    let market_view::Record::Order(mut order) = test_support::records().remove(1) else {
        unreachable!()
    };
    order.meta = [
        ("url".into(), "https://example.invalid/order".into()),
        ("source.label".into(), "Offer 🪙".into()),
    ]
    .into();
    let entry = ArchiveEntry::Batch {
        cursor: cursor("generation-a", 11),
        catalog_revision: "catalog-1".into(),
        events: vec![market_view::MarketEvent::Upsert(
            market_view::Record::Order(order),
        )],
    };
    archive.append(entry.clone()).unwrap();
    drop(archive);
    let archive = HistoryArchive::open(&file.0).unwrap();
    let mut found = false;
    archive
        .visit(|value| {
            if let ArchiveEntry::Batch { .. } = value {
                assert_eq!(value, entry);
                found = true;
            }
            Ok(())
        })
        .unwrap();
    assert!(found);
}
