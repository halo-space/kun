use halo_spider::error::SpiderError;
use halo_spider::scheduler::checkpoint::{Checkpoint, Counts};
use halo_spider::scheduler::{ClaimedTask, Control, Scheduler, Snapshot, Task, TaskLease, Worker};

/// MySQL scheduler scaffold example.
///
/// This file is intentionally a skeleton, not a ready-to-run backend.
/// The real implementation steps and SQL shape are documented in:
/// `docs/custom_scheduler_backend.md`
#[allow(dead_code)]
struct MysqlScheduler {
    url: String,
    scope: String,
    worker: Worker,
}

#[allow(dead_code)]
impl MysqlScheduler {
    fn new(url: impl Into<String>, scope: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            scope: scope.into(),
            worker: Worker::new("mysql-worker"),
        }
    }

    fn with_worker(mut self, worker: Worker) -> Self {
        self.worker = worker;
        self
    }

    fn scope(&self) -> &str {
        self.scope.as_str()
    }

    fn worker_id(&self) -> &str {
        self.worker.worker_id()
    }

    async fn open_pool(&self) -> Result<(), SpiderError> {
        let _ = (&self.url, self.scope(), self.worker_id());
        todo!("接你的 MySQL client 或 sqlx::MySqlPool")
    }

    async fn ensure_scope_row(&self) -> Result<(), SpiderError> {
        todo!("确保 scheduler_scopes 里存在当前 scope")
    }

    async fn reserve_sequence(&self, _scope: &str) -> Result<u64, SpiderError> {
        todo!("为当前 scope 分配一个稳定 sequence")
    }

    async fn reclaim_expired_tasks(&self, _scope: &str) -> Result<usize, SpiderError> {
        todo!("把过期 inflight 回收到 ready / delayed")
    }

    async fn promote_delayed_tasks(&self, _scope: &str) -> Result<(), SpiderError> {
        todo!("把 ready_at <= now 的 delayed 提升为 ready")
    }
}

impl Scheduler for MysqlScheduler {
    async fn enqueue(&self, _task: Task) -> Result<(), SpiderError> {
        todo!("一个事务里完成 ensure_scope + reserve_sequence + insert task")
    }

    async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        todo!("导出当前 scope 的 ready / delayed / inflight")
    }

    async fn counts(&self) -> Result<Counts, SpiderError> {
        todo!("统计当前 scope 的 ready / delayed / inflight")
    }

    async fn snapshot(&self) -> Result<Snapshot, SpiderError> {
        todo!("读取统一 Snapshot 结构")
    }

    async fn scopes(&self) -> Result<Vec<String>, SpiderError> {
        todo!("共享 MySQL 后端应返回当前库里可见的多个 scope")
    }

    async fn snapshots(&self) -> Result<Vec<Snapshot>, SpiderError> {
        todo!("读取多个 scope 的统一 Snapshot")
    }

    async fn take_ready(&self) -> Result<Option<ClaimedTask>, SpiderError> {
        todo!("claim ready -> inflight，写入 worker_id / lease_id / deadline")
    }

    async fn complete(&self, _lease: &TaskLease) -> Result<(), SpiderError> {
        todo!("校验 lease 后删除 inflight task")
    }

    async fn complete_and_enqueue(
        &self,
        _lease: &TaskLease,
        _tasks: Vec<Task>,
    ) -> Result<(), SpiderError> {
        todo!("同一个 MySQL 事务里完成 follow tasks enqueue + current lease complete")
    }

    async fn requeue(&self, _lease: &TaskLease) -> Result<(), SpiderError> {
        todo!("校验 lease 后放回 ready / delayed")
    }

    async fn release_inflight(&self) -> Result<usize, SpiderError> {
        todo!("把当前 worker 持有的 inflight 主动交回 scheduler")
    }

    async fn heartbeat(&self, _lease: &TaskLease) -> Result<(), SpiderError> {
        todo!("刷新 deadline 和 worker last_seen")
    }

    async fn close(&self) -> Result<(), SpiderError> {
        todo!("必要时清理 worker runtime / 连接资源")
    }

    fn runtime_scope(&self) -> Option<String> {
        Some(self.scope.clone())
    }

    fn runtime_worker_id(&self) -> Option<String> {
        Some(self.worker_id().to_string())
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        Ok(self.counts().await?.has_pending())
    }
}

impl Control for MysqlScheduler {
    async fn pause_scope(&self, _scope: &str) -> Result<bool, SpiderError> {
        todo!("设置 scope paused")
    }

    async fn resume_scope(&self, _scope: &str) -> Result<bool, SpiderError> {
        todo!("恢复 scope claim")
    }

    async fn release_scope(&self, _scope: &str) -> Result<usize, SpiderError> {
        todo!("回收整个 scope 的 inflight")
    }

    async fn purge_scope(&self, _scope: &str) -> Result<Counts, SpiderError> {
        todo!("清空整个 scope")
    }
}

fn main() {
    println!("See docs/custom_scheduler_backend.md for the full MySQL backend guide.");
}
