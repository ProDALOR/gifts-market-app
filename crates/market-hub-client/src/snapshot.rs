//! Coordinated bounded exports, with paged bootstrap for the deployed older hub.
use super::*;

pub struct Snapshot {
    source: Source,
    failed: Option<Status>,
}
enum Source {
    Split(Box<Split>),
    Pages {
        hub: Box<HubClient>,
        token: Option<String>,
    },
}
impl HubClient {
    pub async fn snapshot(&self) -> Result<Snapshot, Status> {
        // No RPC deadline: the whole stream may take minutes. Bound setup and
        // each awaited frame instead; backpressure during staging is intentional.
        let session = tokio::time::timeout(
            Duration::from_secs(10),
            self.rpc().open_snapshot(self.request(w::Empty {})),
        )
        .await
        .map_err(|_| Status::deadline_exceeded("snapshot session setup timed out"))?;
        match session {
            Ok(session) => return Split::open(self.clone(), session.into_inner()).await,
            Err(error) if error.code() == tonic::Code::Unimplemented => {}
            Err(error) => return Err(error),
        }
        tracing::info!("hub coordinated snapshots unavailable; using paged bootstrap");
        let source = Source::Pages {
            hub: Box::new(self.clone()),
            token: Some(String::new()),
        };
        Ok(Snapshot {
            source,
            failed: None,
        })
    }
}
impl Snapshot {
    pub fn transport(&self) -> &'static str {
        match self.source {
            Source::Split(_) => "split_stream",
            Source::Pages { .. } => "pages",
        }
    }
    pub async fn next(&mut self) -> Result<Option<w::Page>, Status> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        let result = self.next_page().await;
        if let Err(error) = &result {
            self.failed = Some(error.clone());
        }
        result
    }
    async fn next_page(&mut self) -> Result<Option<w::Page>, Status> {
        let page = match &mut self.source {
            Source::Split(split) => return split.next().await,
            Source::Pages { hub, token } => {
                let Some(next) = token.as_ref() else {
                    return Ok(None);
                };
                Some(hub.snapshot_page(next.clone()).await?)
            }
        };
        if let Some(page) = &page
            && let Source::Pages { token, .. } = &mut self.source
        {
            *token = (!page.next_page_token.is_empty()).then(|| page.next_page_token.clone());
        }
        Ok(page)
    }
}

/// A lease covers setup cancellation too, including a failure to start either part.
struct SessionLease {
    hub: HubClient,
    token: String,
    active: bool,
}
impl Drop for SessionLease {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let hub = self.hub.clone();
        let token = self.token.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(
                    Duration::from_secs(5),
                    hub.rpc()
                        .close_snapshot(hub.request(w::SnapshotToken { token })),
                )
                .await;
            });
        }
    }
}
struct Part {
    kind: w::SnapshotPart,
    started: std::time::Instant,
    pages: u64,
    rows: u64,
    stream: tonic::Streaming<w::Page>,
    terminal: bool,
    ended: bool,
}
impl Part {
    fn new(kind: w::SnapshotPart, stream: tonic::Streaming<w::Page>) -> Self {
        Self {
            kind,
            stream,
            started: std::time::Instant::now(),
            pages: 0,
            rows: 0,
            terminal: false,
            ended: false,
        }
    }
    async fn next(&mut self) -> Result<Option<w::Page>, Status> {
        if self.ended {
            return Ok(None);
        }
        let page = tokio::time::timeout(Duration::from_secs(10), self.stream.message())
            .await
            .map_err(|_| Status::deadline_exceeded("snapshot part frame timed out"))??;
        match &page {
            Some(_) if self.terminal => {
                return Err(Status::data_loss(
                    "snapshot part sent data after final page",
                ));
            }
            None if !self.terminal => {
                return Err(Status::data_loss("snapshot part ended before final page"));
            }
            Some(page) => {
                self.terminal = page.next_page_token.is_empty();
                self.pages += 1;
                self.rows += page.entities.len() as u64;
            }
            None => {
                self.ended = true;
                tracing::info!(part=?self.kind, pages=self.pages, rows=self.rows, elapsed_ms=self.started.elapsed().as_millis(), "replica snapshot part completed");
            }
        }
        Ok(page)
    }
}
struct Split {
    current: Part,
    history: Part,
    cursor: String,
    lease: SessionLease,
}
impl Split {
    async fn open(hub: HubClient, session: w::SnapshotSession) -> Result<Snapshot, Status> {
        let lease = SessionLease {
            hub: hub.clone(),
            token: session.token.clone(),
            active: true,
        };
        if session.cursor.is_empty() || session.token.is_empty() {
            return Err(Status::data_loss("invalid snapshot session"));
        }
        let current = hub.request(w::SnapshotPartRequest {
            token: session.token.clone(),
            part: w::SnapshotPart::Current as i32,
        });
        let history = hub.request(w::SnapshotPartRequest {
            token: session.token,
            part: w::SnapshotPart::History as i32,
        });
        let (current, history) = tokio::time::timeout(Duration::from_secs(10), async {
            let mut current_rpc = hub.current_rpc();
            let mut history_rpc = hub.history_rpc();
            tokio::try_join!(
                current_rpc.read_snapshot_part(current),
                history_rpc.read_snapshot_part(history)
            )
        })
        .await
        .map_err(|_| Status::deadline_exceeded("snapshot parts setup timed out"))??;
        Ok(Snapshot {
            source: Source::Split(Box::new(Self {
                current: Part::new(w::SnapshotPart::Current, current.into_inner()),
                history: Part::new(w::SnapshotPart::History, history.into_inner()),
                cursor: session.cursor,
                lease,
            })),
            failed: None,
        })
    }
    async fn next(&mut self) -> Result<Option<w::Page>, Status> {
        loop {
            if self.current.ended && self.history.ended {
                self.lease.active = false;
                return Ok(None);
            }
            let (page, history) = tokio::select! {
                page = self.current.next(), if !self.current.ended => (page?, false),
                page = self.history.next(), if !self.history.ended => (page?, true),
            };
            if let Some(page) = page {
                if page.cursor != self.cursor {
                    return Err(Status::data_loss("snapshot part checkpoint changed"));
                }
                for entity in &page.entities {
                    let executed = matches!(&entity.entity, Some(w::entity::Entity::Order(order)) if order.mode == w::OrderMode::Executed as i32);
                    if executed != history {
                        return Err(Status::data_loss("entity belongs to another snapshot part"));
                    }
                }
                return Ok(Some(page));
            }
        }
    }
}
