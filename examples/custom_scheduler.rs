use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::request::Request;
use halo_spider::scheduler::checkpoint::{Checkpoint, Persist};
use halo_spider::scheduler::{self, Scheduler, Task, TaskLease, TaskResolution};
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
    memory_snapshot_demo().await?;
    memory_batch_demo().await?;
    memory_scope_overview_demo().await?;
    redis_snapshot_demo().await?;
    redis_batch_demo().await?;
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

    println!("counts after batch operations: {:?}", Scheduler::counts(&scheduler).await?);

    scheduler.close().await?;
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

    println!("redis batch counts: {:?}", Scheduler::counts(&scheduler).await?);
    scheduler.close().await?;
    Ok(())
}
