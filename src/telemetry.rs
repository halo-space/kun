use crate::scheduler::{MetricsReporter, MetricsSnapshot, RuntimeEvent, RuntimeReporter};
use crate::stats::{self, Snapshot as StatsSnapshot};
use jiff::Timestamp;
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, MeterProvider as _};
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs::{File as FsFile, OpenOptions, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;

const DEFAULT_RECENT_EVENT_LIMIT: usize = 256;

/// One unified runtime event exported from the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Stats {
        kind: stats::Event,
        snapshot: StatsSnapshot,
    },
    Scheduler(RuntimeEvent),
}

impl Event {
    pub fn stats(kind: stats::Event, snapshot: StatsSnapshot) -> Self {
        Self::Stats { kind, snapshot }
    }

    pub fn scheduler(event: RuntimeEvent) -> Self {
        Self::Scheduler(event)
    }

    pub fn stream(&self) -> &'static str {
        match self {
            Self::Stats { .. } => "stats",
            Self::Scheduler(_) => "scheduler",
        }
    }
}

/// Unified telemetry export boundary for runtime stats and scheduler events.
pub trait Exporter: Send + Sync {
    fn export(&self, event: Event);
}

impl<E> Exporter for Arc<E>
where
    E: Exporter + ?Sized,
{
    fn export(&self, event: Event) {
        (**self).export(event);
    }
}

impl<T> crate::stats::Reporter for T
where
    T: Exporter + ?Sized,
{
    fn report(&self, event: stats::Event, snapshot: StatsSnapshot) {
        self.export(Event::stats(event, snapshot));
    }
}

impl<T> RuntimeReporter for T
where
    T: Exporter + ?Sized,
{
    fn report(&self, event: RuntimeEvent) {
        self.export(Event::scheduler(event));
    }
}

/// Fan out one telemetry stream into multiple exporters.
#[derive(Clone, Default)]
pub struct Fanout {
    exporters: Arc<Mutex<Vec<Arc<dyn Exporter>>>>,
}

impl Fanout {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_exporter(self, exporter: impl Exporter + 'static) -> Self {
        self.add_exporter(exporter);
        self
    }

    pub fn add_exporter(&self, exporter: impl Exporter + 'static) {
        if let Ok(mut exporters) = self.exporters.lock() {
            exporters.push(Arc::new(exporter));
        }
    }
}

impl Exporter for Fanout {
    fn export(&self, event: Event) {
        let exporters = match self.exporters.lock() {
            Ok(exporters) => exporters.clone(),
            Err(_) => return,
        };

        for exporter in exporters {
            exporter.export(event.clone());
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CollectorSnapshot {
    pub stats: StatsSnapshot,
    pub scheduler: MetricsSnapshot,
    pub recent_events: Vec<Event>,
}

#[derive(Default)]
struct CollectorState {
    stats: StatsSnapshot,
    recent_events: VecDeque<Event>,
}

/// In-memory telemetry collector for tests, exporters, and dashboards.
#[derive(Clone)]
pub struct Collector {
    state: Arc<Mutex<CollectorState>>,
    scheduler: MetricsReporter,
    recent_event_limit: usize,
}

impl Default for Collector {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(CollectorState::default())),
            scheduler: MetricsReporter::new(),
            recent_event_limit: DEFAULT_RECENT_EVENT_LIMIT,
        }
    }
}

impl Collector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_recent_event_limit(mut self, limit: usize) -> Self {
        self.recent_event_limit = limit;
        self
    }

    pub fn snapshot(&self) -> CollectorSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        CollectorSnapshot {
            stats: state.stats,
            scheduler: self.scheduler.snapshot(),
            recent_events: state.recent_events.iter().cloned().collect(),
        }
    }
}

impl Exporter for Collector {
    fn export(&self, event: Event) {
        if let Event::Scheduler(runtime_event) = &event {
            RuntimeReporter::report(&self.scheduler, runtime_event.clone());
        }

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Event::Stats { snapshot, .. } = &event {
            state.stats = *snapshot;
        }

        if self.recent_event_limit > 0 {
            while state.recent_events.len() >= self.recent_event_limit {
                state.recent_events.pop_front();
            }
            state.recent_events.push_back(event);
        }
    }
}

