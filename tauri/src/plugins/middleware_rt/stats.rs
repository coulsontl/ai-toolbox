//! Per-middleware counters, shared by the gateway and the settings pane.
//!
//! Every counter is atomic and owned by one middleware for its whole life, so
//! reading a view never blocks a request and a reload keeps the previous
//! numbers for modules that are still installed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::Serialize;

#[derive(Default)]
pub struct MiddlewareStats {
    calls: AtomicU64,
    failures: AtomicU64,
    total_micros: AtomicU64,
    usage_rows: AtomicU64,
    usage_dropped: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl MiddlewareStats {
    pub fn record_call(&self, micros: u64) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.total_micros.fetch_add(micros, Ordering::Relaxed);
    }

    pub fn record_failure(&self, message: &str) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        *self
            .last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(message.to_string());
    }

    pub fn record_usage_row(&self) {
        self.usage_rows.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_usage_dropped(&self) {
        self.usage_dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }

    pub fn failures(&self) -> u64 {
        self.failures.load(Ordering::Relaxed)
    }

    pub fn average_micros(&self) -> u64 {
        let calls = self.calls.load(Ordering::Relaxed);
        if calls == 0 {
            0
        } else {
            self.total_micros.load(Ordering::Relaxed) / calls
        }
    }

    pub fn usage_rows(&self) -> u64 {
        self.usage_rows.load(Ordering::Relaxed)
    }

    pub fn usage_dropped(&self) -> u64 {
        self.usage_dropped.load(Ordering::Relaxed)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// One row of the middleware list the settings pane renders.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiddlewareStateView {
    pub plugin_id: String,
    pub spec: String,
    pub name: String,
    pub path: String,
    pub hooks: Vec<String>,
    /// `None` means "every event"; an empty list means no event is wanted.
    pub events: Option<Vec<String>>,
    pub calls: u64,
    pub failures: u64,
    pub avg_micros: u64,
    pub last_error: Option<String>,
    pub load_error: Option<String>,
    pub usage_rows: u64,
    pub usage_dropped: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_accumulate_and_report_an_average() {
        let stats = MiddlewareStats::default();
        assert_eq!(stats.average_micros(), 0);
        stats.record_call(100);
        stats.record_call(300);
        assert_eq!(stats.calls(), 2);
        assert_eq!(stats.average_micros(), 200);
        stats.record_failure("boom");
        assert_eq!(stats.failures(), 1);
        assert_eq!(stats.last_error().as_deref(), Some("boom"));
        stats.record_usage_row();
        stats.record_usage_dropped();
        assert_eq!(stats.usage_rows(), 1);
        assert_eq!(stats.usage_dropped(), 1);
    }
}
