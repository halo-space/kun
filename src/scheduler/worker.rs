use jiff::SignedDuration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worker {
    worker_id: String,
    lease_timeout_override: Option<Option<SignedDuration>>,
    heartbeat_interval_override: Option<SignedDuration>,
}

impl Worker {
    pub fn new(worker_id: impl Into<String>) -> Self {
        Self {
            worker_id: worker_id.into(),
            lease_timeout_override: None,
            heartbeat_interval_override: None,
        }
    }

    pub fn worker_id(&self) -> &str {
        self.worker_id.as_str()
    }

    pub fn with_lease_timeout(mut self, timeout: SignedDuration) -> Self {
        self.lease_timeout_override = Some(Some(timeout));
        self
    }

    pub fn with_heartbeat_interval(mut self, interval: SignedDuration) -> Self {
        self.heartbeat_interval_override = Some(interval);
        self
    }

    pub fn without_lease_timeout(mut self) -> Self {
        self.lease_timeout_override = Some(None);
        self.heartbeat_interval_override = None;
        self
    }

    pub(crate) fn effective_lease_timeout(
        &self,
        default: Option<SignedDuration>,
    ) -> Option<SignedDuration> {
        match &self.lease_timeout_override {
            None => default,
            Some(Some(timeout)) => Some(timeout.clone()),
            Some(None) => None,
        }
    }

    pub(crate) fn effective_heartbeat_interval(
        &self,
        default: Option<SignedDuration>,
    ) -> Option<SignedDuration> {
        self.heartbeat_interval_override.clone().or(default)
    }
}
