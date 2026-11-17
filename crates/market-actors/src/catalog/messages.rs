use actix::Message;
#[cfg(test)]
use market_hub_protocol::wire as w;
#[cfg(test)]
use market_replica::ReadResult;
use tokio::sync::OwnedSemaphorePermit;
use tonic::Status;
#[derive(Message)]
#[rtype(result = "Result<ReadResult,Status>")]
#[cfg(test)]
pub(crate) struct ReadCatalog {
    pub request: w::ReadRequest,
    pub permit: OwnedSemaphorePermit,
}
#[derive(Message)]
#[rtype(result = "Result<super::CatalogSnapshot<market_gifts::GiftCatalog>,Status>")]
pub(crate) struct GiftCatalog {
    pub projection: OwnedSemaphorePermit,
    pub permit: OwnedSemaphorePermit,
}
#[derive(Message)]
#[rtype(result = "Result<super::CatalogSnapshot<market_gifts::GiftCollectionCatalog>,Status>")]
pub(crate) struct GiftCollection {
    pub projection: OwnedSemaphorePermit,
    pub id: market_gifts::CollectionId,
    pub permit: OwnedSemaphorePermit,
}
#[derive(Message)]
#[rtype(result = "Result<market_gifts::TelegramDocumentId,Status>")]
pub(crate) struct MediaDocument {
    pub target: market_gifts::MediaTarget,
    pub permit: OwnedSemaphorePermit,
}
#[derive(Message)]
#[rtype(result = "Result<Vec<market_gifts::TelegramDocumentId>,Status>")]
pub(crate) struct ModelDocuments {
    pub limit: usize,
    pub permit: OwnedSemaphorePermit,
}

#[derive(Message)]
#[rtype(result = "Result<market_gifts::CatalogVersions,Status>")]
pub(crate) struct Versions {
    pub ids: Vec<market_gifts::CollectionId>,
    pub permit: OwnedSemaphorePermit,
    pub projection: OwnedSemaphorePermit,
}
