use crate::engine::context::EngineContext;
use crate::engine::types::Flow;
use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct RetryByErrorMiddleware {
    count: u64,
    backoff_ms: Vec<u64>,
}

impl RetryByErrorMiddleware {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            count: parse_count(options).unwrap_or(1),
            backoff_ms: parse_backoff(options),
        }
    }

    fn should_retry(&self, context: &EngineContext) -> bool {
        retry_times(context) < self.count
    }

    fn backoff(&self, context: &EngineContext) -> Option<u64> {
        let index = retry_times(context) as usize;
        self.backoff_ms
            .get(index)
            .copied()
            .or_else(|| self.backoff_ms.last().copied())
    }
}

impl Middleware for RetryByErrorMiddleware {
    fn process_exception<'a>(
        &'a self,
        context: &'a mut EngineContext,
        error: &'a SpiderError,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            if !self.should_retry(context) {
                return Ok(Flow::Continue);
            }

            Ok(Flow::Retry {
                reason: format!("retry by error: {error}"),
                backoff_ms: self.backoff(context),
            })
        })
    }
}

fn retry_times(context: &EngineContext) -> u64 {
    context
        .request
        .meta
        .get("_retry_times")
        .and_then(Value::as_f64)
        .unwrap_or(0.0) as u64
}

fn parse_count(options: &BTreeMap<String, Value>) -> Option<u64> {
    options
        .get("count")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .and_then(Value::as_f64)
        .map(|value| value as u64)
}

fn parse_backoff(options: &BTreeMap<String, Value>) -> Vec<u64> {
    options
        .get("backoff_ms")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_f64)
                .map(|value| value as u64)
                .collect()
        })
        .unwrap_or_default()
}
