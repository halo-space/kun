use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::request::Request;
use halo_spider::scheduler::checkpoint::{Checkpoint, Persist};
use halo_spider::scheduler::{self, Control, Scheduler, Task, TaskLease, TaskResolution};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

static NEXT_SQLITE_DEMO_ID: AtomicU64 = AtomicU64::new(1);

/// Custom scheduler example.
///
/// This reuses the built-in memory scheduler for actual ready/delayed/inflight
/// semantics, and adds custom bookkeeping around completed task ids.
#[derive(Default)]
struct RecordingScheduler {
    inner: scheduler::Memory,
    completed_task_ids: StdMutex<Vec<String>>,
}

impl RecordingScheduler {
    fn completed_task_ids(&self) -> Vec<String> {
        self.completed_task_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Scheduler for RecordingScheduler {
    async fn enqueue(&self, task: Task) -> Result<(), SpiderError> {
        println!("custom scheduler enqueue: {}", task.request.url);
        self.inner.enqueue(task).await
    }

    async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        Ok(self.inner.checkpoint())
    }

    async fn counts(&self) -> Result<scheduler::checkpoint::Counts, SpiderError> {
        Ok(self.inner.counts())
    }

    async fn snapshot(&self) -> Result<scheduler::Snapshot, SpiderError> {
        self.inner.snapshot().await
    }

    async fn take_ready(&self) -> Result<Option<scheduler::ClaimedTask>, SpiderError> {
        let task = self.inner.take_ready().await?;
        if let Some(task) = &task {
            println!("custom scheduler take_ready: {}", task.task.request.url);
        }
        Ok(task)
    }

    async fn complete(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        self.completed_task_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(lease.task_id().as_str().to_string());
        self.inner.complete(lease).await
    }

    async fn requeue(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        self.inner.requeue(lease).await
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        self.inner.has_pending().await
    }
}

impl Control for RecordingScheduler {
    async fn pause_scope(&self, scope: &str) -> Result<bool, SpiderError> {
        self.inner.pause_scope(scope).await
    }

    async fn resume_scope(&self, scope: &str) -> Result<bool, SpiderError> {
        self.inner.resume_scope(scope).await
    }

    async fn release_scope(&self, scope: &str) -> Result<usize, SpiderError> {
        self.inner.release_scope(scope).await
    }

    async fn purge_scope(&self, scope: &str) -> Result<scheduler::checkpoint::Counts, SpiderError> {
        self.inner.purge_scope(scope).await
    }
}

/// Custom checkpoint persistence example.
///
/// Real projects can replace this with Redis, S3, a database, or any other
/// backend, as long as it implements `scheduler::checkpoint::Persist`.
#[derive(Clone, Default)]
struct InMemoryCheckpoint {
    inner: Arc<Mutex<Checkpoint>>,
}

impl Persist for InMemoryCheckpoint {
    async fn load(&self) -> Result<Checkpoint, SpiderError> {
        Ok(self.inner.lock().await.clone())
    }

