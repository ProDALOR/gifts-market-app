//! The same compact model/price grid for market search and comparable sales.
use super::*;
use market_gifts::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) type ModelDocuments = BTreeMap<ModelId, TelegramDocumentId>;

pub(super) fn traits(
    order: &GiftOrder,
) -> (Option<&ModelId>, Option<&BackdropId>, Option<&PatternId>) {
    match &order.target {
        GiftTarget::Gift { gift } => (
            gift.traits.model.as_ref(),
            gift.traits.backdrop.as_ref(),
            gift.traits.pattern.as_ref(),
        ),
        GiftTarget::Criteria { criteria } => (
            if criteria.models.len() == 1 {
                criteria.models.first()
            } else {
                None
            },
            if criteria.backdrops.len() == 1 {
                criteria.backdrops.first()
            } else {
                None
            },
            if criteria.patterns.len() == 1 {
                criteria.patterns.first()
            } else {
                None
            },
        ),
        _ => (None, None, None),
    }
}

fn model(order: &GiftOrder) -> Option<&ModelId> {
    traits(order).0
}
pub(super) fn number(order: &GiftOrder) -> Option<GiftNumber> {
    match &order.target {
        GiftTarget::Gift { gift } => gift.number,
        GiftTarget::Criteria { criteria } if order.quantity == 1 => criteria
            .number
            .as_ref()
            .and_then(|range| range.min.filter(|n| Some(*n) == range.max)),
        _ => None,
    }
}
fn venue(id: &str) -> String {
    match id.to_ascii_lowercase().as_str() {
        "mrkt" => "MRKT".into(),
        "tonnel" => "Tonnel".into(),
        "portals" => "Portals".into(),
        "getgems" => "Getgems".into(),
        "fragment" => "Fragment".into(),
        "telegram" => "Telegram".into(),
        _ => id.chars().take(24).collect(),
    }
}
impl MarketCommands {
    pub(super) async fn order_documents(
        &self,
        rows: &[market_actors::OrderedEntry],
        known: &ModelDocuments,
    ) -> ModelDocuments {
        let missing: BTreeSet<_> = rows
            .iter()
            .filter_map(|row| model(&row.order))
            .filter(|id| !known.contains_key(*id))
            .cloned()
            .collect();
        let mut tasks = tokio::task::JoinSet::new();
        for id in missing {
            let market = self.market.clone();
            tasks.spawn(async move {
                let document = market
                    .media_document(MediaTarget::Model(id.clone()))
                    .await
                    .ok();
                (id, document)
            });
        }
        let mut result = ModelDocuments::new();
        while let Some(completed) = tasks.join_next().await {
            if let Ok((id, Some(document))) = completed {
                result.insert(id, document);
            }
        }
        result
    }
}

pub(super) fn render<'a>(
    reply: &mut Reply,
    rows: impl IntoIterator<Item = &'a GiftOrder>,
    icons: &crate::ReportIcons,
    collections: &[GiftCollection],
    documents: &ModelDocuments,
    current: Option<(&CollectionId, &GiftSlug)>,
) {
    let rows = rows.into_iter().collect::<Vec<_>>();
    let multiple = rows.first().is_some_and(|first| {
        rows.iter()
            .any(|row| row.book.collection_id != first.book.collection_id)
    });
    let mut cells = Vec::with_capacity(rows.len());
    for order in rows {
        let collection = collections
            .iter()
            .find(|c| c.id == order.book.collection_id);
        let number = number(order);
        let same_collection = current.filter(|(id, _)| *id == &order.book.collection_id);
        let exact = same_collection.is_some_and(|(_, slug)| number == Some(slug.number()));
        let slug = number.and_then(|n| {
            let base = collection
                .and_then(|c| c.slug.as_ref().map(|s| s.as_str()))
                .or_else(|| same_collection.map(|(_, s)| s.collection().as_str()))?;
            GiftSlug::new(format!("{base}-{n}")).ok()
        });
        let gift_url = slug.as_ref().map(|slug| format!("https://t.me/nft/{slug}"));
        let market = order.market_url.as_deref().and_then(market_link);
        let image = model(order)
            .and_then(|id| documents.get(id))
            .copied()
            .map(|document| reply.custom_emoji(document))
            .unwrap_or_else(|| icons.html(crate::ReportIcon::Collection));
        let price = rich::amount(&order.price);
        let currency = crate::ReportIcons::currency(order.book.currency_id.as_str())
            .map(|icon| icons.html(icon))
            .unwrap_or_else(|| report::escape(order.book.currency_id.as_str()));
        let mut cell = rich::price(
            &image,
            &price,
            &currency,
            gift_url.as_deref().or(market.as_deref()),
        );
        let name = collection.map_or(order.book.collection_id.as_str(), |c| c.name.as_str());
        let mut label = match number {
            Some(number) if !multiple => format!("#{number}"),
            Some(number) => format!("{name} #{number}"),
            None => name.to_owned(),
        };
        if order.quantity > 1 {
            label.push_str(&format!(" × {}", order.quantity));
        }
        let label_html = if exact {
            format!(
                "{} <b>{}</b>",
                icons.html(crate::ReportIcon::CurrentGift),
                report::escape(&label)
            )
        } else {
            report::escape(&label)
        };
        let venue = venue(order.book.venue_id.as_str());
        let venue_html = market
            .as_ref()
            .map(|url| {
                format!(
                    "<a href=\"{}\">{}</a>",
                    report::escape(url),
                    report::escape(&venue)
                )
            })
            .unwrap_or_else(|| report::escape(&venue));
        let when = order
            .executed_at
            .map(|time| time.strftime("%d %b %H:%M").to_string());
        cell.push_str(&format!("<br>{label_html} · {venue_html}"));
        if let Some(when) = &when {
            cell.push_str(&format!("<br>{when}"));
        }
        cells.push(cell);
        reply.text.push_str(&format!(
            "\n{}{label} · {price} {}{} · {venue}{}",
            if exact {
                format!("{} ", icons.text(crate::ReportIcon::CurrentGift))
            } else {
                String::new()
            },
            order.book.currency_id,
            when.map(|when| format!(" · {when} UTC"))
                .unwrap_or_default(),
            market.map(|url| format!(" · {url}")).unwrap_or_default()
        ));
    }
    if !cells.is_empty() {
        reply
            .rich_html
            .get_or_insert_default()
            .push_str(&rich::grid(
                &cells,
                if current.is_some() { 2 } else { rich::COLUMNS },
            ));
    }
}
