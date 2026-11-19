//! Native query positions and their versioned, order-preserving boundary encoding.
use crate::{
    READ_LIMITS,
    query::Filter,
    ranking::ordered_decimal,
    types::{EntityKind, Key, Position as HistoryPosition, Sort},
};
use rust_decimal::Decimal;
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
        bytes.push(1);
        bytes.extend(self.selection.0);
        bytes.push(self.sort as u8);
        bytes.push(u8::from(self.at.is_some()));
        bytes.extend(ordered_decimal(if self.rank.is_zero() {
            Decimal::ZERO
        } else {
            self.rank
        }));
        if let Some(at) = self.at {
            bytes.extend(((at as u64) ^ (1 << 63)).to_be_bytes());
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
        if bytes.len() < 98 || bytes[0] != 1 || bytes[34] > 1 {
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
        if payload.len() < 62 {
            return Err(invalid());
        }
        let rank = decode_decimal(&payload[..58])?;
        let mut i = 58;
        let at = if bytes[34] == 1 {
            let raw: [u8; 8] = payload
                .get(i..i + 8)
                .ok_or_else(invalid)?
                .try_into()
                .map_err(|_| invalid())?;
            i += 8;
            Some((u64::from_be_bytes(raw) ^ (1 << 63)) as i64)
        } else {
            None
        };
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
    pub(crate) fn encoded_len(&self) -> usize {
        (35 + 58
            + usize::from(self.at.is_some()) * 8
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
                for rank in [
                    Decimal::MIN,
                    Decimal::new(-1, 28),
                    Decimal::ZERO,
                    Decimal::new(1, 28),
                    Decimal::MAX,
                ] {
                    for text in ["", "a", "a\0", "a\0b", "aa", "é", "😀"] {
                        for edge in [Edge::Before, Edge::At, Edge::After] {
                            let key = ReadPosition {
                                selection: QueryFingerprint([42; 32]),
                                sort,
                                key: Key::order("venue\0x".into(), text.into()),
                                rank,
                                at: history.then_some(-1),
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
        assert!(ReadPosition::decode(&(key + "00")).is_err());
    }
}
