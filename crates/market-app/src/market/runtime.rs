//! Application worker ownership, supervision and shutdown.
use super::supervisor::{TaskKind, TaskSupervisor};
use anyhow::{Result, anyhow, ensure};
use market_actors::MarketDataHandle;
use std::sync::{Arc, Mutex};
/// Process owner. Never construct this inside the HTTP-worker factory.
pub struct MarketRuntime {
    handle: MarketDataHandle,
    supervisor: Arc<TaskSupervisor>,
    replication: Mutex<Option<market_replica::ReplicationWorker>>,
}
impl MarketRuntime {
    pub async fn open(
        directory: &std::path::Path,
        source: &str,
        query_limit: usize,
        subscription_limit: usize,
    ) -> Result<Self> {
        ensure!(
            query_limit > 0 && subscription_limit > 0,
            "Market limits must be positive"
        );
        let market = market_replica::ReplicaRuntime::open(directory, source).await?;
        let supervisor = Arc::new(TaskSupervisor::new());
        let stop = supervisor.token();
        let data = market_actors::MarketDataService::start_with_shutdown(
            market.reader,
            query_limit,
            subscription_limit,
            stop.clone(),
        )
        .await?;
        supervisor.spawn_critical(TaskKind::MarketData, async move {
            if let Err(error) = data.task.run(stop).await {
                tracing::error!(code=?error.code(),"market data actor stopped");
                return Err(());
            }
            Ok(())
        })?;
        Ok(Self {
            handle: data.handle,
            supervisor,
            replication: Mutex::new(Some(market.replication)),
        })
    }
    pub fn handle(&self) -> MarketDataHandle {
        self.handle.clone()
    }
    pub fn start_replication(&self, hub: market_hub_client::HubClient) -> Result<()> {
        let replication = self
            .replication
            .lock()
            .map_err(|_| anyhow!("Replication worker state is unavailable"))?
            .take()
            .ok_or_else(|| anyhow!("Replication worker was already started"))?;
        let stop = self.supervisor.token();
        self.supervisor
            .spawn_critical(TaskKind::Replication, async move {
                if let Err(error) = replication.run(hub, stop).await {
                    tracing::error!(code=?error.code(), "replication worker stopped");
                    return Err(());
                }
                Ok(())
            })
    }
    #[cfg(feature = "bot")]
    pub fn start_bot(&self, worker: market_telegram::bot::BotWorker) -> Result<()> {
        let stop = self.supervisor.token();
        let commands = market_telegram::bot::MarketCommands::new(self.handle());
        self.supervisor.spawn_optional(TaskKind::Bot, async move {
            if let Err(error) = worker.run(commands, stop).await {
                tracing::error!(%error, "bot worker stopped");
                return Err(());
            }
            Ok(())
        })
    }
    #[cfg(feature = "bot")]
    pub fn start_media_warmup(&self, cache: market_media::MediaCache) -> Result<()> {
        let data = self.handle();
        let stop = self.supervisor.token();
        self.supervisor
            .spawn_optional(TaskKind::MediaWarmup, async move {
                tokio::select! {
                    _ = stop.cancelled() => {},
                    _ = crate::media::warmup(data, cache) => { stop.cancelled().await; },
                }
                Ok(())
            })
    }
    pub fn start_api(&self, server: market_api::ApiServer) -> Result<()> {
        let stop = self.supervisor.token();
        self.supervisor.spawn_critical(TaskKind::Api, async move {
            if let Err(error) = server.run(stop).await {
                tracing::error!(%error, "API server stopped");
                return Err(());
            }
            Ok(())
        })
    }
    pub async fn cancelled(&self) {
        self.supervisor.cancelled().await;
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.supervisor.shutdown().await
    }
    pub fn begin_shutdown(&self) -> Result<()> {
        self.supervisor.begin_shutdown()
    }
}
impl Drop for MarketRuntime {
    fn drop(&mut self) {
        let _ = self.supervisor.begin_shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn dropping_runtime_immediately_closes_data_access_and_drains_the_actor() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = MarketRuntime::open(directory.path(), "test", 1, 1)
            .await
            .unwrap();
        let handle = runtime.handle();
        let supervisor = runtime.supervisor.clone();
        drop(runtime);
        assert!(handle.status().await.is_err());
        tokio::time::timeout(std::time::Duration::from_secs(3), supervisor.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}
