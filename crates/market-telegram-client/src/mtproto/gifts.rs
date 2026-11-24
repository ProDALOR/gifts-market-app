//! One-page Telegram inventory operations. Pagination and caching belong to the portfolio actor.
use super::{
    client::rpc,
    invocations::{Executor, Group, policy},
    session_store::DurableSession,
};
use crate::BotCredentials;
use grammers_client::{Client, tl};
use grammers_session::{Session, types::PeerId};
use market_gifts::inventory::*;
use market_gifts::{TelegramDocumentId, TelegramGiftId};
use market_invocations::{AttemptError, Invocation, Policy, Replay, Retry};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub(super) struct Live {
    pub client: Client,
    pub session: Arc<DurableSession>,
    pub executor: Executor,
}
/// Stable capability for the authenticated bot's cached peers, refreshed after every login.
/// Callers must authorize the subject before requesting pages; this does not resolve usernames.
#[derive(Clone)]
pub struct InventoryClient {
    account: u64,
    live: Arc<watch::Sender<Option<Arc<Live>>>>,
}
impl InventoryClient {
    pub fn new(credentials: &BotCredentials) -> Self {
        Self {
            account: credentials.bot_id(),
            live: Arc::new(watch::channel(None).0),
        }
    }
    pub(super) fn bind(&self, account: u64) -> Result<(), crate::BotError> {
        if self.account == account {
            Ok(())
        } else {
            Err(crate::BotError::Configuration(
                "Inventory client belongs to another bot",
            ))
        }
    }
    pub(super) fn publish(&self, live: Arc<Live>) {
        self.live.send_replace(Some(live));
    }
    pub(super) fn clear(&self, live: &Arc<Live>) {
        self.live.send_if_modified(|current| {
            if current
                .as_ref()
                .is_some_and(|value| Arc::ptr_eq(value, live))
            {
                *current = None;
                true
            } else {
                false
            }
        });
    }
    pub fn page_operation(
        &self,
        user: u64,
        offset: String,
        limit: usize,
    ) -> impl Invocation<Output = InventoryPage, Error = InventoryError> + use<> {
        Page {
            live: self.live.borrow().clone(),
            user,
            offset,
            limit,
            policy: policy(self.account, Group::Gifts(user), Replay::Safe),
        }
    }
}
struct Page {
    live: Option<Arc<Live>>,
    user: u64,
    offset: String,
    limit: usize,
    policy: Policy,
}
impl Invocation for Page {
    type Output = InventoryPage;
    type Error = InventoryError;
    fn policy(&self) -> Policy {
        self.policy.clone()
    }
    fn cancellation(&self) -> Option<CancellationToken> {
        self.live.as_ref().map(|live| live.executor.stopped.clone())
    }
    async fn execute(&self) -> Result<InventoryPage, AttemptError<InventoryError>> {
        let permanent = |error| AttemptError {
            error,
            retry: Retry::Never,
        };
        if self.limit == 0
            || self.limit > 100
            || self.offset.len() > 4096
            || self.user == 0
            || self.user > i64::MAX as u64
        {
            return Err(permanent(InventoryError::InvalidData(
                "invalid inventory page request".into(),
            )));
        }
        let live = self
            .live
            .as_ref()
            .ok_or_else(|| permanent(InventoryError::Unavailable))?;
        let user = PeerId::user(self.user as i64).ok_or_else(|| {
            permanent(InventoryError::InvalidData(
                "invalid Telegram user ID".into(),
            ))
        })?;
        let peer = live
            .session
            .peer_ref(user)
            .await
            .map_err(|_| permanent(InventoryError::Unavailable))?
            .ok_or_else(|| permanent(InventoryError::Forbidden))?;
        let client = live.client.clone();
        let offset = self.offset.clone();
        let limit = self.limit as i32;
        let response = live
            .executor
            .operation(Group::Gifts(self.user), Replay::Safe, move || {
                let client = client.clone();
                let offset = offset.clone();
                async move {
                    client
                        .invoke(&tl::functions::payments::GetSavedStarGifts {
                            exclude_unsaved: false,
                            exclude_saved: false,
                            exclude_unlimited: false,
                            exclude_unique: false,
                            sort_by_value: false,
                            exclude_upgradable: false,
                            exclude_unupgradable: false,
                            peer_color_available: false,
                            exclude_hosted: false,
                            peer: peer.into(),
                            collection_id: None,
                            offset,
                            limit,
                        })
                        .await
                }
            })
            .execute()
            .await
            .map_err(|failure| AttemptError {
                retry: failure.retry,
                error: match rpc(failure.error) {
                    crate::BotError::Authorization => InventoryError::Authorization,
                    crate::BotError::RetryAfter(seconds) => {
                        InventoryError::RetryAfter(std::time::Duration::from_secs(seconds))
                    }
                    crate::BotError::Rpc(400 | 403) | crate::BotError::PeerUnavailable => {
                        InventoryError::Forbidden
                    }
                    _ => InventoryError::Unavailable,
                },
            })?;
        let tl::enums::payments::SavedStarGifts::Gifts(response) = response;
        let mut page = InventoryPage {
            gifts: vec![],
            regular_gifts: 0,
            skipped: BTreeMap::new(),
            scanned: response.gifts.len(),
            reported_total: usize::try_from(response.count).ok(),
            next_cursor: response.next_offset,
        };
        for gift in response.gifts {
            match map_gift(gift, Some(self.user as i64)) {
                Ok(Some(gift)) => page.gifts.push(gift),
                Ok(None) => page.regular_gifts += 1,
                Err(reason) => *page.skipped.entry(reason).or_default() += 1,
            }
        }
        Ok(page)
    }
}
fn map_gift(
    saved: tl::enums::SavedStarGift,
    user_id: Option<i64>,
) -> Result<Option<InventoryGift>, SkipReason> {
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
    Ok(Some(InventoryGift {
        id: gift
            .id
            .to_string()
            .try_into()
            .map_err(|_| SkipReason::MissingGiftId)?,
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
pub(crate) mod tests {
    use super::*;
    pub(crate) fn saved_gift(collection_id: i64, document_id: i64) -> tl::enums::SavedStarGift {
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

#[cfg(test)]
mod connection_tests {
    use super::*;
    use grammers_client::sender::SenderPool;
    use tokio_util::task::TaskTracker;
    #[tokio::test]
    async fn reconnect_replaces_the_inventory_capability_and_old_cleanup_cannot_remove_it() {
        let directory = tempfile::tempdir().unwrap();
        let jobs = TaskTracker::new();
        let store =
            super::super::session_store::SessionStore::open(directory.path(), 42, 12345, &jobs)
                .await
                .unwrap();
        let runtime = market_invocations::Runtime::start(Default::default()).unwrap();
        let credentials = crate::tests::credentials();
        let inventory = InventoryClient::new(&credentials);
        let live = || {
            let pool = SenderPool::new(store.session.clone(), 12345);
            Arc::new(Live {
                client: Client::new(pool.handle),
                session: store.session.clone(),
                executor: Executor {
                    handle: runtime.handle(),
                    account: 42,
                    stopped: CancellationToken::new(),
                    authorization: Default::default(),
                },
            })
        };
        assert!(matches!(
            inventory
                .page_operation(7, String::new(), 100)
                .execute()
                .await
                .unwrap_err()
                .error,
            InventoryError::Unavailable
        ));
        let first = live();
        inventory.publish(first.clone());
        let old = inventory.page_operation(7, String::new(), 100);
        let second = live();
        inventory.publish(second.clone());
        first.executor.stopped.cancel();
        inventory.clear(&first);
        assert!(old.cancellation().unwrap().is_cancelled());
        assert!(Arc::ptr_eq(
            inventory.live.borrow().as_ref().unwrap(),
            &second
        ));
        let page = inventory.page_operation(7, String::new(), 100);
        // Unknown subjects fail locally; no peer resolution or RPC is performed.
        assert_eq!(
            page.execute().await.unwrap_err().error,
            InventoryError::Forbidden
        );
        assert_eq!(
            inventory
                .page_operation(0, String::new(), 100)
                .execute()
                .await
                .unwrap_err()
                .error,
            InventoryError::InvalidData("invalid inventory page request".into())
        );
        assert_eq!(
            inventory
                .page_operation(i64::MAX as u64, String::new(), 100)
                .execute()
                .await
                .unwrap_err()
                .error,
            InventoryError::InvalidData("invalid Telegram user ID".into())
        );
        inventory.clear(&second);
        assert!(inventory.live.borrow().is_none());
        drop(page);
        drop(old);
        drop(first);
        drop(second);
        runtime.shutdown().await;
        jobs.close();
        jobs.wait().await;
    }
}
