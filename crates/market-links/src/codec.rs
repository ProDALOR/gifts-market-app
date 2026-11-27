use crate::{LinkError, MarketSelection, Result};
use market_gifts::*;
use market_gifts_hub::input::{BackdropRef, CollectionRef, GiftFilters, ModelRef, PatternRef};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_SELECTION_BYTES: usize = 256 * 1024;
pub const LAUNCH_LIMIT: usize = 512;
const ATOM: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'@')
    .remove(b':')
    .remove(b'.')
    .remove(b'_')
    .remove(b'-')
    .remove(b'~');
fn encode(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, ATOM).to_string()
}
fn decode(value: &str) -> Result<String> {
    let b = value.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'%'
            && (i + 2 >= b.len() || !b[i + 1].is_ascii_hexdigit() || !b[i + 2].is_ascii_hexdigit())
        {
            return Err(LinkError::Invalid("Invalid filter escaping"));
        }
    }
    percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map(|v| v.into_owned())
        .map_err(|_| LinkError::Invalid("Invalid filter text"))
}
fn values<T: TryFrom<String>>(text: &str) -> Result<Vec<T>> {
    if text.is_empty() {
        return Err(LinkError::Invalid("Empty filter group"));
    }
    text.split(',')
        .map(|value| {
            T::try_from(decode(value)?).map_err(|_| LinkError::Invalid("Invalid catalog reference"))
        })
        .collect()
}
fn number(text: &str) -> Result<GiftNumber> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(LinkError::Invalid("Invalid gift number"));
    }
    text.parse::<u32>()
        .ok()
        .and_then(|n| GiftNumber::new(n).ok())
        .ok_or(LinkError::Invalid("Invalid gift number"))
}
impl MarketSelection {
    pub fn filter_line(&self) -> String {
        let mut parts = Vec::new();
        if let Some(c) = &self.filters.collection_id {
            parts.push(format!("c={}", encode(&c.to_string())));
        }
        for (key, values) in [
            (
                "m",
                self.filters
                    .models
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            ),
            (
                "b",
                self.filters
                    .backdrops
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
            (
                "s",
                self.filters
                    .patterns
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
        ] {
            if !values.is_empty() {
                parts.push(format!(
                    "{key}={}",
                    values
                        .iter()
                        .map(|v| encode(v))
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
        }
        if let Some(n) = &self.filters.number {
            if n.min.is_some() || n.max.is_some() {
                parts.push(if n.min == n.max {
                    format!("n={}", n.min.unwrap())
                } else {
                    format!(
                        "n={}..{}",
                        n.min.map(|v| v.to_string()).unwrap_or_default(),
                        n.max.map(|v| v.to_string()).unwrap_or_default()
                    )
                });
            }
        }
        parts.join("|")
    }
    fn apply_line(&mut self, line: &str) -> Result<()> {
        if line.is_empty() {
            return Ok(());
        }
        let mut seen = BTreeSet::new();
        for part in line.split('|') {
            let (key, value) = part
                .split_once('=')
                .ok_or(LinkError::Invalid("Filter groups use key=value"))?;
            if !seen.insert(key) {
                return Err(LinkError::Invalid("Repeated filter group"));
            }
            match key {
                "c" => {
                    let mut values = values::<CollectionRef>(value)?;
                    if values.len() != 1 {
                        return Err(LinkError::Invalid("Select one collection"));
                    }
                    self.filters.collection_id = values.pop();
                }
                "m" => self.filters.models = values::<ModelRef>(value)?,
                "b" => self.filters.backdrops = values::<BackdropRef>(value)?,
                "s" => self.filters.patterns = values::<PatternRef>(value)?,
                "n" => {
                    if value == ".." {
                        return Err(LinkError::Invalid("Empty gift number range"));
                    }
                    self.filters.number = Some(if let Some((min, max)) = value.split_once("..") {
                        GiftNumberRange {
                            min: (!min.is_empty()).then(|| number(min)).transpose()?,
                            max: (!max.is_empty()).then(|| number(max)).transpose()?,
                        }
                    } else {
                        let n = number(value)?;
                        GiftNumberRange {
                            min: Some(n),
                            max: Some(n),
                        }
                    })
                }
                _ => return Err(LinkError::Invalid("Unknown filter group")),
            }
        }
        Ok(())
    }
    pub fn from_query(query: &str) -> Result<Self> {
        if query.len() > MAX_SELECTION_BYTES {
            return Err(LinkError::TooLarge);
        }
        let mut p = BTreeMap::new();
        const KEYS: &[&str] = &[
            "f",
            "collection",
            "currency",
            "venue",
            "mode",
            "bid_match",
            "sort",
            "filters",
            "min",
            "max",
            "from",
            "until",
            "gift",
        ];
        for (key, value) in url::form_urlencoded::parse(query.trim_start_matches('?').as_bytes()) {
            if KEYS.contains(&key.as_ref())
                && p.insert(key.into_owned(), value.into_owned()).is_some()
            {
                return Err(LinkError::Invalid("Repeated selection parameter"));
            }
        }
        let get = |key: &str| p.get(key).map(String::as_str).filter(|v| !v.is_empty());
        let currency = get("currency")
            .unwrap_or("GRAM")
            .to_owned()
            .try_into()
            .map_err(|_| LinkError::Invalid("Invalid currency"))?;
        let mut filters = GiftFilters::new("unused".into(), currency);
        filters.collection_id = get("collection")
            .filter(|_| !p.contains_key("f"))
            .map(|v| CollectionRef::new(v))
            .transpose()
            .map_err(|_| LinkError::Invalid("Invalid collection"))?;
        filters.venue_id = get("venue")
            .map(|v| VenueId::new(v))
            .transpose()
            .map_err(|_| LinkError::Invalid("Invalid venue"))?;
        let price = PriceRange {
            min: get("min")
                .map(|v| Amount::new(v))
                .transpose()
                .map_err(|_| LinkError::Invalid("Invalid price"))?,
            max: get("max")
                .map(|v| Amount::new(v))
                .transpose()
                .map_err(|_| LinkError::Invalid("Invalid price"))?,
        };
        if price.min.is_some() || price.max.is_some() {
            filters.price = Some(price);
        }
        if let Some(legacy) = get("filters").filter(|_| !p.contains_key("f")) {
            #[derive(serde::Deserialize)]
            #[serde(default, deny_unknown_fields)]
            struct Legacy {
                models: Vec<ModelRef>,
                backdrops: Vec<BackdropRef>,
                patterns: Vec<PatternRef>,
                number: Option<GiftNumberRange>,
            }
            impl Default for Legacy {
                fn default() -> Self {
                    Self {
                        models: vec![],
                        backdrops: vec![],
                        patterns: vec![],
                        number: None,
                    }
                }
            }
            let old: Legacy = serde_json::from_str(legacy)
                .map_err(|_| LinkError::Invalid("Invalid legacy filters"))?;
            filters.models = old.models;
            filters.backdrops = old.backdrops;
            filters.patterns = old.patterns;
            filters.number = old.number;
        }
        let mut selection = Self::new(filters);
        if let Some(line) = p.get("f") {
            // A readable line replaces the complete attribute selection, never only some groups.
            selection.filters.collection_id = None;
            selection.filters.models.clear();
            selection.filters.backdrops.clear();
            selection.filters.patterns.clear();
            selection.filters.number = None;
            selection.apply_line(line)?;
        }
        selection.source = match get("mode").unwrap_or("ask") {
            "ask" => OrderSource::Asks,
            "bid" => OrderSource::Bids,
            "executed" => OrderSource::History,
            _ => return Err(LinkError::Invalid("Invalid market mode")),
        };
        selection.bid_match = match get("bid_match").unwrap_or("all") {
            "all" => BidMatch::All,
            "any" => BidMatch::Any,
            _ => return Err(LinkError::Invalid("Invalid bid matching")),
        };
        selection.sort = get("sort")
            .map(|v| {
                serde_json::from_value(serde_json::Value::String(v.into()))
                    .map_err(|_| LinkError::Invalid("Invalid sort"))
            })
            .transpose()?;
        let period = Period {
            from: get("from")
                .map(str::parse)
                .transpose()
                .map_err(|_| LinkError::Invalid("Invalid history period"))?,
            until: get("until")
                .map(str::parse)
                .transpose()
                .map_err(|_| LinkError::Invalid("Invalid history period"))?,
        };
        if period.from.is_some() || period.until.is_some() {
            selection.period = Some(period);
        }
        selection.gift = get("gift")
            .map(GiftSlug::new)
            .transpose()
            .map_err(|_| LinkError::Invalid("Invalid gift slug"))?;
        selection.validate()?;
        Ok(selection)
    }
    pub fn to_query(&self) -> Result<String> {
        self.validate()?;
        let mut p = url::form_urlencoded::Serializer::new(String::new());
        let f = self.filter_line();
        // Keep an explicit empty scope so old launch attributes cannot leak back in.
        p.append_pair("f", &f);
        p.append_pair("currency", self.filters.currency_id.as_str());
        if let Some(venue) = &self.filters.venue_id {
            p.append_pair("venue", venue.as_str());
        }
        p.append_pair(
            "mode",
            match self.source {
                OrderSource::Asks => "ask",
                OrderSource::Bids => "bid",
                OrderSource::History => "executed",
            },
        );
        if self.source == OrderSource::Bids || self.bid_match == BidMatch::Any {
            p.append_pair(
                "bid_match",
                if self.bid_match == BidMatch::All {
                    "all"
                } else {
                    "any"
                },
            );
        }
        if let Some(sort) = self.sort {
            let v = serde_json::to_value(sort).map_err(|_| LinkError::Invalid("Invalid sort"))?;
            p.append_pair("sort", v.as_str().unwrap());
        }
        if let Some(price) = &self.filters.price {
            if let Some(min) = &price.min {
                p.append_pair("min", &min.to_string());
            }
            if let Some(max) = &price.max {
                p.append_pair("max", &max.to_string());
            }
        }
        if let Some(period) = &self.period {
            if let Some(from) = period.from {
                p.append_pair("from", &from.to_string());
            }
            if let Some(until) = period.until {
                p.append_pair("until", &until.to_string());
            }
        }
        if let Some(gift) = &self.gift {
            p.append_pair("gift", &gift.to_string());
        }
        let query = p.finish();
        if query.len() > MAX_SELECTION_BYTES {
            return Err(LinkError::TooLarge);
        }
        Ok(query)
    }
}

pub(crate) fn website_base(origin: &url::Url) -> Result<url::Url> {
    if !matches!(origin.scheme(), "http" | "https")
        || origin.host_str().is_none()
        || !origin.username().is_empty()
        || origin.password().is_some()
    {
        return Err(LinkError::Invalid("Invalid public app URL"));
    }
    Ok(origin.clone())
}
