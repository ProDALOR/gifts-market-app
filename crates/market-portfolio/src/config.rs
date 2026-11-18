use crate::PortfolioError;
use std::time::Duration;
#[derive(Debug, Clone)]
pub struct PortfolioConfig {
    pub max_requests: usize,
    pub cached_users: usize,
    pub inventory_ttl: Duration,
    pub request_timeout: Duration,
    pub valuation_timeout: Duration,
    pub page_size: usize,
    pub max_gifts: usize,
    pub max_pages: usize,
}
impl Default for PortfolioConfig {
    fn default() -> Self {
        Self {
            max_requests: 128,
            cached_users: 64,
            inventory_ttl: Duration::from_secs(30),
            request_timeout: Duration::from_secs(60),
            valuation_timeout: Duration::from_secs(10),
            page_size: 100,
            max_gifts: 1000,
            max_pages: 20,
        }
    }
}
impl PortfolioConfig {
    pub(crate) fn validate(&self) -> Result<(), PortfolioError> {
        if !(1..=4096).contains(&self.max_requests)
            || self.cached_users > 4096
            || self.max_pages > 1000
            || self.page_size == 0
            || self.page_size > self.max_gifts
            || self.max_gifts > 1000
            || self.max_pages == 0
        {
            return Err(PortfolioError::InvalidConfig(
                "positive request/page limits required; page_size <= max_gifts <= 1000",
            ));
        }
        if self.inventory_ttl.is_zero()
            || self.request_timeout.is_zero()
            || self.valuation_timeout.is_zero()
            || self.valuation_timeout > self.request_timeout
            || self.request_timeout > Duration::from_secs(3600)
            || self.inventory_ttl > Duration::from_secs(3600)
        {
            return Err(PortfolioError::InvalidConfig(
                "positive durations up to one hour required; valuation_timeout <= request_timeout",
            ));
        }
        Ok(())
    }
}
