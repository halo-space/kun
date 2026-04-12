use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::AUTO_THROTTLE;
use crate::middleware::bucket::{BucketConfig, BucketKey, options_signature};
use crate::middleware::traits::Middleware;
use crate::value::Value;
use jiff::Timestamp;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
struct AutoThrottleState {
    delay: u64,
    next_allowed_at: u64,
}

type SharedStore = Arc<Mutex<BTreeMap<BucketKey, AutoThrottleState>>>;

#[derive(Default, Clone)]
pub(crate) struct SharedRegistry {
    stores: Arc<Mutex<BTreeMap<String, SharedStore>>>,
}

impl SharedRegistry {
    fn store(&self, options: &BTreeMap<String, Value>) -> Result<SharedStore, SpiderError> {
        let signature = options_signature(options)?;
        let mut stores = self
            .stores
            .lock()
            .map_err(|_| SpiderError::engine("auto throttle shared registry poisoned"))?;
        Ok(stores
            .entry(signature)
            .or_insert_with(|| Arc::new(Mutex::new(BTreeMap::new())))
            .clone())
    }
}

impl AutoThrottleState {
    fn with_delay(delay: u64) -> Self {
        Self {
            delay,
            next_allowed_at: 0,
        }
    }
}

pub struct AutoThrottle {
    enabled: bool,
    target_concurrency: f64,
    start_interval: u64,
    min_interval: u64,
    max_interval: u64,
    error_backoff_ratio: f64,
    bucket: BucketConfig,
    states: SharedStore,
}

impl Default for AutoThrottle {
    fn default() -> Self {
        Self {
            enabled: false,
            target_concurrency: 1.0,
            start_interval: 0,
            min_interval: 0,
            max_interval: 60_000,
            error_backoff_ratio: 2.0,
            bucket: BucketConfig::from_options(&BTreeMap::new(), AUTO_THROTTLE)
                .expect("default auto throttle bucket should be valid"),
            states: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}

impl AutoThrottle {
    pub(crate) fn new(
        options: &BTreeMap<String, Value>,
        shared: &SharedRegistry,
    ) -> Result<Self, SpiderError> {
        let mut throttle = Self::default();
        throttle.enabled = !options.is_empty();
        throttle.bucket = BucketConfig::from_options(options, AUTO_THROTTLE)?;
        throttle.states = shared.store(options)?;

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

        Ok(throttle)
    }

    fn override_options<'a>(
        &self,
        context: &'a context::Download,
    ) -> Option<&'a BTreeMap<String, Value>> {
        context.request.middleware_options(AUTO_THROTTLE)
    }

    fn effective_enabled(&self, context: &context::Download) -> bool {
        self.override_options(context).is_some() || self.enabled
    }

    fn effective_target_concurrency(&self, context: &context::Download) -> f64 {
        self.override_options(context)
            .and_then(|options| options.get("target_concurrency"))
            .and_then(Value::as_f64)
            .map(|value| sanitize_positive_f64(value, self.target_concurrency))
            .unwrap_or(self.target_concurrency)
    }

    fn effective_start_interval(&self, context: &context::Download) -> u64 {
        self.override_options(context)
            .and_then(|options| options.get("start_interval"))
            .and_then(Value::as_f64)
            .map(sanitize_non_negative)
            .unwrap_or(self.start_interval)
    }

    fn effective_min_interval(&self, context: &context::Download) -> u64 {
        self.override_options(context)
            .and_then(|options| options.get("min_interval"))
            .and_then(Value::as_f64)
            .map(sanitize_non_negative)
            .unwrap_or(self.min_interval)
    }

    fn effective_max_interval(&self, context: &context::Download) -> u64 {
        self.override_options(context)
            .and_then(|options| options.get("max_interval"))
            .and_then(Value::as_f64)
            .map(sanitize_non_negative)
            .unwrap_or(self.max_interval)
    }

    fn effective_error_backoff_ratio(&self, context: &context::Download) -> f64 {
        self.override_options(context)
            .and_then(|options| options.get("error_backoff_ratio"))
            .and_then(Value::as_f64)
            .map(|value| sanitize_positive_f64(value, self.error_backoff_ratio))
            .unwrap_or(self.error_backoff_ratio)
    }

    fn effective_bucket(&self, context: &context::Download) -> Result<BucketConfig, SpiderError> {
        match self.override_options(context) {
            Some(options) => BucketConfig::from_options(options, AUTO_THROTTLE),
            None => Ok(self.bucket.clone()),
        }
    }

    fn current_delay(&self, state: &AutoThrottleState, start_interval: u64) -> u64 {
        if state.delay == 0 {
            start_interval
        } else {
            state.delay
        }
    }

    fn set_delay(
        &self,
        state: &mut AutoThrottleState,
        delay: u64,
        min_interval: u64,
        max_interval: u64,
        now: u64,
    ) {
        state.delay = delay.clamp(min_interval, max_interval);
        state.next_allowed_at = state.next_allowed_at.max(now.saturating_add(state.delay));
    }

