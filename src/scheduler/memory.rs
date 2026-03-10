use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::scheduler::traits::Scheduler;
use crate::scheduler::types::ScheduledTask;
use std::collections::VecDeque;

#[derive(Default)]
pub struct MemoryScheduler {
    queue: VecDeque<ScheduledTask>,
}

impl Scheduler for MemoryScheduler {
    fn enqueue<'a>(&'a mut self, task: ScheduledTask) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            self.queue.push_back(task);
            Ok(())
        })
    }

    fn lease<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<ScheduledTask>, SpiderError>> {
        Box::pin(async move { Ok(self.queue.pop_front()) })
    }

    fn ack<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async { Ok(()) })
    }

    fn nack<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::scheduler::traits::Scheduler;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn memory_scheduler_supports_async_enqueue_and_lease() {
        let mut scheduler = MemoryScheduler::default();
        let task = ScheduledTask {
            request: Request::new("https://example.com"),
        };

        block_on(scheduler.enqueue(task.clone())).unwrap();
        let leased = block_on(scheduler.lease()).unwrap();

        assert_eq!(leased.map(|task| task.request.url), Some(task.request.url));
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
