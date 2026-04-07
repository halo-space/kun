use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
    pub dedup_reject_count: u64,
    pub robots_disallow_count: u64,
    pub robots_delay_count: u64,
    pub http_cache_hit_count: u64,
    pub http_cache_revalidate_count: u64,
    pub http_cache_store_count: u64,
    pub http_cache_miss_count: u64,
    pub store_error_count: u64,
    pub scheduler_claim_count: u64,
    pub scheduler_complete_count: u64,
    pub scheduler_requeue_count: u64,
    pub scheduler_heartbeat_count: u64,
    pub scheduler_lease_lost_count: u64,
}

/// Event emitted whenever one runtime stats counter is updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Request,
    Response,
    Error,
    Retry,
    Item,
    PipelineDrop,
    DedupReject,
    RobotsDisallow,
    RobotsDelay,
    HttpCacheHit,
    HttpCacheRevalidate,
    HttpCacheStore,
    HttpCacheMiss,
    StoreError,
    SchedulerClaim,
    SchedulerComplete,
    SchedulerRequeue,
    SchedulerHeartbeat,
    SchedulerLeaseLost,
}

impl Event {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Response => "response",
            Self::Error => "error",
            Self::Retry => "retry",
            Self::Item => "item",
            Self::PipelineDrop => "pipeline_drop",
            Self::DedupReject => "dedup_reject",
            Self::RobotsDisallow => "robots_disallow",
            Self::RobotsDelay => "robots_delay",
            Self::HttpCacheHit => "http_cache_hit",
            Self::HttpCacheRevalidate => "http_cache_revalidate",
            Self::HttpCacheStore => "http_cache_store",
            Self::HttpCacheMiss => "http_cache_miss",
            Self::StoreError => "store_error",
            Self::SchedulerClaim => "scheduler_claim",
            Self::SchedulerComplete => "scheduler_complete",
            Self::SchedulerRequeue => "scheduler_requeue",
            Self::SchedulerHeartbeat => "scheduler_heartbeat",
            Self::SchedulerLeaseLost => "scheduler_lease_lost",
        }
    }
}

/// Lightweight hook for custom stats reporters or exporters.
///
/// `engine.stats()` remains the primary read API. Reporters are a minimal
/// observation boundary so future telemetry integrations can subscribe to
/// counter updates without changing the current stats surface.
pub trait Reporter: Send + Sync {
    fn report(&self, event: Event, snapshot: Snapshot);
}

#[derive(Default)]
pub(crate) struct Tracker {
    request_count: AtomicU64,
    response_count: AtomicU64,
    error_count: AtomicU64,
    retry_count: AtomicU64,
    item_count: AtomicU64,
    pipeline_drop_count: AtomicU64,
    dedup_reject_count: AtomicU64,
    robots_disallow_count: AtomicU64,
    robots_delay_count: AtomicU64,
    http_cache_hit_count: AtomicU64,
    http_cache_revalidate_count: AtomicU64,
    http_cache_store_count: AtomicU64,
    http_cache_miss_count: AtomicU64,
    store_error_count: AtomicU64,
    scheduler_claim_count: AtomicU64,
    scheduler_complete_count: AtomicU64,
    scheduler_requeue_count: AtomicU64,
    scheduler_heartbeat_count: AtomicU64,
    scheduler_lease_lost_count: AtomicU64,
    reporters: Mutex<Vec<Arc<dyn Reporter>>>,
}

impl std::fmt::Debug for Tracker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Tracker")
            .field("snapshot", &self.snapshot())
            .finish_non_exhaustive()
    }
}

impl Tracker {
    pub(crate) fn add_reporter(&self, reporter: Arc<dyn Reporter>) {
        if let Ok(mut reporters) = self.reporters.lock() {
            reporters.push(reporter);
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            request_count: self.request_count.load(Ordering::Relaxed),
            response_count: self.response_count.load(Ordering::Relaxed),
            error_count: self.error_count.load(Ordering::Relaxed),
            retry_count: self.retry_count.load(Ordering::Relaxed),
            item_count: self.item_count.load(Ordering::Relaxed),
            pipeline_drop_count: self.pipeline_drop_count.load(Ordering::Relaxed),
            dedup_reject_count: self.dedup_reject_count.load(Ordering::Relaxed),
            robots_disallow_count: self.robots_disallow_count.load(Ordering::Relaxed),
            robots_delay_count: self.robots_delay_count.load(Ordering::Relaxed),
            http_cache_hit_count: self.http_cache_hit_count.load(Ordering::Relaxed),
            http_cache_revalidate_count: self.http_cache_revalidate_count.load(Ordering::Relaxed),
            http_cache_store_count: self.http_cache_store_count.load(Ordering::Relaxed),
            http_cache_miss_count: self.http_cache_miss_count.load(Ordering::Relaxed),
            store_error_count: self.store_error_count.load(Ordering::Relaxed),
            scheduler_claim_count: self.scheduler_claim_count.load(Ordering::Relaxed),
            scheduler_complete_count: self.scheduler_complete_count.load(Ordering::Relaxed),
            scheduler_requeue_count: self.scheduler_requeue_count.load(Ordering::Relaxed),
            scheduler_heartbeat_count: self.scheduler_heartbeat_count.load(Ordering::Relaxed),
            scheduler_lease_lost_count: self.scheduler_lease_lost_count.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn record_request(&self) {
        self.request_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::Request);
    }

    pub(crate) fn record_response(&self) {
        self.response_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::Response);
    }

