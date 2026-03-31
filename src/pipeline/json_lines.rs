use crate::error::SpiderError;
use crate::item::Item;
use crate::pipeline::Pipeline;
use crate::value::Value;
use std::fs::{OpenOptions, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Built-in pipeline that writes processed items to a JSON Lines file.
///
/// `open()` ensures the parent directory exists and truncates the target file.
/// `process()` appends each item as a single JSON object line.
#[derive(Debug, Clone)]
pub struct JsonLines {
    path: PathBuf,
}

impl JsonLines {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Pipeline for JsonLines {
    async fn open(&self, _spider_name: &str) -> Result<(), SpiderError> {
        ensure_parent_dir(&self.path)?;
        std::fs::write(&self.path, []).map_err(|error| {
            SpiderError::engine(format!("failed to initialize json lines pipeline: {error}"))
        })?;
        Ok(())
    }

    async fn process(&self, item: &mut Item, _spider_name: &str) -> Result<bool, SpiderError> {
        ensure_parent_dir(&self.path)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| {
                SpiderError::engine(format!("failed to open json lines pipeline file: {error}"))
            })?;
        let line = serde_json::to_string(&item_to_json(item)).map_err(|error| {
            SpiderError::engine(format!("failed to serialize item into json lines: {error}"))
        })?;

        file.write_all(line.as_bytes()).map_err(|error| {
            SpiderError::engine(format!("failed to write json lines record: {error}"))
        })?;
        file.write_all(b"\n").map_err(|error| {
            SpiderError::engine(format!("failed to finalize json lines record: {error}"))
        })?;

        Ok(true)
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), SpiderError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    create_dir_all(parent).map_err(|error| {
        SpiderError::engine(format!("failed to create pipeline directory: {error}"))
    })?;
    Ok(())
}

fn item_to_json(item: &Item) -> serde_json::Value {
    serde_json::Value::Object(
        item.fields
            .iter()
            .map(|(key, value)| (key.clone(), value_to_json(value)))
            .collect(),
    )
}

fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(value) => serde_json::Value::Bool(*value),
        Value::Number(value) => serde_json::Value::from(*value),
        Value::String(value) => serde_json::Value::String(value.clone()),
        Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(value_to_json).collect())
        }
        Value::Object(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), value_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Wake, Waker};

    static NEXT_TEMP_FILE_ID: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn json_lines_pipeline_writes_one_item_per_line() {
        let path = unique_path("writes");
        let pipeline = JsonLines::new(path.clone());
        let mut item = Item::new()
            .with_field("title", Value::String("period".to_string()))
            .with_field("front_page", Value::String("A01".to_string()));

        block_on(pipeline.open("test")).unwrap();
        let keep = block_on(pipeline.process(&mut item, "test")).unwrap();

        assert!(keep);
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "{\"front_page\":\"A01\",\"title\":\"period\"}\n".to_string()
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn json_lines_pipeline_open_truncates_existing_file() {
        let path = unique_path("truncate");
        ensure_parent_dir(&path).unwrap();
        std::fs::write(&path, b"stale\n").unwrap();

        let pipeline = JsonLines::new(path.clone());
        block_on(pipeline.open("test")).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.is_empty());

        std::fs::remove_file(path).ok();
    }

    fn unique_path(label: &str) -> PathBuf {
        let id = NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("halo_spider_json_lines_{label}_{id}.jsonl"))
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut future = Pin::from(Box::new(future));
        let mut context = Context::from_waker(&waker);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }
}
