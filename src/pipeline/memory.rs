use crate::error::SpiderError;
use crate::item::Item;
use crate::pipeline::Pipeline;
use std::sync::{Arc, Mutex};

/// 将处理过的 Item 保存在内存中的内置 pipeline。
///
/// 这个类型主要用于测试、调试或最小化示例。
#[derive(Debug, Clone, Default)]
pub struct Memory {
    items: Arc<Mutex<Vec<Item>>>,
}

impl Memory {
    /// 返回当前已经写入内存的所有 Item 快照。
    pub fn items(&self) -> Vec<Item> {
        self.items.lock().expect("pipeline memory poisoned").clone()
    }
}

impl Pipeline for Memory {
    async fn process(&self, item: &mut Item, _spider_name: &str) -> Result<bool, SpiderError> {
        self.items
            .lock()
            .map_err(|_| SpiderError::engine("pipeline memory poisoned"))?
            .push(item.clone());
        Ok(true)
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
        let pipeline = Memory::default();
        let mut item = Item::new().with_field("title", Value::String("post".to_string()));

        let keep = block_on(pipeline.process(&mut item, "test")).unwrap();

        assert!(keep);
        assert_eq!(pipeline.items(), vec![item]);
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
