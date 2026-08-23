use serde::{Deserialize, Serialize};

pub type TokenId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeSpan {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl TimeSpan {
    pub fn new(start_ms: u64, end_ms: u64) -> Self {
        Self { start_ms, end_ms: end_ms.max(start_ms) }
    }

    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }

    pub fn contains_ms(&self, timestamp_ms: u64) -> bool {
        timestamp_ms >= self.start_ms && timestamp_ms <= self.end_ms
    }

    pub fn merge(&self, other: &TimeSpan) -> Self {
        Self {
            start_ms: self.start_ms.min(other.start_ms),
            end_ms: self.end_ms.max(other.end_ms),
        }
    }
}
