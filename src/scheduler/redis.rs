use crate::error::SpiderError;
use crate::redis::{Connection, ErrorContext, connect, query, validate_url};
use crate::scheduler::checkpoint::{Checkpoint, Counts};
use crate::scheduler::snapshot::{InflightTaskSnapshot, Snapshot, WorkerSnapshot};
use crate::scheduler::{ClaimedTask, Scheduler, Task, TaskId, TaskLease, Worker};
use jiff::{SignedDuration, Timestamp};
use redis::FromRedisValue;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tokio::sync::Mutex;

const DEFAULT_LEASE_TIMEOUT: u64 = 300_000;
static NEXT_WORKER_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_LEASE_ID: AtomicU64 = AtomicU64::new(1);
const SCHEDULER_NAMESPACE_REGISTRY_KEY: &str = "kun:scheduler:namespaces:v1";
const META_LEASE_TIMEOUT: &str = "lease_timeout";
const META_HEARTBEAT_INTERVAL: &str = "heartbeat_interval";
const SCHEDULER_ENQUEUE_SCRIPT: &str = r#"
-- kun:scheduler:enqueue_v1
local tasks = KEYS[1]
local ready = KEYS[2]
local ready_order = KEYS[3]
local delayed = KEYS[4]
local sequence = KEYS[5]

local task_id = ARGV[1]
local task_json = ARGV[2]
local ready_at = ARGV[3]

local function push_ready(id)
    local next_ready_order = redis.call('INCR', sequence)
    redis.call('SADD', ready, id)
    redis.call('HSET', ready_order, id, tostring(next_ready_order))
end

redis.call('HSET', tasks, task_id, task_json)

if ready_at == '' then
    push_ready(task_id)
else
    redis.call('ZADD', delayed, ready_at, task_id)
end

return 1
"#;
const SCHEDULER_RECLAIM_SCRIPT: &str = r#"
-- kun:scheduler:reclaim_v2
local tasks = KEYS[1]
local ready = KEYS[2]
local ready_order = KEYS[3]
local delayed = KEYS[4]
local inflight = KEYS[5]
local inflight_deadlines = KEYS[6]
local inflight_workers = KEYS[7]
local inflight_leases = KEYS[8]
local sequence = KEYS[9]
local reclaimed_total = KEYS[10]

local now = tonumber(ARGV[1])
local reclaimed = 0

local function push_ready(id)
    local next_ready_order = redis.call('INCR', sequence)
    redis.call('SADD', ready, id)
    redis.call('HSET', ready_order, id, tostring(next_ready_order))
end

local function sync_worker_runtime()
    redis.call('SADD', workers, worker_id)
    redis.call('HSET', worker_seen, worker_id, tostring(now))

    if lease_timeout ~= '' then
        redis.call('HSET', worker_lease_timeout, worker_id, lease_timeout)
    else
        redis.call('HDEL', worker_lease_timeout, worker_id)
    end

    if heartbeat_interval ~= '' then
        redis.call('HSET', worker_heartbeat_interval, worker_id, heartbeat_interval)
    else
        redis.call('HDEL', worker_heartbeat_interval, worker_id)
    end
end

local function route_task(id, task_json)
    local ok, task = pcall(cjson.decode, task_json)
    if not ok then
        return
    end

    local ready_at = task['ready_at']
    if (not ready_at) or tonumber(ready_at) <= now then
        push_ready(id)
    else
        redis.call('ZADD', delayed, tostring(ready_at), id)
    end
end

local expired_ids = redis.call('ZRANGEBYSCORE', inflight_deadlines, '-inf', tostring(now))
for _, task_id in ipairs(expired_ids) do
    local removed_deadline = redis.call('ZREM', inflight_deadlines, task_id)
    local removed_inflight = redis.call('SREM', inflight, task_id)
    redis.call('HDEL', inflight_workers, task_id)
    redis.call('HDEL', inflight_leases, task_id)
    if removed_deadline > 0 or removed_inflight > 0 then
        local task_json = redis.call('HGET', tasks, task_id)
        if task_json then
            route_task(task_id, task_json)
            reclaimed = reclaimed + 1
        end
    end
end

for _ = 1, reclaimed do
    redis.call('INCR', reclaimed_total)
end

return reclaimed
"#;
const SCHEDULER_CLAIM_READY_SCRIPT: &str = r#"
-- kun:scheduler:claim_ready_v3
local tasks = KEYS[1]
local ready = KEYS[2]
local ready_order = KEYS[3]
local delayed = KEYS[4]
local inflight = KEYS[5]
local inflight_deadlines = KEYS[6]
local inflight_workers = KEYS[7]
local inflight_leases = KEYS[8]
local sequence = KEYS[9]
local reclaimed_total = KEYS[10]
local workers = KEYS[11]
local worker_seen = KEYS[12]
local worker_lease_timeout = KEYS[13]
local worker_heartbeat_interval = KEYS[14]

local now = tonumber(ARGV[1])
local lease_timeout = ARGV[2]
local worker_id = ARGV[3]
local lease_id = ARGV[4]
local heartbeat_interval = ARGV[5]
local max_ready_order = 9007199254740991

local function push_ready(id)
    local next_ready_order = redis.call('INCR', sequence)
    redis.call('SADD', ready, id)
    redis.call('HSET', ready_order, id, tostring(next_ready_order))
end

local function route_task(id, task_json)
    local ok, task = pcall(cjson.decode, task_json)
    if not ok then
        return
    end

    local ready_at = task['ready_at']
    if (not ready_at) or tonumber(ready_at) <= now then
        push_ready(id)
    else
        redis.call('ZADD', delayed, tostring(ready_at), id)
    end
end

local expired_ids = redis.call('ZRANGEBYSCORE', inflight_deadlines, '-inf', tostring(now))
local reclaimed = 0
for _, task_id in ipairs(expired_ids) do
    local removed_deadline = redis.call('ZREM', inflight_deadlines, task_id)
    local removed_inflight = redis.call('SREM', inflight, task_id)
    redis.call('HDEL', inflight_workers, task_id)
    redis.call('HDEL', inflight_leases, task_id)
    if removed_deadline > 0 or removed_inflight > 0 then
        local task_json = redis.call('HGET', tasks, task_id)
        if task_json then
            route_task(task_id, task_json)
            reclaimed = reclaimed + 1
        end
    end
end

for _ = 1, reclaimed do
    redis.call('INCR', reclaimed_total)
end

local delayed_ids = redis.call('ZRANGEBYSCORE', delayed, '-inf', tostring(now))
for _, task_id in ipairs(delayed_ids) do
    if redis.call('ZREM', delayed, task_id) > 0 then
        local task_json = redis.call('HGET', tasks, task_id)
        if task_json then
            push_ready(task_id)
        end
    end
end

local best_task_id = nil
local best_task_json = nil
local best_priority = nil
local best_depth = nil
local best_order = nil

local ready_ids = redis.call('SMEMBERS', ready)
for _, task_id in ipairs(ready_ids) do
    local task_json = redis.call('HGET', tasks, task_id)
    if not task_json then
        redis.call('SREM', ready, task_id)
        redis.call('HDEL', ready_order, task_id)
    else
        local ok, task = pcall(cjson.decode, task_json)
        if not ok then
            redis.call('SREM', ready, task_id)
            redis.call('HDEL', ready_order, task_id)
        else
            local priority = tonumber(task['priority']) or 0
            local depth = tonumber(task['depth']) or 0
            local order = tonumber(redis.call('HGET', ready_order, task_id)) or max_ready_order
            if (not best_task_id)
                or priority > best_priority
                or (priority == best_priority and depth < best_depth)
                or (priority == best_priority and depth == best_depth and order < best_order)
                or (priority == best_priority and depth == best_depth and order == best_order and task_id < best_task_id)
            then
                best_task_id = task_id
                best_task_json = task_json
                best_priority = priority
                best_depth = depth
                best_order = order
            end
        end
    end
end

if not best_task_id then
    local known_worker = redis.call('HGET', worker_seen, worker_id)
    if known_worker then
        sync_worker_runtime()
    end
    return nil
end

redis.call('SREM', ready, best_task_id)
redis.call('HDEL', ready_order, best_task_id)
redis.call('SADD', inflight, best_task_id)
redis.call('ZREM', inflight_deadlines, best_task_id)
redis.call('HSET', inflight_workers, best_task_id, worker_id)
redis.call('HSET', inflight_leases, best_task_id, lease_id)
sync_worker_runtime()

if lease_timeout ~= '' then
    local deadline = now + tonumber(lease_timeout)
    redis.call('ZADD', inflight_deadlines, tostring(deadline), best_task_id)
end

return {best_task_json, lease_id}
"#;
const SCHEDULER_COMPLETE_SCRIPT: &str = r#"
-- kun:scheduler:complete_v4
local tasks = KEYS[1]
local ready = KEYS[2]
local ready_order = KEYS[3]
local delayed = KEYS[4]
local inflight = KEYS[5]
local inflight_deadlines = KEYS[6]
local inflight_workers = KEYS[7]
local inflight_leases = KEYS[8]
local workers = KEYS[9]
local worker_seen = KEYS[10]
local worker_lease_timeout = KEYS[11]
local worker_heartbeat_interval = KEYS[12]

local task_id = ARGV[1]
local worker_id = ARGV[2]
local lease_id = ARGV[3]
local now = ARGV[4]
local lease_timeout = ARGV[5]
local heartbeat_interval = ARGV[6]

local function sync_worker_runtime()
    redis.call('SADD', workers, worker_id)
    redis.call('HSET', worker_seen, worker_id, now)
    if lease_timeout == '' then
        redis.call('HDEL', worker_lease_timeout, worker_id)
    else
        redis.call('HSET', worker_lease_timeout, worker_id, lease_timeout)
    end
    if heartbeat_interval == '' then
        redis.call('HDEL', worker_heartbeat_interval, worker_id)
    else
        redis.call('HSET', worker_heartbeat_interval, worker_id, heartbeat_interval)
    end
end

local current_worker = redis.call('HGET', inflight_workers, task_id)
local current_lease = redis.call('HGET', inflight_leases, task_id)

if (not current_worker) or (not current_lease) then
    return 0
end

if current_worker ~= worker_id then
    return -1
end

if current_lease ~= lease_id then
    return -2
end

sync_worker_runtime()
redis.call('ZREM', inflight_deadlines, task_id)
redis.call('SREM', inflight, task_id)
redis.call('SREM', ready, task_id)
redis.call('ZREM', delayed, task_id)
redis.call('HDEL', ready_order, task_id)
redis.call('HDEL', inflight_workers, task_id)
redis.call('HDEL', inflight_leases, task_id)
redis.call('HDEL', tasks, task_id)
return 1
"#;
const SCHEDULER_REQUEUE_SCRIPT: &str = r#"
-- kun:scheduler:requeue_v4
local tasks = KEYS[1]
local ready = KEYS[2]
local ready_order = KEYS[3]
local delayed = KEYS[4]
local inflight = KEYS[5]
local inflight_deadlines = KEYS[6]
local inflight_workers = KEYS[7]
local inflight_leases = KEYS[8]
local sequence = KEYS[9]
local workers = KEYS[10]
local worker_seen = KEYS[11]
local worker_lease_timeout = KEYS[12]
local worker_heartbeat_interval = KEYS[13]