    fn success_delay(
        &self,
        current_delay: u64,
        latency: u64,
        target_concurrency: f64,
        min_interval: u64,
        max_interval: u64,
    ) -> u64 {
        let target_delay = ((latency as f64) / target_concurrency).round().max(0.0) as u64;
        let clamped_target = target_delay.clamp(min_interval, max_interval);

        if current_delay == 0 {
            clamped_target
        } else {
            ((current_delay + clamped_target) / 2).clamp(min_interval, max_interval)
        }
    }

    fn error_delay(
        &self,
        current_delay: u64,
        min_interval: u64,
        max_interval: u64,
        error_backoff_ratio: f64,
    ) -> u64 {
        let baseline = current_delay.max(min_interval).max(1);
        let grown = (baseline as f64 * error_backoff_ratio).round();
        (grown.max(1.0) as u64).clamp(min_interval, max_interval)
    }

    fn finish_request(
        &self,
        context: &context::Download,
        delay_builder: impl FnOnce(u64, u64, f64, u64, u64) -> u64,
    ) -> Result<(), SpiderError> {
        if !self.effective_enabled(context) {
            return Ok(());
        }

        let Some(started_at) = context.request_started_at else {
            return Ok(());
        };

        let now = now();
        let latency = now.saturating_sub(started_at);
        let start_interval = self.effective_start_interval(context);
        let min_interval = self.effective_min_interval(context);
        let max_interval = self.effective_max_interval(context).max(min_interval);
        let target_concurrency = self.effective_target_concurrency(context);
        let error_backoff_ratio = self.effective_error_backoff_ratio(context);
        let bucket_key = self
            .effective_bucket(context)?
            .resolve(context.spider_name.as_deref(), &context.request);
        let mut states = self
            .states
            .lock()
            .map_err(|_| SpiderError::engine("auto throttle state poisoned"))?;
        let state = states
            .entry(bucket_key)
            .or_insert_with(|| AutoThrottleState::with_delay(start_interval));
        let current_delay = self.current_delay(state, start_interval);
        let next_delay = delay_builder(
            current_delay,
            latency,
            error_backoff_ratio,
            min_interval,
            max_interval,
        );
        let _ = target_concurrency;
        self.set_delay(state, next_delay, min_interval, max_interval, now);
        Ok(())
    }
}

impl Middleware for AutoThrottle {
    async fn before_download(
        &self,
        context: &mut context::Download,
    ) -> Result<flow::Download, SpiderError> {
        if context.request.middleware_skips(AUTO_THROTTLE) {
            return Ok(flow::Download::Continue);
        }

        if !self.effective_enabled(context) {
            return Ok(flow::Download::Continue);
        }

        let now = now();
        let start_interval = self.effective_start_interval(context);
        let bucket_key = self
            .effective_bucket(context)?
            .resolve(context.spider_name.as_deref(), &context.request);
        let mut states = self
            .states
            .lock()
            .map_err(|_| SpiderError::engine("auto throttle state poisoned"))?;
        let state = states
            .entry(bucket_key)
            .or_insert_with(|| AutoThrottleState::with_delay(start_interval));
        let current_delay = self.current_delay(state, start_interval);

        if state.next_allowed_at > now {
            return Ok(flow::Download::Delay {
                reason: AUTO_THROTTLE.to_string(),
                millis: state.next_allowed_at.saturating_sub(now).max(1),
            });
        }

        state.next_allowed_at = now.saturating_add(current_delay);
        context.request_started_at = Some(now);

        Ok(flow::Download::Continue)
    }

    async fn after_download(
        &self,
        context: &mut context::Download,
        response: &mut crate::response::Response,
    ) -> Result<flow::Download, SpiderError> {
        let status = Some(response.status);

        self.finish_request(
            context,
            |current_delay, latency, error_backoff_ratio, min_interval, max_interval| {
                if status == Some(429) || status.map(|code| code >= 500).unwrap_or(false) {
                    self.error_delay(
                        current_delay,
                        min_interval,
                        max_interval,
                        error_backoff_ratio,
                    )
                } else {
                    self.success_delay(
                        current_delay,
                        latency,
                        self.effective_target_concurrency(context),
                        min_interval,
                        max_interval,
                    )
                }
            },
        )?;

        Ok(flow::Download::Continue)
    }

    async fn download_error(
        &self,
        context: &mut context::Download,
        _error: &SpiderError,
    ) -> Result<flow::Download, SpiderError> {
        self.finish_request(
            context,
            |current_delay, _latency, error_backoff_ratio, min_interval, max_interval| {
                self.error_delay(
                    current_delay,
                    min_interval,
                    max_interval,
                    error_backoff_ratio,
                )
            },
        )?;

        Ok(flow::Download::Continue)
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
            &SharedRegistry::default(),
        )
        .unwrap();
        let mut first = context::Download::new(Request::new("https://example.com/1"));

        let flow = block_on(middleware.before_download(&mut first)).unwrap();
        assert!(matches!(flow, flow::Download::Continue));

        std::thread::sleep(std::time::Duration::try_from(SignedDuration::from_millis(8)).unwrap());
        let mut response =
            Response::from_request(first.request.clone(), 200, Default::default(), Vec::new());
        block_on(middleware.after_download(&mut first, &mut response)).unwrap();

