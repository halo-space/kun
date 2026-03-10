use crate::engine::context::EngineContext;
use crate::engine::types::Flow;
use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct RetryByStatusMiddleware {
    count: u64,
    statuses: Vec<u16>,
    backoff_ms: Vec<u64>,
}

impl RetryByStatusMiddleware {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            count: parse_count(options).unwrap_or(1),
            statuses: options
                .get("status")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_f64)
                        .map(|value| value as u16)
                        .collect()
                })
                .unwrap_or_default(),
            backoff_ms: parse_backoff(options),
        }
    }

    fn should_retry(&self, context: &EngineContext) -> bool {
        let status = context.response.as_ref().map(|response| response.status);
        let retried = retry_times(context);

        status
            .map(|status| self.statuses.contains(&status) && retried < self.count)
            .unwrap_or(false)
    }

    fn backoff(&self, context: &EngineContext) -> Option<u64> {
        let index = retry_times(context) as usize;
        self.backoff_ms
            .get(index)
            .copied()
            .or_else(|| self.backoff_ms.last().copied())
    }
}

impl Middleware for RetryByStatusMiddleware {
    fn process_response<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            if !self.should_retry(context) {
                return Ok(Flow::Continue);
            }

            let status = context.response.as_ref().map(|response| response.status).unwrap_or(0);
            Ok(Flow::Retry {
                reason: format!("retry by status: {status}"),
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