/// Append-only telemetry file sink.
///
/// Each exported event is written as one JSON line with a timestamp, stream,
/// and event payload. This is useful as a minimal persistent event bus and can
/// later be tailed or forwarded into external collectors.
#[derive(Clone)]
pub struct File {
    path: PathBuf,
    file: Arc<Mutex<FsFile>>,
}

impl File {
    pub fn new(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        ensure_parent_dir(&path)?;
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file: Arc::new(Mutex::new(file)),
        })
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl Exporter for File {
    fn export(&self, event: Event) {
        let record = json!({
            "ts": Timestamp::now().to_string(),
            "stream": event.stream(),
            "event": event_json(&event),
        });

        let Ok(mut file) = self.file.lock() else {
            return;
        };

        if serde_json::to_writer(&mut *file, &record).is_err() {
            return;
        }

        if file.write_all(b"\n").is_err() {
            return;
        }

        let _ = file.flush();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpenTelemetryError {
    #[error("open telemetry exporter setup failed: {0}")]
    Setup(String),
    #[error("open telemetry exporter shutdown failed: {0}")]
    Shutdown(String),
}

/// OTLP/OpenTelemetry exporter that reuses the unified telemetry event stream.
///
/// It keeps local telemetry state for inspection, and in parallel records
/// counters into an OpenTelemetry meter provider that can periodically export
/// to an OTLP collector over HTTP/protobuf.
#[derive(Clone)]
pub struct OpenTelemetry {
    collector: Collector,
    meter_provider: Arc<SdkMeterProvider>,
    stats: Arc<OpenTelemetryStats>,
    scheduler: Arc<OpenTelemetryScheduler>,
}

impl OpenTelemetry {
    pub fn new(endpoint: impl Into<String>) -> Result<Self, OpenTelemetryError> {
        OpenTelemetryBuilder::new(endpoint).build()
    }

    pub fn builder(endpoint: impl Into<String>) -> OpenTelemetryBuilder {
        OpenTelemetryBuilder::new(endpoint)
    }

    pub fn snapshot(&self) -> CollectorSnapshot {
        self.collector.snapshot()
    }

    pub fn shutdown(&self) -> Result<(), OpenTelemetryError> {
        self.meter_provider
            .shutdown()
            .map_err(|error| OpenTelemetryError::Shutdown(error.to_string()))
    }
}

impl Exporter for OpenTelemetry {
    fn export(&self, event: Event) {
        self.collector.export(event.clone());

        match event {
            Event::Stats { kind, .. } => self.stats.record(kind),
            Event::Scheduler(event) => self.scheduler.record(&event),
        }
    }
}

pub struct OpenTelemetryBuilder {
    endpoint: String,
    service_name: String,
    export_interval: Duration,
}

impl OpenTelemetryBuilder {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            service_name: "halo-spider".to_string(),
            export_interval: Duration::from_secs(60),
        }
    }

    pub fn with_service_name(mut self, service_name: impl Into<String>) -> Self {
        self.service_name = service_name.into();
        self
    }

    pub fn with_export_interval(mut self, export_interval: Duration) -> Self {
        self.export_interval = export_interval;
        self
    }

    pub fn build(self) -> Result<OpenTelemetry, OpenTelemetryError> {
        let exporter = MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(self.endpoint)
            .build()
            .map_err(|error| OpenTelemetryError::Setup(error.to_string()))?;

        let reader = PeriodicReader::builder(exporter)
            .with_interval(self.export_interval)
            .build();

        let meter_provider = SdkMeterProvider::builder()
            .with_resource(
                Resource::builder()
                    .with_service_name(self.service_name)
                    .build(),
            )
            .with_reader(reader)
            .build();
        let meter = meter_provider.meter("halo_spider.telemetry");

        Ok(OpenTelemetry {
            collector: Collector::default(),
            stats: Arc::new(OpenTelemetryStats::new(&meter)),
            scheduler: Arc::new(OpenTelemetryScheduler::new(&meter)),
            meter_provider: Arc::new(meter_provider),
        })
    }
}

/// In-memory Prometheus exporter for engine stats and scheduler runtime
/// metrics.
///
/// This keeps the same unified telemetry input boundary as `Collector`, but
/// exposes a Prometheus text rendering API suitable for pull-style scraping.
#[derive(Clone)]
pub struct Prometheus {
    collector: Collector,
    prefix: String,
}

