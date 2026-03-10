use crate::engine::context::EngineContext;
use crate::engine::types::Flow;
use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct IntervalGateMiddleware {
    interval_ms: u64,
    next_allowed_ms: Mutex<u64>,
}

impl IntervalGateMiddleware {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            interval_ms: options
                .get("interval_ms")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as u64,
            next_allowed_ms: Mutex::new(0),
        }
    }
}

impl Middleware for IntervalGateMiddleware {
    fn process_request<'a>(
        &'a self,
        _context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            if self.interval_ms == 0 {
                return Ok(Flow::Continue);
            }

            let now = now_ms();
            let mut next_allowed = self
                .next_allowed_ms
                .lock()
                .map_err(|_| SpiderError::engine("interval gate state poisoned"))?;

            if *next_allowed > now {
                return Ok(Flow::Retry {
                    reason: "interval gate".to_string(),
                    backoff_ms: Some(*next_allowed - now),
                });
            }

            *next_allowed = now.saturating_add(self.interval_ms);
            Ok(Flow::Continue)
        })
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
