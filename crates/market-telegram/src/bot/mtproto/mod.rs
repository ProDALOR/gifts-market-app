//! Private grammers adapter: TL calls, payloads, session storage and gift mapping.
mod client;
mod connection;
mod cooldown;
mod gifts;
mod requests;
mod session_store;
pub(super) use connection::Connection;
pub(super) use cooldown::RateLimits;
#[cfg(test)]
mod tests;
