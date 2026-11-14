use super::*;
use tokio_util::sync::CancellationToken;
use tonic::{
    Request,
    metadata::{Ascii, MetadataValue},
    transport::Channel,
};
fn request<T>(authorization: &MetadataValue<Ascii>, value: T) -> Request<T> {
    let mut r = Request::new(value);
    r.metadata_mut()
        .insert("authorization", authorization.clone());
    r
}
fn rpc(channel: Channel) -> w::market_read_client::MarketReadClient<Channel> {
    w::market_read_client::MarketReadClient::new(channel).max_decoding_message_size(1024 * 1024)
}
async fn page(
    channel: Channel,
    auth: &MetadataValue<Ascii>,
    token: String,
) -> Result<w::Page, Status> {
    for attempt in 0..6 {
        let mut request = request(
            auth,
            w::ReadRequest {
                limit: 500,
                page_token: token.clone(),
                ..Default::default()
            },
        );
        request.set_timeout(Duration::from_secs(10));
        let result = rpc(channel.clone()).read(request).await;
        let mut stream = match result {
            Err(error) if error.code() == tonic::Code::ResourceExhausted && attempt < 5 => {
                tokio::time::sleep(Duration::from_millis(25 << attempt)).await;
                continue;
            }
            result => result?.into_inner(),
        };
        let page = stream
            .message()
            .await?
            .ok_or_else(|| Status::data_loss("missing snapshot page"))?;
        if stream.message().await?.is_some() {
            return Err(Status::data_loss("extra snapshot page"));
        }
        return Ok(page);
    }
    unreachable!()
}
async fn watch(
    channel: Channel,
    auth: &MetadataValue<Ascii>,
    cursor: String,
) -> Result<tonic::Streaming<w::Batch>, Status> {
    tokio::time::timeout(
        Duration::from_secs(10),
        rpc(channel).watch(request(
            auth,
            w::WatchRequest {
                after_cursor: cursor,
                ..Default::default()
            },
        )),
    )
    .await
    .map_err(|_| Status::deadline_exceeded("sync connection timed out"))?
    .map(|r| r.into_inner())
}
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Status> + Send + 'static,
) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(storage::failure)?
}
async fn collecting<T>(
    work: impl std::future::Future<Output = Result<T, Status>>,
    stream: &mut tonic::Streaming<w::Batch>,
    stage: &Store,
    queued: &mut u64,
) -> Result<T, Status> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            result = &mut work => return result,
            batch = stream.message() => {
                let batch=batch?.ok_or_else(||Status::unavailable("replication stream closed"))?;
                let stage=stage.clone();blocking(move||stage.journal(&batch)).await?;*queued+=1;
            }
        }
    }
}
async fn bootstrap(
    replica: &Replica,
    channel: Channel,
    auth: &MetadataValue<Ascii>,
) -> Result<tonic::Streaming<w::Batch>, Status> {
    replica.mark(SyncState::Syncing);
    let mut next = page(channel.clone(), auth, String::new()).await?;
    let baseline = next.cursor.clone();
    if baseline.is_empty() {
        return Err(Status::data_loss("missing snapshot checkpoint"));
    }
    let mut changes = watch(channel.clone(), auth, baseline.clone()).await?;
    let path = replica.0.path.with_file_name("building.redb");
    let source = replica.0.source.clone();
    let stage = blocking(move || {
        if path.exists() {
            std::fs::remove_file(&path).map_err(storage::failure)?;
        }
        Store::open(&path, &source)
    })
    .await?;
    let mut state = State::default();
    let mut queued = 0;
    loop {
        if next.cursor != baseline {
            return Err(Status::data_loss("snapshot checkpoint changed"));
        }
        let token = next.next_page_token;
        let rows = next
            .entities
            .into_iter()
            .map(|entity| w::Change {
                change: Some(w::change::Change::Upsert(entity)),
            })
            .collect::<Vec<_>>();
        let store = stage.clone();
        state = collecting(
            blocking(move || {
                state.apply(&rows)?;
                store.apply(&rows, None)?;
                Ok(state)
            }),
            &mut changes,
            &stage,
            &mut queued,
        )
        .await?;
        if token.is_empty() {
            break;
        }
        next = collecting(
            page(channel.clone(), auth, token),
            &mut changes,
            &stage,
            &mut queued,
        )
        .await?;
    }
    // Replay one bounded delivery at a time while continuing to drain Watch to disk.
    let mut position = 0;
    let mut cursor = baseline;
    loop {
        let store = stage.clone();
        let (next_state, checkpoint) = collecting(
            blocking(move || {
                let batch = store.journal_at(position)?;
                if let Some(batch) = batch {
                    state.apply(&batch.changes)?;
                    store.apply(&batch.changes, None)?;
                    Ok((state, Some(batch.cursor)))
                } else {
                    Ok((state, None))
                }
            }),
            &mut changes,
            &stage,
            &mut queued,
        )
        .await?;
        state = next_state;
        if let Some(checkpoint) = checkpoint {
            cursor = checkpoint;
            position += 1;
        } else if position == queued {
            break;
        }
    }
    let store = stage.clone();
    blocking(move || store.finish(&cursor)).await?;
    let target = replica.0.path.clone();
    let install = replica.clone();
    blocking(move || {
        std::fs::rename(target.with_file_name("building.redb"), &target)
            .map_err(storage::failure)?;
        std::fs::File::open(
            target
                .parent()
                .ok_or_else(|| storage::failure("missing replica directory"))?,
        )
        .map_err(storage::failure)?
        .sync_all()
        .map_err(storage::failure)?;
        install.install_state(stage, state)
    })
    .await?;
    Ok(changes)
}
async fn session(
    replica: &Replica,
    channel: Channel,
    auth: &MetadataValue<Ascii>,
    rebuild: &mut bool,
    backoff: &mut u64,
) -> Result<(), Status> {
    let mut stream = if *rebuild {
        bootstrap(replica, channel, auth).await?
    } else {
        let cursor = replica.0.store.lock().map_err(storage::failure)?.cursor()?;
        watch(channel, auth, cursor).await?
    };
    *rebuild = false;
    loop {
        let batch = stream
            .message()
            .await?
            .ok_or_else(|| Status::unavailable("replication stream closed"))?;
        let target = replica.clone();
        blocking(move || target.apply(batch)).await?;
        *backoff = 1;
    }
}
pub async fn run(
    replica: Replica,
    channel: Channel,
    auth: MetadataValue<Ascii>,
    stop: CancellationToken,
) {
    let mut rebuild = replica
        .0
        .store
        .lock()
        .map(|s| s.cursor().unwrap_or_default().is_empty())
        .unwrap_or(true);
    let mut backoff = 1;
    loop {
        let error = tokio::select! {
            _=stop.cancelled()=>break,
            result=session(&replica,channel.clone(),&auth,&mut rebuild,&mut backoff)=>match result{Ok(())=>break,Err(error)=>error},
        };
        if matches!(
            error.code(),
            tonic::Code::FailedPrecondition
                | tonic::Code::OutOfRange
                | tonic::Code::InvalidArgument
        ) {
            rebuild = true;
        }
        tracing::warn!(code=?error.code(),"replica disconnected; reconnecting");
        replica.mark(SyncState::Stale);
        let delay =
            Duration::from_millis(backoff * 1000 + (uuid::Uuid::new_v4().as_u128() % 251) as u64);
        tokio::select! {_=stop.cancelled()=>break,_=tokio::time::sleep(delay)=>{}}
        backoff = (backoff * 2).min(30);
    }
}