impl Default for Prometheus {
    fn default() -> Self {
        Self {
            collector: Collector::default(),
            prefix: "halo_spider".to_string(),
        }
    }
}

impl Prometheus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = normalize_metric_prefix(prefix.into().as_str());
        self
    }

    pub fn with_recent_event_limit(mut self, limit: usize) -> Self {
        self.collector = self.collector.with_recent_event_limit(limit);
        self
    }

    pub fn snapshot(&self) -> CollectorSnapshot {
        self.collector.snapshot()
    }

    pub fn render(&self) -> String {
        let snapshot = self.collector.snapshot();
        let mut output = String::new();

        render_stats_metrics(&mut output, self.prefix.as_str(), &snapshot.stats);
        render_scheduler_metrics(&mut output, self.prefix.as_str(), &snapshot.scheduler);

        output
    }
}

impl Exporter for Prometheus {
    fn export(&self, event: Event) {
        self.collector.export(event);
    }
}

struct OpenTelemetryStats {
    request: Counter<u64>,
    response: Counter<u64>,
    error: Counter<u64>,
    retry: Counter<u64>,
    item: Counter<u64>,
    pipeline_drop: Counter<u64>,
    dedup_reject: Counter<u64>,
    robots_disallow: Counter<u64>,
    robots_delay: Counter<u64>,
    http_cache_hit: Counter<u64>,
    http_cache_revalidate: Counter<u64>,
    http_cache_store: Counter<u64>,
    http_cache_miss: Counter<u64>,
    store_error: Counter<u64>,
    scheduler_claim: Counter<u64>,
    scheduler_complete: Counter<u64>,
    scheduler_requeue: Counter<u64>,
    scheduler_heartbeat: Counter<u64>,
    scheduler_lease_lost: Counter<u64>,
}

impl OpenTelemetryStats {
    fn new(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            request: build_counter(
                meter,
                "request_total",
                "Total engine requests handled by this engine instance.",
            ),
            response: build_counter(
                meter,
                "response_total",
                "Total engine responses handled by this engine instance.",
            ),
            error: build_counter(
                meter,
                "error_total",
                "Total engine errors observed by this engine instance.",
            ),
            retry: build_counter(
                meter,
                "retry_total",
                "Total task retries scheduled by this engine instance.",
            ),
            item: build_counter(
                meter,
                "item_total",
                "Total items persisted by this engine instance.",
            ),
            pipeline_drop: build_counter(
                meter,
                "pipeline_drop_total",
                "Total items dropped by pipelines in this engine instance.",
            ),
            dedup_reject: build_counter(
                meter,
                "dedup_reject_total",
                "Total requests rejected by dedup in this engine instance.",
            ),
            robots_disallow: build_counter(
                meter,
                "robots_disallow_total",
                "Total requests rejected by robots in this engine instance.",
            ),
            robots_delay: build_counter(
                meter,
                "robots_delay_total",
                "Total requests delayed by robots in this engine instance.",
            ),
            http_cache_hit: build_counter(
                meter,
                "http_cache_hit_total",
                "Total HTTP cache hits in this engine instance.",
            ),
            http_cache_revalidate: build_counter(
                meter,
                "http_cache_revalidate_total",
                "Total HTTP cache revalidations in this engine instance.",
            ),
            http_cache_store: build_counter(
                meter,
                "http_cache_store_total",
                "Total HTTP cache stores in this engine instance.",
            ),
            http_cache_miss: build_counter(
                meter,
                "http_cache_miss_total",
                "Total HTTP cache misses in this engine instance.",
            ),
            store_error: build_counter(
                meter,
                "store_error_total",
                "Total final store write errors in this engine instance.",
            ),
            scheduler_claim: build_counter(
                meter,
                "scheduler_claim_total",
                "Total scheduler claims observed by this engine instance.",
            ),
            scheduler_complete: build_counter(
                meter,
                "scheduler_complete_total",
                "Total scheduler completes observed by this engine instance.",
            ),
            scheduler_requeue: build_counter(
                meter,
                "scheduler_requeue_total",
                "Total scheduler requeues observed by this engine instance.",
            ),
            scheduler_heartbeat: build_counter(
                meter,
                "scheduler_heartbeat_total",
                "Total scheduler heartbeats observed by this engine instance.",
            ),
            scheduler_lease_lost: build_counter(
                meter,
                "scheduler_lease_lost_total",
                "Total scheduler lease-lost events observed by this engine instance.",
            ),
        }
    }

    fn record(&self, kind: stats::Event) {
        match kind {
            stats::Event::Request => self.request.add(1, &[]),
            stats::Event::Response => self.response.add(1, &[]),
            stats::Event::Error => self.error.add(1, &[]),
            stats::Event::Retry => self.retry.add(1, &[]),
            stats::Event::Item => self.item.add(1, &[]),
            stats::Event::PipelineDrop => self.pipeline_drop.add(1, &[]),
            stats::Event::DedupReject => self.dedup_reject.add(1, &[]),
            stats::Event::RobotsDisallow => self.robots_disallow.add(1, &[]),
            stats::Event::RobotsDelay => self.robots_delay.add(1, &[]),
            stats::Event::HttpCacheHit => self.http_cache_hit.add(1, &[]),
            stats::Event::HttpCacheRevalidate => self.http_cache_revalidate.add(1, &[]),
            stats::Event::HttpCacheStore => self.http_cache_store.add(1, &[]),
            stats::Event::HttpCacheMiss => self.http_cache_miss.add(1, &[]),
            stats::Event::StoreError => self.store_error.add(1, &[]),
            stats::Event::SchedulerClaim => self.scheduler_claim.add(1, &[]),
            stats::Event::SchedulerComplete => self.scheduler_complete.add(1, &[]),
            stats::Event::SchedulerRequeue => self.scheduler_requeue.add(1, &[]),
            stats::Event::SchedulerHeartbeat => self.scheduler_heartbeat.add(1, &[]),
            stats::Event::SchedulerLeaseLost => self.scheduler_lease_lost.add(1, &[]),
        }
    }
}

