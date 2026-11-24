//! Equality membership shares order identities with the covering rank indexes.
use crate::{
    query::Scope,
    state::{Key, order},
};
use im::{OrdMap, OrdSet};
use market_hub_protocol::wire as w;
use std::sync::Arc;

#[derive(Clone, Default)]
pub(super) struct AttributeIndex(OrdMap<String, OrdMap<String, OrdSet<Arc<Key>>>>);

fn option_value(predicate: &w::AttributePredicate) -> Option<&str> {
    match predicate.predicate.as_ref()? {
        w::attribute_predicate::Predicate::Eq(w::AttributeValue {
            value: Some(w::attribute_value::Value::OptionId(value)),
        }) => Some(value),
        _ => None,
    }
}
fn fact<'a>(order: &'a w::Order, id: &str) -> Option<&'a str> {
    if order.mode != w::OrderMode::Ask as i32 {
        return None;
    }
    option_value(order.attributes.get(id)?)
}
fn facts(order: &w::Order) -> impl Iterator<Item = (&str, &str)> {
    let attributes = (order.mode == w::OrderMode::Ask as i32).then_some(&order.attributes);
    attributes
        .into_iter()
        .flatten()
        .filter_map(|(id, predicate)| option_value(predicate).map(|value| (id.as_str(), value)))
}
impl AttributeIndex {
    pub(super) fn insert(&mut self, key: &Arc<Key>, entity: &w::Entity) {
        for (id, value) in facts(order(entity).expect("partition records are orders")) {
            self.insert_fact(id, value, key);
        }
    }
    pub(super) fn replace(&mut self, key: &Arc<Key>, previous: &w::Entity, next: &w::Entity) {
        let previous = order(previous).expect("partition records are orders");
        let next = order(next).expect("partition records are orders");
        if previous.attributes == next.attributes && previous.mode == next.mode {
            return;
        }
        for (id, value) in facts(previous) {
            if fact(next, id) != Some(value) {
                self.remove_fact(id, value, key);
            }
        }
        for (id, value) in facts(next) {
            if fact(previous, id) != Some(value) {
                self.insert_fact(id, value, key);
            }
        }
    }
    fn insert_fact(&mut self, id: &str, value: &str, key: &Arc<Key>) {
        if !self.0.contains_key(id) {
            self.0.insert(id.into(), OrdMap::new());
        }
        let values = self.0.get_mut(id).expect("inserted attribute");
        if !values.contains_key(value) {
            values.insert(value.into(), OrdSet::new());
        }
        values
            .get_mut(value)
            .expect("inserted value")
            .insert(Arc::clone(key));
    }
    fn remove_fact(&mut self, id: &str, value: &str, key: &Key) {
        if let Some(values) = self.0.get_mut(id) {
            if let Some(ids) = values.get_mut(value) {
                ids.remove(key);
                if ids.is_empty() {
                    values.remove(value);
                }
            }
            if values.is_empty() {
                self.0.remove(id);
            }
        }
    }
    pub(super) fn remove(&mut self, key: &Key, entity: &w::Entity) {
        for (id, value) in facts(order(entity).expect("partition records are orders")) {
            self.remove_fact(id, value, key);
        }
    }
    /// Choose the smallest indexed trait group; values within a group form a disjoint union.
    pub(super) fn candidates(
        &self,
        scope: &Scope,
        budget: usize,
    ) -> Option<(usize, Vec<&OrdSet<Arc<Key>>>)> {
        if scope.attribute_match != crate::AttributeMatch::Value {
            return None;
        }
        scope
            .attributes
            .iter()
            .filter_map(|(id, predicate)| {
                use w::attribute_predicate::Predicate as P;
                let values = match predicate.predicate.as_ref()? {
                    P::Eq(value) => std::slice::from_ref(value),
                    P::OneOf(values) => &values.values,
                    _ => return None,
                };
                let mut sets = Vec::new();
                let mut count = 0;
                for value in values {
                    let Some(w::attribute_value::Value::OptionId(value)) = &value.value else {
                        return None;
                    };
                    if let Some(ids) = self
                        .0
                        .get(id.as_str())
                        .and_then(|values| values.get(value.as_str()))
                    {
                        count += ids.len();
                        sets.push(ids);
                    }
                }
                (count <= budget).then_some((count, sets))
            })
            .min_by_key(|(count, _)| *count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{entity, order};

    #[test]
    fn trait_replacement_prunes_old_membership_without_changing_pinned_view() {
        let key = Arc::new(Key::order("venue".into(), "order".into()));
        let make = |model: Option<&str>, mode| {
            let mut value = order("order", "1", None);
            value.mode = mode;
            if let Some(model) = model {
                value.attributes.insert(
                    "model".into(),
                    w::AttributePredicate {
                        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId(model.into())),
                        })),
                    },
                );
            }
            entity(w::entity::Entity::Order(value))
        };
        let mut index = AttributeIndex::default();
        let first = make(Some("first"), 1);
        index.insert(&key, &first);
        let pinned = index.clone();
        let next = make(Some("next"), 1);
        index.replace(&key, &first, &next);
        assert!(!index.0["model"].contains_key("first"));
        assert_eq!(index.0["model"]["next"].len(), 1);
        assert!(index.0["model"]["next"].contains(key.as_ref()));
        assert!(pinned.0["model"]["first"].contains(key.as_ref()));
        let bid = make(Some("next"), 2);
        index.replace(&key, &next, &bid);
        assert!(index.0.is_empty());
        index.replace(&key, &bid, &next);
        assert!(index.0["model"]["next"].contains(key.as_ref()));
        let unrestricted = make(None, 1);
        index.replace(&key, &next, &unrestricted);
        assert!(index.0.is_empty());
        index.insert(&key, &first);
        index.remove(&key, &first);
        assert!(index.0.is_empty());
        assert_eq!(pinned.0["model"]["first"].len(), 1);
    }
}
