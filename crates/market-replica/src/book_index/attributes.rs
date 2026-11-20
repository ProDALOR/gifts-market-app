//! Equality membership for concrete enum facts. IDs reuse the partition's authoritative records.
use crate::{query::Scope, state::order};
use im::{OrdMap, OrdSet};
use market_hub_protocol::wire as w;
use std::collections::BTreeSet;

#[derive(Clone, Default)]
pub(super) struct AttributeIndex(OrdMap<(String, String), OrdSet<String>>);

fn facts(entity: &w::Order) -> BTreeSet<(String, String)> {
    if entity.mode != w::OrderMode::Ask as i32 {
        return BTreeSet::new();
    }
    entity
        .attributes
        .iter()
        .filter_map(|(id, predicate)| match predicate.predicate.as_ref() {
            Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                value: Some(w::attribute_value::Value::OptionId(value)),
            })) => Some((id.clone(), value.clone())),
            _ => None,
        })
        .collect()
}
impl AttributeIndex {
    pub(super) fn replace(&mut self, previous: Option<&w::Entity>, next: &w::Entity) {
        let previous = previous.and_then(order);
        let next = order(next).expect("partition records are orders");
        if previous.is_some_and(|old| old.attributes == next.attributes && old.mode == next.mode) {
            return;
        }
        let old = previous.map(facts).unwrap_or_default();
        let new = facts(next);
        for fact in old.difference(&new) {
            self.remove_fact(fact, &next.id);
        }
        for fact in new.difference(&old) {
            self.0
                .entry(fact.clone())
                .or_default()
                .insert(next.id.clone());
        }
    }
    fn remove_fact(&mut self, fact: &(String, String), id: &str) {
        if let Some(ids) = self.0.get_mut(fact) {
            ids.remove(id);
            if ids.is_empty() {
                self.0.remove(fact);
            }
        }
    }
    pub(super) fn remove(&mut self, entity: &w::Entity) {
        let order = order(entity).expect("partition records are orders");
        for fact in facts(order) {
            self.remove_fact(&fact, &order.id);
        }
    }
    /// Choose the smallest indexed trait group; values within a group form a disjoint union.
    pub(super) fn candidates(
        &self,
        scope: &Scope,
        budget: usize,
    ) -> Option<(usize, Vec<&OrdSet<String>>)> {
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
                    if let Some(ids) = self.0.get(&(id.clone(), value.clone())) {
                        count += ids.len();
                        sets.push(ids);
                    }
                }
                (count <= budget).then_some((count, sets))
            })
            .min_by_key(|(count, _)| *count)
    }
}