struct OpenTelemetryScheduler {
    claimed: Counter<u64>,
    completed: Counter<u64>,
    requeued: Counter<u64>,
    heartbeat: Counter<u64>,
    lease_lost: Counter<u64>,
    reclaimed: Counter<u64>,
    released: Counter<u64>,
    closed: Counter<u64>,
}

impl OpenTelemetryScheduler {
    fn new(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            claimed: build_counter(
                meter,
                "scheduler_runtime_claim_total",
                "Total scheduler claim runtime events.",
            ),
            completed: build_counter(
                meter,
                "scheduler_runtime_complete_total",
                "Total scheduler complete runtime events.",
            ),
            requeued: build_counter(
                meter,
                "scheduler_runtime_requeue_total",
                "Total scheduler requeue runtime events.",
            ),
            heartbeat: build_counter(
                meter,
                "scheduler_runtime_heartbeat_total",
                "Total scheduler heartbeat runtime events.",
            ),
            lease_lost: build_counter(
                meter,
                "scheduler_runtime_lease_lost_total",
                "Total scheduler lease-lost runtime events.",
            ),
            reclaimed: build_counter(
                meter,
                "scheduler_runtime_reclaimed_total",
                "Total scheduler reclaimed runtime events.",
            ),
            released: build_counter(
                meter,
                "scheduler_runtime_released_total",
                "Total scheduler released runtime events.",
            ),
            closed: build_counter(
                meter,
                "scheduler_runtime_closed_total",
                "Total scheduler closed runtime events.",
            ),
        }
    }

    fn record(&self, event: &RuntimeEvent) {
        let value = u64::try_from(event.count.max(1)).unwrap_or(u64::MAX);
        let attrs = scheduler_attributes(event);

        match event.kind {
            crate::scheduler::RuntimeEventKind::Claimed => {
                self.claimed.add(value, attrs.as_slice())
            }
            crate::scheduler::RuntimeEventKind::Completed => {
                self.completed.add(value, attrs.as_slice())
            }
            crate::scheduler::RuntimeEventKind::Requeued => {
                self.requeued.add(value, attrs.as_slice())
            }
            crate::scheduler::RuntimeEventKind::Heartbeat => {
                self.heartbeat.add(value, attrs.as_slice())
            }
            crate::scheduler::RuntimeEventKind::LeaseLost => {
                self.lease_lost.add(value, attrs.as_slice())
            }
            crate::scheduler::RuntimeEventKind::Reclaimed => {
                self.reclaimed.add(value, attrs.as_slice())
            }
            crate::scheduler::RuntimeEventKind::Released => {
                self.released.add(value, attrs.as_slice())
            }
            crate::scheduler::RuntimeEventKind::Closed => self.closed.add(value, attrs.as_slice()),
        }
    }
}

