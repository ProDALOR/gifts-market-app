//! Half-open order ranges over one pinned active/history view.
use crate::read_key::{Edge, PositionSource};
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
pub struct ReadEntry {
    pub entity: Arc<w::Entity>,
    pub key: ReadPosition,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// Half-open ordering coverage: start is inclusive, end is exclusive.
pub struct KeyRange {
    pub start: Option<ReadPosition>,
    pub end: Option<ReadPosition>,
}
#[derive(Clone)]
pub struct RangeRequest {
    pub range: KeyRange,
    pub limit: u32,
    pub snapshot: Option<SnapshotId>,
    pub options: ReadOptions,
}
pub struct ReadBatch {
    pub entries: Vec<ReadEntry>,
    pub range: KeyRange,
    pub revision: ReadRevision,
    /// Monotonic observation clock within this reader instance; not a scoped cache key.
    pub observed: u64,
    pub snapshot: Option<SnapshotId>,
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
    /// Read a bounded page or reconcile a complete half-open range.
    /// Continue a page at its returned range end, retaining its snapshot lease.
    pub fn query_range_checked(
        &self,
        request: w::ReadRequest,
        read: RangeRequest,
        complete: bool,
        validate: impl FnOnce(&Self, &w::ReadRequest) -> Result<(), Status>,
        include: impl Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<ReadBatch, Status> {
        let size = read.limit as usize;
        if size == 0 || size > READ_LIMITS.max_batch_rows as usize {
            return Err(Status::invalid_argument("query exceeds row budget"));
        }
        if !request.page_token.is_empty() {
            return Err(Status::invalid_argument(
                "range reads do not use page tokens",
            ));
        }
        self.expire();
        let frozen = match w::ReadMode::try_from(request.mode) {
            Ok(w::ReadMode::Snapshot) => true,
            Ok(w::ReadMode::Live) => false,
            _ => return Err(Status::invalid_argument("unknown read mode")),
        };
        if !frozen && read.snapshot.is_some() {
            return Err(Status::invalid_argument(
                "latest reads do not use snapshot leases",
            ));
        }
        let snapshot = if frozen {
            Some(read.snapshot.unwrap_or_else(SnapshotId::fresh))
        } else {
            None
        };
        let reader = if let Some(id) = &read.snapshot {
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
        let filter = Filter::compile_with_options(&view.state, &request, read.options)?;
        if filter.scope.is_none() {
            return Err(Status::invalid_argument("reads require an order scope"));
        }
        let position = read.range.start.clone();
        let end = read.range.end.clone();
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
        let source = PositionSource::from_filter(&filter);
        let make_position = |key: Key, rank: Decimal, at: Option<i64>| ReadPosition {
            source,
            sort: filter.sort,
            key,
            rank,
            at,
            edge: Edge::At,
        };
        let eligible = |e: &w::Entity| -> Result<bool, Status> {
            reader.check_cancelled()?;
            if read.range.start.is_some() || read.range.end.is_some() {
                let range = &read.range;
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
                    || range.end.as_ref().is_some_and(|end| point >= *end)
                {
                    return Ok(false);
                }
            }
            Ok(filter.matches(e)? && include(e)?)
        };
        // Ranked scans start strictly after a key. Include a surviving lower
        // bound only when it still occupies that exact observed ordering position.
        let pivot = if let Some(p) = &position {
            let existing = if filter.history() {
                view.history
                    .execution(&p.history_position()?)?
                    .map(Arc::new)
            } else {
                view.state.books().get(&p.key).cloned()
            };
            existing
                .filter(|_| p.edge != Edge::After)
                .map(|entity| {
                    Ok::<_, Status>((
                        p.key.clone(),
                        entity_rank(&entity, filter.sort, filter.history())?,
                        entity,
                    ))
                })
                .transpose()?
                .filter(|(_, rank, _)| *rank == p.rank)
                .map(|row| eligible(&row.2).map(|include| include.then_some(row)))
                .transpose()?
                .flatten()
        } else {
            None
        };
        let directional = !complete;
        let mut resume = None;
        let mut scan = || -> Result<Vec<(Key, Decimal, Arc<w::Entity>)>, Status> {
            let boundary = position.as_ref();
            if filter.history() {
                let mut scan_filter = filter.clone();
                if scan_filter.sort == Sort::Identity {
                    scan_filter.sort = Sort::TimeAsc;
                }
                let after = boundary
                    .map(|p| Ok::<_, Status>((p.rank.to_string(), p.history_position()?)))
                    .transpose()?;
                let end = end
                    .as_ref()
                    .map(|p| Ok::<_, Status>((p.rank.to_string(), p.history_position()?)))
                    .transpose()?;
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
                    let low = boundary
                        .map(|p| Excluded(p.key.clone()))
                        .unwrap_or(Unbounded);
                    Box::new(
                        view.state
                            .books()
                            .range((low, Unbounded))
                            .map(|(key, entity)| (key, Decimal::ZERO, entity)),
                    )
                } else {
                    Box::new(
                        view.state
                            .books()
                            .ranked(
                                filter.scope.as_ref().expect("scope checked"),
                                boundary.map(|p| RankKey::new(p.rank, p.key.clone())),
                                filter.sort,
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
                if end
                    .as_ref()
                    .is_some_and(|end| make_position(key.clone(), rank, None) >= *end)
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
        let mut rows = scan()?;
        let count = (size - usize::from(pivot.is_some())).min(rows.len());
        let has_after = rows.len() > count || resume.is_some();
        let selected = pivot.into_iter().chain(rows.drain(..count));
        let mut bytes = 0;
        let mut entries = Vec::with_capacity(size.min(128));
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
        if complete && has_after {
            return Err(Status::resource_exhausted(
                "watched range exceeds row budget; narrow its bounds",
            ));
        }
        if let Some(id) = &snapshot {
            let mut views = self.0.views.lock().map_err(storage::failure)?;
            self.check_cancelled()?;
            if read.snapshot.is_some() {
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
        let coverage = if !complete && has_after {
            KeyRange {
                start: read.range.start,
                end: resume
                    .or_else(|| entries.last().map(|entry| entry.key.clone()))
                    .map(|point| point.after()),
            }
        } else {
            read.range
        };
        Ok(ReadBatch {
            entries,
            range: coverage,
            revision: reader.read_revision(&request)?,
            observed: view.revisions.sequence,
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
        source: PositionSource::decode(1).unwrap(),
        sort,
        key: key.clone(),
        rank,
        at: None,
        edge: Edge::At,
    }
    .encode()
}
