use crate::request::Request;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub request: Request,
    pub ready_at_ms: Option<u64>,
}

impl ScheduledTask {
    pub fn new(request: Request) -> Self {
        Self {
            request,
            ready_at_ms: None,
        }
    }

    pub fn with_delay_ms(request: Request, delay_ms: u64) -> Self {
        Self {
            request,
            ready_at_ms: Some(now_ms().saturating_add(delay_ms)),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready_at_ms.map(|value| value <= now_ms()).unwrap_or(true)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
