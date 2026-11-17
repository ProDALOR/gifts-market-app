//! Current orderbooks, execution history, book directory and market freshness.
pub(crate) mod handle;
pub(crate) mod messages;
mod valuation;
use crate::{runtime::ActorEnvironment, selection};
use actix::prelude::*;
#[cfg(test)]
use market_hub_protocol::wire as w;
use market_replica::{ReadResult, ReplicaReader, SyncStatus};
#[cfg(test)]
use messages::ListBooks;
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
        if let Err(error) = selection::orders(&message.request) {
            return Box::pin(async move { Err(error) });
        }
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                reader.read_checked(message.request, crate::catalog::validate_order_collection)
            })
    }
}
#[cfg(test)]
impl Handler<ListBooks> for OrderbooksActor {
    type Result = ResponseFuture<Result<w::BooksPage, Status>>;
    fn handle(&mut self, message: ListBooks, _: &mut Self::Context) -> Self::Result {
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                reader.books(message.request.after, message.request.limit as usize)
            })
    }
}
impl Handler<GetStatus> for OrderbooksActor {
    type Result = Result<SyncStatus, Status>;
    fn handle(&mut self, _: GetStatus, _: &mut Self::Context) -> Self::Result {
        Ok(self.reader.status())
    }
}
impl Handler<messages::ValueModels> for OrderbooksActor {
    type Result = ResponseFuture<Result<market_gifts::valuation::Valuation, Status>>;
    fn handle(&mut self, message: messages::ValueModels, _: &mut Self::Context) -> Self::Result {
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                let _heavy = message.heavy;
                valuation::value(reader, message.request)
            })
    }
}
