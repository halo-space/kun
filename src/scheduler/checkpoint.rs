//! Checkpoint is the persistence boundary for scheduler state.
//!
//! This module is intentionally separate from the core scheduler
//! implementations:
//!
//! - [`crate::scheduler::Memory`] and [`crate::scheduler::Redis`] are real schedulers
//! - [`Checkpoint`] is a serializable snapshot of scheduler buckets
//! - [`Persist`] stores or restores that snapshot
//! - [`File`] and [`Redis`] are built-in checkpoint persistence implementations
//! - [`Memory`] is a wrapper that combines core `scheduler::Memory` with `Persist`

use crate::error::SpiderError;
use crate::scheduler::Task;
use serde::{Deserialize, Serialize};

pub mod file;
pub mod memory;
pub mod redis;

pub use file::File;
pub use memory::Memory;
pub use redis::Redis;

/// A serializable boundary for scheduler checkpoint data.
///
/// In the current codebase, checkpoint data means the three task buckets
/// managed by the scheduler: `ready`, `delayed`, and `inflight`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Checkpoint {
    pub ready: Vec<Task>,
    pub delayed: Vec<Task>,
    pub inflight: Vec<Task>,
}

impl Checkpoint {
    /// Returns the task counts for each scheduler bucket.
    pub fn counts(&self) -> Counts {
        Counts {
            ready: self.ready.len(),
            delayed: self.delayed.len(),
            inflight: self.inflight.len(),
        }
    }

    /// Returns whether any task still remains in the checkpoint.
    pub fn has_pending(&self) -> bool {
        self.counts().has_pending()
    }
}

/// Task counts grouped by scheduler buckets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub ready: usize,
    pub delayed: usize,
    pub inflight: usize,
}

impl Counts {
    /// Returns the total number of tracked tasks across all scheduler buckets.
    pub fn total(self) -> usize {
        self.ready + self.delayed + self.inflight
    }

    /// Returns whether any tracked task still remains in the checkpoint.
    pub fn has_pending(self) -> bool {
        self.total() > 0
    }
}

#[allow(async_fn_in_trait)]
/// Persists and restores scheduler checkpoints.
pub trait Persist: Send + Sync {
    /// Loads a persisted scheduler checkpoint.
    async fn load(&self) -> Result<Checkpoint, SpiderError>;

    /// Persists the current scheduler checkpoint.
    async fn save(&self, checkpoint: &Checkpoint) -> Result<(), SpiderError>;
}
