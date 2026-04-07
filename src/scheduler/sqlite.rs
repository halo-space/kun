use crate::error::{SchedulerError, SpiderError};
use crate::scheduler::checkpoint::{Checkpoint, Counts};
use crate::scheduler::control::Control;
use crate::scheduler::runtime::RuntimeEvent;
use crate::scheduler::snapshot::{InflightTaskSnapshot, Snapshot, WorkerSnapshot};
use crate::scheduler::{ClaimedTask, Scheduler, Task, TaskLease, Worker};
use jiff::{SignedDuration, Timestamp};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tokio::sync::Mutex;

const DEFAULT_LEASE_TIMEOUT: u64 = 300_000;
static NEXT_WORKER_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_SCOPE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_LEASE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct Sqlite {
    path: PathBuf,
    scope: String,
    worker: Worker,
    pool: Arc<Mutex<Option<SqlitePool>>>,
    runtime_events: Arc<StdMutex<Vec<RuntimeEvent>>>,
}

impl Sqlite {
    pub fn new(path: impl Into<PathBuf>, scope: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            scope: scope.into(),
            worker: Worker::new(next_worker_id()).with_lease_timeout(SignedDuration::from_millis(
                i64::try_from(DEFAULT_LEASE_TIMEOUT).unwrap_or(i64::MAX),
            )),
            pool: Arc::new(Mutex::new(None)),
            runtime_events: Arc::new(StdMutex::new(Vec::new())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn scope(&self) -> &str {
        self.scope.as_str()
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    pub fn worker(&self) -> &Worker {
        &self.worker
    }

    pub fn worker_id(&self) -> &str {
        self.worker.worker_id()
    }

    pub fn with_worker(mut self, worker: Worker) -> Self {
        self.worker = worker;
        self
    }

    pub async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        self.read_checkpoint_for_scope(self.scope(), true).await
    }

    pub async fn counts(&self) -> Result<Counts, SpiderError> {
        self.read_counts_for_scope(self.scope(), true).await
    }

    pub async fn snapshot(&self) -> Result<Snapshot, SpiderError> {
        self.read_snapshot_for_scope(self.scope(), true).await
    }

    pub async fn scopes(&self) -> Result<Vec<String>, SpiderError> {
        self.read_scopes_with_prefix("", true).await
    }

    pub async fn scopes_with_prefix(
        &self,
        prefix: impl AsRef<str>,
    ) -> Result<Vec<String>, SpiderError> {
        self.read_scopes_with_prefix(prefix.as_ref(), true).await
    }

    pub async fn snapshots(&self) -> Result<Vec<Snapshot>, SpiderError> {
        self.read_snapshots_with_prefix("", true).await
    }

    pub async fn snapshots_with_prefix(
        &self,
        prefix: impl AsRef<str>,
    ) -> Result<Vec<Snapshot>, SpiderError> {
        self.read_snapshots_with_prefix(prefix.as_ref(), true).await
    }

    fn validate(&self) -> Result<(), SpiderError> {
        if self.scope.trim().is_empty() {
            return Err(SpiderError::scheduler(
                "sqlite scheduler scope cannot be empty",
            ));
        }

        if self.worker_id().trim().is_empty() {
            return Err(SpiderError::scheduler(
                "sqlite scheduler worker_id cannot be empty",
            ));
        }

        Ok(())
    }

    async fn pool(&self) -> Result<SqlitePool, SpiderError> {
        self.validate()?;

        {
            let guard = self.pool.lock().await;
            if let Some(pool) = guard.clone() {
                return Ok(pool);
            }
        }

        let pool = self.open_pool().await?;
        let mut guard = self.pool.lock().await;
        if let Some(existing) = guard.clone() {
            return Ok(existing);
        }
        *guard = Some(pool.clone());
        Ok(pool)
    }

    async fn open_pool(&self) -> Result<SqlitePool, SpiderError> {
        ensure_parent_dir(&self.path).await?;

        let options = sqlite_connect_options(&self.path)?;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|error| {
                SpiderError::engine(format!("failed to open sqlite scheduler database: {error}"))
            })?;

        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .map_err(|error| {
                SpiderError::engine(format!(
                    "failed to enable sqlite scheduler foreign_keys: {error}"
                ))
            })?;

        for statement in schema_statements() {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| {
                    SpiderError::engine(format!(
                        "failed to initialize sqlite scheduler schema: {error}"
                    ))
                })?;
        }

        Ok(pool)
    }

    fn lease_timeout_millis(&self) -> Option<u64> {
        self.worker
            .effective_lease_timeout(Some(SignedDuration::from_millis(
                i64::try_from(DEFAULT_LEASE_TIMEOUT).unwrap_or(i64::MAX),
            )))
            .map(non_negative_milliseconds)
    }

    fn heartbeat_interval_millis(&self) -> Option<u64> {
        let default = self.lease_timeout_millis().map(|timeout| {
            SignedDuration::from_millis(
                i64::try_from(default_heartbeat_interval(timeout)).unwrap_or(i64::MAX),
            )
        });
        self.worker
            .effective_heartbeat_interval(default)
            .map(non_negative_milliseconds)
    }

    fn push_runtime_event(&self, event: RuntimeEvent) {
        if let Ok(mut events) = self.runtime_events.lock() {
            events.push(event);
        }
    }

    fn validate_lease_worker(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        if lease.worker_id() != self.worker_id() {
            return Err(SpiderError::scheduler(
                SchedulerError::LeaseWorkerMismatch {
                    lease_worker_id: lease.worker_id().to_string(),
                    current_worker_id: self.worker_id().to_string(),
                },
            ));
        }
        Ok(())
    }

    async fn ensure_current_scope(&self, pool: &SqlitePool) -> Result<(), SpiderError> {
        ensure_scope_row(
            pool,
            self.scope(),
            self.lease_timeout_millis(),
            self.heartbeat_interval_millis(),
        )
        .await
    }

    async fn read_scopes_with_prefix(
        &self,
        prefix: &str,
        sync_current_scope: bool,
    ) -> Result<Vec<String>, SpiderError> {
        let pool = self.pool().await?;
        if sync_current_scope {
            self.ensure_current_scope(&pool).await?;
        }

        let pattern = format!("{prefix}%");
        let rows = sqlx::query(
            "SELECT scope
             FROM scheduler_scopes
             WHERE scope LIKE ?
             ORDER BY scope ASC",
        )
        .bind(pattern)
        .fetch_all(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to read sqlite scheduler scopes: {error}"))
        })?;

