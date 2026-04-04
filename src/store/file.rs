use crate::error::SpiderError;
use crate::item::Item;
use crate::store::Store;
use std::path::{Path, PathBuf};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

/// Built-in file store.
///
/// `open()` ensures the parent directory exists and truncates the target file.
/// `write()` appends each item as a single JSON object line.
#[derive(Debug, Clone)]
pub struct File {
    path: Option<PathBuf>,
    directory: PathBuf,
}

impl File {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
            directory: PathBuf::from("output"),
        }
    }

    pub fn with_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = directory.into();
        self
    }

    pub fn explicit_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn path_for(&self, spider_name: &str) -> PathBuf {
        self.path
            .clone()
            .unwrap_or_else(|| self.directory.join(format!("{spider_name}.jsonl")))
    }
}

impl Default for File {
    fn default() -> Self {
        Self {
            path: None,
            directory: PathBuf::from("output"),
        }
    }
}

impl Store for File {
    async fn open(&self, spider_name: &str) -> Result<(), SpiderError> {
        let path = self.path_for(spider_name);
        ensure_parent_dir(&path).await?;
        tokio::fs::write(&path, []).await.map_err(|error| {
            SpiderError::engine(format!("failed to initialize file store: {error}"))
        })?;
        Ok(())
    }

    async fn write(&self, item: &Item, spider_name: &str) -> Result<(), SpiderError> {
        let path = self.path_for(spider_name);
        ensure_parent_dir(&path).await?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|error| SpiderError::engine(format!("failed to open file store: {error}")))?;
        let mut buffer = serialize_item_line(item)?;
        buffer.push('\n');

        file.write_all(buffer.as_bytes()).await.map_err(|error| {
            SpiderError::engine(format!("failed to write file store record: {error}"))
        })?;
        file.flush().await.map_err(|error| {
            SpiderError::engine(format!("failed to flush file store record: {error}"))
        })?;

        Ok(())
    }

    async fn batch_write(&self, items: &[Item], spider_name: &str) -> Result<(), SpiderError> {
        if items.is_empty() {
            return Ok(());
        }

        let path = self.path_for(spider_name);
        ensure_parent_dir(&path).await?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|error| SpiderError::engine(format!("failed to open file store: {error}")))?;
        let mut buffer = String::new();

        for item in items {
            buffer.push_str(&serialize_item_line(item)?);
            buffer.push('\n');
        }

        file.write_all(buffer.as_bytes()).await.map_err(|error| {
            SpiderError::engine(format!("failed to write file store batch: {error}"))
        })?;
        file.flush().await.map_err(|error| {
            SpiderError::engine(format!("failed to flush file store batch: {error}"))
        })?;

        Ok(())
    }
}

fn serialize_item_line(item: &Item) -> Result<String, SpiderError> {
    serde_json::to_string(&item.to_json()).map_err(|error| {
        SpiderError::engine(format!("failed to serialize item for file store: {error}"))
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
        SpiderError::engine(format!("failed to create store directory: {error}"))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP_FILE_ID: AtomicUsize = AtomicUsize::new(0);

    #[tokio::test]
    async fn file_store_writes_one_item_per_line() {
        let path = unique_path("writes");
        let store = File::new(path.clone());
        let item = Item::new()
            .with_field("title", Value::String("period".to_string()))
            .with_field("front_page", Value::String("A01".to_string()));

        store.open("test").await.unwrap();
        store.write(&item, "test").await.unwrap();
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(
            content,
            "{\"front_page\":\"A01\",\"title\":\"period\"}\n".to_string()
        );

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn file_store_batch_write_appends_multiple_lines() {
        let path = unique_path("batch");
        let store = File::new(path.clone());
        let first = Item::new().with_field("title", Value::String("first".to_string()));
        let second = Item::new().with_field("title", Value::String("second".to_string()));

        store.open("test").await.unwrap();
        store.batch_write(&[first, second], "test").await.unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(
            content,
            "{\"title\":\"first\"}\n{\"title\":\"second\"}\n".to_string()
        );

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn file_store_open_truncates_existing_file() {
        let path = unique_path("truncate");
        ensure_parent_dir(&path).await.unwrap();
        tokio::fs::write(&path, b"stale\n").await.unwrap();

        let store = File::new(path.clone());
        store.open("test").await.unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.is_empty());

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn file_default_path_uses_output_directory_and_spider_name() {
        let store = File::default().with_directory(std::env::temp_dir());
        let path = store.path_for("period");

        assert_eq!(path, std::env::temp_dir().join("period.jsonl"));
    }

    fn unique_path(label: &str) -> PathBuf {
        let id = NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("halo_spider_file_store_{label}_{id}.jsonl"))
    }
}
