use crate::error::SpiderError;
use crate::item::Item;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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

type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

pub const DEFAULT_STORE_KEY: &str = "default";

/// One runtime store registration entry.
///
/// `Engine::with_stores(...)` accepts a list of `StoreEntry`, where `key`
/// is the runtime store registry name resolved by rules step output routing.
pub struct StoreEntry {
    pub key: String,
    pub(crate) store: SharedStore,
}

impl StoreEntry {
    pub fn new(key: impl Into<String>, store: impl Store + 'static) -> Self {
        Self {
            key: key.into(),
            store: shared_store(store),
        }
    }
}

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

pub(crate) trait StoreObject: Send + Sync {
    fn open<'a>(&'a self, spider_name: &'a str) -> LocalBoxFuture<'a, Result<(), SpiderError>>;
    fn batch_write<'a>(
        &'a self,
        items: &'a [Item],
        spider_name: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), SpiderError>>;
    fn close<'a>(&'a self, spider_name: &'a str) -> LocalBoxFuture<'a, Result<(), SpiderError>>;
}

impl<T> StoreObject for T
where
    T: Store,
{
    fn open<'a>(&'a self, spider_name: &'a str) -> LocalBoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move { Store::open(self, spider_name).await })
    }

    fn batch_write<'a>(
        &'a self,
        items: &'a [Item],
        spider_name: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move { Store::batch_write(self, items, spider_name).await })
    }

    fn close<'a>(&'a self, spider_name: &'a str) -> LocalBoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move { Store::close(self, spider_name).await })
    }
}

pub(crate) type SharedStore = Arc<dyn StoreObject>;

pub(crate) fn shared_store(store: impl Store + 'static) -> SharedStore {
    Arc::new(store)
}
