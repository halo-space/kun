//! Scheduler is split into two layers:
//!
//! - core scheduler: [`Memory`], [`Redis`], [`Task`], [`TaskId`], [`Scheduler`]
//! - checkpoint layer: [`checkpoint`]
//!
//! Core schedulers own task lifecycle semantics.
//! Checkpoint types only handle exporting or persisting scheduler state.
//! They do not replace runtime recovery behaviors such as durable lease
//! reclaim, which remain the scheduler's responsibility.

pub mod checkpoint;
pub mod memory;
pub mod redis;
pub mod snapshot;
pub mod task;
pub mod traits;

pub use memory::Memory;
pub use redis::Redis;
pub use snapshot::{InflightTaskSnapshot, Snapshot, WorkerSnapshot};
pub use task::{ClaimedTask, Task, TaskId, TaskLease};
pub use traits::Scheduler;
