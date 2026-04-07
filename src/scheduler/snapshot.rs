use crate::scheduler::TaskId;
use crate::scheduler::checkpoint::Counts;
use jiff::{SignedDuration, Timestamp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    /// Number of visible scheduler scopes included in this overview.
    pub scope_count: usize,
    /// Number of scopes that still have ready, delayed, or inflight tasks.
    pub pending_scope_count: usize,
    /// Number of scopes currently paused for new claims.
    pub paused_scope_count: usize,
    /// Number of scopes that currently report at least one stale worker.
    pub stale_scope_count: usize,
    /// Aggregate ready / delayed / inflight task counts across all scopes.
    pub counts: Counts,
    /// Total number of worker runtime entries observed across all scopes.
    pub worker_count: usize,
    /// Total number of stale workers observed across all scopes.
    pub stale_worker_count: usize,
    /// Total number of active inflight lease tokens across all scopes.
    pub active_lease_count: usize,
    /// Aggregate reclaimed stale inflight task count across all scopes.
    pub reclaimed_total: u64,
}

impl Overview {
    pub fn from_snapshots<I>(snapshots: I) -> Self
    where
        I: IntoIterator<Item = Snapshot>,
    {
        let mut scope_count = 0usize;
        let mut pending_scope_count = 0usize;
        let mut paused_scope_count = 0usize;
        let mut stale_scope_count = 0usize;
        let mut counts = Counts::default();
        let mut worker_count = 0usize;
        let mut stale_worker_count = 0usize;
        let mut active_lease_count = 0usize;
        let mut reclaimed_total = 0u64;

        for snapshot in snapshots {
            scope_count += 1;
            if snapshot.counts.has_pending() {
                pending_scope_count += 1;
            }
            if snapshot.is_paused {
                paused_scope_count += 1;
            }
            if snapshot.workers.iter().any(|worker| worker.is_stale) {
                stale_scope_count += 1;
            }
            counts.ready += snapshot.counts.ready;
            counts.delayed += snapshot.counts.delayed;
            counts.inflight += snapshot.counts.inflight;
            worker_count += snapshot.workers.len();
            stale_worker_count += snapshot
                .workers
                .iter()
                .filter(|worker| worker.is_stale)
                .count();
            active_lease_count += snapshot.active_lease_count;
            reclaimed_total = reclaimed_total.saturating_add(snapshot.reclaimed_total);
        }

        Self {
            scope_count,
            pending_scope_count,
            paused_scope_count,
            stale_scope_count,
            counts,
            worker_count,
            stale_worker_count,
            active_lease_count,
            reclaimed_total,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Logical scheduler scope this runtime snapshot was read from.
    pub scope: String,
    /// Whether this scope is currently paused for new claims.
    pub is_paused: bool,
    /// Instantaneous ready / delayed / inflight counts after any refresh work
    /// done for this snapshot.
    pub counts: Counts,
    /// Unique worker ids that currently own at least one inflight task.
    pub worker_ids: Vec<String>,
    /// Number of active inflight lease tokens currently tracked.
    pub active_lease_count: usize,
    /// Number of inflight deadline entries currently tracked.
    pub deadline_count: usize,
    /// Cumulative reclaimed stale inflight task count for this scope.
    pub reclaimed_total: u64,
    /// Number of stale inflight tasks reclaimed during this snapshot refresh.
    pub reclaimed_in_refresh: u64,
    /// Per-task inflight runtime details for the current scope.
    pub inflight_tasks: Vec<InflightTaskSnapshot>,
    /// Per-worker runtime details currently observed for this scope.
    pub workers: Vec<WorkerSnapshot>,
    /// Effective lease timeout configured for this scope, if any.
    pub lease_timeout: Option<SignedDuration>,
    /// Effective heartbeat interval configured for this scope, if any.
    pub heartbeat_interval: Option<SignedDuration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InflightTaskSnapshot {
    /// Stable scheduler task identity.
    pub task_id: TaskId,
    /// Request URL currently attached to this inflight task.
    pub url: String,
    /// Current worker ownership recorded for this inflight task.
    pub worker_id: Option<String>,
    /// Current lease token recorded for this inflight task.
    pub lease_id: Option<String>,
    /// Current runtime deadline recorded for this inflight task.
    pub deadline: Option<Timestamp>,
    /// Scheduler priority attached to this task.
    pub priority: i32,
    /// Scheduler crawl depth attached to this task.
    pub depth: u32,
    /// Delayed execution timestamp carried by this task, if any.
    pub ready_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSnapshot {
    /// Logical worker id currently registered for this scope.
    pub worker_id: String,
    /// Most recent runtime touch observed for this worker.
    pub last_seen: Option<Timestamp>,
    /// Whether this worker has exceeded its configured lease timeout window.
    pub is_stale: bool,
    /// Number of inflight tasks currently owned by this worker.
    pub inflight_count: usize,
    /// Number of active lease tokens currently owned by this worker.
    pub active_lease_count: usize,
    /// Stable task ids currently owned by this worker.
    pub inflight_task_ids: Vec<TaskId>,
    /// Earliest inflight deadline currently owned by this worker.
    pub next_deadline: Option<Timestamp>,
    /// Lease timeout last reported by this worker.
    pub lease_timeout: Option<SignedDuration>,
    /// Heartbeat interval last reported by this worker.
    pub heartbeat_interval: Option<SignedDuration>,
}
