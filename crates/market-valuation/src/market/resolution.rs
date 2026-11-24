//! Shared bounded external slug resolution for single and batched queries.
use super::*;
use std::{sync::Arc, time::Duration};
pub(crate) async fn external(
    input: &mut PreparedQuery,
    resolver: Option<Arc<dyn market_gifts::GiftResolver>>,
) -> Result<Option<ValuationError>, ValuationError> {
    let mut resolution_failure = None;
    if let Some(resolver) = resolver
        && let ValuationTarget::Gift {
            gift: market_gifts::GiftLookup::Slug(slug),
        } = &input.target
    {
        // Leave time for a local fallback within the shared request deadline.
        let resolved = tokio::time::timeout(Duration::from_secs(10), resolver.resolve(slug))
            .await
            .unwrap_or(Err(market_gifts::GiftResolutionError::Unavailable));
        match resolved {
            Ok(gift) if &gift.slug == slug && gift.number == slug.number() => {
                input.external_gift = Some(gift);
            }
            Ok(_) => {
                return Err(ValuationError::unavailable(
                    "Gift resolver returned a different gift",
                ));
            }
            Err(market_gifts::GiftResolutionError::NotFound) => {
                return Err(tonic::Status::not_found("Telegram gift slug was not found").into());
            }
            Err(error) => {
                resolution_failure = Some(ValuationError::unavailable(match error {
                    market_gifts::GiftResolutionError::Authorization => {
                        "Telegram gift lookup session is not authorized".to_owned()
                    }
                    market_gifts::GiftResolutionError::RateLimited(seconds) => {
                        format!("Telegram gift lookup is rate limited; retry in {seconds}s")
                    }
                    _ => "Telegram gift lookup is temporarily unavailable".to_owned(),
                }))
            }
        }
    }
    Ok(resolution_failure)
}
