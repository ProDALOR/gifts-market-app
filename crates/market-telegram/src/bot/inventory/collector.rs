use super::*;
use std::{collections::BTreeSet, time::Duration};
use tokio::time::Instant;

pub(in crate::bot) struct InventoryPage {
    pub gifts: Vec<ProfileGift>,
    pub regular: usize,
    pub skipped: BTreeMap<SkipReason, usize>,
    pub scanned: usize,
    pub total: Option<usize>,
    pub next: Option<String>,
}
pub(in crate::bot) async fn collect<F, Fut>(page: F) -> Result<VisibleInventory, InventoryError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<InventoryPage, InventoryError>>,
{
    collect_until(page, Instant::now() + Duration::from_secs(30)).await
}
async fn collect_until<F, Fut>(
    mut page: F,
    deadline: Instant,
) -> Result<VisibleInventory, InventoryError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<InventoryPage, InventoryError>>,
{
    let mut result = VisibleInventory::default();
    let mut offset = String::new();
    let mut offsets = BTreeSet::new();
    let mut gifts = BTreeSet::new();
    for _ in 0..20 {
        let page = match tokio::time::timeout_at(deadline, page(offset)).await {
            Ok(Ok(page)) => page,
            Ok(Err(InventoryError::Authorization)) => return Err(InventoryError::Authorization),
            Ok(Err(error)) if result.pages_read == 0 => return Err(error),
            Err(_) if result.pages_read == 0 => return Err(InventoryError::Unavailable),
            failed => {
                result.completion = InventoryCompletion::Partial(match failed {
                    Ok(Err(error)) => PartialReason::ReadFailed(error),
                    _ => PartialReason::TimedOut,
                });
                return Ok(result);
            }
        };
        result.pages_read += 1;
        result.gifts_scanned += page.scanned;
        result.reported_total = page.total.or(result.reported_total);
        tracing::debug!(
            page = result.pages_read,
            received = page.scanned,
            scanned = result.gifts_scanned,
            reported_total = result.reported_total,
            has_next = page.next.is_some(),
            "Telegram gift inventory page read"
        );
        result.regular_gifts += page.regular;
        for (reason, count) in page.skipped {
            *result.skipped.entry(reason).or_default() += count;
        }
        for gift in page.gifts {
            if gifts.insert(gift.id) {
                result.gifts.push(gift);
            }
        }
        let Some(next) = page.next.filter(|next| !next.is_empty()) else {
            result.completion = if result
                .reported_total
                .is_none_or(|total| result.gifts_scanned >= total)
            {
                InventoryCompletion::Complete
            } else {
                InventoryCompletion::Partial(PartialReason::MissingCursor)
            };
            return Ok(result);
        };
        if !offsets.insert(next.clone()) {
            result.completion = InventoryCompletion::Partial(PartialReason::RepeatedCursor);
            return Ok(result);
        }
        if result.gifts_scanned >= 1000 {
            break;
        }
        offset = next;
    }
    result.completion = InventoryCompletion::Partial(PartialReason::LimitReached);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(start: i64, end: i64, next: Option<&str>, total: usize) -> InventoryPage {
        InventoryPage {
            gifts: (start..end)
                .map(|id| ProfileGift {
                    id,
                    collection_id: 1,
                    model_document_id: 2,
                    relation: GiftRelation::Hosted,
                })
                .collect(),
            regular: 0,
            skipped: BTreeMap::new(),
            scanned: (end - start) as usize,
            total: Some(total),
            next: next.map(String::from),
        }
    }
    #[tokio::test]
    async fn follows_opaque_cursors_across_seven_pages_and_deduplicates_gifts() {
        let mut calls = 0;
        let result = collect(|offset| {
            assert_eq!(
                offset,
                if calls == 0 {
                    String::new()
                } else {
                    format!("cursor-{calls}")
                }
            );
            let start = calls * 100;
            calls += 1;
            std::future::ready(Ok(page(
                start,
                start + 100,
                (calls < 7).then(|| format!("cursor-{calls}")).as_deref(),
                700,
            )))
        })
        .await
        .unwrap();
        assert_eq!((calls, result.gifts.len(), result.pages_read), (7, 700, 7));
        assert_eq!(result.completion, InventoryCompletion::Complete);
        let mut calls = 0;
        let result = collect(|_| {
            calls += 1;
            std::future::ready(Ok(page(
                0,
                1,
                if calls == 1 { Some("next") } else { None },
                2,
            )))
        })
        .await
        .unwrap();
        assert_eq!(result.gifts.len(), 1);
        assert_eq!(result.relation_count(GiftRelation::Hosted), 1);
    }
    #[tokio::test]
    async fn later_failure_preserves_progress_but_auth_failure_is_never_hidden() {
        for failure in [
            InventoryError::Unavailable,
            InventoryError::RetryAfter(10),
            InventoryError::Authorization,
        ] {
            let mut first = true;
            let result = collect(|_| {
                let result = if first {
                    Ok(page(1, 2, Some("next"), 700))
                } else {
                    Err(failure.clone())
                };
                first = false;
                std::future::ready(result)
            })
            .await;
            if failure == InventoryError::Authorization {
                assert_eq!(result.unwrap_err(), failure);
            } else {
                let result = result.unwrap();
                assert_eq!(result.gifts.len(), 1);
                assert_eq!(
                    result.completion,
                    InventoryCompletion::Partial(PartialReason::ReadFailed(failure))
                );
            }
        }
    }
    #[tokio::test]
    async fn limits_missing_and_repeated_cursors_are_explicit() {
        let result = collect(|_| std::future::ready(Ok(page(0, 100, None, 700))))
            .await
            .unwrap();
        assert_eq!(
            result.completion,
            InventoryCompletion::Partial(PartialReason::MissingCursor)
        );
        let result = collect(|_| std::future::ready(Ok(page(0, 1, Some("loop"), 700))))
            .await
            .unwrap();
        assert_eq!(
            result.completion,
            InventoryCompletion::Partial(PartialReason::RepeatedCursor)
        );
        let result = collect(|_| std::future::ready(Ok(page(0, 1000, Some("next"), 1001))))
            .await
            .unwrap();
        assert_eq!(
            result.completion,
            InventoryCompletion::Partial(PartialReason::LimitReached)
        );
    }
    #[tokio::test]
    async fn overall_deadline_preserves_already_read_pages() {
        let mut first = true;
        let result = collect_until(
            |_| {
                let ready = first;
                first = false;
                async move {
                    if ready {
                        Ok(page(0, 1, Some("next"), 700))
                    } else {
                        std::future::pending().await
                    }
                }
            },
            Instant::now() + Duration::from_millis(10),
        )
        .await
        .unwrap();
        assert_eq!(result.gifts.len(), 1);
        assert_eq!(
            result.completion,
            InventoryCompletion::Partial(PartialReason::TimedOut)
        );
    }
}
