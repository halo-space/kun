use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::RETRY_BY_STATUS;
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct RetryByStatus {
    count: u64,
    statuses: Vec<u16>,
    backoff: Vec<u64>,
}

impl RetryByStatus {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            count: parse_count(options).unwrap_or(1),
            statuses: parse_statuses(options),
            backoff: parse_backoff(options),
        }
    }

    fn override_options<'a>(
        &self,
        context: &'a context::Download,
    ) -> Option<&'a BTreeMap<String, Value>> {
        context.request.middleware_options(RETRY_BY_STATUS)
    }

    fn effective_count(&self, context: &context::Download) -> u64 {
        self.override_options(context)
            .and_then(parse_count)
            .unwrap_or(self.count)
    }

    fn effective_statuses(&self, context: &context::Download) -> Vec<u16> {
        self.override_options(context)
            .map(parse_statuses)
            .unwrap_or_else(|| self.statuses.clone())
    }

    fn effective_backoff(&self, context: &context::Download) -> Vec<u64> {
        self.override_options(context)
            .map(parse_backoff)
            .unwrap_or_else(|| self.backoff.clone())
    }

    fn should_retry(
        &self,
        context: &context::Download,
        response: &crate::response::Response,
    ) -> bool {
        let status = Some(response.status);
        let retried = retry_times(context);
        let statuses = self.effective_statuses(context);
        let count = self.effective_count(context);

        status
            .map(|status| statuses.contains(&status) && retried < count)
            .unwrap_or(false)
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

impl Middleware for RetryByStatus {
    async fn after_download(
        &self,
        context: &mut context::Download,
        response: &mut crate::response::Response,
    ) -> Result<flow::Download, SpiderError> {
        if context.request.middleware_skips(RETRY_BY_STATUS) {
            return Ok(flow::Download::Continue);
        }

        if !self.should_retry(context, response) {
            return Ok(flow::Download::Continue);
        }

        let status = response.status;
        let _ = status;
        Ok(flow::Download::Retry {
            reason: RETRY_BY_STATUS.to_string(),
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

fn parse_statuses(options: &BTreeMap<String, Value>) -> Vec<u16> {
    options
        .get("status")
        .or_else(|| options.get("http_status"))
        .map(values_to_numbers)
        .unwrap_or_default()
        .into_iter()
        .map(|value| value as u16)
        .collect()
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
