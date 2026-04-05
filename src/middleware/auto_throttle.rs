use crate::engine::context::EngineContext;
use crate::engine::flow::Flow;
use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::middleware::traits::Middleware;
use crate::value::Value;
use jiff::Timestamp;
use std::collections::BTreeMap;
use std::sync::Mutex;
use url::Url;

#[derive(Debug, Clone)]
struct OriginState {
    delay: u64,
    next_allowed_at: u64,
    inflight: usize,
}

impl OriginState {
    fn with_delay(delay: u64) -> Self {
        Self {
            delay,
            next_allowed_at: 0,
            inflight: 0,
        }
    }
}

pub struct AutoThrottle {
    target_concurrency: f64,
    start_interval: u64,
    min_interval: u64,
    max_interval: u64,
    error_backoff_ratio: f64,
    states: Mutex<BTreeMap<String, OriginState>>,
}

impl Default for AutoThrottle {
    fn default() -> Self {
        Self {
            target_concurrency: 1.0,
            start_interval: 0,
            min_interval: 0,
            max_interval: 60_000,
            error_backoff_ratio: 2.0,
            states: Mutex::new(BTreeMap::new()),
        }
    }
}

impl AutoThrottle {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        let mut throttle = Self::default();

        if let Some(value) = options.get("target_concurrency").and_then(Value::as_f64) {
            throttle.target_concurrency = sanitize_positive_f64(value, 1.0);
        }

        if let Some(value) = options.get("start_interval").and_then(Value::as_f64) {
            throttle.start_interval = sanitize_non_negative(value);
        }

        if let Some(value) = options.get("min_interval").and_then(Value::as_f64) {
            throttle.min_interval = sanitize_non_negative(value);
        }

        if let Some(value) = options.get("max_interval").and_then(Value::as_f64) {
            throttle.max_interval = sanitize_non_negative(value);
        }

        if let Some(value) = options.get("error_backoff_ratio").and_then(Value::as_f64) {
            throttle.error_backoff_ratio = sanitize_positive_f64(value, 2.0);
        }

        if throttle.max_interval < throttle.min_interval {
            throttle.max_interval = throttle.min_interval;
        }

        throttle.start_interval = throttle
            .start_interval
            .clamp(throttle.min_interval, throttle.max_interval);

        throttle
    }

    fn concurrency_limit(&self) -> usize {
        self.target_concurrency.ceil().max(1.0) as usize
    }

    fn current_delay(&self, state: &OriginState) -> u64 {
        if state.delay == 0 {
            self.start_interval
        } else {
            state.delay
        }
    }

    fn set_delay(&self, state: &mut OriginState, delay: u64, now: u64) {
        state.delay = delay.clamp(self.min_interval, self.max_interval);
        state.next_allowed_at = state.next_allowed_at.max(now.saturating_add(state.delay));
    }

    fn success_delay(&self, current_delay: u64, latency: u64) -> u64 {
        let target_delay = ((latency as f64) / self.target_concurrency)
            .round()
            .max(0.0) as u64;
        let clamped_target = target_delay.clamp(self.min_interval, self.max_interval);

        if current_delay == 0 {
            clamped_target
        } else {
            ((current_delay + clamped_target) / 2).clamp(self.min_interval, self.max_interval)
        }
    }

    fn error_delay(&self, current_delay: u64) -> u64 {
        let baseline = current_delay.max(self.min_interval).max(1);
        let grown = (baseline as f64 * self.error_backoff_ratio).round();
        (grown.max(1.0) as u64).clamp(self.min_interval, self.max_interval)
    }

    fn finish_request(
        &self,
        context: &EngineContext,
        delay_builder: impl FnOnce(u64, u64) -> u64,
    ) -> Result<(), SpiderError> {
        let Some(origin) = context.request_origin.as_ref() else {
            return Ok(());
        };
        let Some(started_at) = context.request_started_at else {
            return Ok(());
        };

        let now = now();
        let latency = now.saturating_sub(started_at);
        let mut states = self
            .states
            .lock()
            .map_err(|_| SpiderError::engine("auto throttle state poisoned"))?;
        let state = states
            .entry(origin.clone())
            .or_insert_with(|| OriginState::with_delay(self.start_interval));
        state.inflight = state.inflight.saturating_sub(1);
        let current_delay = self.current_delay(state);
        let next_delay = delay_builder(current_delay, latency);
        self.set_delay(state, next_delay, now);
        Ok(())
    }
}

impl Middleware for AutoThrottle {
    fn process_request<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            let origin = request_origin(context.request.url.as_str());
            let now = now();
            let mut states = self
                .states
                .lock()
                .map_err(|_| SpiderError::engine("auto throttle state poisoned"))?;
            let state = states
                .entry(origin.clone())
                .or_insert_with(|| OriginState::with_delay(self.start_interval));
            let current_delay = self.current_delay(state);

            if state.inflight >= self.concurrency_limit() {
                return Ok(Flow::Retry {
                    reason: "auto throttle concurrency".to_string(),
                    backoff: Some(current_delay.max(1)),
                });
            }

