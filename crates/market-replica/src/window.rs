//! Bounded, bidirectional active-order windows over one pinned view.
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
    pub anchor: Option<String>,
    pub before: u32,
    pub after: u32,
    pub snapshot: Option<String>,
}
#[derive(Clone)]
pub struct WindowEntity {
    pub entity: w::Entity,
    pub position: String,
}
pub struct WindowReadResult {
    pub entries: Vec<WindowEntity>,
    pub anchor: Option<String>,
    pub has_before: bool,
    pub has_after: bool,
    pub cursor: String,
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
}
impl Position {
    fn encode(&self) -> Result<String, Status> {
        Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).map_err(storage::failure)?))
    }
    fn decode(value: &str) -> Result<Self, Status> {
        if value.len() > 8192 {
            return Err(Status::invalid_argument("window anchor too large"));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| Status::invalid_argument("invalid window anchor"))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| Status::invalid_argument("invalid window anchor"))
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
        let size = window
            .before
            .checked_add(window.after)
            .and_then(|n| n.checked_add(1))
            .filter(|n| *n <= 200)
            .ok_or_else(|| Status::invalid_argument("window must contain 1..200 orders"))?
            as usize;
        if !request.page_token.is_empty() {
            return Err(Status::invalid_argument("windows do not use page tokens"));
        }
        self.expire();
        let frozen = match w::ReadMode::try_from(request.mode) {
            Ok(w::ReadMode::Snapshot) => true,
            Ok(w::ReadMode::Live) => false,
            _ => return Err(Status::invalid_argument("unknown read mode")),
        };
        if !frozen && window.snapshot.is_some() {
            return Err(Status::invalid_argument(
                "live windows do not use snapshot leases",
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
            let retained = views.get_mut(id).ok_or_else(|| {
                Status::failed_precondition("snapshot expired; restart window read")
            })?;
            retained.until = Instant::now() + Duration::from_secs(60);
            self.pin(retained.view.clone())
        } else {
            self.snapshot()?
        };
        validate(&reader, &request)?;
        let view = reader.view()?;
        let filter = Filter::compile(&view.state, &request)?;
        if filter.history() || filter.scope.is_none() {
            return Err(Status::invalid_argument(
                "windows require active order scope",
            ));
        }
        let mut position = window.anchor.as_deref().map(Position::decode).transpose()?;
        if let Some(p) = &position
            && (p.selection != filter.fingerprint
                || p.sort != filter.sort
                || p.key.kind != types::EntityKind::Order)
        {
            return Err(Status::invalid_argument(
                "window anchor belongs to another query or sort",
            ));
        }
        let eligible = |e: &w::Entity| -> Result<bool, Status> {
            reader.check_cancelled()?;
            Ok(filter.matches(e)? && include(e)?)
        };
        let mut pivot = None;
        if let Some(p) = &mut position
            && let Some(e) = view.state.books().get(&p.key)
            && eligible(e)?
        {
            p.rank = if filter.sort == Sort::Identity {
                Decimal::ZERO
            } else {
                filter.sort.rank(
                    state::order(e).ok_or_else(|| Status::data_loss("missing order"))?,
                    false,
                )?
            };
            pivot = Some(e.clone());
        }
        let scan = |reverse: bool| -> Result<Vec<(Key, Decimal, Arc<w::Entity>)>, Status> {
            let boundary = position.as_ref();
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
                if scanned >= 10_000 {
                    return Err(Status::resource_exhausted(
                        "window scan budget exceeded; narrow the filters",
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
        let mut before = if position.is_some() {
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
        let after_count = (size - 1 - before_count).min(after.len());
        let before_count = (size - 1 - after_count).min(before.len());
        let has_before = before.len() > before_count;
        let has_after = after.len() > after_count;
        let anchor = pivot
            .as_ref()
            .map(|(key, rank, _)| {
                Position {
                    selection: filter.fingerprint.clone(),
                    sort: filter.sort,
                    key: key.clone(),
                    rank: *rank,
                }
                .encode()
            })
            .transpose()?;
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
            let position = Position {
                selection: filter.fingerprint.clone(),
                sort: filter.sort,
                key,
                rank,
            }
            .encode()?;
            bytes += entity.encoded_len() + position.len() + 16;
            if bytes > 512 * 1024 {
                return Err(Status::resource_exhausted(
                    "window exceeds byte budget; reduce its size",
                ));
            }
            entries.push(WindowEntity {
                entity: entity.as_ref().clone(),
                position,
            });
        }
        if let Some(id) = &snapshot {
            let mut views = self.0.views.lock().map_err(storage::failure)?;
            self.check_cancelled()?;
            if window.snapshot.is_some() {
                let retained = views.get_mut(id).ok_or_else(|| {
                    Status::failed_precondition("snapshot released; restart window read")
                })?;
                retained.until = Instant::now() + Duration::from_secs(60);
            } else {
                if views.len() >= 128 {
                    return Err(Status::resource_exhausted("snapshot capacity reached"));
                }
                views.insert(
                    id.clone(),
                    RetainedView {
                        view: view.clone(),
                        until: Instant::now() + Duration::from_secs(60),
                    },
                );
            }
        }
        Ok(WindowReadResult {
            entries,
            anchor,
            has_before,
            has_after,
            cursor: view.cursor.clone(),
            sync: reader.status(),
            revision: reader.read_revision(&request)?,
            snapshot,
        })
    }
    pub fn release_window_snapshot(&self, id: &str) -> Result<(), Status> {
        self.check_cancelled()?;
        validate_snapshot(id)?;
        self.0.views.lock().map_err(storage::failure)?.remove(id);
        Ok(())
    }
}

fn validate_snapshot(id: &str) -> Result<(), Status> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(Status::invalid_argument("invalid snapshot token"));
    }
    Ok(())
}
