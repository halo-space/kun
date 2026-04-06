use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::request::Request;
use halo_spider::scheduler::checkpoint::{Checkpoint, Persist};
use halo_spider::scheduler::{self, Scheduler, Task, TaskLease};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

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
    redis_snapshot_demo().await?;
    redis_namespace_overview_demo().await?;
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

async fn redis_snapshot_demo() -> Result<(), SpiderError> {
    println!("== redis scheduler snapshot demo ==");

    let Ok(url) = std::env::var("HALO_SPIDER_EXAMPLE_REDIS_URL") else {
        println!(
            "set HALO_SPIDER_EXAMPLE_REDIS_URL=redis://127.0.0.1:6379 to run the Redis snapshot demo"
        );
        return Ok(());
    };

    let scheduler = scheduler::Redis::new(url, "examples:custom-scheduler:snapshot")
        .with_worker_id("example-observer");
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

async fn redis_namespace_overview_demo() -> Result<(), SpiderError> {
    println!("== redis scheduler multi-namespace overview demo ==");

    let Ok(url) = std::env::var("HALO_SPIDER_EXAMPLE_REDIS_URL") else {
        println!(
            "set HALO_SPIDER_EXAMPLE_REDIS_URL=redis://127.0.0.1:6379 to run the Redis namespace overview demo"
        );
        return Ok(());
    };

    let scheduler =
        scheduler::Redis::new(url, "examples:custom-scheduler:ops").with_worker_id("example-ops");
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

    scheduler.close().await?;

    Ok(())
}
