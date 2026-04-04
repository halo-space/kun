use crate::error::SpiderError;
use crate::item::Item;
use crate::store::Store;
use std::sync::{Arc, Mutex};

/// Built-in store that keeps items in memory.
///
/// This type is mainly useful for tests, debugging, and minimal examples.
#[derive(Debug, Clone, Default)]
pub struct Memory {
    items: Arc<Mutex<Vec<Item>>>,
}

impl Memory {
    /// Return a snapshot of all items currently stored in memory.
    pub fn items(&self) -> Vec<Item> {
        self.items.lock().expect("store memory poisoned").clone()
    }
}

impl Store for Memory {
    async fn write(&self, item: &Item, _spider_name: &str) -> Result<(), SpiderError> {
        self.items
            .lock()
            .map_err(|_| SpiderError::engine("store memory poisoned"))?
            .push(item.clone());
        Ok(())
    }

    async fn batch_write(&self, items: &[Item], _spider_name: &str) -> Result<(), SpiderError> {
        self.items
            .lock()
            .map_err(|_| SpiderError::engine("store memory poisoned"))?
            .extend(items.iter().cloned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn memory_stores_items_in_memory() {
        let store = Memory::default();
        let item = Item::new().with_field("title", Value::String("post".to_string()));

        block_on(store.write(&item, "test")).unwrap();
        assert_eq!(store.items(), vec![item]);
    }

    #[test]
    fn memory_batch_write_stores_all_items_in_memory() {
        let store = Memory::default();
        let first = Item::new().with_field("title", Value::String("first".to_string()));
        let second = Item::new().with_field("title", Value::String("second".to_string()));

        block_on(store.batch_write(&[first.clone(), second.clone()], "test")).unwrap();

        assert_eq!(store.items(), vec![first, second]);
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