local task_id = ARGV[1]
local now = tonumber(ARGV[2])
local worker_id = ARGV[3]
local lease_id = ARGV[4]
local worker_seen_now = ARGV[5]
local lease_timeout = ARGV[6]
local heartbeat_interval = ARGV[7]

local function push_ready(id)
    local next_ready_order = redis.call('INCR', sequence)
    redis.call('SADD', ready, id)
    redis.call('HSET', ready_order, id, tostring(next_ready_order))
end

local function sync_worker_runtime()
    redis.call('SADD', workers, worker_id)
    redis.call('HSET', worker_seen, worker_id, worker_seen_now)
    if lease_timeout == '' then
        redis.call('HDEL', worker_lease_timeout, worker_id)
    else
        redis.call('HSET', worker_lease_timeout, worker_id, lease_timeout)
    end
    if heartbeat_interval == '' then
        redis.call('HDEL', worker_heartbeat_interval, worker_id)
    else
        redis.call('HSET', worker_heartbeat_interval, worker_id, heartbeat_interval)
    end
end

local current_worker = redis.call('HGET', inflight_workers, task_id)
local current_lease = redis.call('HGET', inflight_leases, task_id)

if (not current_worker) or (not current_lease) then
    return 0
end

if current_worker ~= worker_id then
    return -1
end

if current_lease ~= lease_id then
    return -2
end

sync_worker_runtime()
local task_json = redis.call('HGET', tasks, task_id)

redis.call('SREM', inflight, task_id)
redis.call('ZREM', inflight_deadlines, task_id)
redis.call('HDEL', inflight_workers, task_id)
redis.call('HDEL', inflight_leases, task_id)
redis.call('SREM', ready, task_id)
redis.call('HDEL', ready_order, task_id)
redis.call('ZREM', delayed, task_id)

if not task_json then
    return 0
end

local ok, task = pcall(cjson.decode, task_json)
if not ok then
    return redis.error_reply('ERR invalid task payload')
end

local ready_at = task['ready_at']
if (not ready_at) or tonumber(ready_at) <= now then
    push_ready(task_id)
else
    redis.call('ZADD', delayed, tostring(ready_at), task_id)
end

return 1
"#;
const SCHEDULER_HEARTBEAT_SCRIPT: &str = r#"
-- kun:scheduler:heartbeat_v3
local inflight = KEYS[1]
local inflight_deadlines = KEYS[2]
local inflight_workers = KEYS[3]
local inflight_leases = KEYS[4]
local workers = KEYS[5]
local worker_seen = KEYS[6]
local worker_lease_timeout = KEYS[7]
local worker_heartbeat_interval = KEYS[8]

local task_id = ARGV[1]
local deadline = ARGV[2]
local worker_id = ARGV[3]
local lease_id = ARGV[4]
local now = ARGV[5]
local lease_timeout = ARGV[6]
local heartbeat_interval = ARGV[7]

local function sync_worker_runtime()
    redis.call('SADD', workers, worker_id)
    redis.call('HSET', worker_seen, worker_id, now)
    if lease_timeout == '' then
        redis.call('HDEL', worker_lease_timeout, worker_id)
    else
        redis.call('HSET', worker_lease_timeout, worker_id, lease_timeout)
    end
    if heartbeat_interval == '' then
        redis.call('HDEL', worker_heartbeat_interval, worker_id)
    else
        redis.call('HSET', worker_heartbeat_interval, worker_id, heartbeat_interval)
    end
end

local current_worker = redis.call('HGET', inflight_workers, task_id)
local current_lease = redis.call('HGET', inflight_leases, task_id)

if (not current_worker) or (not current_lease) then
    return 0
end

if current_worker ~= worker_id then
    return -1
end

if current_lease ~= lease_id then
    return -2
end

if redis.call('SREM', inflight, task_id) == 0 then
    return 0
end
redis.call('SADD', inflight, task_id)
redis.call('ZADD', inflight_deadlines, deadline, task_id)
sync_worker_runtime()
return 1
"#;
const SCHEDULER_RELEASE_INFLIGHT_SCRIPT: &str = r#"
-- kun:scheduler:release_inflight_v1
local tasks = KEYS[1]
local ready = KEYS[2]
local ready_order = KEYS[3]
local delayed = KEYS[4]
local inflight = KEYS[5]
local inflight_deadlines = KEYS[6]
local inflight_workers = KEYS[7]
local inflight_leases = KEYS[8]
local sequence = KEYS[9]

local now = tonumber(ARGV[1])
local worker_id = ARGV[2]
local released = 0

local function push_ready(id)
    local next_ready_order = redis.call('INCR', sequence)
    redis.call('SADD', ready, id)
    redis.call('HSET', ready_order, id, tostring(next_ready_order))
end

local function route_task(id, task_json)
    local ok, task = pcall(cjson.decode, task_json)
    if not ok then
        return
    end

    local ready_at = task['ready_at']
    if (not ready_at) or tonumber(ready_at) <= now then
        push_ready(id)
    else
        redis.call('ZADD', delayed, tostring(ready_at), id)
    end
end

local worker_entries = redis.call('HGETALL', inflight_workers)
for index = 1, #worker_entries, 2 do
    local task_id = worker_entries[index]
    local owner = worker_entries[index + 1]
    if owner == worker_id then
        local task_json = redis.call('HGET', tasks, task_id)
        local removed = redis.call('SREM', inflight, task_id)
        redis.call('ZREM', inflight_deadlines, task_id)
        redis.call('HDEL', inflight_workers, task_id)
        redis.call('HDEL', inflight_leases, task_id)
        if removed > 0 and task_json then
            route_task(task_id, task_json)
            released = released + 1
        end
    end
end

return released
"#;

#[derive(Debug, Clone)]
/// Redis-backed durable scheduler.
///
/// This scheduler persists `ready / delayed / inflight` buckets directly in
/// Redis. Unlike checkpoint persistence, it also owns runtime recovery
/// semantics such as reclaiming stale `inflight` tasks after a lease timeout.
pub struct Redis {
    url: String,
    namespace: String,
    worker: Worker,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl Redis {
    pub fn new(url: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            namespace: namespace.into(),
            worker: Worker::new(next_worker_id()).with_lease_timeout(SignedDuration::from_millis(
                i64::try_from(DEFAULT_LEASE_TIMEOUT).unwrap_or(i64::MAX),
            )),
            connection: Arc::new(Mutex::new(None)),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn scope(&self) -> &str {
        &self.namespace
    }

    pub fn worker_id(&self) -> &str {
        self.worker.worker_id()
    }

    pub fn worker(&self) -> &Worker {
        &self.worker
    }

    pub fn with_worker(mut self, worker: Worker) -> Self {
        self.worker = worker;
        self
    }

    pub async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        self.read_checkpoint().await
    }

    pub async fn counts(&self) -> Result<Counts, SpiderError> {
        self.read_counts().await
    }

    /// Reads one scope-level runtime snapshot for this Redis durable
    /// scheduler.
    pub async fn snapshot(&self) -> Result<Snapshot, SpiderError> {
        self.read_snapshot().await
    }

    /// Lists scheduler scopes already observed in the current Redis backend.
    pub async fn scopes(&self) -> Result<Vec<String>, SpiderError> {
        self.read_scopes_with_prefix("").await
    }

    /// Lists scheduler scopes whose names start with `prefix`.
    pub async fn scopes_with_prefix(
        &self,
        prefix: impl AsRef<str>,
    ) -> Result<Vec<String>, SpiderError> {
        self.read_scopes_with_prefix(prefix.as_ref()).await
    }

    /// Reads scope-level snapshots for every visible scope in the current
    /// Redis backend.
    pub async fn snapshots(&self) -> Result<Vec<Snapshot>, SpiderError> {
        self.read_snapshots_with_prefix("").await
    }

    /// Reads scope-level snapshots for every visible scope whose name starts
    /// with `prefix`.
    pub async fn snapshots_with_prefix(
        &self,
        prefix: impl AsRef<str>,
    ) -> Result<Vec<Snapshot>, SpiderError> {
        self.read_snapshots_with_prefix(prefix.as_ref()).await
    }

    pub async fn close(&self) -> Result<(), SpiderError> {
        self.close_runtime().await
    }

    fn validate(&self) -> Result<(), SpiderError> {
        validate_url(&self.url, "redis scheduler", ErrorContext::Scheduler)?;

        if self.namespace.trim().is_empty() {
            return Err(SpiderError::scheduler(
                "redis scheduler namespace cannot be empty",
            ));
        }

        if self.worker_id().trim().is_empty() {
            return Err(SpiderError::scheduler(
                "redis scheduler worker_id cannot be empty",
            ));
        }

        Ok(())
    }

    async fn connection(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<Connection>>, SpiderError> {
        self.validate()?;
        let mut guard = self.connection.lock().await;

        if guard.is_none() {
            *guard = Some(connect(&self.url, "redis scheduler", ErrorContext::Scheduler).await?);
        }

        Ok(guard)
    }

    fn connection_mut<'a>(
        &self,
        guard: &'a mut tokio::sync::MutexGuard<'_, Option<Connection>>,
    ) -> Result<&'a mut Connection, SpiderError> {
        guard.as_mut().ok_or_else(|| {
            SpiderError::scheduler("redis scheduler connection is missing after initialization")
        })
    }

    fn keys(&self) -> Keys {
        Keys::for_namespace(&self.namespace)
    }

    fn heartbeat_interval_millis(&self) -> Option<u64> {
        let lease_timeout = self.lease_timeout_millis()?;
        let default = Some(signed_duration_from_millis(default_heartbeat_interval(
            lease_timeout,
        )));
        self.worker
            .effective_heartbeat_interval(default)
            .map(non_negative_milliseconds)
    }

    fn lease_timeout_millis(&self) -> Option<u64> {
        self.worker
            .effective_lease_timeout(Some(signed_duration_from_millis(DEFAULT_LEASE_TIMEOUT)))
            .map(non_negative_milliseconds)
    }

    async fn read_checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.sync_namespace_metadata(connection).await?;
        let _ = self.reclaim_expired_inflight(connection).await?;
        let keys = self.keys();

        let ready_tasks = self.load_ready_tasks(connection).await?;
        let delayed_ids: Vec<String> = scheduler_query(
            connection,
            redis_command("ZRANGE", [&keys.delayed, "0", "-1"]),
        )
        .await?;
        let delayed = self.load_tasks(connection, &delayed_ids).await?;

        let mut inflight_ids: Vec<String> =
            scheduler_query(connection, redis_command("SMEMBERS", [&keys.inflight])).await?;
        inflight_ids.sort();
        let inflight = self.load_tasks(connection, &inflight_ids).await?;

