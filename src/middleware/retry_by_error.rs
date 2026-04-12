use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::RETRY_BY_ERROR;
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct RetryByError {
    count: u64,
    backoff: Vec<u64>,
}

impl RetryByError {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            count: parse_count(options).unwrap_or(1),
            backoff: parse_backoff(options),
        }
    }

    fn override_options<'a>(
        &self,
        context: &'a context::Download,
    ) -> Option<&'a BTreeMap<String, Value>> {
        context.request.middleware_options(RETRY_BY_ERROR)
    }

    fn effective_count(&self, context: &context::Download) -> u64 {
        self.override_options(context)
            .and_then(parse_count)
            .unwrap_or(self.count)
    }

    fn effective_backoff(&self, context: &context::Download) -> Vec<u64> {
        self.override_options(context)
            .map(parse_backoff)
            .unwrap_or_else(|| self.backoff.clone())
    }

    fn should_retry(&self, context: &context::Download) -> bool {
        retry_times(context) < self.effective_count(context)
    }

    fn backoff(&self, context: &context::Download) -> Option<u64> {
        let index = retry_times(context) as usize;
        let backoff = self.effective_backoff(context);
        backoff
            .get(index)
            .copied()
            .or_else(|| backoff.last().copied())
    }
}

impl Middleware for RetryByError {
    async fn download_error(
        &self,
        context: &mut context::Download,
        error: &SpiderError,
    ) -> Result<flow::Download, SpiderError> {
        if context.request.middleware_skips(RETRY_BY_ERROR) {
            return Ok(flow::Download::Continue);
        }

        if !self.should_retry(context) {
            return Ok(flow::Download::Continue);
        }

        let _ = error;
        Ok(flow::Download::Retry {
            reason: RETRY_BY_ERROR.to_string(),
            backoff: self.backoff(context),
        })
    }
}

fn retry_times(context: &context::Download) -> u64 {
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
        .and_then(|value| first_numeric(value))
        .and_then(Value::as_f64)
        .map(|value| value as u64)
}

fn parse_backoff(options: &BTreeMap<String, Value>) -> Vec<u64> {
    options
        .get("backoff")
        .map(values_to_numbers)
        .unwrap_or_default()
        .into_iter()
        .map(|value| value as u64)
        .collect()
}

fn first_numeric(value: &Value) -> Option<&Value> {
    value
        .as_array()
        .and_then(|values| values.first())
        .or(Some(value))
}

fn values_to_numbers(value: &Value) -> Vec<f64> {
    if let Some(values) = value.as_array() {
        values.iter().filter_map(Value::as_f64).collect()
    } else {
        value.as_f64().into_iter().collect()
    }
}
