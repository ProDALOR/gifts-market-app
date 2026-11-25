//! Gift catalog projections and internal reference lookups.
pub(crate) mod cache;
pub(crate) mod handle;
pub(crate) mod messages;
pub(crate) mod projection;
mod version;
use crate::runtime::ActorEnvironment;
use actix::prelude::*;
use tonic::Status;
pub use version::CatalogSnapshot;
pub(crate) struct CatalogActor {
    pub aliases: std::sync::Arc<crate::aliases::AliasCache>,
    pub cache: std::sync::Arc<cache::ProjectionCache>,
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
impl Handler<messages::GiftCatalog> for CatalogActor {
    type Result = ResponseFuture<Result<CatalogSnapshot<market_gifts::GiftCatalog>, Status>>;
    fn handle(&mut self, message: messages::GiftCatalog, _: &mut Self::Context) -> Self::Result {
        let cache = self.cache.clone();
        self.environment.blocking(message.permit, move |reader| {
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
        let aliases = self.aliases.clone();
        self.environment.blocking(message.permit, move |reader| {
            let _projection = message.projection;
            let reader = reader.snapshot()?;
            let id = if message.id.is_alias() {
                aliases
                    .get(&reader)?
                    .collection(&message.id)
                    .map_err(crate::aliases::alias_status)?
            } else {
                message.id.into_id().map_err(crate::aliases::alias_status)?
            };
            cache.collection_snapshot(reader, id)
        })
    }
}
impl Handler<messages::MediaDocument> for CatalogActor {
    type Result = ResponseFuture<Result<market_gifts::TelegramDocumentId, Status>>;
    fn handle(&mut self, message: messages::MediaDocument, _: &mut Self::Context) -> Self::Result {
        self.environment.blocking(message.permit, move |reader| {
            projection::media_document(reader, message.target)
        })
    }
}

impl Handler<messages::Versions> for CatalogActor {
    type Result = ResponseFuture<Result<market_gifts_hub::input::CatalogVersions, Status>>;
    fn handle(&mut self, message: messages::Versions, _: &mut Self::Context) -> Self::Result {
        let cache = self.cache.clone();
        let aliases = self.aliases.clone();
        self.environment.blocking(message.permit, move |reader| {
            let _projection = message.projection;
            if !message.ids.iter().any(|id| id.is_alias()) {
                return cache.versions(
                    reader,
                    message
                        .ids
                        .into_iter()
                        .map(|id| id.into_id().map_err(crate::aliases::alias_status))
                        .collect::<Result<_, _>>()?,
                );
            }
            let reader = reader.snapshot()?;
            let aliases = aliases.get(&reader)?;
            let pairs = message
                .ids
                .into_iter()
                .map(|id| {
                    let native = if id.is_alias() {
                        aliases
                            .collection_if_known(&id)
                            .map_err(crate::aliases::alias_status)?
                    } else {
                        Some(id.clone().into_id().map_err(crate::aliases::alias_status)?)
                    };
                    Ok((id, native))
                })
                .collect::<Result<Vec<_>, Status>>()?;
            let mut versions = cache.versions(
                reader,
                pairs
                    .iter()
                    .filter_map(|(_, native)| native.clone())
                    .collect(),
            )?;
            versions.collections = pairs
                .into_iter()
                .map(|(id, native)| {
                    let version = native
                        .as_ref()
                        .and_then(|native| versions.collections.get(&native.clone().into()))
                        .cloned()
                        .flatten();
                    (id, version)
                })
                .collect();
            Ok(versions)
        })
    }
}

impl Handler<messages::ResolveFilters> for CatalogActor {
    type Result = ResponseFuture<Result<market_gifts::ResolvedGiftFilters, Status>>;
    fn handle(&mut self, message: messages::ResolveFilters, _: &mut Self::Context) -> Self::Result {
        let aliases = self.aliases.clone();
        self.environment.blocking(message.permit, move |reader| {
            let reader = reader.snapshot()?;
            aliases
                .get(&reader)?
                .filters(&message.filters)
                .map_err(crate::aliases::alias_status)
        })
    }
}
