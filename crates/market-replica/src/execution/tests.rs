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
    let reads = ReadExecutor::new(writer.reader(), 1, CancellationToken::new()).unwrap();
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
    let reads = ReadExecutor::new(writer.reader(), 1, CancellationToken::new()).unwrap();
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
    let jobs = reads.tasks();
    jobs.close();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), jobs.wait())
            .await
            .is_err()
    );
    release.send(()).unwrap();
    assert!(done.await.unwrap());
    tokio::time::timeout(Duration::from_secs(1), jobs.wait())
        .await
        .unwrap();
    assert!(reads.budget().try_query().is_ok());
}

#[tokio::test]
async fn shutdown_rejects_queued_reads_and_panic_releases_worker_capacity() {
    let (_directory, writer) = fixture();
    let stop = CancellationToken::new();
    let reads = ReadExecutor::new(writer.reader(), 1, stop.clone()).unwrap();
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
