//! Public market navigation, independent of HTTP, Telegram transport and replica state.
mod codec;
mod selection;
mod target;
pub use codec::{LAUNCH_LIMIT, MAX_SELECTION_BYTES};
pub use selection::MarketSelection;
pub use target::{PortfolioLink, ShareDestination, Target};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    Invalid(&'static str),
    TooLarge,
}
impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid(reason) => reason,
            Self::TooLarge => "Market selection is too large",
        })
    }
}
impl std::error::Error for LinkError {}
pub type Result<T> = std::result::Result<T, LinkError>;