        Ok(rows
            .into_iter()
            .map(|row| row.get::<String, _>("scope"))
            .collect())
    }

    async fn read_counts_for_scope(
        &self,
        scope: &str,
        sync_current_scope: bool,
    ) -> Result<Counts, SpiderError> {
        let pool = self.pool().await?;
        if sync_current_scope && scope == self.scope() {
            self.ensure_current_scope(&pool).await?;
        } else {
            ensure_visible_scope(&pool, scope).await?;
        }

        let reclaimed = reclaim_expired_tasks(&pool, scope).await?;
        if reclaimed > 0 {
            self.push_runtime_event(RuntimeEvent::reclaimed(Some(scope.to_string()), reclaimed));
        }
        promote_delayed_tasks(&pool, scope).await?;

        read_counts_no_refresh(&pool, scope).await
    }

    async fn read_checkpoint_for_scope(
        &self,
        scope: &str,
        sync_current_scope: bool,
    ) -> Result<Checkpoint, SpiderError> {
        let pool = self.pool().await?;
        if sync_current_scope && scope == self.scope() {
            self.ensure_current_scope(&pool).await?;
        } else {
            ensure_visible_scope(&pool, scope).await?;
        }

        let reclaimed = reclaim_expired_tasks(&pool, scope).await?;
        if reclaimed > 0 {
            self.push_runtime_event(RuntimeEvent::reclaimed(Some(scope.to_string()), reclaimed));
        }
        promote_delayed_tasks(&pool, scope).await?;

        let ready_rows = sqlx::query(
            "SELECT task_json
             FROM scheduler_tasks
             WHERE scope = ? AND state = 'ready'
             ORDER BY priority DESC, depth ASC, sequence ASC",
        )
        .bind(scope)
        .fetch_all(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to read sqlite ready checkpoint: {error}"))
        })?;

        let delayed_rows = sqlx::query(
            "SELECT task_json
             FROM scheduler_tasks
             WHERE scope = ? AND state = 'delayed'
             ORDER BY ready_at_ms ASC, sequence ASC",
        )
        .bind(scope)
        .fetch_all(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to read sqlite delayed checkpoint: {error}"))
        })?;

        let inflight_rows = sqlx::query(
            "SELECT task_json
             FROM scheduler_tasks
             WHERE scope = ? AND state = 'inflight'
             ORDER BY claimed_at_ms ASC, sequence ASC",
        )
        .bind(scope)
        .fetch_all(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to read sqlite inflight checkpoint: {error}"
            ))
        })?;

        Ok(Checkpoint {
            ready: decode_task_rows(ready_rows)?,
            delayed: decode_task_rows(delayed_rows)?,
            inflight: decode_task_rows(inflight_rows)?,
        })
    }

    async fn read_snapshot_for_scope(
        &self,
        scope: &str,
        sync_current_scope: bool,
    ) -> Result<Snapshot, SpiderError> {
        let pool = self.pool().await?;
        if sync_current_scope && scope == self.scope() {
            self.ensure_current_scope(&pool).await?;
        } else {
            ensure_visible_scope(&pool, scope).await?;
        }

        let reclaimed_in_refresh = reclaim_expired_tasks(&pool, scope).await?;
        if reclaimed_in_refresh > 0 {
            self.push_runtime_event(RuntimeEvent::reclaimed(
                Some(scope.to_string()),
                reclaimed_in_refresh,
            ));
        }
        promote_delayed_tasks(&pool, scope).await?;

        let meta = sqlx::query(
            "SELECT is_paused, reclaimed_total, lease_timeout_ms, heartbeat_interval_ms
             FROM scheduler_scopes
             WHERE scope = ?",
        )
        .bind(scope)
        .fetch_one(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to read sqlite scheduler scope metadata: {error}"
            ))
        })?;

        let counts = read_counts_no_refresh(&pool, scope).await?;

        let inflight_rows = sqlx::query(
            "SELECT task_id, task_json, worker_id, lease_id, deadline_ms
             FROM scheduler_tasks
             WHERE scope = ? AND state = 'inflight'
             ORDER BY deadline_ms ASC, sequence ASC",
        )
        .bind(scope)
        .fetch_all(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to read sqlite inflight snapshot rows: {error}"
            ))
        })?;

        let mut worker_task_ids: BTreeMap<String, Vec<crate::scheduler::TaskId>> = BTreeMap::new();
        let mut worker_deadlines: BTreeMap<String, Vec<Timestamp>> = BTreeMap::new();
        let mut inflight_tasks = Vec::with_capacity(inflight_rows.len());
        let mut worker_ids = BTreeSet::new();
        let mut active_lease_count = 0usize;
        let mut deadline_count = 0usize;

        for row in inflight_rows {
            let task = decode_task_json(row.get::<String, _>("task_json"))?;
            let worker_id = row.get::<Option<String>, _>("worker_id");
            let lease_id = row.get::<Option<String>, _>("lease_id");
            let deadline = row
                .get::<Option<i64>, _>("deadline_ms")
                .map(timestamp_from_millis)
                .transpose()?;
            let ready_at = task.ready_at.map(timestamp_from_u64).transpose()?;

            if let Some(worker_id) = worker_id.clone() {
                worker_ids.insert(worker_id.clone());
                worker_task_ids
                    .entry(worker_id.clone())
                    .or_default()
                    .push(task.id.clone());
                if let Some(deadline) = deadline {
                    worker_deadlines
                        .entry(worker_id)
                        .or_default()
                        .push(deadline);
                }
            }

            if lease_id.is_some() {
                active_lease_count += 1;
            }
            if deadline.is_some() {
                deadline_count += 1;
            }

            inflight_tasks.push(InflightTaskSnapshot {
                task_id: task.id.clone(),
                url: task.request.url.clone(),
                worker_id,
                lease_id,
                deadline,
                priority: task.priority,
                depth: task.depth,
                ready_at,
            });
        }

        let worker_rows = sqlx::query(
            "SELECT worker_id, last_seen_ms, lease_timeout_ms, heartbeat_interval_ms
             FROM scheduler_workers
             WHERE scope = ?
             ORDER BY worker_id ASC",
        )
        .bind(scope)
        .fetch_all(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to read sqlite worker snapshot rows: {error}"
            ))
        })?;

        let now_millis = now_i64();
        let mut workers = Vec::with_capacity(worker_rows.len());
        for row in worker_rows {
            let worker_id = row.get::<String, _>("worker_id");
            let last_seen = row
                .get::<Option<i64>, _>("last_seen_ms")
                .map(timestamp_from_millis)
                .transpose()?;
            let lease_timeout = row
                .get::<Option<i64>, _>("lease_timeout_ms")
                .map(signed_duration_from_millis)
                .transpose()?;
            let heartbeat_interval = row
                .get::<Option<i64>, _>("heartbeat_interval_ms")
                .map(signed_duration_from_millis)
                .transpose()?;
            let inflight_task_ids = worker_task_ids.remove(&worker_id).unwrap_or_default();
            let deadlines = worker_deadlines.remove(&worker_id).unwrap_or_default();
            let next_deadline = deadlines.into_iter().min();
            let is_stale = match (last_seen, lease_timeout) {
                (Some(last_seen), Some(lease_timeout)) => {
                    last_seen.as_millisecond().saturating_add(
                        i64::try_from(lease_timeout.as_millis()).unwrap_or(i64::MAX),
                    ) < now_millis
                }
                _ => false,
            };

            workers.push(WorkerSnapshot {
                worker_id,
                last_seen,
                is_stale,
                inflight_count: inflight_task_ids.len(),
                active_lease_count: inflight_task_ids.len(),
                inflight_task_ids,
                next_deadline,
                lease_timeout,
                heartbeat_interval,
            });
        }

        Ok(Snapshot {
            scope: scope.to_string(),
            is_paused: meta.get::<i64, _>("is_paused") != 0,
            counts,
            worker_ids: worker_ids.into_iter().collect(),
            active_lease_count,
            deadline_count,
            reclaimed_total: u64::try_from(meta.get::<i64, _>("reclaimed_total"))
                .unwrap_or_default(),
            reclaimed_in_refresh: u64::try_from(reclaimed_in_refresh).unwrap_or_default(),
            inflight_tasks,
            workers,
            lease_timeout: meta
                .get::<Option<i64>, _>("lease_timeout_ms")
                .map(signed_duration_from_millis)
                .transpose()?,
            heartbeat_interval: meta
                .get::<Option<i64>, _>("heartbeat_interval_ms")
                .map(signed_duration_from_millis)
                .transpose()?,
        })
    }

    async fn read_snapshots_with_prefix(
        &self,
        prefix: &str,
        sync_current_scope: bool,
    ) -> Result<Vec<Snapshot>, SpiderError> {
        let scopes = self
            .read_scopes_with_prefix(prefix, sync_current_scope)
            .await?;
        let mut snapshots = Vec::with_capacity(scopes.len());
        for scope in scopes {
            snapshots.push(
                self.read_snapshot_for_scope(&scope, sync_current_scope && scope == self.scope())
                    .await?,
            );
        }
        Ok(snapshots)
    }
}

