//! Native query positions and their order-preserving boundary encoding.
use crate::{
    READ_LIMITS,
    query::Filter,
    ranking::ordered_decimal,
    types::{EntityKind, Key, OrderMode, Position as HistoryPosition, Sort},
};
use base64::{
    Engine,
    alphabet::Alphabet,
    engine::{GeneralPurpose, general_purpose::NO_PAD},
};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use std::{cmp::Ordering, str::FromStr};
use tonic::Status;

// ASCII-ordered, URL-unreserved characters preserve byte order without JSON escaping.
const KEY_ALPHABET: Alphabet =
    match Alphabet::new("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz~") {
        Ok(value) => value,
        Err(_) => panic!("valid ordered key alphabet"),
    };
const KEY_ENCODING: GeneralPurpose = GeneralPurpose::new(&KEY_ALPHABET, NO_PAD);
const HEADER_LEN: usize = 1;
/// Selected order modes and active/history storage, packed into one byte.
/// Filters and query identity do not belong to an ordering position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PositionSource(u8);
impl PositionSource {
    pub(crate) fn from_filter(filter: &Filter) -> Self {
        let modes = filter
            .scope
            .as_ref()
            .filter(|scope| !scope.modes.is_empty())
            .map_or(7, |scope| {
                scope.modes.iter().fold(0, |bits, mode| {
                    bits | match mode {
                        OrderMode::Ask => 1,
                        OrderMode::Bid => 2,
                        OrderMode::Executed => 4,
                    }
                })
            });
        Self(modes | if filter.history() { 8 } else { 0 })
    }
    pub(crate) fn decode(value: u8) -> Result<Self, Status> {
        // Executions alone always use history storage.
        if value & 7 == 0 || value == 4 || value > 15 {
            return Err(invalid());
        }
        Ok(Self(value))
    }
    fn history(self) -> bool {
        self.0 & 8 != 0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub(crate) enum Edge {
    Before = 0x7f,
    At = 0x80,
    After = 0x81,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReadPosition {
    pub(crate) source: PositionSource,
    pub(crate) sort: Sort,
    pub(crate) key: Key,
    pub(crate) rank: Decimal,
    pub(crate) at: Option<i64>,
    pub(crate) edge: Edge,
}
impl Ord for ReadPosition {
    fn cmp(&self, other: &Self) -> Ordering {
        let context = (self.sort as i32)
            .cmp(&(other.sort as i32))
            .then(self.source.cmp(&other.source));
        if context != Ordering::Equal {
            return context;
        }
        let order = self
            .rank
            .cmp(&other.rank)
            .then(self.at.cmp(&other.at))
            .then(self.key.cmp(&other.key));
        (if self.sort.descending() {
            order.reverse()
        } else {
            order
        })
        .then(self.edge.cmp(&other.edge))
    }
}
impl PartialOrd for ReadPosition {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl serde::Serialize for ReadPosition {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.encode())
    }
}
impl<'de> serde::Deserialize<'de> for ReadPosition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::decode(&encoded).map_err(serde::de::Error::custom)
    }
}
impl ReadPosition {
    pub fn encode(&self) -> String {
        let mut bytes = Vec::with_capacity(self.binary_len());
        bytes.push((self.sort as u8) << 4 | self.source.0);
        match self.sort {
            Sort::Identity => {}
            Sort::TimeAsc | Sort::TimeDesc => bytes.extend(ordered_integer(integer(self.rank))),
            Sort::NumberAsc | Sort::NumberDesc => {
                let missing = self.rank == Decimal::MAX;
                bytes.push(u8::from(missing));
                bytes.extend(if missing {
                    [0; 8]
                } else {
                    ordered_integer(integer(self.rank))
                });
            }
            Sort::PriceAsc | Sort::PriceDesc => bytes.extend(compact_decimal(self.rank)),
        }
        if let Some(at) = self.at.filter(|_| !self.time_rank()) {
            bytes.extend(ordered_integer(at));
        }
        for part in [&self.key.venue_id, &self.key.id] {
            for b in part.bytes() {
                bytes.push(b);
                if b == 0 {
                    bytes.push(255);
                }
            }
            bytes.extend([0, 0]);
        }
        if self.sort.descending() {
            for b in &mut bytes[HEADER_LEN..] {
                *b = 255 - *b;
            }
        }
        bytes.push(self.edge as u8);
        KEY_ENCODING.encode(bytes)
    }
    pub fn decode(value: &str) -> Result<Self, Status> {
        if value.len() > READ_LIMITS.max_position_bytes {
            return Err(invalid());
        }
        let bytes = KEY_ENCODING.decode(value).map_err(|_| invalid())?;
        if bytes.len() < HEADER_LEN + 5 {
            return Err(invalid());
        }
        let sort = Sort::try_from(i32::from(bytes[0] >> 4))?;
        let source = PositionSource::decode(bytes[0] & 15)?;
        let edge = match bytes.last() {
            Some(0x7f) => Edge::Before,
            Some(0x80) => Edge::At,
            Some(0x81) => Edge::After,
            _ => return Err(invalid()),
        };
        let mut payload = bytes[HEADER_LEN..bytes.len() - 1].to_vec();
        if sort.descending() {
            for b in &mut payload {
                *b = 255 - *b;
            }
        }
        let mut i = 0;
        let mut rank = match sort {
            Sort::Identity => Decimal::ZERO,
            Sort::TimeAsc | Sort::TimeDesc => Decimal::from(read_integer(&payload, &mut i)?),
            Sort::NumberAsc | Sort::NumberDesc => {
                let missing = *payload.get(i).ok_or_else(invalid)?;
                i += 1;
                let raw = read_integer(&payload, &mut i)?;
                match missing {
                    0 => Decimal::from(raw),
                    1 if raw == i64::MIN => Decimal::MAX,
                    _ => return Err(invalid()),
                }
            }
            Sort::PriceAsc | Sort::PriceDesc => {
                let raw = payload.get(..29).ok_or_else(invalid)?;
                i = 29;
                expand_decimal(raw)?
            }
        };
        let at = if source.history() {
            Some(if matches!(sort, Sort::TimeAsc | Sort::TimeDesc) {
                integer(rank)
            } else {
                read_integer(&payload, &mut i)?
            })
        } else {
            None
        };
        if sort == Sort::Identity
            && let Some(at) = at
        {
            rank = Decimal::from(at);
        }
        let venue_id = part(&payload, &mut i)?;
        let id = part(&payload, &mut i)?;
        if i != payload.len() {
            return Err(invalid());
        }
        let result = Self {
            source,
            sort,
            key: Key {
                kind: EntityKind::Order,
                venue_id,
                id,
            },
            rank,
            at,
            edge,
        };
        if result.encode() != value {
            return Err(invalid());
        }
        Ok(result)
    }
    pub(crate) fn validate(&self, filter: &Filter) -> Result<(), Status> {
        if self.sort != filter.sort || self.source != PositionSource::from_filter(filter) {
            return Err(Status::invalid_argument(
                "ordering key uses another sort or source",
            ));
        }
        Ok(())
    }
    pub(crate) fn after(&self) -> Self {
        let mut p = self.clone();
        if p.edge == Edge::At {
            p.edge = Edge::After;
        }
        p
    }
    pub(crate) fn history_position(&self) -> Result<HistoryPosition, Status> {
        Ok(HistoryPosition {
            executed_at_ms: self.at.ok_or_else(invalid)?,
            venue_id: self.key.venue_id.clone(),
            order_id: self.key.id.clone(),
        })
    }
    fn time_rank(&self) -> bool {
        matches!(self.sort, Sort::TimeAsc | Sort::TimeDesc)
    }
    pub(crate) fn encoded_len(&self) -> usize {
        (self.binary_len() * 8).div_ceil(6)
    }
    fn binary_len(&self) -> usize {
        HEADER_LEN
            + match self.sort {
                Sort::Identity => 0,
                Sort::TimeAsc | Sort::TimeDesc => 8,
                Sort::NumberAsc | Sort::NumberDesc => 9,
                Sort::PriceAsc | Sort::PriceDesc => 29,
            }
            + usize::from(self.at.is_some() && !self.time_rank()) * 8
            + self.key.venue_id.len()
            + self.key.id.len()
            + self
                .key
                .venue_id
                .bytes()
                .chain(self.key.id.bytes())
                .filter(|b| *b == 0)
                .count()
            + 5
    }
}
// Pack the existing exact decimal order into BCD without changing durable index keys.
fn compact_decimal(value: Decimal) -> [u8; 29] {
    let raw = ordered_decimal(if value.is_zero() {
        Decimal::ZERO
    } else {
        value
    });
    let mut out = [0; 29];
    out[0] = (raw[0] << 4) | (raw[1] - b'0');
    for i in 1..29 {
        out[i] = ((raw[2 * i] - b'0') << 4) | (raw[2 * i + 1] - b'0');
    }
    out
}
fn expand_decimal(raw: &[u8]) -> Result<Decimal, Status> {
    let mut digits = [b'0'; 58];
    if raw.len() != 29 || raw[0] >> 4 > 1 {
        return Err(invalid());
    }
    digits[0] = raw[0] >> 4;
    for (i, byte) in raw.iter().enumerate() {
        if byte & 15 > 9 || (i > 0 && byte >> 4 > 9) {
            return Err(invalid());
        }
        if i > 0 {
            digits[i * 2] = b'0' + (byte >> 4);
        }
        digits[i * 2 + 1] = b'0' + (byte & 15);
    }
    decode_decimal(&digits)
}
fn integer(value: Decimal) -> i64 {
    assert!(value.fract().is_zero(), "integer sort has an integral rank");
    value.to_i64().expect("integer sort rank fits i64")
}
fn ordered_integer(value: i64) -> [u8; 8] {
    ((value as u64) ^ (1 << 63)).to_be_bytes()
}
fn read_integer(bytes: &[u8], i: &mut usize) -> Result<i64, Status> {
    let raw: [u8; 8] = bytes
        .get(*i..*i + 8)
        .ok_or_else(invalid)?
        .try_into()
        .map_err(|_| invalid())?;
    *i += 8;
    Ok((u64::from_be_bytes(raw) ^ (1 << 63)) as i64)
}
fn invalid() -> Status {
    Status::invalid_argument("invalid ordering key")
}
fn part(bytes: &[u8], i: &mut usize) -> Result<String, Status> {
    let mut out = vec![];
    loop {
        let b = *bytes.get(*i).ok_or_else(invalid)?;
        *i += 1;
        if b != 0 {
            out.push(b);
            continue;
        }
        match bytes.get(*i) {
            Some(0) => {
                *i += 1;
                break;
            }
            Some(255) => {
                *i += 1;
                out.push(0);
            }
            _ => return Err(invalid()),
        }
    }
    String::from_utf8(out).map_err(|_| invalid())
}
fn decode_decimal(raw: &[u8]) -> Result<Decimal, Status> {
    if raw.len() != 58 || raw[0] > 1 {
        return Err(invalid());
    }
    let negative = raw[0] == 0;
    let mut digits = raw[1..].to_vec();
    if !digits.iter().all(u8::is_ascii_digit) {
        return Err(invalid());
    }
    if negative {
        for b in &mut digits {
            *b = b'9' - (*b - b'0');
        }
    }
    let whole = std::str::from_utf8(&digits[..29])
        .map_err(|_| invalid())?
        .trim_start_matches('0');
    let fraction = std::str::from_utf8(&digits[29..])
        .map_err(|_| invalid())?
        .trim_end_matches('0');
    let text = format!(
        "{}{}.{}",
        if negative { "-" } else { "" },
        if whole.is_empty() { "0" } else { whole },
        if fraction.is_empty() { "0" } else { fraction }
    );
    let value = Decimal::from_str(&text).map_err(|_| invalid())?;
    if ordered_decimal(value) != raw {
        return Err(invalid());
    }
    Ok(value)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapshotId(uuid::Uuid);
impl SnapshotId {
    pub(crate) fn fresh() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}
impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl FromStr for SnapshotId {
    type Err = Status;
    fn from_str(value: &str) -> Result<Self, Status> {
        uuid::Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| Status::invalid_argument("invalid snapshot ID"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_price_ranks_preserve_exact_order_for_scales_signs_and_extremes() {
        let mut values = vec![Decimal::MIN, Decimal::MAX, Decimal::ZERO];
        let mut seed = 17u128;
        for scale in 0..=28 {
            for _ in 0..32 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let coefficient = (seed & ((1u128 << 96) - 1)) as i128;
                values.push(Decimal::from_i128_with_scale(coefficient, scale));
                values.push(Decimal::from_i128_with_scale(-coefficient, scale));
            }
        }
        values.sort();
        for value in &values {
            assert_eq!(expand_decimal(&compact_decimal(*value)).unwrap(), *value);
        }
        assert!(
            values
                .windows(2)
                .all(|pair| compact_decimal(pair[0]) <= compact_decimal(pair[1]))
        );
        assert_eq!(
            compact_decimal("1.00".parse().unwrap()),
            compact_decimal(Decimal::ONE)
        );
    }
    #[test]
    fn compact_positions_remove_redundant_ranks_without_losing_history_identity() {
        for (sort, rank, history, expected) in [
            (Sort::Identity, Decimal::ZERO, false, 11),
            (Sort::Identity, Decimal::from(-1), true, 22),
            (Sort::PriceAsc, Decimal::ONE, false, 50),
            (Sort::PriceDesc, Decimal::ONE, true, 60),
            (Sort::TimeAsc, Decimal::from(-1), false, 22),
            (Sort::TimeDesc, Decimal::from(-1), true, 22),
            (Sort::NumberAsc, Decimal::MAX, false, 23),
            (Sort::NumberDesc, Decimal::from(i64::MIN), true, 34),
        ] {
            let position = ReadPosition {
                source: PositionSource(if history { 12 } else { 1 }),
                sort,
                key: Key::order("v".into(), "x".into()),
                rank,
                at: history.then_some(-1),
                edge: Edge::At,
            };
            let key = position.encode();
            assert_eq!(key.len(), expected);
            assert_eq!(ReadPosition::decode(&key).unwrap(), position);
        }
    }
    #[test]
    fn one_key_roundtrips_and_matches_native_order_for_every_sort_and_edge() {
        let mut contexts = Vec::new();
        for sort in [
            Sort::Identity,
            Sort::PriceAsc,
            Sort::PriceDesc,
            Sort::TimeAsc,
            Sort::TimeDesc,
            Sort::NumberAsc,
            Sort::NumberDesc,
        ] {
            for source in [1, 2, 3, 7, 9, 10, 11, 12, 15].map(PositionSource) {
                let history = source.history();
                let mut keys = vec![];
                let ranks = match sort {
                    Sort::Identity => vec![Decimal::ZERO],
                    Sort::TimeAsc | Sort::TimeDesc => {
                        [i64::MIN, -1, 0, 1, i64::MAX].map(Decimal::from).to_vec()
                    }
                    Sort::NumberAsc | Sort::NumberDesc => vec![
                        Decimal::from(i64::MIN),
                        Decimal::ZERO,
                        Decimal::from(i64::MAX),
                        Decimal::MAX,
                    ],
                    _ => vec![
                        Decimal::MIN,
                        Decimal::new(-1, 28),
                        Decimal::ZERO,
                        Decimal::new(1, 28),
                        Decimal::MAX,
                    ],
                };
                for rank in ranks {
                    for text in ["", "a", "a\0", "a\0b", "aa", "é", "😀"] {
                        for edge in [Edge::Before, Edge::At, Edge::After] {
                            let at = history.then_some(
                                if matches!(sort, Sort::TimeAsc | Sort::TimeDesc) {
                                    integer(rank)
                                } else {
                                    -1
                                },
                            );
                            let key = ReadPosition {
                                source,
                                sort,
                                key: Key::order("venue\0x".into(), text.into()),
                                rank: if sort == Sort::Identity && history {
                                    Decimal::from(-1)
                                } else {
                                    rank
                                },
                                at,
                                edge,
                            };
                            let encoded = key.encode();
                            assert_eq!(key.encoded_len(), encoded.len());
                            assert_eq!(ReadPosition::decode(&encoded).unwrap(), key);
                            let json = serde_json::to_string(&key).unwrap();
                            assert_eq!(json, format!("\"{encoded}\""));
                            assert_eq!(serde_json::from_str::<ReadPosition>(&json).unwrap(), key);
                            keys.push(key);
                        }
                    }
                }
                keys.sort();
                assert!(
                    keys.windows(2).all(|p| p[0].encode() < p[1].encode()),
                    "{sort:?}, history={history}"
                );
                contexts.extend(keys);
            }
        }
        contexts.sort();
        assert!(
            contexts
                .windows(2)
                .all(|pair| pair[0].encode() < pair[1].encode())
        );
    }
    #[test]
    fn malformed_and_noncanonical_keys_are_rejected_without_panics() {
        for input in ["", "0", "GG", "00", "1001TW00OG00W0", &"ff".repeat(5000)] {
            assert!(ReadPosition::decode(input).is_err());
        }
        let key = ReadPosition {
            source: PositionSource(1),
            sort: Sort::PriceAsc,
            key: Key::order("v".into(), "xy".into()),
            rank: Decimal::ONE,
            at: None,
            edge: Edge::At,
        }
        .encode();
        for i in 0..key.len() {
            assert!(ReadPosition::decode(&key[..i]).is_err());
        }
        for (offset, value) in [
            (0, 0xff),
            (0, 0x71),
            (0, 0x10),
            (0, 0x14),
            (0, 0x18),
            (1, 0x20),
            (2, 0xfa),
        ] {
            let mut malformed = KEY_ENCODING.decode(&key).unwrap();
            malformed[offset] = value;
            assert!(ReadPosition::decode(&KEY_ENCODING.encode(malformed)).is_err());
        }
        assert!(ReadPosition::decode(&(key.clone() + "=")).is_err());
        assert!(ReadPosition::decode(&(key.clone() + "0")).is_err());
        // This payload leaves two padding bits. A second spelling of the same
        // bytes would break string equality and boundary comparisons.
        assert_eq!(key.len() % 4, 3);
        let last = KEY_ALPHABET
            .as_str()
            .find(key.chars().last().unwrap())
            .unwrap();
        let mut noncanonical = key.into_bytes();
        *noncanonical.last_mut().unwrap() = KEY_ALPHABET.as_str().as_bytes()[last + 1];
        let noncanonical = String::from_utf8(noncanonical).unwrap();
        assert!(ReadPosition::decode(&noncanonical).is_err());
        assert!(serde_json::from_value::<ReadPosition>(serde_json::json!(noncanonical)).is_err());
    }
}
