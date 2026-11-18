use super::*;
use crate::{ReplicaWriter, test_support::catalog, w};
use std::time::Duration;

fn fixture() -> (tempfile::TempDir, ReplicaWriter) {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "executor-test").unwrap();
    let mut snapshot = writer.begin_snapshot().unwrap();
    snapshot.append(&catalog()).unwrap();
    writer
        .install_snapshot(snapshot.finish("initial").unwrap())
        .unwrap();
    (directory, writer)
}

#[tokio::test]
async fn queued_preparation_pins_after_shared_read_admission() {
    let (_directory, mut writer) = fixture();
    let runtime = ReadRuntime::new(writer.reader(), 1, CancellationToken::new()).unwrap();
    let reads = runtime.handle();
    let permit = reads.budget().try_query().unwrap();
    let mut queued = Box::pin(reads.prepare(|snapshot, _| snapshot.cursor()));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut queued)
            .await
            .is_err()
    );
    writer
        .apply(w::Batch {
            cursor: "newer".into(),
            changes: vec![],
        })
        .unwrap();
    drop(permit);
    assert_eq!(queued.await.unwrap().unwrap(), "newer");
}

#[tokio::test]
async fn cancelled_call_retains_capacity_and_shutdown_tracking_until_worker_exits() {
    let (_directory, writer) = fixture();
    let runtime = ReadRuntime::new(writer.reader(), 1, CancellationToken::new()).unwrap();
    let reads = runtime.handle();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    let (finished, done) = tokio::sync::oneshot::channel();
    let source = reads.clone();
    let request = tokio::spawn(async move {
        source
            .prepare(move |_, cancelled| {
                entered.send(()).unwrap();
                let _ = proceed.recv();
                finished.send(cancelled.is_cancelled()).unwrap();
            })
            .await
    });
    started.await.unwrap();
    request.abort();
    let _ = request.await;
    assert_eq!(
        reads.budget().try_query().unwrap_err().code(),
        tonic::Code::ResourceExhausted
    );
    let mut shutdown = Box::pin(runtime.shutdown());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut shutdown)
            .await
            .is_err()
    );
    release.send(()).unwrap();
    assert!(done.await.unwrap());
    tokio::time::timeout(Duration::from_secs(1), shutdown)
        .await
        .unwrap();
    assert!(reads.budget().try_query().is_err());
}

#[tokio::test]
async fn shutdown_rejects_queued_reads_and_panic_releases_worker_capacity() {
    let (_directory, writer) = fixture();
    let stop = CancellationToken::new();
    let runtime = ReadRuntime::new(writer.reader(), 1, stop.clone()).unwrap();
    let reads = runtime.handle();
    let error = reads
        .prepare(|_, _| panic!("worker failure"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::Unavailable);
    assert!(reads.prepare(|_, _| ()).await.is_ok());
    let permit = reads.budget().try_query().unwrap();
    let mut queued = Box::pin(reads.prepare(|_, _| panic!("cancelled work must not start")));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut queued)
            .await
            .is_err()
    );
    stop.cancel();
    assert_eq!(queued.await.unwrap_err().code(), tonic::Code::Unavailable);
    drop(permit);
    assert_eq!(
        reads.run(|_| ()).await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
}

#[tokio::test]
async fn caller_cancellation_reaches_catalog_helpers_and_pinned_readers() {
    let (_directory, writer) = fixture();
    let runtime = ReadRuntime::new(writer.reader(), 1, CancellationToken::new()).unwrap();
    let reads = runtime.handle();
    let permit = reads.budget().try_query().unwrap();
    let cancel = permit.cancel_on_drop();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    let request = tokio::spawn(async move {
        reads
            .admitted(permit, move |reader| {
                let pinned = reader.snapshot().unwrap();
                entered.send(()).unwrap();
                let _ = proceed.recv();
                crate::catalog_read::entities(&pinned, w::EntityKind::Currency, None)
            })
            .await
            .unwrap()
    });
    started.await.unwrap();
    drop(cancel);
    release.send(()).unwrap();
    assert_eq!(
        request.await.unwrap().unwrap_err().code(),
        tonic::Code::Cancelled
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn cancelled_admission_never_starts_and_owner_drop_closes_surviving_handles() {
    let (_directory, writer) = fixture();
    let parent = CancellationToken::new();
    let runtime = ReadRuntime::new(writer.reader(), 1, parent.clone()).unwrap();
    let reads = runtime.handle();
    let permit = reads.budget().try_query().unwrap();
    drop(permit.cancel_on_drop());
    assert!(
        reads
            .admitted(permit, |_| panic!("cancelled callback must not run"))
            .await
            .is_err()
    );
    assert!(reads.budget().try_query().is_ok());
    drop(runtime);
    assert!(!parent.is_cancelled());
    assert!(reads.budget().try_query().is_err());
    assert!(reads.prepare(|_, _| ()).await.is_err());
}

#[test]
fn construction_without_tokio_and_invalid_limits_return_errors() {
    let (_directory, writer) = fixture();
    for limit in [0, 1, usize::MAX] {
        assert!(ReadRuntime::new(writer.reader(), limit, CancellationToken::new()).is_err());
    }
}

#[tokio::test]
async fn admitted_work_cannot_use_a_different_runtimes_capacity() {
    let (_directory, writer) = fixture();
    let first = ReadRuntime::new(writer.reader(), 1, CancellationToken::new()).unwrap();
    let second = ReadRuntime::new(writer.reader(), 1, CancellationToken::new()).unwrap();
    let permit = first.handle().budget().try_query().unwrap();
    let error = second
        .handle()
        .admitted(permit, |_| panic!("foreign admission must not execute"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert!(first.handle().budget().try_query().is_ok());
    first.shutdown().await;
    second.shutdown().await;
}