        let mut second = context::Download::new(Request::new("https://example.com/2"));
        let second_flow = block_on(middleware.before_download(&mut second)).unwrap();

        assert!(matches!(
            second_flow,
            flow::Download::Delay {
                millis: ms,
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
            &SharedRegistry::default(),
        )
        .unwrap();
        let mut first = context::Download::new(Request::new("https://example.com/1"));

        let flow = block_on(middleware.before_download(&mut first)).unwrap();
        assert!(matches!(flow, flow::Download::Continue));
        block_on(middleware.download_error(&mut first, &SpiderError::download("boom"))).unwrap();

        let mut second = context::Download::new(Request::new("https://example.com/2"));
        let second_flow = block_on(middleware.before_download(&mut second)).unwrap();

        assert!(matches!(
            second_flow,
            flow::Download::Delay {
                millis: ms,
                ..
            } if ms >= 5
        ));
    }

    #[test]
    fn auto_throttle_keeps_delay_state_per_origin() {
        let middleware = AutoThrottle::new(
            &[
                ("target_concurrency".to_string(), Value::Number(1.0)),
                ("start_interval".to_string(), Value::Number(5.0)),
                ("max_interval".to_string(), Value::Number(1_000.0)),
            ]
            .into_iter()
            .collect(),
            &SharedRegistry::default(),
        )
        .unwrap();
        let mut first = context::Download::new(Request::new("https://example.com/1"));
        let mut second_same_origin = context::Download::new(Request::new("https://example.com/2"));
        let mut other_origin = context::Download::new(Request::new("https://other.example.com/1"));

        let first_flow = block_on(middleware.before_download(&mut first)).unwrap();
        let second_flow = block_on(middleware.before_download(&mut second_same_origin)).unwrap();
        let other_flow = block_on(middleware.before_download(&mut other_origin)).unwrap();

        assert!(matches!(first_flow, flow::Download::Continue));
        assert!(matches!(
            second_flow,
            flow::Download::Delay {
                reason,
                millis: ms,
            } if reason == AUTO_THROTTLE && ms > 0
        ));
        assert!(matches!(other_flow, flow::Download::Continue));
    }

    #[test]
    fn auto_throttle_shares_state_across_instances_with_same_options() {
        let shared = SharedRegistry::default();
        let options = [
            ("start_interval".to_string(), Value::Number(5.0)),
            ("max_interval".to_string(), Value::Number(1_000.0)),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>();
        let first_middleware = AutoThrottle::new(&options, &shared).unwrap();
        let second_middleware = AutoThrottle::new(&options, &shared).unwrap();

        let mut first = context::Download::new(Request::new("https://example.com/1"));
        let mut second = context::Download::new(Request::new("https://example.com/2"));

        let first_flow = block_on(first_middleware.before_download(&mut first)).unwrap();
        let second_flow = block_on(second_middleware.before_download(&mut second)).unwrap();

        assert!(matches!(first_flow, flow::Download::Continue));
        assert!(matches!(
            second_flow,
            flow::Download::Delay {
                reason,
                millis: ms,
            } if reason == AUTO_THROTTLE && ms > 0
        ));
    }

    #[test]
    fn auto_throttle_is_dormant_without_base_or_request_config() {
        let middleware = AutoThrottle::new(&BTreeMap::new(), &SharedRegistry::default()).unwrap();
        let mut first = context::Download::new(Request::new("https://example.com/1"));
        let mut second = context::Download::new(Request::new("https://example.com/2"));

        let first_flow = block_on(middleware.before_download(&mut first)).unwrap();
        let second_flow = block_on(middleware.before_download(&mut second)).unwrap();

        assert!(matches!(first_flow, flow::Download::Continue));
        assert!(matches!(second_flow, flow::Download::Continue));
    }

    #[test]
    fn request_override_can_enable_auto_throttle_without_base_config() {
        let middleware = AutoThrottle::new(&BTreeMap::new(), &SharedRegistry::default()).unwrap();
        let mut first = context::Download::new(
            Request::new("https://example.com/1").with_middleware_options(
                AUTO_THROTTLE,
                BTreeMap::from([
                    ("start_interval".to_string(), Value::Number(5.0)),
                    ("max_interval".to_string(), Value::Number(1_000.0)),
                ]),
            ),
        );
        let mut second = context::Download::new(
            Request::new("https://example.com/2").with_middleware_options(
                AUTO_THROTTLE,
                BTreeMap::from([
                    ("start_interval".to_string(), Value::Number(5.0)),
                    ("max_interval".to_string(), Value::Number(1_000.0)),
                ]),
            ),
        );

        let first_flow = block_on(middleware.before_download(&mut first)).unwrap();
        let second_flow = block_on(middleware.before_download(&mut second)).unwrap();

        assert!(matches!(first_flow, flow::Download::Continue));
        assert!(matches!(
            second_flow,
            flow::Download::Delay {
                reason,
                millis: ms,
            } if reason == AUTO_THROTTLE && ms > 0
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
