//! Ordered slices and fixed ranges over one pinned active/history view.
use crate::read_key::{Edge, QueryFingerprint};
use crate::{
    query::Filter,
    state,
    types::{Key, RankKey, Sort},
    *,
};
use prost::Message;
use rust_decimal::Decimal;
use std::ops::Bound::{Excluded, Unbounded};

#[derive(Clone)]
pub struct SliceRequest {
    pub navigation: Navigation,
    pub limit: u32,
    pub snapshot: Option<SnapshotId>,
}
#[derive(Clone)]
pub enum Navigation {
    Head,
    After(ReadPosition),
    Before(ReadPosition),
    Around(ReadPosition),
}
#[derive(Clone)]
pub struct ReadEntry {
    pub entity: Arc<w::Entity>,
    pub key: ReadPosition,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyRange {
    pub start: Option<ReadPosition>,
    pub end: Option<ReadPosition>,
}
pub struct ReadBatch {
    pub entries: Vec<ReadEntry>,
    pub range: KeyRange,
    pub next: Option<ReadPosition>,
    pub revision: ReadRevision,
    pub snapshot: Option<SnapshotId>,
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
    pub fn read_slice_checked(
        &self,
        request: w::ReadRequest,
        slice: SliceRequest,
        validate: impl FnOnce(&Self, &w::ReadRequest) -> Result<(), Status>,
        include: impl Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<ReadBatch, Status> {
        self.read_selection_checked(request, slice, None, validate, include)
    }
    pub fn read_range_checked(
        &self,
        request: w::ReadRequest,
        range: KeyRange,
        validate: impl FnOnce(&Self, &w::ReadRequest) -> Result<(), Status>,
        include: impl Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<ReadBatch, Status> {
        let slice = SliceRequest {
            navigation: Navigation::Head,
            limit: READ_LIMITS.max_batch_rows,
            snapshot: None,
        };
        self.read_selection_checked(request, slice, Some(range), validate, include)
    }
    fn read_selection_checked(
        &self,
        request: w::ReadRequest,
        slice: SliceRequest,
        range: Option<KeyRange>,
        validate: impl FnOnce(&Self, &w::ReadRequest) -> Result<(), Status>,
        include: impl Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<ReadBatch, Status> {
        let size = slice.limit as usize;
        if size == 0
            || size
                > if range.is_some() {
                    READ_LIMITS.max_batch_rows
                } else {
                    READ_LIMITS.max_rows
                } as usize
        {
            return Err(Status::invalid_argument("query exceeds row budget"));
        }
        let around = matches!(slice.navigation, Navigation::Around(_));
        let backwards = matches!(slice.navigation, Navigation::Before(_));
        let before_target = if around {
            size / 3
        } else if backwards {
            size
        } else {
            0
        };
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
        if !frozen && slice.snapshot.is_some() {
            return Err(Status::invalid_argument(
                "latest reads do not use snapshot leases",
            ));
        }
        let snapshot = if frozen {
            Some(slice.snapshot.unwrap_or_else(SnapshotId::fresh))
        } else {
            None
        };
        let reader = if let Some(id) = &slice.snapshot {
            let mut views = self.0.views.lock().map_err(storage::failure)?;
            let retained = views
                .get_mut(&id.to_string())
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
        let anchor = match &slice.navigation {
            Navigation::Head => None,
            Navigation::After(p) | Navigation::Before(p) | Navigation::Around(p) => Some(p.clone()),
        };
        let mut position = range.as_ref().map(|r| r.start.clone()).unwrap_or(anchor);
        let end = range.as_ref().and_then(|r| r.end.clone());
        for p in position.iter().chain(end.iter()) {
            p.validate(&filter)?;
        }
        if position
            .as_ref()
            .zip(end.as_ref())
            .is_some_and(|(a, b)| a > b)
        {
            return Err(Status::invalid_argument("inverted order range"));
        }
        let fingerprint = QueryFingerprint::parse(&filter.fingerprint)?;
        let make_position = |key: Key, rank: Decimal, at: Option<i64>| ReadPosition {
            selection: fingerprint,
            sort: filter.sort,
            key,
            rank,
            at,
            edge: Edge::At,
        };
        let eligible = |e: &w::Entity| -> Result<bool, Status> {
            reader.check_cancelled()?;
            if let Some(range) = &range {
                let point = make_position(
                    state::entity_key(e)?,
                    entity_rank(e, filter.sort, filter.history())?,
                    if filter.history() {
                        state::order(e).and_then(|o| o.executed_at_ms)
                    } else {
                        None
                    },
                );
                if range.start.as_ref().is_some_and(|start| point < *start)
                    || range.end.as_ref().is_some_and(|end| point > *end)
                {
                    return Ok(false);
                }
            }
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
                if around || rank == p.rank {
                    p.rank = rank;
                    if (around
                        || (range.is_some() && p.edge != Edge::After)
                        || (matches!(slice.navigation, Navigation::After(_))
                            && p.edge == Edge::Before)
                        || (backwards && p.edge == Edge::After))
                        && end
                            .as_ref()
                            .is_none_or(|end| make_position(p.key.clone(), p.rank, p.at) <= *end)
                    {
                        pivot = Some(e);
                    }
                }
            }
        }
        let directional = range.is_none() && !around;
        let mut resume = None;
        let mut scan = |reverse: bool| -> Result<Vec<(Key, Decimal, Arc<w::Entity>)>, Status> {
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
                if let Some((rank, at)) = next.filter(|_| rows.len() < size + 1) {
                    if directional {
                        resume = Some(make_position(
                            Key::order(at.venue_id.clone(), at.order_id.clone()),
                            rank.parse().map_err(storage::failure)?,
                            Some(at.executed_at_ms),
                        ));
                    } else {
                        return Err(Status::resource_exhausted(
                            "history range exceeds scan or byte budget",
                        ));
                    }
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
            let mut scanned_key = None;
            for (scanned, (key, rank, e)) in rows.enumerate() {
                reader.check_cancelled()?;
                if !reverse
                    && end
                        .as_ref()
                        .is_some_and(|end| make_position(key.clone(), rank, None) > *end)
                {
                    break;
                }
                if scanned >= READ_LIMITS.max_scan_candidates {
                    if directional {
                        resume = scanned_key;
                        break;
                    }
                    return Err(Status::resource_exhausted(
                        "query scan budget exceeded; narrow the filters",
                    ));
                }
                if directional && scanned + 1 == READ_LIMITS.max_scan_candidates {
                    scanned_key = Some(make_position(key.clone(), rank, None));
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
        let mut before = if position.is_some() && (around || backwards) && range.is_none() {
            scan(true)?
        } else {
            vec![]
        };
        let mut after = if backwards { vec![] } else { scan(false)? };
        // Around follows the current identity; directional reads stay strictly beyond the observed key.
        let pivot = if let (Some(e), Some(p)) = (pivot, position.as_ref()) {
            Some((p.key.clone(), p.rank, e))
        } else {
            None
        };
        let before_count =
            (before_target - usize::from(backwards && pivot.is_some())).min(before.len());
        let after_count = (if around {
            size - before_count - usize::from(pivot.is_some())
        } else if backwards {
            0
        } else {
            size - usize::from(pivot.is_some())
        })
        .min(after.len());
        let before_count = if around {
            (size - after_count - usize::from(pivot.is_some())).min(before.len())
        } else {
            before_count
        };
        let has_before = before.len() > before_count || (backwards && resume.is_some());
        let has_after = after.len() > after_count || (!backwards && resume.is_some());
        let selected = before
            .drain(..before_count)
            .rev()
            .chain(pivot)
            .chain(after.drain(..after_count))
            .collect::<Vec<_>>();
        let mut bytes = 0;
        let mut entries = Vec::with_capacity(selected.len());
        for (key, rank, entity) in selected {
            let at = if filter.history() {
                state::order(&entity).and_then(|o| o.executed_at_ms)
            } else {
                None
            };
            let key = make_position(key, rank, at);
            bytes += entity.encoded_len() + key.encoded_len() + 16;
            if bytes > READ_LIMITS.max_read_bytes {
                return Err(Status::resource_exhausted(
                    "query exceeds byte budget; reduce its size",
                ));
            }
            entries.push(ReadEntry { entity, key });
        }
        if range.is_some() && has_after {
            return Err(Status::resource_exhausted(
                "watched range exceeds row budget; narrow its bounds",
            ));
        }
        if let Some(id) = &snapshot {
            let mut views = self.0.views.lock().map_err(storage::failure)?;
            self.check_cancelled()?;
            if slice.snapshot.is_some() {
                let retained = views.get_mut(&id.to_string()).ok_or_else(|| {
                    Status::failed_precondition("snapshot released; restart read")
                })?;
                retained.until =
                    Instant::now() + Duration::from_secs(READ_LIMITS.snapshot_idle_seconds);
            } else {
                if views.len() >= READ_LIMITS.max_retained_snapshots {
                    return Err(Status::resource_exhausted("snapshot capacity reached"));
                }
                views.insert(
                    id.to_string(),
                    RetainedView {
                        view: view.clone(),
                        until: Instant::now()
                            + Duration::from_secs(READ_LIMITS.snapshot_idle_seconds),
                    },
                );
            }
        }
        let coverage = range.unwrap_or_else(|| KeyRange {
            start: if backwards || around {
                if has_before {
                    resume
                        .clone()
                        .or_else(|| entries.first().map(|e| e.key.clone()))
                } else {
                    None
                }
            } else {
                position.as_ref().map(ReadPosition::after)
            },
            end: if backwards {
                position.as_ref().map(ReadPosition::before)
            } else if has_after {
                resume
                    .clone()
                    .or_else(|| entries.last().map(|e| e.key.clone()))
            } else {
                None
            },
        });
        let next = if backwards {
            if has_before {
                resume
                    .clone()
                    .or_else(|| entries.first().map(|e| e.key.clone()))
            } else {
                None
            }
        } else if has_after {
            resume
                .clone()
                .or_else(|| entries.last().map(|e| e.key.clone()))
        } else {
            None
        };
        Ok(ReadBatch {
            entries,
            range: coverage,
            next,
            revision: reader.read_revision(&request)?,
            snapshot,
        })
    }
    pub fn release_snapshot(&self, id: SnapshotId) -> Result<(), Status> {
        self.check_cancelled()?;
        self.0
            .views
            .lock()
            .map_err(storage::failure)?
            .remove(&id.to_string());
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn ordering_key(rank: Decimal, key: &Key, sort: Sort) -> String {
    ReadPosition {
        selection: QueryFingerprint::parse(&"00".repeat(32)).unwrap(),
        sort,
        key: key.clone(),
        rank,
        at: None,
        edge: Edge::At,
    }
    .encode()
}
