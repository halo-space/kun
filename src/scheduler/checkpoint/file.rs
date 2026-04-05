use crate::error::SpiderError;
use crate::scheduler::checkpoint::{Checkpoint, Persist};
use std::path::{Path, PathBuf};

/// File-backed scheduler checkpoint persistence.
///
/// This persists `scheduler::checkpoint::Checkpoint` as JSON and can restore it
/// when the process starts again.
#[derive(Debug, Clone)]
pub struct File {
    path: PathBuf,
}

impl File {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Default for File {
    fn default() -> Self {
        Self::new("output/scheduler-checkpoint.json")
    }
}

impl Persist for File {
    async fn load(&self) -> Result<Checkpoint, SpiderError> {
        let bytes = match tokio::fs::read(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Checkpoint::default());
            }
            Err(error) => {
                return Err(SpiderError::scheduler(format!(
                    "failed to load scheduler checkpoint file: {error}"
                )));
            }
        };

        serde_json::from_slice(&bytes).map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to decode scheduler checkpoint file: {error}"
            ))
        })
    }

    async fn save(&self, checkpoint: &Checkpoint) -> Result<(), SpiderError> {
        ensure_parent_dir(&self.path).await?;
        let bytes = serde_json::to_vec_pretty(checkpoint).map_err(|error| {
            SpiderError::scheduler(format!("failed to encode scheduler checkpoint: {error}"))
        })?;
        let temporary_path = temporary_path(&self.path);

        tokio::fs::write(&temporary_path, bytes)
            .await
            .map_err(|error| {
                SpiderError::scheduler(format!(
                    "failed to write temporary scheduler checkpoint file: {error}"
                ))
            })?;
        tokio::fs::rename(&temporary_path, &self.path)
            .await
            .map_err(|error| {
                SpiderError::scheduler(format!(
                    "failed to finalize scheduler checkpoint file: {error}"
                ))
            })
    }
}

async fn ensure_parent_dir(path: &Path) -> Result<(), SpiderError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        SpiderError::scheduler(format!(
            "failed to create scheduler checkpoint directory: {error}"
        ))
    })?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(".tmp");
    PathBuf::from(temporary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::scheduler::Task;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_FILE_ID: AtomicUsize = AtomicUsize::new(0);

    #[tokio::test]
    async fn file_round_trips_checkpoint() {
        let path = unique_path("round_trip");
        let persist = File::new(path.clone());
        let checkpoint = Checkpoint {
            ready: vec![Task::new(Request::new("https://example.com/ready")).with_priority(5)],
            delayed: vec![
                Task::with_delay(Request::new("https://example.com/delayed"), 50).with_depth(2),
            ],
            inflight: vec![Task::new(Request::new("https://example.com/inflight"))],
        };

        persist.save(&checkpoint).await.unwrap();
        let restored = persist.load().await.unwrap();

        assert_eq!(restored.ready.len(), 1);
        assert_eq!(restored.ready[0].request.url, "https://example.com/ready");
        assert_eq!(restored.ready[0].priority, 5);
        assert_eq!(restored.delayed.len(), 1);
        assert_eq!(restored.delayed[0].depth, 2);
        assert_eq!(restored.inflight.len(), 1);

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn file_returns_default_checkpoint_when_file_is_missing() {
        let path = unique_path("missing");
        let persist = File::new(path);

        let checkpoint = persist.load().await.unwrap();

        assert!(checkpoint.ready.is_empty());
        assert!(checkpoint.delayed.is_empty());
        assert!(checkpoint.inflight.is_empty());
    }

    fn unique_path(label: &str) -> PathBuf {
        let id = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "halo_spider_scheduler_checkpoint_{label}_{id}.json"
        ))
    }
}
