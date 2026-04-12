use crate::item;
use crate::request::Request;
use crate::response::Response;
use crate::scheduler::TaskId;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Enqueue {
    pub task_id: TaskId,
    pub spider_name: Option<String>,
    pub request: Request,
}

impl Enqueue {
    pub fn new(request: Request) -> Self {
        Self {
            task_id: TaskId::new(),
            spider_name: None,
            request,
        }
    }

    pub fn with_task_id(mut self, task_id: TaskId) -> Self {
        self.task_id = task_id;
        self
    }

    pub fn with_spider_name(mut self, spider_name: impl Into<String>) -> Self {
        self.spider_name = Some(spider_name.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct Download {
    pub task_id: TaskId,
    pub spider_name: Option<String>,
    pub request: Request,
    pub attempt: u32,
    pub request_started_at: Option<u64>,
    stats: Option<Arc<crate::stats::Tracker>>,
}

impl Download {
    pub fn new(request: Request) -> Self {
        Self {
            task_id: TaskId::new(),
            spider_name: None,
            request,
            attempt: 1,
            request_started_at: None,
            stats: None,
        }
    }

    pub fn with_task_id(mut self, task_id: TaskId) -> Self {
        self.task_id = task_id;
        self
    }

    pub fn with_spider_name(mut self, spider_name: impl Into<String>) -> Self {
        self.spider_name = Some(spider_name.into());
        self
    }

    pub fn with_attempt(mut self, attempt: u32) -> Self {
        self.attempt = attempt.max(1);
        self
    }

    pub(crate) fn with_stats(mut self, stats: Arc<crate::stats::Tracker>) -> Self {
        self.stats = Some(stats);
        self
    }

    pub(crate) fn stats(&self) -> Option<&crate::stats::Tracker> {
        self.stats.as_deref()
    }
}

#[derive(Debug, Clone)]
pub struct Parse {
    pub task_id: TaskId,
    pub spider_name: Option<String>,
    pub request: Request,
    pub response: Response,
}

impl Parse {
    pub fn new(request: Request, response: Response) -> Self {
        Self {
            task_id: TaskId::new(),
            spider_name: None,
            request,
            response,
        }
    }

    pub fn with_task_id(mut self, task_id: TaskId) -> Self {
        self.task_id = task_id;
        self
    }

    pub fn with_spider_name(mut self, spider_name: impl Into<String>) -> Self {
        self.spider_name = Some(spider_name.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    pub task_id: TaskId,
    pub spider_name: Option<String>,
    pub request: Request,
    pub response: Option<Response>,
    pub item: item::Item,
}

impl Item {
    pub fn new(request: Request, item: item::Item) -> Self {
        Self {
            task_id: TaskId::new(),
            spider_name: None,
            request,
            response: None,
            item,
        }
    }

    pub fn with_task_id(mut self, task_id: TaskId) -> Self {
        self.task_id = task_id;
        self
    }

    pub fn with_spider_name(mut self, spider_name: impl Into<String>) -> Self {
        self.spider_name = Some(spider_name.into());
        self
    }

    pub fn with_response(mut self, response: Response) -> Self {
        self.response = Some(response);
        self
    }
}