        Ok(Checkpoint {
            ready: ready_tasks.into_iter().map(|(task, _)| task).collect(),
            delayed,
            inflight,
        })
    }

    async fn read_counts(&self) -> Result<Counts, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.sync_namespace_metadata(connection).await?;
        let _ = self.reclaim_expired_inflight(connection).await?;
        let keys = self.keys();

        let ready: usize =
            scheduler_query(connection, redis_command("SCARD", [&keys.ready])).await?;
        let delayed: usize =
            scheduler_query(connection, redis_command("ZCARD", [&keys.delayed])).await?;
        let inflight: usize =
            scheduler_query(connection, redis_command("SCARD", [&keys.inflight])).await?;

        Ok(Counts {
            ready,
            delayed,
            inflight,
        })
    }

    async fn read_snapshot(&self) -> Result<Snapshot, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.sync_namespace_metadata(connection).await?;
        read_scope_snapshot(connection, self.scope()).await
    }

    async fn read_scopes_with_prefix(&self, prefix: &str) -> Result<Vec<String>, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        load_registry_namespaces(connection, prefix).await
    }

    async fn read_snapshots_with_prefix(&self, prefix: &str) -> Result<Vec<Snapshot>, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        let scopes = load_registry_namespaces(connection, prefix).await?;
        let mut snapshots = Vec::with_capacity(scopes.len());
        for scope in scopes {
            snapshots.push(read_scope_snapshot(connection, &scope).await?);
        }
        Ok(snapshots)
    }

    async fn close_runtime(&self) -> Result<(), SpiderError> {
        let mut guard = self.connection.lock().await;
        let Some(connection) = guard.as_mut() else {
            return Ok(());
        };
        let keys = self.keys();
        clear_worker_runtime_if_idle(connection, &keys, self.worker_id()).await?;
        guard.take();
        Ok(())
    }

    fn validate_lease_worker(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        if lease.worker_id() != self.worker_id() {
            return Err(SpiderError::scheduler(
                crate::error::SchedulerError::LeaseWorkerMismatch {
                    lease_worker_id: lease.worker_id().to_string(),
                    current_worker_id: self.worker_id().to_string(),
                },
            ));
        }

        Ok(())
    }

    async fn enqueue_internal(
        &self,
        connection: &mut Connection,
        task: Task,
    ) -> Result<(), SpiderError> {
        let keys = self.keys();
        let task_json = serde_json::to_string(&task).map_err(|error| {
            SpiderError::scheduler(format!("failed to encode redis scheduler task: {error}"))
        })?;
        let ready_at = task
            .ready_at
            .map(|value| value.to_string())
            .unwrap_or_default();

        let _: i64 = scheduler_eval(
            connection,
            SCHEDULER_ENQUEUE_SCRIPT,
            &[
                keys.tasks.as_str(),
                keys.ready.as_str(),
                keys.ready_order.as_str(),
                keys.delayed.as_str(),
                keys.sequence.as_str(),
            ],
            &[task.id.as_str(), task_json.as_str(), ready_at.as_str()],
        )
        .await?;
        Ok(())
    }

    async fn sync_namespace_metadata(
        &self,
        connection: &mut Connection,
    ) -> Result<(), SpiderError> {
        let _: i64 = scheduler_query(
            connection,
            redis_command(
                "SADD",
                [SCHEDULER_NAMESPACE_REGISTRY_KEY, self.namespace.as_str()],
            ),
        )
        .await?;
        Ok(())
    }

    async fn sync_scope_metadata(&self, connection: &mut Connection) -> Result<(), SpiderError> {
        let keys = self.keys();
        self.sync_namespace_metadata(connection).await?;

        sync_namespace_meta_field(
            connection,
            &keys.meta,
            META_LEASE_TIMEOUT,
            self.lease_timeout_millis(),
        )
        .await?;
        sync_namespace_meta_field(
            connection,
            &keys.meta,
            META_HEARTBEAT_INTERVAL,
            self.heartbeat_interval_millis(),
        )
        .await
    }

    async fn reclaim_expired_inflight(
        &self,
        connection: &mut Connection,
    ) -> Result<u64, SpiderError> {
        let keys = self.keys();
        let reclaimed = reclaim_expired_inflight_for_keys(connection, &keys).await?;

        for _ in 0..usize::try_from(reclaimed).unwrap_or_default() {
            tracing::warn!("redis scheduler reclaimed stale inflight task");
        }

        Ok(reclaimed)
    }

    async fn load_ready_tasks(
        &self,
        connection: &mut Connection,
    ) -> Result<Vec<(Task, i64)>, SpiderError> {
        let keys = self.keys();
        let ready_ids: Vec<String> =
            scheduler_query(connection, redis_command("SMEMBERS", [&keys.ready])).await?;

        let mut tasks = Vec::with_capacity(ready_ids.len());
        for task_id in ready_ids {
            let task = self.load_task(connection, &task_id).await?;
            let ready_order = self.load_ready_order(connection, &task_id).await?;
            tasks.push((task, ready_order));
        }
        tasks.sort_by(ready_task_ordering);
        Ok(tasks)
    }

    async fn load_ready_order(
        &self,
        connection: &mut Connection,
        task_id: &str,
    ) -> Result<i64, SpiderError> {
        let keys = self.keys();
        let ready_order: Option<i64> = scheduler_query(
            connection,
            redis_command("HGET", [&keys.ready_order, task_id]),
        )
        .await?;
        Ok(ready_order.unwrap_or(i64::MAX))
    }

    async fn load_tasks(
        &self,
        connection: &mut Connection,
        task_ids: &[String],
    ) -> Result<Vec<Task>, SpiderError> {
        let mut tasks = Vec::with_capacity(task_ids.len());
        for task_id in task_ids {
            tasks.push(self.load_task(connection, task_id).await?);
        }
        Ok(tasks)
    }

    async fn load_task(
        &self,
        connection: &mut Connection,
        task_id: &str,
    ) -> Result<Task, SpiderError> {
        let keys = self.keys();
        let task_json: Option<String> =
            scheduler_query(connection, redis_command("HGET", [&keys.tasks, task_id])).await?;
        let task_json = task_json.ok_or_else(|| {
            SpiderError::scheduler(format!(
                "redis scheduler task payload is missing for task id {task_id}"
            ))
        })?;

        serde_json::from_str(&task_json).map_err(|error| {
            SpiderError::scheduler(format!("failed to decode redis scheduler task: {error}"))
        })
    }

    async fn heartbeat_internal(
        &self,
        connection: &mut Connection,
        lease: &TaskLease,
    ) -> Result<(), SpiderError> {
        let Some(heartbeat_interval) = self.heartbeat_interval_millis() else {
            return Ok(());
        };

        let Some(lease_timeout) = self.lease_timeout_millis() else {
            return Ok(());
        };

        let _ = heartbeat_interval;
        let keys = self.keys();
        let current_time = now();
        let worker_seen = current_time.to_string();
        let deadline = current_time
            .saturating_add(i64::try_from(lease_timeout).unwrap_or(i64::MAX))
            .to_string();
        let lease_timeout = lease_timeout.to_string();
        let heartbeat_interval = heartbeat_interval.to_string();
        let result: i64 = scheduler_eval(
            connection,
            SCHEDULER_HEARTBEAT_SCRIPT,
            &[
                keys.inflight.as_str(),
                keys.inflight_deadlines.as_str(),
                keys.inflight_workers.as_str(),
                keys.inflight_leases.as_str(),
                keys.workers.as_str(),
                keys.worker_seen.as_str(),
                keys.worker_lease_timeout.as_str(),
                keys.worker_heartbeat_interval.as_str(),
            ],
            &[
                lease.task_id().as_str(),
                deadline.as_str(),
                lease.worker_id(),
                lease.lease_id(),
                worker_seen.as_str(),
                lease_timeout.as_str(),
                heartbeat_interval.as_str(),
            ],
        )
        .await?;
        ensure_lease_transition("heartbeat", lease, result)
    }
}

