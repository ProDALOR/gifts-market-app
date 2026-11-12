use super::{RawAttributes, Result, Scalar, decode_attributes, strict::StrictValue};
use market_view as m;
use serde::Serialize;
use serde_json::{Map, Value, json};
fn invalid(message: &'static str) -> super::Error {
    m::ValidationError(message).into()
}
fn object(value: Value) -> Result<Map<String, Value>> {
    match value {
        Value::Object(value) => Ok(value),
        _ => Err(invalid("expected a JSON object")),
    }
}
fn take(map: &mut Map<String, Value>, key: &str) -> Result<Value> {
    map.remove(key)
        .ok_or_else(|| invalid("required JSON field is missing"))
}
fn text(value: Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value),
        _ => Err(invalid("expected a JSON string")),
    }
}
fn encoded_value(value: &m::AttributeValue) -> Result<Value> {
    Ok(serde_json::to_value(Scalar::from(value))?)
}
fn attrs(values: &m::Attributes) -> Result<Value> {
    Ok(serde_json::to_value(super::attributes(values))?)
}
fn predicate(value: &m::AttributePredicate) -> Result<Value> {
    Ok(match value {
        m::AttributePredicate::Eq { value } => json!({"op":"eq","value":encoded_value(value)?}),
        m::AttributePredicate::OneOf { values } => {
            json!({"op":"one_of","values":values.iter().map(encoded_value).collect::<Result<Vec<_>>>()?})
        }
        m::AttributePredicate::Range { min, max } => {
            let mut map = Map::new();
            map.insert("op".into(), "range".into());
            if let Some(v) = min {
                map.insert("min".into(), encoded_value(v)?);
            }
            if let Some(v) = max {
                map.insert("max".into(), encoded_value(v)?);
            }
            Value::Object(map)
        }
    })
}
fn filter(value: &m::AttributeFilter) -> Result<Value> {
    Ok(Value::Object(
        value
            .iter()
            .map(|(id, p)| Ok((id.to_string(), predicate(p)?)))
            .collect::<Result<_>>()?,
    ))
}
fn observed<T: Serialize>(value: &m::Observed<T>) -> Result<Map<String, Value>> {
    let mut body = object(serde_json::to_value(&value.data)?)?;
    if let Some(amounts) = body.remove("amounts") {
        body.extend(object(amounts)?);
    }
    body.insert("feed_id".into(), value.feed_id.to_string().into());
    body.insert("item_type_id".into(), value.item_type_id.to_string().into());
    body.insert(
        "observed_at".into(),
        serde_json::to_value(value.observed_at)?,
    );
    Ok(body)
}
fn tag(mut value: Map<String, Value>, kind: &str) -> Value {
    value.insert("type".into(), kind.into());
    value.into()
}
pub fn order_value(order: &m::Order) -> Result<Value> {
    let mut body = observed(order)?;
    match &order.kind {
        m::OrderKind::AskUnits { attributes } => {
            body.get_mut("kind").expect("order kind")["attributes"] = attrs(attributes)?
        }
        m::OrderKind::BidAttributes { filter: v } => {
            body.get_mut("kind").expect("order kind")["filter"] = filter(v)?
        }
        _ => {}
    }
    Ok(body.into())
}
pub fn item_value(item: &m::Item) -> Result<Value> {
    let mut value = observed(item)?;
    value.insert("attributes".into(), attrs(&item.attributes)?);
    Ok(value.into())
}
pub fn record_value(record: &m::Record) -> Result<Value> {
    Ok(match record {
        m::Record::Item(item) => tag(object(item_value(item)?)?, "item"),
        m::Record::Order(order) => tag(object(order_value(order)?)?, "order"),
        m::Record::Feed(feed) => tag(object(serde_json::to_value(feed)?)?, "feed"),
    })
}
fn trade_value(trade: &m::Trade) -> Result<Value> {
    let mut value = observed(trade)?;
    if let m::TradeKind::Units { attributes } = &trade.kind {
        value.get_mut("kind").expect("trade kind")["attributes"] = attrs(attributes)?;
    }
    Ok(tag(value, "trade"))
}
pub fn source_value(event: &m::SourceEvent) -> Result<Value> {
    Ok(match event {
        m::SourceEvent::Trade(trade) => trade_value(trade)?,
        m::SourceEvent::OrderStatus(status) => tag(observed(status)?, "order_status"),
    })
}
pub fn event_value(event: &m::MarketEvent) -> Result<Value> {
    Ok(match event {
        m::MarketEvent::Upsert(v) => json!({"type":"upsert","record":record_value(v)?}),
        m::MarketEvent::Delete(v) => json!({"type":"delete","key":v}),
        m::MarketEvent::OrderRemoved(v) => {
            json!({"type":"order_removed","feed_id":v.feed_id,"item_type_id":v.item_type_id,"observed_at":v.observed_at,"order":v.data})
        }
        m::MarketEvent::Trade(v) => trade_value(v)?,
        m::MarketEvent::OrderStatus(v) => tag(observed(v)?, "order_status"),
    })
}
pub fn catalog_value(entry: &m::CatalogEntry) -> Result<Value> {
    let mut envelope = object(serde_json::to_value(entry)?)?;
    let kind = text(take(&mut envelope, "type")?)?;
    let mut body = object(take(&mut envelope, "data")?)?;
    if let m::CatalogEntry::Attribute(definition) = entry {
        for (key, value) in [
            ("minimum", &definition.minimum),
            ("maximum", &definition.maximum),
        ] {
            if let Some(value) = value {
                body.insert(key.into(), encoded_value(value)?);
            }
        }
    }
    Ok(tag(body, &kind))
}
fn decode_value(
    catalog: &m::Catalog,
    kind: &m::ItemTypeId,
    id: &m::AttributeId,
    value: Value,
) -> Result<m::AttributeValue> {
    // Reuse the same schema conversion and validation as standalone candidate attributes.
    let raw = serde_json::from_value::<Scalar>(value)?;
    let mut values = decode_attributes(catalog, kind, [(id.clone(), raw)].into())?;
    Ok(values.remove(id).expect("decoded attribute"))
}
fn decode_filter(
    catalog: &m::Catalog,
    kind: &m::ItemTypeId,
    value: Value,
) -> Result<m::AttributeFilter> {
    let raw = object(value)?;
    if raw.is_empty() || raw.len() > m::MAX_ATTRIBUTES {
        return Err(invalid("filter requires 1 to 64 attributes"));
    }
    let mut filter = m::AttributeFilter::new();
    for (id, value) in raw {
        let id: m::AttributeId = id.parse()?;
        let mut value = object(value)?;
        let op = text(take(&mut value, "op")?)?;
        let predicate = match op.as_str() {
            "eq" => m::AttributePredicate::Eq {
                value: decode_value(catalog, kind, &id, take(&mut value, "value")?)?,
            },
            "one_of" => {
                let Value::Array(raw) = take(&mut value, "values")? else {
                    return Err(invalid("one_of requires an array"));
                };
                if raw.is_empty() || raw.len() > m::MAX_ATTRIBUTE_FILTER_VALUES {
                    return Err(invalid("one_of requires 1 to 128 values"));
                }
                let mut values = std::collections::BTreeSet::new();
                for value in raw {
                    if !values.insert(decode_value(catalog, kind, &id, value)?) {
                        return Err(invalid("duplicate one_of value"));
                    }
                }
                m::AttributePredicate::OneOf { values }
            }
            "range" => m::AttributePredicate::Range {
                min: value
                    .remove("min")
                    .map(|v| decode_value(catalog, kind, &id, v))
                    .transpose()?,
                max: value
                    .remove("max")
                    .map(|v| decode_value(catalog, kind, &id, v))
                    .transpose()?,
            },
            _ => return Err(invalid("unsupported predicate")),
        };
        if !value.is_empty() {
            return Err(invalid("unknown predicate field"));
        }
        filter.insert(id, predicate);
    }
    catalog.validate_attribute_filter(kind, &filter)?;
    Ok(filter)
}
fn normalize_attributes(
    catalog: &m::Catalog,
    kind: &m::ItemTypeId,
    map: &mut Map<String, Value>,
    field: &str,
) -> Result<()> {
    let raw = serde_json::from_value::<RawAttributes>(take(map, field)?)?;
    map.insert(
        field.into(),
        serde_json::to_value(decode_attributes(catalog, kind, raw)?)?,
    );
    Ok(())
}
fn normalize_kind(
    catalog: &m::Catalog,
    kind: &m::ItemTypeId,
    map: &mut Map<String, Value>,
) -> Result<()> {
    let mut selector = object(take(map, "kind")?)?;
    match selector.get("type").and_then(Value::as_str) {
        Some("ask_units" | "units") => {
            normalize_attributes(catalog, kind, &mut selector, "attributes")?
        }
        Some("bid_attributes") => {
            let value = decode_filter(catalog, kind, take(&mut selector, "filter")?)?;
            selector.insert("filter".into(), serde_json::to_value(value)?);
        }
        _ => {}
    }
    map.insert("kind".into(), selector.into());
    Ok(())
}
fn normalize_amounts(map: &mut Map<String, Value>) -> Result<()> {
    if map.contains_key("amounts") {
        return Err(invalid("nested amounts are not accepted in public JSON"));
    }
    let mut price = object(take(map, "price")?)?;
    let currency = serde_json::from_value(take(&mut price, "currency_id")?)?;
    let amount = text(take(&mut price, "amount")?)?;
    if !price.is_empty() {
        return Err(invalid("unknown price field"));
    }
    let price = m::Money::parse(currency, &amount)?;
    let quantity = take(map, "quantity")?;
    map.insert("amounts".into(), json!({"price":price,"quantity":quantity}));
    Ok(())
}
fn normalize_observed(
    mut map: Map<String, Value>,
    catalog: &m::Catalog,
    category: &str,
) -> Result<Value> {
    let feed = take(&mut map, "feed_id")?;
    let kind: m::ItemTypeId = serde_json::from_value(take(&mut map, "item_type_id")?)?;
    let time = take(&mut map, "observed_at")?;
    match category {
        "item" => normalize_attributes(catalog, &kind, &mut map, "attributes")?,
        "order" | "trade" => {
            normalize_amounts(&mut map)?;
            normalize_kind(catalog, &kind, &mut map)?;
        }
        "order_status" => {}
        _ => return Err(invalid("unsupported market record")),
    }
    Ok(json!({"feed_id":feed,"item_type_id":kind,"observed_at":time,"data":map}))
}
pub fn parse_records(bytes: &[u8], catalog: &m::Catalog) -> Result<Vec<m::Record>> {
    serde_json::from_slice::<Vec<StrictValue>>(bytes)?
        .into_iter()
        .map(|StrictValue(value)| {
            let mut map = object(value)?;
            let category = text(take(&mut map, "type")?)?;
            let body = if category == "feed" {
                Value::Object(map)
            } else {
                normalize_observed(map, catalog, &category)?
            };
            let record: m::Record = serde_json::from_value(json!({"type":category,"data":body}))?;
            catalog.validate_mutation(&m::Mutation::Upsert(record.clone()))?;
            Ok(record)
        })
        .collect()
}
pub fn parse_sources(bytes: &[u8], catalog: &m::Catalog) -> Result<Vec<m::SourceEvent>> {
    serde_json::from_slice::<Vec<StrictValue>>(bytes)?
        .into_iter()
        .map(|StrictValue(value)| {
            let mut map = object(value)?;
            let category = text(take(&mut map, "type")?)?;
            if !matches!(category.as_str(), "trade" | "order_status") {
                return Err(invalid("unsupported source event"));
            }
            let body = normalize_observed(map, catalog, &category)?;
            let event: m::SourceEvent =
                serde_json::from_value(json!({"type":category,"data":body}))?;
            catalog.validate_source(&event)?;
            Ok(event)
        })
        .collect()
}
