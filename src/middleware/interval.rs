use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::INTERVAL;
use crate::middleware::bucket::{BucketConfig, BucketKey, options_signature};
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
struct IntervalState {
    next_allowed_at: u64,
}

type SharedStore = Arc<Mutex<BTreeMap<BucketKey, IntervalState>>>;

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
            .map_err(|_| SpiderError::engine("interval shared registry poisoned"))?;
        Ok(stores
            .entry(signature)
            .or_insert_with(|| Arc::new(Mutex::new(BTreeMap::new())))
            .clone())
    }
}

pub struct Interval {
    interval: u64,
    bucket: BucketConfig,
    states: SharedStore,
}

impl Interval {
    pub(crate) fn new(
        options: &BTreeMap<String, Value>,
        shared: &SharedRegistry,
    ) -> Result<Self, SpiderError> {
        Ok(Self {
            interval: options
                .get("interval")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as u64,
            bucket: BucketConfig::from_options(options, INTERVAL)?,
            states: shared.store(options)?,
        })
    }

    fn effective_interval(&self, context: &context::Download) -> u64 {
        context
            .request
            .middleware_options(INTERVAL)
            .and_then(|options| options.get("interval"))
            .and_then(Value::as_f64)
            .unwrap_or(self.interval as f64) as u64
    }

    fn effective_bucket(&self, context: &context::Download) -> Result<BucketConfig, SpiderError> {
        match context.request.middleware_options(INTERVAL) {
            Some(options) => BucketConfig::from_options(options, INTERVAL),
            None => Ok(self.bucket.clone()),
        }
    }
}

impl Middleware for Interval {
    async fn before_download(
        &self,
        context: &mut context::Download,
    ) -> Result<flow::Download, SpiderError> {
        if context.request.middleware_skips(INTERVAL) {
            return Ok(flow::Download::Continue);
        }

        let interval = self.effective_interval(context);
        if interval == 0 {
            return Ok(flow::Download::Continue);
        }

        let now = now();
        let bucket_key = self
            .effective_bucket(context)?
            .resolve(context.spider_name.as_deref(), &context.request);
        let mut states = self
            .states
            .lock()
            .map_err(|_| SpiderError::engine("interval state poisoned"))?;
        let state = states.entry(bucket_key).or_default();

        if state.next_allowed_at > now {
            return Ok(flow::Download::Delay {
                reason: INTERVAL.to_string(),
                millis: state.next_allowed_at - now,
            });
        }

        state.next_allowed_at = now.saturating_add(interval);
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
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn interval_shares_state_across_instances_with_same_options() {
        let shared = SharedRegistry::default();
        let options = [("interval".to_string(), Value::Number(1_000.0))]
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let first_middleware = Interval::new(&options, &shared).unwrap();
        let second_middleware = Interval::new(&options, &shared).unwrap();

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
            } if reason == INTERVAL && ms > 0
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
