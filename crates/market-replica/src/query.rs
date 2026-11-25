use crate::{
    AttributeMatch, ReadOptions,
    state::{self, Key, State},
    storage,
    types::{EntityKind, IndexKind, IndexTerm, Leaf, OrderMode, Sort},
};
use market_hub_protocol::wire as w;
use prost::Message;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use tonic::Status;
mod constraints;
fn invalid(message: &str) -> Status {
    Status::invalid_argument(message)
}
#[derive(Clone)]
pub struct ExactPriceRange {
    pub min: Option<rust_decimal::Decimal>,
    pub max: Option<rust_decimal::Decimal>,
}
#[derive(Clone)]
pub struct Scope {
    pub venue_id: Option<String>,
    pub item_type_id: Option<String>,
    pub currency_id: Option<String>,
    pub modes: BTreeSet<OrderMode>,
    pub attributes: std::collections::BTreeMap<String, w::AttributePredicate>,
    pub attribute_match: AttributeMatch,
    pub collection_attributes: Option<
        std::sync::Arc<
            std::collections::BTreeMap<
                String,
                std::collections::BTreeMap<String, w::AttributePredicate>,
            >,
        >,
    >,
    pub price: Option<ExactPriceRange>,
}
impl TryFrom<w::Scope> for Scope {
    type Error = Status;
    fn try_from(scope: w::Scope) -> Result<Self, Status> {
        Ok(Self {
            venue_id: scope.venue_id,
            item_type_id: scope.item_type_id,
            currency_id: scope.currency_id,
            modes: scope
                .modes
                .into_iter()
                .map(OrderMode::try_from)
                .collect::<Result<_, _>>()?,
            attributes: scope.attributes,
            attribute_match: AttributeMatch::Value,
            collection_attributes: None,
            price: scope
                .price
                .map(|price| {
                    Ok::<_, Status>(ExactPriceRange {
                        min: price.min.as_deref().map(state::decimal).transpose()?,
                        max: price.max.as_deref().map(state::decimal).transpose()?,
                    })
                })
                .transpose()?,
        })
    }
}
#[derive(Clone)]
pub struct Filter {
    pub scope: Option<Scope>,
    pub period: Option<w::Period>,
    pub kinds: BTreeSet<EntityKind>,
    pub keys: BTreeSet<Key>,
    pub sort: Sort,
    pub fingerprint: String,
}
impl Filter {
    #[cfg(test)]
    pub fn compile(state: &State, request: &w::ReadRequest) -> Result<Self, Status> {
        Self::compile_with_options(state, request, ReadOptions::default())
    }
    pub fn compile_with_options(
        state: &State,
        request: &w::ReadRequest,
        options: ReadOptions,
    ) -> Result<Self, Status> {
        let options = options.normalized_for(request);
        let attribute_match = options.attribute_match;
        let mut selection = request.selection.clone().unwrap_or_default();
        if selection.keys.len() > 500 || selection.kinds.len() > 7 {
            return Err(invalid("too many query keys or kinds"));
        }
        // Validate before sorting so malformed keys never enter the compiled representation.
        let mut keys = selection
            .keys
            .iter()
            .map(Key::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        keys.sort();
        let keys = keys.into_iter().collect();
        selection
            .keys
            .sort_by(|a, b| (a.kind, &a.venue_id, &a.id).cmp(&(b.kind, &b.venue_id, &b.id)));
        selection.keys.dedup();
        selection.kinds.sort();
        selection.kinds.dedup();
        let kinds = selection
            .kinds
            .iter()
            .copied()
            .map(EntityKind::try_from)
            .collect::<Result<_, _>>()?;
        let scope = match &mut selection.source {
            Some(w::selection::Source::Scope(scope)) => {
                for (kind, id) in [
                    (EntityKind::Venue, &scope.venue_id),
                    (EntityKind::Currency, &scope.currency_id),
                    (EntityKind::ItemType, &scope.item_type_id),
                ] {
                    if let Some(id) = id
                        && state.get(kind, id).is_none()
                    {
                        return Err(invalid("unknown venue, currency or item type"));
                    }
                }
                scope.modes.sort();
                scope.modes.dedup();
                if scope.modes.iter().any(|m| OrderMode::try_from(*m).is_err()) {
                    return Err(invalid("unknown order mode"));
                }
                if scope.modes.len() == 3 {
                    scope.modes.clear();
                }
                if let Some(price) = &mut scope.price {
                    if scope.currency_id.is_none() {
                        return Err(invalid("price bounds require a currency"));
                    }
                    for bound in [&mut price.min, &mut price.max].into_iter().flatten() {
                        let value = state::decimal(bound)?;
                        if value < rust_decimal::Decimal::ZERO {
                            return Err(invalid("negative price bound"));
                        }
                        *bound = value.to_string();
                    }
                    if let (Some(min), Some(max)) = (&price.min, &price.max)
                        && state::decimal(min)? > state::decimal(max)?
                    {
                        return Err(invalid("reversed price bounds"));
                    }
                    if price.min.is_none() && price.max.is_none() {
                        scope.price = None;
                    }
                }
                if scope.attributes.len() > 64 {
                    return Err(invalid("too many attribute filters"));
                }
                if scope.item_type_id.is_none()
                    && scope
                        .attributes
                        .values()
                        .map(|p| match &p.predicate {
                            Some(w::attribute_predicate::Predicate::OneOf(v)) => v.values.len(),
                            _ => 0,
                        })
                        .sum::<usize>()
                        > 16_384
                {
                    return Err(invalid("expanded attribute selection exceeds query budget"));
                }
                if attribute_match != AttributeMatch::Value
                    && scope.modes != [w::OrderMode::Bid as i32]
                {
                    return Err(invalid("constraint matching requires bids"));
                }
                if !scope.attributes.is_empty() {
                    if attribute_match == AttributeMatch::Value
                        && (scope.modes.is_empty()
                            || scope.modes.contains(&(w::OrderMode::Bid as i32)))
                    {
                        return Err(invalid("attribute filters require asks or executions"));
                    }
                    let item = scope
                        .item_type_id
                        .as_ref()
                        .and_then(|id| state.get(EntityKind::ItemType, id));
                    let mut nodes = 0;
                    for (id, predicate) in &mut scope.attributes {
                        let rule = match item {
                            Some(w::entity::Entity::ItemType(item)) => {
                                Some(item.attributes.get(id).ok_or_else(|| {
                                    invalid("attribute does not apply to item type")
                                })?)
                            }
                            _ => None,
                        };
                        validate_predicate(state, id, rule, predicate, 0, &mut nodes)?;
                        if attribute_match != AttributeMatch::Value
                            && !constraints::supported(predicate)
                        {
                            return Err(invalid(
                                "constraint matching requires scalar equality, sets or ranges",
                            ));
                        }
                    }
                }
                let mut compiled = Scope::try_from(scope.clone())?;
                compiled.attribute_match = attribute_match;
                if compiled.item_type_id.is_none() {
                    let mut collections = std::collections::BTreeMap::new();
                    for (_, entity) in state.range(None, Some(EntityKind::ItemType)) {
                        let Some(w::entity::Entity::ItemType(item)) = &entity.entity else {
                            continue;
                        };
                        let mut attributes = std::collections::BTreeMap::new();
                        let mut valid = true;
                        for (id, predicate) in &compiled.attributes {
                            let Some(rule) = item.attributes.get(id) else {
                                valid = false;
                                break;
                            };
                            let mut predicate = predicate.clone();
                            if let Some(w::attribute_predicate::Predicate::OneOf(values)) =
                                &mut predicate.predicate
                            {
                                values.values.retain(|v| match &v.value {
                                    Some(w::attribute_value::Value::OptionId(v)) => rule
                                        .allowed_values
                                        .as_ref()
                                        .is_none_or(|allowed| allowed.ids.contains(v)),
                                    _ => true,
                                });
                                if values.values.is_empty() {
                                    valid = false;
                                    break;
                                }
                            }
                            let mut nodes = 0;
                            if validate_predicate(
                                state,
                                id,
                                Some(rule),
                                &mut predicate,
                                0,
                                &mut nodes,
                            )
                            .is_err()
                            {
                                valid = false;
                                break;
                            }
                            attributes.insert(id.clone(), predicate);
                        }
                        if valid {
                            collections.insert(item.id.clone(), attributes);
                        }
                    }
                    compiled.collection_attributes = Some(std::sync::Arc::new(collections));
                }
                Some(compiled)
            }
            Some(w::selection::Source::FeedId(_)) => {
                return Err(invalid("public queries use an explicit scope"));
            }
            None => None,
        };
        if let Some(period) = &selection.executed_at
            && matches!((period.from_ms,period.until_ms),(Some(a),Some(b))if a>=b)
        {
            return Err(invalid("invalid execution period"));
        }
        let wire_sort = Sort::from_wire(request.sort)?;
        let sort = options.sort.unwrap_or(wire_sort);
        if sort != Sort::Identity
            && (!scope.as_ref().is_some_and(|scope| {
                scope.currency_id.is_some()
                    && !scope.modes.is_empty()
                    && (!scope.modes.contains(&OrderMode::Executed) || scope.modes.len() == 1)
            }))
        {
            return Err(invalid("sorting requires orders in one currency"));
        }
        let mut digest = Sha256::new();
        digest.update(selection.encode_to_vec());
        if attribute_match != AttributeMatch::Value {
            digest.update([attribute_match as u8]);
        }
        let fingerprint = format!("{:x}", digest.finalize());
        Ok(Self {
            scope,
            period: selection.executed_at,
            kinds,
            keys,
            sort,
            fingerprint,
        })
    }
    pub fn history(&self) -> bool {
        self.period.is_some()
            || self
                .scope
                .as_ref()
                .is_some_and(|s| s.modes == BTreeSet::from([OrderMode::Executed]))
    }
    pub fn matches(&self, entity: &w::Entity) -> Result<bool, Status> {
        let key = state::entity_key(entity)?;
        if (!self.kinds.is_empty() && !self.kinds.contains(&key.kind))
            || (!self.keys.is_empty() && !self.keys.contains(&key))
        {
            return Ok(false);
        }
        if let Some(order) = state::order(entity) {
            self.matches_order(order)
        } else {
            Ok(self.scope.is_none() && self.period.is_none())
        }
    }
    pub fn matches_order(&self, order: &w::Order) -> Result<bool, Status> {
        if !self.kinds.is_empty() && !self.kinds.contains(&EntityKind::Order) {
            return Ok(false);
        }
        let book = order
            .book
            .as_ref()
            .ok_or_else(|| Status::data_loss("missing order book"))?;
        if !self.keys.is_empty()
            && !self
                .keys
                .contains(&Key::order(book.venue_id.clone(), order.id.clone()))
        {
            return Ok(false);
        }
        if let Some(period) = &self.period
            && (!order.executed_at_ms.is_some_and(|t| {
                period.from_ms.is_none_or(|v| t >= v) && period.until_ms.is_none_or(|v| t < v)
            }) || OrderMode::try_from(order.mode)? != OrderMode::Executed)
        {
            return Ok(false);
        }
        let Some(scope) = &self.scope else {
            return Ok(true);
        };
        if scope.venue_id.as_ref().is_some_and(|v| v != &book.venue_id)
            || scope
                .item_type_id
                .as_ref()
                .is_some_and(|v| v != &book.item_type_id)
            || scope
                .currency_id
                .as_ref()
                .is_some_and(|v| v != &book.currency_id)
            || (!scope.modes.is_empty() && !scope.modes.contains(&OrderMode::try_from(order.mode)?))
        {
            return Ok(false);
        }
        if let Some(range) = &scope.price {
            let price = state::decimal(&order.price)?;
            if range.min.is_some_and(|min| price < min) || range.max.is_some_and(|max| price > max)
            {
                return Ok(false);
            }
        }
        let attributes = if let Some(collections) = &scope.collection_attributes {
            let Some(attributes) = collections.get(&book.item_type_id) else {
                return Ok(false);
            };
            attributes
        } else {
            &scope.attributes
        };
        if scope.attribute_match != AttributeMatch::Value {
            if scope.attribute_match == AttributeMatch::Exact
                && attributes.len() != order.attributes.len()
            {
                return Ok(false);
            }
            // Unknown complex restrictions must never be treated as unrestricted bids.
            return Ok(order.attributes.values().all(constraints::supported)
                && attributes.iter().all(|(id, requested)| {
                    order
                        .attributes
                        .get(id)
                        .map_or(scope.attribute_match != AttributeMatch::Exact, |accepted| {
                            constraints::matches(requested, accepted, scope.attribute_match)
                        })
                }));
        }
        for (id, predicate) in attributes {
            let Some(w::attribute_predicate::Predicate::Eq(value)) =
                order.attributes.get(id).and_then(|v| v.predicate.as_ref())
            else {
                return Ok(false);
            };
            if !matches(predicate, value) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn index(&self) -> Result<Option<IndexTerm>, Status> {
        if let Some(scope) = &self.scope {
            for (id, predicate) in scope
                .attributes
                .iter()
                .filter(|_| scope.attribute_match == AttributeMatch::Value)
            {
                if let Some(Leaf { path, value }) = necessary(&mut vec![id.clone()], predicate)? {
                    return Ok(Some(IndexTerm {
                        kind: IndexKind::Attribute,
                        path,
                        value,
                    }));
                }
            }
            if let (Some(item), Some(currency)) = (&scope.item_type_id, &scope.currency_id) {
                return Ok(Some(IndexTerm {
                    kind: IndexKind::Book,
                    path: item.as_bytes().to_vec(),
                    value: currency.as_bytes().to_vec(),
                }));
            }
            if let Some(item) = &scope.item_type_id {
                return Ok(Some(IndexTerm {
                    kind: IndexKind::ItemType,
                    path: vec![],
                    value: item.as_bytes().to_vec(),
                }));
            }
            if let Some(venue) = &scope.venue_id {
                return Ok(Some(IndexTerm {
                    kind: IndexKind::Venue,
                    path: vec![],
                    value: venue.as_bytes().to_vec(),
                }));
            }
        }
        Ok(None)
    }
}
fn necessary(
    path: &mut Vec<String>,
    predicate: &w::AttributePredicate,
) -> Result<Option<Leaf>, Status> {
    use w::attribute_predicate::Predicate as P;
    match &predicate.predicate {
        Some(P::Eq(value)) => {
            let mut facts = BTreeSet::new();
            storage::leaves(path, value, &mut facts)?;
            Ok(facts.into_iter().next())
        }
        Some(P::OneOf(values)) if values.values.len() == 1 => {
            let mut facts = BTreeSet::new();
            storage::leaves(path, &values.values[0], &mut facts)?;
            Ok(facts.into_iter().next())
        }
        Some(P::Object(object)) => {
            for (id, p) in &object.fields {
                path.push(id.clone());
                let found = necessary(path, p)?;
                path.pop();
                if found.is_some() {
                    return Ok(found);
                }
            }
            Ok(None)
        }
        Some(P::Contains(p)) if p.min_count.unwrap_or(1) > 0 => match &p.predicate {
            Some(p) => necessary(path, p),
            None => Ok(None),
        },
        Some(P::AllOf(values)) => {
            for p in &values.predicates {
                if let Some(found) = necessary(path, p)? {
                    return Ok(Some(found));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}
fn compare(a: &w::AttributeValue, b: &w::AttributeValue) -> Option<std::cmp::Ordering> {
    use w::attribute_value::Value as V;
    match (a.value.as_ref()?, b.value.as_ref()?) {
        (V::Integer(a), V::Integer(b)) => Some(a.cmp(b)),
        (V::TimestampMs(a), V::TimestampMs(b)) => Some(a.cmp(b)),
        (V::Decimal(a), V::Decimal(b)) => {
            Some(state::decimal(a).ok()?.cmp(&state::decimal(b).ok()?))
        }
        _ => None,
    }
}
fn equal(a: &w::AttributeValue, b: &w::AttributeValue) -> bool {
    use w::attribute_value::Value as V;
    match (&a.value, &b.value) {
        (Some(V::Decimal(_)), Some(V::Decimal(_))) => compare(a, b).is_some_and(|v| v.is_eq()),
        (Some(V::List(a)), Some(V::List(b))) => {
            a.values.len() == b.values.len()
                && a.values.iter().zip(&b.values).all(|(a, b)| equal(a, b))
        }
        (Some(V::Object(a)), Some(V::Object(b))) => {
            a.fields.len() == b.fields.len()
                && a.fields
                    .iter()
                    .all(|(k, v)| b.fields.get(k).is_some_and(|other| equal(v, other)))
        }
        _ => a == b,
    }
}
pub fn matches(predicate: &w::AttributePredicate, value: &w::AttributeValue) -> bool {
    use w::{attribute_predicate::Predicate as P, attribute_value::Value as V};
    match &predicate.predicate {
        Some(P::Eq(other)) => equal(value, other),
        Some(P::OneOf(values)) => values.values.iter().any(|v| equal(value, v)),
        Some(P::Range(range)) => {
            range
                .min
                .as_ref()
                .is_none_or(|min| compare(value, min).is_some_and(|v| !v.is_lt()))
                && range
                    .max
                    .as_ref()
                    .is_none_or(|max| compare(value, max).is_some_and(|v| !v.is_gt()))
        }
        Some(P::Object(fields)) => {
            matches!(&value.value,Some(V::Object(object))if fields.fields.iter().all(|(id,p)|object.fields.get(id).is_some_and(|v|matches(p,v))))
        }
        Some(P::Contains(p)) => {
            if let (Some(V::List(values)), Some(predicate)) = (&value.value, &p.predicate) {
                let count = values
                    .values
                    .iter()
                    .filter(|v| matches(predicate, v))
                    .count() as u32;
                count >= p.min_count.unwrap_or(1) && p.max_count.is_none_or(|max| count <= max)
            } else {
                false
            }
        }
        Some(P::AllOf(values)) => values.predicates.iter().all(|p| matches(p, value)),
        None => false,
    }
}
fn schema<'a>(state: &'a State, id: &str) -> Result<&'a w::Attribute, Status> {
    match state.get(EntityKind::Attribute, id) {
        Some(w::entity::Entity::Attribute(v)) => Ok(v),
        _ => Err(invalid("unknown attribute schema")),
    }
}
fn tree(depth: usize, nodes: &mut usize) -> Result<(), Status> {
    *nodes += 1;
    if depth > 8 || *nodes > 1024 {
        Err(invalid("attribute filter too complex"))
    } else {
        Ok(())
    }
}
fn validate_value(
    state: &State,
    id: &str,
    rule: Option<&w::AttributeRule>,
    value: &mut w::AttributeValue,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), Status> {
    use w::attribute_value::Value as V;
    tree(depth, nodes)?;
    let attr = schema(state, id)?;
    let valid = match value
        .value
        .as_mut()
        .ok_or_else(|| invalid("missing attribute value"))?
    {
        V::OptionId(option) => {
            if let Some(w::entity::Entity::AttributeOption(v)) =
                state.get(EntityKind::AttributeOption, option)
            {
                attr.value_type == 1
                    && v.attribute_id == id
                    && rule
                        .and_then(|r| r.allowed_values.as_ref())
                        .is_none_or(|r| r.ids.contains(option))
            } else {
                false
            }
        }
        V::Integer(_) => attr.value_type == 2,
        V::Decimal(v) => {
            *v = state::decimal(v)?.to_string();
            attr.value_type == 3
        }
        V::Boolean(_) => attr.value_type == 4,
        V::Text(v) => {
            attr.value_type == 5 && v.len() <= attr.max_length.unwrap_or(2048).min(2048) as usize
        }
        V::TimestampMs(v) => attr.value_type == 6 && jiff::Timestamp::from_millisecond(*v).is_ok(),
        V::List(list) => {
            if attr.value_type != 7
                || list.values.len() > attr.max_length.unwrap_or(128).min(128) as usize
            {
                return Err(invalid("invalid list value"));
            }
            let element = attr
                .element
                .as_deref()
                .ok_or_else(|| invalid("missing list element schema"))?;
            for value in &mut list.values {
                validate_value(state, element, None, value, depth + 1, nodes)?;
            }
            true
        }
        V::Object(object) => {
            if attr.value_type != 8 || object.fields.len() > 64 {
                return Err(invalid("invalid object value"));
            }
            if attr
                .fields
                .iter()
                .any(|(id, rule)| rule.required && !object.fields.contains_key(id))
            {
                return Err(invalid("required object field missing"));
            }
            for (key, value) in &mut object.fields {
                let rule = attr
                    .fields
                    .get(key)
                    .ok_or_else(|| invalid("unknown object field"))?;
                validate_value(state, key, Some(rule), value, depth + 1, nodes)?;
            }
            true
        }
    };
    if !valid {
        return Err(invalid("attribute value does not match its schema"));
    }
    if attr
        .minimum
        .as_ref()
        .is_some_and(|min| compare(value, min).is_some_and(|v| v.is_lt()))
        || attr
            .maximum
            .as_ref()
            .is_some_and(|max| compare(value, max).is_some_and(|v| v.is_gt()))
    {
        return Err(invalid("attribute value outside catalog bounds"));
    }
    Ok(())
}
fn validate_predicate(
    state: &State,
    id: &str,
    rule: Option<&w::AttributeRule>,
    predicate: &mut w::AttributePredicate,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), Status> {
    use w::attribute_predicate::Predicate as P;
    tree(depth, nodes)?;
    let attr = schema(state, id)?;
    match predicate
        .predicate
        .as_mut()
        .ok_or_else(|| invalid("missing predicate"))?
    {
        P::Eq(value) => validate_value(state, id, rule, value, 0, nodes)?,
        P::OneOf(values) => {
            if values.values.is_empty()
                || values.values.len() > if rule.is_none() { 4096 } else { 500 }
                || !(1..=3).contains(&attr.value_type)
            {
                return Err(invalid("one_of requires enum or numeric values"));
            }
            for v in &mut values.values {
                // Global scalar alternatives expand across collections; their total has
                // its own bounded budget, while nested values retain the ordinary depth limit.
                if rule.is_none() {
                    validate_value(state, id, rule, v, 0, &mut 0)?;
                } else {
                    validate_value(state, id, rule, v, 0, nodes)?;
                }
            }
            values.values.sort_by_key(Message::encode_to_vec);
            values.values.dedup();
        }
        P::Range(range) => {
            if ![2, 3, 6].contains(&attr.value_type) || range.min.is_none() && range.max.is_none() {
                return Err(invalid("range requires numeric or timestamp endpoints"));
            }
            for v in [&mut range.min, &mut range.max].into_iter().flatten() {
                validate_value(state, id, rule, v, 0, nodes)?;
            }
            if let (Some(a), Some(b)) = (&range.min, &range.max)
                && compare(a, b).is_none_or(|v| v.is_gt())
            {
                return Err(invalid("invalid attribute range"));
            }
        }
        P::Object(object) => {
            if attr.value_type != 8 || object.fields.len() > 64 {
                return Err(invalid("invalid object filter"));
            }
            for (key, p) in &mut object.fields {
                let rule = attr
                    .fields
                    .get(key)
                    .ok_or_else(|| invalid("unknown object field"))?;
                validate_predicate(state, key, Some(rule), p, depth + 1, nodes)?;
            }
        }
        P::Contains(p) => {
            if attr.value_type != 7
                || p.min_count.unwrap_or(1) > attr.max_length.unwrap_or(128).min(128)
                || p.max_count.is_some_and(|max| {
                    max < p.min_count.unwrap_or(1) || max > attr.max_length.unwrap_or(128).min(128)
                })
            {
                return Err(invalid("invalid list count"));
            }
            validate_predicate(
                state,
                attr.element
                    .as_deref()
                    .ok_or_else(|| invalid("missing list element"))?,
                None,
                p.predicate
                    .as_mut()
                    .ok_or_else(|| invalid("missing list predicate"))?,
                depth + 1,
                nodes,
            )?;
        }
        P::AllOf(values) => {
            if values.predicates.is_empty() || values.predicates.len() > 128 {
                return Err(invalid("invalid all_of filter"));
            }
            for p in &mut values.predicates {
                validate_predicate(state, id, rule, p, depth + 1, nodes)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod multi_scope_tests {
    use super::*;
    #[test]
    fn expanded_alias_values_are_validated_globally_and_clipped_per_collection() {
        let mut state = State::default();
        let mut insert = |value| {
            state
                .upsert(w::Entity {
                    entity: Some(value),
                })
                .unwrap()
        };
        insert(w::entity::Entity::Currency(w::Currency {
            id: "GRAM".into(),
            ..Default::default()
        }));
        insert(w::entity::Entity::Attribute(w::Attribute {
            id: "color".into(),
            value_type: 1,
            ..Default::default()
        }));
        let mut values = Vec::new();
        for collection in 0..3 {
            let mut ids = Vec::new();
            for index in 0..400 {
                let id = format!("color-{collection}-{index}");
                ids.push(id.clone());
                insert(w::entity::Entity::AttributeOption(w::AttributeOption {
                    id: id.clone(),
                    attribute_id: "color".into(),
                    ..Default::default()
                }));
                values.push(w::AttributeValue {
                    value: Some(w::attribute_value::Value::OptionId(id)),
                });
            }
            insert(w::entity::Entity::ItemType(w::ItemType {
                id: format!("collection-{collection}"),
                attributes: [(
                    "color".into(),
                    w::AttributeRule {
                        allowed_values: Some(w::AllowedAttributeValues { ids }),
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            }));
        }
        let request = w::ReadRequest {
            selection: Some(w::Selection {
                source: Some(w::selection::Source::Scope(w::Scope {
                    currency_id: Some("GRAM".into()),
                    modes: vec![w::OrderMode::Ask as i32],
                    attributes: [(
                        "color".into(),
                        w::AttributePredicate {
                            predicate: Some(w::attribute_predicate::Predicate::OneOf(
                                w::AttributeValues { values },
                            )),
                        },
                    )]
                    .into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        };
        let filter = Filter::compile(&state, &request).unwrap();
        let scopes = filter.scope.unwrap().collection_attributes.unwrap();
        assert_eq!(scopes.len(), 3);
        for predicates in scopes.values() {
            let Some(w::attribute_predicate::Predicate::OneOf(values)) =
                &predicates["color"].predicate
            else {
                panic!()
            };
            assert_eq!(values.values.len(), 400);
        }
    }
}