impl Scheduler for Redis {
    async fn enqueue(&self, task: Task) -> Result<(), SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.sync_scope_metadata(connection).await?;
        self.enqueue_internal(connection, task).await
    }

    async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        self.read_checkpoint().await
    }

    async fn counts(&self) -> Result<Counts, SpiderError> {
        self.read_counts().await
    }

    async fn snapshot(&self) -> Result<Snapshot, SpiderError> {
        self.read_snapshot().await
    }

    async fn scopes(&self) -> Result<Vec<String>, SpiderError> {
        self.read_scopes_with_prefix("").await
    }

    async fn scopes_with_prefix(&self, prefix: &str) -> Result<Vec<String>, SpiderError> {
        self.read_scopes_with_prefix(prefix).await
    }

    async fn snapshots(&self) -> Result<Vec<Snapshot>, SpiderError> {
        self.read_snapshots_with_prefix("").await
    }

    async fn snapshots_with_prefix(&self, prefix: &str) -> Result<Vec<Snapshot>, SpiderError> {
        self.read_snapshots_with_prefix(prefix).await
    }

    async fn take_ready(&self) -> Result<Option<ClaimedTask>, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.sync_scope_metadata(connection).await?;
        let keys = self.keys();
        let now = now().to_string();
        let lease_timeout = self
            .lease_timeout_millis()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let heartbeat_interval = self
            .heartbeat_interval_millis()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let lease_id = next_lease_id(self.worker_id());
        let result: Option<Vec<String>> = scheduler_eval(
            connection,
            SCHEDULER_CLAIM_READY_SCRIPT,
            &[
                keys.tasks.as_str(),
                keys.ready.as_str(),
                keys.ready_order.as_str(),
                keys.delayed.as_str(),
                keys.inflight.as_str(),
                keys.inflight_deadlines.as_str(),
                keys.inflight_workers.as_str(),
                keys.inflight_leases.as_str(),
                keys.sequence.as_str(),
                keys.reclaimed_total.as_str(),
                keys.workers.as_str(),
                keys.worker_seen.as_str(),
                keys.worker_lease_timeout.as_str(),
                keys.worker_heartbeat_interval.as_str(),
            ],
            &[
                now.as_str(),
                lease_timeout.as_str(),
                self.worker_id(),
                lease_id.as_str(),
                heartbeat_interval.as_str(),
            ],
        )
        .await?;

        let Some(result) = result else {
            return Ok(None);
        };
        if result.len() != 2 {
            return Err(SpiderError::scheduler(
                "redis scheduler claim script returned invalid lease payload",
            ));
        }
        let task_json = result[0].as_str();
        let lease_id = result[1].clone();
        let task: Task = serde_json::from_str(task_json).map_err(|error| {
            SpiderError::scheduler(format!("failed to decode redis scheduler task: {error}"))
        })?;
        let lease = TaskLease::new(task.id.clone(), self.worker_id().to_string(), lease_id);

        Ok(Some(ClaimedTask::new(task, lease)))
    }

    async fn complete(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        self.validate_lease_worker(lease)?;
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.sync_scope_metadata(connection).await?;
        let keys = self.keys();
        let worker_seen = now().to_string();
        let lease_timeout = self
            .lease_timeout_millis()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let heartbeat_interval = self
            .heartbeat_interval_millis()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let result: i64 = scheduler_eval(
            connection,
            SCHEDULER_COMPLETE_SCRIPT,
            &[
                keys.tasks.as_str(),
                keys.ready.as_str(),
                keys.ready_order.as_str(),
                keys.delayed.as_str(),
                keys.inflight.as_str(),
                keys.inflight_deadlines.as_str(),
                keys.inflight_workers.as_str(),
                keys.inflight_leases.as_str(),
                keys.workers.as_str(),
                keys.worker_seen.as_str(),
                keys.worker_lease_timeout.as_str(),
                keys.worker_heartbeat_interval.as_str(),
            ],
            &[
                lease.task_id().as_str(),
                lease.worker_id(),
                lease.lease_id(),
                worker_seen.as_str(),
                lease_timeout.as_str(),
                heartbeat_interval.as_str(),
            ],
        )
        .await?;
        ensure_lease_transition("complete", lease, result)
    }

    async fn requeue(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        self.validate_lease_worker(lease)?;
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.sync_scope_metadata(connection).await?;
        let keys = self.keys();
        let now = now().to_string();
        let worker_seen = now.clone();
        let lease_timeout = self
            .lease_timeout_millis()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let heartbeat_interval = self
            .heartbeat_interval_millis()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let result: i64 = scheduler_eval(
            connection,
            SCHEDULER_REQUEUE_SCRIPT,
            &[
                keys.tasks.as_str(),
                keys.ready.as_str(),
                keys.ready_order.as_str(),
                keys.delayed.as_str(),
                keys.inflight.as_str(),
                keys.inflight_deadlines.as_str(),
                keys.inflight_workers.as_str(),
                keys.inflight_leases.as_str(),
                keys.sequence.as_str(),
                keys.workers.as_str(),
                keys.worker_seen.as_str(),
                keys.worker_lease_timeout.as_str(),
                keys.worker_heartbeat_interval.as_str(),
            ],
            &[
                lease.task_id().as_str(),
                now.as_str(),
                lease.worker_id(),
                lease.lease_id(),
                worker_seen.as_str(),
                lease_timeout.as_str(),
                heartbeat_interval.as_str(),
            ],
        )
        .await?;
        ensure_lease_transition("requeue", lease, result)
    }

    async fn release_inflight(&self) -> Result<usize, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.sync_scope_metadata(connection).await?;
        let keys = self.keys();
        let now = now().to_string();
        let released: i64 = scheduler_eval(
            connection,
            SCHEDULER_RELEASE_INFLIGHT_SCRIPT,
            &[
                keys.tasks.as_str(),
                keys.ready.as_str(),
                keys.ready_order.as_str(),
                keys.delayed.as_str(),
                keys.inflight.as_str(),
                keys.inflight_deadlines.as_str(),
                keys.inflight_workers.as_str(),
                keys.inflight_leases.as_str(),
                keys.sequence.as_str(),
            ],
            &[now.as_str(), self.worker_id()],
        )
        .await?;
        clear_worker_runtime_if_idle(connection, &keys, self.worker_id()).await?;
        Ok(usize::try_from(released).unwrap_or_default())
    }

    async fn heartbeat(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        self.validate_lease_worker(lease)?;
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.sync_scope_metadata(connection).await?;
        self.heartbeat_internal(connection, lease).await
    }

    fn heartbeat_interval(&self) -> Option<SignedDuration> {
        let millis = self.heartbeat_interval_millis()?;
        Some(SignedDuration::from_millis(
            i64::try_from(millis).unwrap_or(i64::MAX),
        ))
    }

    async fn close(&self) -> Result<(), SpiderError> {
        self.close_runtime().await
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        Ok(self.counts().await?.has_pending())
    }
}

#[derive(Debug, Clone)]
struct Keys {
    tasks: String,
    ready: String,
    ready_order: String,
    delayed: String,
    inflight: String,
    inflight_deadlines: String,
    inflight_workers: String,
    inflight_leases: String,
    sequence: String,
    reclaimed_total: String,
    meta: String,
    workers: String,
    worker_seen: String,
    worker_lease_timeout: String,
    worker_heartbeat_interval: String,
}