            if state.next_allowed_at > now {
                return Ok(Flow::Retry {
                    reason: "auto throttle delay".to_string(),
                    backoff: Some(state.next_allowed_at.saturating_sub(now).max(1)),
                });
            }

            state.inflight += 1;
            state.next_allowed_at = now.saturating_add(current_delay);
            context.request_origin = Some(origin);
            context.request_started_at = Some(now);

            Ok(Flow::Continue)
        })
    }

    fn process_response<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            let status = context.response.as_ref().map(|response| response.status);

            self.finish_request(context, |current_delay, latency| {
                if status == Some(429) || status.map(|code| code >= 500).unwrap_or(false) {
                    self.error_delay(current_delay)
                } else {
                    self.success_delay(current_delay, latency)
                }
            })?;

            Ok(Flow::Continue)
        })
    }

    fn process_exception<'a>(
        &'a self,
        context: &'a mut EngineContext,
        _error: &'a SpiderError,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            self.finish_request(context, |current_delay, _latency| {
                self.error_delay(current_delay)
            })?;

            Ok(Flow::Continue)
        })
    }
}

fn sanitize_non_negative(value: f64) -> u64 {
    value.max(0.0).round() as u64
}

fn sanitize_positive_f64(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

fn request_origin(url: &str) -> String {
    let Ok(parsed_url) = Url::parse(url) else {
        return url.to_string();
    };
    let Some(host) = parsed_url.host_str() else {
        return url.to_string();
    };

    let mut origin = format!("{}://{}", parsed_url.scheme(), host);
    if let Some(port) = parsed_url.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    origin
}

fn now() -> u64 {
    u64::try_from(Timestamp::now().as_millisecond()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::response::Response;
    use jiff::SignedDuration;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn auto_throttle_applies_latency_feedback() {
        let middleware = AutoThrottle::new(
            &[
                ("target_concurrency".to_string(), Value::Number(1.0)),
                ("max_interval".to_string(), Value::Number(1_000.0)),
            ]
            .into_iter()
            .collect(),
        );
        let mut first = EngineContext::new(Request::new("https://example.com/1"));

        let flow = block_on(middleware.process_request(&mut first)).unwrap();
        assert!(matches!(flow, Flow::Continue));

        std::thread::sleep(std::time::Duration::try_from(SignedDuration::from_millis(8)).unwrap());
        first.response = Some(Response::from_request(
            first.request.clone(),
            200,
            Default::default(),
            Vec::new(),
        ));
        block_on(middleware.process_response(&mut first)).unwrap();

        let mut second = EngineContext::new(Request::new("https://example.com/2"));
        let second_flow = block_on(middleware.process_request(&mut second)).unwrap();

        assert!(matches!(
            second_flow,
            Flow::Retry {
                backoff: Some(ms),
                ..
            } if ms > 0
        ));
    }

    #[test]
    fn auto_throttle_applies_error_feedback() {
        let middleware = AutoThrottle::new(
            &[
                ("start_interval".to_string(), Value::Number(5.0)),
                ("max_interval".to_string(), Value::Number(1_000.0)),
            ]
            .into_iter()
            .collect(),
        );
        let mut first = EngineContext::new(Request::new("https://example.com/1"));

        let flow = block_on(middleware.process_request(&mut first)).unwrap();
        assert!(matches!(flow, Flow::Continue));
        block_on(middleware.process_exception(&mut first, &SpiderError::download("boom"))).unwrap();

        let mut second = EngineContext::new(Request::new("https://example.com/2"));
        let second_flow = block_on(middleware.process_request(&mut second)).unwrap();

        assert!(matches!(
            second_flow,
            Flow::Retry {
                backoff: Some(ms),
                ..
            } if ms >= 5
        ));
    }

    #[test]
    fn auto_throttle_applies_concurrency_feedback_per_origin() {
        let middleware = AutoThrottle::new(
            &[
                ("target_concurrency".to_string(), Value::Number(1.0)),
                ("start_interval".to_string(), Value::Number(5.0)),
                ("max_interval".to_string(), Value::Number(1_000.0)),
            ]
            .into_iter()
            .collect(),
        );
        let mut first = EngineContext::new(Request::new("https://example.com/1"));
        let mut second_same_origin = EngineContext::new(Request::new("https://example.com/2"));
        let mut other_origin = EngineContext::new(Request::new("https://other.example.com/1"));

        let first_flow = block_on(middleware.process_request(&mut first)).unwrap();
        let second_flow = block_on(middleware.process_request(&mut second_same_origin)).unwrap();
        let other_flow = block_on(middleware.process_request(&mut other_origin)).unwrap();

        assert!(matches!(first_flow, Flow::Continue));
        assert!(matches!(
            second_flow,
            Flow::Retry {
                reason,
                backoff: Some(ms),
            } if reason == "auto throttle concurrency" && ms > 0
        ));
        assert!(matches!(other_flow, Flow::Continue));
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
