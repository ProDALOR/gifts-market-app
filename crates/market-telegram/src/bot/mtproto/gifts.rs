//! TL profile-gift mapping. No market catalog or actor types cross this adapter.
use super::{client::rpc, cooldown::Cooldown};
use crate::bot::{
    GiftRelation, GiftSource, InventoryError, ProfileGift, SkipReason, VisibleInventory,
    inventory::{InventoryPage, collect},
};
use grammers_client::{Client, tl};
use grammers_session::types::PeerRef;
use std::{collections::BTreeMap, time::Duration};

pub(in crate::bot) struct TelegramGifts {
    pub client: Client,
    pub peer: PeerRef,
    pub cooldown: Cooldown,
}
impl GiftSource for TelegramGifts {
    async fn inventory(&self) -> Result<VisibleInventory, InventoryError> {
        if let Some(seconds) = self.cooldown.remaining() {
            return Err(InventoryError::RetryAfter(seconds));
        }
        collect(|offset| self.page(offset)).await
    }
}
impl TelegramGifts {
    async fn page(&self, offset: String) -> Result<InventoryPage, InventoryError> {
        if let Some(seconds) = self.cooldown.remaining() {
            return Err(InventoryError::RetryAfter(seconds));
        }
        let request = tl::functions::payments::GetSavedStarGifts {
            exclude_unsaved: false,
            exclude_saved: false,
            exclude_unlimited: false,
            exclude_unique: false,
            sort_by_value: false,
            exclude_upgradable: false,
            exclude_unupgradable: false,
            peer_color_available: false,
            exclude_hosted: false,
            peer: self.peer.into(),
            collection_id: None,
            offset,
            limit: 100,
        };
        let response = tokio::time::timeout(Duration::from_secs(10), self.client.invoke(&request))
            .await
            .map_err(|_| InventoryError::Unavailable)?
            .map_err(|error| match rpc(error) {
                crate::bot::BotError::Authorization => InventoryError::Authorization,
                crate::bot::BotError::RetryAfter(seconds) => {
                    self.cooldown.observe(seconds);
                    InventoryError::RetryAfter(seconds)
                }
                crate::bot::BotError::Rpc(400 | 403) | crate::bot::BotError::PeerUnavailable => {
                    InventoryError::Forbidden
                }
                _ => InventoryError::Unavailable,
            })?;
        let tl::enums::payments::SavedStarGifts::Gifts(response) = response;
        let mut page = InventoryPage {
            gifts: vec![],
            regular: 0,
            skipped: BTreeMap::new(),
            scanned: response.gifts.len(),
            total: usize::try_from(response.count).ok(),
            next: response.next_offset,
        };
        for saved in response.gifts {
            match map_gift(saved, self.peer.id.bare_id()) {
                Ok(Some(gift)) => page.gifts.push(gift),
                Ok(None) => page.regular += 1,
                Err(reason) => *page.skipped.entry(reason).or_default() += 1,
            }
        }
        Ok(page)
    }
}
fn map_gift(
    saved: tl::enums::SavedStarGift,
    user_id: Option<i64>,
) -> Result<Option<ProfileGift>, SkipReason> {
    let tl::enums::SavedStarGift::Gift(saved) = saved;
    if saved.refunded {
        return Err(SkipReason::Refunded);
    }
    let tl::enums::StarGift::Unique(gift) = saved.gift else {
        return Ok(None);
    };
    if gift.burned {
        return Err(SkipReason::Burned);
    }
    if gift.id == 0 {
        return Err(SkipReason::MissingGiftId);
    }
    if gift.gift_id == 0 {
        return Err(SkipReason::MissingCollection);
    }
    let relation = profile_relation(
        gift.owner_id.as_ref(),
        gift.host_id.as_ref(),
        gift.owner_address.is_some(),
        user_id,
    )?;
    let model = gift
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            tl::enums::StarGiftAttribute::Model(model) => Some(model.document.id()),
            _ => None,
        })
        .filter(|id| *id != 0)
        .ok_or(SkipReason::MissingModel)?;
    Ok(Some(ProfileGift {
        id: gift.id,
        collection_id: gift.gift_id,
        model_document_id: model,
        relation,
    }))
}
fn profile_relation(
    owner: Option<&tl::enums::Peer>,
    host: Option<&tl::enums::Peer>,
    wallet_owner: bool,
    user_id: Option<i64>,
) -> Result<GiftRelation, SkipReason> {
    let matches = |peer: &tl::enums::Peer| matches!(peer,tl::enums::Peer::User(user) if Some(user.user_id)==user_id);
    if owner.is_some_and(matches) {
        Ok(GiftRelation::Owned)
    } else if host.is_some_and(matches) {
        Ok(GiftRelation::Hosted)
    } else if owner.is_none() && host.is_none() && !wallet_owner {
        Ok(GiftRelation::Unverified)
    } else {
        Err(SkipReason::OtherProfile)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_hosting_and_missing_metadata_remain_distinct() {
        let user = |id| tl::enums::Peer::User(tl::types::PeerUser { user_id: id });
        assert_eq!(
            profile_relation(Some(&user(7)), None, false, Some(7)),
            Ok(GiftRelation::Owned)
        );
        assert_eq!(
            profile_relation(None, Some(&user(7)), true, Some(7)),
            Ok(GiftRelation::Hosted)
        );
        assert_eq!(
            profile_relation(Some(&user(8)), Some(&user(7)), true, Some(7)),
            Ok(GiftRelation::Hosted)
        );
        assert_eq!(
            profile_relation(None, None, false, Some(7)),
            Ok(GiftRelation::Unverified)
        );
        assert_eq!(
            profile_relation(None, Some(&user(8)), true, Some(7)),
            Err(SkipReason::OtherProfile)
        );
        assert_eq!(
            profile_relation(Some(&user(8)), None, false, Some(7)),
            Err(SkipReason::OtherProfile)
        );
        assert_eq!(
            profile_relation(None, None, true, Some(7)),
            Err(SkipReason::OtherProfile)
        );
    }
}
