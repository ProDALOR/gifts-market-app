//! Generic revision keys survive coalesced notifications without retaining an event log.
use crate::{
    ReplicaReader, ReplicaView,
    state::{self, State},
    types::{EntityKind, Key},
};
use im::OrdMap;
use market_hub_protocol::wire as w;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tonic::Status;

type Book<'a> = (&'a str, &'a str, &'a str);
type Scope = (String, String, Option<String>);
#[derive(Clone, Default)]
pub(crate) struct Revisions {
    pub instance: uuid::Uuid,
    pub sequence: u64,
    pub dataset: u64,
    pub catalog: u64,
    history: u64,
    pub directory: u64,
    metadata: [u64; 5],
    option_groups: OrdMap<String, OptionGroup>,
    books: OrdMap<Scope, u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadRevision {
    instance: uuid::Uuid,
    dataset: u64,
    catalog: u64,
    data: u64,
}
impl ReadRevision {
    pub fn dataset_token(&self) -> String {
        format!("{}:{}", self.instance, self.dataset)
    }
    pub fn same_dataset(&self, other: &Self) -> bool {
        self.instance == other.instance && self.dataset == other.dataset
    }
    /// Compare only dataset and selected data; callers must track metadata dependencies separately.
    pub fn same_data(&self, other: &Self) -> bool {
        self.instance == other.instance && self.dataset == other.dataset && self.data == other.data
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataRevision {
    instance: uuid::Uuid,
    dataset: u64,
    kinds: [u64; 5],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryRevision {
    instance: uuid::Uuid,
    dataset: u64,
    directory: u64,
}

#[derive(Clone)]
struct OptionGroup {
    count: usize,
    revision: u64,
}
/// Exact reference dependencies of a projection, including absent keys and unrestricted options.
/// Only selected entity payloads are retained; this never retains a whole replica view.
pub struct MetadataDependencies {
    instance: uuid::Uuid,
    dataset: u64,
    entities: Vec<(Key, Option<Arc<w::Entity>>)>,
    option_groups: Vec<(String, u64)>,
}
impl MetadataDependencies {
    /// Retained key count, including missing references, for bounded projection caches.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }
}
fn option_group(entity: &w::Entity) -> Option<&str> {
    match entity.entity.as_ref()? {
        w::entity::Entity::AttributeOption(option) => Some(&option.attribute_id),
        _ => None,
    }
}

fn book(value: &w::BookKey) -> Book<'_> {
    (&value.item_type_id, &value.currency_id, &value.venue_id)
}
fn affected<'a>(entity: &'a w::Entity, books: &mut BTreeMap<Book<'a>, bool>) {
    match &entity.entity {
        Some(w::entity::Entity::Order(order)) => {
            if let Some(key) = &order.book {
                books.insert(book(key), true);
            }
        }
        Some(w::entity::Entity::Feed(feed)) => {
            for key in &feed.books {
                books.entry(book(key)).or_insert(false);
            }
        }
        _ => {}
    }
}
impl Revisions {
    pub fn replaced(previous: Option<&ReplicaView>, state: &State) -> Self {
        Self::replaced_checked(previous, state, &|| Ok(()))
            .expect("unchecked revision restore cannot fail")
    }
    pub(crate) fn replaced_checked(
        previous: Option<&ReplicaView>,
        state: &State,
        check: &impl Fn() -> Result<(), Status>,
    ) -> Result<Self, Status> {
        check()?;
        let sequence = previous.map_or(1, |view| view.revisions.sequence + 1);
        let mut option_groups = OrdMap::<String, OptionGroup>::new();
        for (_, entity) in state.range(None, Some(EntityKind::AttributeOption)) {
            check()?;
            if let Some(attribute) = option_group(entity) {
                if let Some(group) = option_groups.get_mut(attribute) {
                    group.count += 1;
                } else {
                    option_groups.insert(
                        attribute.to_owned(),
                        OptionGroup {
                            count: 1,
                            revision: sequence,
                        },
                    );
                }
            }
        }
        Ok(Self {
            instance: previous.map_or_else(uuid::Uuid::new_v4, |view| view.revisions.instance),
            sequence,
            dataset: sequence,
            catalog: sequence,
            history: sequence,
            directory: sequence,
            metadata: [sequence; 5],
            option_groups,
            books: OrdMap::new(),
        })
    }
    pub fn updated<'a>(
        &self,
        before: &State,
        after: &State,
        changes: impl IntoIterator<Item = &'a w::Change>,
        history_changed: bool,
    ) -> Result<Self, Status> {
        let mut next = self.clone();
        let mut data_changed = history_changed;
        next.sequence += 1;
        if history_changed {
            next.history = next.sequence;
        }
        let mut touched = BTreeMap::new();
        let mut changed_options = BTreeSet::new();
        for change in changes {
            let key = match &change.change {
                Some(w::change::Change::Upsert(value)) => state::entity_key(value)?,
                Some(w::change::Change::Remove(value)) => Key::try_from(
                    value
                        .key
                        .as_ref()
                        .ok_or_else(|| Status::data_loss("missing key"))?,
                )?,
                None => return Err(Status::data_loss("empty change")),
            };
            if key.kind == EntityKind::AttributeOption && !changed_options.insert(key.id.clone()) {
                continue;
            }
            let old = before.entity(&key);
            let new = after.entity(&key);
            // Immutable execution retries and identical observations need no active-book invalidation.
            if old == new {
                continue;
            }
            data_changed = true;
            if key.kind != EntityKind::Order && key.kind != EntityKind::Feed {
                next.catalog = next.sequence;
                next.metadata[key.kind as usize - 1] = next.sequence;
            }
            if let Some(attribute) = old.and_then(|e| option_group(e)) {
                let group = next
                    .option_groups
                    .get_mut(attribute)
                    .expect("existing option group");
                group.count -= 1;
                if group.count == 0 {
                    next.option_groups.remove(attribute);
                } else {
                    group.revision = next.sequence;
                }
            }
            if let Some(attribute) = new.and_then(|e| option_group(e)) {
                let group = next
                    .option_groups
                    .entry(attribute.to_owned())
                    .or_insert(OptionGroup {
                        count: 0,
                        revision: next.sequence,
                    });
                group.count += 1;
                group.revision = next.sequence;
            }
            if let Some(entity) = old {
                affected(entity, &mut touched);
            }
            if let Some(entity) = new {
                affected(entity, &mut touched);
            }
        }
        for ((item, currency, venue), orders_changed) in touched {
            let key = w::BookKey {
                item_type_id: item.to_owned(),
                currency_id: currency.to_owned(),
                venue_id: venue.to_owned(),
            };
            if before.books().contains(&key) != after.books().contains(&key) {
                next.directory = next.sequence;
            }
            if !orders_changed {
                continue;
            }
            let specific = (item.to_owned(), currency.to_owned(), Some(venue.to_owned()));
            if after.books().has_orders(&key) {
                next.books.insert(specific, next.sequence);
            } else {
                next.books.remove(&specific);
            }
            let group = (item.to_owned(), currency.to_owned(), None);
            if after.books().has_group(item, currency) {
                next.books.insert(group, next.sequence);
            } else {
                next.books.remove(&group);
            }
        }
        Ok(if data_changed { next } else { self.clone() })
    }
}
impl ReadRevision {
    pub fn token(self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.instance, self.dataset, self.catalog, self.data
        )
    }
}
impl ReplicaReader {
    pub fn read_revision(&self, request: &w::ReadRequest) -> Result<ReadRevision, Status> {
        let view = self.view()?;
        let revisions = &view.revisions;
        let data = match request
            .selection
            .as_ref()
            .and_then(|selection| selection.source.as_ref())
        {
            Some(w::selection::Source::Scope(scope))
                if scope.modes.contains(&(w::OrderMode::Executed as i32))
                    || request
                        .selection
                        .as_ref()
                        .is_some_and(|selection| selection.executed_at.is_some()) =>
            {
                revisions.history
            }
            Some(w::selection::Source::Scope(scope))
                if request
                    .selection
                    .as_ref()
                    .is_none_or(|selection| selection.executed_at.is_none())
                    && !scope.modes.contains(&(w::OrderMode::Executed as i32)) =>
            {
                match (&scope.item_type_id, &scope.currency_id) {
                    (Some(item), Some(currency)) => revisions
                        .books
                        .get(&(item.clone(), currency.clone(), scope.venue_id.clone()))
                        .copied()
                        .unwrap_or_else(|| {
                            let present = scope.venue_id.as_ref().map_or_else(
                                || view.state.books().has_group(item, currency),
                                |venue| {
                                    view.state.books().has_orders(&w::BookKey {
                                        item_type_id: item.clone(),
                                        currency_id: currency.clone(),
                                        venue_id: venue.clone(),
                                    })
                                },
                            );
                            if present { revisions.dataset } else { 0 }
                        }),
                    _ => revisions.sequence,
                }
            }
            _ => revisions.sequence,
        };
        Ok(ReadRevision {
            instance: revisions.instance,
            dataset: revisions.dataset,
            catalog: revisions.catalog,
            data,
        })
    }
    /// Revision of selected reference entity kinds, independent of order/feed updates.
    pub fn metadata_revision(&self, kinds: &[w::EntityKind]) -> Result<MetadataRevision, Status> {
        let view = self.view()?;
        let mut selected = [0; 5];
        for kind in kinds {
            let kind = EntityKind::try_from(*kind as i32)?;
            if matches!(kind, EntityKind::Order | EntityKind::Feed) {
                return Err(Status::invalid_argument(
                    "metadata revision requires reference entity kinds",
                ));
            }
            let index = kind as usize - 1;
            selected[index] = view.revisions.metadata[index];
        }
        Ok(MetadataRevision {
            instance: view.revisions.instance,
            dataset: view.revisions.dataset,
            kinds: selected,
        })
    }
    /// Capture exact references and unrestricted attribute-option groups over a pinned read.
    pub fn metadata_dependencies(
        &self,
        keys: Vec<w::Key>,
        option_attributes: Vec<String>,
    ) -> Result<MetadataDependencies, Status> {
        let view = self.view()?;
        let mut keys = keys
            .iter()
            .map(Key::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        if keys
            .iter()
            .any(|key| matches!(key.kind, EntityKind::Order | EntityKind::Feed))
        {
            return Err(Status::invalid_argument(
                "metadata dependencies require reference entity keys",
            ));
        }
        keys.sort();
        keys.dedup();
        let mut attributes = option_attributes;
        attributes.sort();
        attributes.dedup();
        Ok(MetadataDependencies {
            instance: view.revisions.instance,
            dataset: view.revisions.dataset,
            entities: keys
                .into_iter()
                .map(|key| {
                    let entity = view.state.entity(&key).cloned();
                    (key, entity)
                })
                .collect(),
            option_groups: attributes
                .into_iter()
                .map(|attribute| {
                    let revision = view
                        .revisions
                        .option_groups
                        .get(&attribute)
                        .map_or(0, |group| group.revision);
                    (attribute, revision)
                })
                .collect(),
        })
    }
    /// Check dependencies without scanning the catalog or retaining removed-key tombstones.
    pub fn metadata_matches(&self, dependencies: &MetadataDependencies) -> Result<bool, Status> {
        self.metadata_matches_with(dependencies, || Ok(()))
    }
    /// Consumers can cooperatively cancel a long reference comparison without owning the replica.
    pub fn metadata_matches_with(
        &self,
        dependencies: &MetadataDependencies,
        mut check: impl FnMut() -> Result<(), Status>,
    ) -> Result<bool, Status> {
        check()?;
        let view = self.view()?;
        if dependencies.instance != view.revisions.instance
            || dependencies.dataset != view.revisions.dataset
        {
            return Ok(false);
        }
        Ok(dependencies
            .option_groups
            .iter()
            .all(|(attribute, revision)| {
                view.revisions
                    .option_groups
                    .get(attribute)
                    .map_or(0, |group| group.revision)
                    == *revision
            })
            && {
                for (index, (key, expected)) in dependencies.entities.iter().enumerate() {
                    if index % 64 == 0 {
                        check()?;
                    }
                    let actual = view.state.entity(key);
                    if !match (actual, expected) {
                        (None, None) => true,
                        (Some(actual), Some(expected)) => {
                            Arc::ptr_eq(actual, expected) || actual == expected
                        }
                        _ => false,
                    } {
                        return Ok(false);
                    }
                }
                true
            })
    }

    /// Changes only when available books change or the replica dataset is replaced.
    pub fn directory_revision(&self) -> Result<DirectoryRevision, Status> {
        let view = self.view()?;
        Ok(DirectoryRevision {
            instance: view.revisions.instance,
            dataset: view.revisions.dataset,
            directory: view.revisions.directory,
        })
    }
    pub fn cursor(&self) -> Result<String, Status> {
        Ok(self.view()?.cursor.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn option(id: &str, attribute: &str, name: &str) -> w::Change {
        w::Change {
            change: Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::AttributeOption(w::AttributeOption {
                    id: id.into(),
                    attribute_id: attribute.into(),
                    name: name.into(),
                    ..Default::default()
                })),
            })),
        }
    }
    fn remove(id: &str) -> w::Change {
        w::Change {
            change: Some(w::change::Change::Remove(w::Removal {
                key: Some(w::Key {
                    kind: w::EntityKind::AttributeOption as i32,
                    id: id.into(),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }
    }
    #[test]
    fn option_group_revisions_handle_duplicate_keys_moves_and_churn_without_tombstones() {
        let mut state = State::default();
        state
            .apply(&[option("a", "one", "A"), option("b", "one", "B")])
            .unwrap();
        // Replacement/reopen reconstructs group counts from existing reference entities.
        let mut revisions = Revisions::replaced(None, &state);
        assert_eq!(revisions.option_groups.get("one").unwrap().count, 2);
        let changes = [
            option("a", "two", "Intermediate"),
            option("a", "two", "Final"),
        ];
        let mut after = state.clone();
        after.apply(&changes).unwrap();
        revisions = revisions.updated(&state, &after, &changes, false).unwrap();
        assert_eq!(revisions.option_groups.get("one").unwrap().count, 1);
        assert_eq!(revisions.option_groups.get("two").unwrap().count, 1);
        assert_eq!(
            revisions.option_groups.get("one").unwrap().revision,
            revisions.sequence
        );
        state = after;
        let changes = [remove("a"), remove("a"), remove("b")];
        let mut after = state.clone();
        after.apply(&changes).unwrap();
        revisions = revisions.updated(&state, &after, &changes, false).unwrap();
        assert!(revisions.option_groups.is_empty());
        state = after;
        for i in 0..100 {
            let changes = [option("a", &format!("group.{i}"), "A")];
            let mut after = state.clone();
            after.apply(&changes).unwrap();
            revisions = revisions.updated(&state, &after, &changes, false).unwrap();
            assert_eq!(revisions.option_groups.len(), 1);
            assert_eq!(
                revisions
                    .option_groups
                    .get(&format!("group.{i}"))
                    .unwrap()
                    .count,
                1
            );
            state = after;
        }
    }
}