impl Scheduler for Sqlite {
    async fn enqueue(&self, task: Task) -> Result<(), SpiderError> {
        let pool = self.pool().await?;
        self.ensure_current_scope(&pool).await?;
        let sequence = reserve_sequence(&pool, self.scope()).await?;
        insert_task(&pool, self.scope(), task, sequence).await
    }

    async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        self.read_checkpoint_for_scope(self.scope(), true).await
    }

    async fn counts(&self) -> Result<Counts, SpiderError> {
        self.read_counts_for_scope(self.scope(), true).await
    }

    async fn snapshot(&self) -> Result<Snapshot, SpiderError> {
        self.read_snapshot_for_scope(self.scope(), true).await
    }

    async fn scopes(&self) -> Result<Vec<String>, SpiderError> {
        self.read_scopes_with_prefix("", true).await
    }

    async fn scopes_with_prefix(&self, prefix: &str) -> Result<Vec<String>, SpiderError> {
        self.read_scopes_with_prefix(prefix, true).await
    }

    async fn snapshots(&self) -> Result<Vec<Snapshot>, SpiderError> {
        self.read_snapshots_with_prefix("", true).await
    }

    async fn snapshots_with_prefix(&self, prefix: &str) -> Result<Vec<Snapshot>, SpiderError> {
        self.read_snapshots_with_prefix(prefix, true).await
    }

    async fn take_ready(&self) -> Result<Option<ClaimedTask>, SpiderError> {
        let pool = self.pool().await?;
        self.ensure_current_scope(&pool).await?;

        let reclaimed = reclaim_expired_tasks(&pool, self.scope()).await?;
        if reclaimed > 0 {
            self.push_runtime_event(RuntimeEvent::reclaimed(
                Some(self.scope().to_string()),
                reclaimed,
            ));
        }
        promote_delayed_tasks(&pool, self.scope()).await?;

        let paused: Option<i64> = sqlx::query_scalar(
            "SELECT is_paused
             FROM scheduler_scopes
             WHERE scope = ?",
        )
        .bind(self.scope())
        .fetch_optional(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to read sqlite paused scope flag: {error}"))
        })?;

        if paused.unwrap_or_default() != 0 {
            refresh_registered_worker(&pool, self.scope(), self.worker_id()).await?;
            return Ok(None);
        }

        let lease_id = next_lease_id(self.worker_id());
        let deadline = self
            .lease_timeout_millis()
            .map(|timeout| now_u64().saturating_add(timeout))
            .and_then(|value| i64::try_from(value).ok());

        let claimed = sqlx::query(
            "UPDATE scheduler_tasks
             SET state = 'inflight',
                 worker_id = ?,
                 lease_id = ?,
                 deadline_ms = ?,
                 claimed_at_ms = ?
             WHERE scope = ?
               AND task_id = (
                    SELECT task_id
                    FROM scheduler_tasks
                    WHERE scope = ? AND state = 'ready'
                    ORDER BY priority DESC, depth ASC, sequence ASC
                    LIMIT 1
               )",
        )
        .bind(self.worker_id())
        .bind(&lease_id)
        .bind(deadline)
        .bind(now_i64())
        .bind(self.scope())
        .bind(self.scope())
        .execute(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to claim sqlite ready task: {error}"))
        })?;

        if claimed.rows_affected() == 0 {
            refresh_registered_worker(&pool, self.scope(), self.worker_id()).await?;
            return Ok(None);
        }

        upsert_worker_runtime(
            &pool,
            self.scope(),
            self.worker_id(),
            self.lease_timeout_millis(),
            self.heartbeat_interval_millis(),
        )
        .await?;

        let row = sqlx::query(
            "SELECT task_json
             FROM scheduler_tasks
             WHERE scope = ? AND state = 'inflight' AND worker_id = ? AND lease_id = ?
             LIMIT 1",
        )
        .bind(self.scope())
        .bind(self.worker_id())
        .bind(&lease_id)
        .fetch_one(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to load claimed sqlite task: {error}"))
        })?;

        let task = decode_task_json(row.get::<String, _>("task_json"))?;
        Ok(Some(ClaimedTask::new(
            task.clone(),
            TaskLease::new(task.id, self.worker_id().to_string(), lease_id),
        )))
    }

    async fn complete(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        self.validate_lease_worker(lease)?;
        let pool = self.pool().await?;
        let result = sqlx::query(
            "DELETE FROM scheduler_tasks
             WHERE scope = ? AND task_id = ? AND state = 'inflight' AND worker_id = ? AND lease_id = ?",
        )
        .bind(self.scope())
        .bind(lease.task_id().as_str())
        .bind(self.worker_id())
        .bind(lease.lease_id())
        .execute(&pool)
        .await
        .map_err(|error| SpiderError::scheduler(format!("failed to complete sqlite inflight task: {error}")))?;

        if result.rows_affected() == 1 {
            clear_worker_runtime_if_idle(&pool, self.scope(), self.worker_id()).await?;
            return Ok(());
        }

        Err(resolve_lease_error(&pool, self.scope(), "complete", lease).await)
    }

    async fn complete_and_enqueue(
        &self,
        lease: &TaskLease,
        tasks: Vec<Task>,
    ) -> Result<(), SpiderError> {
        self.validate_lease_worker(lease)?;
        let pool = self.pool().await?;
        self.ensure_current_scope(&pool).await?;

        sqlx::query("BEGIN IMMEDIATE")
            .execute(&pool)
            .await
            .map_err(|error| {
                SpiderError::scheduler(format!(
                    "failed to begin sqlite completion transaction: {error}"
                ))
            })?;

        let result = async {
            for task in tasks {
                let sequence = reserve_sequence(&pool, self.scope()).await?;
                insert_task(&pool, self.scope(), task, sequence).await?;
            }

            let deleted = sqlx::query(
                "DELETE FROM scheduler_tasks
                 WHERE scope = ? AND task_id = ? AND state = 'inflight' AND worker_id = ? AND lease_id = ?",
            )
            .bind(self.scope())
            .bind(lease.task_id().as_str())
            .bind(self.worker_id())
            .bind(lease.lease_id())
            .execute(&pool)
            .await
            .map_err(|error| SpiderError::scheduler(format!("failed to complete sqlite inflight task inside transaction: {error}")))?;

            if deleted.rows_affected() != 1 {
                return Err(resolve_lease_error(&pool, self.scope(), "complete", lease).await);
            }

            Ok(())
        }
        .await;

        let finalize = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
        let finalize_result = sqlx::query(finalize).execute(&pool).await;
        if let Err(error) = finalize_result {
            return Err(SpiderError::scheduler(format!(
                "failed to finalize sqlite completion transaction: {error}"
            )));
        }

        result?;
        clear_worker_runtime_if_idle(&pool, self.scope(), self.worker_id()).await?;
        Ok(())
    }

    async fn requeue(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        self.validate_lease_worker(lease)?;
        let pool = self.pool().await?;

        let row = sqlx::query(
            "SELECT task_json
             FROM scheduler_tasks
             WHERE scope = ? AND task_id = ? AND state = 'inflight' AND worker_id = ? AND lease_id = ?",
        )
        .bind(self.scope())
        .bind(lease.task_id().as_str())
        .bind(self.worker_id())
        .bind(lease.lease_id())
        .fetch_optional(&pool)
        .await
        .map_err(|error| SpiderError::scheduler(format!("failed to read sqlite requeue task: {error}")))?;

        let Some(row) = row else {
            return Err(resolve_lease_error(&pool, self.scope(), "requeue", lease).await);
        };

        let task = decode_task_json(row.get::<String, _>("task_json"))?;
        let target_state = if task.is_ready() { "ready" } else { "delayed" };
        let sequence = reserve_sequence(&pool, self.scope()).await?;

        let updated = sqlx::query(
            "UPDATE scheduler_tasks
             SET state = ?,
                 worker_id = NULL,
                 lease_id = NULL,
                 deadline_ms = NULL,
                 claimed_at_ms = NULL,
                 sequence = ?
             WHERE scope = ? AND task_id = ? AND state = 'inflight' AND worker_id = ? AND lease_id = ?",
        )
        .bind(target_state)
        .bind(sequence)
        .bind(self.scope())
        .bind(lease.task_id().as_str())
        .bind(self.worker_id())
        .bind(lease.lease_id())
        .execute(&pool)
        .await
        .map_err(|error| SpiderError::scheduler(format!("failed to requeue sqlite inflight task: {error}")))?;

        if updated.rows_affected() != 1 {
            return Err(resolve_lease_error(&pool, self.scope(), "requeue", lease).await);
        }

        clear_worker_runtime_if_idle(&pool, self.scope(), self.worker_id()).await?;
        Ok(())
    }

    async fn release_inflight(&self) -> Result<usize, SpiderError> {
        let pool = self.pool().await?;
        self.ensure_current_scope(&pool).await?;

        let rows = sqlx::query(
            "SELECT task_id, ready_at_ms
             FROM scheduler_tasks
             WHERE scope = ? AND state = 'inflight' AND worker_id = ?
             ORDER BY claimed_at_ms ASC, sequence ASC",
        )
        .bind(self.scope())
        .bind(self.worker_id())
        .fetch_all(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to read sqlite inflight release rows: {error}"
            ))
        })?;

        let mut released = 0usize;
        for row in rows {
            let task_id = row.get::<String, _>("task_id");
            let ready_at = row.get::<Option<i64>, _>("ready_at_ms");
            let state = if ready_at.unwrap_or_default() <= now_i64() {
                "ready"
            } else {
                "delayed"
            };
            let sequence = reserve_sequence(&pool, self.scope()).await?;
            let updated = sqlx::query(
                "UPDATE scheduler_tasks
                 SET state = ?,
                     worker_id = NULL,
                     lease_id = NULL,
                     deadline_ms = NULL,
                     claimed_at_ms = NULL,
                     sequence = ?
                 WHERE scope = ? AND task_id = ? AND state = 'inflight' AND worker_id = ?",
            )
            .bind(state)
            .bind(sequence)
            .bind(self.scope())
            .bind(task_id)
            .bind(self.worker_id())
            .execute(&pool)
            .await
            .map_err(|error| {
                SpiderError::scheduler(format!("failed to release sqlite inflight task: {error}"))
            })?;
            released += usize::try_from(updated.rows_affected()).unwrap_or_default();
        }

        clear_worker_runtime_if_idle(&pool, self.scope(), self.worker_id()).await?;
        if released > 0 {
            self.push_runtime_event(RuntimeEvent::released(
                Some(self.scope().to_string()),
                Some(self.worker_id().to_string()),
                released,
            ));
        }
        Ok(released)
    }

    async fn heartbeat(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        self.validate_lease_worker(lease)?;
        let Some(lease_timeout) = self.lease_timeout_millis() else {
            return Ok(());
        };
        let Some(_heartbeat_interval) = self.heartbeat_interval_millis() else {
            return Ok(());
        };

        let pool = self.pool().await?;
        let deadline = now_u64().saturating_add(lease_timeout);
        let updated = sqlx::query(
            "UPDATE scheduler_tasks
             SET deadline_ms = ?
             WHERE scope = ? AND task_id = ? AND state = 'inflight' AND worker_id = ? AND lease_id = ?",
        )
        .bind(i64::try_from(deadline).ok())
        .bind(self.scope())
        .bind(lease.task_id().as_str())
        .bind(self.worker_id())
        .bind(lease.lease_id())
        .execute(&pool)
        .await
        .map_err(|error| SpiderError::scheduler(format!("failed to heartbeat sqlite inflight task: {error}")))?;

        if updated.rows_affected() != 1 {
            return Err(resolve_lease_error(&pool, self.scope(), "heartbeat", lease).await);
        }

        upsert_worker_runtime(
            &pool,
            self.scope(),
            self.worker_id(),
            self.lease_timeout_millis(),
            self.heartbeat_interval_millis(),
        )
        .await?;
        Ok(())
    }

    fn heartbeat_interval(&self) -> Option<SignedDuration> {
        self.heartbeat_interval_millis()
            .and_then(|millis| i64::try_from(millis).ok())
            .map(SignedDuration::from_millis)
    }

    async fn close(&self) -> Result<(), SpiderError> {
        let pool = {
            let mut guard = self.pool.lock().await;
            guard.take()
        };

        if let Some(pool) = pool {
            clear_worker_runtime_if_idle(&pool, self.scope(), self.worker_id()).await?;
            pool.close().await;
            self.push_runtime_event(RuntimeEvent::closed(
                Some(self.scope().to_string()),
                Some(self.worker_id().to_string()),
            ));
        }

        Ok(())
    }

    fn runtime_scope(&self) -> Option<String> {
        Some(self.scope().to_string())
    }

    fn runtime_worker_id(&self) -> Option<String> {
        Some(self.worker_id().to_string())
    }

    fn drain_runtime_events(&self) -> Vec<RuntimeEvent> {
        self.runtime_events
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default()
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        Ok(self.counts().await?.has_pending())
    }
}

