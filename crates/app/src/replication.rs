//! One background synchronization loop per process. Reads never issue core RPCs.
mod view;
use market_reader::{node::NodeConfig, registration::NodeCredentials};
use market_view::protocol::{MAX_MESSAGE_BYTES, wire as w};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tonic::{Code, Status};
use uuid::Uuid;
pub use view::ReplicaView;
#[derive(Clone)]
pub struct ReplicaConfig {
    pub node: NodeConfig,
    pub max_records: usize,
    pub max_snapshot_bytes: usize,
    pub contact_timeout: Duration,
}
impl ReplicaConfig {
    pub fn new(endpoint: String, identity: NodeCredentials) -> Self {
        Self {
            node: NodeConfig::new(endpoint, identity),
            max_records: 1_000_000,
            max_snapshot_bytes: 256 * 1024 * 1024,
            contact_timeout: Duration::from_secs(10),
        }
    }
    pub fn validate(&self) -> Result<(), Status> {
        self.node.validate()?;
        if self.contact_timeout < Duration::from_secs(3)
            || self.max_records == 0
            || self.max_snapshot_bytes < MAX_MESSAGE_BYTES
            || self.node.identity.role != w::NodeRole::Reader
        {
            return Err(Status::invalid_argument("invalid replica limits or role"));
        }
        Ok(())
    }
}
mod store;
pub use store::{ReplayBatch, ReplayEvent, ReplicaHandle, ReplicaSnapshot};
impl ReplicaHandle {
    pub async fn run(&self, config: ReplicaConfig, stop: CancellationToken) -> Result<(), Status> {
        config.validate()?;
        let mut reset = false;
        let mut backoff = Duration::from_millis(250);
        loop {
            let started = Instant::now();
            let result = tokio::select! {_=stop.cancelled()=>{self.disconnected();return Ok(());},result=self.follow(&config,reset)=>result};
            self.disconnected();
            if let Err(error) = result {
                reset = matches!(
                    error.code(),
                    Code::FailedPrecondition | Code::OutOfRange | Code::DataLoss
                );
                tracing::warn!(code=?error.code(),reason=%error.message(),"replica synchronization interrupted");
            }
            if started.elapsed() > Duration::from_secs(5) {
                backoff = Duration::from_millis(250);
            }
            let jitter = Duration::from_millis((Uuid::new_v4().as_u128() % 125) as u64);
            tokio::select! {_=stop.cancelled()=>return Ok(()),_=tokio::time::sleep(backoff+jitter)=>{}}
            backoff = (backoff * 2).min(Duration::from_secs(10));
        }
    }
    async fn follow(&self, config: &ReplicaConfig, reset: bool) -> Result<(), Status> {
        use market_reader::sync::{Limits, Reader, Update};
        let channel = config.node.connect().await?;
        let mut view = if reset { None } else { self.snapshot().view };
        let mut staging = view.is_none();
        let mut caught_up = false;
        let mut reader = Reader::connect(
            channel,
            &config.node.authorization()?,
            view.as_ref().map(|view| view.cursor.clone()),
            w::ChangeProjection::Live,
            Limits {
                max_records: config.max_records,
                max_record_bytes: config.max_snapshot_bytes,
                contact_timeout: config.contact_timeout,
                ..Default::default()
            },
        )
        .await?;
        loop {
            let update = reader
                .message()
                .await?
                .ok_or_else(|| Status::unavailable("replication stream closed"))?;
            match update {
                Update::Snapshot(snapshot) => {
                    let mut state =
                        market_query::MarketState::new(snapshot.catalog.catalog.clone());
                    let changes = market_query::PreparedChanges::new(
                        snapshot.catalog.catalog.clone(),
                        snapshot
                            .records
                            .into_iter()
                            .map(market_view::Mutation::Upsert)
                            .collect(),
                        |r| {
                            use prost::Message;
                            w::Record::from(r.clone()).encoded_len()
                        },
                    )
                    .map_err(|_| Status::data_loss("invalid snapshot rows"))?;
                    state
                        .apply_prepared(&changes)
                        .map_err(|_| Status::data_loss("invalid snapshot relationships"))?;
                    view = Some(Arc::new(ReplicaView {
                        catalog: snapshot.catalog,
                        state,
                        cursor: snapshot.cursor,
                    }));
                    staging = true;
                }
                Update::Delta(batch) => {
                    if batch.events.iter().any(|event| !event.is_live()) {
                        return Err(Status::data_loss("non-live event on live projection"));
                    }
                    let next = Arc::new(
                        view.as_ref()
                            .ok_or_else(|| Status::data_loss("delta without view"))?
                            .apply_batch(&batch, config)?,
                    );
                    if !staging {
                        self.activate_catalog(
                            next.clone(),
                            caught_up,
                            Some(batch.events),
                            batch.catalog,
                        );
                    }
                    view = Some(next);
                }
                Update::Checkpoint(cursor) | Update::Heartbeat(cursor) => {
                    let view = view
                        .as_ref()
                        .ok_or_else(|| Status::data_loss("checkpoint without view"))?;
                    if view.cursor != cursor {
                        return Err(Status::data_loss("checkpoint position mismatch"));
                    }
                    // The reader emits checkpoint before heartbeats on every new Sync.
                    caught_up = true;
                    staging = false;
                    self.activate(view.clone(), caught_up, None);
                }
            }
        }
    }
}
