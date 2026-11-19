//! Native query positions and their versioned, order-preserving boundary encoding.
use crate::{
    READ_LIMITS,
    query::Filter,
    ranking::ordered_decimal,
    types::{EntityKind, Key, Position as HistoryPosition, Sort},
};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use std::{cmp::Ordering, str::FromStr};
use tonic::Status;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct QueryFingerprint([u8; 32]);
impl QueryFingerprint {
    pub(crate) fn parse(value: &str) -> Result<Self, Status> {
        let raw = unhex(value)?;
        Ok(Self(raw.try_into().map_err(|_| invalid())?))
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
    pub(crate) selection: QueryFingerprint,
    pub(crate) sort: Sort,
    pub(crate) key: Key,
    pub(crate) rank: Decimal,
    pub(crate) at: Option<i64>,
    pub(crate) edge: Edge,
}
impl Ord for ReadPosition {
    fn cmp(&self, other: &Self) -> Ordering {
        let context = self
            .selection
            .cmp(&other.selection)
            .then((self.sort as i32).cmp(&(other.sort as i32)))
            .then(self.at.is_some().cmp(&other.at.is_some()));
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
impl ReadPosition {
    pub fn encode(&self) -> String {
        let mut bytes = Vec::with_capacity(self.encoded_len() / 2);
        bytes.push(2);
        bytes.extend(self.selection.0);
        bytes.push(self.sort as u8);
        bytes.push(u8::from(self.at.is_some()));
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
            for b in &mut bytes[35..] {
                *b = 255 - *b;
            }
        }
        bytes.push(self.edge as u8);
        hex(&bytes)
    }
    pub fn decode(value: &str) -> Result<Self, Status> {
        if value.len() > READ_LIMITS.max_position_bytes {
            return Err(invalid());
        }
        let bytes = unhex(value)?;
        if bytes.len() < 40 || bytes[0] != 2 || bytes[34] > 1 {
            return Err(invalid());
        }
        let selection = QueryFingerprint(bytes[1..33].try_into().map_err(|_| invalid())?);
        let sort = Sort::try_from(i32::from(bytes[33]))?;
        let edge = match bytes.last() {
            Some(0x7f) => Edge::Before,
            Some(0x80) => Edge::At,
            Some(0x81) => Edge::After,
            _ => return Err(invalid()),
        };
        let mut payload = bytes[35..bytes.len() - 1].to_vec();
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
        let at = if bytes[34] == 1 {
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
            selection,
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
        if self.selection != QueryFingerprint::parse(&filter.fingerprint)?
            || self.sort != filter.sort
            || self.at.is_some() != filter.history()
        {
            return Err(Status::invalid_argument(
                "ordering key belongs to another query",
            ));
        }
        Ok(())
    }
    pub(crate) fn before(&self) -> Self {
        let mut p = self.clone();
        if p.edge == Edge::At {
            p.edge = Edge::Before;
        }
        p
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
        (35 + match self.sort {
            Sort::Identity => 0,
            Sort::TimeAsc | Sort::TimeDesc => 8,
            Sort::NumberAsc | Sort::NumberDesc => 9,
            Sort::PriceAsc | Sort::PriceDesc => 29,
        } + usize::from(self.at.is_some() && !self.time_rank()) * 8
            + self.key.venue_id.len()
            + self.key.id.len()
            + self
                .key
                .venue_id
                .bytes()
                .chain(self.key.id.bytes())
                .filter(|b| *b == 0)
                .count()
            + 5)
            * 2
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
fn hex(raw: &[u8]) -> String {
    const H: &[u8] = b"0123456789abcdef";
    let mut output = String::with_capacity(raw.len() * 2);
    for b in raw {
        output.push(H[(b >> 4) as usize] as char);
        output.push(H[(b & 15) as usize] as char);
    }
    output
}
fn unhex(raw: &str) -> Result<Vec<u8>, Status> {
    if !raw.len().is_multiple_of(2) {
        return Err(invalid());
    }
    raw.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| Ok((nibble(p[0])? << 4) | nibble(p[1])?))
        .collect()
}
fn nibble(b: u8) -> Result<u8, Status> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        _ => Err(invalid()),
    }
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
            (Sort::Identity, Decimal::ZERO, false, 84),
            (Sort::Identity, Decimal::from(-1), true, 100),
            (Sort::PriceAsc, Decimal::ONE, false, 142),
            (Sort::PriceDesc, Decimal::ONE, true, 158),
            (Sort::TimeAsc, Decimal::from(-1), false, 100),
            (Sort::TimeDesc, Decimal::from(-1), true, 100),
            (Sort::NumberAsc, Decimal::MAX, false, 102),
            (Sort::NumberDesc, Decimal::from(i64::MIN), true, 118),
        ] {
            let position = ReadPosition {
                selection: QueryFingerprint([0; 32]),
                sort,
                key: Key::order("v".into(), "x".into()),
                rank,
                at: history.then_some(-1),
                edge: Edge::At,
            };
            let key = position.encode();
            assert_eq!(key.len(), expected);
            assert_eq!(ReadPosition::decode(&key).unwrap(), position);
            let mut retired = key.clone();
            retired.replace_range(..2, "01");
            assert!(ReadPosition::decode(&retired).is_err());
        }
    }
    #[test]
    fn one_key_roundtrips_and_matches_native_order_for_every_sort_and_edge() {
        for sort in [
            Sort::Identity,
            Sort::PriceAsc,
            Sort::PriceDesc,
            Sort::TimeAsc,
            Sort::TimeDesc,
            Sort::NumberAsc,
            Sort::NumberDesc,
        ] {
            for history in [false, true] {
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
                                selection: QueryFingerprint([42; 32]),
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
                            keys.push(key);
                        }
                    }
                }
                keys.sort();
                assert!(
                    keys.windows(2).all(|p| p[0].encode() < p[1].encode()),
                    "{sort:?}, history={history}"
                );
            }
        }
    }
    #[test]
    fn malformed_and_noncanonical_keys_are_rejected_without_panics() {
        for input in ["", "0", "GG", "00", &"ff".repeat(5000)] {
            assert!(ReadPosition::decode(input).is_err());
        }
        let key = ReadPosition {
            selection: QueryFingerprint([0xab; 32]),
            sort: Sort::PriceAsc,
            key: Key::order("v".into(), "x".into()),
            rank: Decimal::ONE,
            at: None,
            edge: Edge::At,
        }
        .encode();
        for i in 0..key.len() {
            assert!(ReadPosition::decode(&key[..i]).is_err());
        }
        assert!(ReadPosition::decode(&key.to_uppercase()).is_err());
        let mut wrong_version = key.clone();
        wrong_version.replace_range(..2, "ff");
        assert!(ReadPosition::decode(&wrong_version).is_err());
        for (offset, value) in [(68, "02"), (70, "20"), (72, "fa")] {
            let mut malformed = key.clone();
            malformed.replace_range(offset..offset + 2, value);
            assert!(ReadPosition::decode(&malformed).is_err());
        }
        assert!(ReadPosition::decode(&(key + "00")).is_err());
    }
}
