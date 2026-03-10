use crate::engine::context::EngineContext;
use crate::engine::types::Flow;
use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

#[derive(Default)]
pub struct DedupMiddleware {
    key: String,
    seen: Mutex<HashSet<String>>,
}

impl DedupMiddleware {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            key: options
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or("url")
                .to_string(),
            seen: Mutex::new(HashSet::new()),
        }
    }

    fn resolve_key(&self, context: &EngineContext) -> Option<String> {
        if self.key == "url" {
            return Some(context.request.url.clone());
        }

        if let Some(key) = self.key.strip_prefix("meta.") {
            return context
                .request
                .meta
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string);
        }

        None
    }
}

impl Middleware for DedupMiddleware {
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
