//! Ordered slices and fixed ranges over one pinned active/history view.
use crate::{
    query::Filter,
    state,
    types::{Key, RankKey, Sort},
    *,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use prost::Message;
use rust_decimal::Decimal;
use std::ops::Bound::{Excluded, Unbounded};

#[derive(Clone)]
pub struct WindowRequest {
    pub fill_edges: bool,
    pub anchor: Option<String>,
    pub before: u32,
    pub after: u32,
    pub snapshot: Option<String>,
}
#[derive(Clone)]
pub struct WindowEntity {
    pub entity: w::Entity,
    pub position: String,
    pub sort_key: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRange {
    pub start: Option<String>,
    pub end: Option<String>,
}
pub struct WindowReadResult {
    pub entries: Vec<WindowEntity>,
    pub has_before: bool,
    pub has_after: bool,
    pub sync: SyncStatus,
    pub revision: ReadRevision,
    pub snapshot: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Position {
    selection: String,
    sort: Sort,
    key: Key,
    rank: Decimal,
    #[serde(default)]
    at: Option<i64>,
}
impl Position {
    fn encode(&self) -> Result<String, Status> {
        Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).map_err(storage::failure)?))
    }
    fn sort_key(&self) -> String {
        ordering_key_at(self.rank, &self.key, self.sort, self.at)
    }
    fn history_position(&self) -> Result<types::Position, Status> {
        Ok(types::Position {
            executed_at_ms: self
                .at
                .ok_or_else(|| Status::invalid_argument("missing history position"))?,
            venue_id: self.key.venue_id.clone(),
            order_id: self.key.id.clone(),
        })
    }
    fn decode(value: &str) -> Result<Self, Status> {
        if value.len() > READ_LIMITS.max_position_bytes {
            return Err(Status::invalid_argument("order position too large"));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| Status::invalid_argument("invalid order position"))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| Status::invalid_argument("invalid order position"))
    }
}
pub fn position_sort_key(value: &str) -> Result<String, Status> {
    Ok(Position::decode(value)?.sort_key())
}
fn reverse_sort(sort: Sort) -> Sort {
    match sort {
        Sort::PriceAsc => Sort::PriceDesc,
        Sort::PriceDesc => Sort::PriceAsc,
        Sort::TimeAsc => Sort::TimeDesc,
        Sort::TimeDesc => Sort::TimeAsc,
        Sort::NumberAsc => Sort::NumberDesc,
        Sort::NumberDesc => Sort::NumberAsc,
        Sort::Identity => Sort::Identity,
    }
}
fn entity_rank(entity: &w::Entity, sort: Sort, history: bool) -> Result<Decimal, Status> {
    let order = state::order(entity).ok_or_else(|| Status::data_loss("missing order"))?;
    if sort == Sort::Identity {
        Ok(if history {
            Decimal::from(
                order
                    .executed_at_ms
                    .ok_or_else(|| Status::data_loss("execution time missing"))?,
            )
        } else {
            Decimal::ZERO
        })
    } else {
        sort.rank(order, history)
    }
}
impl ReplicaReader {
    pub fn read_window_checked(
        &self,
        request: w::ReadRequest,
        window: WindowRequest,
        validate: impl FnOnce(&Self, &w::ReadRequest) -> Result<(), Status>,
        include: impl Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<WindowReadResult, Status> {
        self.read_selection_checked(request, window, None, validate, include)
    }
    pub fn read_range_checked(
        &self,
        request: w::ReadRequest,
        range: WindowRange,
        validate: impl FnOnce(&Self, &w::ReadRequest) -> Result<(), Status>,
        include: impl Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<WindowReadResult, Status> {
        let window = WindowRequest {
            fill_edges: false,
            anchor: range.start.clone(),
            before: 0,
            after: READ_LIMITS.max_batch_rows - 1,
            snapshot: None,
        };
        self.read_selection_checked(request, window, Some(range), validate, include)
    }
    fn read_selection_checked(
        &self,
        request: w::ReadRequest,
        window: WindowRequest,
        range: Option<WindowRange>,
        validate: impl FnOnce(&Self, &w::ReadRequest) -> Result<(), Status>,
        include: impl Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<WindowReadResult, Status> {
        let size = window
            .before
            .checked_add(window.after)
            .and_then(|n| n.checked_add(1))
            .filter(|n| {
                *n <= if range.is_some() {
                    READ_LIMITS.max_batch_rows
                } else {
                    READ_LIMITS.max_rows
                }
            })
            .ok_or_else(|| Status::invalid_argument("query exceeds row budget"))?
            as usize;
        if !request.page_token.is_empty() {
            return Err(Status::invalid_argument(
                "slice reads do not use page tokens",
            ));
        }
        self.expire();
        let frozen = match w::ReadMode::try_from(request.mode) {
            Ok(w::ReadMode::Snapshot) => true,
            Ok(w::ReadMode::Live) => false,
            _ => return Err(Status::invalid_argument("unknown read mode")),
        };
        if !frozen && window.snapshot.is_some() {
            return Err(Status::invalid_argument(
                "latest reads do not use snapshot leases",
            ));
        }
        let snapshot = if frozen {
            Some(
                window
                    .snapshot
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            )
        } else {
            None
        };
        let reader = if let Some(id) = &window.snapshot {
            validate_snapshot(id)?;
            let mut views = self.0.views.lock().map_err(storage::failure)?;
            let retained = views
                .get_mut(id)
                .ok_or_else(|| Status::failed_precondition("snapshot expired; restart read"))?;
            retained.until =
                Instant::now() + Duration::from_secs(READ_LIMITS.snapshot_idle_seconds);
            self.pin(retained.view.clone())
        } else {
            self.snapshot()?
        };
        validate(&reader, &request)?;
        let view = reader.view()?;
        let filter = Filter::compile(&view.state, &request)?;
        if filter.scope.is_none() {
            return Err(Status::invalid_argument("reads require an order scope"));
        }
        let mut position = window.anchor.as_deref().map(Position::decode).transpose()?;
        if let Some(p) = &position
            && (p.selection != filter.fingerprint
                || p.sort != filter.sort
                || p.key.kind != types::EntityKind::Order)
        {
            return Err(Status::invalid_argument(
                "order position belongs to another query or sort",
            ));
        }
        let end = range
            .as_ref()
            .and_then(|r| r.end.as_deref())
            .map(Position::decode)
            .transpose()?;
        for p in position.iter().chain(end.iter()) {
            if p.selection != filter.fingerprint
                || p.sort != filter.sort
                || p.key.kind != types::EntityKind::Order
                || filter.history() != p.at.is_some()
            {
                return Err(Status::invalid_argument(
                    "range boundary belongs to another query",
                ));
            }
        }
        let end_key = end.as_ref().map(Position::sort_key);
        if let (Some(start), Some(end_key)) = (&position, &end_key)
            && start.sort_key() > *end_key
        {
            return Err(Status::invalid_argument("inverted order range"));
        }
        let eligible = |e: &w::Entity| -> Result<bool, Status> {
            reader.check_cancelled()?;
            Ok(filter.matches(e)? && include(e)?)
        };
        let mut pivot = None;
        if let Some(p) = &mut position {
            let existing = if filter.history() {
                view.history
                    .execution(&p.history_position()?)?
                    .map(Arc::new)
            } else {
                view.state.books().get(&p.key).cloned()
            };
            if let Some(e) = existing
                && eligible(&e)?
            {
                let rank = entity_rank(&e, filter.sort, filter.history())?;
                if range.is_none() || rank == p.rank {
                    p.rank = rank;
                    if end_key.as_ref().is_none_or(|end| p.sort_key() <= *end) {
                        pivot = Some(e);
                    }
                }
            }
        }
        let scan = |reverse: bool| -> Result<Vec<(Key, Decimal, Arc<w::Entity>)>, Status> {
            let boundary = position.as_ref();
            if filter.history() {
                let mut scan_filter = filter.clone();
                if scan_filter.sort == Sort::Identity {
                    scan_filter.sort = Sort::TimeAsc;
                }
                if reverse {
                    scan_filter.sort = reverse_sort(scan_filter.sort);
                }
                let after = boundary
                    .map(|p| Ok::<_, Status>((p.rank.to_string(), p.history_position()?)))
                    .transpose()?;
                let end = if reverse {
                    None
                } else {
                    end.as_ref()
                        .map(|p| Ok::<_, Status>((p.rank.to_string(), p.history_position()?)))
                        .transpose()?
                };
                let (rows, next) = view.history.ranked_history_between(
                    &scan_filter,
                    after,
                    end,
                    size + 1,
                    &eligible,
                )?;
                if next.is_some() && rows.len() < size + 1 {
                    return Err(Status::resource_exhausted(
                        "history range exceeds scan or byte budget",
                    ));
                }
                return rows
                    .into_iter()
                    .map(|entity| {
                        let key = state::entity_key(&entity)?;
                        let rank = entity_rank(&entity, filter.sort, true)?;
                        Ok((key, rank, Arc::new(entity)))
                    })
                    .collect();
            }
            let rows: Box<dyn Iterator<Item = (&Key, Decimal, &Arc<w::Entity>)> + '_> =
                if filter.sort == Sort::Identity {
                    let low = if reverse {
                        Unbounded
                    } else {
                        boundary
                            .map(|p| Excluded(p.key.clone()))
                            .unwrap_or(Unbounded)
                    };
                    let high = if reverse {
                        boundary
                            .map(|p| Excluded(p.key.clone()))
                            .unwrap_or(Unbounded)
                    } else {
                        Unbounded
                    };
                    let rows = view.state.books().range((low, high));
                    if reverse {
                        Box::new(rows.rev().map(|(k, e)| (k, Decimal::ZERO, e)))
                    } else {
                        Box::new(rows.map(|(k, e)| (k, Decimal::ZERO, e)))
                    }
                } else {
                    let sort = if reverse {
                        match filter.sort {
                            Sort::PriceAsc => Sort::PriceDesc,
                            Sort::PriceDesc => Sort::PriceAsc,
                            Sort::TimeAsc => Sort::TimeDesc,
                            Sort::TimeDesc => Sort::TimeAsc,
                            Sort::NumberAsc => Sort::NumberDesc,
                            Sort::NumberDesc => Sort::NumberAsc,
                            Sort::Identity => unreachable!(),
                        }
                    } else {
                        filter.sort
                    };
                    Box::new(
                        view.state
                            .books()
                            .ranked(
                                filter.scope.as_ref().expect("scope checked"),
                                boundary.map(|p| RankKey::new(p.rank, p.key.clone())),
                                sort,
                            )
                            .map(|(r, e)| {
                                let (v, k) = r.entity();
                                (k, v, e)
                            }),
                    )
                };
            let mut result = Vec::new();
            for (scanned, (key, rank, e)) in rows.enumerate() {
                reader.check_cancelled()?;
                if !reverse
                    && end_key
                        .as_ref()
                        .is_some_and(|end| ordering_key(rank, key, filter.sort) > *end)
                {
                    break;
                }
                if scanned >= READ_LIMITS.max_scan_candidates {
                    return Err(Status::resource_exhausted(
                        "query scan budget exceeded; narrow the filters",
                    ));
                }
                if eligible(e)? {
                    result.push((key.clone(), rank, e.clone()));
                    if result.len() > size {
                        break;
                    }
                }
            }
            Ok(result)
        };
        let mut before = if position.is_some() && range.is_none() {
            scan(true)?
        } else {
            vec![]
        };
        let mut after = scan(false)?;
        let pivot = if let (Some(e), Some(p)) = (pivot, position.as_ref()) {
            Some((p.key.clone(), p.rank, e))
        } else if !after.is_empty() {
            Some(after.remove(0))
        } else if !before.is_empty() {
            Some(before.remove(0))
        } else {
            None
        };
        let before_count = (window.before as usize).min(before.len());
        let after_count = (if window.fill_edges {
            size - 1 - before_count
        } else {
            window.after as usize
        })
        .min(after.len());
        let before_count = (if window.fill_edges {
            size - 1 - after_count
        } else {
            window.before as usize
        })
        .min(before.len());
        let has_before = before.len() > before_count;
        let has_after = after.len() > after_count;
        let selected = before
            .into_iter()
            .take(before_count)
            .rev()
            .chain(pivot)
            .chain(after.into_iter().take(after_count))
            .collect::<Vec<_>>();
        let mut bytes = 0;
        let mut entries = Vec::with_capacity(selected.len());
        for (key, rank, entity) in selected {
            let at = if filter.history() {
                state::order(&entity).and_then(|o| o.executed_at_ms)
            } else {
                None
            };
            let sort_key = ordering_key_at(rank, &key, filter.sort, at);
            let position = Position {
                selection: filter.fingerprint.clone(),
                sort: filter.sort,
                key,
                rank,
                at,
            }
            .encode()?;
            bytes += entity.encoded_len() + position.len() + sort_key.len() + 16;
            if bytes > READ_LIMITS.max_read_bytes {
                return Err(Status::resource_exhausted(
                    "query exceeds byte budget; reduce its size",
                ));
            }
            entries.push(WindowEntity {
                entity: entity.as_ref().clone(),
                position,
                sort_key,
            });
        }
        if range.is_some() && has_after {
            return Err(Status::resource_exhausted(
                "watched range exceeds row budget; narrow its bounds",
            ));
        }
        if let Some(id) = &snapshot {
            let mut views = self.0.views.lock().map_err(storage::failure)?;
            self.check_cancelled()?;
            if window.snapshot.is_some() {
                let retained = views.get_mut(id).ok_or_else(|| {
                    Status::failed_precondition("snapshot released; restart read")
                })?;
                retained.until =
                    Instant::now() + Duration::from_secs(READ_LIMITS.snapshot_idle_seconds);
            } else {
                if views.len() >= READ_LIMITS.max_retained_snapshots {
                    return Err(Status::resource_exhausted("snapshot capacity reached"));
                }
                views.insert(
                    id.clone(),
                    RetainedView {
                        view: view.clone(),
                        until: Instant::now()
                            + Duration::from_secs(READ_LIMITS.snapshot_idle_seconds),
                    },
                );
            }
        }
        Ok(WindowReadResult {
            entries,
            has_before,
            has_after,
            sync: reader.status(),
            revision: reader.read_revision(&request)?,
            snapshot,
        })
    }
    pub fn release_snapshot(&self, id: &str) -> Result<(), Status> {
        self.check_cancelled()?;
        validate_snapshot(id)?;
        self.0.views.lock().map_err(storage::failure)?.remove(id);
        Ok(())
    }
}

fn validate_snapshot(id: &str) -> Result<(), Status> {
    if id.is_empty()
        || id.len() > READ_LIMITS.max_snapshot_token_bytes
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(Status::invalid_argument("invalid snapshot token"));
    }
    Ok(())
}

/// ASCII ordering shared with clients; no rank/token decoding or domain sorting is required.
pub(crate) fn ordering_key(rank: Decimal, key: &Key, sort: Sort) -> String {
    ordering_key_at(rank, key, sort, None)
}
fn ordering_key_at(rank: Decimal, key: &Key, sort: Sort, at: Option<i64>) -> String {
    let mut bytes =
        crate::ranking::ordered_decimal(if rank.is_zero() { Decimal::ZERO } else { rank });
    if let Some(at) = at {
        bytes.extend_from_slice(&((at as u64) ^ (1 << 63)).to_be_bytes());
    }
    for text in [&key.venue_id, &key.id] {
        for byte in text.bytes() {
            bytes.push(byte);
            if byte == 0 {
                bytes.push(255);
            }
        }
        bytes.extend([0, 0]);
    }
    if sort.descending() {
        for byte in &mut bytes {
            *byte = 255 - *byte;
        }
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes
        .into_iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 15) as usize] as char,
            ]
        })
        .collect()
}
