//! Venue/currency definitions and available gift books.
use super::read::{entities, invalid};
use market_gifts::*;
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use std::{collections::BTreeSet, sync::Arc};
use tonic::Status;
pub(crate) fn venues(reader: &ReplicaReader) -> Result<Arc<Vec<Venue>>, Status> {
    let venues = entities(reader, w::EntityKind::Venue, None)?
        .into_iter()
        .filter_map(|row| {
            if let w::entity::Entity::Venue(v) = row {
                Some(v)
            } else {
                None
            }
        })
        .map(|v| {
            Ok(Venue {
                id: v.id.try_into().map_err(invalid)?,
                name: v.name,
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;
    Ok(venues.into())
}
pub(crate) fn currencies(reader: &ReplicaReader) -> Result<Arc<Vec<Currency>>, Status> {
    let currencies = entities(reader, w::EntityKind::Currency, None)?
        .into_iter()
        .filter_map(|row| {
            if let w::entity::Entity::Currency(v) = row {
                Some(v)
            } else {
                None
            }
        })
        .map(|v| {
            Ok(Currency {
                id: v.id.try_into().map_err(invalid)?,
                name: v.name,
                decimals: v.decimals,
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;
    Ok(currencies.into())
}
pub(crate) fn books(
    reader: &ReplicaReader,
    collections: &[GiftCollection],
) -> Result<Arc<Vec<GiftBook>>, Status> {
    let collections: BTreeSet<_> = collections.iter().map(|c| c.id.as_str()).collect();
    let mut books = Vec::new();
    let mut after = None;
    loop {
        let page = reader.books(after, 500)?;
        for book in page.books {
            if collections.contains(book.item_type_id.as_str()) {
                books.push(crate::protocol::book(book).map_err(invalid)?);
            }
        }
        if books.len() > 50_000 {
            return Err(Status::resource_exhausted(
                "gift directory exceeds response budget",
            ));
        }
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    Ok(books.into())
}
