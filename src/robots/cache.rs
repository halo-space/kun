pub mod file;

use crate::error::SpiderError;
use crate::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use file::File;

/// Cached robots policy for one origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub origin: String,
    pub fetched_at: u64,
    pub policy: Policy,
}

impl Entry {
    pub fn new(origin: impl Into<String>, fetched_at: u64, policy: Policy) -> Self {
        Self {
            origin: origin.into(),
            fetched_at,
            policy,
        }
    }
}

/// Serialized robots policy state stored in a cache backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Policy {
    AllowAll,
    DisallowAll,
    Body(String),
}

/// Replaceable robots policy cache boundary.
///
/// The default backend is in-memory and keeps one cached policy per origin for
/// the lifetime of the engine instance. Future persistent backends can reuse
/// this same entry shape.
pub trait Cache: Send + Sync {
    fn load<'a>(&'a self, origin: &'a str) -> BoxFuture<'a, Result<Option<Entry>, SpiderError>>;

    fn save<'a>(&'a self, entry: &'a Entry) -> BoxFuture<'a, Result<(), SpiderError>>;
}

/// In-memory robots cache backend.
#[derive(Debug, Default)]
pub struct Memory {
    entries: tokio::sync::Mutex<BTreeMap<String, Entry>>,
}

impl Cache for Memory {
    fn load<'a>(&'a self, origin: &'a str) -> BoxFuture<'a, Result<Option<Entry>, SpiderError>> {
        Box::pin(async move { Ok(self.entries.lock().await.get(origin).cloned()) })
    }

    fn save<'a>(&'a self, entry: &'a Entry) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            self.entries
                .lock()
                .await
                .insert(entry.origin.clone(), entry.clone());
            Ok(())
        })
    }
}
