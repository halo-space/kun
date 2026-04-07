use crate::error::SpiderError;
use crate::scheduler::control::Control;
use crate::scheduler::{ClaimedTask, Scheduler, TaskId, TaskLease};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEventKind {
    Claimed,
    Completed,
    Requeued,
    Heartbeat,
    LeaseLost,
    Reclaimed,
    Released,
    Closed,
}

impl RuntimeEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::Completed => "completed",
            Self::Requeued => "requeued",
            Self::Heartbeat => "heartbeat",
            Self::LeaseLost => "lease_lost",
            Self::Reclaimed => "reclaimed",
            Self::Released => "released",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEvent {
    pub scope: Option<String>,
    pub kind: RuntimeEventKind,
    pub worker_id: Option<String>,
    pub task_id: Option<TaskId>,
    pub lease_id: Option<String>,
    pub url: Option<String>,
    pub count: usize,
    pub error: Option<SpiderError>,
}

impl RuntimeEvent {
    pub fn claimed(scope: Option<String>, task: &ClaimedTask) -> Self {
        Self {
            scope,
            kind: RuntimeEventKind::Claimed,
            worker_id: Some(task.lease.worker_id().to_string()),
            task_id: Some(task.lease.task_id().clone()),
            lease_id: Some(task.lease.lease_id().to_string()),
            url: Some(task.task.request.url.clone()),
            count: 1,
            error: None,
        }
    }

    pub fn lease(
        scope: Option<String>,
        kind: RuntimeEventKind,
        lease: &TaskLease,
        url: Option<String>,
        error: Option<SpiderError>,
    ) -> Self {
        Self {
            scope,
            kind,
            worker_id: Some(lease.worker_id().to_string()),
            task_id: Some(lease.task_id().clone()),
            lease_id: Some(lease.lease_id().to_string()),
            url,
            count: 1,
            error,
        }
    }

    pub fn reclaimed(scope: Option<String>, count: usize) -> Self {
        Self {
            scope,
            kind: RuntimeEventKind::Reclaimed,
            worker_id: None,
            task_id: None,
            lease_id: None,
            url: None,
            count,
            error: None,
        }
    }

    pub fn released(scope: Option<String>, worker_id: Option<String>, count: usize) -> Self {
        Self {
            scope,
            kind: RuntimeEventKind::Released,
            worker_id,
            task_id: None,
            lease_id: None,
            url: None,
            count,
            error: None,
        }
    }

