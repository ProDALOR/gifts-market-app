//! Startup warming uses the catalog actor's bounded, typed model-document query.
use market_actors::MarketDataHandle;
use market_media::{DocumentId, MediaCache, MediaKey, MediaPriority};
use std::time::Duration;
pub(crate) async fn warmup(data: MarketDataHandle, cache: MediaCache) {
    while !data.status().await.is_ok_and(|status| status.ready) {
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let Ok(documents) = data.model_documents().await else {
        return;
    };
    let mut cached = 0;
    for document in documents {
        let Some(id) = DocumentId::new(document.get()) else {
            continue;
        };
        if cache
            .get_with_priority(MediaKey::thumbnail(id), MediaPriority::Prefetch)
            .await
            .is_ok()
        {
            cached += 1;
        }
    }
    tracing::info!(cached, "model thumbnail startup warming finished");
}
