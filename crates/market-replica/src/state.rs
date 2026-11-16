use crate::book_index::BookIndex;
pub use crate::types::Key;
use crate::types::{EntityKind, OrderMode};
use im::OrdMap;
use market_hub_protocol::wire as w;
use rust_decimal::Decimal;
use std::sync::Arc;
use tonic::Status;

pub fn entity_key(entity: &w::Entity) -> Result<Key, Status> {
    use w::entity::Entity as E;
    let (kind, id, venue) = match entity
        .entity
        .as_ref()
        .ok_or_else(|| Status::data_loss("empty entity"))?
    {
        E::Venue(v) => (EntityKind::Venue, &v.id, ""),
        E::Currency(v) => (EntityKind::Currency, &v.id, ""),
        E::ItemType(v) => (EntityKind::ItemType, &v.id, ""),
        E::Attribute(v) => (EntityKind::Attribute, &v.id, ""),
        E::AttributeOption(v) => (EntityKind::AttributeOption, &v.id, ""),
        E::Feed(v) => (EntityKind::Feed, &v.id, ""),
        E::Order(v) => (
            EntityKind::Order,
            &v.id,
            v.book
                .as_ref()
                .ok_or_else(|| Status::data_loss("missing order book"))?
                .venue_id
                .as_str(),
        ),
    };
    Ok(Key {
        kind,
        venue_id: venue.into(),
        id: id.clone(),
    })
}
pub fn order(entity: &w::Entity) -> Option<&w::Order> {
    match entity.entity.as_ref()? {
        w::entity::Entity::Order(order) => Some(order),
        _ => None,
    }
}
pub fn decimal(value: &str) -> Result<Decimal, Status> {
    if value.len() > 128 {
        return Err(Status::invalid_argument("decimal too long"));
    }
    Decimal::from_str_exact(value)
        .map(|v| v.normalize())
        .map_err(|_| Status::invalid_argument("invalid exact decimal"))
}
#[derive(Clone, Default)]
pub struct State {
    pub entities: OrdMap<Key, Arc<w::Entity>>,
    pub books: BookIndex,
}
impl State {
    pub fn apply(&mut self, changes: &[w::Change]) -> Result<(), Status> {
        for change in changes {
            match &change.change {
                Some(w::change::Change::Upsert(entity)) => self.upsert(entity.clone())?,
                Some(w::change::Change::Remove(remove)) => self.remove(&Key::try_from(
                    remove
                        .key
                        .as_ref()
                        .ok_or_else(|| Status::data_loss("missing removal key"))?,
                )?)?,
                None => return Err(Status::data_loss("empty change")),
            }
        }
        Ok(())
    }
    pub fn get(&self, kind: EntityKind, id: &str) -> Option<&w::entity::Entity> {
        self.entities
            .get(&Key {
                kind,
                venue_id: String::new(),
                id: id.into(),
            })?
            .entity
            .as_ref()
    }
    pub fn upsert(&mut self, entity: w::Entity) -> Result<(), Status> {
        let key = entity_key(&entity)?;
        self.remove(&key)?;
        if order(&entity)
            .map(|order| OrderMode::try_from(order.mode))
            .transpose()?
            == Some(OrderMode::Executed)
        {
            return Ok(());
        }
        self.books.insert(&key, &entity)?;
        self.entities.insert(key, Arc::new(entity));
        Ok(())
    }
    pub fn remove(&mut self, key: &Key) -> Result<(), Status> {
        if let Some(entity) = self.entities.remove(key) {
            self.books.remove(key, &entity)?;
        }
        Ok(())
    }
}
