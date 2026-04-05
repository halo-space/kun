use crate::error::SpiderError;
use crate::item::Item;
use crate::store::Store;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileFormat {
    #[default]
    JsonLines,
    PrettyJsonBlocks,
}

/// Built-in file store.
///
/// `open()` ensures the parent directory exists and truncates the target file.
/// `write()` appends each item in the configured file format.
#[derive(Debug, Clone)]
pub struct File {
    path: Option<PathBuf>,
    directory: PathBuf,
    format: FileFormat,
    rotate_items: Option<usize>,
    rotate_bytes: Option<u64>,
    state: Arc<Mutex<FileState>>,
}

#[derive(Debug, Default)]
struct FileState {
    spider_name: Option<String>,
    current_path: Option<PathBuf>,
    current_index: usize,
    written_items: usize,
    written_bytes: u64,
}

impl File {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
            ..Self::default()
        }
    }

    pub fn with_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = directory.into();
        self
    }

    pub fn with_format(mut self, format: FileFormat) -> Self {
        self.format = format;
        self
    }

    pub fn with_rotate_items(mut self, rotate_items: usize) -> Self {
        self.rotate_items = (rotate_items > 0).then_some(rotate_items);
        self
    }

    pub fn with_rotate_bytes(mut self, rotate_bytes: u64) -> Self {
        self.rotate_bytes = (rotate_bytes > 0).then_some(rotate_bytes);
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

    fn rotation_enabled(&self) -> bool {
        self.rotate_items.is_some() || self.rotate_bytes.is_some()
    }

    fn path_for_index(&self, spider_name: &str, index: usize) -> PathBuf {
        let base_path = self.path_for(spider_name);
        if !self.rotation_enabled() {
            return base_path;
        }

        rotated_path(&base_path, index)
    }

    async fn initialize_state(&self, spider_name: &str) -> Result<FileState, SpiderError> {
        let base_path = self.path_for(spider_name);

        if self.rotation_enabled() {
            cleanup_rotated_paths(&base_path).await?;
            let first_path = self.path_for_index(spider_name, 1);
            ensure_parent_dir(&first_path).await?;
            tokio::fs::write(&first_path, []).await.map_err(|error| {
                SpiderError::engine(format!("failed to initialize rotated file store: {error}"))
            })?;

            return Ok(FileState {
                spider_name: Some(spider_name.to_string()),
                current_path: Some(first_path),
                current_index: 1,
                written_items: 0,
                written_bytes: 0,
            });
        }

        ensure_parent_dir(&base_path).await?;
        tokio::fs::write(&base_path, []).await.map_err(|error| {
            SpiderError::engine(format!("failed to initialize file store: {error}"))
        })?;

        Ok(FileState {
            spider_name: Some(spider_name.to_string()),
            current_path: Some(base_path),
            current_index: 0,
            written_items: 0,
            written_bytes: 0,
        })
    }

    async fn ensure_opened(
        &self,
        state: &mut FileState,
        spider_name: &str,
    ) -> Result<(), SpiderError> {
        if state.current_path.is_some() && state.spider_name.as_deref() == Some(spider_name) {
            return Ok(());
        }

        *state = self.initialize_state(spider_name).await?;
        Ok(())
    }

    fn should_rotate(
        &self,
        state: &FileState,
        pending_items: usize,
        pending_bytes: u64,
        next_record_bytes: u64,
    ) -> bool {
        if !self.rotation_enabled() {
            return false;
        }

        let current_items = state.written_items + pending_items;
        let current_bytes = state.written_bytes + pending_bytes;
        if current_items == 0 && current_bytes == 0 {
            return false;
        }

        if let Some(rotate_items) = self.rotate_items
            && current_items + 1 > rotate_items
        {
            return true;
        }

        if let Some(rotate_bytes) = self.rotate_bytes
            && current_bytes + next_record_bytes > rotate_bytes
        {
            return true;
        }

        false
    }

    async fn rotate(&self, state: &mut FileState, spider_name: &str) -> Result<(), SpiderError> {
        state.current_index += 1;
        let next_path = self.path_for_index(spider_name, state.current_index);
        ensure_parent_dir(&next_path).await?;
        tokio::fs::write(&next_path, []).await.map_err(|error| {
            SpiderError::engine(format!("failed to initialize rotated file store: {error}"))
        })?;
        state.current_path = Some(next_path);
        state.written_items = 0;
        state.written_bytes = 0;
        Ok(())
    }

    async fn flush_buffer(
        state: &mut FileState,
        buffer: &mut String,
        buffered_items: &mut usize,
        buffered_bytes: &mut u64,
    ) -> Result<(), SpiderError> {
        if buffer.is_empty() {
            return Ok(());
        }

        let path = state.current_path.clone().ok_or_else(|| {
            SpiderError::engine("file store state is missing the current output path")
        })?;
        append_to_path(&path, buffer.as_bytes()).await?;
        state.written_items += *buffered_items;
        state.written_bytes += *buffered_bytes;
        buffer.clear();
        *buffered_items = 0;
        *buffered_bytes = 0;
        Ok(())
    }

    async fn append_items(&self, items: &[Item], spider_name: &str) -> Result<(), SpiderError> {
        if items.is_empty() {
            return Ok(());
        }

        let mut state = self.state.lock().await;
        self.ensure_opened(&mut state, spider_name).await?;

        let mut buffer = String::new();
        let mut buffered_items = 0usize;
        let mut buffered_bytes = 0u64;

        for item in items {
            let record = serialize_item_record(item, self.format)?;
            let entry = format_record_entry(&record, self.format);
            let entry_bytes = entry.as_bytes().len() as u64;

            if self.should_rotate(&state, buffered_items, buffered_bytes, entry_bytes) {
                Self::flush_buffer(
                    &mut state,
                    &mut buffer,
                    &mut buffered_items,
                    &mut buffered_bytes,
                )
                .await?;
                self.rotate(&mut state, spider_name).await?;
            }

            buffer.push_str(&entry);
            buffered_items += 1;
            buffered_bytes += entry_bytes;
        }

        Self::flush_buffer(
            &mut state,
            &mut buffer,
            &mut buffered_items,
            &mut buffered_bytes,
        )
        .await
    }
}

