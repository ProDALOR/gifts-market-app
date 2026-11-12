//! Schemas for the application-owned JSON representation; domain types have no UI dependencies.
use std::{collections::BTreeMap, sync::OnceLock};
use utoipa::{
    PartialSchema, ToSchema,
    openapi::{RefOr, schema::Schema},
};
fn definitions() -> &'static BTreeMap<String, RefOr<Schema>> {
    static SCHEMAS: OnceLock<BTreeMap<String, RefOr<Schema>>> = OnceLock::new();
    SCHEMAS.get_or_init(|| {
        serde_json::from_str(include_str!("schemas.json")).expect("validated public schemas")
    })
}
macro_rules! model {($($name:ident),+)=>{$(
    pub struct $name;
    impl PartialSchema for $name {fn schema()->RefOr<Schema> {definitions()[stringify!($name)].clone()}}
    impl ToSchema for $name {fn schemas(output:&mut Vec<(String,RefOr<Schema>)>) {for (name,schema) in definitions() {if !output.iter().any(|(key,_)|key==name) {output.push((name.clone(),schema.clone()));}}}}
)+};}
model!(
    Item,
    Order,
    FeedStatus,
    CatalogEntry,
    Record,
    OrderSide,
    Trade
);
