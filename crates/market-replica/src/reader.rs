use super::*;
use crate::{
    query::Filter,
    types::{EntityKind, Key, Position, RankKey, Sort},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use prost::Message;
use std::ops::Bound::{Excluded, Unbounded};
#[derive(Serialize, Deserialize, Clone)]
enum Boundary {
    Current(Option<Key>),
    History(Option<Position>),
    // Accept pre-window price cursors while emitting one canonical ranked format.
    #[serde(alias = "Price")]
    Ranked(Option<(String, Key)>),
    SortedHistory(Option<(String, Position)>),
}
#[derive(Serialize, Deserialize)]
struct Continuation {
    selection: String,
    sort: Sort,
    position: Boundary,
    snapshot: Option<String>,
}
fn decode(text: &str) -> Result<Continuation, Status> {
    if text.len() > READ_LIMITS.max_position_bytes {
        return Err(Status::invalid_argument("page token too large"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| Status::invalid_argument("invalid page token"))?;
    serde_json::from_slice(&bytes).map_err(|_| Status::invalid_argument("invalid page token"))
}
impl ReplicaReader {
    /// Public tokens contain validated keyset boundaries, never upstream credentials or view IDs.
    pub fn read(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        self.read_inner(
            request,
            None::<fn(&Self, &w::ReadRequest) -> Result<(), Status>>,
            &|_| Ok(true),
        )
    }
    /// Validate a query against the exact view used for its results, including retained
    /// continuations. The long-lived reader keeps ownership of pagination state.
    /// The callback's reader is pinned for short metadata reads; use LIVE mode there.
    /// Apply a caller-owned row policy within the pinned scan, before page size/byte accounting.
    /// The policy must stay stable across a continuation traversal; replica semantics stay generic.
    pub fn read_checked(
        &self,
        request: w::ReadRequest,
        validate: impl FnOnce(&Self, &w::ReadRequest) -> Result<(), Status>,
        include: impl Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<ReadResult, Status> {
        self.read_inner(request, Some(validate), &include)
    }
    fn read_inner(
        &self,
        request: w::ReadRequest,
        validate: Option<impl FnOnce(&Self, &w::ReadRequest) -> Result<(), Status>>,
        include: &dyn Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<ReadResult, Status> {
        let live = match w::ReadMode::try_from(request.mode) {
            Ok(w::ReadMode::Snapshot) => false,
            Ok(w::ReadMode::Live) => true,
            Err(_) => return Err(Status::invalid_argument("unknown read mode")),
        };
        let limit = if request.limit == 0 {
            READ_LIMITS.max_rows as usize
        } else {
            request.limit as usize
        };
        if limit > READ_LIMITS.max_batch_rows as usize {
            return Err(Status::invalid_argument("invalid page size"));
        }
        self.expire();
        let token = if request.page_token.is_empty() {
            None
        } else {
            Some(decode(&request.page_token)?)
        };
        let existing = token
            .as_ref()
            .and_then(|t| t.snapshot.as_ref())
            .filter(|_| !live);
        let view = if let Some(id) = existing {
            let mut views = self.0.views.lock().map_err(storage::failure)?;
            let retained = views.get_mut(id).ok_or_else(|| {
                Status::failed_precondition("snapshot expired or app restarted; restart read")
            })?;
            retained.until =
                Instant::now() + Duration::from_secs(READ_LIMITS.snapshot_idle_seconds);
            retained.view.clone()
        } else {
            self.view()?
        };
        if let Some(validate) = validate {
            validate(&self.pin(view.clone()), &request)?;
        }
        let filter = Filter::compile(&view.state, &request)?;
        if token
            .as_ref()
            .is_some_and(|t| t.selection != filter.fingerprint || t.sort != filter.sort)
        {
            return Err(Status::invalid_argument(
                "page belongs to another query or sort",
            ));
        }
        let position = token
            .as_ref()
            .map(|t| t.position.clone())
            .unwrap_or_else(|| {
                if filter.history() {
                    if filter.sort == Sort::Identity {
                        Boundary::History(None)
                    } else {
                        Boundary::SortedHistory(None)
                    }
                } else if filter.sort != Sort::Identity {
                    Boundary::Ranked(None)
                } else {
                    Boundary::Current(None)
                }
            });
        let (entities, next) = scan(
            &view,
            &filter,
            position,
            limit,
            &|entity| {
                self.check_cancelled()?;
                include(entity)
            },
            &|| self.check_cancelled(),
        )?;
        let id = if live {
            None
        } else {
            Some(
                existing
                    .cloned()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            )
        };
        let next_page_token = if let Some(position) = next {
            if let Some(id) = &id {
                let mut views = self.0.views.lock().map_err(storage::failure)?;
                if views.len() >= READ_LIMITS.max_retained_snapshots && !views.contains_key(id) {
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
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Continuation {
                    selection: filter.fingerprint,
                    sort: filter.sort,
                    position,
                    snapshot: id,
                })
                .map_err(storage::failure)?,
            )
        } else {
            if let Some(id) = id {
                self.0.views.lock().map_err(storage::failure)?.remove(&id);
            }
            String::new()
        };
        Ok(ReadResult {
            page: w::Page {
                entities,
                next_page_token,
                cursor: view.cursor.clone(),
            },
            sync: self.status(),
        })
    }
}
struct ScanRow<'a> {
    rank: Option<rust_decimal::Decimal>,
    key: &'a Key,
    entity: Option<&'a Arc<w::Entity>>,
}

fn scan(
    view: &ReplicaView,
    filter: &Filter,
    mut boundary: Boundary,
    limit: usize,
    include: &dyn Fn(&w::Entity) -> Result<bool, Status>,
    check: &dyn Fn() -> Result<(), Status>,
) -> Result<(Vec<w::Entity>, Option<Boundary>), Status> {
    if filter.history() {
        if filter.sort != Sort::Identity {
            let Boundary::SortedHistory(after) = boundary else {
                return Err(Status::invalid_argument("invalid sorted history boundary"));
            };
            let (data, next) = view.history.ranked_history(filter, after, limit, include)?;
            return Ok((data, next.map(|p| Boundary::SortedHistory(Some(p)))));
        }
        let Boundary::History(after) = boundary else {
            return Err(Status::invalid_argument("invalid history boundary"));
        };
        let (data, next) = view
            .history
            .history(filter, after.as_ref(), limit, include)?;
        return Ok((data, next.map(|p| Boundary::History(Some(p)))));
    }
    let mut data = Vec::new();
    let mut bytes = 0;
    let rows: Box<dyn Iterator<Item = ScanRow<'_>> + '_> = match &boundary {
        Boundary::Ranked(after) if filter.sort != Sort::Identity => {
            let after: Option<RankKey> = after
                .as_ref()
                .map(|(p, k)| Ok::<_, Status>(RankKey::new(state::decimal(p)?, k.clone())))
                .transpose()?;
            Box::new(
                view.state
                    .books()
                    .ranked(
                        filter
                            .scope
                            .as_ref()
                            .ok_or_else(|| Status::invalid_argument("missing price scope"))?,
                        after,
                        filter.sort,
                    )
                    .map(|(rank, entity)| {
                        let (value, key) = rank.entity();
                        ScanRow {
                            rank: Some(value),
                            key,
                            entity: Some(entity),
                        }
                    }),
            )
        }
        Boundary::Current(after) if filter.sort == Sort::Identity => {
            if !filter.keys.is_empty() {
                Box::new(
                    filter
                        .keys
                        .range((after.as_ref().map_or(Unbounded, Excluded), Unbounded))
                        .map(|key| ScanRow {
                            rank: None,
                            key,
                            entity: view.state.entity(key),
                        }),
                )
            } else {
                let kind = if filter.scope.is_some() {
                    Some(EntityKind::Order)
                } else if filter.kinds.len() == 1 {
                    filter.kinds.first().copied()
                } else {
                    None
                };
                if let Some(kind) = kind
                    && after.as_ref().is_some_and(|key| key.kind != kind)
                {
                    return Err(Status::invalid_argument("invalid entity boundary"));
                }
                Box::new(
                    view.state
                        .range(after.as_ref(), kind)
                        .map(|(key, entity)| ScanRow {
                            rank: None,
                            key,
                            entity: Some(entity),
                        }),
                )
            }
        }
        _ => return Err(Status::invalid_argument("invalid page boundary")),
    };
    for (count, ScanRow { rank, key, entity }) in rows.enumerate() {
        if count % 64 == 0 {
            check()?;
        }
        if count == READ_LIMITS.max_scan_candidates {
            return Ok((data, Some(boundary)));
        }
        if let Some(entity) = entity
            && filter.matches(entity)?
            && include(entity)?
        {
            if data.len() == limit {
                return Ok((data, Some(boundary)));
            }
            let size = entity.encoded_len() + 8;
            if bytes + size > READ_LIMITS.max_read_bytes {
                if data.is_empty() {
                    return Err(Status::resource_exhausted("order exceeds page budget"));
                }
                return Ok((data, Some(boundary)));
            }
            bytes += size;
            data.push(entity.as_ref().clone());
        }
        boundary = match rank {
            Some(value) => Boundary::Ranked(Some((value.to_string(), key.clone()))),
            None => Boundary::Current(Some(key.clone())),
        };
    }
    Ok((data, None))
}

#[cfg(test)]
mod tests;