fn ensure_parent_dir(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_dir_all(parent)?;
    }

    Ok(())
}

fn event_json(event: &Event) -> Value {
    match event {
        Event::Stats { kind, snapshot } => json!({
            "kind": kind.as_str(),
            "snapshot": stats_snapshot_json(snapshot),
        }),
        Event::Scheduler(event) => json!({
            "kind": event.kind.as_str(),
            "scope": event.scope,
            "worker_id": event.worker_id,
            "task_id": event.task_id.as_ref().map(|task_id| task_id.as_str().to_string()),
            "lease_id": event.lease_id,
            "url": event.url,
            "count": event.count,
            "error": event.error.as_ref().map(ToString::to_string),
        }),
    }
}

fn stats_snapshot_json(snapshot: &StatsSnapshot) -> Value {
    json!({
        "request_count": snapshot.request_count,
        "response_count": snapshot.response_count,
        "error_count": snapshot.error_count,
        "retry_count": snapshot.retry_count,
        "item_count": snapshot.item_count,
        "pipeline_drop_count": snapshot.pipeline_drop_count,
        "dedup_reject_count": snapshot.dedup_reject_count,
        "robots_disallow_count": snapshot.robots_disallow_count,
        "robots_delay_count": snapshot.robots_delay_count,
        "http_cache_hit_count": snapshot.http_cache_hit_count,
        "http_cache_revalidate_count": snapshot.http_cache_revalidate_count,
        "http_cache_store_count": snapshot.http_cache_store_count,
        "http_cache_miss_count": snapshot.http_cache_miss_count,
        "store_error_count": snapshot.store_error_count,
        "scheduler_claim_count": snapshot.scheduler_claim_count,
        "scheduler_complete_count": snapshot.scheduler_complete_count,
        "scheduler_requeue_count": snapshot.scheduler_requeue_count,
        "scheduler_heartbeat_count": snapshot.scheduler_heartbeat_count,
        "scheduler_lease_lost_count": snapshot.scheduler_lease_lost_count,
    })
}

fn build_counter(
    meter: &opentelemetry::metrics::Meter,
    name: &'static str,
    description: &'static str,
) -> Counter<u64> {
    meter
        .u64_counter(name)
        .with_description(description)
        .build()
}

fn scheduler_attributes(event: &RuntimeEvent) -> Vec<KeyValue> {
    let mut attrs = Vec::new();

    if let Some(scope) = &event.scope {
        attrs.push(KeyValue::new("scope", scope.clone()));
    }

    if let Some(worker_id) = &event.worker_id {
        attrs.push(KeyValue::new("worker_id", worker_id.clone()));
    }

    attrs
}

