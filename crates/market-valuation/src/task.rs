//! The application supervises both typed valuation interfaces as one service lifecycle.
use crate::{ServiceError, Task};
use tokio_util::sync::CancellationToken;
pub struct ValuationTask {
    pub(crate) gift: Task,
    pub(crate) targets: Task,
}
impl ValuationTask {
    pub async fn shutdown(self) -> Result<(), ServiceError> {
        let (gift, targets) = tokio::join!(self.gift.shutdown(), self.targets.shutdown());
        gift.and(targets)
    }
    pub async fn run(self, stop: CancellationToken) -> Result<(), ServiceError> {
        let stop = stop.child_token();
        let gift = self.gift.run(stop.clone());
        let targets = self.targets.run(stop.clone());
        tokio::pin!(gift, targets);
        tokio::select! {
            result=&mut gift=>{stop.cancel(); result.and(targets.await)},
            result=&mut targets=>{stop.cancel(); result.and(gift.await)},
        }
    }
}