impl Control for Sqlite {
    async fn pause_scope(&self, scope: &str) -> Result<bool, SpiderError> {
        let pool = self.pool().await?;
        ensure_visible_scope(&pool, scope).await?;

        let result = sqlx::query(
            "UPDATE scheduler_scopes
             SET is_paused = 1,
                 updated_at_ms = ?
             WHERE scope = ? AND is_paused = 0",
        )
        .bind(now_i64())
        .bind(scope)
        .execute(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to pause sqlite scheduler scope: {error}"))
        })?;

        Ok(result.rows_affected() == 1)
    }

    async fn resume_scope(&self, scope: &str) -> Result<bool, SpiderError> {
        let pool = self.pool().await?;
        ensure_visible_scope(&pool, scope).await?;

        let result = sqlx::query(
            "UPDATE scheduler_scopes
             SET is_paused = 0,
                 updated_at_ms = ?
             WHERE scope = ? AND is_paused != 0",
        )
        .bind(now_i64())
        .bind(scope)
        .execute(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to resume sqlite scheduler scope: {error}"))
        })?;

        Ok(result.rows_affected() == 1)
    }

    async fn release_scope(&self, scope: &str) -> Result<usize, SpiderError> {
        let pool = self.pool().await?;
        ensure_visible_scope(&pool, scope).await?;

        let rows = sqlx::query(
            "SELECT task_id, ready_at_ms
             FROM scheduler_tasks
             WHERE scope = ? AND state = 'inflight'
             ORDER BY claimed_at_ms ASC, sequence ASC",
        )
        .bind(scope)
        .fetch_all(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to read sqlite scope release rows: {error}"))
        })?;

        let mut released = 0usize;
        for row in rows {
            let task_id = row.get::<String, _>("task_id");
            let ready_at = row.get::<Option<i64>, _>("ready_at_ms");
            let state = if ready_at.unwrap_or_default() <= now_i64() {
                "ready"
            } else {
                "delayed"
            };
            let sequence = reserve_sequence(&pool, scope).await?;
            let updated = sqlx::query(
                "UPDATE scheduler_tasks
                 SET state = ?,
                     worker_id = NULL,
                     lease_id = NULL,
                     deadline_ms = NULL,
                     claimed_at_ms = NULL,
                     sequence = ?
                 WHERE scope = ? AND task_id = ? AND state = 'inflight'",
            )
            .bind(state)
            .bind(sequence)
            .bind(scope)
            .bind(task_id)
            .execute(&pool)
            .await
            .map_err(|error| {
                SpiderError::scheduler(format!(
                    "failed to release sqlite scope inflight task: {error}"
                ))
            })?;
            released += usize::try_from(updated.rows_affected()).unwrap_or_default();
        }

        prune_idle_workers_in_scope(&pool, scope).await?;
        if released > 0 {
            self.push_runtime_event(RuntimeEvent::released(
                Some(scope.to_string()),
                None,
                released,
            ));
        }
        Ok(released)
    }

    async fn purge_scope(&self, scope: &str) -> Result<Counts, SpiderError> {
        let pool = self.pool().await?;
        ensure_visible_scope(&pool, scope).await?;

        let counts = read_counts_no_refresh(&pool, scope).await?;

        sqlx::query("DELETE FROM scheduler_tasks WHERE scope = ?")
            .bind(scope)
            .execute(&pool)
            .await
            .map_err(|error| {
                SpiderError::scheduler(format!("failed to purge sqlite scheduler tasks: {error}"))
            })?;
        sqlx::query("DELETE FROM scheduler_workers WHERE scope = ?")
            .bind(scope)
            .execute(&pool)
            .await
            .map_err(|error| {
                SpiderError::scheduler(format!("failed to purge sqlite scheduler workers: {error}"))
            })?;
        sqlx::query(
            "UPDATE scheduler_scopes
             SET is_paused = 0,
                 reclaimed_total = 0,
                 updated_at_ms = ?
             WHERE scope = ?",
        )
        .bind(now_i64())
        .bind(scope)
        .execute(&pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to reset sqlite scheduler scope metadata: {error}"
            ))
        })?;

        Ok(counts)
    }
}

