use super::*;
impl MarketReader {
    /// Dropping the future signals cancellation. CPU work owns its permits until it exits.
    /// The callback owns its result/error type; only execution failures are data errors.
    pub async fn prepare<T: Send + 'static>(
        &self,
        work: impl FnOnce(MarketSnapshot) -> T + Send + 'static,
    ) -> Result<T> {
        let _admission = self.environment.jobs.token();
        let cancelled = self.environment.stop.child_token();
        let _cancel = cancelled.clone().drop_guard();
        let heavy = tokio::select! {biased;
            _=cancelled.cancelled()=>return Err(DataError::unavailable("Market data service is stopping")),
            permit=self.workers.clone().acquire_owned()=>permit.map_err(|_|DataError::unavailable("Read workers closed"))?,
        };
        let permit = self.budget.wait().await?;
        let snapshot = MarketSnapshot {
            reader: self.reader.snapshot()?,
            cancelled,
        };
        self.environment
            .jobs
            .spawn_blocking_on(
                move || {
                    let (_heavy, _permit) = (heavy, permit);
                    work(snapshot)
                },
                &self.environment.runtime,
            )
            .await
            .map_err(|_| DataError::unavailable("Market read worker stopped"))
    }
}
