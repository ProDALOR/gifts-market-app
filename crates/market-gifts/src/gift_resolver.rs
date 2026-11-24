//! Optional gift facts supplied by a transport; prices still come from market data.
use crate::{GiftId, GiftNumber, GiftSlug, TelegramDocumentId, TelegramGiftId};
use std::{future::Future, pin::Pin};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct TelegramGiftDetails {
    pub id: GiftId,
    pub slug: GiftSlug,
    pub collection: TelegramGiftId,
    pub number: GiftNumber,
    pub model: TelegramDocumentId,
    pub backdrop: Option<i32>,
    pub pattern: Option<TelegramDocumentId>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiftResolutionError {
    NotFound,
    Unavailable,
    Authorization,
    InvalidData,
    RateLimited(u64),
}
pub trait GiftResolver: Send + Sync {
    fn resolve<'a>(
        &'a self,
        slug: &'a GiftSlug,
    ) -> Pin<Box<dyn Future<Output = Result<TelegramGiftDetails, GiftResolutionError>> + Send + 'a>>;
}
