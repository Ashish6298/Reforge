use crate::digest::CacheKey;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    #[serde(rename = "cache.lookup")]
    CacheLookup,
    #[serde(rename = "cache.hit")]
    CacheHit,
    #[serde(rename = "cache.miss")]
    CacheMiss,
    #[serde(rename = "cache.store")]
    CacheStore,
    #[serde(rename = "cache.restore")]
    CacheRestore,
    #[serde(rename = "cache.delete")]
    CacheDelete,
    #[serde(rename = "cache.verify")]
    CacheVerify,
    #[serde(rename = "computation.start")]
    ComputationStart,
    #[serde(rename = "computation.finish")]
    ComputationFinish,
    #[serde(rename = "computation.failed")]
    ComputationFailed,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CacheLookup => "cache.lookup",
            Self::CacheHit => "cache.hit",
            Self::CacheMiss => "cache.miss",
            Self::CacheStore => "cache.store",
            Self::CacheRestore => "cache.restore",
            Self::CacheDelete => "cache.delete",
            Self::CacheVerify => "cache.verify",
            Self::ComputationStart => "computation.start",
            Self::ComputationFinish => "computation.finish",
            Self::ComputationFailed => "computation.failed",
        }
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredEvent {
    pub timestamp: DateTime<Utc>,
    pub event: EventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<CacheKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

impl StructuredEvent {
    pub fn new(event: EventKind) -> Self {
        Self {
            timestamp: Utc::now(),
            event,
            key: None,
            operation: None,
            duration_ms: None,
            message: None,
            fields: BTreeMap::new(),
        }
    }

    pub fn with_key(mut self, key: CacheKey) -> Self {
        self.key = Some(key);
        self
    }

    pub fn with_operation(mut self, op: impl Into<String>) -> Self {
        self.operation = Some(op.into());
        self
    }

    pub fn with_duration_ms(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    pub fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.message = Some(msg.into());
        self
    }

    pub fn with_field(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.fields.insert(key.into(), val.into());
        self
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

pub trait EventSubscriber: Send + Sync {
    fn on_event(&self, event: &StructuredEvent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::Digest;

    #[test]
    fn test_event_formatting() {
        let digest = Digest::from_bytes(b"test");
        let key = CacheKey::new(digest);

        let event = StructuredEvent::new(EventKind::CacheHit)
            .with_key(key)
            .with_operation("codegen")
            .with_duration_ms(15)
            .with_message("Restored 3 outputs");

        assert_eq!(event.event.as_str(), "cache.hit");
        let json = event.to_json();
        assert!(json.contains("cache.hit"));
        assert!(json.contains("codegen"));
    }
}
