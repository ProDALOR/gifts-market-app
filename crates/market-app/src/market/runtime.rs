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
    #[cfg(feature = "bot")]
    portfolio: std::sync::RwLock<Option<market_portfolio::PortfolioHandle>>,
    #[cfg(feature = "bot")]
    invocations: Mutex<Option<market_invocations::Runtime>>,
}
impl MarketRuntime {
    pub async fn open(
        directory: &std::path::Path,
        source: &str,
        query_limit: usize,
        subscription_limit: usize,
    ) -> Result<Self> {
        Self::open_with_cancellation(
            directory,
            source,
            query_limit,
            subscription_limit,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
    }
    pub async fn open_with_cancellation(
        directory: &std::path::Path,
        source: &str,
        query_limit: usize,
        subscription_limit: usize,
        stop: tokio_util::sync::CancellationToken,
    ) -> Result<Self> {
        ensure!(
            query_limit > 0 && subscription_limit > 0,
            "Market limits must be positive"
        );
        let stop = stop.child_token();
        let cancel = stop.clone().drop_guard();
        let market =
            market_replica::ReplicaRuntime::open_with_cancellation(directory, source, stop.clone())
                .await?;
        let supervisor = Arc::new(TaskSupervisor::with_stop(stop));
        let diagnostics_stop = supervisor.token();
        supervisor.spawn_critical(
            TaskKind::Diagnostics,
            super::worker::monitor("market-app", diagnostics_stop),
        )?;
        let stop = supervisor.token();
        let reads = market_replica::ReadRuntime::new(market.reader, query_limit, stop.clone())?;
        let services = async {
            ensure!(!supervisor.is_stopping(), "Application is stopping");
            let starting = std::time::Instant::now();
            tracing::info!("starting market data actors");
            let data = market_actors::MarketDataService::start_with_shutdown(
                reads.handle(),
                subscription_limit,
                stop.clone(),
            )
            .await?;
            tracing::info!(
                elapsed_ms = starting.elapsed().as_millis(),
                "market data actors started"
            );
            let arbiter = data.arbiter();
            let handle = data.handle;
            supervisor.spawn_critical(TaskKind::MarketData, async move {
                if let Err(error) = data.task.run(stop).await {
                    tracing::error!(code=?error.code(),"market data actor stopped");
                    return Err(());
                }
                Ok(())
            })?;
            let starting = std::time::Instant::now();
            tracing::info!("starting valuation service");
            let (registry, valuation_task) =
                market_valuation::ValuationHandle::start(&arbiter, reads.handle()).await?;
            tracing::info!(
                elapsed_ms = starting.elapsed().as_millis(),
                "valuation service started"
            );
            let valuation_stop = supervisor.token();
            supervisor.spawn_critical(TaskKind::Valuation("valuation"), async move {
                valuation_task.run(valuation_stop).await.map_err(|error| {
                    tracing::error!(%error,"valuation service stopped");
                })
            })?;

            ensure!(!supervisor.is_stopping(), "Application is stopping");
            Ok::<_, anyhow::Error>((handle, registry))
        }
        .await;
        let (handle, valuation) = match services {
            Ok(services) => services,
            Err(error) => {
                let _ = supervisor.shutdown().await;
                reads.shutdown().await;
                tokio::task::spawn_blocking(move || drop(market.replication)).await?;
                return Err(error);
            }
        };
        cancel.disarm();
        Ok(Self {
            handle,
            reads,
            valuation,
            supervisor,
            replication: Mutex::new(Some(market.replication)),
            #[cfg(feature = "bot")]
            portfolio: std::sync::RwLock::new(None),
            #[cfg(feature = "bot")]
            invocations: Mutex::new(None),
        })
    }
    pub fn handle(&self) -> MarketDataHandle {
        self.handle.clone()
    }
    pub fn valuation(&self) -> market_valuation::ValuationHandle {
        self.valuation.clone()
    }
    #[cfg(feature = "bot")]
    pub fn start_accounts(&self, worker: market_telegram_client::AccountWorker) -> Result<()> {
        self.valuation
            .set_gift_resolver(Arc::new(worker.resolver()))?;
        let stop = self.supervisor.token();
        self.supervisor
            .spawn_optional(TaskKind::TelegramAccounts, async move {
                worker.run(stop).await.map_err(|error| {
                    tracing::error!(%error, "Telegram gift lookup service stopped");
                })
            })
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
        let invocations = self.invocations()?;
        let mut bot = market_telegram_bot::BotService::new(worker, data, valuation);
        if let Some(portfolio) = self.portfolio() {
            bot = bot.with_portfolio(portfolio);
        }
        self.supervisor.spawn_optional(TaskKind::Bot, async move {
            super::worker::run("market-bot", stop, move |stop| async move {
                bot.run_with_invocations(stop, invocations)
                    .await
                    .map_err(|error| {
                        tracing::error!(%error, "bot worker stopped");
                    })
            })
            .await
        })?;
        Ok(())
    }

    #[cfg(feature = "bot")]
    fn invocations(&self) -> Result<market_invocations::Handle> {
        let mut current = self
            .invocations
            .lock()
            .map_err(|_| anyhow!("Invocation runtime unavailable"))?;
        if current.is_none() {
            *current = Some(
                market_invocations::Runtime::start(Default::default())
                    .map_err(|e| anyhow!("Invocation runtime: {e:?}"))?,
            );
        }
        Ok(current.as_ref().unwrap().handle())
    }
    #[cfg(feature = "bot")]
    pub fn portfolio(&self) -> Option<market_portfolio::PortfolioHandle> {
        self.portfolio.read().ok()?.clone()
    }
    #[cfg(feature = "bot")]
    pub async fn start_portfolio(
        &self,
        client: market_telegram_client::InventoryClient,
    ) -> Result<()> {
        struct Provider(market_telegram_client::InventoryClient);
        impl market_portfolio::InventoryProvider for Provider {
            fn page(
                &self,
                request: market_portfolio::InventoryPageRequest,
            ) -> impl market_invocations::Invocation<
                Output = market_portfolio::InventoryPage,
                Error = market_portfolio::InventoryError,
            > {
                self.0.page_operation(
                    request.user.get(),
                    request.cursor.unwrap_or_default(),
                    request.limit,
                )
            }
        }
        let service = market_portfolio::PortfolioService::start(
            Provider(client),
            self.valuation(),
            Default::default(),
            self.invocations()?,
        )
        .await?;
        let handle = service.handle.clone();
        let stop = self.supervisor.token();
        self.supervisor
            .spawn_optional(TaskKind::Portfolio, async move {
                service
                    .task
                    .run(stop)
                    .await
                    .map_err(|e| tracing::warn!(%e,"Portfolio service stopped"))
            })?;
        *self
            .portfolio
            .write()
            .map_err(|_| anyhow!("Portfolio state unavailable"))? = Some(handle);
        Ok(())
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
    pub fn cancellation_token(&self) -> tokio_util::sync::CancellationToken {
        self.supervisor.token()
    }
    pub async fn cancelled(&self) {
        self.supervisor.cancelled().await;
    }
    pub async fn shutdown(&self) -> Result<()> {
        let result = self.supervisor.shutdown().await;
        let unused = self
            .replication
            .lock()
            .map_err(|_| anyhow!("Replication worker state is unavailable"))?
            .take();
        tokio::task::spawn_blocking(move || drop(unused)).await?;
        #[cfg(feature = "bot")]
        {
            let invocations = self
                .invocations
                .lock()
                .map_err(|_| anyhow!("Invocation runtime unavailable"))?
                .take();
            if let Some(invocations) = invocations {
                invocations.shutdown().await;
            }
        }
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
    async fn shutdown_before_replication_drains_services_and_preserves_storage() {
        let directory = tempfile::tempdir().unwrap();
        let mut writer = market_replica::ReplicaWriter::open(directory.path(), "test").unwrap();
        let snapshot = writer.begin_snapshot().unwrap().finish("saved").unwrap();
        writer.install_snapshot(snapshot).unwrap();
        drop(writer);
        let runtime = MarketRuntime::open(directory.path(), "test", 1, 1)
            .await
            .unwrap();
        let handle = runtime.handle();
        runtime.shutdown().await.unwrap();
        assert!(runtime.replication.lock().unwrap().is_none());
        assert!(handle.status().await.is_err());
        drop(runtime);
        let reopened = market_replica::ReplicaWriter::open(directory.path(), "test").unwrap();
        assert_eq!(reopened.cursor().unwrap(), "saved");
    }
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
