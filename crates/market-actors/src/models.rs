//! Actor-owned queries and data, independent of hub transport and replica storage.
use jiff::Timestamp;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BookKey {
    pub venue_id: String,
    pub item_type_id: String,
    pub currency_id: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Ask,
    Bid,
    Executed,
}
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Sort {
    Identity,
    #[default]
    PriceAsc,
    PriceDesc,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PriceRange {
    pub min: Option<String>,
    pub max: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeValue {
    Enum {
        value: String,
    },
    Integer {
        value: String,
    },
    Decimal {
        value: String,
    },
    Boolean {
        value: bool,
    },
    Text {
        value: String,
    },
    Timestamp {
        value: Timestamp,
    },
    List {
        value: Vec<AttributeValue>,
    },
    Object {
        value: BTreeMap<String, AttributeValue>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    Eq {
        value: AttributeValue,
    },
    OneOf {
        values: Vec<AttributeValue>,
    },
    Range {
        min: Option<AttributeValue>,
        max: Option<AttributeValue>,
    },
    Object {
        fields: BTreeMap<String, Predicate>,
    },
    Contains {
        predicate: Box<Predicate>,
        min_count: Option<u32>,
        max_count: Option<u32>,
    },
    AllOf {
        predicates: Vec<Predicate>,
    },
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scope {
    pub venue_id: Option<String>,
    pub item_type_id: Option<String>,
    pub currency_id: Option<String>,
    pub modes: Vec<Mode>,
    pub attributes: BTreeMap<String, Predicate>,
    pub price: Option<PriceRange>,
}
fn default_limit() -> u32 {
    50
}
#[derive(Debug, Clone, PartialEq)]
pub struct OrdersQuery {
    pub scope: Scope,
    pub sort: Sort,
    pub limit: u32,
    pub page_token: String,
}
impl Default for OrdersQuery {
    fn default() -> Self {
        Self {
            scope: Scope::default(),
            sort: Sort::default(),
            limit: default_limit(),
            page_token: String::new(),
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Period {
    pub from: Option<Timestamp>,
    pub until: Option<Timestamp>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionsQuery {
    pub scope: Scope,
    pub period: Option<Period>,
    pub limit: u32,
    pub page_token: String,
}
impl Default for ExecutionsQuery {
    fn default() -> Self {
        Self {
            scope: Scope::default(),
            period: None,
            limit: default_limit(),
            page_token: String::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub id: String,
    pub book: BookKey,
    pub mode: Mode,
    pub item_id: Option<String>,
    pub attributes: BTreeMap<String, Predicate>,
    pub price: String,
    pub quantity: String,
    pub observed_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub executed_at: Option<Timestamp>,
    pub parent_order_id: Option<String>,
    pub meta: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Page<T> {
    pub data: Vec<T>,
    pub next_page_token: String,
    pub cursor: String,
    pub sync: SyncStatus,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Alias {
    pub namespace: String,
    pub id: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeRule {
    pub required: bool,
    pub allowed_values: Option<Vec<String>>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum CatalogEntry {
    Venue {
        id: String,
        name: String,
    },
    Currency {
        id: String,
        name: String,
        decimals: u32,
    },
    ItemType {
        id: String,
        name: String,
        quantity_step: String,
        aliases: Vec<Alias>,
        attributes: BTreeMap<String, AttributeRule>,
    },
    Attribute {
        id: String,
        name: String,
        value_type: String,
        minimum: Option<AttributeValue>,
        maximum: Option<AttributeValue>,
        max_length: Option<u32>,
        element: Option<String>,
        fields: BTreeMap<String, AttributeRule>,
    },
    AttributeOption {
        id: String,
        name: String,
        attribute_id: String,
        aliases: Vec<Alias>,
        meta: BTreeMap<String, String>,
    },
    Feed {
        id: String,
        books: Vec<BookKey>,
        modes: Vec<Mode>,
        attributes: BTreeMap<String, Predicate>,
        last_verified_at: Option<Timestamp>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CatalogKind {
    Venue,
    Currency,
    ItemType,
    Attribute,
    AttributeOption,
    Feed,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogQuery {
    pub kind: CatalogKind,
    pub ids: Vec<String>,
    pub limit: u32,
    pub page_token: String,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BooksQuery {
    pub after: Option<BookKey>,
    pub limit: u32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct BooksPage {
    pub data: Vec<BookKey>,
    pub next_after: Option<BookKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    Starting,
    Syncing,
    Live,
    Stale,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStatus {
    pub ready: bool,
    pub state: SyncState,
    pub updated_at: Option<i64>,
}