    async fn save(&self, checkpoint: &Checkpoint) -> Result<(), SpiderError> {
        *self.inner.lock().await = checkpoint.clone();
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), SpiderError> {
    custom_scheduler_demo().await?;
    custom_checkpoint_demo().await?;
    memory_snapshot_demo().await?;
    memory_batch_demo().await?;
    memory_control_demo().await?;
    memory_scope_overview_demo().await?;
    sqlite_snapshot_demo().await?;
    sqlite_batch_demo().await?;
    sqlite_control_demo().await?;
    sqlite_scope_overview_demo().await?;
    redis_snapshot_demo().await?;
    redis_batch_demo().await?;
    redis_control_demo().await?;
    redis_scope_overview_demo().await?;
    Ok(())
}

async fn custom_scheduler_demo() -> Result<(), SpiderError> {
    println!("== custom scheduler demo ==");

    let scheduler = RecordingScheduler::default();
    let task = Task::new(Request::new("https://example.com/custom-scheduler"));

    scheduler.enqueue(task).await?;
    let taken = scheduler
        .take_ready()
        .await?
        .expect("custom scheduler should yield one ready task");
    scheduler.complete(&taken.lease).await?;

    println!(
        "completed ids recorded by custom scheduler: {:?}",
        scheduler.completed_task_ids()
    );

    let _engine = Engine::new().with_scheduler(scheduler);
    println!("Engine::with_scheduler(...) can replace the default scheduler.");

    Ok(())
}

async fn custom_checkpoint_demo() -> Result<(), SpiderError> {
    println!("== custom checkpoint demo ==");

    let persist = InMemoryCheckpoint::default();
    persist
        .save(&Checkpoint {
            ready: vec![Task::new(Request::new(
                "https://example.com/restored-from-custom-checkpoint",
            ))],
            delayed: Vec::new(),
            inflight: Vec::new(),
        })
        .await?;

    let engine = Engine::new().load_checkpoint(persist.clone()).await?;
    println!(
        "restored checkpoint counts: {:?}",
        engine.scheduler.counts()
    );
    println!("Engine::load_checkpoint(...) can restore and attach custom checkpoint persistence.");

    let _fresh_engine = Engine::new().with_checkpoint(persist);
    println!(
        "Engine::with_checkpoint(...) can attach a checkpoint backend to fresh memory scheduling."
    );

    Ok(())
}

async fn memory_snapshot_demo() -> Result<(), SpiderError> {
    println!("== memory scheduler snapshot demo ==");

    let scheduler = scheduler::Memory::default()
        .with_scope("examples:custom-scheduler:memory")
        .with_worker(scheduler::Worker::new("example-memory-worker"));
    scheduler
        .enqueue(
            Task::new(Request::new("https://example.com/memory/ready"))
                .with_priority(5)
                .with_depth(1),
        )
        .await?;
    scheduler
        .enqueue(Task::with_delay(
            Request::new("https://example.com/memory/delayed"),
            500,
        ))
        .await?;

    let claimed = scheduler
        .take_ready()
        .await?
        .expect("memory task should exist");

    let counts = Scheduler::counts(&scheduler).await?;
    let snapshot = scheduler.snapshot().await?;

    println!("counts: {:?}", counts);
    println!("scope: {}", snapshot.scope);
    println!("workers: {:?}", snapshot.worker_ids);
    println!(
        "reclaimed_total={}, reclaimed_in_refresh={}",
        snapshot.reclaimed_total, snapshot.reclaimed_in_refresh
    );
    for task in &snapshot.inflight_tasks {
        println!(
            "inflight task={} url={} worker={:?} lease={:?}",
            task.task_id.as_str(),
            task.url,
            task.worker_id,
            task.lease_id
        );
    }

    scheduler.complete(&claimed.lease).await?;
    scheduler.close().await?;
    Ok(())
}

async fn memory_scope_overview_demo() -> Result<(), SpiderError> {
    println!("== memory scheduler scope overview demo ==");

    let scheduler = scheduler::Memory::default()
        .with_scope("examples:custom-scheduler:memory-overview")
        .with_worker(scheduler::Worker::new("example-memory-worker"));
    let scopes = scheduler
        .scopes_with_prefix("examples:custom-scheduler:")
        .await?;
    println!("visible scopes: {:?}", scopes);

    let snapshots = scheduler
        .snapshots_with_prefix("examples:custom-scheduler:")
        .await?;
    for snapshot in snapshots {
        println!("scope: {}", snapshot.scope);
        println!("counts: {:?}", snapshot.counts);
        println!("workers: {:?}", snapshot.worker_ids);
    }
    let overview = scheduler
        .overview_with_prefix("examples:custom-scheduler:")
        .await?;
    println!("overview: {:?}", overview);

    scheduler.close().await?;
    Ok(())
}

async fn memory_control_demo() -> Result<(), SpiderError> {
    println!("== memory scheduler control demo ==");

    let scheduler = scheduler::Memory::default()
        .with_scope("examples:custom-scheduler:memory-control")
        .with_worker(scheduler::Worker::new("example-memory-control-worker"));
    scheduler
        .enqueue(Task::new(Request::new(
            "https://example.com/memory-control/ready",
        )))
        .await?;

    scheduler
        .pause_scope("examples:custom-scheduler:memory-control")
        .await?;
    println!(
        "take while paused: {:?}",
        scheduler
            .take_ready()
            .await?
            .as_ref()
            .map(|task| task.task.request.url.as_str())
    );

    scheduler
        .resume_scope("examples:custom-scheduler:memory-control")
        .await?;
    let claimed = scheduler
        .take_ready()
        .await?
        .expect("memory control task should exist");
    println!("claimed after resume: {}", claimed.task.request.url);

    let released = scheduler
        .release_scope("examples:custom-scheduler:memory-control")
        .await?;
    println!("released via control: {released}");

    let removed = scheduler
        .purge_scope("examples:custom-scheduler:memory-control")
        .await?;
    println!("purged counts: {:?}", removed);

    scheduler.close().await?;
    Ok(())
}

async fn memory_batch_demo() -> Result<(), SpiderError> {
    println!("== memory scheduler batch demo ==");

    let scheduler = scheduler::Memory::default()
        .with_scope("examples:custom-scheduler:memory-batch")
        .with_worker(scheduler::Worker::new("example-memory-batch-worker"));
    scheduler
        .enqueue(Task::new(Request::new("https://example.com/batch/a")))
        .await?;
    scheduler
        .enqueue(Task::new(Request::new("https://example.com/batch/b")))
        .await?;
    scheduler
        .enqueue(Task::new(Request::new("https://example.com/batch/c")))
        .await?;

    let claimed = scheduler.take_batch_ready(2).await?;
    println!(
        "claimed batch urls: {:?}",
        claimed
            .iter()
            .map(|task| task.task.request.url.as_str())
            .collect::<Vec<_>>()
    );
    scheduler
        .complete_batch(claimed.iter().map(|task| task.lease.clone()).collect())
        .await?;

    let remaining = scheduler.take_batch_ready(1).await?;
    scheduler
        .complete_and_enqueue_batch(vec![TaskResolution::new(
            remaining[0].lease.clone(),
            vec![Task::new(Request::new("https://example.com/batch/follow"))],
        )])
        .await?;

    println!(
        "counts after batch operations: {:?}",
        Scheduler::counts(&scheduler).await?
    );

    scheduler.close().await?;
    Ok(())
}

async fn sqlite_snapshot_demo() -> Result<(), SpiderError> {
    println!("== sqlite scheduler snapshot demo ==");

    let path = sqlite_demo_path("snapshot");
    let scheduler = scheduler::Sqlite::new(&path, "examples:custom-scheduler:sqlite")
        .with_worker(scheduler::Worker::new("example-sqlite-worker"));
    scheduler
        .enqueue(
            Task::new(Request::new("https://example.com/sqlite/ready"))
                .with_priority(5)
                .with_depth(1),
        )
        .await?;
    scheduler
        .enqueue(Task::with_delay(
            Request::new("https://example.com/sqlite/delayed"),
            500,
        ))
        .await?;

    let claimed = scheduler
        .take_ready()
        .await?
        .expect("sqlite task should exist");

    let counts = Scheduler::counts(&scheduler).await?;
    let snapshot = scheduler.snapshot().await?;

    println!("db path: {}", path.display());
    println!("counts: {:?}", counts);
    println!("scope: {}", snapshot.scope);
    println!("workers: {:?}", snapshot.worker_ids);
    println!(
        "reclaimed_total={}, reclaimed_in_refresh={}",
        snapshot.reclaimed_total, snapshot.reclaimed_in_refresh
    );
    for task in &snapshot.inflight_tasks {
        println!(
            "inflight task={} url={} worker={:?} lease={:?}",
            task.task_id.as_str(),
            task.url,
            task.worker_id,
            task.lease_id
        );
    }

    scheduler.complete(&claimed.lease).await?;
    scheduler.close().await?;
    cleanup_sqlite_demo(&path);
    Ok(())
}

async fn sqlite_batch_demo() -> Result<(), SpiderError> {
    println!("== sqlite scheduler batch demo ==");

    let path = sqlite_demo_path("batch");
    let scheduler = scheduler::Sqlite::new(&path, "examples:custom-scheduler:sqlite-batch")
        .with_worker(scheduler::Worker::new("example-sqlite-batch-worker"));
    scheduler
        .enqueue(Task::new(Request::new("https://example.com/sqlite-batch/a")))
        .await?;
    scheduler
        .enqueue(Task::new(Request::new("https://example.com/sqlite-batch/b")))
        .await?;
    scheduler
        .enqueue(Task::new(Request::new("https://example.com/sqlite-batch/c")))
        .await?;

    let claimed = scheduler.take_batch_ready(2).await?;
    println!(
        "sqlite claimed batch urls: {:?}",
        claimed
            .iter()
            .map(|task| task.task.request.url.as_str())
            .collect::<Vec<_>>()
    );
    scheduler
        .complete_batch(claimed.iter().map(|task| task.lease.clone()).collect())
        .await?;

    let remaining = scheduler.take_batch_ready(1).await?;
    scheduler
        .complete_and_enqueue_batch(vec![TaskResolution::new(
            remaining[0].lease.clone(),
            vec![Task::new(Request::new(
                "https://example.com/sqlite-batch/follow",
            ))],
        )])
        .await?;

    println!(
        "sqlite batch counts: {:?}",
        Scheduler::counts(&scheduler).await?
    );

    scheduler.close().await?;
    cleanup_sqlite_demo(&path);
    Ok(())
}

async fn sqlite_control_demo() -> Result<(), SpiderError> {
    println!("== sqlite scheduler control demo ==");

    let path = sqlite_demo_path("control");
    let scheduler = scheduler::Sqlite::new(&path, "examples:custom-scheduler:sqlite-control")
        .with_worker(scheduler::Worker::new("example-sqlite-control-worker"));
    scheduler
        .enqueue(Task::new(Request::new(
            "https://example.com/sqlite-control/ready",
        )))
        .await?;

    scheduler
        .pause_scope("examples:custom-scheduler:sqlite-control")
        .await?;
    println!(
        "take while paused: {:?}",
        scheduler
            .take_ready()
            .await?
            .as_ref()
            .map(|task| task.task.request.url.as_str())
    );

    scheduler
        .resume_scope("examples:custom-scheduler:sqlite-control")
        .await?;
    let claimed = scheduler
        .take_ready()
        .await?
        .expect("sqlite control task should exist");
    println!("claimed after resume: {}", claimed.task.request.url);

    let released = scheduler
        .release_scope("examples:custom-scheduler:sqlite-control")
        .await?;
    println!("released via control: {released}");

    let removed = scheduler
        .purge_scope("examples:custom-scheduler:sqlite-control")
        .await?;
    println!("purged counts: {:?}", removed);

    scheduler.close().await?;
    cleanup_sqlite_demo(&path);
    Ok(())
}

async fn sqlite_scope_overview_demo() -> Result<(), SpiderError> {
    println!("== sqlite scheduler multi-scope overview demo ==");

    let path = sqlite_demo_path("overview");
    let news = scheduler::Sqlite::new(&path, "examples:custom-scheduler:sqlite-news")
        .with_worker(scheduler::Worker::new("example-sqlite-news-worker"));
    let blog = scheduler::Sqlite::new(&path, "examples:custom-scheduler:sqlite-blog")
        .with_worker(scheduler::Worker::new("example-sqlite-blog-worker"));

    news.enqueue(Task::new(Request::new(
        "https://example.com/sqlite-overview/news",
    )))
    .await?;
    blog.enqueue(Task::with_delay(
        Request::new("https://example.com/sqlite-overview/blog"),
        500,
    ))
    .await?;
    let _claimed = news.take_ready().await?;

    let scopes = news
        .scopes_with_prefix("examples:custom-scheduler:sqlite-")
        .await?;
    println!("visible scopes: {:?}", scopes);

    let snapshots = news
        .snapshots_with_prefix("examples:custom-scheduler:sqlite-")
        .await?;
    for snapshot in snapshots {
        println!("scope: {}", snapshot.scope);
        println!("counts: {:?}", snapshot.counts);
        println!("workers: {:?}", snapshot.worker_ids);
    }
    let overview = news
        .overview_with_prefix("examples:custom-scheduler:sqlite-")
        .await?;
    println!("overview: {:?}", overview);

    news.close().await?;
    blog.close().await?;
    cleanup_sqlite_demo(&path);
    Ok(())
}

async fn redis_snapshot_demo() -> Result<(), SpiderError> {
    println!("== redis scheduler snapshot demo ==");

    let Ok(url) = std::env::var("HALO_SPIDER_EXAMPLE_REDIS_URL") else {
        println!(
            "set HALO_SPIDER_EXAMPLE_REDIS_URL=redis://127.0.0.1:6379 to run the Redis snapshot demo"
        );
        return Ok(());
    };

    let scheduler = scheduler::Redis::new(url, "examples:custom-scheduler:snapshot")
        .with_worker(scheduler::Worker::new("example-observer"));
    let snapshot = scheduler.snapshot().await?;

    println!("scope snapshot counts: {:?}", snapshot.counts);
    println!("scope snapshot workers: {:?}", snapshot.worker_ids);
    println!(
        "scope snapshot reclaimed_total={}, reclaimed_in_refresh={}",
        snapshot.reclaimed_total, snapshot.reclaimed_in_refresh
    );

    scheduler.close().await?;
    Ok(())
}

async fn redis_scope_overview_demo() -> Result<(), SpiderError> {
    println!("== redis scheduler multi-scope overview demo ==");

    let Ok(url) = std::env::var("HALO_SPIDER_EXAMPLE_REDIS_URL") else {
        println!(
            "set HALO_SPIDER_EXAMPLE_REDIS_URL=redis://127.0.0.1:6379 to run the Redis scope overview demo"
        );
        return Ok(());
    };

    let scheduler = scheduler::Redis::new(url, "examples:custom-scheduler:ops")
        .with_worker(scheduler::Worker::new("example-ops"));
    let scopes = scheduler
        .scopes_with_prefix("examples:custom-scheduler:")
        .await?;
    println!("registered scopes: {:?}", scopes);

    let snapshots = scheduler
        .snapshots_with_prefix("examples:custom-scheduler:")
        .await?;
    for snapshot in snapshots {
        println!("scope: {}", snapshot.scope);
        println!("counts: {:?}", snapshot.counts);
        println!("workers: {:?}", snapshot.worker_ids);
    }
    let overview = scheduler
        .overview_with_prefix("examples:custom-scheduler:")
        .await?;
    println!("overview: {:?}", overview);

    scheduler.close().await?;

    Ok(())
}

async fn redis_control_demo() -> Result<(), SpiderError> {
    println!("== redis scheduler control demo ==");

    let Ok(url) = std::env::var("HALO_SPIDER_EXAMPLE_REDIS_URL") else {
        println!(
            "set HALO_SPIDER_EXAMPLE_REDIS_URL=redis://127.0.0.1:6379 to run the Redis control demo"
        );
        return Ok(());
    };

    let target = "examples:custom-scheduler:control";
    let worker = scheduler::Redis::new(url.clone(), target)
        .with_worker(scheduler::Worker::new("example-redis-worker"));
    worker
        .enqueue(Task::new(Request::new(
            "https://example.com/redis-control/ready",
        )))
        .await?;

    let ops = scheduler::Redis::new(url, "examples:custom-scheduler:ops-control")
        .with_worker(scheduler::Worker::new("example-redis-ops"));
    println!("pause changed: {}", ops.pause_scope(target).await?);
    println!(
        "claim while paused: {:?}",
        worker
            .take_ready()
            .await?
            .as_ref()
            .map(|task| task.task.request.url.as_str())
    );

    println!("resume changed: {}", ops.resume_scope(target).await?);
    let claimed = worker
        .take_ready()
        .await?
        .expect("redis control task should exist");
    println!("claimed after resume: {}", claimed.task.request.url);

    println!("release_scope count: {}", ops.release_scope(target).await?);
    println!("purge_scope counts: {:?}", ops.purge_scope(target).await?);

    worker.close().await?;
    ops.close().await?;
    Ok(())
}

async fn redis_batch_demo() -> Result<(), SpiderError> {
    println!("== redis scheduler batch demo ==");

    let Ok(url) = std::env::var("HALO_SPIDER_EXAMPLE_REDIS_URL") else {
        println!(
            "set HALO_SPIDER_EXAMPLE_REDIS_URL=redis://127.0.0.1:6379 to run the Redis batch demo"
        );
        return Ok(());
    };

    let scheduler = scheduler::Redis::new(url, "examples:custom-scheduler:batch")
        .with_worker(scheduler::Worker::new("example-redis-batch-worker"));
    scheduler
        .enqueue(Task::new(Request::new("https://example.com/redis-batch/a")))
        .await?;
    scheduler
        .enqueue(Task::new(Request::new("https://example.com/redis-batch/b")))
        .await?;

    let claimed = scheduler.take_batch_ready(2).await?;
    println!(
        "redis claimed batch urls: {:?}",
        claimed
            .iter()
            .map(|task| task.task.request.url.as_str())
            .collect::<Vec<_>>()
    );
    scheduler
        .requeue_batch(claimed.iter().map(|task| task.lease.clone()).collect())
        .await?;

    println!(
        "redis batch counts: {:?}",
        Scheduler::counts(&scheduler).await?
    );
    scheduler.close().await?;
    Ok(())
}

fn sqlite_demo_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "halo-spider-example-{name}-{}-{}.db",
        std::process::id(),
        NEXT_SQLITE_DEMO_ID.fetch_add(1, Ordering::Relaxed)
    ));
    path
}

fn cleanup_sqlite_demo(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}
