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
    pub id: market_gifts::CollectionId,
    pub permit: market_replica::ReadPermit,
}
#[derive(Message)]
#[rtype(result = "Result<market_gifts::TelegramDocumentId,Status>")]
pub(crate) struct MediaDocument {
    pub target: market_gifts::MediaTarget,
    pub permit: market_replica::ReadPermit,
}
#[derive(Message)]
#[rtype(result = "Result<market_gifts::CatalogVersions,Status>")]
pub(crate) struct Versions {
    pub ids: Vec<market_gifts::CollectionId>,
    pub permit: market_replica::ReadPermit,
    pub projection: OwnedSemaphorePermit,
}
