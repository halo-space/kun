use crate::error::SpiderError;
use crate::scheduler::traits::Scheduler;
use crate::scheduler::types::ScheduledTask;
use std::collections::VecDeque;

#[derive(Default)]
pub struct MemoryScheduler {
    queue: VecDeque<ScheduledTask>,
}

impl Scheduler for MemoryScheduler {
    fn enqueue(&mut self, task: ScheduledTask) -> Result<(), SpiderError> {
        self.queue.push_back(task);
        Ok(())
    }

    fn lease(&mut self) -> Result<Option<ScheduledTask>, SpiderError> {
        Ok(self.queue.pop_front())
    }

    fn ack(&mut self) -> Result<(), SpiderError> {
        Ok(())
    }

    fn nack(&mut self) -> Result<(), SpiderError> {
        Ok(())
    }
}