impl Default for File {
    fn default() -> Self {
        Self {
            path: None,
            directory: PathBuf::from("output"),
            format: FileFormat::default(),
            rotate_items: None,
            rotate_bytes: None,
            state: Arc::new(Mutex::new(FileState::default())),
        }
    }
}

impl Store for File {
    async fn open(&self, spider_name: &str) -> Result<(), SpiderError> {
        let mut state = self.state.lock().await;
        *state = self.initialize_state(spider_name).await?;
        Ok(())
    }

    async fn write(&self, item: &Item, spider_name: &str) -> Result<(), SpiderError> {
        self.append_items(std::slice::from_ref(item), spider_name)
            .await
    }

    async fn batch_write(&self, items: &[Item], spider_name: &str) -> Result<(), SpiderError> {
        self.append_items(items, spider_name).await
    }

    async fn close(&self, _spider_name: &str) -> Result<(), SpiderError> {
        let mut state = self.state.lock().await;
        *state = FileState::default();
        Ok(())
    }
}

fn serialize_item_record(item: &Item, format: FileFormat) -> Result<String, SpiderError> {
    match format {
        FileFormat::JsonLines => serde_json::to_string(&item.to_json()).map_err(|error| {
            SpiderError::engine(format!("failed to serialize item for file store: {error}"))
        }),
        FileFormat::PrettyJsonBlocks => {
            serde_json::to_string_pretty(&item.to_json()).map_err(|error| {
                SpiderError::engine(format!("failed to serialize item for file store: {error}"))
            })
        }
    }
}

fn format_record_entry(record: &str, format: FileFormat) -> String {
    match format {
        FileFormat::JsonLines => format!("{record}\n"),
        FileFormat::PrettyJsonBlocks => format!("{record}\n\n"),
    }
}

fn rotated_path(base_path: &Path, index: usize) -> PathBuf {
    let file_name = base_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "items".to_string());
    let stem = base_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or(file_name.clone());
    let suffix = base_path
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    let rotated_name = format!("{stem}-{index:04}{suffix}");

    base_path
        .parent()
        .map(|parent| parent.join(&rotated_name))
        .unwrap_or_else(|| PathBuf::from(rotated_name))
}

