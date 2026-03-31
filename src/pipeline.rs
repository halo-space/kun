use crate::error::SpiderError;
use crate::item::Item;

pub mod json_lines;
pub mod memory;

pub use json_lines::JsonLines;
pub use memory::Memory;

/// Item processing pipeline, similar to Scrapy's `ITEM_PIPELINES`.
///
/// Every item produced by a spider is processed through this pipeline in order.
/// A pipeline can:
/// - modify an item
/// - drop an item by returning `Ok(false)`
/// - persist an item to storage
/// - emit logs or other side effects
///
/// This is the single item output path. If an item should be kept in memory,
/// written to a file, or stored in a database, it should happen through a
/// pipeline implementation.
///
/// The default implementation for `()` is a no-op pipeline.
/// Multiple pipelines can be composed as tuples, for example
/// `(LogPipeline, StorePipeline)`.
#[allow(async_fn_in_trait)]
pub trait Pipeline: Send + Sync {
    /// Called once when the spider starts.
    async fn open(&self, _spider_name: &str) -> Result<(), SpiderError> {
        Ok(())
    }

    /// Process one item. Return `true` to keep it, `false` to drop it.
    async fn process(&self, _item: &mut Item, _spider_name: &str) -> Result<bool, SpiderError> {
        Ok(true)
    }

    /// Called once when the spider finishes.
    async fn close(&self, _spider_name: &str) -> Result<(), SpiderError> {
        Ok(())
    }
}

/// No-op pipeline.
impl Pipeline for () {}

/// A two-stage pipeline that runs `A` and then `B`.
impl<A: Pipeline, B: Pipeline> Pipeline for (A, B) {
    async fn open(&self, spider_name: &str) -> Result<(), SpiderError> {
        self.0.open(spider_name).await?;
        self.1.open(spider_name).await
    }

    async fn process(&self, item: &mut Item, spider_name: &str) -> Result<bool, SpiderError> {
        if !self.0.process(item, spider_name).await? {
            return Ok(false);
        }
        self.1.process(item, spider_name).await
    }

    async fn close(&self, spider_name: &str) -> Result<(), SpiderError> {
        self.0.close(spider_name).await?;
        self.1.close(spider_name).await
    }
}
