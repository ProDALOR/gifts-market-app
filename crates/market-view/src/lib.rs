//! Application-owned read projections. Core storage and ingestion rules are private to core.
pub mod attributes;
pub mod catalog;
pub mod primitives;
pub mod protocol;
pub mod records;
pub use attributes::*;
pub use catalog::*;
pub use jiff::SignedDuration;
pub use primitives::*;
pub use records::*;
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid market view: {0}")]
pub struct ValidationError(pub &'static str);
pub mod time {
    pub use crate::MarketTimestamp;
    pub mod duration {
        pub mod option {
            use std::time::Duration;
            pub fn serialize<S: serde::Serializer>(
                v: &Option<Duration>,
                s: S,
            ) -> Result<S::Ok, S::Error> {
                use serde::Serialize;
                v.map(|v| humantime::format_duration(v).to_string())
                    .serialize(s)
            }
            pub fn deserialize<'de, D: serde::Deserializer<'de>>(
                d: D,
            ) -> Result<Option<Duration>, D::Error> {
                use serde::Deserialize;
                Option::<String>::deserialize(d)?
                    .map(|s| humantime::parse_duration(&s))
                    .transpose()
                    .map_err(serde::de::Error::custom)
            }
        }

        use std::time::Duration;
        pub fn serialize<S: serde::Serializer>(v: &Duration, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&humantime::format_duration(*v).to_string())
        }
        pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
            use serde::Deserialize;
            humantime::parse_duration(&String::deserialize(d)?).map_err(serde::de::Error::custom)
        }
    }
}
pub mod numbers {
    pub mod u64_string {
        pub fn serialize<S: serde::Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&v.to_string())
        }
        pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
            use serde::Deserialize;
            String::deserialize(d)?
                .parse()
                .map_err(serde::de::Error::custom)
        }
    }
}
