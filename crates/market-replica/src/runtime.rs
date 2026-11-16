//! Assemble a read capability and one ordinary synchronization worker.
use crate::{ReplicaReader, ReplicaWriter, ReplicationWorker};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tonic::Status;

pub struct ReplicaRuntime {
    pub reader: ReplicaReader,
    pub replication: ReplicationWorker,
}
impl ReplicaRuntime {
    pub async fn open(directory: &Path, source: &str) -> Result<Self, Status> {
        let directory = directory.to_owned();
        let source = source.to_owned();
        let writer = tokio::task::spawn_blocking(move || ReplicaWriter::open(&directory, &source))
            .await
            .map_err(|_| Status::unavailable("Replica storage worker stopped"))??;
        Ok(Self {
            reader: writer.reader(),
            replication: ReplicationWorker {
                writer: Arc::new(Mutex::new(writer)),
            },
        })
    }
}
