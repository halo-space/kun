use crate::error::SpiderError;
use crate::item::Item;

mod database;
pub mod file;
pub mod kafka;
pub mod memory;
pub mod redis;
pub mod sqlite;
pub mod webhook;

pub use database::FieldColumnType;
pub use file::{File, FileFormat};
pub use kafka::Kafka;
pub use memory::Memory;
pub use redis::Redis;
pub use sqlite::Sqlite;
pub use webhook::{Webhook, WebhookMethod};

/// Final item destination.
///
/// Stores are the last step of the item chain:
/// `parse -> item -> pipeline -> store`.
///
/// A store is responsible for persisting or delivering the final item, for
/// example by writing a file, inserting into a database, or pushing to an API.
#[allow(async_fn_in_trait)]
pub trait Store: Send + Sync {
    /// Called once when the spider starts.
    async fn open(&self, _spider_name: &str) -> Result<(), SpiderError> {
        Ok(())
    }

    /// Persist or deliver one final item.
    async fn write(&self, _item: &Item, _spider_name: &str) -> Result<(), SpiderError>;

    /// Persist or deliver multiple final items in one call.
    ///
    /// The default implementation falls back to `write()` item by item.
    /// Stores may override this to use a native batch operation.
    async fn batch_write(&self, items: &[Item], spider_name: &str) -> Result<(), SpiderError> {
        for item in items {
            self.write(item, spider_name).await?;
        }
        Ok(())
    }

    /// Called once when the spider finishes.
    async fn close(&self, _spider_name: &str) -> Result<(), SpiderError> {
        Ok(())
    }
}
