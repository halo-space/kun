use std::sync::atomic::{AtomicU64, Ordering};

/// Immutable runtime stats snapshot.
///
/// These counters are cumulative for the lifetime of one engine instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub request_count: u64,
    pub response_count: u64,
    pub error_count: u64,
    pub retry_count: u64,
    pub item_count: u64,
    pub pipeline_drop_count: u64,
}

#[derive(Debug, Default)]
pub(crate) struct Tracker {
    request_count: AtomicU64,
    response_count: AtomicU64,
    error_count: AtomicU64,
    retry_count: AtomicU64,
    item_count: AtomicU64,
    pipeline_drop_count: AtomicU64,
}

impl Tracker {
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            request_count: self.request_count.load(Ordering::Relaxed),
            response_count: self.response_count.load(Ordering::Relaxed),
            error_count: self.error_count.load(Ordering::Relaxed),
            retry_count: self.retry_count.load(Ordering::Relaxed),
            item_count: self.item_count.load(Ordering::Relaxed),
            pipeline_drop_count: self.pipeline_drop_count.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn record_request(&self) {
        self.request_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_response(&self) {
        self.response_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_retry(&self) {
        self.retry_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_item(&self) {
        self.item_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_pipeline_drop(&self) {
        self.pipeline_drop_count.fetch_add(1, Ordering::Relaxed);
    }
}
