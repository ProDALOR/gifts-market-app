//! Current orderbooks, execution history, market freshness.
pub(crate) mod handle;
pub(crate) mod messages;
use crate::{query, runtime::ActorEnvironment};
use actix::prelude::*;
use market_replica::{ReplicaReader, SyncStatus};
use messages::GetStatus;
use tonic::Status;
pub(crate) struct OrderbooksActor {
    pub reader: ReplicaReader,
    pub environment: ActorEnvironment,
}
impl Actor for OrderbooksActor {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.environment.started(ctx);
    }
}
impl Drop for OrderbooksActor {
    fn drop(&mut self) {
        self.environment.stopped();
    }
}
impl Handler<GetStatus> for OrderbooksActor {
    type Result = Result<SyncStatus, Status>;
    fn handle(&mut self, _: GetStatus, _: &mut Self::Context) -> Self::Result {
        Ok(self.reader.status())
    }
}

impl Handler<messages::ReadSlice> for OrderbooksActor {
    type Result = ResponseFuture<Result<(crate::OrderRead, market_replica::ReadRevision), Status>>;
    fn handle(&mut self, message: messages::ReadSlice, _: &mut Self::Context) -> Self::Result {
        self.environment.blocking(message.permit, move |reader| {
            reader
                .read_slice_checked(
                    message.request,
                    message.slice,
                    query::validate_order_collection,
                    crate::protocol::include_order,
                )
                .and_then(crate::protocol::projected_read)
        })
    }
}

impl Handler<messages::ReleaseSnapshot> for OrderbooksActor {
    type Result = Result<(), Status>;
    fn handle(
        &mut self,
        message: messages::ReleaseSnapshot,
        _: &mut Self::Context,
    ) -> Self::Result {
        self.reader.release_snapshot(message.snapshot)
    }
}

impl Handler<messages::ReadRange> for OrderbooksActor {
    type Result = ResponseFuture<Result<(crate::OrderRead, market_replica::ReadRevision), Status>>;
    fn handle(&mut self, message: messages::ReadRange, _: &mut Self::Context) -> Self::Result {
        self.environment.blocking(message.permit, move |reader| {
            reader
                .read_range_checked(
                    message.request,
                    message.range,
                    query::validate_order_collection,
                    crate::protocol::include_order,
                )
                .and_then(crate::protocol::projected_read)
        })
    }
}
