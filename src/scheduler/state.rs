use crate::error::SpiderError;
use crate::scheduler::Task;

/// A serializable boundary for scheduler state.
///
/// In the current codebase, scheduler state means the three task buckets
/// managed by the scheduler: `ready`, `delayed`, and `inflight`.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub ready: Vec<Task>,
    pub delayed: Vec<Task>,
    pub inflight: Vec<Task>,
}

impl Snapshot {
    /// Returns the task counts for each scheduler state bucket.
    pub fn counts(&self) -> Counts {
        Counts {
            ready: self.ready.len(),
            delayed: self.delayed.len(),
            inflight: self.inflight.len(),
        }
    }

    /// Returns whether any task still remains in scheduler state.
    pub fn has_pending(&self) -> bool {
        self.counts().has_pending()
    }
}

/// Task counts grouped by scheduler state buckets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub ready: usize,
    pub delayed: usize,
    pub inflight: usize,
}

impl Counts {
    /// Returns the total number of tracked tasks across all state buckets.
    pub fn total(self) -> usize {
        self.ready + self.delayed + self.inflight
    }

    /// Returns whether any tracked task still remains in scheduler state.
    pub fn has_pending(self) -> bool {
        self.total() > 0
    }
}

#[allow(async_fn_in_trait)]
/// Persists and restores scheduler state snapshots.
pub trait Store: Send + Sync {
    /// Loads persisted scheduler state from the backing store.
    async fn load(&self) -> Result<Snapshot, SpiderError>;

    /// Persists the current scheduler state into the backing store.
    async fn save(&self, snapshot: &Snapshot) -> Result<(), SpiderError>;
}
