use crate::error::SpiderError;
use crate::scheduler::checkpoint::File;
use crate::scheduler::checkpoint::{Checkpoint, Counts, Persist};
use crate::scheduler::memory::Memory as CoreMemory;
use crate::scheduler::runtime::RuntimeEvent;
use crate::scheduler::{ClaimedTask, Scheduler, Snapshot, Task, TaskLease};

/// Memory scheduler with automatic checkpoint persistence.
///
/// This type keeps using the in-memory scheduling semantics while saving every
/// state transition through a shared `scheduler::checkpoint::Persist`.
pub struct Memory<P = File> {
    scheduler: CoreMemory,
    persist: P,
}

impl<P> Memory<P>
where
    P: Persist,
{
    pub async fn load(persist: P) -> Result<Self, SpiderError> {
        let checkpoint = persist.load().await?;
        Ok(Self {
            scheduler: CoreMemory::from_checkpoint(checkpoint),
            persist,
        })
    }

    pub fn from_parts(scheduler: CoreMemory, persist: P) -> Self {
        Self { scheduler, persist }
    }

    pub fn checkpoint(&self) -> Checkpoint {
        self.scheduler.checkpoint()
    }

    pub fn counts(&self) -> Counts {
        self.scheduler.counts()
    }

    async fn save_checkpoint(&self) -> Result<(), SpiderError> {
        self.persist.save(&self.scheduler.checkpoint()).await
    }
}

impl Memory<File> {
    pub async fn load_default() -> Result<Self, SpiderError> {
        Self::load(File::default()).await
    }
}

impl Default for Memory<File> {
    fn default() -> Self {
        Self::from_parts(CoreMemory::default(), File::default())
    }
}

impl<P> Scheduler for Memory<P>
where
    P: Persist,
{
    async fn enqueue(&self, task: Task) -> Result<(), SpiderError> {
        self.scheduler.enqueue(task).await?;
        self.save_checkpoint().await
    }

    async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        Ok(Memory::<P>::checkpoint(self))
    }

    async fn counts(&self) -> Result<Counts, SpiderError> {
        Ok(Memory::<P>::counts(self))
    }

    async fn snapshot(&self) -> Result<Snapshot, SpiderError> {
        self.scheduler.snapshot().await
    }

    async fn take_ready(&self) -> Result<Option<ClaimedTask>, SpiderError> {
        let task = self.scheduler.take_ready().await?;
        self.save_checkpoint().await?;
        Ok(task)
    }

    async fn complete(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        self.scheduler.complete(lease).await?;
        self.save_checkpoint().await
    }

    async fn complete_and_enqueue(
        &self,
        lease: &TaskLease,
        tasks: Vec<Task>,
    ) -> Result<(), SpiderError> {
        self.scheduler.complete_and_enqueue(lease, tasks).await?;
        self.save_checkpoint().await
    }

    async fn requeue(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        self.scheduler.requeue(lease).await?;
        self.save_checkpoint().await
    }

    async fn release_inflight(&self) -> Result<usize, SpiderError> {
        let released = self.scheduler.release_inflight().await?;
        self.save_checkpoint().await?;
        Ok(released)
    }

    async fn close(&self) -> Result<(), SpiderError> {
        self.scheduler.close().await?;
        self.save_checkpoint().await
    }

    fn runtime_scope(&self) -> Option<String> {
        self.scheduler.runtime_scope()
    }

    fn runtime_worker_id(&self) -> Option<String> {
        self.scheduler.runtime_worker_id()
    }

    fn drain_runtime_events(&self) -> Vec<RuntimeEvent> {
        self.scheduler.drain_runtime_events()
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        self.scheduler.has_pending().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::scheduler::checkpoint::File;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_FILE_ID: AtomicUsize = AtomicUsize::new(0);

    #[tokio::test]
    async fn checkpoint_memory_restores_saved_checkpoint() {
        let path = unique_path("restore");
        let persist = File::new(path.clone());
        persist
            .save(&Checkpoint {
                ready: vec![Task::new(Request::new("https://example.com/ready")).with_priority(3)],
                delayed: Vec::new(),
                inflight: Vec::new(),
            })
            .await
            .unwrap();

        let scheduler = Memory::load(File::new(path.clone())).await.unwrap();

        assert_eq!(scheduler.counts().ready, 1);
        assert_eq!(scheduler.checkpoint().ready[0].priority, 3);

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn checkpoint_memory_saves_checkpoint_after_scheduler_transitions() {
        let path = unique_path("save");
        let scheduler = Memory::load(File::new(path.clone())).await.unwrap();

        scheduler
            .enqueue(
                Task::new(Request::new("https://example.com/first"))
                    .with_priority(2)
                    .with_depth(1),
            )
            .await
            .unwrap();
        let taken = scheduler.take_ready().await.unwrap().unwrap();
        scheduler.complete(&taken.lease).await.unwrap();

        let checkpoint = File::new(path.clone()).load().await.unwrap();

        assert!(checkpoint.ready.is_empty());
        assert!(checkpoint.delayed.is_empty());
        assert!(checkpoint.inflight.is_empty());

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn checkpoint_memory_persists_release_inflight() {
        let path = unique_path("release_inflight");
        let scheduler = Memory::load(File::new(path.clone())).await.unwrap();

        scheduler
            .enqueue(Task::new(Request::new(
                "https://example.com/release-inflight",
            )))
            .await
            .unwrap();
        let taken = scheduler.take_ready().await.unwrap().unwrap();

        assert_eq!(scheduler.release_inflight().await.unwrap(), 1);

        let checkpoint = File::new(path.clone()).load().await.unwrap();
        assert_eq!(checkpoint.ready.len(), 1);
        assert!(checkpoint.inflight.is_empty());
        assert_eq!(checkpoint.ready[0].id, taken.task.id);

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn checkpoint_memory_persists_complete_and_enqueue() {
        let path = unique_path("complete_and_enqueue");
        let scheduler = Memory::load(File::new(path.clone())).await.unwrap();

        scheduler
            .enqueue(Task::new(Request::new("https://example.com/current")))
            .await
            .unwrap();
        let taken = scheduler.take_ready().await.unwrap().unwrap();
        let follow = Task::new(Request::new("https://example.com/follow"));

        scheduler
            .complete_and_enqueue(&taken.lease, vec![follow.clone()])
            .await
            .unwrap();

        let checkpoint = File::new(path.clone()).load().await.unwrap();
        assert_eq!(checkpoint.ready.len(), 1);
        assert_eq!(checkpoint.ready[0].id, follow.id);
        assert!(checkpoint.inflight.is_empty());

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn checkpoint_memory_restores_exact_inflight_snapshot_without_runtime_reclaim() {
        let path = unique_path("inflight_snapshot");
        let inflight = Task::new(Request::new("https://example.com/inflight"));
        File::new(path.clone())
            .save(&Checkpoint {
                ready: Vec::new(),
                delayed: Vec::new(),
                inflight: vec![inflight.clone()],
            })
            .await
            .unwrap();

        let scheduler = Memory::load(File::new(path.clone())).await.unwrap();

        assert_eq!(scheduler.counts().ready, 0);
        assert_eq!(scheduler.counts().inflight, 1);
        assert!(scheduler.take_ready().await.unwrap().is_none());
        assert_eq!(scheduler.checkpoint().inflight[0].id, inflight.id);

        tokio::fs::remove_file(path).await.ok();
    }

    fn unique_path(label: &str) -> PathBuf {
        let id = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("halo_spider_checkpoint_memory_{label}_{id}.json"))
    }
}
