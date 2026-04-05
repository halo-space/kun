use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::robots::cache::{Cache, Entry};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// File-backed robots cache backend.
///
/// This persists one JSON map of `origin -> robots cache entry`, so future
/// engine instances can reuse previously fetched robots policies.
#[derive(Debug)]
pub struct File {
    path: PathBuf,
    lock: tokio::sync::Mutex<()>,
}

impl File {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Default for File {
    fn default() -> Self {
        Self::new("output/robots-cache.json")
    }
}

impl Cache for File {
    fn load<'a>(&'a self, origin: &'a str) -> BoxFuture<'a, Result<Option<Entry>, SpiderError>> {
        Box::pin(async move {
            let _guard = self.lock.lock().await;
            let entries = load_entries(&self.path).await?;
            Ok(entries.get(origin).cloned())
        })
    }

    fn save<'a>(&'a self, entry: &'a Entry) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            let _guard = self.lock.lock().await;
            let mut entries = load_entries(&self.path).await?;
            entries.insert(entry.origin.clone(), entry.clone());
            save_entries(&self.path, &entries).await
        })
    }
}

async fn load_entries(path: &Path) -> Result<BTreeMap<String, Entry>, SpiderError> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(SpiderError::engine(format!(
                "failed to load robots cache file: {error}"
            )));
        }
    };

    serde_json::from_slice(&bytes).map_err(|error| {
        SpiderError::engine(format!("failed to decode robots cache file: {error}"))
    })
}

async fn save_entries(path: &Path, entries: &BTreeMap<String, Entry>) -> Result<(), SpiderError> {
    ensure_parent_dir(path).await?;
    let bytes = serde_json::to_vec_pretty(entries)
        .map_err(|error| SpiderError::engine(format!("failed to encode robots cache: {error}")))?;
    let temporary_path = temporary_path(path);

    tokio::fs::write(&temporary_path, bytes)
        .await
        .map_err(|error| {
            SpiderError::engine(format!(
                "failed to write temporary robots cache file: {error}"
            ))
        })?;
    tokio::fs::rename(&temporary_path, path)
        .await
        .map_err(|error| {
            SpiderError::engine(format!("failed to finalize robots cache file: {error}"))
        })
}

async fn ensure_parent_dir(path: &Path) -> Result<(), SpiderError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        SpiderError::engine(format!("failed to create robots cache directory: {error}"))
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
    use crate::robots::cache::Policy;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_FILE_ID: AtomicUsize = AtomicUsize::new(0);

    #[tokio::test]
    async fn file_cache_round_trips_saved_entry() {
        let path = unique_path("round_trip");
        let cache = File::new(path.clone());
        let entry = Entry::new(
            "https://example.com",
            123,
            Policy::Body("User-agent: *\nDisallow: /private\n".to_string()),
        );

        cache.save(&entry).await.unwrap();
        let restored = cache.load("https://example.com").await.unwrap();

        assert_eq!(restored, Some(entry));

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn file_cache_returns_none_when_path_is_missing() {
        let path = unique_path("missing");
        let cache = File::new(path);

        let restored = cache.load("https://example.com").await.unwrap();

        assert_eq!(restored, None);
    }

    #[tokio::test]
    async fn file_cache_keeps_multiple_origin_entries() {
        let path = unique_path("multiple");
        let cache = File::new(path.clone());
        let first = Entry::new("https://example.com", 1, Policy::AllowAll);
        let second = Entry::new("https://example.org", 2, Policy::DisallowAll);

        cache.save(&first).await.unwrap();
        cache.save(&second).await.unwrap();

        assert_eq!(
            cache.load("https://example.com").await.unwrap(),
            Some(first)
        );
        assert_eq!(
            cache.load("https://example.org").await.unwrap(),
            Some(second)
        );

        tokio::fs::remove_file(path).await.ok();
    }

    fn unique_path(label: &str) -> PathBuf {
        let id = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("halo_spider_robots_cache_{label}_{id}.json"))
    }
}
