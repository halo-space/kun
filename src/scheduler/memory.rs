use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::scheduler::traits::Scheduler;
use crate::scheduler::types::ScheduledTask;
use std::collections::VecDeque;

#[derive(Default)]
pub struct MemoryScheduler {
    ready: VecDeque<ScheduledTask>,
    delayed: Vec<ScheduledTask>,
    inflight: Option<ScheduledTask>,
}

impl MemoryScheduler {
    fn push_task(&mut self, task: ScheduledTask) {
        if task.is_ready() {
            self.ready.push_back(task);
        } else {
            self.delayed.push(task);
        }
    }

    fn promote_delayed(&mut self) {
        let delayed = std::mem::take(&mut self.delayed);

        for task in delayed {
            if task.is_ready() {
                self.ready.push_back(task);
            } else {
                self.delayed.push(task);
            }
        }
    }
}

impl Scheduler for MemoryScheduler {
    fn enqueue<'a>(&'a mut self, task: ScheduledTask) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            self.push_task(task);
            Ok(())
        })
    }

    fn lease<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<ScheduledTask>, SpiderError>> {
        Box::pin(async move {
            if self.inflight.is_some() {
                return Ok(None);
            }

            self.promote_delayed();

            let Some(task) = self.ready.pop_front() else {
                return Ok(None);
            };

            self.inflight = Some(task.clone());
            Ok(Some(task))
        })
    }

    fn ack<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            self.inflight = None;
            Ok(())
        })
    }

    fn nack<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            if let Some(task) = self.inflight.take() {
                self.ready.push_front(task);
            }
            Ok(())
        })
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
        let task = ScheduledTask::new(Request::new("https://example.com"));

        block_on(scheduler.enqueue(task.clone())).unwrap();
        let leased = block_on(scheduler.lease()).unwrap();

        assert_eq!(leased.map(|task| task.request.url), Some(task.request.url));
    }

    #[test]
    fn memory_scheduler_tracks_inflight_until_ack() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/a",
        ))))
        .unwrap();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/b",
        ))))
        .unwrap();

        let first = block_on(scheduler.lease()).unwrap();
        let second = block_on(scheduler.lease()).unwrap();

        assert_eq!(
            first.map(|task| task.request.url),
            Some("https://example.com/a".to_string())
        );
        assert!(second.is_none());

        block_on(scheduler.ack()).unwrap();

        let third = block_on(scheduler.lease()).unwrap();
        assert_eq!(
            third.map(|task| task.request.url),
            Some("https://example.com/b".to_string())
        );
    }

    #[test]
    fn memory_scheduler_nack_requeues_inflight_task() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/retry",
        ))))
        .unwrap();

        let first = block_on(scheduler.lease()).unwrap();
        assert_eq!(
            first.map(|task| task.request.url.clone()),
            Some("https://example.com/retry".to_string())
        );

        block_on(scheduler.nack()).unwrap();

        let second = block_on(scheduler.lease()).unwrap();
        assert_eq!(
            second.map(|task| task.request.url),
            Some("https://example.com/retry".to_string())
        );
    }

    #[test]
    fn memory_scheduler_skips_delayed_task_until_ready() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::with_delay_ms(
            Request::new("https://example.com/delayed"),
            10,
        )))
        .unwrap();

        let first = block_on(scheduler.lease()).unwrap();
        assert!(first.is_none());

        std::thread::sleep(std::time::Duration::from_millis(15));

        let second = block_on(scheduler.lease()).unwrap();
        assert_eq!(
            second.map(|task| task.request.url),
            Some("https://example.com/delayed".to_string())
        );
    }

    #[test]
    fn memory_scheduler_keeps_ready_order_when_delayed_exists() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::with_delay_ms(
            Request::new("https://example.com/delayed"),
            20,
        )))
        .unwrap();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/ready",
        ))))
        .unwrap();

        let first = block_on(scheduler.lease()).unwrap();

        assert_eq!(
            first.map(|task| task.request.url),
            Some("https://example.com/ready".to_string())
        );
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
