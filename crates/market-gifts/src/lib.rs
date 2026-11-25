//! Gift-market language shared by actors and transports. No hub, storage, HTTP or Telegram client dependencies.
mod alias;
pub use alias::*;
pub mod catalog;
pub mod gift;
mod gift_resolver;
pub use gift_resolver::{GiftResolutionError, GiftResolver, TelegramGiftDetails};
mod ids;
pub mod inventory;
mod lookup;
pub mod media;
mod money;
pub mod orders;
pub mod query;
pub mod read;
pub use lookup::*;

pub use catalog::*;
pub use gift::*;
pub use ids::*;
pub use media::*;
pub use money::*;
pub use orders::*;
pub use query::*;
pub use read::*;

#[cfg(test)]
mod tests;
