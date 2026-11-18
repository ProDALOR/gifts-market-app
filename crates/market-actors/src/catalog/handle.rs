use super::CatalogActor;
use crate::runtime::{QueryBudget, closed};
use actix::Addr;
use tonic::Status;
#[derive(Clone)]
pub(crate) struct CatalogHandle {
    pub address: Addr<CatalogActor>,
    pub budget: QueryBudget,
    pub projections: std::sync::Arc<tokio::sync::Semaphore>,
}
impl CatalogHandle {
    pub async fn bootstrap(
        &self,
    ) -> Result<super::CatalogSnapshot<market_gifts::GiftCatalog>, Status> {
        let projection = self
            .projections
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("Catalog capacity reached"))?;
        let permit = self.budget.try_query()?;
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(super::messages::GiftCatalog { permit, projection })
            .await
            .map_err(closed)?
    }
    pub async fn collection(
        &self,
        id: market_gifts::CollectionId,
    ) -> Result<super::CatalogSnapshot<market_gifts::GiftCollectionCatalog>, Status> {
        let projection = self
            .projections
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("Catalog capacity reached"))?;
        let permit = self.budget.try_query()?;
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(super::messages::GiftCollection {
                id,
                permit,
                projection,
            })
            .await
            .map_err(closed)?
    }
    pub async fn versions(
        &self,
        ids: Vec<market_gifts::CollectionId>,
    ) -> Result<market_gifts::CatalogVersions, Status> {
        let projection = tokio::select! { biased;
            _ = self.budget.cancelled() => return Err(Status::unavailable("Catalog stopped")),
            result = self.projections.clone().acquire_owned() => result.map_err(|_| Status::unavailable("Catalog stopped"))?,
        };
        let permit = self.budget.wait().await?;
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(super::messages::Versions {
                ids,
                permit,
                projection,
            })
            .await
            .map_err(closed)?
    }
    pub async fn media_document(
        &self,
        target: market_gifts::MediaTarget,
    ) -> Result<market_gifts::TelegramDocumentId, Status> {
        let permit = self.budget.try_query()?;
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(super::messages::MediaDocument { target, permit })
            .await
            .map_err(closed)?
    }
    pub async fn model_documents(
        &self,
        limit: usize,
    ) -> Result<Vec<market_gifts::TelegramDocumentId>, Status> {
        if !(1..=1000).contains(&limit) {
            return Err(Status::invalid_argument(
                "model document limit must be 1..1000",
            ));
        }
        let permit = self.budget.try_query()?;
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(super::messages::ModelDocuments { limit, permit })
            .await
            .map_err(closed)?
    }
}
