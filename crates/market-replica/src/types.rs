//! Validated internal values. Numeric codes and tuples are confined to wire/storage boundaries.
use market_hub_protocol::wire as w;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tonic::Status;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i32)]
pub enum EntityKind {
    Venue = 1,
    Currency = 2,
    ItemType = 3,
    Attribute = 4,
    AttributeOption = 5,
    Feed = 6,
    Order = 7,
}
impl TryFrom<i32> for EntityKind {
    type Error = Status;
    fn try_from(value: i32) -> Result<Self, Status> {
        match w::EntityKind::try_from(value) {
            Ok(w::EntityKind::Venue) => Ok(Self::Venue),
            Ok(w::EntityKind::Currency) => Ok(Self::Currency),
            Ok(w::EntityKind::ItemType) => Ok(Self::ItemType),
            Ok(w::EntityKind::Attribute) => Ok(Self::Attribute),
            Ok(w::EntityKind::AttributeOption) => Ok(Self::AttributeOption),
            Ok(w::EntityKind::Feed) => Ok(Self::Feed),
            Ok(w::EntityKind::Order) => Ok(Self::Order),
            _ => Err(Status::invalid_argument("unknown entity kind")),
        }
    }
}
impl EntityKind {
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Venue => Some(Self::Currency),
            Self::Currency => Some(Self::ItemType),
            Self::ItemType => Some(Self::Attribute),
            Self::Attribute => Some(Self::AttributeOption),
            Self::AttributeOption => Some(Self::Feed),
            Self::Feed => Some(Self::Order),
            Self::Order => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "(i32, String, String)", into = "(i32, String, String)")]
pub struct Key {
    pub kind: EntityKind,
    pub venue_id: String,
    pub id: String,
}
impl Key {
    pub fn first(kind: EntityKind) -> Self {
        Self {
            kind,
            venue_id: String::new(),
            id: String::new(),
        }
    }
    pub fn order(venue_id: String, id: String) -> Self {
        Self {
            kind: EntityKind::Order,
            venue_id,
            id,
        }
    }
    pub fn stored(&self) -> (i32, &str, &str) {
        (self.kind as i32, &self.venue_id, &self.id)
    }
}
impl TryFrom<&w::Key> for Key {
    type Error = Status;
    fn try_from(value: &w::Key) -> Result<Self, Status> {
        (value.kind, value.venue_id.clone(), value.id.clone()).try_into()
    }
}
impl TryFrom<(i32, String, String)> for Key {
    type Error = Status;
    fn try_from((kind, venue_id, id): (i32, String, String)) -> Result<Self, Status> {
        let kind = EntityKind::try_from(kind)?;
        if kind != EntityKind::Order && !venue_id.is_empty() {
            return Err(Status::invalid_argument("catalog keys cannot have a venue"));
        }
        Ok(Self { kind, venue_id, id })
    }
}
impl From<Key> for (i32, String, String) {
    fn from(key: Key) -> Self {
        (key.kind as i32, key.venue_id, key.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrderMode {
    Ask,
    Bid,
    Executed,
}
impl TryFrom<i32> for OrderMode {
    type Error = Status;
    fn try_from(value: i32) -> Result<Self, Status> {
        match w::OrderMode::try_from(value) {
            Ok(w::OrderMode::Ask) => Ok(Self::Ask),
            Ok(w::OrderMode::Bid) => Ok(Self::Bid),
            Ok(w::OrderMode::Executed) => Ok(Self::Executed),
            _ => Err(Status::invalid_argument("unknown order mode")),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActiveMode {
    Ask,
    Bid,
}
impl TryFrom<i32> for ActiveMode {
    type Error = Status;
    fn try_from(value: i32) -> Result<Self, Status> {
        match OrderMode::try_from(value)? {
            OrderMode::Ask => Ok(Self::Ask),
            OrderMode::Bid => Ok(Self::Bid),
            OrderMode::Executed => Err(Status::data_loss("execution cannot enter an active book")),
        }
    }
}
impl From<ActiveMode> for OrderMode {
    fn from(value: ActiveMode) -> Self {
        match value {
            ActiveMode::Ask => Self::Ask,
            ActiveMode::Bid => Self::Bid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "i32", into = "i32")]
#[repr(i32)]
pub enum Sort {
    Identity = 0,
    PriceAsc = 1,
    PriceDesc = 2,
    TimeAsc = 3,
    TimeDesc = 4,
    NumberAsc = 5,
    NumberDesc = 6,
}
impl TryFrom<i32> for Sort {
    type Error = Status;
    fn try_from(value: i32) -> Result<Self, Status> {
        match w::ReadSort::try_from(value) {
            Ok(w::ReadSort::Identity) => Ok(Self::Identity),
            Ok(w::ReadSort::PriceAsc) => Ok(Self::PriceAsc),
            Ok(w::ReadSort::PriceDesc) => Ok(Self::PriceDesc),
            Ok(w::ReadSort::TimeAsc) => Ok(Self::TimeAsc),
            Ok(w::ReadSort::TimeDesc) => Ok(Self::TimeDesc),
            Ok(w::ReadSort::NumberAsc) => Ok(Self::NumberAsc),
            Ok(w::ReadSort::NumberDesc) => Ok(Self::NumberDesc),
            _ => Err(Status::invalid_argument("unknown read sort")),
        }
    }
}
impl From<Sort> for i32 {
    fn from(value: Sort) -> Self {
        value as i32
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "(i64, String, String)", into = "(i64, String, String)")]
pub struct Position {
    pub executed_at_ms: i64,
    pub venue_id: String,
    pub order_id: String,
}
impl Position {
    pub fn stored(&self) -> (i64, &str, &str) {
        (self.executed_at_ms, &self.venue_id, &self.order_id)
    }
}
impl From<(i64, String, String)> for Position {
    fn from((executed_at_ms, venue_id, order_id): (i64, String, String)) -> Self {
        Self {
            executed_at_ms,
            venue_id,
            order_id,
        }
    }
}
impl From<Position> for (i64, String, String) {
    fn from(value: Position) -> Self {
        (value.executed_at_ms, value.venue_id, value.order_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RankIdentity {
    First,
    Entity(Key),
    Last,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RankKey {
    pub value: Decimal,
    identity: RankIdentity,
}
impl RankKey {
    pub fn new(value: Decimal, key: Key) -> Self {
        Self {
            value,
            identity: RankIdentity::Entity(key),
        }
    }
    pub fn first(value: Decimal) -> Self {
        Self {
            value,
            identity: RankIdentity::First,
        }
    }
    pub fn last(value: Decimal) -> Self {
        Self {
            value,
            identity: RankIdentity::Last,
        }
    }
    pub fn entity(&self) -> (Decimal, &Key) {
        match &self.identity {
            RankIdentity::Entity(key) => (self.value, key),
            _ => unreachable!("range boundary cannot be inserted into a rank index"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum IndexKind {
    Venue = 0,
    ItemType = 1,
    Book = 2,
    Attribute = 3,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IndexTerm {
    pub kind: IndexKind,
    pub path: Vec<u8>,
    pub value: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Leaf {
    pub path: Vec<u8>,
    pub value: Vec<u8>,
}
