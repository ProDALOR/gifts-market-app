use super::{Result, Scalar};
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
