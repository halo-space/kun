use crate::engine::context::EngineContext;
use crate::engine::types::Flow;
use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct RateLimitMiddleware {
    rate_per_minute: usize,
    hits_ms: Mutex<VecDeque<u64>>,
}

impl RateLimitMiddleware {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            rate_per_minute: options
                .get("rate_per_minute")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as usize,
            hits_ms: Mutex::new(VecDeque::new()),
        }
    }
}

impl Middleware for RateLimitMiddleware {
    fn process_request<'a>(
        &'a self,
        _context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            if self.rate_per_minute == 0 {
                return Ok(Flow::Continue);
            }

            let now = now_ms();
            let window_start = now.saturating_sub(60_000);
            let mut hits = self
                .hits_ms
                .lock()
                .map_err(|_| SpiderError::engine("rate limit state poisoned"))?;

            while hits.front().copied().map(|value| value < window_start).unwrap_or(false) {
                hits.pop_front();
            }

            if hits.len() >= self.rate_per_minute {
                let oldest = hits.front().copied().unwrap_or(now);
                let backoff = oldest.saturating_add(60_000).saturating_sub(now);
                return Ok(Flow::Retry {
                    reason: "rate limit".to_string(),
                    backoff_ms: Some(backoff.max(1)),
                });
            }

            hits.push_back(now);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::context::EngineContext;
    use crate::request::Request;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn rate_limit_returns_retry_when_window_is_full() {
        let middleware = RateLimitMiddleware::new(
            &[("rate_per_minute".to_string(), Value::Number(1.0))]
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
        );
        let mut first = EngineContext::new(Request::new("https://example.com/1"));
        let mut second = EngineContext::new(Request::new("https://example.com/2"));

        let first_flow = block_on(middleware.process_request(&mut first)).unwrap();
        let second_flow = block_on(middleware.process_request(&mut second)).unwrap();

        assert!(matches!(first_flow, Flow::Continue));
        assert!(matches!(
            second_flow,
            Flow::Retry {
                backoff_ms: Some(_),
                ..
            }
        ));
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