impl Keys {
    fn for_namespace(namespace: &str) -> Self {
        Self {
            tasks: format!("{namespace}:tasks"),
            ready: format!("{namespace}:ready"),
            ready_order: format!("{namespace}:ready_order"),
            delayed: format!("{namespace}:delayed"),
            inflight: format!("{namespace}:inflight"),
            inflight_deadlines: format!("{namespace}:inflight_deadlines"),
            inflight_workers: format!("{namespace}:inflight_workers"),
            inflight_leases: format!("{namespace}:inflight_leases"),
            sequence: format!("{namespace}:ready_sequence"),
            reclaimed_total: format!("{namespace}:reclaimed_total"),
            meta: format!("{namespace}:meta"),
            workers: format!("{namespace}:workers"),
            worker_seen: format!("{namespace}:worker_seen"),
            worker_lease_timeout: format!("{namespace}:worker_lease_timeout"),
            worker_heartbeat_interval: format!("{namespace}:worker_heartbeat_interval"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct NamespaceMeta {
    lease_timeout: Option<u64>,
    heartbeat_interval: Option<u64>,
}

fn ready_task_ordering(left: &(Task, i64), right: &(Task, i64)) -> Ordering {
    right
        .0
        .priority
        .cmp(&left.0.priority)
        .then_with(|| left.0.depth.cmp(&right.0.depth))
        .then_with(|| left.1.cmp(&right.1))
        .then_with(|| left.0.id.as_str().cmp(right.0.id.as_str()))
}

fn now() -> i64 {
    Timestamp::now().as_millisecond()
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

fn signed_duration_from_millis(millis: u64) -> SignedDuration {
    SignedDuration::from_millis(i64::try_from(millis).unwrap_or(i64::MAX))
}

fn next_worker_id() -> String {
    format!(
        "worker-{}-{}-{}",
        std::process::id(),
        now(),
        NEXT_WORKER_ID.fetch_add(1, AtomicOrdering::Relaxed)
    )
}

fn next_lease_id(worker_id: &str) -> String {
    format!(
        "{worker_id}-lease-{}-{}",
        now(),
        NEXT_LEASE_ID.fetch_add(1, AtomicOrdering::Relaxed)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseTransitionResult {
    Success,
    OwnershipConflict,
    StaleLease,
    InactiveLease,
}

impl LeaseTransitionResult {
    fn from_redis_code(code: i64) -> Self {
        match code {
            1 => Self::Success,
            -1 => Self::OwnershipConflict,
            -2 => Self::StaleLease,
            _ => Self::InactiveLease,
        }
    }
}

fn ensure_lease_transition(
    action: &'static str,
    lease: &TaskLease,
    result: i64,
) -> Result<(), SpiderError> {
    match LeaseTransitionResult::from_redis_code(result) {
        LeaseTransitionResult::Success => Ok(()),
        LeaseTransitionResult::OwnershipConflict => Err(SpiderError::scheduler(
            crate::error::SchedulerError::LeaseOwnershipConflict {
                action,
                task_id: lease.task_id().as_str().to_string(),
                worker_id: lease.worker_id().to_string(),
            },
        )),
        LeaseTransitionResult::StaleLease => Err(SpiderError::scheduler(
            crate::error::SchedulerError::StaleLease {
                action,
                task_id: lease.task_id().as_str().to_string(),
                worker_id: lease.worker_id().to_string(),
                lease_id: lease.lease_id().to_string(),
            },
        )),
        LeaseTransitionResult::InactiveLease => Err(SpiderError::scheduler(
            crate::error::SchedulerError::InactiveLease {
                action,
                task_id: lease.task_id().as_str().to_string(),
            },
        )),
    }
}

async fn load_registry_namespaces(
    connection: &mut Connection,
    prefix: &str,
) -> Result<Vec<String>, SpiderError> {
    let mut namespaces: Vec<String> = scheduler_query(
        connection,
        redis_command("SMEMBERS", [SCHEDULER_NAMESPACE_REGISTRY_KEY]),
    )
    .await?;
    namespaces.sort();
    if !prefix.is_empty() {
        namespaces.retain(|namespace| namespace.starts_with(prefix));
    }
    Ok(namespaces)
}

async fn sync_namespace_meta_field(
    connection: &mut Connection,
    meta_key: &str,
    field: &str,
    value: Option<u64>,
) -> Result<(), SpiderError> {
    match value {
        Some(value) => {
            let value = value.to_string();
            let _: i64 =
                scheduler_query(connection, redis_command("HSET", [meta_key, field, &value]))
                    .await?;
        }
        None => {
            let _: i64 =
                scheduler_query(connection, redis_command("HDEL", [meta_key, field])).await?;
        }
    }
    Ok(())
}

async fn clear_worker_runtime_if_idle(
    connection: &mut Connection,
    keys: &Keys,
    worker_id: &str,
) -> Result<(), SpiderError> {
    let inflight_workers = load_hash_entries(connection, &keys.inflight_workers).await?;
    if inflight_workers.values().any(|value| value == worker_id) {
        return Ok(());
    }

    let _: i64 = scheduler_query(
        connection,
        redis_command("SREM", [&keys.workers, worker_id]),
    )
    .await?;
    let _: i64 = scheduler_query(
        connection,
        redis_command("HDEL", [&keys.worker_seen, worker_id]),
    )
    .await?;
    let _: i64 = scheduler_query(
        connection,
        redis_command("HDEL", [&keys.worker_lease_timeout, worker_id]),
    )
    .await?;
    let _: i64 = scheduler_query(
        connection,
        redis_command("HDEL", [&keys.worker_heartbeat_interval, worker_id]),
    )
    .await?;
    Ok(())
}

async fn load_namespace_meta(
    connection: &mut Connection,
    namespace: &str,
) -> Result<NamespaceMeta, SpiderError> {
    let keys = Keys::for_namespace(namespace);
    let lease_timeout = parse_optional_meta_u64(
        namespace,
        META_LEASE_TIMEOUT,
        scheduler_query(
            connection,
            redis_command("HGET", [&keys.meta, META_LEASE_TIMEOUT]),
        )
        .await?,
    )?;
    let heartbeat_interval = parse_optional_meta_u64(
        namespace,
        META_HEARTBEAT_INTERVAL,
        scheduler_query(
            connection,
            redis_command("HGET", [&keys.meta, META_HEARTBEAT_INTERVAL]),
        )
        .await?,
    )?;

    Ok(NamespaceMeta {
        lease_timeout,
        heartbeat_interval: heartbeat_interval
            .or_else(|| lease_timeout.map(default_heartbeat_interval)),
    })
}

fn parse_optional_meta_u64(
    namespace: &str,
    field: &str,
    value: Option<String>,
) -> Result<Option<u64>, SpiderError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }

    value.parse::<u64>().map(Some).map_err(|error| {
        SpiderError::scheduler(format!(
            "redis scheduler metadata `{field}` for namespace `{namespace}` is invalid: {error}"
        ))
    })
}

async fn reclaim_expired_inflight_for_keys(
    connection: &mut Connection,
    keys: &Keys,
) -> Result<u64, SpiderError> {
    let now = now().to_string();
    let reclaimed: i64 = scheduler_eval(
        connection,
        SCHEDULER_RECLAIM_SCRIPT,
        &[
            keys.tasks.as_str(),
            keys.ready.as_str(),
            keys.ready_order.as_str(),
            keys.delayed.as_str(),
            keys.inflight.as_str(),
            keys.inflight_deadlines.as_str(),
            keys.inflight_workers.as_str(),
            keys.inflight_leases.as_str(),
            keys.sequence.as_str(),
            keys.reclaimed_total.as_str(),
        ],
        &[now.as_str()],
    )
    .await?;

    Ok(u64::try_from(reclaimed).unwrap_or_default())
}

async fn load_hash_entries(
    connection: &mut Connection,
    key: &str,
) -> Result<BTreeMap<String, String>, SpiderError> {
    scheduler_query(connection, redis_command("HGETALL", [key])).await
}

async fn load_deadline_entries(
    connection: &mut Connection,
    namespace: &str,
    key: &str,
) -> Result<BTreeMap<String, Timestamp>, SpiderError> {
    let deadline_entries: Vec<String> = scheduler_query(
        connection,
        redis_command("ZRANGE", [key, "0", "-1", "WITHSCORES"]),
    )
    .await?;

    if deadline_entries.len() % 2 != 0 {
        return Err(SpiderError::scheduler(format!(
            "redis scheduler deadline view for namespace `{namespace}` returned an odd number of values"
        )));
    }

    let mut deadlines = BTreeMap::new();
    for entry in deadline_entries.chunks_exact(2) {
        let task_id = entry[0].clone();
        let millis = parse_timestamp_score(namespace, &task_id, "deadline", &entry[1])?;
        deadlines.insert(
            task_id,
            timestamp_from_i64(namespace, &entry[0], "deadline", millis)?,
        );
    }

    Ok(deadlines)
}

async fn load_task_payloads(
    connection: &mut Connection,
    namespace: &str,
    tasks_key: &str,
    task_ids: &[String],
) -> Result<Vec<Task>, SpiderError> {
    if task_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut command = redis::cmd("HMGET");
    command.arg(tasks_key);
    for task_id in task_ids {
        command.arg(task_id);
    }
    let task_jsons: Vec<Option<String>> = scheduler_query(connection, command).await?;

    if task_jsons.len() != task_ids.len() {
        return Err(SpiderError::scheduler(format!(
            "redis scheduler task payload view for namespace `{namespace}` returned {} values for {} task ids",
            task_jsons.len(),
            task_ids.len()
        )));
    }

    let mut tasks = Vec::with_capacity(task_ids.len());
    for (task_id, task_json) in task_ids.iter().zip(task_jsons) {
        let task_json = task_json.ok_or_else(|| {
            SpiderError::scheduler(format!(
                "redis scheduler task payload is missing for namespace `{namespace}` task id `{task_id}`"
            ))
        })?;
        let task: Task = serde_json::from_str(&task_json).map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to decode redis scheduler task for namespace `{namespace}` task id `{task_id}`: {error}"
            ))
        })?;
        if task.id.as_str() != task_id.as_str() {
            return Err(SpiderError::scheduler(format!(
                "redis scheduler task payload id mismatch for namespace `{namespace}`: expected `{task_id}`, got `{}`",
                task.id.as_str()
            )));
        }
        tasks.push(task);
    }

    Ok(tasks)
}

async fn load_inflight_tasks(
    connection: &mut Connection,
    namespace: &str,
    keys: &Keys,
    task_ids: &[String],
) -> Result<Vec<InflightTaskSnapshot>, SpiderError> {
    if task_ids.is_empty() {
        return Ok(Vec::new());
    }

    let tasks = load_task_payloads(connection, namespace, &keys.tasks, task_ids).await?;
    let workers = load_hash_entries(connection, &keys.inflight_workers).await?;
    let leases = load_hash_entries(connection, &keys.inflight_leases).await?;
    let deadlines = load_deadline_entries(connection, namespace, &keys.inflight_deadlines).await?;

    let mut inflight_tasks = Vec::with_capacity(tasks.len());
    for task in tasks {
        let task_id = task.id.clone();
        let ready_at = task
            .ready_at
            .map(|value| timestamp_from_u64(namespace, task_id.as_str(), "ready_at", value))
            .transpose()?;

        inflight_tasks.push(InflightTaskSnapshot {
            task_id: task_id.clone(),
            url: task.request.url,
            worker_id: workers.get(task_id.as_str()).cloned(),
            lease_id: leases.get(task_id.as_str()).cloned(),
            deadline: deadlines.get(task_id.as_str()).cloned(),
            priority: task.priority,
            depth: task.depth,
            ready_at,
        });
    }

    Ok(inflight_tasks)
}

#[derive(Debug, Default)]
struct WorkerAggregate {
    inflight_task_ids: Vec<TaskId>,
    active_lease_count: usize,
    next_deadline: Option<Timestamp>,
}

async fn load_worker_snapshots(
    connection: &mut Connection,
    namespace: &str,
    keys: &Keys,
    inflight_tasks: &[InflightTaskSnapshot],
) -> Result<Vec<WorkerSnapshot>, SpiderError> {
    let worker_ids: Vec<String> =
        scheduler_query(connection, redis_command("SMEMBERS", [&keys.workers])).await?;
    let seen = load_hash_entries(connection, &keys.worker_seen).await?;
    let lease_timeouts = load_hash_entries(connection, &keys.worker_lease_timeout).await?;
    let heartbeat_intervals =
        load_hash_entries(connection, &keys.worker_heartbeat_interval).await?;

    let mut workers = worker_ids.into_iter().collect::<BTreeSet<_>>();
    let mut aggregates = BTreeMap::<String, WorkerAggregate>::new();

    for task in inflight_tasks {
        let Some(worker_id) = task.worker_id.as_ref() else {
            continue;
        };
        workers.insert(worker_id.clone());
        let aggregate = aggregates.entry(worker_id.clone()).or_default();
        aggregate.inflight_task_ids.push(task.task_id.clone());
        if task.lease_id.is_some() {
            aggregate.active_lease_count += 1;
        }
        if let Some(deadline) = task.deadline.as_ref() {
            let replace_deadline = aggregate
                .next_deadline
                .as_ref()
                .map(|current| deadline.as_millisecond() < current.as_millisecond())
                .unwrap_or(true);
            if replace_deadline {
                aggregate.next_deadline = Some(*deadline);
            }
        }
    }

    let now = now();
    let mut snapshots = Vec::with_capacity(workers.len());
    for worker_id in workers {
        let last_seen = seen
            .get(&worker_id)
            .map(|value| parse_worker_timestamp(namespace, &worker_id, "last_seen", value))
            .transpose()?;
        let lease_timeout = lease_timeouts
            .get(&worker_id)
            .map(|value| parse_worker_u64(namespace, &worker_id, "lease_timeout", value))
            .transpose()?;
        let heartbeat_interval = heartbeat_intervals
            .get(&worker_id)
            .map(|value| parse_worker_u64(namespace, &worker_id, "heartbeat_interval", value))
            .transpose()?;
        let aggregate = aggregates.remove(&worker_id).unwrap_or_default();
        let is_stale = match (last_seen.as_ref(), lease_timeout) {
            (Some(last_seen), Some(lease_timeout)) => {
                last_seen
                    .as_millisecond()
                    .saturating_add(i64::try_from(lease_timeout).unwrap_or(i64::MAX))
                    < now
            }
            _ => false,
        };

        snapshots.push(WorkerSnapshot {
            worker_id,
            last_seen,
            is_stale,
            inflight_count: aggregate.inflight_task_ids.len(),
            active_lease_count: aggregate.active_lease_count,
            inflight_task_ids: aggregate.inflight_task_ids,
            next_deadline: aggregate.next_deadline,
            lease_timeout: lease_timeout.map(signed_duration_from_millis),
            heartbeat_interval: heartbeat_interval.map(signed_duration_from_millis),
        });
    }

    Ok(snapshots)
}

async fn read_scope_snapshot(
    connection: &mut Connection,
    namespace: &str,
) -> Result<Snapshot, SpiderError> {
    let keys = Keys::for_namespace(namespace);
    let meta = load_namespace_meta(connection, namespace).await?;
    let reclaimed_in_refresh = reclaim_expired_inflight_for_keys(connection, &keys).await?;

    let ready: usize = scheduler_query(connection, redis_command("SCARD", [&keys.ready])).await?;
    let delayed: usize =
        scheduler_query(connection, redis_command("ZCARD", [&keys.delayed])).await?;
    let mut inflight_ids: Vec<String> =
        scheduler_query(connection, redis_command("SMEMBERS", [&keys.inflight])).await?;
    inflight_ids.sort();
    let deadline_count: usize = scheduler_query(
        connection,
        redis_command("ZCARD", [&keys.inflight_deadlines]),
    )
    .await?;
    let reclaimed_total: Option<u64> =
        scheduler_query(connection, redis_command("GET", [&keys.reclaimed_total])).await?;
    let inflight_tasks = load_inflight_tasks(connection, namespace, &keys, &inflight_ids).await?;
    let workers = load_worker_snapshots(connection, namespace, &keys, &inflight_tasks).await?;

    let mut worker_ids = BTreeSet::new();
    let mut active_lease_count = 0usize;
    for task in &inflight_tasks {
        if let Some(worker_id) = task.worker_id.clone() {
            worker_ids.insert(worker_id);
        }
        if task.lease_id.is_some() {
            active_lease_count += 1;
        }
    }

    Ok(Snapshot {
        scope: namespace.to_string(),
        counts: Counts {
            ready,
            delayed,
            inflight: inflight_tasks.len(),
        },
        worker_ids: worker_ids.into_iter().collect(),
        active_lease_count,
        deadline_count,
        reclaimed_total: reclaimed_total.unwrap_or_default(),
        reclaimed_in_refresh,
        inflight_tasks,
        workers,
        lease_timeout: meta.lease_timeout.map(signed_duration_from_millis),
        heartbeat_interval: meta.heartbeat_interval.map(signed_duration_from_millis),
    })
}

fn timestamp_from_i64(
    namespace: &str,
    task_id: &str,
    field: &str,
    millis: i64,
) -> Result<Timestamp, SpiderError> {
    Timestamp::from_millisecond(millis).map_err(|error| {
        SpiderError::scheduler(format!(
            "redis scheduler snapshot `{field}` for namespace `{namespace}` task `{task_id}` is invalid: {error}"
        ))
    })
}

fn timestamp_from_u64(
    namespace: &str,
    task_id: &str,
    field: &str,
    millis: u64,
) -> Result<Timestamp, SpiderError> {
    let millis = i64::try_from(millis).map_err(|_| {
        SpiderError::scheduler(format!(
            "redis scheduler snapshot `{field}` for namespace `{namespace}` task `{task_id}` exceeds i64 millisecond range"
        ))
    })?;
    timestamp_from_i64(namespace, task_id, field, millis)
}

fn parse_worker_u64(
    namespace: &str,
    worker_id: &str,
    field: &str,
    value: &str,
) -> Result<u64, SpiderError> {
    value.parse::<u64>().map_err(|error| {
        SpiderError::scheduler(format!(
            "redis scheduler worker snapshot `{field}` for namespace `{namespace}` worker `{worker_id}` is invalid: {error}"
        ))
    })
}

fn parse_worker_timestamp(
    namespace: &str,
    worker_id: &str,
    field: &str,
    value: &str,
) -> Result<Timestamp, SpiderError> {
    let millis = value.parse::<i64>().map_err(|error| {
        SpiderError::scheduler(format!(
            "redis scheduler worker snapshot `{field}` for namespace `{namespace}` worker `{worker_id}` is invalid: {error}"
        ))
    })?;
    Timestamp::from_millisecond(millis).map_err(|error| {
        SpiderError::scheduler(format!(
            "redis scheduler worker snapshot `{field}` for namespace `{namespace}` worker `{worker_id}` is invalid: {error}"
        ))
    })
}

fn parse_timestamp_score(
    namespace: &str,
    task_id: &str,
    field: &str,
    value: &str,
) -> Result<i64, SpiderError> {
    if let Ok(millis) = value.parse::<i64>() {
        return Ok(millis);
    }

    let score = value.parse::<f64>().map_err(|error| {
        SpiderError::scheduler(format!(
            "redis scheduler snapshot `{field}` for namespace `{namespace}` task `{task_id}` is invalid: {error}"
        ))
    })?;
    if !score.is_finite() || score.fract() != 0.0 {
        return Err(SpiderError::scheduler(format!(
            "redis scheduler snapshot `{field}` for namespace `{namespace}` task `{task_id}` is not an integer millisecond score: {value}"
        )));
    }
    if score < i64::MIN as f64 || score > i64::MAX as f64 {
        return Err(SpiderError::scheduler(format!(
            "redis scheduler snapshot `{field}` for namespace `{namespace}` task `{task_id}` exceeds i64 millisecond range"
        )));
    }

    Ok(score as i64)
}

fn redis_command<T>(name: &'static str, args: impl IntoIterator<Item = T>) -> redis::Cmd
where
    T: AsRef<str>,
{
    let mut command = redis::cmd(name);
    for arg in args {
        command.arg(arg.as_ref());
    }
    command
}

async fn scheduler_query<T>(
    connection: &mut Connection,
    mut command: redis::Cmd,
) -> Result<T, SpiderError>
where
    T: FromRedisValue,
{
    query(
        connection,
        &mut command,
        "redis scheduler",
        ErrorContext::Scheduler,
    )
    .await
}

async fn scheduler_eval<T>(
    connection: &mut Connection,
    script: &str,
    keys: &[&str],
    args: &[&str],
) -> Result<T, SpiderError>
where
    T: FromRedisValue,
{
    let mut command = redis::cmd("EVAL");
    command.arg(script).arg(keys.len());
    for key in keys {
        command.arg(key);
    }
    for arg in args {
        command.arg(arg);
    }
    scheduler_query(connection, command).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{SchedulerError, SpiderError};
    use crate::request::Request;
    use crate::test_support::redis::spawn_redis_server;
    use jiff::SignedDuration;

    #[tokio::test]
    async fn redis_scheduler_supports_async_enqueue_and_take_ready() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let scheduler = Redis::new(format!("redis://{url}"), "news");
        let task = Task::new(Request::new("https://example.com"));

        scheduler.enqueue(task.clone()).await.unwrap();
        let taken = scheduler.take_ready().await.unwrap();

        assert_eq!(
            taken.as_ref().map(|task| task.task.id.as_str()),
            Some(task.id.as_str())
        );
        assert_eq!(
            taken.map(|task| task.task.request.url),
            Some(task.request.url)
        );

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_prefers_higher_priority_then_lower_depth() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let scheduler = Redis::new(format!("redis://{url}"), "ordering");
        scheduler
            .enqueue(
                Task::new(Request::new("https://example.com/depth-2"))
                    .with_priority(0)
                    .with_depth(2),
            )
            .await
            .unwrap();
        scheduler
            .enqueue(
                Task::new(Request::new("https://example.com/high-priority"))
                    .with_priority(10)
                    .with_depth(8),
            )
            .await
            .unwrap();
        scheduler
            .enqueue(
                Task::new(Request::new("https://example.com/depth-0"))
                    .with_priority(0)
                    .with_depth(0),
            )
            .await
            .unwrap();

        let first = scheduler.take_ready().await.unwrap().unwrap();
        let second = scheduler.take_ready().await.unwrap().unwrap();
        let third = scheduler.take_ready().await.unwrap().unwrap();

        assert_eq!(first.task.request.url, "https://example.com/high-priority");
        assert_eq!(second.task.request.url, "https://example.com/depth-0");
        assert_eq!(third.task.request.url, "https://example.com/depth-2");

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_skips_delayed_task_until_ready() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let scheduler = Redis::new(format!("redis://{url}"), "delayed");
        scheduler
            .enqueue(Task::with_delay(
                Request::new("https://example.com/delayed"),
                60,
            ))
            .await
            .unwrap();

        assert!(scheduler.take_ready().await.unwrap().is_none());
        tokio::time::sleep(std::time::Duration::try_from(SignedDuration::from_millis(80)).unwrap())
            .await;

        let taken = scheduler.take_ready().await.unwrap().unwrap();
        assert_eq!(taken.task.request.url, "https://example.com/delayed");

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_restores_tasks_from_existing_namespace() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "restore";
        let first = Redis::new(format!("redis://{url}"), namespace);
        first
            .enqueue(Task::new(Request::new("https://example.com/restored")))
            .await
            .unwrap();
        first.close().await.unwrap();

        let second = Redis::new(format!("redis://{url}"), namespace);
        let taken = second.take_ready().await.unwrap().unwrap();

        assert_eq!(taken.task.request.url, "https://example.com/restored");

        second.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_reclaims_stale_inflight_after_lease_timeout() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "lease_reclaim";
        let first = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-a").with_lease_timeout(SignedDuration::from_millis(20)),
        );
        first
            .enqueue(Task::new(Request::new("https://example.com/reclaim")))
            .await
            .unwrap();

        let taken = first.take_ready().await.unwrap().unwrap();
        first.close().await.unwrap();

        tokio::time::sleep(std::time::Duration::try_from(SignedDuration::from_millis(40)).unwrap())
            .await;

        let second = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-b").with_lease_timeout(SignedDuration::from_millis(20)),
        );
        let checkpoint = second.checkpoint().await.unwrap();