fn schema_statements() -> [&'static str; 6] {
    [
        "CREATE TABLE IF NOT EXISTS scheduler_scopes (
            scope TEXT PRIMARY KEY,
            is_paused INTEGER NOT NULL DEFAULT 0,
            reclaimed_total INTEGER NOT NULL DEFAULT 0,
            next_sequence INTEGER NOT NULL DEFAULT 0,
            lease_timeout_ms INTEGER,
            heartbeat_interval_ms INTEGER,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL
        )",
        "CREATE TABLE IF NOT EXISTS scheduler_tasks (
            scope TEXT NOT NULL,
            task_id TEXT NOT NULL,
            task_json TEXT NOT NULL,
            state TEXT NOT NULL,
            priority INTEGER NOT NULL,
            depth INTEGER NOT NULL,
            ready_at_ms INTEGER,
            sequence INTEGER NOT NULL,
            worker_id TEXT,
            lease_id TEXT,
            deadline_ms INTEGER,
            claimed_at_ms INTEGER,
            PRIMARY KEY (scope, task_id),
            FOREIGN KEY (scope) REFERENCES scheduler_scopes(scope) ON DELETE CASCADE
        )",
        "CREATE INDEX IF NOT EXISTS scheduler_tasks_ready_idx
         ON scheduler_tasks(scope, state, priority DESC, depth ASC, sequence ASC)",
        "CREATE INDEX IF NOT EXISTS scheduler_tasks_delayed_idx
         ON scheduler_tasks(scope, state, ready_at_ms ASC, sequence ASC)",
        "CREATE INDEX IF NOT EXISTS scheduler_tasks_inflight_idx
         ON scheduler_tasks(scope, state, deadline_ms ASC, worker_id, lease_id)",
        "CREATE TABLE IF NOT EXISTS scheduler_workers (
            scope TEXT NOT NULL,
            worker_id TEXT NOT NULL,
            last_seen_ms INTEGER NOT NULL,
            lease_timeout_ms INTEGER,
            heartbeat_interval_ms INTEGER,
            PRIMARY KEY (scope, worker_id),
            FOREIGN KEY (scope) REFERENCES scheduler_scopes(scope) ON DELETE CASCADE
        )",
    ]
}

async fn ensure_scope_row(
    pool: &SqlitePool,
    scope: &str,
    lease_timeout_ms: Option<u64>,
    heartbeat_interval_ms: Option<u64>,
) -> Result<(), SpiderError> {
    let now = now_i64();
    let lease_timeout = lease_timeout_ms.and_then(|value| i64::try_from(value).ok());
    let heartbeat_interval = heartbeat_interval_ms.and_then(|value| i64::try_from(value).ok());
    sqlx::query(
        "INSERT INTO scheduler_scopes (
            scope, is_paused, reclaimed_total, next_sequence, lease_timeout_ms, heartbeat_interval_ms, created_at_ms, updated_at_ms
         ) VALUES (?, 0, 0, 0, ?, ?, ?, ?)
         ON CONFLICT(scope) DO UPDATE SET
            lease_timeout_ms = excluded.lease_timeout_ms,
            heartbeat_interval_ms = excluded.heartbeat_interval_ms,
            updated_at_ms = excluded.updated_at_ms",
    )
    .bind(scope)
    .bind(lease_timeout)
    .bind(heartbeat_interval)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|error| SpiderError::scheduler(format!("failed to ensure sqlite scheduler scope `{scope}`: {error}")))?;
    Ok(())
}

async fn ensure_visible_scope(pool: &SqlitePool, scope: &str) -> Result<(), SpiderError> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1
         FROM scheduler_scopes
         WHERE scope = ?",
    )
    .bind(scope)
    .fetch_optional(pool)
    .await
    .map_err(|error| {
        SpiderError::scheduler(format!(
            "failed to validate sqlite scheduler scope visibility: {error}"
        ))
    })?;

    if exists.is_some() {
        Ok(())
    } else {
        Err(SpiderError::scheduler(format!(
            "scheduler scope `{scope}` is not visible from current backend"
        )))
    }
}

