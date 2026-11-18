//! One concrete gift, every registered strategy, independent typed outcomes.
use crate::*;
use market_gifts::{valuation::*, *};
#[derive(Debug, Clone)]
pub struct GiftStrategyResult {
    pub strategy: StrategyId,
    pub result: Result<ValuationResult, ValuationError>,
}
#[derive(Debug, Clone)]
pub struct GiftValuationResult {
    pub slug: GiftSlug,
    pub gift: CollectibleGift,
    pub currency_id: CurrencyId,
    pub gift_revision: String,
    pub strategies: Vec<GiftStrategyResult>,
}
impl ValuationHandle {
    pub async fn gift(
        &self,
        request: GiftValuationRequest,
    ) -> Result<GiftValuationResult, ValuationError> {
        let _slot = self
            .gift_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ValuationError::capacity("Gift valuation requests are busy"))?;
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| ValuationError::unavailable("Gift lookup is unavailable"))?;
        let work = async {
            let slug = request.gift.clone();
            let currency = request.currency_id.clone();
            let lookup = reader.prepare(
                move |snapshot| -> Result<_, market_actors::error::DataError> {
                    snapshot.require_currency(&currency)?;
                    Ok((snapshot.gift_by_slug(&slug)?, snapshot.revision()?))
                },
            );
            let (gift, gift_revision) =
                tokio::time::timeout(std::time::Duration::from_secs(10), lookup)
                    .await
                    .map_err(|_| ValuationError::timed_out("Gift lookup timed out"))???;
            let strategies =
                futures_util::future::join_all(self.strategies().cloned().map(|strategy| {
                    let gift = &gift;
                    let currency = &request.currency_id;
                    let revision = &gift_revision;
                    async move {
                        let result = match query(&strategy, gift, currency, revision) {
                            Ok(query) => tokio::time::timeout(
                                std::time::Duration::from_secs(15),
                                self.query(query),
                            )
                            .await
                            .unwrap_or_else(|_| {
                                Err(ValuationError::timed_out("Strategy evaluation timed out")
                                    .with_context(strategy.clone(), currency.clone()))
                            }),
                            Err(error) => {
                                Err(error.with_context(strategy.clone(), currency.clone()))
                            }
                        };
                        GiftStrategyResult { strategy, result }
                    }
                }))
                .await;
            Ok(GiftValuationResult {
                slug: request.gift,
                gift,
                currency_id: request.currency_id,
                gift_revision,
                strategies,
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(30), work)
            .await
            .map_err(|_| ValuationError::timed_out("Gift valuation timed out"))?
    }
}
fn query(
    id: &StrategyId,
    gift: &CollectibleGift,
    currency: &CurrencyId,
    revision: &str,
) -> Result<ValuationQuery, ValuationError> {
    let missing = |name| {
        ValuationError::with_details(
            ValuationErrorCode::NotEnoughData,
            format!("Gift has no resolved catalog {name}"),
            ValuationErrorDetails {
                currency_id: Some(currency.clone()),
                revision: Some(revision.to_owned()),
                ..ValuationErrorDetails::strategy(id.clone())
            },
        )
    };
    Ok(match id.as_str() {
        "collection_floor" | "model_floor" => {
            let model = if id.as_str() == "collection_floor" {
                ModelLookup::Collection {
                    collection: gift.collection_id.clone(),
                }
            } else {
                ModelLookup::Catalog {
                    collection: gift.collection_id.clone(),
                    model: gift.traits.model.clone().ok_or_else(|| missing("model"))?,
                }
            };
            let positions = PortfolioPositions {
                positions: vec![ModelPosition { model, quantity: 1 }],
                currency_id: currency.clone(),
            };
            if id.as_str() == "model_floor" {
                ValuationQuery::ModelFloor(positions)
            } else {
                ValuationQuery::CollectionFloor(positions)
            }
        }
        "model_multiplier" => ValuationQuery::ModelMultiplier(ModelMultiplierRequest {
            collection_id: gift.collection_id.clone(),
            model_id: gift.traits.model.clone().ok_or_else(|| missing("model"))?,
            currency_id: currency.clone(),
        }),
        "backdrop_multiplier" => ValuationQuery::BackdropMultiplier(BackdropMultiplierRequest {
            collection_id: gift.collection_id.clone(),
            backdrop_id: gift
                .traits
                .backdrop
                .clone()
                .ok_or_else(|| missing("backdrop"))?,
            currency_id: currency.clone(),
        }),
        _ => {
            return Err(ValuationError::unavailable(
                "Unsupported gift valuation strategy",
            ));
        }
    })
}
