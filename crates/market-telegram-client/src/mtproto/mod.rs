//! Private grammers adapter: TL calls, payloads, session storage and gift mapping.
mod client;
mod connection;
mod gifts;
mod invocations;
mod media;
mod requests;
mod session_store;
pub use connection::Connection;
pub use gifts::TelegramGifts;
#[cfg(test)]
mod tests;
