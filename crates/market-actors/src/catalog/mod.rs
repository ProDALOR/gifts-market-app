//! Reference definitions: venues, currencies, collections, attributes and feeds.
pub(crate) mod handle;
pub(crate) mod messages;
mod resolve;
use crate::{runtime::ActorEnvironment, selection};
use actix::prelude::*;
use market_replica::{ReadResult, ReplicaReader};
use messages::{ReadCatalog, ResolveModels};
use tonic::Status;
pub(crate) struct CatalogActor {
    pub reader: ReplicaReader,
    pub environment: ActorEnvironment,
}
impl Actor for CatalogActor {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.environment.started(ctx);
    }
}
impl Drop for CatalogActor {
    fn drop(&mut self) {
        self.environment.stopped();
    }
}
impl Handler<ReadCatalog> for CatalogActor {
    type Result = ResponseFuture<Result<ReadResult, Status>>;
    fn handle(&mut self, message: ReadCatalog, _: &mut Self::Context) -> Self::Result {
        if let Err(error) = selection::catalog(&message.request) {
            return Box::pin(async move { Err(error) });
        }
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                reader.read(message.request)
            })
    }
}
impl Handler<ResolveModels> for CatalogActor {
    type Result = ResponseFuture<Result<Vec<crate::valuation::ResolvedPosition>, Status>>;
    fn handle(&mut self, message: ResolveModels, _: &mut Self::Context) -> Self::Result {
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                resolve::resolve(reader, message.positions)
            })
    }
}