async fn reserve_sequence(pool: &SqlitePool, scope: &str) -> Result<i64, SpiderError> {
    sqlx::query(
        "UPDATE scheduler_scopes
         SET next_sequence = next_sequence + 1,
             updated_at_ms = ?
         WHERE scope = ?",
    )
    .bind(now_i64())
    .bind(scope)
    .execute(pool)
    .await
    .map_err(|error| {
        SpiderError::scheduler(format!(
            "failed to reserve sqlite scheduler sequence: {error}"
        ))
    })?;

    sqlx::query_scalar(
        "SELECT next_sequence
         FROM scheduler_scopes
         WHERE scope = ?",
    )
    .bind(scope)
    .fetch_one(pool)
    .await
    .map_err(|error| {
        SpiderError::scheduler(format!("failed to load sqlite scheduler sequence: {error}"))
    })
}

async fn insert_task(
    pool: &SqlitePool,
    scope: &str,
    task: Task,
    sequence: i64,
) -> Result<(), SpiderError> {
    let task_json = serde_json::to_string(&task).map_err(|error| {
        SpiderError::scheduler(format!("failed to encode sqlite scheduler task: {error}"))
    })?;

    sqlx::query(
        "INSERT INTO scheduler_tasks (
            scope, task_id, task_json, state, priority, depth, ready_at_ms, sequence, worker_id, lease_id, deadline_ms, claimed_at_ms
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL, NULL)",
    )
    .bind(scope)
    .bind(task.id.as_str())
    .bind(task_json)
    .bind(if task.is_ready() { "ready" } else { "delayed" })
    .bind(task.priority)
    .bind(i64::from(task.depth))
    .bind(task.ready_at.and_then(|value| i64::try_from(value).ok()))
    .bind(sequence)
    .execute(pool)
    .await
    .map_err(|error| SpiderError::scheduler(format!("failed to insert sqlite scheduler task: {error}")))?;

    Ok(())
}

async fn promote_delayed_tasks(pool: &SqlitePool, scope: &str) -> Result<(), SpiderError> {
    sqlx::query(
        "UPDATE scheduler_tasks
         SET state = 'ready'
         WHERE scope = ? AND state = 'delayed' AND ready_at_ms IS NOT NULL AND ready_at_ms <= ?",
    )
    .bind(scope)
    .bind(now_i64())
    .execute(pool)
    .await
    .map_err(|error| {
        SpiderError::scheduler(format!("failed to promote sqlite delayed tasks: {error}"))
    })?;
    Ok(())
}

async fn reclaim_expired_tasks(pool: &SqlitePool, scope: &str) -> Result<usize, SpiderError> {
    let rows = sqlx::query(
        "SELECT task_id, ready_at_ms
         FROM scheduler_tasks
         WHERE scope = ? AND state = 'inflight' AND deadline_ms IS NOT NULL AND deadline_ms <= ?
         ORDER BY deadline_ms ASC, sequence ASC",
    )
    .bind(scope)
    .bind(now_i64())
    .fetch_all(pool)
    .await
    .map_err(|error| {
        SpiderError::scheduler(format!(
            "failed to read sqlite expired inflight rows: {error}"
        ))
    })?;

    let mut reclaimed = 0usize;
    for row in rows {
        let task_id = row.get::<String, _>("task_id");
        let ready_at = row.get::<Option<i64>, _>("ready_at_ms");
        let state = if ready_at.unwrap_or_default() <= now_i64() {
            "ready"
        } else {
            "delayed"
        };
        let sequence = reserve_sequence(pool, scope).await?;
        let updated = sqlx::query(
            "UPDATE scheduler_tasks
             SET state = ?,
                 worker_id = NULL,
                 lease_id = NULL,
                 deadline_ms = NULL,
                 claimed_at_ms = NULL,
                 sequence = ?
             WHERE scope = ? AND task_id = ? AND state = 'inflight'",
        )
        .bind(state)
        .bind(sequence)
        .bind(scope)
        .bind(task_id)
        .execute(pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to reclaim sqlite inflight task: {error}"))
        })?;
        reclaimed += usize::try_from(updated.rows_affected()).unwrap_or_default();
    }

    if reclaimed > 0 {
        sqlx::query(
            "UPDATE scheduler_scopes
             SET reclaimed_total = reclaimed_total + ?,
                 updated_at_ms = ?
             WHERE scope = ?",
        )
        .bind(i64::try_from(reclaimed).unwrap_or(i64::MAX))
        .bind(now_i64())
        .bind(scope)
        .execute(pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to increment sqlite reclaimed counter: {error}"
            ))
        })?;
    }

    Ok(reclaimed)
}

async fn upsert_worker_runtime(
    pool: &SqlitePool,
    scope: &str,
    worker_id: &str,
    lease_timeout_ms: Option<u64>,
    heartbeat_interval_ms: Option<u64>,
) -> Result<(), SpiderError> {
    sqlx::query(
        "INSERT INTO scheduler_workers (
            scope, worker_id, last_seen_ms, lease_timeout_ms, heartbeat_interval_ms
         ) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(scope, worker_id) DO UPDATE SET
            last_seen_ms = excluded.last_seen_ms,
            lease_timeout_ms = excluded.lease_timeout_ms,
            heartbeat_interval_ms = excluded.heartbeat_interval_ms",
    )
    .bind(scope)
    .bind(worker_id)
    .bind(now_i64())
    .bind(lease_timeout_ms.and_then(|value| i64::try_from(value).ok()))
    .bind(heartbeat_interval_ms.and_then(|value| i64::try_from(value).ok()))
    .execute(pool)
    .await
    .map_err(|error| {
        SpiderError::scheduler(format!("failed to upsert sqlite worker runtime: {error}"))
    })?;
    Ok(())
}

async fn refresh_registered_worker(
    pool: &SqlitePool,
    scope: &str,
    worker_id: &str,
) -> Result<(), SpiderError> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1
         FROM scheduler_workers
         WHERE scope = ? AND worker_id = ?",
    )
    .bind(scope)
    .bind(worker_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| {
        SpiderError::scheduler(format!(
            "failed to check sqlite worker runtime presence: {error}"
        ))
    })?;

    if exists.is_some() {
        sqlx::query(
            "UPDATE scheduler_workers
             SET last_seen_ms = ?
             WHERE scope = ? AND worker_id = ?",
        )
        .bind(now_i64())
        .bind(scope)
        .bind(worker_id)
        .execute(pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to refresh sqlite worker runtime: {error}"))
        })?;
    }

    Ok(())
}

async fn clear_worker_runtime_if_idle(
    pool: &SqlitePool,
    scope: &str,
    worker_id: &str,
) -> Result<(), SpiderError> {
    let inflight: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM scheduler_tasks
         WHERE scope = ? AND state = 'inflight' AND worker_id = ?",
    )
    .bind(scope)
    .bind(worker_id)
    .fetch_one(pool)
    .await
    .map_err(|error| {
        SpiderError::scheduler(format!(
            "failed to count sqlite inflight worker tasks: {error}"
        ))
    })?;

    if inflight == 0 {
        sqlx::query(
            "DELETE FROM scheduler_workers
             WHERE scope = ? AND worker_id = ?",
        )
        .bind(scope)
        .bind(worker_id)
        .execute(pool)
        .await
        .map_err(|error| {
            SpiderError::scheduler(format!("failed to clear sqlite worker runtime: {error}"))
        })?;
    }

    Ok(())
}