async fn cleanup_rotated_paths(base_path: &Path) -> Result<(), SpiderError> {
    let Some(parent) = base_path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        SpiderError::engine(format!("failed to create store directory: {error}"))
    })?;

    if let Err(error) = tokio::fs::remove_file(base_path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(SpiderError::engine(format!(
            "failed to remove previous file store output: {error}"
        )));
    }

    let file_name = base_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "items".to_string());
    let stem = base_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or(file_name);
    let suffix = base_path
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    let prefix = format!("{stem}-");

    let mut entries = tokio::fs::read_dir(parent).await.map_err(|error| {
        SpiderError::engine(format!("failed to read file store directory: {error}"))
    })?;

    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        SpiderError::engine(format!("failed to iterate file store directory: {error}"))
    })? {
        let candidate = entry.file_name().to_string_lossy().into_owned();
        if is_rotated_file_name(&candidate, &prefix, &suffix) {
            tokio::fs::remove_file(entry.path())
                .await
                .map_err(|error| {
                    SpiderError::engine(format!(
                        "failed to remove rotated file store output: {error}"
                    ))
                })?;
        }
    }

    Ok(())
}

fn is_rotated_file_name(candidate: &str, prefix: &str, suffix: &str) -> bool {
    if !candidate.starts_with(prefix) || !candidate.ends_with(suffix) {
        return false;
    }

    let end = candidate.len().saturating_sub(suffix.len());
    let middle = &candidate[prefix.len()..end];
    middle.len() == 4 && middle.chars().all(|character| character.is_ascii_digit())
}

async fn append_to_path(path: &Path, bytes: &[u8]) -> Result<(), SpiderError> {
    ensure_parent_dir(path).await?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|error| SpiderError::engine(format!("failed to open file store: {error}")))?;
    file.write_all(bytes).await.map_err(|error| {
        SpiderError::engine(format!("failed to write file store record: {error}"))
    })?;
    file.flush().await.map_err(|error| {
        SpiderError::engine(format!("failed to flush file store record: {error}"))
    })?;
    Ok(())
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
    async fn file_store_can_write_pretty_json_blocks() {
        let path = unique_path("pretty");
        let store = File::new(path.clone()).with_format(FileFormat::PrettyJsonBlocks);
        let item = Item::new()
            .with_field("title", Value::String("period".to_string()))
            .with_field("front_page", Value::String("A01".to_string()));

        store.open("test").await.unwrap();
        store.write(&item, "test").await.unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("\n  \"front_page\": \"A01\","));
        assert!(content.ends_with("}\n\n"));

        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn file_store_rotates_by_item_count() {
        let path = unique_path("rotate_items");
        let store = File::new(path.clone()).with_rotate_items(2);
        let first = Item::new().with_field("title", Value::String("first".to_string()));
        let second = Item::new().with_field("title", Value::String("second".to_string()));
        let third = Item::new().with_field("title", Value::String("third".to_string()));

        store.open("period").await.unwrap();
        store
            .batch_write(&[first, second, third], "period")
            .await
            .unwrap();

        let first_path = rotated_path(&path, 1);
        let second_path = rotated_path(&path, 2);
        let first_content = tokio::fs::read_to_string(&first_path).await.unwrap();
        let second_content = tokio::fs::read_to_string(&second_path).await.unwrap();

        assert_eq!(
            first_content,
            "{\"title\":\"first\"}\n{\"title\":\"second\"}\n".to_string()
        );
        assert_eq!(second_content, "{\"title\":\"third\"}\n".to_string());

        tokio::fs::remove_file(first_path).await.ok();
        tokio::fs::remove_file(second_path).await.ok();
    }

    #[tokio::test]
    async fn file_store_rotates_by_byte_size() {
        let path = unique_path("rotate_bytes");
        let store = File::new(path.clone()).with_rotate_bytes(30);
        let first = Item::new().with_field("title", Value::String("first".to_string()));
        let second = Item::new().with_field("title", Value::String("second".to_string()));

        store.open("period").await.unwrap();
        store.batch_write(&[first, second], "period").await.unwrap();

        let first_path = rotated_path(&path, 1);
        let second_path = rotated_path(&path, 2);

        assert!(tokio::fs::try_exists(&first_path).await.unwrap());
        assert!(tokio::fs::try_exists(&second_path).await.unwrap());

        tokio::fs::remove_file(first_path).await.ok();
        tokio::fs::remove_file(second_path).await.ok();
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

    #[test]
    fn rotated_path_appends_index_before_extension() {
        let path = PathBuf::from("output/items.jsonl");

        assert_eq!(
            rotated_path(&path, 3),
            PathBuf::from("output/items-0003.jsonl")
        );
    }

    fn unique_path(label: &str) -> PathBuf {
        let id = NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("halo_spider_file_store_{label}_{id}.jsonl"))
    }
}
