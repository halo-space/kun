use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::CONCURRENCY;
use crate::middleware::bucket::{BucketConfig, BucketKey, options_signature};
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const SLOT_KEY: &str = "_concurrency_slot";

#[derive(Default)]
struct ConcurrencyState {
    inflight: usize,
}

type SharedStore = Arc<Mutex<BTreeMap<BucketKey, ConcurrencyState>>>;

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
            .map_err(|_| SpiderError::engine("concurrency shared registry poisoned"))?;
        Ok(stores
            .entry(signature)
            .or_insert_with(|| Arc::new(Mutex::new(BTreeMap::new())))
            .clone())
    }
}

pub struct Concurrency {
    limit: usize,
    bucket: BucketConfig,
    states: SharedStore,
}

impl Concurrency {
    pub(crate) fn new(
        options: &BTreeMap<String, Value>,
        shared: &SharedRegistry,
    ) -> Result<Self, SpiderError> {
        Ok(Self {
            limit: options
                .get("concurrency")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as usize,
            bucket: BucketConfig::from_options(options, CONCURRENCY)?,
            states: shared.store(options)?,
        })
    }

    fn release_slot(&self, context: &mut context::Download) -> Result<(), SpiderError> {
        let acquired = context
            .request
            .meta
            .remove(SLOT_KEY)
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        if !acquired {
            return Ok(());
        }

        let bucket_key = self
            .effective_bucket(context)?
            .resolve(context.spider_name.as_deref(), &context.request);
        let mut states = self
            .states
            .lock()
            .map_err(|_| SpiderError::engine("concurrency state poisoned"))?;
        if let Some(state) = states.get_mut(&bucket_key) {
            state.inflight = state.inflight.saturating_sub(1);
        }
        Ok(())
    }

    fn effective_limit(&self, context: &context::Download) -> usize {
        context
            .request
            .middleware_options(CONCURRENCY)
            .and_then(|options| options.get("concurrency"))
            .and_then(Value::as_f64)
            .unwrap_or(self.limit as f64) as usize
    }

    fn effective_bucket(&self, context: &context::Download) -> Result<BucketConfig, SpiderError> {
        match context.request.middleware_options(CONCURRENCY) {
            Some(options) => BucketConfig::from_options(options, CONCURRENCY),
            None => Ok(self.bucket.clone()),
        }
    }
}

impl Middleware for Concurrency {
    async fn before_download(
        &self,
        context: &mut context::Download,
    ) -> Result<flow::Download, SpiderError> {
        if context.request.middleware_skips(CONCURRENCY) {
            return Ok(flow::Download::Continue);
        }

        let limit = self.effective_limit(context);
        if limit == 0 {
            return Ok(flow::Download::Continue);
        }

        let bucket_key = self
            .effective_bucket(context)?
            .resolve(context.spider_name.as_deref(), &context.request);
        let mut states = self
            .states
            .lock()
            .map_err(|_| SpiderError::engine("concurrency state poisoned"))?;
        let state = states.entry(bucket_key).or_default();

        if state.inflight >= limit {
            return Ok(flow::Download::Delay {
                reason: CONCURRENCY.to_string(),
                millis: 1,
            });
        }

        state.inflight += 1;
        context
            .request
            .meta
            .insert(SLOT_KEY.to_string(), Value::Bool(true));
        Ok(flow::Download::Continue)
    }

    async fn after_download(
        &self,
        context: &mut context::Download,
        _response: &mut crate::response::Response,
    ) -> Result<flow::Download, SpiderError> {
        self.release_slot(context)?;
        Ok(flow::Download::Continue)
    }

    async fn download_error(
        &self,
        context: &mut context::Download,
        _error: &SpiderError,
    ) -> Result<flow::Download, SpiderError> {
        self.release_slot(context)?;
        Ok(flow::Download::Continue)
    }
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
    fn concurrency_retries_when_limit_is_reached_and_recovers_after_release() {
        let middleware = Concurrency::new(
            &[("concurrency".to_string(), Value::Number(1.0))]
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
            flow::Download::Delay {
                reason,
                millis: 1,
            } if reason == CONCURRENCY
        ));

        let mut response = crate::response::Response::from_request(
            first.request.clone(),
            200,
            crate::request::Headers::new(),
            Vec::new(),
        );
        block_on(middleware.after_download(&mut first, &mut response)).unwrap();
        let third_flow = block_on(middleware.before_download(&mut second)).unwrap();
        assert!(matches!(third_flow, flow::Download::Continue));
    }

    #[test]
    fn concurrency_shares_state_across_instances_with_same_options() {
        let shared = SharedRegistry::default();
        let options = [("concurrency".to_string(), Value::Number(1.0))]
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let first_middleware = Concurrency::new(&options, &shared).unwrap();
        let second_middleware = Concurrency::new(&options, &shared).unwrap();

        let mut first = context::Download::new(Request::new("https://example.com/1"));
        let mut second = context::Download::new(Request::new("https://example.com/2"));

        let first_flow = block_on(first_middleware.before_download(&mut first)).unwrap();
        let second_flow = block_on(second_middleware.before_download(&mut second)).unwrap();

        assert!(matches!(first_flow, flow::Download::Continue));
        assert!(matches!(
            second_flow,
            flow::Download::Delay {
                reason,
                millis: 1,
            } if reason == CONCURRENCY
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
