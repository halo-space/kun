use crate::error::SpiderError;
use crate::scheduler::checkpoint::Counts;

#[allow(async_fn_in_trait)]
/// Scheduler control actions.
///
/// Read access continues to come from `Scheduler` (`scopes`, `snapshots`,
/// `overview`). This trait adds the mutable control actions that should stay
/// distinct from batch execution APIs.
pub trait Control: Send + Sync {
    /// Prevents new claims from being taken from `scope`.
    ///
    /// Returns `true` when the paused state changed.
    async fn pause_scope(&self, scope: &str) -> Result<bool, SpiderError>;

    /// Re-enables claims from `scope`.
    ///
    /// Returns `true` when the paused state changed.
    async fn resume_scope(&self, scope: &str) -> Result<bool, SpiderError>;

    /// Releases every inflight task currently tracked in `scope` back into its
    /// runnable bucket.
    async fn release_scope(&self, scope: &str) -> Result<usize, SpiderError>;

    /// Removes every tracked task from `scope`.
    ///
    /// Returns the number of `ready / delayed / inflight` tasks removed.
    async fn purge_scope(&self, scope: &str) -> Result<Counts, SpiderError>;
}
