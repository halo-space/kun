use crate::engine::context::EngineContext;
use crate::engine::flow::Flow;
use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

const SLOT_KEY: &str = "_concurrency_gate_slot";

#[derive(Default)]
pub struct ConcurrencyGate {
    limit: usize,
    inflight: Mutex<usize>,
}

impl ConcurrencyGate {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            limit: options
                .get("concurrency")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as usize,
            inflight: Mutex::new(0),
        }
    }

    fn release_slot(&self, context: &mut EngineContext) -> Result<(), SpiderError> {
        let acquired = context
            .request
            .meta
            .remove(SLOT_KEY)
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        if !acquired {
            return Ok(());
        }

        let mut inflight = self
            .inflight
            .lock()
            .map_err(|_| SpiderError::engine("concurrency gate state poisoned"))?;
        *inflight = inflight.saturating_sub(1);
        Ok(())
    }
}

impl Middleware for ConcurrencyGate {
    fn process_request<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            if self.limit == 0 {
                return Ok(Flow::Continue);
            }

            let mut inflight = self
                .inflight
                .lock()
                .map_err(|_| SpiderError::engine("concurrency gate state poisoned"))?;

            if *inflight >= self.limit {
                return Ok(Flow::Retry {
                    reason: "concurrency gate".to_string(),
                    backoff_ms: Some(1),
                });
            }

            *inflight += 1;
            context
                .request
                .meta
                .insert(SLOT_KEY.to_string(), Value::Bool(true));
            Ok(Flow::Continue)
        })
    }

    fn process_response<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            self.release_slot(context)?;
            Ok(Flow::Continue)
        })
    }

    fn process_exception<'a>(
        &'a self,
        context: &'a mut EngineContext,
        _error: &'a SpiderError,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            self.release_slot(context)?;
            Ok(Flow::Continue)
        })
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
    fn concurrency_gate_retries_when_limit_is_reached_and_recovers_after_release() {
        let middleware = ConcurrencyGate::new(
            &[("concurrency".to_string(), Value::Number(1.0))]
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
                reason,
                backoff_ms: Some(1),
            } if reason == "concurrency gate"
        ));

        block_on(middleware.process_response(&mut first)).unwrap();
        let third_flow = block_on(middleware.process_request(&mut second)).unwrap();
        assert!(matches!(third_flow, Flow::Continue));
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
