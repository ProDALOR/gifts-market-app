use crate::ValuationSource;
use crate::test_support::MarketDataService;
use crate::*;
use actix::prelude::*;
use market_hub_protocol::wire as w;
use market_replica::ReplicaWriter;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
pub(crate) fn entity(value: w::entity::Entity) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Upsert(w::Entity {
            entity: Some(value),
        })),
    }
}
pub(crate) fn order(price: &str) -> w::Change {
    entity(w::entity::Entity::Order(w::Order {
        id: "order".into(),
        book: Some(w::BookKey {
            venue_id: "venue".into(),
            item_type_id: "gift".into(),
            currency_id: "GRAM".into(),
        }),
        mode: 1,
        price: price.into(),
        quantity: "1".into(),
        ..Default::default()
    }))
}
pub(crate) fn fixture() -> (tempfile::TempDir, ReplicaWriter) {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "test").unwrap();
    let mut builder = writer.begin_snapshot().unwrap();
    builder
        .append(&[
            entity(w::entity::Entity::Venue(w::Venue {
                id: "venue".into(),
                name: "Venue".into(),
            })),
            entity(w::entity::Entity::Currency(w::Currency {
                id: "GRAM".into(),
                name: "Gram".into(),
                decimals: 9,
            })),
            entity(w::entity::Entity::ItemType(w::ItemType {
                id: "gift".into(),
                name: "Gift".into(),
                aliases: vec![w::ExternalId {
                    namespace: "tg".into(),
                    id: "99".into(),
                }],
                quantity_step: "1".into(),
                ..Default::default()
            })),
            order("3"),
        ])
        .unwrap();
    writer
        .install_snapshot(builder.finish("c0").unwrap())
        .unwrap();
    (directory, writer)
}

struct SnapshotProbe {
    source: ValuationSource,
    life: crate::Lifecycle,
    entered: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    release: Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>>,
}
impl Actor for SnapshotProbe {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.life.started(ctx);
    }
}
type ProbeMessage =
    crate::Evaluate<(), (std::thread::ThreadId, std::thread::ThreadId), crate::ValuationError>;
impl Handler<ProbeMessage> for SnapshotProbe {
    type Result = AtomicResponse<
        Self,
        Result<
            Arc<crate::Mark<(std::thread::ThreadId, std::thread::ThreadId)>>,
            crate::ValuationError,
        >,
    >;
    fn handle(&mut self, message: ProbeMessage, _: &mut Self::Context) -> Self::Result {
        let (_, request) = message.into_parts();
        let source = self.source.clone();
        let entered = self.entered.clone();
        let release = self.release.clone();
        let stop = self.life.stop_token();
        AtomicResponse::new(Box::pin(async move{
            let _request=request;
            tokio::select!{biased;
                _=stop.cancelled()=>Err(ValuationError::unavailable("Stopping")),
                _=_request.cancelled.cancelled()=>Err(ValuationError::unavailable("Cancelled")),
                result=source.prepare(move|snapshot|{
                    if let Some(entered)=entered.lock().unwrap().take(){let _=entered.send(());}
                    if let Some(release)=release.lock().unwrap().take(){release.recv().unwrap();}
                    Ok(crate::Prepared {stale:false, prepared_at:jiff::Timestamp::now(),revision:snapshot.revision()?,data:std::thread::current().id()})
                })=>result?,
            }
        }.into_actor(self).map(|result,actor,_|result.map(|prepared|Arc::new(crate::Mark::evaluated(actor.life.strategy(),crate::Prepared {stale:false, prepared_at:jiff::Timestamp::now(),revision:prepared.revision,data:(std::thread::current().id(),prepared.data)}))))))
    }
}
#[tokio::test]
async fn strategy_actors_share_arbiter_and_read_only_views_while_preparation_runs_elsewhere() {
    let (_directory, mut writer) = fixture();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let snapshot = data.reader.snapshot().unwrap();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let first = crate::Service::start_on(
        &data.arbiter(),
        "snapshot_probe",
        {
            let source = data.reader.clone();
            move |life| SnapshotProbe {
                source,
                life,
                entered: Arc::new(std::sync::Mutex::new(Some(entered))),
                release: Arc::new(std::sync::Mutex::new(Some(blocked))),
            }
        },
        Default::default(),
    )
    .await
    .unwrap();
    let second = crate::Service::start_on(
        &data.arbiter(),
        "snapshot_probe",
        {
            let source = data.reader.clone();
            move |life| SnapshotProbe {
                source,
                life,
                entered: Default::default(),
                release: Default::default(),
            }
        },
        Default::default(),
    )
    .await
    .unwrap();
    let handle = first.handle.clone();
    let request = tokio::spawn(async move { handle.evaluate(()).await });
    started.await.unwrap();
    writer
        .apply(w::Batch {
            cursor: "newer".into(),
            changes: vec![order("7")],
        })
        .unwrap();
    assert_eq!(snapshot.revision().unwrap(), "c0");
    assert_eq!(data.reader.snapshot().unwrap().revision().unwrap(), "newer");
    assert!(
        tokio::time::timeout(Duration::from_secs(1), data.handle.status())
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    release.send(()).unwrap();
    let old = request.await.unwrap().unwrap();
    assert_eq!(old.revision, "c0");
    let current = second.handle.evaluate(()).await.unwrap();
    assert_eq!(current.revision, "newer");
    assert_eq!(old.data.0, current.data.0);
    assert_ne!(old.data.0, old.data.1);
    assert_ne!(current.data.0, current.data.1);
    first.task.shutdown().await.unwrap();
    second.task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
#[tokio::test]
async fn cancelled_strategy_preparation_retains_its_worker_until_shutdown_drains_it() {
    let (_directory, writer) = fixture();
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let strategy = crate::Service::start_on(
        &data.arbiter(),
        "snapshot_probe",
        {
            let source = data.reader.clone();
            move |life| SnapshotProbe {
                source,
                life,
                entered: Arc::new(std::sync::Mutex::new(Some(entered))),
                release: Arc::new(std::sync::Mutex::new(Some(blocked))),
            }
        },
        Default::default(),
    )
    .await
    .unwrap();
    let handle = strategy.handle.clone();
    let request = tokio::spawn(async move { handle.evaluate(()).await });
    started.await.unwrap();
    strategy.task.shutdown().await.unwrap();
    assert!(matches!(request.await.unwrap(), Err(crate::Error::Stopped)));
    let stop = CancellationToken::new();
    stop.cancel();
    let mut drain = tokio::spawn(data.task.run(stop));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut drain)
            .await
            .is_err()
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), drain)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

pub(crate) struct HeldWorker {
    _permits: Vec<market_replica::ReadPermit>,
}
pub(crate) async fn hold(source: &ValuationSource) -> HeldWorker {
    let budget = source.executor().budget();
    let mut permits = Vec::new();
    for _ in 0..budget.limit() {
        permits.push(budget.wait().await.unwrap());
    }
    HeldWorker { _permits: permits }
}