        assert_eq!(checkpoint.ready.len(), 1);
        assert!(checkpoint.inflight.is_empty());
        assert_eq!(checkpoint.ready[0].id, taken.task.id);

        let reclaimed = second.take_ready().await.unwrap().unwrap();
        assert_eq!(reclaimed.task.id, taken.task.id);
        assert_eq!(reclaimed.task.request.url, "https://example.com/reclaim");

        second.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_claims_one_ready_task_across_concurrent_workers() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "atomic_claim";
        let producer = Redis::new(format!("redis://{url}"), namespace);
        let task = Task::new(Request::new("https://example.com/claim-once"));
        producer.enqueue(task.clone()).await.unwrap();

        let first_worker = Redis::new(format!("redis://{url}"), namespace);
        let second_worker = Redis::new(format!("redis://{url}"), namespace);

        let (first, second) = tokio::join!(first_worker.take_ready(), second_worker.take_ready());
        let first = first.unwrap();
        let second = second.unwrap();

        let taken = [first.as_ref(), second.as_ref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].task.id, task.id);

        producer.close().await.unwrap();
        first_worker.close().await.unwrap();
        second_worker.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_heartbeat_keeps_lease_active_past_initial_timeout() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "heartbeat";
        let first = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-a")
                .with_lease_timeout(SignedDuration::from_millis(20))
                .with_heartbeat_interval(SignedDuration::from_millis(10)),
        );
        first
            .enqueue(Task::new(Request::new("https://example.com/heartbeat")))
            .await
            .unwrap();

        let claimed = first.take_ready().await.unwrap().unwrap();
        tokio::time::sleep(std::time::Duration::try_from(SignedDuration::from_millis(15)).unwrap())
            .await;
        first.heartbeat(&claimed.lease).await.unwrap();

        tokio::time::sleep(std::time::Duration::try_from(SignedDuration::from_millis(10)).unwrap())
            .await;

        let second = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-b").with_lease_timeout(SignedDuration::from_millis(20)),
        );
        let checkpoint = second.checkpoint().await.unwrap();

        assert!(checkpoint.ready.is_empty());
        assert_eq!(checkpoint.inflight.len(), 1);
        assert_eq!(checkpoint.inflight[0].id, claimed.task.id);

        first.close().await.unwrap();
        second.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_snapshot_reports_current_namespace_state() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let scheduler = Redis::new(format!("redis://{url}"), "snapshot").with_worker(
            Worker::new("worker-a").with_lease_timeout(SignedDuration::from_millis(50)),
        );
        scheduler
            .enqueue(
                Task::new(Request::new("https://example.com/ready"))
                    .with_priority(7)
                    .with_depth(2),
            )
            .await
            .unwrap();
        scheduler
            .enqueue(Task::with_delay(
                Request::new("https://example.com/delayed"),
                500,
            ))
            .await
            .unwrap();

        let claimed = scheduler.take_ready().await.unwrap().unwrap();
        let snapshot = scheduler.snapshot().await.unwrap();

        assert_eq!(snapshot.scope, "snapshot");
        assert_eq!(
            snapshot.counts,
            Counts {
                ready: 0,
                delayed: 1,
                inflight: 1,
            }
        );
        assert_eq!(snapshot.worker_ids, vec!["worker-a".to_string()]);
        assert_eq!(snapshot.active_lease_count, 1);
        assert_eq!(snapshot.deadline_count, 1);
        assert_eq!(snapshot.reclaimed_total, 0);
        assert_eq!(snapshot.reclaimed_in_refresh, 0);
        assert_eq!(snapshot.inflight_tasks.len(), 1);
        assert_eq!(
            snapshot.lease_timeout,
            Some(SignedDuration::from_millis(50))
        );
        assert_eq!(
            snapshot.heartbeat_interval,
            Some(SignedDuration::from_millis(25))
        );
        assert_eq!(snapshot.workers.len(), 1);
        let inflight = &snapshot.inflight_tasks[0];
        assert_eq!(inflight.task_id, claimed.task.id);
        assert_eq!(inflight.url, "https://example.com/ready");
        assert_eq!(inflight.worker_id.as_deref(), Some("worker-a"));
        assert_eq!(inflight.lease_id.as_deref(), Some(claimed.lease.lease_id()));
        assert!(inflight.deadline.is_some());
        assert_eq!(inflight.priority, 7);
        assert_eq!(inflight.depth, 2);
        assert_eq!(inflight.ready_at, None);
        let worker = snapshot
            .workers
            .iter()
            .find(|worker| worker.worker_id == "worker-a")
            .unwrap();
        assert!(worker.last_seen.is_some());
        assert!(!worker.is_stale);
        assert_eq!(worker.inflight_count, 1);
        assert_eq!(worker.active_lease_count, 1);
        assert_eq!(worker.inflight_task_ids, vec![claimed.task.id.clone()]);
        assert!(worker.next_deadline.is_some());
        assert_eq!(worker.lease_timeout, Some(SignedDuration::from_millis(50)));
        assert_eq!(
            worker.heartbeat_interval,
            Some(SignedDuration::from_millis(25))
        );

        scheduler.complete(&claimed.lease).await.unwrap();
        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_snapshot_tracks_reclaimed_totals() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "snapshot_reclaim";
        let first = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-a").with_lease_timeout(SignedDuration::from_millis(20)),
        );
        first
            .enqueue(Task::new(Request::new("https://example.com/reclaim-total")))
            .await
            .unwrap();

        let claimed = first.take_ready().await.unwrap().unwrap();
        first.close().await.unwrap();

        tokio::time::sleep(std::time::Duration::try_from(SignedDuration::from_millis(40)).unwrap())
            .await;

        let second = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-b").with_lease_timeout(SignedDuration::from_millis(20)),
        );
        let first_snapshot = second.snapshot().await.unwrap();

        assert_eq!(
            first_snapshot.counts,
            Counts {
                ready: 1,
                delayed: 0,
                inflight: 0,
            }
        );
        assert_eq!(first_snapshot.reclaimed_in_refresh, 1);
        assert_eq!(first_snapshot.reclaimed_total, 1);
        assert!(first_snapshot.worker_ids.is_empty());
        assert_eq!(first_snapshot.active_lease_count, 0);
        assert_eq!(first_snapshot.deadline_count, 0);
        assert!(first_snapshot.inflight_tasks.is_empty());
        assert_eq!(first_snapshot.workers.len(), 1);
        let stale_worker = &first_snapshot.workers[0];
        assert_eq!(stale_worker.worker_id, "worker-a");
        assert!(stale_worker.last_seen.is_some());
        assert!(stale_worker.is_stale);
        assert_eq!(stale_worker.inflight_count, 0);
        assert_eq!(stale_worker.active_lease_count, 0);
        assert!(stale_worker.inflight_task_ids.is_empty());
        assert_eq!(
            stale_worker.lease_timeout,
            Some(SignedDuration::from_millis(20))
        );
        assert_eq!(
            stale_worker.heartbeat_interval,
            Some(SignedDuration::from_millis(10))
        );

        let second_snapshot = second.snapshot().await.unwrap();
        assert_eq!(second_snapshot.reclaimed_in_refresh, 0);
        assert_eq!(second_snapshot.reclaimed_total, 1);
        assert!(second_snapshot.inflight_tasks.is_empty());
        assert_eq!(second_snapshot.workers.len(), 1);
        assert!(second_snapshot.workers[0].is_stale);

        let reclaimed = second.take_ready().await.unwrap().unwrap();
        assert_eq!(reclaimed.task.id, claimed.task.id);
        second.complete(&reclaimed.lease).await.unwrap();

        second.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_close_cleans_idle_worker_runtime() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "close_idle_worker";
        let worker =
            Redis::new(format!("redis://{url}"), namespace).with_worker(Worker::new("worker-a"));
        worker
            .enqueue(Task::with_delay(
                Request::new("https://example.com/close-idle"),
                500,
            ))
            .await
            .unwrap();
        worker.close().await.unwrap();

        let observer =
            Redis::new(format!("redis://{url}"), namespace).with_worker(Worker::new("observer"));
        let snapshot = observer.snapshot().await.unwrap();

        assert_eq!(snapshot.scope, namespace);
        assert_eq!(
            snapshot.counts,
            Counts {
                ready: 0,
                delayed: 1,
                inflight: 0,
            }
        );
        assert!(snapshot.worker_ids.is_empty());
        assert!(snapshot.workers.is_empty());

        observer.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_release_inflight_requeues_current_worker_tasks() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "release_inflight";
        let worker_a =
            Redis::new(format!("redis://{url}"), namespace).with_worker(Worker::new("worker-a"));
        worker_a
            .enqueue(Task::new(Request::new(
                "https://example.com/release-inflight",
            )))
            .await
            .unwrap();
        let claimed = worker_a.take_ready().await.unwrap().unwrap();

        assert_eq!(worker_a.release_inflight().await.unwrap(), 1);

        let observer =
            Redis::new(format!("redis://{url}"), namespace).with_worker(Worker::new("observer"));
        let snapshot = observer.snapshot().await.unwrap();
        assert_eq!(
            snapshot.counts,
            Counts {
                ready: 1,
                delayed: 0,
                inflight: 0,
            }
        );
        assert!(snapshot.worker_ids.is_empty());
        assert!(snapshot.workers.is_empty());

        let worker_b =
            Redis::new(format!("redis://{url}"), namespace).with_worker(Worker::new("worker-b"));
        let reclaimed = worker_b.take_ready().await.unwrap().unwrap();
        assert_eq!(reclaimed.task.id, claimed.task.id);

        worker_b.complete(&reclaimed.lease).await.unwrap();
        worker_a.close().await.unwrap();
        worker_b.close().await.unwrap();
        observer.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_close_keeps_worker_runtime_when_inflight_is_still_owned() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "close_inflight_worker";
        let worker =
            Redis::new(format!("redis://{url}"), namespace).with_worker(Worker::new("worker-a"));
        worker
            .enqueue(Task::new(Request::new(
                "https://example.com/close-inflight",
            )))
            .await
            .unwrap();
        let claimed = worker.take_ready().await.unwrap().unwrap();
        worker.close().await.unwrap();

        let observer =
            Redis::new(format!("redis://{url}"), namespace).with_worker(Worker::new("observer"));
        let snapshot = observer.snapshot().await.unwrap();

        assert_eq!(snapshot.scope, namespace);
        assert_eq!(snapshot.worker_ids, vec!["worker-a".to_string()]);
        assert_eq!(snapshot.workers.len(), 1);
        let worker_snapshot = &snapshot.workers[0];
        assert_eq!(worker_snapshot.worker_id, "worker-a");
        assert_eq!(
            worker_snapshot.inflight_task_ids,
            vec![claimed.task.id.clone()]
        );
        assert!(worker_snapshot.last_seen.is_some());

        observer.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_lists_registered_namespaces_by_prefix() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let redis_url = format!("redis://{url}");

        let news = Redis::new(redis_url.clone(), "jobs:news");
        let blog = Redis::new(redis_url.clone(), "jobs:blog");
        let scratch = Redis::new(redis_url.clone(), "scratch:demo");

        news.counts().await.unwrap();
        blog.counts().await.unwrap();
        scratch.counts().await.unwrap();

        let scopes = news.scopes_with_prefix("jobs:").await.unwrap();
        assert_eq!(
            scopes,
            vec!["jobs:blog".to_string(), "jobs:news".to_string()]
        );

        news.close().await.unwrap();
        blog.close().await.unwrap();
        scratch.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_reads_namespace_snapshots_across_jobs() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let redis_url = format!("redis://{url}");

        let news = Redis::new(redis_url.clone(), "jobs:news").with_worker(
            Worker::new("news-worker").with_lease_timeout(SignedDuration::from_millis(50)),
        );
        news.enqueue(Task::new(Request::new("https://example.com/news")))
            .await
            .unwrap();
        let claimed = news.take_ready().await.unwrap().unwrap();

        let blog = Redis::new(redis_url.clone(), "jobs:blog").with_worker(
            Worker::new("blog-worker")
                .with_lease_timeout(SignedDuration::from_millis(80))
                .with_heartbeat_interval(SignedDuration::from_millis(20)),
        );
        blog.enqueue(Task::with_delay(
            Request::new("https://example.com/blog"),
            500,
        ))
        .await
        .unwrap();

        let snapshots = news.snapshots_with_prefix("jobs:").await.unwrap();
        assert_eq!(snapshots.len(), 2);

        let news_snapshot = snapshots
            .iter()
            .find(|snapshot| snapshot.scope == "jobs:news")
            .unwrap();
        assert_eq!(
            news_snapshot.counts,
            Counts {
                ready: 0,
                delayed: 0,
                inflight: 1,
            }
        );
        assert_eq!(news_snapshot.worker_ids, vec!["news-worker".to_string()]);
        assert_eq!(
            news_snapshot.lease_timeout,
            Some(SignedDuration::from_millis(50))
        );
        assert_eq!(
            news_snapshot.heartbeat_interval,
            Some(SignedDuration::from_millis(25))
        );
        assert_eq!(news_snapshot.inflight_tasks.len(), 1);
        assert_eq!(news_snapshot.workers.len(), 1);
        let news_task = &news_snapshot.inflight_tasks[0];
        assert_eq!(news_task.task_id, claimed.task.id);
        assert_eq!(news_task.url, "https://example.com/news");
        assert_eq!(news_task.worker_id.as_deref(), Some("news-worker"));
        assert_eq!(
            news_task.lease_id.as_deref(),
            Some(claimed.lease.lease_id())
        );
        assert!(news_task.deadline.is_some());
        let news_worker = &news_snapshot.workers[0];
        assert_eq!(news_worker.worker_id, "news-worker");
        assert!(!news_worker.is_stale);
        assert_eq!(news_worker.inflight_count, 1);
        assert_eq!(news_worker.inflight_task_ids, vec![claimed.task.id.clone()]);

        let blog_snapshot = snapshots
            .iter()
            .find(|snapshot| snapshot.scope == "jobs:blog")
            .unwrap();
        assert_eq!(
            blog_snapshot.counts,
            Counts {
                ready: 0,
                delayed: 1,
                inflight: 0,
            }
        );
        assert!(blog_snapshot.worker_ids.is_empty());
        assert_eq!(
            blog_snapshot.lease_timeout,
            Some(SignedDuration::from_millis(80))
        );
        assert_eq!(
            blog_snapshot.heartbeat_interval,
            Some(SignedDuration::from_millis(20))
        );
        assert!(blog_snapshot.inflight_tasks.is_empty());
        assert!(blog_snapshot.workers.is_empty());

        let overview = news.overview().await.unwrap();
        assert_eq!(overview.scope_count, 2);
        assert_eq!(overview.pending_scope_count, 2);
        assert_eq!(overview.stale_scope_count, 0);
        assert_eq!(
            overview.counts,
            Counts {
                ready: 0,
                delayed: 1,
                inflight: 1,
            }
        );
        assert_eq!(overview.worker_count, 1);
        assert_eq!(overview.stale_worker_count, 0);
        assert_eq!(overview.active_lease_count, 1);
        assert_eq!(overview.reclaimed_total, 0);

        let blog_overview = news.overview_with_prefix("jobs:blog").await.unwrap();
        assert_eq!(blog_overview.scope_count, 1);
        assert_eq!(blog_overview.pending_scope_count, 1);
        assert_eq!(blog_overview.stale_scope_count, 0);
        assert_eq!(
            blog_overview.counts,
            Counts {
                ready: 0,
                delayed: 1,
                inflight: 0,
            }
        );
        assert_eq!(blog_overview.worker_count, 0);
        assert_eq!(blog_overview.stale_worker_count, 0);
        assert_eq!(blog_overview.active_lease_count, 0);
        assert_eq!(blog_overview.reclaimed_total, 0);

        let no_overview = news.overview_with_prefix("other:").await.unwrap();
        assert_eq!(no_overview.scope_count, 0);
        assert_eq!(no_overview.pending_scope_count, 0);
        assert_eq!(no_overview.stale_scope_count, 0);
        assert_eq!(no_overview.counts, Counts::default());
        assert_eq!(no_overview.worker_count, 0);
        assert_eq!(no_overview.stale_worker_count, 0);
        assert_eq!(no_overview.active_lease_count, 0);
        assert_eq!(no_overview.reclaimed_total, 0);

        news.complete(&claimed.lease).await.unwrap();
        news.close().await.unwrap();
        blog.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_rejects_stale_or_foreign_lease_resolution() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "ownership";
        let first = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-a").with_lease_timeout(SignedDuration::from_millis(20)),
        );
        first
            .enqueue(Task::new(Request::new("https://example.com/ownership")))
            .await
            .unwrap();

        let claimed = first.take_ready().await.unwrap().unwrap();

        let second = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-b").with_lease_timeout(SignedDuration::from_millis(20)),
        );
        let error = second.complete(&claimed.lease).await.unwrap_err();
        assert_eq!(
            error,
            SpiderError::scheduler(SchedulerError::LeaseWorkerMismatch {
                lease_worker_id: "worker-a".to_string(),
                current_worker_id: "worker-b".to_string(),
            })
        );

        tokio::time::sleep(std::time::Duration::try_from(SignedDuration::from_millis(40)).unwrap())
            .await;

        let reclaimed = second.take_ready().await.unwrap().unwrap();
        let stale_error = first.complete(&claimed.lease).await.unwrap_err();
        assert_eq!(
            stale_error,
            SpiderError::scheduler(SchedulerError::LeaseOwnershipConflict {
                action: "complete",
                task_id: claimed.task.id.as_str().to_string(),
                worker_id: "worker-a".to_string(),
            })
        );
        second.complete(&reclaimed.lease).await.unwrap();

        first.close().await.unwrap();
        second.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_reports_stale_complete_for_same_worker_old_lease() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "stale_complete";
        let scheduler = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-a").with_lease_timeout(SignedDuration::from_millis(20)),
        );
        scheduler
            .enqueue(Task::new(Request::new(
                "https://example.com/stale-complete",
            )))
            .await
            .unwrap();

        let first_claim = scheduler.take_ready().await.unwrap().unwrap();
        tokio::time::sleep(std::time::Duration::try_from(SignedDuration::from_millis(40)).unwrap())
            .await;
        let second_claim = scheduler.take_ready().await.unwrap().unwrap();

        let error = scheduler.complete(&first_claim.lease).await.unwrap_err();
        assert_eq!(
            error,
            SpiderError::scheduler(SchedulerError::StaleLease {
                action: "complete",
                task_id: first_claim.task.id.as_str().to_string(),
                worker_id: "worker-a".to_string(),
                lease_id: first_claim.lease.lease_id().to_string(),
            })
        );

        scheduler.complete(&second_claim.lease).await.unwrap();
        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_reports_inactive_complete_after_task_is_already_resolved() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let scheduler = Redis::new(format!("redis://{url}"), "inactive_complete");
        scheduler
            .enqueue(Task::new(Request::new(
                "https://example.com/inactive-complete",
            )))
            .await
            .unwrap();

        let claimed = scheduler.take_ready().await.unwrap().unwrap();
        scheduler.complete(&claimed.lease).await.unwrap();

        let error = scheduler.complete(&claimed.lease).await.unwrap_err();
        assert_eq!(
            error,
            SpiderError::scheduler(SchedulerError::InactiveLease {
                action: "complete",
                task_id: claimed.task.id.as_str().to_string(),
            })
        );

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_reports_stale_heartbeat_for_same_worker_old_lease() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "stale_heartbeat";
        let scheduler = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-a")
                .with_lease_timeout(SignedDuration::from_millis(20))
                .with_heartbeat_interval(SignedDuration::from_millis(10)),
        );
        scheduler
            .enqueue(Task::new(Request::new(
                "https://example.com/stale-heartbeat",
            )))
            .await
            .unwrap();

        let first_claim = scheduler.take_ready().await.unwrap().unwrap();
        tokio::time::sleep(std::time::Duration::try_from(SignedDuration::from_millis(40)).unwrap())
            .await;
        let second_claim = scheduler.take_ready().await.unwrap().unwrap();

        let error = scheduler.heartbeat(&first_claim.lease).await.unwrap_err();
        assert_eq!(
            error,
            SpiderError::scheduler(SchedulerError::StaleLease {
                action: "heartbeat",
                task_id: first_claim.task.id.as_str().to_string(),
                worker_id: "worker-a".to_string(),
                lease_id: first_claim.lease.lease_id().to_string(),
            })
        );

        scheduler.complete(&second_claim.lease).await.unwrap();
        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_enqueue_only_does_not_register_worker_runtime() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let scheduler = Redis::new(format!("redis://{url}"), "enqueue_only").with_worker(
            Worker::new("worker-a")
                .with_lease_timeout(SignedDuration::from_millis(80))
                .with_heartbeat_interval(SignedDuration::from_millis(20)),
        );
        scheduler
            .enqueue(Task::with_delay(
                Request::new("https://example.com/enqueue-only"),
                500,
            ))
            .await
            .unwrap();

        let snapshot = scheduler.snapshot().await.unwrap();
        assert_eq!(
            snapshot.counts,
            Counts {
                ready: 0,
                delayed: 1,
                inflight: 0,
            }
        );
        assert!(snapshot.worker_ids.is_empty());
        assert!(snapshot.workers.is_empty());
        assert_eq!(
            snapshot.lease_timeout,
            Some(SignedDuration::from_millis(80))
        );
        assert_eq!(
            snapshot.heartbeat_interval,
            Some(SignedDuration::from_millis(20))
        );

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_empty_take_ready_does_not_register_worker_runtime() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let scheduler = Redis::new(format!("redis://{url}"), "empty_take_ready")
            .with_worker(Worker::new("worker-a"));

        assert!(scheduler.take_ready().await.unwrap().is_none());

        let snapshot = scheduler.snapshot().await.unwrap();
        assert_eq!(snapshot.counts, Counts::default());
        assert!(snapshot.worker_ids.is_empty());
        assert!(snapshot.workers.is_empty());

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_empty_take_ready_refreshes_registered_worker_runtime() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let scheduler = Redis::new(format!("redis://{url}"), "idle_refresh").with_worker(
            Worker::new("worker-a").with_lease_timeout(SignedDuration::from_millis(20)),
        );
        scheduler
            .enqueue(Task::new(Request::new("https://example.com/idle-refresh")))
            .await
            .unwrap();

        let claimed = scheduler.take_ready().await.unwrap().unwrap();
        scheduler.complete(&claimed.lease).await.unwrap();

        let first_snapshot = scheduler.snapshot().await.unwrap();
        assert_eq!(first_snapshot.workers.len(), 1);
        let first_last_seen = first_snapshot.workers[0]
            .last_seen
            .expect("worker should have last_seen after complete");

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert!(scheduler.take_ready().await.unwrap().is_none());

        let second_snapshot = scheduler.snapshot().await.unwrap();
        assert_eq!(second_snapshot.workers.len(), 1);
        let worker = &second_snapshot.workers[0];
        let second_last_seen = worker.last_seen;
        assert!(second_last_seen.is_some());
        assert!(second_last_seen >= Some(first_last_seen));
        assert!(!worker.is_stale);
        assert_eq!(worker.inflight_count, 0);
        assert_eq!(worker.active_lease_count, 0);

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_stale_heartbeat_does_not_refresh_worker_last_seen() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "stale_heartbeat_last_seen";
        let scheduler = Redis::new(format!("redis://{url}"), namespace).with_worker(
            Worker::new("worker-a")
                .with_lease_timeout(SignedDuration::from_millis(20))
                .with_heartbeat_interval(SignedDuration::from_millis(10)),
        );
        scheduler
            .enqueue(Task::new(Request::new(
                "https://example.com/stale-heartbeat-last-seen",
            )))
            .await
            .unwrap();

        let first_claim = scheduler.take_ready().await.unwrap().unwrap();
        tokio::time::sleep(std::time::Duration::try_from(SignedDuration::from_millis(40)).unwrap())
            .await;
        let second_claim = scheduler.take_ready().await.unwrap().unwrap();

        let before_snapshot = scheduler.snapshot().await.unwrap();
        let before_last_seen = before_snapshot
            .workers
            .iter()
            .find(|worker| worker.worker_id == "worker-a")
            .and_then(|worker| worker.last_seen)
            .expect("worker-a should have last_seen after reclaim");

        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let error = scheduler.heartbeat(&first_claim.lease).await.unwrap_err();
        assert_eq!(
            error,
            SpiderError::scheduler(SchedulerError::StaleLease {
                action: "heartbeat",
                task_id: first_claim.task.id.as_str().to_string(),
                worker_id: "worker-a".to_string(),
                lease_id: first_claim.lease.lease_id().to_string(),
            })
        );

        let after_snapshot = scheduler.snapshot().await.unwrap();
        let after_last_seen = after_snapshot
            .workers
            .iter()
            .find(|worker| worker.worker_id == "worker-a")
            .and_then(|worker| worker.last_seen)
            .expect("worker-a should still have last_seen");

        assert_eq!(after_last_seen, before_last_seen);

        scheduler.complete(&second_claim.lease).await.unwrap();
        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_rejects_empty_namespace() {
        let scheduler = Redis::new("redis://127.0.0.1:6379", "   ");

        let error = scheduler
            .enqueue(Task::new(Request::new("https://example.com")))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            SpiderError::scheduler("redis scheduler namespace cannot be empty")
        );
    }

    #[tokio::test]
    async fn redis_scheduler_rejects_empty_worker_id() {
        let scheduler = Redis::new("redis://127.0.0.1:6379", "news").with_worker(Worker::new("  "));

        let error = scheduler
            .enqueue(Task::new(Request::new("https://example.com")))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            SpiderError::scheduler("redis scheduler worker_id cannot be empty")
        );
    }
}
