use crate::{
    state::{self, Key, State},
    storage,
};
use hub_protocol::wire as w;
use prost::Message;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use tonic::Status;
fn invalid(message: &str) -> Status {
    Status::invalid_argument(message)
}
type Index = storage::IndexTerm;
#[derive(Clone)]
pub struct Filter {
    pub scope: Option<w::Scope>,
    pub period: Option<w::Period>,
    pub kinds: BTreeSet<i32>,
    pub keys: BTreeSet<Key>,
    pub sort: i32,
    pub fingerprint: String,
}
impl Filter {
    pub fn compile(state: &State, request: &w::ReadRequest) -> Result<Self, Status> {
        let mut selection = request.selection.clone().unwrap_or_default();
        if selection.keys.len() > 500 || selection.kinds.len() > 7 {
            return Err(invalid("too many query keys or kinds"));
        }
        selection.keys.sort_by_key(Key::wire);
        selection.keys.dedup();
        selection.kinds.sort();
        selection.kinds.dedup();
        for kind in &selection.kinds {
            if !(1..=7).contains(kind) {
                return Err(invalid("unknown entity kind"));
            }
        }
        let kinds = selection.kinds.iter().copied().collect();
        let keys = selection.keys.iter().map(Key::wire).collect();
        let scope = match &mut selection.source {
            Some(w::selection::Source::Scope(scope)) => {
                for (kind, id) in [
                    (1, &scope.venue_id),
                    (2, &scope.currency_id),
                    (3, &scope.item_type_id),
                ] {
                    if let Some(id) = id
                        && state.get(kind, id).is_none()
                    {
                        return Err(invalid("unknown venue, currency or item type"));
                    }
                }
                scope.modes.sort();
                scope.modes.dedup();
                if scope.modes.iter().any(|m| !(1..=3).contains(m)) {
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
                if !scope.attributes.is_empty() {
                    if scope.modes.is_empty() || scope.modes.contains(&2) {
                        return Err(invalid("attribute filters require asks or executions"));
                    }
                    let item = scope.item_type_id.as_ref().and_then(|id| state.get(3, id));
                    let Some(w::entity::Entity::ItemType(item)) = item else {
                        return Err(invalid("attribute filters require an item type"));
                    };
                    let mut nodes = 0;
                    for (id, predicate) in &mut scope.attributes {
                        let rule = item
                            .attributes
                            .get(id)
                            .ok_or_else(|| invalid("attribute does not apply to item type"))?;
                        validate_predicate(state, id, Some(rule), predicate, 0, &mut nodes)?;
                    }
                }
                Some(scope.clone())
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
        if !(0..=2).contains(&request.sort) {
            return Err(invalid("unknown read sort"));
        }
        if request.sort != 0
            && (!scope.as_ref().is_some_and(|s| {
                s.item_type_id.is_some()
                    && s.currency_id.is_some()
                    && !s.modes.is_empty()
                    && !s.modes.contains(&3)
            }) || selection.executed_at.is_some())
        {
            return Err(invalid(
                "price sorting requires active orders in one item type and currency",
            ));
        }
        let fingerprint = format!("{:x}", Sha256::digest(selection.encode_to_vec()));
        Ok(Self {
            scope,
            period: selection.executed_at,
            kinds,
            keys,
            sort: request.sort,
            fingerprint,
        })
    }
    pub fn history(&self) -> bool {
        self.period.is_some() || self.scope.as_ref().is_some_and(|s| s.modes == [3])
    }
    pub fn matches(&self, entity: &w::Entity) -> Result<bool, Status> {
        let key = state::entity_key(entity)?;
        if (!self.kinds.is_empty() && !self.kinds.contains(&key.0))
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
        if !self.kinds.is_empty() && !self.kinds.contains(&7) {
            return Ok(false);
        }
        let book = order
            .book
            .as_ref()
            .ok_or_else(|| Status::data_loss("missing order book"))?;
        if !self.keys.is_empty()
            && !self
                .keys
                .contains(&Key(7, book.venue_id.clone(), order.id.clone()))
        {
            return Ok(false);
        }
        if let Some(period) = &self.period
            && (!order.executed_at_ms.is_some_and(|t| {
                period.from_ms.is_none_or(|v| t >= v) && period.until_ms.is_none_or(|v| t < v)
            }) || order.mode != 3)
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
            || (!scope.modes.is_empty() && !scope.modes.contains(&order.mode))
        {
            return Ok(false);
        }
        if let Some(range) = &scope.price {
            let price = state::decimal(&order.price)?;
            if range
                .min
                .as_ref()
                .is_some_and(|v| state::decimal(v).is_ok_and(|v| price < v))
                || range
                    .max
                    .as_ref()
                    .is_some_and(|v| state::decimal(v).is_ok_and(|v| price > v))
            {
                return Ok(false);
            }
        }
        for (id, predicate) in &scope.attributes {
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
    pub fn index(&self) -> Result<Option<Index>, Status> {
        if let Some(scope) = &self.scope {
            for (id, predicate) in &scope.attributes {
                if let Some((path, value)) = necessary(&mut vec![id.clone()], predicate)? {
                    return Ok(Some((3, path, value)));
                }
            }
            if let (Some(item), Some(currency)) = (&scope.item_type_id, &scope.currency_id) {
                return Ok(Some((
                    2,
                    item.as_bytes().to_vec(),
                    currency.as_bytes().to_vec(),
                )));
            }
            if let Some(item) = &scope.item_type_id {
                return Ok(Some((1, vec![], item.as_bytes().to_vec())));
            }
            if let Some(venue) = &scope.venue_id {
                return Ok(Some((0, vec![], venue.as_bytes().to_vec())));
            }
        }
        Ok(None)
    }
}
fn necessary(
    path: &mut Vec<String>,
    predicate: &w::AttributePredicate,
) -> Result<Option<storage::Leaf>, Status> {
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
    match state.get(4, id) {
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
            if let Some(w::entity::Entity::AttributeOption(v)) = state.get(5, option) {
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
                || values.values.len() > 128
                || !(1..=3).contains(&attr.value_type)
            {
                return Err(invalid("one_of requires enum or numeric values"));
            }
            for v in &mut values.values {
                validate_value(state, id, rule, v, 0, nodes)?;
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
