use crate::error::SpiderError;
use crate::scheduler::traits::Scheduler;
use crate::scheduler::types::ScheduledTask;
use std::collections::VecDeque;

#[derive(Default)]
pub struct MemoryScheduler {
    ready: VecDeque<ScheduledTask>,
    delayed: Vec<ScheduledTask>,
    inflight: Vec<ScheduledTask>,
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
    async fn enqueue(&mut self, task: ScheduledTask) -> Result<(), SpiderError> {
        self.push_task(task);
        Ok(())
    }

    async fn lease(&mut self) -> Result<Option<ScheduledTask>, SpiderError> {
        self.promote_delayed();

        let Some(task) = self.ready.pop_front() else {
            return Ok(None);
        };

        self.inflight.push(task.clone());
        Ok(Some(task))
    }

    async fn ack(&mut self, url: &str) -> Result<(), SpiderError> {
        self.inflight.retain(|t| t.request.url != url);
        Ok(())
    }

    async fn nack(&mut self, url: &str) -> Result<(), SpiderError> {
        if let Some(pos) = self.inflight.iter().position(|t| t.request.url == url) {
            let task = self.inflight.remove(pos);
            self.ready.push_front(task);
        }
        Ok(())
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        Ok(!self.ready.is_empty() || !self.delayed.is_empty() || !self.inflight.is_empty())
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
            first.as_ref().map(|t| t.request.url.as_str()),
            Some("https://example.com/a")
        );
        assert_eq!(
            second.as_ref().map(|t| t.request.url.as_str()),
            Some("https://example.com/b")
        );

        assert!(block_on(scheduler.has_pending()).unwrap());

        block_on(scheduler.ack("https://example.com/a")).unwrap();
        block_on(scheduler.ack("https://example.com/b")).unwrap();

        assert!(!block_on(scheduler.has_pending()).unwrap());
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

        block_on(scheduler.nack("https://example.com/retry")).unwrap();

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