fn render_stats_metrics(output: &mut String, prefix: &str, snapshot: &StatsSnapshot) {
    for (name, help, value) in [
        (
            "request_total",
            "Total engine requests handled by this engine instance.",
            snapshot.request_count,
        ),
        (
            "response_total",
            "Total engine responses handled by this engine instance.",
            snapshot.response_count,
        ),
        (
            "error_total",
            "Total engine errors observed by this engine instance.",
            snapshot.error_count,
        ),
        (
            "retry_total",
            "Total task retries scheduled by this engine instance.",
            snapshot.retry_count,
        ),
        (
            "item_total",
            "Total items persisted by this engine instance.",
            snapshot.item_count,
        ),
        (
            "pipeline_drop_total",
            "Total items dropped by pipelines in this engine instance.",
            snapshot.pipeline_drop_count,
        ),
        (
            "dedup_reject_total",
            "Total requests rejected by dedup in this engine instance.",
            snapshot.dedup_reject_count,
        ),
        (
            "robots_disallow_total",
            "Total requests rejected by robots in this engine instance.",
            snapshot.robots_disallow_count,
        ),
        (
            "robots_delay_total",
            "Total requests delayed by robots in this engine instance.",
            snapshot.robots_delay_count,
        ),
        (
            "http_cache_hit_total",
            "Total HTTP cache hits in this engine instance.",
            snapshot.http_cache_hit_count,
        ),
        (
            "http_cache_revalidate_total",
            "Total HTTP cache revalidations in this engine instance.",
            snapshot.http_cache_revalidate_count,
        ),
        (
            "http_cache_store_total",
            "Total HTTP cache stores in this engine instance.",
            snapshot.http_cache_store_count,
        ),
        (
            "http_cache_miss_total",
            "Total HTTP cache misses in this engine instance.",
            snapshot.http_cache_miss_count,
        ),
        (
            "store_error_total",
            "Total final store write errors in this engine instance.",
            snapshot.store_error_count,
        ),
        (
            "scheduler_claim_total",
            "Total scheduler claims observed by this engine instance.",
            snapshot.scheduler_claim_count,
        ),
        (
            "scheduler_complete_total",
            "Total scheduler completes observed by this engine instance.",
            snapshot.scheduler_complete_count,
        ),
        (
            "scheduler_requeue_total",
            "Total scheduler requeues observed by this engine instance.",
            snapshot.scheduler_requeue_count,
        ),
        (
            "scheduler_heartbeat_total",
            "Total scheduler heartbeats observed by this engine instance.",
            snapshot.scheduler_heartbeat_count,
        ),
        (
            "scheduler_lease_lost_total",
            "Total scheduler lease-lost events observed by this engine instance.",
            snapshot.scheduler_lease_lost_count,
        ),
    ] {
        let metric_name = format!("{prefix}_{name}");
        write_counter_header(output, metric_name.as_str(), help);
        write_metric_sample(output, metric_name.as_str(), &[], value);
    }
}

fn render_scheduler_metrics(output: &mut String, prefix: &str, snapshot: &MetricsSnapshot) {
    for (metric_suffix, help, total, select) in scheduler_metric_families(snapshot).iter() {
        let metric_name = format!("{prefix}_scheduler_runtime_{metric_suffix}");
        write_counter_header(output, metric_name.as_str(), help);
        write_metric_sample(output, metric_name.as_str(), &[], *total);

        for (scope, counts) in &snapshot.scopes {
            let value = select(counts);
            if value == 0 {
                continue;
            }
            write_metric_sample(
                output,
                metric_name.as_str(),
                &[("scope", scope.clone())],
                value,
            );
        }

        for (worker, counts) in &snapshot.workers {
            let value = select(counts);
            if value == 0 {
                continue;
            }

            let mut labels = Vec::new();
            if let Some(scope) = &worker.scope {
                labels.push(("scope", scope.clone()));
            }
            labels.push(("worker_id", worker.worker_id.clone()));
            write_metric_sample(output, metric_name.as_str(), labels.as_slice(), value);
        }
    }
}

fn scheduler_metric_families(
    snapshot: &MetricsSnapshot,
) -> [(
    &'static str,
    &'static str,
    u64,
    fn(&crate::scheduler::MetricCounts) -> u64,
); 8] {
    [
        (
            "claim_total",
            "Total scheduler claim runtime events.",
            snapshot.totals.claimed_total,
            |counts| counts.claimed_total,
        ),
        (
            "complete_total",
            "Total scheduler complete runtime events.",
            snapshot.totals.completed_total,
            |counts| counts.completed_total,
        ),
        (
            "requeue_total",
            "Total scheduler requeue runtime events.",
            snapshot.totals.requeued_total,
            |counts| counts.requeued_total,
        ),
        (
            "heartbeat_total",
            "Total scheduler heartbeat runtime events.",
            snapshot.totals.heartbeat_total,
            |counts| counts.heartbeat_total,
        ),
        (
            "lease_lost_total",
            "Total scheduler lease-lost runtime events.",
            snapshot.totals.lease_lost_total,
            |counts| counts.lease_lost_total,
        ),
        (
            "reclaimed_total",
            "Total scheduler reclaimed runtime events.",
            snapshot.totals.reclaimed_total,
            |counts| counts.reclaimed_total,
        ),
        (
            "released_total",
            "Total scheduler released runtime events.",
            snapshot.totals.released_total,
            |counts| counts.released_total,
        ),
        (
            "closed_total",
            "Total scheduler closed runtime events.",
            snapshot.totals.closed_total,
            |counts| counts.closed_total,
        ),
    ]
}

