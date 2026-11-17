use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ErrorDisposition {
    Retryable,
    Rejected,
    Fatal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotError {
    Configuration(&'static str),
    Transport,
    Session,
    Authorization,
    Runner,
    InvalidResponse,
    Rpc(i32),
    PeerUnavailable,
    RetryAfter(u64),
}
impl std::fmt::Display for BotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration(message) => f.write_str(message),
            Self::Session => f.write_str("Bot session storage unavailable"),
            Self::Authorization => f.write_str("Telegram bot authorization failed"),
            Self::Runner => f.write_str("Telegram connection runner stopped"),
            Self::Transport => f.write_str("Telegram connection failed"),
            Self::InvalidResponse => f.write_str("Invalid MTProto response"),
            Self::PeerUnavailable => f.write_str("Telegram peer cannot receive messages"),
            Self::Rpc(code) => write!(f, "Telegram rejected request ({code})"),
            Self::RetryAfter(seconds) => write!(f, "Telegram rate limit; retry in {seconds}s"),
        }
    }
}
impl std::error::Error for BotError {}
impl BotError {
    pub(super) fn disposition(self) -> ErrorDisposition {
        match self {
            Self::Configuration(_) | Self::Session | Self::Authorization | Self::Runner => {
                ErrorDisposition::Fatal
            }
            Self::PeerUnavailable | Self::InvalidResponse | Self::Rpc(..=499) => {
                ErrorDisposition::Rejected
            }
            _ => ErrorDisposition::Retryable,
        }
    }
    pub(super) fn fatal(self) -> bool {
        self.disposition() == ErrorDisposition::Fatal
    }
    pub fn delay(&self, fallback: u64) -> Duration {
        Duration::from_secs(match self {
            Self::RetryAfter(seconds) => (*seconds).max(1),
            _ => fallback,
        })
    }
}
