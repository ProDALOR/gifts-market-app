//! Current orderbooks, execution history, market freshness.
pub(crate) mod handle;
pub(crate) mod messages;
use crate::{query, runtime::ActorEnvironment};
use actix::prelude::*;
use market_replica::{ReadResult, ReplicaReader, SyncStatus};
use messages::{GetStatus, ReadOrders};
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
impl Handler<ReadOrders> for OrderbooksActor {
    type Result = ResponseFuture<Result<ReadResult, Status>>;
    fn handle(&mut self, message: ReadOrders, _: &mut Self::Context) -> Self::Result {
        if let Err(error) = query::validate_read(&message.request) {
            return Box::pin(async move { Err(error) });
        }
        let stop = self.environment.stop.clone();
        self.environment.blocking(message.permit, move |reader| {
            reader.read_checked(
                message.request,
                query::validate_order_collection,
                |entity| {
                    if stop.is_cancelled() {
                        return Err(Status::unavailable("Market read cancelled"));
                    }
                    crate::protocol::include_order(entity)
                },
            )
        })
    }
}
impl Handler<GetStatus> for OrderbooksActor {
    type Result = Result<SyncStatus, Status>;
    fn handle(&mut self, _: GetStatus, _: &mut Self::Context) -> Self::Result {
        Ok(self.reader.status())
    }
}

impl Handler<messages::ReadWindow> for OrderbooksActor {
    type Result = ResponseFuture<Result<market_replica::WindowReadResult, Status>>;
    fn handle(&mut self, message: messages::ReadWindow, _: &mut Self::Context) -> Self::Result {
        if let Err(error) = query::validate_read(&message.request) {
            return Box::pin(async move { Err(error) });
        }
        self.environment.blocking(message.permit, move |reader| {
            reader.read_window_checked(
                message.request,
                message.window,
                query::validate_order_collection,
                crate::protocol::include_order,
            )
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
        self.reader.release_snapshot(&message.snapshot)
    }
}

impl Handler<messages::ReadRange> for OrderbooksActor {
    type Result = ResponseFuture<Result<market_replica::WindowReadResult, Status>>;
    fn handle(&mut self, message: messages::ReadRange, _: &mut Self::Context) -> Self::Result {
        if let Err(error) = query::validate_read(&message.request) {
            return Box::pin(async move { Err(error) });
        }
        self.environment.blocking(message.permit, move |reader| {
            reader.read_range_checked(
                message.request,
                message.range,
                query::validate_order_collection,
                crate::protocol::include_order,
            )
        })
    }
}
