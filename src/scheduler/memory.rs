use crate::error::SpiderError;
use crate::scheduler::checkpoint::{Checkpoint, Counts};
use crate::scheduler::{Scheduler, Task, TaskId};
use std::cmp::Ordering;
use std::collections::VecDeque;

#[derive(Default)]
pub struct Memory {
    ready: VecDeque<Task>,
    delayed: Vec<Task>,
    inflight: Vec<Task>,
}

impl Memory {
    /// Restores a memory scheduler from an existing checkpoint.
    pub fn from_checkpoint(checkpoint: Checkpoint) -> Self {
        Self {
            ready: VecDeque::from(checkpoint.ready),
            delayed: checkpoint.delayed,
            inflight: checkpoint.inflight,
        }
    }

    /// Exports the current in-memory scheduler checkpoint.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            ready: self.ready.iter().cloned().collect(),
            delayed: self.delayed.clone(),
            inflight: self.inflight.clone(),
        }
    }

    /// Returns the number of tasks tracked in each state bucket.
    pub fn counts(&self) -> Counts {
        Counts {
            ready: self.ready.len(),
            delayed: self.delayed.len(),
            inflight: self.inflight.len(),
        }
    }

    fn push_task(&mut self, task: Task) {
        if task.is_ready() {
            self.push_ready_task(task);
        } else {
            self.delayed.push(task);
        }
    }

    fn push_ready_task(&mut self, task: Task) {
        let insert_at = self
            .ready
            .iter()
            .position(|existing| ready_ordering(&task, existing) == Ordering::Less)
            .unwrap_or(self.ready.len());

        self.ready.insert(insert_at, task);
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

impl Scheduler for Memory {
    async fn enqueue(&mut self, task: Task) -> Result<(), SpiderError> {
        self.push_task(task);
        Ok(())
    }

    async fn take_ready(&mut self) -> Result<Option<Task>, SpiderError> {
        self.promote_delayed();

        let Some(task) = self.ready.pop_front() else {
            return Ok(None);
        };

        self.inflight.push(task.clone());
        Ok(Some(task))
    }

    async fn complete(&mut self, task_id: &TaskId) -> Result<(), SpiderError> {
        self.inflight.retain(|task| &task.id != task_id);
        Ok(())
    }

    async fn requeue(&mut self, task_id: &TaskId) -> Result<(), SpiderError> {
        if let Some(pos) = self.inflight.iter().position(|task| &task.id == task_id) {
            let task = self.inflight.remove(pos);
            self.push_task(task);
        }
        Ok(())
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        Ok(!self.ready.is_empty() || !self.delayed.is_empty() || !self.inflight.is_empty())
    }
}

fn ready_ordering(left: &Task, right: &Task) -> Ordering {
    right
        .priority
        .cmp(&left.priority)
        .then_with(|| left.depth.cmp(&right.depth))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::scheduler::Scheduler;
    use jiff::SignedDuration;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn memory_scheduler_supports_async_enqueue_and_take_ready() {
        let mut scheduler = Memory::default();
        let task = Task::new(Request::new("https://example.com"));

        block_on(scheduler.enqueue(task.clone())).unwrap();
        let taken = block_on(scheduler.take_ready()).unwrap();

        assert_eq!(
            taken.as_ref().map(|task| task.id.as_str()),
            Some(task.id.as_str())
        );
        assert_eq!(taken.map(|task| task.request.url), Some(task.request.url));
    }

    #[test]
    fn memory_scheduler_tracks_inflight_until_complete() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/a")))).unwrap();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/b")))).unwrap();

        let first = block_on(scheduler.take_ready()).unwrap();
        let second = block_on(scheduler.take_ready()).unwrap();

        assert_eq!(
            first.as_ref().map(|t| t.request.url.as_str()),
            Some("https://example.com/a")
        );
        assert_eq!(
            second.as_ref().map(|t| t.request.url.as_str()),
            Some("https://example.com/b")
        );

        assert!(block_on(scheduler.has_pending()).unwrap());

        block_on(scheduler.complete(&first.as_ref().unwrap().id)).unwrap();
        block_on(scheduler.complete(&second.as_ref().unwrap().id)).unwrap();

        assert!(!block_on(scheduler.has_pending()).unwrap());
    }

    #[test]
    fn memory_scheduler_requeue_puts_inflight_task_back_to_ready() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/retry")))).unwrap();

        let first = block_on(scheduler.take_ready()).unwrap().unwrap();
        assert_eq!(first.request.url, "https://example.com/retry".to_string());

        block_on(scheduler.requeue(&first.id)).unwrap();

        let second = block_on(scheduler.take_ready()).unwrap();
        assert_eq!(
            second.map(|task| task.request.url),
            Some("https://example.com/retry".to_string())
        );
    }

    #[test]
    fn memory_scheduler_distinguishes_same_url_tasks_by_task_identity() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(
            Request::new("https://example.com/detail").with_method("GET"),
        )))
        .unwrap();
        block_on(
            scheduler.enqueue(Task::new(
                Request::new("https://example.com/detail")
                    .with_method("POST")
                    .with_meta("page", crate::value::Value::Number(2.0)),
            )),
        )
        .unwrap();

        let first = block_on(scheduler.take_ready()).unwrap().unwrap();
        let second = block_on(scheduler.take_ready()).unwrap().unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(first.request.url, second.request.url);

        block_on(scheduler.complete(&first.id)).unwrap();
        assert!(block_on(scheduler.has_pending()).unwrap());

        block_on(scheduler.complete(&second.id)).unwrap();
        assert!(!block_on(scheduler.has_pending()).unwrap());
    }

    #[test]
    fn memory_scheduler_skips_delayed_task_until_ready() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::with_delay_ms(
            Request::new("https://example.com/delayed"),
            10,
        )))
        .unwrap();

        let first = block_on(scheduler.take_ready()).unwrap();
        assert!(first.is_none());

        std::thread::sleep(std::time::Duration::try_from(SignedDuration::from_millis(15)).unwrap());

        let second = block_on(scheduler.take_ready()).unwrap();
        assert_eq!(
            second.map(|task| task.request.url),
            Some("https://example.com/delayed".to_string())
        );
    }

    #[test]
    fn memory_scheduler_keeps_ready_order_when_delayed_exists() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::with_delay_ms(
            Request::new("https://example.com/delayed"),
            20,
        )))
        .unwrap();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/ready")))).unwrap();

        let first = block_on(scheduler.take_ready()).unwrap();

        assert_eq!(
            first.map(|task| task.request.url),
            Some("https://example.com/ready".to_string())
        );
    }

    #[test]
    fn memory_scheduler_prefers_higher_priority_then_lower_depth() {
        let mut scheduler = Memory::default();
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/depth-2"))
                    .with_priority(0)
                    .with_depth(2),
            ),
        )
        .unwrap();
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/high-priority"))
                    .with_priority(10)
                    .with_depth(8),
            ),
        )
        .unwrap();
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/depth-0"))
                    .with_priority(0)
                    .with_depth(0),
            ),
        )
        .unwrap();

        let first = block_on(scheduler.take_ready()).unwrap().unwrap();
        let second = block_on(scheduler.take_ready()).unwrap().unwrap();
        let third = block_on(scheduler.take_ready()).unwrap().unwrap();

        assert_eq!(first.request.url, "https://example.com/high-priority");
        assert_eq!(second.request.url, "https://example.com/depth-0");
        assert_eq!(third.request.url, "https://example.com/depth-2");
    }

    #[test]
    fn memory_scheduler_keeps_fifo_for_same_priority_and_depth() {
        let mut scheduler = Memory::default();
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/first"))
                    .with_priority(1)
                    .with_depth(2),
            ),
        )
        .unwrap();
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/second"))
                    .with_priority(1)
                    .with_depth(2),
            ),
        )
        .unwrap();

        let first = block_on(scheduler.take_ready()).unwrap().unwrap();
        let second = block_on(scheduler.take_ready()).unwrap().unwrap();

        assert_eq!(first.request.url, "https://example.com/first");
        assert_eq!(second.request.url, "https://example.com/second");
    }

    #[test]
    fn memory_scheduler_exposes_scheduler_state_with_explicit_state_buckets() {
        let mut scheduler = Memory::default();
        let ready = Task::new(Request::new("https://example.com/ready"));
        let delayed = Task::with_delay_ms(Request::new("https://example.com/delayed"), 20);

        block_on(scheduler.enqueue(ready.clone())).unwrap();
        block_on(scheduler.enqueue(delayed.clone())).unwrap();

        let taken = block_on(scheduler.take_ready()).unwrap().unwrap();
        let checkpoint = scheduler.checkpoint();

        assert_eq!(checkpoint.counts().ready, 0);
        assert_eq!(checkpoint.counts().delayed, 1);
        assert_eq!(checkpoint.counts().inflight, 1);
        assert_eq!(checkpoint.counts().total(), 2);
        assert!(checkpoint.has_pending());
        assert_eq!(
            checkpoint.delayed[0].request.url,
            "https://example.com/delayed".to_string()
        );
        assert_eq!(checkpoint.inflight[0].id.as_str(), taken.id.as_str());
    }

    #[test]
    fn memory_scheduler_can_restore_from_checkpoint() {
        let ready = Task::new(Request::new("https://example.com/ready"));
        let delayed = Task::with_delay_ms(Request::new("https://example.com/delayed"), 20);
        let inflight = Task::new(Request::new("https://example.com/inflight"));
        let scheduler = Memory::from_checkpoint(Checkpoint {
            ready: vec![ready.clone()],
            delayed: vec![delayed.clone()],
            inflight: vec![inflight.clone()],
        });

        let counts = scheduler.counts();

        assert_eq!(counts.ready, 1);
        assert_eq!(counts.delayed, 1);
        assert_eq!(counts.inflight, 1);
        assert_eq!(counts.total(), 3);
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
