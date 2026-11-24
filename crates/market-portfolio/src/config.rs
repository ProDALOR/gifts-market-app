use std::time::Duration;
#[derive(Debug, Clone)]
pub struct PortfolioConfig {
    pub max_requests: usize,
    pub cached_users: usize,
    pub refresh_interval: Duration,
    pub request_timeout: Duration,
    pub valuation_idle_timeout: Duration,
    pub page_size: usize,
    pub max_gifts: usize,
    pub max_pages: usize,
}
impl Default for PortfolioConfig {
    fn default() -> Self {
        Self {
            max_requests: 128,
            cached_users: 64,
            refresh_interval: Duration::from_secs(60),
            request_timeout: Duration::from_secs(60),
            valuation_idle_timeout: Duration::from_secs(60),
            page_size: 100,
            max_gifts: 10000,
            max_pages: 100,
        }
    }
}
impl PortfolioConfig {
    pub(crate) fn validate(&self) -> Result<(), crate::PortfolioError> {
        if self.max_requests == 0
            || self.max_requests > 4096
            || self.cached_users == 0
            || self.cached_users > 256
            || self.page_size == 0
            || self.page_size > 100
            || self.max_gifts < self.page_size
            || self.max_gifts > 10000
            || self.max_pages == 0
            || self.max_pages > 1000
            || self.refresh_interval.is_zero()
            || self.request_timeout.is_zero()
            || self.valuation_idle_timeout.is_zero()
            || self.refresh_interval > Duration::from_secs(86400)
            || self.request_timeout > Duration::from_secs(300)
            || self.valuation_idle_timeout > Duration::from_secs(300)
        {
            return Err(crate::PortfolioError::InvalidConfig(
                "Invalid portfolio limits",
            ));
        }
        Ok(())
    }
}
