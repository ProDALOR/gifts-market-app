//! TL profile-gift mapping. No market catalog or actor types cross this adapter.
use super::client::rpc;
use super::invocations::CallError;
use super::invocations::{Executor, Group};
use crate::{
    GiftRelation, GiftSource, InventoryError, ProfileGift, SkipReason, VisibleInventory,
    inventory::{InventoryPage, collect},
};
use grammers_client::{Client, tl};
use grammers_session::types::PeerRef;
use market_gifts::{TelegramDocumentId, TelegramGiftId};
use market_invocations::{Error, Invocation, InvocationExt, Options, Priority, Replay};
use std::collections::BTreeMap;
use std::time::Duration;

pub struct TelegramGifts {
    pub(super) client: Client,
    pub(super) peer: PeerRef,
    pub(super) invocations: Executor,
}
impl GiftSource for TelegramGifts {
    async fn inventory(&self) -> Result<VisibleInventory, InventoryError> {
        collect(|offset| async move {
            let operation = self.page_operation(offset, 100)?;
            self.invocations
                .handle
                .submit(
                    operation,
                    Options {
                        priority: Priority::Normal,
                        timeout: Duration::from_secs(30),
                        cancellation: self.invocations.stopped.clone(),
                    },
                )
                .map_err(|_| InventoryError::Unavailable)?
                .await
                .map_err(|e| match e {
                    Error::Operation(error) => error,
                    Error::Runtime(error) => inventory_error(CallError::Runtime(error).bot()),
                })
        })
        .await
    }
}
impl TelegramGifts {
    /// An unsubmitted, replayable page operation. Submit it to the shared invocation handle.
    /// Adapters can map native page data with `InvocationExt` while preserving rate policy.
    pub fn page_operation(
        &self,
        offset: String,
        limit: u8,
    ) -> Result<
        impl Invocation<Output = InventoryPage, Error = InventoryError> + use<>,
        InventoryError,
    > {
        if !(1..=100).contains(&limit) || offset.len() > 4096 {
            return Err(InventoryError::Unavailable);
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
            limit: i32::from(limit),
        };
        let client = self.client.clone();
        let request = std::sync::Arc::new(request);
        let owner = self.peer.id.bare_id();
        Ok(self
            .invocations
            .operation(Group::Gifts, Replay::Safe, move || {
                let client = client.clone();
                let request = request.clone();
                async move { client.invoke(&*request).await }
            })
            .map(
                move |response| map_page(response, owner),
                |error| inventory_error(rpc(error)),
            ))
    }
}
fn inventory_error(error: crate::BotError) -> InventoryError {
    match error {
        crate::BotError::Authorization => InventoryError::Authorization,
        crate::BotError::RetryAfter(seconds) => InventoryError::RetryAfter(seconds),
        crate::BotError::Rpc(400 | 403) | crate::BotError::PeerUnavailable => {
            InventoryError::Forbidden
        }
        _ => InventoryError::Unavailable,
    }
}
fn map_page(response: tl::enums::payments::SavedStarGifts, owner: Option<i64>) -> InventoryPage {
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
        match map_gift(saved, owner) {
            Ok(Some(gift)) => page.gifts.push(gift),
            Ok(None) => page.regular += 1,
            Err(reason) => *page.skipped.entry(reason).or_default() += 1,
        }
    }
    page
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
    let collection_id =
        TelegramGiftId::new(gift.gift_id).map_err(|_| SkipReason::MissingCollection)?;
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
        .and_then(|id| TelegramDocumentId::new(id).ok())
        .ok_or(SkipReason::MissingModel)?;
    Ok(Some(ProfileGift {
        id: gift.id,
        collection_id,
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
    fn saved_gift(collection_id: i64, document_id: i64) -> tl::enums::SavedStarGift {
        let gift = tl::types::StarGiftUnique {
            require_premium: false,
            resale_ton_only: false,
            theme_available: false,
            burned: false,
            crafted: false,
            id: 1,
            gift_id: collection_id,
            title: "Gift".into(),
            slug: "Gift-1".into(),
            num: 1,
            owner_id: None,
            owner_name: None,
            owner_address: None,
            attributes: vec![tl::enums::StarGiftAttribute::Model(
                tl::types::StarGiftAttributeModel {
                    crafted: false,
                    name: "Model".into(),
                    document: tl::enums::Document::Empty(tl::types::DocumentEmpty {
                        id: document_id,
                    }),
                    rarity: tl::enums::StarGiftAttributeRarity::Rare,
                },
            )],
            availability_issued: 1,
            availability_total: 1,
            gift_address: None,
            resell_amount: None,
            released_by: None,
            value_amount: None,
            value_currency: None,
            value_usd_amount: None,
            theme_peer: None,
            peer_color: None,
            host_id: None,
            offer_min_stars: None,
            craft_chance_permille: None,
        };
        tl::enums::SavedStarGift::Gift(tl::types::SavedStarGift {
            name_hidden: false,
            unsaved: false,
            refunded: false,
            can_upgrade: false,
            pinned_to_top: false,
            upgrade_separate: false,
            from_id: None,
            date: 0,
            gift: tl::enums::StarGift::Unique(gift),
            message: None,
            msg_id: None,
            saved_id: None,
            convert_stars: None,
            upgrade_stars: None,
            can_export_at: None,
            transfer_stars: None,
            can_transfer_at: None,
            can_resell_at: None,
            collection_id: None,
            prepaid_upgrade_hash: None,
            drop_original_details_stars: None,
            gift_num: None,
            can_craft_at: None,
        })
    }
    #[test]
    fn profile_mapping_validates_source_ids_before_inventory_and_preserves_all_64_bits() {
        let id = 9_007_199_254_740_993;
        let gift = map_gift(saved_gift(id, id + 1), Some(7)).unwrap().unwrap();
        assert_eq!(gift.collection_id.get(), id);
        assert_eq!(gift.model_document_id.get(), id + 1);
        assert_eq!(gift.relation, GiftRelation::Unverified);
        for invalid in [0, -1, i64::MIN] {
            assert_eq!(
                map_gift(saved_gift(invalid, 1), Some(7)),
                Err(SkipReason::MissingCollection)
            );
            assert_eq!(
                map_gift(saved_gift(1, invalid), Some(7)),
                Err(SkipReason::MissingModel)
            );
        }
    }
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