    pub fn closed(scope: Option<String>, worker_id: Option<String>) -> Self {
        Self {
            scope,
            kind: RuntimeEventKind::Closed,
            worker_id,
            task_id: None,
            lease_id: None,
            url: None,
            count: 1,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MetricCounts {
    pub claimed_total: u64,
    pub completed_total: u64,
    pub requeued_total: u64,
    pub heartbeat_total: u64,
    pub lease_lost_total: u64,
    pub reclaimed_total: u64,
    pub released_total: u64,
    pub closed_total: u64,
}

impl MetricCounts {
    fn record(&mut self, event: &RuntimeEvent) {
        let delta = u64::try_from(event.count.max(1)).unwrap_or(u64::MAX);
        match event.kind {
            RuntimeEventKind::Claimed => {
                self.claimed_total = self.claimed_total.saturating_add(delta);
            }
            RuntimeEventKind::Completed => {
                self.completed_total = self.completed_total.saturating_add(delta);
            }
            RuntimeEventKind::Requeued => {
                self.requeued_total = self.requeued_total.saturating_add(delta);
            }
            RuntimeEventKind::Heartbeat => {
                self.heartbeat_total = self.heartbeat_total.saturating_add(delta);
            }
            RuntimeEventKind::LeaseLost => {
                self.lease_lost_total = self.lease_lost_total.saturating_add(delta);
            }
            RuntimeEventKind::Reclaimed => {
                self.reclaimed_total = self.reclaimed_total.saturating_add(delta);
            }
            RuntimeEventKind::Released => {
                self.released_total = self.released_total.saturating_add(delta);
            }
            RuntimeEventKind::Closed => {
                self.closed_total = self.closed_total.saturating_add(delta);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorkerMetricsKey {
    pub scope: Option<String>,
    pub worker_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub totals: MetricCounts,
    pub scopes: BTreeMap<String, MetricCounts>,
    pub workers: BTreeMap<WorkerMetricsKey, MetricCounts>,
}

#[derive(Debug, Clone, Default)]
pub struct MetricsReporter {
    snapshot: Arc<Mutex<MetricsSnapshot>>,
}

impl MetricsReporter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        self.snapshot
            .lock()
            .map(|it| it.clone())
            .unwrap_or_default()
    }
}

pub trait RuntimeReporter: Send + Sync {
    fn report(&self, event: RuntimeEvent);
}

impl RuntimeReporter for MetricsReporter {
    fn report(&self, event: RuntimeEvent) {
        let Ok(mut snapshot) = self.snapshot.lock() else {
            return;
        };

        snapshot.totals.record(&event);

        if let Some(scope) = &event.scope {
            snapshot
                .scopes
                .entry(scope.clone())
                .or_default()
                .record(&event);
        }

        if let Some(worker_id) = &event.worker_id {
            snapshot
                .workers
                .entry(WorkerMetricsKey {
                    scope: event.scope.clone(),
                    worker_id: worker_id.clone(),
                })
                .or_default()
                .record(&event);
        }
    }
}

pub struct Observed<S> {
    inner: S,
    reporters: Mutex<Vec<Arc<dyn RuntimeReporter>>>,
}

impl<S> Observed<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            reporters: Mutex::new(Vec::new()),
        }
    }

    pub fn with_reporter(self, reporter: impl RuntimeReporter + 'static) -> Self {
        self.add_reporter(Arc::new(reporter));
        self
    }

    pub fn with_exporter(self, exporter: impl crate::telemetry::Exporter + 'static) -> Self {
        self.with_reporter(exporter)
    }

    pub fn add_reporter(&self, reporter: Arc<dyn RuntimeReporter>) {
        if let Ok(mut reporters) = self.reporters.lock() {
            reporters.push(reporter);
        }
    }

    pub fn inner(&self) -> &S {
        &self.inner
    }

    pub fn into_inner(self) -> S {
        self.inner
    }

    fn emit(&self, event: RuntimeEvent) {
        let reporters = match self.reporters.lock() {
            Ok(reporters) => reporters.clone(),
            Err(_) => return,
        };

        for reporter in reporters {
            reporter.report(event.clone());
        }
    }
}

impl<S> std::fmt::Debug for Observed<S>
where
    S: std::fmt::Debug,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Observed")
            .field("inner", &self.inner)
            .field(
                "reporter_count",
                &self.reporters.lock().map(|it| it.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl<S> Observed<S>
where
    S: Scheduler,
{
    fn emit_drained_runtime_events(&self) -> Vec<RuntimeEvent> {
        let events = self.inner.drain_runtime_events();
        for event in &events {
            self.emit(event.clone());
        }
        events
    }
}

impl<S> Scheduler for Observed<S>
where
    S: Scheduler,
{
    async fn enqueue(&self, task: crate::scheduler::Task) -> Result<(), SpiderError> {
        let result = self.inner.enqueue(task).await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn checkpoint(&self) -> Result<crate::scheduler::checkpoint::Checkpoint, SpiderError> {
        let result = self.inner.checkpoint().await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn counts(&self) -> Result<crate::scheduler::checkpoint::Counts, SpiderError> {
        let result = self.inner.counts().await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn snapshot(&self) -> Result<crate::scheduler::Snapshot, SpiderError> {
        let result = self.inner.snapshot().await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn scopes(&self) -> Result<Vec<String>, SpiderError> {
        let result = self.inner.scopes().await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn scopes_with_prefix(&self, prefix: &str) -> Result<Vec<String>, SpiderError> {
        let result = self.inner.scopes_with_prefix(prefix).await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn snapshots(&self) -> Result<Vec<crate::scheduler::Snapshot>, SpiderError> {
        let result = self.inner.snapshots().await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn snapshots_with_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<crate::scheduler::Snapshot>, SpiderError> {
        let result = self.inner.snapshots_with_prefix(prefix).await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn overview(&self) -> Result<crate::scheduler::Overview, SpiderError> {
        let result = self.inner.overview().await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn overview_with_prefix(
        &self,
        prefix: &str,
    ) -> Result<crate::scheduler::Overview, SpiderError> {
        let result = self.inner.overview_with_prefix(prefix).await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn take_ready(&self) -> Result<Option<ClaimedTask>, SpiderError> {
        let result = self.inner.take_ready().await;
        match &result {
            Ok(task) => {
                self.emit_drained_runtime_events();
                if let Some(task) = task {
                    self.emit(RuntimeEvent::claimed(self.inner.runtime_scope(), task));
                }
            }
            Err(_) => {}
        }
        result
    }

    async fn complete(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        let result = self.inner.complete(lease).await;
        self.emit_drained_runtime_events();
        match &result {
            Ok(()) => self.emit(RuntimeEvent::lease(
                self.inner.runtime_scope(),
                RuntimeEventKind::Completed,
                lease,
                None,
                None,
            )),
            Err(error) if error.is_scheduler_lease_resolution_error() => {
                self.emit(RuntimeEvent::lease(
                    self.inner.runtime_scope(),
                    RuntimeEventKind::LeaseLost,
                    lease,
                    None,
                    Some(error.clone()),
                ))
            }
            Err(_) => {}
        }
        result
    }

    async fn complete_and_enqueue(
        &self,
        lease: &TaskLease,
        tasks: Vec<crate::scheduler::Task>,
    ) -> Result<(), SpiderError> {
        let result = self.inner.complete_and_enqueue(lease, tasks).await;
        self.emit_drained_runtime_events();
        match &result {
            Ok(()) => self.emit(RuntimeEvent::lease(
                self.inner.runtime_scope(),
                RuntimeEventKind::Completed,
                lease,
                None,
                None,
            )),
            Err(error) if error.is_scheduler_lease_resolution_error() => {
                self.emit(RuntimeEvent::lease(
                    self.inner.runtime_scope(),
                    RuntimeEventKind::LeaseLost,
                    lease,
                    None,
                    Some(error.clone()),
                ))
            }
            Err(_) => {}
        }
        result
    }

    async fn requeue(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        let result = self.inner.requeue(lease).await;
        self.emit_drained_runtime_events();
        match &result {
            Ok(()) => self.emit(RuntimeEvent::lease(
                self.inner.runtime_scope(),
                RuntimeEventKind::Requeued,
                lease,
                None,
                None,
            )),
            Err(error) if error.is_scheduler_lease_resolution_error() => {
                self.emit(RuntimeEvent::lease(
                    self.inner.runtime_scope(),
                    RuntimeEventKind::LeaseLost,
                    lease,
                    None,
                    Some(error.clone()),
                ))
            }
            Err(_) => {}
        }
        result
    }

    async fn release_inflight(&self) -> Result<usize, SpiderError> {
        let result = self.inner.release_inflight().await;
        let events = self.emit_drained_runtime_events();
        if let Ok(released) = result.as_ref() {
            if *released > 0
                && !events
                    .iter()
                    .any(|event| event.kind == RuntimeEventKind::Released)
            {
                self.emit(RuntimeEvent::released(
                    self.inner.runtime_scope(),
                    self.inner.runtime_worker_id(),
                    *released,
                ));
            }
        }
        result
    }

    async fn heartbeat(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        let result = self.inner.heartbeat(lease).await;
        self.emit_drained_runtime_events();
        match &result {
            Ok(()) => self.emit(RuntimeEvent::lease(
                self.inner.runtime_scope(),
                RuntimeEventKind::Heartbeat,
                lease,
                None,
                None,
            )),
            Err(error) if error.is_scheduler_lease_resolution_error() => {
                self.emit(RuntimeEvent::lease(
                    self.inner.runtime_scope(),
                    RuntimeEventKind::LeaseLost,
                    lease,
                    None,
                    Some(error.clone()),
                ))
            }
            Err(_) => {}
        }
        result
    }

    fn heartbeat_interval(&self) -> Option<jiff::SignedDuration> {
        self.inner.heartbeat_interval()
    }

    async fn close(&self) -> Result<(), SpiderError> {
        let result = self.inner.close().await;
        let events = self.emit_drained_runtime_events();
        if result.is_ok()
            && !events
                .iter()
                .any(|event| event.kind == RuntimeEventKind::Closed)
        {
            self.emit(RuntimeEvent::closed(
                self.inner.runtime_scope(),
                self.inner.runtime_worker_id(),
            ));
        }
        result
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        let result = self.inner.has_pending().await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    fn runtime_scope(&self) -> Option<String> {
        self.inner.runtime_scope()
    }

    fn runtime_worker_id(&self) -> Option<String> {
        self.inner.runtime_worker_id()
    }

    fn drain_runtime_events(&self) -> Vec<RuntimeEvent> {
        self.inner.drain_runtime_events()
    }
}

impl<S> Control for Observed<S>
where
    S: Control + Scheduler,
{
    async fn pause_scope(&self, scope: &str) -> Result<bool, SpiderError> {
        let result = self.inner.pause_scope(scope).await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn resume_scope(&self, scope: &str) -> Result<bool, SpiderError> {
        let result = self.inner.resume_scope(scope).await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn release_scope(&self, scope: &str) -> Result<usize, SpiderError> {
        let result = self.inner.release_scope(scope).await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }

    async fn purge_scope(
        &self,
        scope: &str,
    ) -> Result<crate::scheduler::checkpoint::Counts, SpiderError> {
        let result = self.inner.purge_scope(scope).await;
        if result.is_ok() {
            self.emit_drained_runtime_events();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::scheduler::{Memory, Redis, Task, Worker};
    use crate::test_support::redis::spawn_redis_server;
    use jiff::SignedDuration;

    #[derive(Default)]
    struct RecordingReporter {
        events: Mutex<Vec<RuntimeEvent>>,
    }

    impl RuntimeReporter for RecordingReporter {
        fn report(&self, event: RuntimeEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn observed_scheduler_reports_claim_and_complete_events() {
        let scheduler = Observed::new(Memory::default().with_scope("jobs:observe"));
        let reporter = Arc::new(RecordingReporter::default());
        scheduler.add_reporter(reporter.clone());

        scheduler
            .enqueue(Task::new(Request::new("https://example.com/observe")))
            .await
            .unwrap();
        let claimed = scheduler.take_ready().await.unwrap().unwrap();
        scheduler.complete(&claimed.lease).await.unwrap();

        let events = reporter.events.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, RuntimeEventKind::Claimed);
        assert_eq!(events[0].scope.as_deref(), Some("jobs:observe"));
        assert_eq!(events[1].kind, RuntimeEventKind::Completed);
        assert_eq!(events[1].scope.as_deref(), Some("jobs:observe"));
    }

    #[tokio::test]
    async fn metrics_reporter_aggregates_claim_complete_and_close_counts() {
        let metrics = MetricsReporter::new();
        let scheduler = Observed::new(
            Memory::default()
                .with_scope("jobs:metrics")
                .with_worker(Worker::new("metrics-worker")),
        )
        .with_reporter(metrics.clone());

        scheduler
            .enqueue(Task::new(Request::new("https://example.com/metrics")))
            .await
            .unwrap();
        let claimed = scheduler.take_ready().await.unwrap().unwrap();
        scheduler.complete(&claimed.lease).await.unwrap();
        scheduler.close().await.unwrap();

        let snapshot = metrics.snapshot();
        assert_eq!(
            snapshot.totals,
            MetricCounts {
                claimed_total: 1,
                completed_total: 1,
                closed_total: 1,
                ..MetricCounts::default()
            }
        );
        assert_eq!(
            snapshot.scopes.get("jobs:metrics"),
            Some(&MetricCounts {
                claimed_total: 1,
                completed_total: 1,
                closed_total: 1,
                ..MetricCounts::default()
            })
        );
        assert_eq!(
            snapshot.workers.get(&WorkerMetricsKey {
                scope: Some("jobs:metrics".to_string()),
                worker_id: "metrics-worker".to_string(),
            }),
            Some(&MetricCounts {
                claimed_total: 1,
                completed_total: 1,
                closed_total: 1,
                ..MetricCounts::default()
            })
        );
    }

    #[tokio::test]
    async fn memory_scheduler_drains_release_and_close_runtime_events() {
        let scheduler = Memory::default()
            .with_scope("jobs:memory-runtime")
            .with_worker(Worker::new("memory-worker"));
        scheduler
            .enqueue(Task::new(Request::new("https://example.com/release")))
            .await
            .unwrap();
        let claimed = scheduler.take_ready().await.unwrap().unwrap();

        assert_eq!(scheduler.release_inflight().await.unwrap(), 1);
        scheduler.close().await.unwrap();

        let events = scheduler.drain_runtime_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, RuntimeEventKind::Released);
        assert_eq!(events[0].scope.as_deref(), Some("jobs:memory-runtime"));
        assert_eq!(events[0].worker_id.as_deref(), Some("memory-worker"));
        assert_eq!(events[0].count, 1);
        assert_eq!(events[1].kind, RuntimeEventKind::Closed);
        assert_eq!(events[1].scope.as_deref(), Some("jobs:memory-runtime"));
        assert_eq!(events[1].worker_id.as_deref(), Some("memory-worker"));

        let reclaimed = scheduler.take_ready().await.unwrap().unwrap();
        assert_eq!(claimed.task.id, reclaimed.task.id);
    }

    #[tokio::test]
    async fn observed_redis_scheduler_reports_reclaim_runtime_events() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "runtime_reclaim";
        let first = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-a").with_lease_timeout(SignedDuration::from_millis(20)),
        );
        first
            .enqueue(Task::new(Request::new("https://example.com/reclaim")))
            .await
            .unwrap();
        let claimed = first.take_ready().await.unwrap().unwrap();
        first.close().await.unwrap();

        std::thread::sleep(std::time::Duration::from_millis(40));

        let scheduler = Observed::new(Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-b").with_lease_timeout(SignedDuration::from_millis(20)),
        ));
        let reporter = Arc::new(RecordingReporter::default());
        scheduler.add_reporter(reporter.clone());

        let snapshot = scheduler.snapshot().await.unwrap();
        assert_eq!(snapshot.reclaimed_in_refresh, 1);

        let events = reporter.events.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, RuntimeEventKind::Reclaimed);
        assert_eq!(events[0].scope.as_deref(), Some(namespace));
        assert_eq!(events[0].count, 1);

        let reclaimed = scheduler.take_ready().await.unwrap().unwrap();
        scheduler.complete(&reclaimed.lease).await.unwrap();
        assert_eq!(reclaimed.task.id, claimed.task.id);

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn metrics_reporter_aggregates_backend_runtime_events() {
        let metrics = MetricsReporter::new();
        let scheduler = Observed::new(
            Memory::default()
                .with_scope("jobs:backend-metrics")
                .with_worker(Worker::new("backend-worker")),
        )
        .with_reporter(metrics.clone());

        scheduler
            .enqueue(Task::new(Request::new("https://example.com/backend")))
            .await
            .unwrap();
        let claimed = scheduler.take_ready().await.unwrap().unwrap();
        assert_eq!(scheduler.release_inflight().await.unwrap(), 1);
        scheduler.close().await.unwrap();

        let snapshot = metrics.snapshot();
        assert_eq!(
            snapshot.totals,
            MetricCounts {
                claimed_total: 1,
                released_total: 1,
                closed_total: 1,
                ..MetricCounts::default()
            }
        );
        assert_eq!(
            snapshot.scopes.get("jobs:backend-metrics"),
            Some(&MetricCounts {
                claimed_total: 1,
                released_total: 1,
                closed_total: 1,
                ..MetricCounts::default()
            })
        );
        assert_eq!(
            snapshot.workers.get(&WorkerMetricsKey {
                scope: Some("jobs:backend-metrics".to_string()),
                worker_id: "backend-worker".to_string(),
            }),
            Some(&MetricCounts {
                claimed_total: 1,
                released_total: 1,
                closed_total: 1,
                ..MetricCounts::default()
            })
        );

        let reclaimed = scheduler.take_ready().await.unwrap().unwrap();
        assert_eq!(claimed.task.id, reclaimed.task.id);
    }

    #[tokio::test]
    async fn redis_scheduler_drains_release_and_close_runtime_events() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "runtime_release";
        let scheduler =
            Redis::new(format!("redis://{url}"), namespace).with_worker(Worker::new("worker-a"));
        scheduler
            .enqueue(Task::new(Request::new(
                "https://example.com/release-runtime",
            )))
            .await
            .unwrap();
        let claimed = scheduler.take_ready().await.unwrap().unwrap();

        assert_eq!(scheduler.release_inflight().await.unwrap(), 1);
        scheduler.close().await.unwrap();

        let events = scheduler.drain_runtime_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, RuntimeEventKind::Released);
        assert_eq!(events[0].scope.as_deref(), Some(namespace));
        assert_eq!(events[0].worker_id.as_deref(), Some("worker-a"));
        assert_eq!(events[0].count, 1);
        assert_eq!(events[1].kind, RuntimeEventKind::Closed);
        assert_eq!(events[1].scope.as_deref(), Some(namespace));
        assert_eq!(events[1].worker_id.as_deref(), Some("worker-a"));

        let reclaimed = scheduler.take_ready().await.unwrap().unwrap();
        assert_eq!(claimed.task.id, reclaimed.task.id);
        scheduler.complete(&reclaimed.lease).await.unwrap();
        scheduler.close().await.unwrap();

        server_handle.await.unwrap().unwrap();
    }
}
