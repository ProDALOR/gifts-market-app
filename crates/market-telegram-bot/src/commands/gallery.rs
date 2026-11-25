//! Resolve artwork only for the requested page; media owns rendering and disk caching.
use super::orders::{self, traits};
use super::*;
use market_gifts::*;
use market_media::{Artwork, CoverPalette, DocumentId, GalleryCell, GiftCover, GiftGallery};
use std::collections::{BTreeMap, BTreeSet};

fn palette(
    catalog: Option<&GiftCollectionCatalog>,
    id: Option<&BackdropId>,
) -> Option<CoverPalette> {
    let palette = catalog?
        .backdrops
        .iter()
        .find(|v| Some(&v.id) == id)?
        .palette
        .as_ref()?;
    let color = |v: &RgbColor| {
        u32::from_str_radix(v.to_string().trim_start_matches('#'), 16).expect("validated RGB")
    };
    Some(CoverPalette {
        center: color(&palette.center),
        edge: color(&palette.edge),
        pattern: color(&palette.pattern),
    })
}
impl MarketCommands {
    async fn gallery_catalogs(
        &self,
        ids: BTreeSet<CollectionId>,
        known: Option<&GiftCollectionCatalog>,
    ) -> BTreeMap<CollectionId, GiftCollectionCatalog> {
        let mut result = BTreeMap::new();
        if let Some(catalog) = known {
            result.insert(catalog.collection.id.clone(), catalog.clone());
        }
        let mut tasks = tokio::task::JoinSet::new();
        for id in ids.into_iter().filter(|id| !result.contains_key(id)) {
            let market = self.market.clone();
            tasks.spawn(async move {
                market
                    .collection_snapshot(id.into())
                    .await
                    .ok()
                    .map(|v| v.data)
            });
        }
        while let Some(loaded) = tasks.join_next().await {
            if let Ok(Some(catalog)) = loaded {
                result.insert(catalog.collection.id.clone(), catalog);
            }
        }
        result
    }
    pub(super) async fn order_gallery(
        &self,
        rows: &[market_actors::OrderedEntry],
        documents: &orders::ModelDocuments,
        known: Option<&GiftCollectionCatalog>,
        columns: u8,
    ) -> Artwork {
        let catalogs = self
            .gallery_catalogs(
                rows.iter()
                    .map(|row| row.order.book.collection_id.clone())
                    .collect(),
                known,
            )
            .await;
        let ids: BTreeSet<_> = rows
            .iter()
            .filter_map(|row| {
                let (model, backdrop, pattern) = traits(&row.order);
                (model.and_then(|id| documents.get(id)).is_some()
                    && palette(catalogs.get(&row.order.book.collection_id), backdrop).is_some())
                .then_some(pattern)
                .flatten()
                .cloned()
            })
            .collect();
        let mut tasks = tokio::task::JoinSet::new();
        for id in ids {
            let market = self.market.clone();
            tasks.spawn(async move {
                let document = market
                    .media_document(MediaTarget::Pattern(id.clone()))
                    .await
                    .ok()
                    .and_then(|d| DocumentId::new(d.get()));
                (id, document)
            });
        }
        let mut patterns = BTreeMap::new();
        while let Some(result) = tasks.join_next().await {
            if let Ok((id, Some(document))) = result {
                patterns.insert(id, document);
            }
        }
        order_cells(rows, documents, &catalogs, &patterns, columns)
    }
    pub(super) async fn portfolio_gallery(
        &self,
        items: &[market_valuation::InventoryValue],
        currency: BotCurrency,
    ) -> Artwork {
        let catalogs = self
            .gallery_catalogs(
                items
                    .iter()
                    .filter_map(|item| item.resolved.as_ref().map(|g| g.collection_id.clone()))
                    .collect(),
                None,
            )
            .await;
        let cells = items
            .iter()
            .take(market_media::MAX_GALLERY_CELLS)
            .map(|item| {
                let resolved = item.resolved.as_ref();
                let catalog = resolved.and_then(|g| catalogs.get(&g.collection_id));
                let title = resolved
                    .and_then(|g| g.traits.model.as_ref())
                    .and_then(|id| catalog?.models.iter().find(|m| &m.id == id))
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| item.gift.gift.slug.collection().to_string());
                GalleryCell {
                    cover: DocumentId::new(item.gift.gift.model.get()).map(|model| GiftCover {
                        model,
                        pattern: item
                            .gift
                            .gift
                            .pattern
                            .and_then(|d| DocumentId::new(d.get())),
                        palette: palette(
                            catalog,
                            resolved.and_then(|g| g.traits.backdrop.as_ref()),
                        ),
                    }),
                    title,
                    identity: format!("#{}", item.gift.gift.number.get()),
                    price: format!(
                        "{} {currency}",
                        item.estimate
                            .as_ref()
                            .map(rich::amount)
                            .unwrap_or_else(|| "—".into())
                    ),
                }
            })
            .collect();
        Artwork::Gallery(GiftGallery { columns: 3, cells })
    }
}

pub(super) fn order_cells(
    rows: &[market_actors::OrderedEntry],
    documents: &orders::ModelDocuments,
    catalogs: &BTreeMap<CollectionId, GiftCollectionCatalog>,
    patterns: &BTreeMap<PatternId, DocumentId>,
    columns: u8,
) -> Artwork {
    let mut cells = Vec::with_capacity(rows.len());
    for row in rows.iter().take(market_media::MAX_GALLERY_CELLS) {
        let order = &row.order;
        let catalog = catalogs.get(&order.book.collection_id);
        let (model, backdrop, pattern) = traits(order);
        let model_document = model
            .and_then(|m| documents.get(m))
            .and_then(|d| DocumentId::new(d.get()));
        let pattern_document = pattern.and_then(|id| patterns.get(id)).copied();
        let title = model
            .and_then(|id| catalog?.models.iter().find(|m| &m.id == id))
            .map(|m| m.name.clone())
            .unwrap_or_else(|| {
                catalog
                    .map(|c| c.collection.name.clone())
                    .unwrap_or_else(|| "Gift".into())
            });
        cells.push(GalleryCell {
            cover: model_document.map(|model| GiftCover {
                model,
                pattern: pattern_document,
                palette: palette(catalog, backdrop),
            }),
            title,
            identity: orders::number(order)
                .map(|n| format!("#{}", n.get()))
                .unwrap_or_else(|| format!("× {}", order.quantity)),
            price: format!("{} {}", rich::amount(&order.price), order.book.currency_id),
        });
    }
    Artwork::Gallery(GiftGallery { columns, cells })
}
