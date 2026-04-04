use crate::error::SpiderError;
use crate::item::Item;

/// Item processing stage.
///
/// Every item produced by a spider can be normalized or filtered here before it
/// is handed to the final store.
///
/// A pipeline can:
/// - modify an item
/// - drop an item by returning `Ok(false)`
/// - emit logs or other side effects
///
/// The full item chain is:
/// `parse -> item -> pipeline -> store`.
///
/// The default implementation for `()` is a no-op pipeline stage.
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