async fn prune_idle_workers_in_scope(pool: &SqlitePool, scope: &str) -> Result<(), SpiderError> {
    sqlx::query(
        "DELETE FROM scheduler_workers
         WHERE scope = ?
           AND worker_id NOT IN (
                SELECT DISTINCT worker_id
                FROM scheduler_tasks
                WHERE scope = ? AND state = 'inflight' AND worker_id IS NOT NULL
           )",
    )
    .bind(scope)
    .bind(scope)
    .execute(pool)
    .await
    .map_err(|error| {
        SpiderError::scheduler(format!(
            "failed to prune sqlite idle worker runtime: {error}"
        ))
    })?;
    Ok(())
}

async fn read_counts_no_refresh(pool: &SqlitePool, scope: &str) -> Result<Counts, SpiderError> {
    let rows = sqlx::query(
        "SELECT state, COUNT(*) AS count
         FROM scheduler_tasks
         WHERE scope = ?
         GROUP BY state",
    )
    .bind(scope)
    .fetch_all(pool)
    .await
    .map_err(|error| {
        SpiderError::scheduler(format!("failed to read sqlite scheduler counts: {error}"))
    })?;

    let mut counts = Counts::default();
    for row in rows {
        let state = row.get::<String, _>("state");
        let count = usize::try_from(row.get::<i64, _>("count")).unwrap_or_default();
        match state.as_str() {
            "ready" => counts.ready = count,
            "delayed" => counts.delayed = count,
            "inflight" => counts.inflight = count,
            _ => {}
        }
    }
    Ok(counts)
}

async fn resolve_lease_error(
    pool: &SqlitePool,
    scope: &str,
    action: &'static str,
    lease: &TaskLease,
) -> SpiderError {
    match sqlx::query(
        "SELECT state, worker_id, lease_id
         FROM scheduler_tasks
         WHERE scope = ? AND task_id = ?",
    )
    .bind(scope)
    .bind(lease.task_id().as_str())
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => {
            let state = row.get::<String, _>("state");
            let worker_id = row.get::<Option<String>, _>("worker_id");
            let lease_id = row.get::<Option<String>, _>("lease_id");

            if state != "inflight" {
                SpiderError::scheduler(SchedulerError::InactiveLease {
                    action,
                    task_id: lease.task_id().as_str().to_string(),
                })
            } else if worker_id.as_deref() != Some(lease.worker_id()) {
                SpiderError::scheduler(SchedulerError::LeaseOwnershipConflict {
                    action,
                    task_id: lease.task_id().as_str().to_string(),
                    worker_id: lease.worker_id().to_string(),
                })
            } else if lease_id.as_deref() != Some(lease.lease_id()) {
                SpiderError::scheduler(SchedulerError::StaleLease {
                    action,
                    task_id: lease.task_id().as_str().to_string(),
                    worker_id: lease.worker_id().to_string(),
                    lease_id: lease.lease_id().to_string(),
                })
            } else {
                SpiderError::scheduler(SchedulerError::InactiveLease {
                    action,
                    task_id: lease.task_id().as_str().to_string(),
                })
            }
        }
        Ok(None) => SpiderError::scheduler(SchedulerError::InactiveLease {
            action,
            task_id: lease.task_id().as_str().to_string(),
        }),
        Err(error) => SpiderError::scheduler(format!(
            "failed to validate sqlite scheduler lease resolution: {error}"
        )),
    }
}

fn decode_task_rows(rows: Vec<sqlx::sqlite::SqliteRow>) -> Result<Vec<Task>, SpiderError> {
    rows.into_iter()
        .map(|row| decode_task_json(row.get::<String, _>("task_json")))
        .collect()
}

fn decode_task_json(task_json: String) -> Result<Task, SpiderError> {
    serde_json::from_str(&task_json).map_err(|error| {
        SpiderError::scheduler(format!("failed to decode sqlite scheduler task: {error}"))
    })
}

fn sqlite_connect_options(path: &Path) -> Result<SqliteConnectOptions, SpiderError> {
    SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map(|options| options.create_if_missing(true))
        .map_err(|error| SpiderError::engine(format!("invalid sqlite scheduler path: {error}")))
}

async fn ensure_parent_dir(path: &Path) -> Result<(), SpiderError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        SpiderError::engine(format!(
            "failed to create sqlite scheduler directory: {error}"
        ))
    })?;
    Ok(())
}

fn non_negative_milliseconds(duration: SignedDuration) -> u64 {
    let millis = duration.as_millis();
    if millis <= 0 {
        0
    } else {
        u64::try_from(millis).unwrap_or_default()
    }
}

fn default_heartbeat_interval(lease_timeout: u64) -> u64 {
    (lease_timeout / 2).max(1)
}

fn signed_duration_from_millis(millis: i64) -> Result<SignedDuration, SpiderError> {
    Ok(SignedDuration::from_millis(millis))
}

fn timestamp_from_millis(millis: i64) -> Result<Timestamp, SpiderError> {
    Timestamp::from_millisecond(millis).map_err(|error| {
        SpiderError::scheduler(format!(
            "sqlite scheduler timestamp `{millis}` is invalid: {error}"
        ))
    })
}

fn timestamp_from_u64(millis: u64) -> Result<Timestamp, SpiderError> {
    let millis = i64::try_from(millis).map_err(|_| {
        SpiderError::scheduler(format!(
            "sqlite scheduler timestamp `{millis}` exceeds i64 millisecond range"
        ))
    })?;
    timestamp_from_millis(millis)
}

fn now_u64() -> u64 {
    u64::try_from(Timestamp::now().as_millisecond()).unwrap_or_default()
}

fn now_i64() -> i64 {
    Timestamp::now().as_millisecond()
}

fn next_worker_id() -> String {
    format!(
        "sqlite-worker-{}-{}-{}",
        std::process::id(),
        now_u64(),
        NEXT_WORKER_ID.fetch_add(1, AtomicOrdering::Relaxed)
    )
}

#[allow(dead_code)]
fn next_scope_id() -> String {
    format!(
        "sqlite:{}:{}",
        std::process::id(),
        NEXT_SCOPE_ID.fetch_add(1, AtomicOrdering::Relaxed)
    )
}

