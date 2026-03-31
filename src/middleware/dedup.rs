use crate::engine::context::EngineContext;
use crate::engine::flow::Flow;
use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

#[derive(Default)]
pub struct Dedup {
    keys: Vec<String>,
    scope: String,
    namespace: Option<String>,
    seen: Mutex<HashSet<String>>,
}

impl Dedup {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            keys: parse_keys(options),
            scope: options
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("TASK")
                .to_string(),
            namespace: options
                .get("namespace")
                .and_then(Value::as_str)
                .map(str::to_string),
            seen: Mutex::new(HashSet::new()),
        }
    }

    fn resolve_key(&self, context: &EngineContext) -> Option<String> {
        let mut parts = Vec::new();

        for key in &self.keys {
            let raw = if key == "url" {
                Some(context.request.url.clone())
            } else if let Some(meta_key) = key.strip_prefix("meta.") {
                context.request.meta.get(meta_key).and_then(resolve_value)
            } else {
                context.request.meta.get(key).and_then(resolve_value)
            };

            let value = raw?.trim().to_string();
            if value.is_empty() {
                return None;
            }
            parts.push(value);
        }

        let base = parts.join("|");

        Some(match self.scope.as_str() {
            "STEP" => {
                let step_id = context
                    .request
                    .meta
                    .get("next_step")
                    .and_then(Value::as_str)
                    .unwrap_or("parse");
                format!("step={step_id}|{base}")
            }
            "CUSTOM" => {
                let namespace = self.namespace.as_deref().unwrap_or("default");
                format!("ns={namespace}|{base}")
            }
            _ => base,
        })
    }
}

impl Middleware for Dedup {
    fn process_request<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            if context.request.dont_filter {
                return Ok(Flow::Continue);
            }

            let Some(key) = self.resolve_key(context) else {
                return Ok(Flow::Continue);
            };

            let mut seen = self
                .seen
                .lock()
                .map_err(|_| SpiderError::engine("dedup state poisoned"))?;

            if !seen.insert(key.clone()) {
                return Ok(Flow::Drop(format!("duplicate request: {key}")));
            }

            Ok(Flow::Continue)
        })
    }
}

fn parse_keys(options: &BTreeMap<String, Value>) -> Vec<String> {
    match options.get("key") {
        Some(Value::String(key)) => vec![key.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => vec!["url".to_string()],
    }
}

fn resolve_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
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
    fn dedup_supports_multi_key_scope_from_request_meta() {
        let middleware = Dedup::new(
            &[
                (
                    "key".to_string(),
                    Value::Array(vec![
                        Value::String("product_id".to_string()),
                        Value::String("meta.category".to_string()),
                    ]),
                ),
                ("scope".to_string(), Value::String("STEP".to_string())),
            ]
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        );

        let mut first = EngineContext::new(
            Request::new("https://example.com/detail/1")
                .with_meta("next_step", Value::String("detail".to_string()))
                .with_meta("product_id", Value::String("sku-1".to_string()))
                .with_meta("category", Value::String("news".to_string())),
        );
        let mut second = EngineContext::new(
            Request::new("https://example.com/detail/2")
                .with_meta("next_step", Value::String("detail".to_string()))
                .with_meta("product_id", Value::String("sku-1".to_string()))
                .with_meta("category", Value::String("news".to_string())),
        );

        let first_flow = block_on(middleware.process_request(&mut first)).unwrap();
        let second_flow = block_on(middleware.process_request(&mut second)).unwrap();

        assert!(matches!(first_flow, Flow::Continue));
        assert!(matches!(second_flow, Flow::Drop(_)));
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
