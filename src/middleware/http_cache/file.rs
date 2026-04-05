use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::middleware::http_cache::{Cache, Entry};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// File-backed HTTP cache backend.
///
/// This persists one JSON map of `cache_key -> entry`, so future engine
/// instances can reuse validators or cached response bodies.
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
        Self::new("output/http-cache.json")
    }
}

impl Cache for File {
    fn load<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Entry>, SpiderError>> {
        Box::pin(async move {
            let _guard = self.lock.lock().await;
            let entries = load_entries(&self.path).await?;
            Ok(entries.get(key).cloned())
        })
    }

    fn save<'a>(&'a self, entry: &'a Entry) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            let _guard = self.lock.lock().await;
            let mut entries = load_entries(&self.path).await?;
            entries.insert(entry.key.clone(), entry.clone());
            save_entries(&self.path, &entries).await
        })
    }

    fn remove<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            let _guard = self.lock.lock().await;
            let mut entries = load_entries(&self.path).await?;
            if entries.remove(key).is_some() {
                save_entries(&self.path, &entries).await?;
            }
            Ok(())
        })
    }
}

async fn load_entries(path: &Path) -> Result<BTreeMap<String, Entry>, SpiderError> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(SpiderError::engine(format!(
                "failed to load http cache file: {error}"
            )));
        }
    };

    serde_json::from_slice(&bytes)
        .map_err(|error| SpiderError::engine(format!("failed to decode http cache file: {error}")))
}

async fn save_entries(path: &Path, entries: &BTreeMap<String, Entry>) -> Result<(), SpiderError> {
    ensure_parent_dir(path).await?;
    let bytes = serde_json::to_vec_pretty(entries)
        .map_err(|error| SpiderError::engine(format!("failed to encode http cache: {error}")))?;
    let temporary_path = temporary_path(path);

    tokio::fs::write(&temporary_path, bytes)
        .await
        .map_err(|error| {
            SpiderError::engine(format!(
                "failed to write temporary http cache file: {error}"
            ))
        })?;
    tokio::fs::rename(&temporary_path, path)
        .await
        .map_err(|error| {
            SpiderError::engine(format!("failed to finalize http cache file: {error}"))
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
        SpiderError::engine(format!("failed to create http cache directory: {error}"))
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
    use crate::middleware::http_cache::Strategy;
    use crate::request::Request;
    use crate::response::Response;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_FILE_ID: AtomicUsize = AtomicUsize::new(0);

    #[tokio::test]
    async fn file_cache_round_trips_saved_entry() {
        let path = unique_path("round_trip");
        let cache = File::new(path.clone());
        let request = Request::new("https://example.com/feed");
        let response = Response::from_request(
            request,
            200,
            [("ETag".to_string(), vec!["v1".to_string()])]
                .into_iter()
                .collect(),
            b"body".to_vec(),
        );
        let entry = Entry::from_response(
            "https://example.com/feed".to_string(),
            &response,
            Strategy::Response,
            123,
        )
        .unwrap();

        cache.save(&entry).await.unwrap();
        let restored = cache.load("https://example.com/feed").await.unwrap();

        assert_eq!(restored, Some(entry));

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn file_cache_removes_saved_entry() {
        let path = unique_path("remove");
        let cache = File::new(path.clone());
        let request = Request::new("https://example.com/feed");
        let response = Response::from_request(
            request,
            200,
            [("ETag".to_string(), vec!["v1".to_string()])]
                .into_iter()
                .collect(),
            b"body".to_vec(),
        );
        let entry = Entry::from_response(
            "https://example.com/feed".to_string(),
            &response,
            Strategy::Response,
            123,
        )
        .unwrap();

        cache.save(&entry).await.unwrap();
        cache.remove("https://example.com/feed").await.unwrap();

        assert_eq!(cache.load("https://example.com/feed").await.unwrap(), None);

        tokio::fs::remove_file(path).await.ok();
    }

    fn unique_path(label: &str) -> PathBuf {
        let id = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("halo_spider_http_cache_{label}_{id}.json"))
    }
}
