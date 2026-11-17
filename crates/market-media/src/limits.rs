use std::time::Duration;
/// All cache and download budgets; defaults suit one embedded media provider.
#[derive(Clone, Debug)]
pub struct MediaLimits {
    pub memory_bytes: usize,
    pub memory_ttl: Duration,
    pub disk_bytes: u64,
    pub temporary_ttl: Duration,
    pub queue: usize,
    pub downloads: usize,
    pub request_timeout: Duration,
    pub download_timeout: Duration,
    pub start_interval: Duration,
}
impl Default for MediaLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 16 * 1024 * 1024,
            memory_ttl: Duration::from_secs(300),
            disk_bytes: 512 * 1024 * 1024,
            temporary_ttl: Duration::from_secs(3600),
            queue: 64,
            downloads: 4,
            request_timeout: Duration::from_secs(30),
            download_timeout: Duration::from_secs(15),
            start_interval: Duration::from_millis(75),
        }
    }
}
impl MediaLimits {
    pub(crate) fn validate(&self) -> std::io::Result<()> {
        if self.memory_bytes == 0
            || self.disk_bytes == 0
            || self.queue == 0
            || self.downloads == 0
            || self.downloads > self.queue
            || self.memory_ttl.is_zero()
            || self.temporary_ttl.is_zero()
            || self.download_timeout.is_zero()
            || self.request_timeout <= self.download_timeout
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid media limits",
            ));
        }
        Ok(())
    }
}