    pub(crate) fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::Error);
    }

    pub(crate) fn record_retry(&self) {
        self.retry_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::Retry);
    }

    pub(crate) fn record_item(&self) {
        self.item_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::Item);
    }

    pub(crate) fn record_pipeline_drop(&self) {
        self.pipeline_drop_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::PipelineDrop);
    }

    pub(crate) fn record_dedup_reject(&self) {
        self.dedup_reject_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::DedupReject);
    }

    pub(crate) fn record_robots_disallow(&self) {
        self.robots_disallow_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::RobotsDisallow);
    }

    pub(crate) fn record_robots_delay(&self) {
        self.robots_delay_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::RobotsDelay);
    }

    pub(crate) fn record_http_cache_hit(&self) {
        self.http_cache_hit_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::HttpCacheHit);
    }

    pub(crate) fn record_http_cache_revalidate(&self) {
        self.http_cache_revalidate_count
            .fetch_add(1, Ordering::Relaxed);
        self.notify(Event::HttpCacheRevalidate);
    }

    pub(crate) fn record_http_cache_store(&self) {
        self.http_cache_store_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::HttpCacheStore);
    }

    pub(crate) fn record_http_cache_miss(&self) {
        self.http_cache_miss_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::HttpCacheMiss);
    }

    pub(crate) fn record_store_error(&self) {
        self.store_error_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::StoreError);
    }

    pub(crate) fn record_scheduler_claim(&self) {
        self.scheduler_claim_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::SchedulerClaim);
    }

    pub(crate) fn record_scheduler_complete(&self) {
        self.scheduler_complete_count
            .fetch_add(1, Ordering::Relaxed);
        self.notify(Event::SchedulerComplete);
    }

    pub(crate) fn record_scheduler_requeue(&self) {
        self.scheduler_requeue_count.fetch_add(1, Ordering::Relaxed);
        self.notify(Event::SchedulerRequeue);
    }

    pub(crate) fn record_scheduler_heartbeat(&self) {
        self.scheduler_heartbeat_count
            .fetch_add(1, Ordering::Relaxed);
        self.notify(Event::SchedulerHeartbeat);
    }

    pub(crate) fn record_scheduler_lease_lost(&self) {
        self.scheduler_lease_lost_count
            .fetch_add(1, Ordering::Relaxed);
        self.notify(Event::SchedulerLeaseLost);
    }

    fn notify(&self, event: Event) {
        let snapshot = self.snapshot();
        let reporters = match self.reporters.lock() {
            Ok(reporters) => reporters.clone(),
            Err(_) => return,
        };

        for reporter in reporters {
            reporter.report(event, snapshot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingReporter {
        events: Mutex<Vec<(Event, Snapshot)>>,
    }

    impl Reporter for RecordingReporter {
        fn report(&self, event: Event, snapshot: Snapshot) {
            self.events.lock().unwrap().push((event, snapshot));
        }
    }

    #[test]
    fn tracker_snapshot_includes_granular_counters() {
        let tracker = Tracker::default();

        tracker.record_dedup_reject();
        tracker.record_robots_disallow();
        tracker.record_robots_delay();
        tracker.record_http_cache_hit();
        tracker.record_http_cache_revalidate();
        tracker.record_http_cache_store();
        tracker.record_http_cache_miss();
        tracker.record_store_error();
        tracker.record_scheduler_claim();
        tracker.record_scheduler_complete();
        tracker.record_scheduler_requeue();
        tracker.record_scheduler_heartbeat();
        tracker.record_scheduler_lease_lost();

        assert_eq!(
            tracker.snapshot(),
            Snapshot {
                dedup_reject_count: 1,
                robots_disallow_count: 1,
                robots_delay_count: 1,
                http_cache_hit_count: 1,
                http_cache_revalidate_count: 1,
                http_cache_store_count: 1,
                http_cache_miss_count: 1,
                store_error_count: 1,
                scheduler_claim_count: 1,
                scheduler_complete_count: 1,
                scheduler_requeue_count: 1,
                scheduler_heartbeat_count: 1,
                scheduler_lease_lost_count: 1,
                ..Snapshot::default()
            }
        );
    }

    #[test]
    fn tracker_notifies_registered_reporters() {
        let tracker = Tracker::default();
        let reporter = Arc::new(RecordingReporter::default());
        tracker.add_reporter(reporter.clone());

        tracker.record_request();
        tracker.record_http_cache_hit();
        tracker.record_http_cache_store();

        assert_eq!(
            reporter.events.lock().unwrap().clone(),
            vec![
                (
                    Event::Request,
                    Snapshot {
                        request_count: 1,
                        ..Snapshot::default()
                    },
                ),
                (
                    Event::HttpCacheHit,
                    Snapshot {
                        request_count: 1,
                        http_cache_hit_count: 1,
                        ..Snapshot::default()
                    },
                ),
                (
                    Event::HttpCacheStore,
                    Snapshot {
                        request_count: 1,
                        http_cache_hit_count: 1,
                        http_cache_store_count: 1,
                        ..Snapshot::default()
                    },
                ),
            ]
        );
    }
}