fn write_counter_header(output: &mut String, name: &str, help: &str) {
    let _ = writeln!(output, "# HELP {name} {help}");
    let _ = writeln!(output, "# TYPE {name} counter");
}

fn write_metric_sample(output: &mut String, name: &str, labels: &[(&str, String)], value: u64) {
    if labels.is_empty() {
        let _ = writeln!(output, "{name} {value}");
        return;
    }

    let labels = labels
        .iter()
        .map(|(key, value)| format!(r#"{key}="{}""#, escape_label_value(value.as_str())))
        .collect::<Vec<_>>()
        .join(",");

    let _ = writeln!(output, "{name}{{{labels}}} {value}");
}

fn escape_label_value(value: &str) -> String {
    value
        .replace('\\', r"\\")
        .replace('\n', r"\n")
        .replace('"', r#"\""#)
}

fn normalize_metric_prefix(prefix: &str) -> String {
    let mut normalized = prefix
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if normalized.is_empty() {
        return "halo_spider".to_string();
    }

    if normalized
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        normalized.insert(0, '_');
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::{MetricCounts, RuntimeEvent, RuntimeEventKind, WorkerMetricsKey};
    use std::io::{Read, Write as IoWrite};
    use std::net::TcpListener;
    use std::thread;

    fn unique_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("halo-spider-{name}-{nanos}.jsonl"))
    }

    #[test]
    fn collector_tracks_stats_and_scheduler_runtime() {
        let collector = Collector::default().with_recent_event_limit(8);

        crate::stats::Reporter::report(
            &collector,
            stats::Event::Request,
            StatsSnapshot {
                request_count: 1,
                ..StatsSnapshot::default()
            },
        );
        RuntimeReporter::report(
            &collector,
            RuntimeEvent::reclaimed(Some("jobs:test".to_string()), 2),
        );

        let snapshot = collector.snapshot();
        assert_eq!(snapshot.stats.request_count, 1);
        assert_eq!(
            snapshot.scheduler.totals,
            MetricCounts {
                reclaimed_total: 2,
                ..MetricCounts::default()
            }
        );
        assert_eq!(snapshot.scheduler.scopes["jobs:test"].reclaimed_total, 2);
        assert_eq!(snapshot.recent_events.len(), 2);
    }

    #[test]
    fn fanout_forwards_events_to_multiple_exporters() {
        let left = Collector::default();
        let right = Collector::default();
        let fanout = Fanout::new()
            .with_exporter(left.clone())
            .with_exporter(right.clone());

        RuntimeReporter::report(
            &fanout,
            RuntimeEvent::released(
                Some("jobs:fanout".to_string()),
                Some("worker-x".to_string()),
                1,
            ),
        );

        assert_eq!(left.snapshot().scheduler.totals.released_total, 1);
        assert_eq!(right.snapshot().scheduler.totals.released_total, 1);
    }

    #[test]
    fn json_lines_writes_stats_and_scheduler_events() {
        let path = unique_path("telemetry");
        let exporter = File::new(&path).expect("telemetry file exporter should create");

        crate::stats::Reporter::report(
            &exporter,
            stats::Event::SchedulerClaim,
            StatsSnapshot {
                scheduler_claim_count: 1,
                ..StatsSnapshot::default()
            },
        );
        RuntimeReporter::report(
            &exporter,
            RuntimeEvent {
                scope: Some("jobs:news".to_string()),
                kind: RuntimeEventKind::Claimed,
                worker_id: Some("worker-a".to_string()),
                task_id: None,
                lease_id: Some("lease-1".to_string()),
                url: Some("https://example.com/news".to_string()),
                count: 1,
                error: None,
            },
        );

        let content = std::fs::read_to_string(&path).expect("telemetry file should be readable");
        assert!(content.contains(r#""stream":"stats""#));
        assert!(content.contains(r#""stream":"scheduler""#));
        assert!(content.contains(r#""kind":"scheduler_claim""#));
        assert!(content.contains(r#""kind":"claimed""#));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn collector_keeps_worker_metric_breakdown() {
        let collector = Collector::default();
        RuntimeReporter::report(
            &collector,
            RuntimeEvent {
                scope: Some("jobs:detail".to_string()),
                kind: RuntimeEventKind::Released,
                worker_id: Some("worker-b".to_string()),
                task_id: None,
                lease_id: None,
                url: None,
                count: 3,
                error: None,
            },
        );

        let snapshot = collector.snapshot();
        assert_eq!(snapshot.scheduler.totals.released_total, 3);
        assert_eq!(
            snapshot.scheduler.workers[&WorkerMetricsKey {
                scope: Some("jobs:detail".to_string()),
                worker_id: "worker-b".to_string(),
            }]
                .released_total,
            3
        );
    }

    #[test]
    fn prometheus_renders_engine_stats_and_scheduler_runtime_metrics() {
        let prometheus = Prometheus::default();

        crate::stats::Reporter::report(
            &prometheus,
            stats::Event::Request,
            StatsSnapshot {
                request_count: 2,
                scheduler_claim_count: 1,
                ..StatsSnapshot::default()
            },
        );
        RuntimeReporter::report(
            &prometheus,
            RuntimeEvent::released(
                Some("jobs:prom".to_string()),
                Some("worker-a".to_string()),
                3,
            ),
        );

        let rendered = prometheus.render();
        assert!(rendered.contains("# HELP halo_spider_request_total"));
        assert!(rendered.contains("halo_spider_request_total 2"));
        assert!(rendered.contains("halo_spider_scheduler_claim_total 1"));
        assert!(rendered.contains("# HELP halo_spider_scheduler_runtime_released_total"));
        assert!(rendered.contains("halo_spider_scheduler_runtime_released_total 3"));
        assert!(
            rendered
                .contains(r#"halo_spider_scheduler_runtime_released_total{scope="jobs:prom"} 3"#)
        );
        assert!(rendered.contains(
            r#"halo_spider_scheduler_runtime_released_total{scope="jobs:prom",worker_id="worker-a"} 3"#
        ));
    }

    #[test]
    fn prometheus_normalizes_custom_prefix() {
        let prometheus = Prometheus::new().with_prefix("halo-spider.demo");
        crate::stats::Reporter::report(
            &prometheus,
            stats::Event::Response,
            StatsSnapshot {
                response_count: 1,
                ..StatsSnapshot::default()
            },
        );

        let rendered = prometheus.render();
        assert!(rendered.contains("# HELP halo_spider_demo_response_total"));
        assert!(rendered.contains("halo_spider_demo_response_total 1"));
    }

    #[test]
    fn open_telemetry_exports_to_local_collector() {
        let (endpoint, server) = spawn_http_sink();
        let exporter = OpenTelemetry::builder(endpoint)
            .with_service_name("halo-spider-test")
            .with_export_interval(Duration::from_secs(3600))
            .build()
            .expect("open telemetry exporter should build");

        crate::stats::Reporter::report(
            &exporter,
            stats::Event::Request,
            StatsSnapshot {
                request_count: 1,
                ..StatsSnapshot::default()
            },
        );
        RuntimeReporter::report(
            &exporter,
            RuntimeEvent::released(
                Some("jobs:otel".to_string()),
                Some("worker-otel".to_string()),
                2,
            ),
        );

        let snapshot = exporter.snapshot();
        assert_eq!(snapshot.stats.request_count, 1);
        assert_eq!(snapshot.scheduler.totals.released_total, 2);

        exporter
            .shutdown()
            .expect("open telemetry exporter should flush and shutdown");
        let request_line = server.join().expect("http sink thread should join");
        assert_eq!(request_line, "POST /v1/metrics HTTP/1.1");
    }

    fn spawn_http_sink() -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local addr");

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("collector should accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("collector stream should set timeout");

            let mut bytes = Vec::new();
            let mut chunk = [0u8; 2048];
            let header_end = loop {
                let read = stream
                    .read(&mut chunk)
                    .expect("collector should read request");
                assert!(read > 0, "collector request should not be empty");
                bytes.extend_from_slice(&chunk[..read]);

                if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    break position + 4;
                }
            };

            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
            let content_length = parse_content_length(headers.as_str());
            while bytes.len().saturating_sub(header_end) < content_length {
                let read = stream
                    .read(&mut chunk)
                    .expect("collector should read request body");
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..read]);
            }

            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .expect("collector should respond");

            headers
                .lines()
                .next()
                .expect("collector request should have request line")
                .to_string()
        });

        (format!("http://{address}/v1/metrics"), handle)
    }

    fn parse_content_length(headers: &str) -> usize {
        headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or_default()
    }
}
