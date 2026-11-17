//! Gift-market language shared by actors and transports. No hub, storage, HTTP or Telegram client dependencies.
pub mod catalog;
mod ids;
pub mod media;
mod money;
pub mod orders;
pub mod query;
pub mod read;
pub mod valuation;

pub use catalog::*;
pub use ids::*;
pub use media::*;
pub use money::*;
pub use orders::*;
pub use query::*;
pub use read::*;

#[cfg(test)]
mod tests;
