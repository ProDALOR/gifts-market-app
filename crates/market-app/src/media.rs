//! Bounded startup warming; GUI requests fill the remaining cache on demand.
use market_actors::{
    MarketDataHandle,
    models::{CatalogEntry, CatalogKind, CatalogQuery},
};
use market_media::{DocumentId, MediaCache, MediaKey, MediaPriority};
use std::{collections::BTreeSet, time::Duration};
pub(crate) async fn warmup(data: MarketDataHandle, cache: MediaCache) {
    loop {
        if data.status().await.is_ok_and(|status| status.ready) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let mut documents = BTreeSet::new();
    let mut token = String::new();
    loop {
        let Ok(page) = data
            .catalog(CatalogQuery {
                kind: CatalogKind::AttributeOption,
                ids: vec![],
                limit: 200,
                page_token: token,
            })
            .await
        else {
            return;
        };
        for entry in page.data {
            if let CatalogEntry::AttributeOption {
                attribute_id,
                aliases,
                ..
            } = entry
                && attribute_id == "tg.model"
                && let Some(id) = aliases
                    .iter()
                    .filter(|alias| alias.namespace == "tg")
                    .find_map(|alias| alias.id.parse().ok().and_then(DocumentId::new))
            {
                documents.insert(id.get());
                if documents.len() == 64 {
                    break;
                }
            }
        }
        if documents.len() >= 64 || page.next_page_token.is_empty() {
            break;
        }
        token = page.next_page_token;
    }
    let mut cached = 0;
    for id in documents {
        if cache
            .get_with_priority(
                MediaKey::thumbnail(DocumentId::new(id).unwrap()),
                MediaPriority::Prefetch,
            )
            .await
            .is_ok()
        {
            cached += 1;
        }
    }
    tracing::info!(cached, "model thumbnail startup warming finished");
}
