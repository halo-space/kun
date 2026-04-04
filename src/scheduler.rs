//! Scheduler is split into two layers:
//!
//! - core scheduler: [`Memory`], [`Redis`], [`Task`], [`TaskId`], [`Scheduler`]
//! - checkpoint layer: [`checkpoint`]
//!
//! Core schedulers own task lifecycle semantics.
//! Checkpoint types only handle exporting or persisting scheduler state.

pub mod checkpoint;
pub mod memory;
pub mod redis;
pub mod task;
pub mod traits;

pub use memory::Memory;
pub use redis::Redis;
pub use task::{Task, TaskId};
pub use traits::Scheduler;
