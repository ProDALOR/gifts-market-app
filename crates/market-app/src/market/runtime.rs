//! Application worker ownership, supervision and shutdown.
use super::supervisor::{TaskKind, TaskSupervisor};
use anyhow::{Result, anyhow, ensure};
use market_actors::MarketDataHandle;
use std::sync::{Arc, Mutex};
/// Process owner. Never construct this inside the HTTP-worker factory.
pub struct MarketRuntime {
    handle: MarketDataHandle,
    reads: market_replica::ReadRuntime,
    valuation: market_valuation::ValuationHandle,
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
        let reads = market_replica::ReadRuntime::new(market.reader, query_limit, stop.clone())?;
        let data = market_actors::MarketDataService::start_with_shutdown(
            reads.handle(),
            subscription_limit,
            stop.clone(),
        )
        .await?;
        let (registry, valuation_task) =
            market_valuation::ValuationHandle::start(&data.arbiter(), reads.handle()).await?;
        let valuation_stop = supervisor.token();
        supervisor.spawn_critical(TaskKind::Valuation(registry.metadata().id), async move {
            valuation_task.run(valuation_stop).await.map_err(|error| {
                tracing::error!(%error,"gift valuation actor stopped");
            })
        })?;
        supervisor.spawn_critical(TaskKind::MarketData, async move {
            if let Err(error) = data.task.run(stop).await {
                tracing::error!(code=?error.code(),"market data actor stopped");
                return Err(());
            }
            Ok(())
        })?;
        Ok(Self {
            handle: data.handle,
            reads,
            valuation: registry,
            supervisor,
            replication: Mutex::new(Some(market.replication)),
        })
    }
    pub fn handle(&self) -> MarketDataHandle {
        self.handle.clone()
    }
    pub fn valuation(&self) -> market_valuation::ValuationHandle {
        self.valuation.clone()
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
    pub fn start_bot(&self, worker: market_telegram::TelegramWorker) -> Result<()> {
        let stop = self.supervisor.token();
        let data = self.handle();
        let valuation = self.valuation();
        self.supervisor.spawn_optional(TaskKind::Bot, async move {
            use futures_util::FutureExt;
            use std::panic::AssertUnwindSafe;
            let invocations =
                market_invocations::Runtime::start(Default::default()).map_err(|error| {
                    tracing::error!(%error, "invocation runtime failed");
                })?;
            let bot = market_telegram_bot::BotService::new(worker, data, valuation);
            let result = AssertUnwindSafe(bot.run_with_invocations(stop, invocations.handle()))
                .catch_unwind()
                .await;
            invocations.shutdown().await;
            result
                .map_err(|_| {
                    tracing::error!("bot worker panicked");
                })?
                .map_err(|error| {
                    tracing::error!(%error, "bot worker stopped");
                })
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
        let result = self.supervisor.shutdown().await;
        self.reads.shutdown().await;
        result
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
    #[tokio::test]
    async fn app_shutdown_drains_shared_read_workers_after_actor_services_stop() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = MarketRuntime::open(directory.path(), "test", 1, 1)
            .await
            .unwrap();
        let reads = runtime.reads.handle();
        let permit = reads.budget().try_query().unwrap();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, proceed) = std::sync::mpsc::channel();
        let work = tokio::spawn(async move {
            reads
                .admitted(permit, move |reader| {
                    entered.send(()).unwrap();
                    let _ = proceed.recv();
                    reader.check_cancelled().is_err()
                })
                .await
                .unwrap()
        });
        started.await.unwrap();
        let mut shutdown = Box::pin(runtime.shutdown());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut shutdown)
                .await
                .is_err()
        );
        assert!(runtime.handle().status().await.is_err());
        release.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), shutdown)
            .await
            .unwrap()
            .unwrap();
        assert!(work.await.unwrap());
    }
    #[cfg(feature = "bot")]
    #[tokio::test]
    async fn shutdown_joins_bot_and_invocations_before_opening_cancelled_transport() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = MarketRuntime::open(&directory.path().join("replica"), "test", 1, 1)
            .await
            .unwrap();
        let worker = market_telegram::TelegramWorker::new(market_telegram::BotConfig {
            credentials: market_telegram::BotCredentials::new(
                "42:private_test_secret".into(),
                12345,
                "0123456789abcdef0123456789abcdef".into(),
            )
            .unwrap(),
            public_url: None,
            data_dir: directory.path().join("telegram"),
        })
        .unwrap();
        runtime.start_bot(worker).unwrap();
        runtime.begin_shutdown().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), runtime.shutdown())
            .await
            .unwrap()
            .unwrap();
        assert!(!directory.path().join("telegram").exists());
    }
}
