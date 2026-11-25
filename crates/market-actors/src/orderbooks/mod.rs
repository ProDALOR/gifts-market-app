//! Current orderbooks, execution history, market freshness.
pub(crate) mod handle;
pub(crate) mod messages;
use crate::{query, runtime::ActorEnvironment};
use actix::prelude::*;
use market_replica::{ReplicaReader, SyncStatus};
use messages::GetStatus;
use tonic::Status;
pub(crate) struct OrderbooksActor {
    pub aliases: std::sync::Arc<crate::aliases::AliasCache>,
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
    type Result = ResponseFuture<Result<crate::OrderSnapshot, Status>>;
    fn handle(&mut self, message: messages::ReadRange, _: &mut Self::Context) -> Self::Result {
        let aliases = self.aliases.clone();
        self.environment.blocking(message.permit, move |reader| {
            use market_hub_protocol::wire as w;
            let gift_collections = std::cell::RefCell::new(None);
            // Move native requests directly into the read. Only aliases need deferred compilation.
            let (request, selectors) = match message.request {
                crate::query::OrderQuery::Native(request) => (request, None),
                crate::query::OrderQuery::Aliased(query) => {
                    (query::pin_request(&query), Some(query))
                }
            };
            reader
                .query_range_prepared(
                    request,
                    message.read,
                    message.complete,
                    |reader, request| {
                        let request = if let Some(query) = selectors {
                            let mut native = aliases.resolve(reader, &query)?;
                            native.mode = request.mode;
                            native
                        } else {
                            request
                        };
                        if let Some(w::selection::Source::Scope(scope)) =
                            request.selection.as_ref().and_then(|s| s.source.as_ref())
                            && scope.item_type_id.is_none()
                        {
                            *gift_collections.borrow_mut() = Some(aliases.get(reader)?);
                        }
                        query::validate_order_collection(reader, &request)?;
                        Ok(request)
                    },
                    |entity| {
                        if let Some(catalog) = gift_collections.borrow().as_ref()
                            && let Some(w::entity::Entity::Order(order)) = &entity.entity
                            && !order
                                .book
                                .as_ref()
                                .is_some_and(|b| catalog.contains_collection(&b.item_type_id))
                        {
                            return Ok(false);
                        }
                        crate::protocol::include_order(entity)
                    },
                )
                .and_then(crate::protocol::projected_read)
        })
    }
}
