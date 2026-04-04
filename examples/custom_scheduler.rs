use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::request::Request;
use halo_spider::scheduler::checkpoint::{Checkpoint, Persist};
use halo_spider::scheduler::{self, Scheduler, Task, TaskId};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Custom scheduler example.
///
/// This reuses the built-in memory scheduler for actual ready/delayed/inflight
/// semantics, and adds custom bookkeeping around completed task ids.
#[derive(Default)]
struct RecordingScheduler {
    inner: scheduler::Memory,
    completed_task_ids: Vec<String>,
}

impl RecordingScheduler {
    fn completed_task_ids(&self) -> &[String] {
        self.completed_task_ids.as_slice()
    }
}

impl Scheduler for RecordingScheduler {
    async fn enqueue(&mut self, task: Task) -> Result<(), SpiderError> {
        println!("custom scheduler enqueue: {}", task.request.url);
        self.inner.enqueue(task).await
    }

    async fn take_ready(&mut self) -> Result<Option<Task>, SpiderError> {
        let task = self.inner.take_ready().await?;
        if let Some(task) = &task {
            println!("custom scheduler take_ready: {}", task.request.url);
        }
        Ok(task)
    }

    async fn complete(&mut self, task_id: &TaskId) -> Result<(), SpiderError> {
        self.completed_task_ids.push(task_id.as_str().to_string());
        self.inner.complete(task_id).await
    }

    async fn requeue(&mut self, task_id: &TaskId) -> Result<(), SpiderError> {
        self.inner.requeue(task_id).await
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
    Ok(())
}

async fn custom_scheduler_demo() -> Result<(), SpiderError> {
    println!("== custom scheduler demo ==");

    let mut scheduler = RecordingScheduler::default();
    let task = Task::new(Request::new("https://example.com/custom-scheduler"));

    scheduler.enqueue(task).await?;
    let taken = scheduler
        .take_ready()
        .await?
        .expect("custom scheduler should yield one ready task");
    scheduler.complete(&taken.id).await?;

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
