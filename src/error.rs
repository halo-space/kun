use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SchedulerError {
    #[error("{0}")]
    Message(String),
    #[error(
        "scheduler lease belongs to worker `{lease_worker_id}`, current worker is `{current_worker_id}`"
    )]
    LeaseWorkerMismatch {
        lease_worker_id: String,
        current_worker_id: String,
    },
    #[error(
        "scheduler cannot {action} task `{task_id}` because worker `{worker_id}` no longer owns its lease"
    )]
    LeaseOwnershipConflict {
        action: &'static str,
        task_id: String,
        worker_id: String,
    },
    #[error(
        "scheduler cannot {action} task `{task_id}` because lease `{lease_id}` for worker `{worker_id}` is stale"
    )]
    StaleLease {
        action: &'static str,
        task_id: String,
        worker_id: String,
        lease_id: String,
    },
    #[error(
        "scheduler cannot {action} task `{task_id}` because its inflight lease is no longer active"
    )]
    InactiveLease {
        action: &'static str,
        task_id: String,
    },
}

impl SchedulerError {
    pub fn is_lease_resolution_error(&self) -> bool {
        matches!(
            self,
            Self::LeaseWorkerMismatch { .. }
                | Self::LeaseOwnershipConflict { .. }
                | Self::StaleLease { .. }
                | Self::InactiveLease { .. }
        )
    }
}

impl From<String> for SchedulerError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for SchedulerError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SpiderError {
    #[error("request build error: {0}")]
    RequestBuild(String),
    #[error("download error: {0}")]
    Download(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("rules error: {0}")]
    Rules(String),
    #[error("plugin error: {0}")]
    Plugin(String),
    #[error("scheduler error: {0}")]
    Scheduler(SchedulerError),
    #[error("engine error: {0}")]
    Engine(String),
}

impl SpiderError {
    pub fn request_build(message: impl Into<String>) -> Self {
        Self::RequestBuild(message.into())
    }

    pub fn download(message: impl Into<String>) -> Self {
        Self::Download(message.into())
    }

    pub fn parse(message: impl Into<String>) -> Self {
        Self::Parse(message.into())
    }

    pub fn rules(message: impl Into<String>) -> Self {
        Self::Rules(message.into())
    }

    pub fn plugin(message: impl Into<String>) -> Self {
        Self::Plugin(message.into())
    }

    pub fn scheduler(message: impl Into<SchedulerError>) -> Self {
        Self::Scheduler(message.into())
    }

    pub fn engine(message: impl Into<String>) -> Self {
        Self::Engine(message.into())
    }

    pub fn scheduler_error(&self) -> Option<&SchedulerError> {
        match self {
            Self::Scheduler(error) => Some(error),
            _ => None,
        }
    }

    pub fn is_scheduler_lease_resolution_error(&self) -> bool {
        self.scheduler_error()
            .is_some_and(SchedulerError::is_lease_resolution_error)
    }
}
