use actix::Message;
use tokio::sync::OwnedSemaphorePermit;
use tonic::Status;
#[derive(Message)]
#[rtype(result = "Result<super::CatalogSnapshot<market_gifts::GiftCatalog>,Status>")]
pub(crate) struct GiftCatalog {
    pub projection: OwnedSemaphorePermit,
    pub permit: market_replica::ReadPermit,
}
#[derive(Message)]
#[rtype(result = "Result<super::CatalogSnapshot<market_gifts::GiftCollectionCatalog>,Status>")]
pub(crate) struct GiftCollection {
    pub projection: OwnedSemaphorePermit,
    pub id: market_gifts_hub::input::CollectionRef,
    pub permit: market_replica::ReadPermit,
}
#[derive(Message)]
#[rtype(result = "Result<market_gifts::TelegramDocumentId,Status>")]
pub(crate) struct MediaDocument {
    pub target: market_gifts::MediaTarget,
    pub permit: market_replica::ReadPermit,
}
#[derive(Message)]
#[rtype(result = "Result<market_gifts_hub::input::CatalogVersions,Status>")]
pub(crate) struct Versions {
    pub ids: Vec<market_gifts_hub::input::CollectionRef>,
    pub permit: market_replica::ReadPermit,
    pub projection: OwnedSemaphorePermit,
}

#[derive(Message)]
#[rtype(result = "Result<market_gifts::ResolvedGiftFilters,Status>")]
pub(crate) struct ResolveFilters {
    pub filters: market_gifts_hub::input::GiftFilters,
    pub permit: market_replica::ReadPermit,
}