fn next_lease_id(worker_id: &str) -> String {
    format!(
        "{worker_id}-sqlite-lease-{}-{}",
        now_u64(),
        NEXT_LEASE_ID.fetch_add(1, AtomicOrdering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::scheduler::{Control, Scheduler, TaskResolution};
    use std::path::PathBuf;
    use tokio::time::{Duration, sleep};

    #[tokio::test]
    async fn sqlite_scheduler_supports_basic_durable_flow() {
        let path = test_db_path("basic_flow");
        let scheduler =
            Sqlite::new(&path, "jobs:sqlite-basic").with_worker(Worker::new("sqlite-worker-a"));

        scheduler
            .enqueue(Task::new(Request::new("https://example.com/a")).with_priority(10))
            .await
            .unwrap();
        scheduler
            .enqueue(Task::new(Request::new("https://example.com/b")).with_priority(1))
            .await
            .unwrap();

        let first = scheduler.take_ready().await.unwrap().unwrap();
        assert_eq!(first.task.request.url, "https://example.com/a");
        assert_eq!(
            scheduler.counts().await.unwrap(),
            Counts {
                ready: 1,
                delayed: 0,
                inflight: 1,
            }
        );

        scheduler.complete(&first.lease).await.unwrap();
        let second = scheduler.take_ready().await.unwrap().unwrap();
        assert_eq!(second.task.request.url, "https://example.com/b");
        scheduler.complete(&second.lease).await.unwrap();
        assert_eq!(scheduler.counts().await.unwrap(), Counts::default());

        scheduler.close().await.unwrap();
        cleanup_test_db(&path);
    }

    #[tokio::test]
    async fn sqlite_scheduler_supports_batch_and_complete_enqueue() {
        let path = test_db_path("batch_flow");
        let scheduler =
            Sqlite::new(&path, "jobs:sqlite-batch").with_worker(Worker::new("sqlite-batch-worker"));

        scheduler
            .enqueue(Task::new(Request::new("https://example.com/1")))
            .await
            .unwrap();
        scheduler
            .enqueue(Task::new(Request::new("https://example.com/2")))
            .await
            .unwrap();

        let claimed = scheduler.take_batch_ready(2).await.unwrap();
        assert_eq!(claimed.len(), 2);

        scheduler
            .requeue_batch(vec![claimed[0].lease.clone(), claimed[1].lease.clone()])
            .await
            .unwrap();

        let claimed = scheduler.take_batch_ready(2).await.unwrap();
        assert_eq!(claimed.len(), 2);

        scheduler
            .complete_and_enqueue_batch(vec![
                TaskResolution::new(
                    claimed[0].lease.clone(),
                    vec![Task::new(Request::new("https://example.com/follow-a"))],
                ),
                TaskResolution::new(
                    claimed[1].lease.clone(),
                    vec![Task::new(Request::new("https://example.com/follow-b"))],
                ),
            ])
            .await
            .unwrap();

        let follow = scheduler.take_batch_ready(2).await.unwrap();
        assert_eq!(follow.len(), 2);
        scheduler
            .complete_batch(follow.into_iter().map(|task| task.lease).collect())
            .await
            .unwrap();

        assert_eq!(scheduler.counts().await.unwrap(), Counts::default());
        scheduler.close().await.unwrap();
        cleanup_test_db(&path);
    }

    #[tokio::test]
    async fn sqlite_scheduler_snapshot_and_control_stay_uniform() {
        let path = test_db_path("snapshot_control");
        let news = Sqlite::new(&path, "jobs:news").with_worker(Worker::new("worker-news"));
        let blog = Sqlite::new(&path, "jobs:blog").with_worker(Worker::new("worker-blog"));

        news.enqueue(Task::new(Request::new("https://example.com/news-ready")))
            .await
            .unwrap();
        blog.enqueue(Task::with_delay(
            Request::new("https://example.com/blog-delayed"),
            500,
        ))
        .await
        .unwrap();
        let claimed = news.take_ready().await.unwrap().unwrap();

        let scopes = news.scopes_with_prefix("jobs:").await.unwrap();
        assert_eq!(
            scopes,
            vec!["jobs:blog".to_string(), "jobs:news".to_string()]
        );

        let snapshots = news.snapshots_with_prefix("jobs:").await.unwrap();
        assert_eq!(snapshots.len(), 2);

        let overview = news.overview_with_prefix("jobs:").await.unwrap();
        assert_eq!(overview.scope_count, 2);
        assert_eq!(overview.pending_scope_count, 2);

        assert!(news.pause_scope("jobs:news").await.unwrap());
        assert!(!news.pause_scope("jobs:news").await.unwrap());
        assert!(news.take_ready().await.unwrap().is_none());
        assert!(news.resume_scope("jobs:news").await.unwrap());

        assert_eq!(news.release_scope("jobs:news").await.unwrap(), 1);
        let reclaimed = news.take_ready().await.unwrap().unwrap();
        assert_eq!(reclaimed.task.id, claimed.task.id);
        news.complete(&reclaimed.lease).await.unwrap();

        let removed = blog.purge_scope("jobs:blog").await.unwrap();
        assert_eq!(
            removed,
            Counts {
                ready: 0,
                delayed: 1,
                inflight: 0,
            }
        );
        assert_eq!(blog.snapshot().await.unwrap().counts, Counts::default());

        news.close().await.unwrap();
        blog.close().await.unwrap();
        cleanup_test_db(&path);
    }

    #[tokio::test]
    async fn sqlite_scheduler_reclaims_stale_leases_and_respects_heartbeat() {
        let path = test_db_path("lease_heartbeat");
        let first = Sqlite::new(&path, "jobs:lease").with_worker(
            Worker::new("worker-a").with_lease_timeout(SignedDuration::from_millis(40)),
        );
        let second = Sqlite::new(&path, "jobs:lease").with_worker(
            Worker::new("worker-b").with_lease_timeout(SignedDuration::from_millis(40)),
        );

        first
            .enqueue(Task::new(Request::new("https://example.com/stale")))
            .await
            .unwrap();

        let claimed = first.take_ready().await.unwrap().unwrap();
        sleep(Duration::from_millis(15)).await;
        first.heartbeat(&claimed.lease).await.unwrap();
        sleep(Duration::from_millis(15)).await;
        assert!(second.take_ready().await.unwrap().is_none());
        sleep(Duration::from_millis(35)).await;

        let reclaimed = second.take_ready().await.unwrap().unwrap();
        assert_eq!(reclaimed.task.id, claimed.task.id);

        let stale_error = first.complete(&claimed.lease).await.unwrap_err();
        assert_eq!(
            stale_error,
            SpiderError::scheduler(SchedulerError::LeaseOwnershipConflict {
                action: "complete",
                task_id: claimed.task.id.as_str().to_string(),
                worker_id: "worker-a".to_string(),
            })
        );

        let snapshot = second.snapshot().await.unwrap();
        assert_eq!(snapshot.reclaimed_total, 1);

        second.complete(&reclaimed.lease).await.unwrap();
        first.close().await.unwrap();
        second.close().await.unwrap();
        cleanup_test_db(&path);
    }

    #[tokio::test]
    async fn sqlite_scheduler_rejects_foreign_scope_control() {
        let path = test_db_path("foreign_scope");
        let scheduler =
            Sqlite::new(&path, "jobs:a").with_worker(Worker::new("sqlite-control-worker"));

        scheduler.snapshot().await.unwrap();
        let error = scheduler.pause_scope("jobs:missing").await.unwrap_err();
        assert_eq!(
            error,
            SpiderError::scheduler(
                "scheduler scope `jobs:missing` is not visible from current backend"
            )
        );

        scheduler.close().await.unwrap();
        cleanup_test_db(&path);
    }

    fn test_db_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "kun-scheduler-{name}-{}-{}.db",
            std::process::id(),
            NEXT_SCOPE_ID.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        path
    }

    fn cleanup_test_db(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(wal_path(path));
        let _ = std::fs::remove_file(shm_path(path));
    }

    fn wal_path(path: &Path) -> PathBuf {
        PathBuf::from(format!("{}-wal", path.display()))
    }

    fn shm_path(path: &Path) -> PathBuf {
        PathBuf::from(format!("{}-shm", path.display()))
    }
}
