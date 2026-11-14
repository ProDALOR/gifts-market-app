use hub_protocol::wire as w;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct BookKey {
    pub venue_id: String,
    pub item_type_id: String,
    pub currency_id: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Ask,
    Bid,
    Executed,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    Identity,
    #[default]
    PriceAsc,
    PriceDesc,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PriceRange {
    pub min: Option<String>,
    pub max: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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
        #[schema(value_type=String,format=DateTime)]
        value: Timestamp,
    },
    List {
        #[schema(no_recursion)]
        value: Vec<AttributeValue>,
    },
    Object {
        #[schema(no_recursion)]
        value: BTreeMap<String, AttributeValue>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
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
        #[schema(no_recursion)]
        fields: BTreeMap<String, Predicate>,
    },
    Contains {
        #[schema(no_recursion)]
        predicate: Box<Predicate>,
        min_count: Option<u32>,
        max_count: Option<u32>,
    },
    AllOf {
        #[schema(no_recursion)]
        predicates: Vec<Predicate>,
    },
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Period {
    #[schema(value_type=Option<String>, format=DateTime)]
    pub from: Option<Timestamp>,
    #[schema(value_type=Option<String>, format=DateTime)]
    pub until: Option<Timestamp>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
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
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct Order {
    pub id: String,
    pub book: BookKey,
    pub mode: Mode,
    pub item_id: Option<String>,
    pub attributes: BTreeMap<String, Predicate>,
    pub price: String,
    pub quantity: String,
    #[schema(value_type=String, format=DateTime)]
    pub observed_at: Timestamp,
    #[schema(value_type=Option<String>, format=DateTime)]
    pub expires_at: Option<Timestamp>,
    #[schema(value_type=Option<String>, format=DateTime)]
    pub executed_at: Option<Timestamp>,
    pub parent_order_id: Option<String>,
    pub meta: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Page<T> {
    pub data: Vec<T>,
    pub next_page_token: String,
    pub cursor: String,
    pub sync: SyncInfo,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Alias {
    pub namespace: String,
    pub id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AttributeRule {
    pub required: bool,
    pub allowed_values: Option<Vec<String>>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CatalogKind {
    Venue,
    Currency,
    ItemType,
    Attribute,
    AttributeOption,
}
impl CatalogKind {
    pub fn wire(self) -> w::EntityKind {
        match self {
            Self::Venue => w::EntityKind::Venue,
            Self::Currency => w::EntityKind::Currency,
            Self::ItemType => w::EntityKind::ItemType,
            Self::Attribute => w::EntityKind::Attribute,
            Self::AttributeOption => w::EntityKind::AttributeOption,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogQuery {
    pub kind: CatalogKind,
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub page_token: String,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BooksQuery {
    pub after: Option<BookKey>,
    pub limit: u32,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BooksPage {
    pub data: Vec<BookKey>,
    pub next_after: Option<BookKey>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Subscribe { id: String, query: OrdersQuery },
    Unsubscribe { id: String },
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Status {
        id: String,
        state: String,
    },
    Page {
        id: String,
        page: std::sync::Arc<Page<Order>>,
    },
    Error {
        id: String,
        error: crate::error::ApiError,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct SyncInfo {
    pub ready: bool,
    #[schema(value_type=String)]
    pub state: market_replica::SyncState,
    #[schema(value_type=Option<String>, format=DateTime)]
    pub updated_at: Option<Timestamp>,
}
impl From<market_replica::SyncStatus> for SyncInfo {
    fn from(value: market_replica::SyncStatus) -> Self {
        Self {
            ready: value.ready,
            state: value.state,
            updated_at: value
                .updated_at
                .and_then(|time| Timestamp::from_millisecond(time).ok()),
        }
    }
}
