//! Gift catalog projections and internal reference lookups.
pub(crate) mod cache;
pub(crate) mod handle;
pub(crate) mod messages;
pub(crate) mod projection;
mod resolve;
mod version;
use crate::runtime::ActorEnvironment;
#[cfg(test)]
use crate::selection;
use actix::prelude::*;
#[cfg(test)]
use market_replica::ReadResult;
use market_replica::ReplicaReader;
#[cfg(test)]
use messages::ReadCatalog;
pub use resolve::ResolvedModel;
pub(crate) use resolve::resolve as resolve_models;
use tonic::Status;
pub use version::CatalogSnapshot;
pub(crate) struct CatalogActor {
    pub cache: std::sync::Arc<cache::ProjectionCache>,
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
#[cfg(test)]
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
impl Handler<messages::GiftCatalog> for CatalogActor {
    type Result = ResponseFuture<Result<CatalogSnapshot<market_gifts::GiftCatalog>, Status>>;
    fn handle(&mut self, message: messages::GiftCatalog, _: &mut Self::Context) -> Self::Result {
        let cache = self.cache.clone();
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                let _projection = message.projection;
                cache.bootstrap_snapshot(reader)
            })
    }
}
impl Handler<messages::GiftCollection> for CatalogActor {
    type Result =
        ResponseFuture<Result<CatalogSnapshot<market_gifts::GiftCollectionCatalog>, Status>>;
    fn handle(&mut self, message: messages::GiftCollection, _: &mut Self::Context) -> Self::Result {
        let cache = self.cache.clone();
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                let _projection = message.projection;
                cache.collection_snapshot(reader, message.id)
            })
    }
}
impl Handler<messages::MediaDocument> for CatalogActor {
    type Result = ResponseFuture<Result<market_gifts::TelegramDocumentId, Status>>;
    fn handle(&mut self, message: messages::MediaDocument, _: &mut Self::Context) -> Self::Result {
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                projection::media_document(reader, message.target)
            })
    }
}
impl Handler<messages::ModelDocuments> for CatalogActor {
    type Result = ResponseFuture<Result<Vec<market_gifts::TelegramDocumentId>, Status>>;
    fn handle(&mut self, message: messages::ModelDocuments, _: &mut Self::Context) -> Self::Result {
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                projection::model_documents(reader, message.limit)
            })
    }
}

pub(crate) fn validate_order_collection(
    reader: &ReplicaReader,
    request: &market_hub_protocol::wire::ReadRequest,
) -> Result<(), Status> {
    use market_hub_protocol::wire::selection::Source;
    if let Some(Source::Scope(scope)) = request
        .selection
        .as_ref()
        .and_then(|selection| selection.source.as_ref())
        && let Some(id) = &scope.item_type_id
    {
        projection::validate_collection(reader, id)?;
    }
    Ok(())
}

impl Handler<messages::Versions> for CatalogActor {
    type Result = ResponseFuture<Result<market_gifts::CatalogVersions, Status>>;
    fn handle(&mut self, message: messages::Versions, _: &mut Self::Context) -> Self::Result {
        let cache = self.cache.clone();
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                let _projection = message.projection;
                cache.versions(reader, message.ids)
            })
    }
}
