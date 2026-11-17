//! Interpretation of generic hub facts as gifts. No actors, storage, HTTP or clients.
mod catalog;
mod lookup;
mod orders;
pub mod query;
pub use catalog::*;
pub use lookup::{find_collection, find_model, model_identity};
pub use orders::{FloorQuote, book, concrete_model, floor_quote, order};
#[derive(Debug)]
pub struct ProjectionError(String);
impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ProjectionError {}
type Result<T> = std::result::Result<T, ProjectionError>;
fn invalid(value: impl std::fmt::Display) -> ProjectionError {
    ProjectionError(value.to_string())
}
pub const MODEL: &str = "tg.model";
pub const BACKDROP: &str = "tg.backdrop";
pub const PATTERN: &str = "tg.pattern";
pub const NUMBER: &str = "tg.number";
const TELEGRAM_NAMESPACE: &str = "tg";
#[cfg(test)]
mod tests;

#[cfg(test)]
mod order_tests;
