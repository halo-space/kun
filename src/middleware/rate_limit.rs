use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::RATE_LIMIT;
use crate::middleware::bucket::{BucketConfig, BucketKey, options_signature};
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
struct RateLimitState {
    hits: VecDeque<u64>,
}

type SharedStore = Arc<Mutex<BTreeMap<BucketKey, RateLimitState>>>;

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
            .map_err(|_| SpiderError::engine("rate limit shared registry poisoned"))?;
        Ok(stores
            .entry(signature)
            .or_insert_with(|| Arc::new(Mutex::new(BTreeMap::new())))
            .clone())
    }
}

pub struct RateLimit {
    rate_per_minute: usize,
    bucket: BucketConfig,
    states: SharedStore,
}

impl RateLimit {
    pub(crate) fn new(
        options: &BTreeMap<String, Value>,
        shared: &SharedRegistry,
    ) -> Result<Self, SpiderError> {
        Ok(Self {
            rate_per_minute: options
                .get("rate_per_minute")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as usize,
            bucket: BucketConfig::from_options(options, RATE_LIMIT)?,
            states: shared.store(options)?,
        })
    }

    fn effective_rate(&self, context: &context::Download) -> usize {
        context
            .request
            .middleware_options(RATE_LIMIT)
            .and_then(|options| options.get("rate_per_minute"))
            .and_then(Value::as_f64)
            .unwrap_or(self.rate_per_minute as f64) as usize
    }

    fn effective_bucket(&self, context: &context::Download) -> Result<BucketConfig, SpiderError> {
        match context.request.middleware_options(RATE_LIMIT) {
            Some(options) => BucketConfig::from_options(options, RATE_LIMIT),
            None => Ok(self.bucket.clone()),
        }
    }
}

impl Middleware for RateLimit {
    async fn before_download(
        &self,
        context: &mut context::Download,
    ) -> Result<flow::Download, SpiderError> {
        if context.request.middleware_skips(RATE_LIMIT) {
            return Ok(flow::Download::Continue);
        }

        let rate_per_minute = self.effective_rate(context);
        if rate_per_minute == 0 {
            return Ok(flow::Download::Continue);
        }

        let now = now();
        let window_start = now.saturating_sub(60_000);
        let bucket_key = self
            .effective_bucket(context)?
            .resolve(context.spider_name.as_deref(), &context.request);
        let mut states = self
            .states
            .lock()
            .map_err(|_| SpiderError::engine("rate limit state poisoned"))?;
        let state = states.entry(bucket_key).or_default();

        while state
            .hits
            .front()
            .copied()
            .map(|value| value < window_start)
            .unwrap_or(false)
        {
            state.hits.pop_front();
        }

        if state.hits.len() >= rate_per_minute {
            let oldest = state.hits.front().copied().unwrap_or(now);
            let backoff = oldest.saturating_add(60_000).saturating_sub(now);
            return Ok(flow::Download::Delay {
                reason: RATE_LIMIT.to_string(),
                millis: backoff.max(1),
            });
        }

        state.hits.push_back(now);
        Ok(flow::Download::Continue)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn rate_limit_returns_retry_when_window_is_full() {
        let middleware = RateLimit::new(
            &[("rate_per_minute".to_string(), Value::Number(1.0))]
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
            &SharedRegistry::default(),
        )
        .unwrap();
        let mut first = context::Download::new(Request::new("https://example.com/1"));
        let mut second = context::Download::new(Request::new("https://example.com/2"));

        let first_flow = block_on(middleware.before_download(&mut first)).unwrap();
        let second_flow = block_on(middleware.before_download(&mut second)).unwrap();

        assert!(matches!(first_flow, flow::Download::Continue));
        assert!(matches!(
            second_flow,
            flow::Download::Delay { millis: _, .. }
        ));
    }

    #[test]
    fn rate_limit_shares_state_across_instances_with_same_options() {
        let shared = SharedRegistry::default();
        let options = [("rate_per_minute".to_string(), Value::Number(1.0))]
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let first_middleware = RateLimit::new(&options, &shared).unwrap();
        let second_middleware = RateLimit::new(&options, &shared).unwrap();

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
            } if reason == RATE_LIMIT && ms > 0
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
