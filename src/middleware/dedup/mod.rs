pub mod bloom;
pub mod memory;
pub mod noop;

use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::DEDUP;
use crate::middleware::traits::{BoxMiddleware, Middleware, box_middleware};
use crate::request::Request;
use crate::value::Value;
use std::collections::BTreeMap;
use std::future::Future;

pub use bloom::Bloom;
pub use memory::Memory;
pub use noop::Noop;

/// Fingerprint fields for the built-in dedup implementations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Url,
    Method,
    Body,
    Meta(String),
}

/// Request deduplication boundary for the engine.
///
/// Dedup runs before a request enters the scheduler. Custom implementations can
/// decide how request fingerprints are built and where seen-state is stored.
pub trait Dedup: Send + Sync {
    /// Returns `true` when the request should be accepted and recorded as seen.
    ///
    /// Returning `false` means the request is considered a duplicate and will
    /// not be enqueued into the scheduler.
    fn check_and_insert(
        &mut self,
        request: &Request,
    ) -> impl Future<Output = Result<bool, SpiderError>> + Send;

    /// Returns `true` when the request should be accepted and recorded as seen
    /// using an explicit per-request fingerprint policy.
    fn check_and_insert_with_keys(
        &mut self,
        request: &Request,
        _keys: &[Key],
    ) -> impl Future<Output = Result<bool, SpiderError>> + Send {
        self.check_and_insert(request)
    }
}

pub struct DedupMiddleware<D> {
    inner: tokio::sync::Mutex<D>,
}

impl<D> DedupMiddleware<D>
where
    D: Dedup,
{
    pub fn new(dedup: D) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(dedup),
        }
    }
}

impl DedupMiddleware<Memory> {
    pub fn memory() -> Self {
        Self::new(Memory::default())
    }
}

impl<D> Middleware for DedupMiddleware<D>
where
    D: Dedup + 'static,
{
    async fn before_enqueue(
        &self,
        context: &mut context::Enqueue,
    ) -> Result<flow::Enqueue, SpiderError> {
        if context.request.middleware_skips(DEDUP) {
            return Ok(flow::Enqueue::Continue);
        }

        let override_keys = request_keys_override(&context.request)?;
        let mut dedup = self.inner.lock().await;
        let accepted = match override_keys {
            Some(keys) => {
                dedup
                    .check_and_insert_with_keys(&context.request, &keys)
                    .await?
            }
            None => dedup.check_and_insert(&context.request).await?,
        };

        if accepted {
            Ok(flow::Enqueue::Continue)
        } else {
            Ok(flow::Enqueue::Drop {
                reason: DEDUP.to_string(),
            })
        }
    }
}

pub(crate) fn from_options(
    options: &BTreeMap<String, Value>,
) -> Result<BoxMiddleware, SpiderError> {
    let backend = options
        .get("backend")
        .or_else(|| options.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("memory");
    let keys = parse_keys(options)?;

    match backend {
        "memory" => Ok(box_middleware(DedupMiddleware::new(
            Memory::default().with_keys(keys),
        ))),
        "bloom" => {
            let expected_items = options
                .get("expected_items")
                .and_then(Value::as_f64)
                .map(|value| value.max(1.0) as usize)
                .unwrap_or(100_000);
            let false_positive_rate = options
                .get("false_positive_rate")
                .and_then(Value::as_f64)
                .unwrap_or(0.01);

            Ok(box_middleware(DedupMiddleware::new(
                Bloom::default()
                    .with_expected_items(expected_items)
                    .with_false_positive_rate(false_positive_rate)
                    .with_keys(keys),
            )))
        }
        "noop" => Ok(box_middleware(DedupMiddleware::new(Noop))),
        other => Err(SpiderError::engine(format!(
            "unsupported dedup backend: {other}"
        ))),
    }
}

pub(crate) fn fingerprint(request: &Request, keys: &[Key]) -> String {
    keys.iter()
        .map(|key| fingerprint_part(request, key))
        .collect::<Vec<_>>()
        .join("|")
}

fn fingerprint_part(request: &Request, key: &Key) -> String {
    match key {
        Key::Url => format!("url={}", request.url),
        Key::Method => format!("method={}", request.method),
        Key::Body => format!("body={}", encode_body(request.body.as_deref())),
        Key::Meta(name) => format!("meta.{name}={}", encode_meta(request, name)),
    }
}

fn encode_body(body: Option<&[u8]>) -> String {
    body.map(hex_encode).unwrap_or_else(|| "<none>".to_string())
}

fn encode_meta(request: &Request, name: &str) -> String {
    request
        .meta
        .get(name)
        .map(|value| {
            serde_json::to_string(&value.to_json()).unwrap_or_else(|_| "<invalid>".to_string())
        })
        .unwrap_or_else(|| "<missing>".to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn parse_keys(options: &BTreeMap<String, Value>) -> Result<Vec<Key>, SpiderError> {
    options
        .get("key")
        .or_else(|| options.get("keys"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(parse_key)
                .collect::<Result<Vec<_>, SpiderError>>()
        })
        .transpose()
        .map(|keys| keys.unwrap_or_else(|| vec![Key::Url]))
}

fn request_keys_override(
    request: &crate::request::Request,
) -> Result<Option<Vec<Key>>, SpiderError> {
    let Some(options) = request.middleware_options(DEDUP) else {
        return Ok(None);
    };

    Ok(Some(parse_keys(options)?))
}

fn parse_key(value: &Value) -> Result<Key, SpiderError> {
    let key = value
        .as_str()
        .ok_or_else(|| SpiderError::engine("request dedup key must be a string"))?;

    match key {
        "url" => Ok(Key::Url),
        "method" => Ok(Key::Method),
        "body" => Ok(Key::Body),
        _ if key.starts_with("meta.") => Ok(Key::Meta(key.trim_start_matches("meta.").to_string())),
        other => Err(SpiderError::engine(format!(
            "unsupported request dedup key: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::pin::Pin;
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct NoopWake;
        impl Wake for NoopWake {
            fn wake(self: Arc<Self>) {}
        }

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

    #[test]
    fn dedup_middleware_drops_duplicate_requests() {
        let middleware = DedupMiddleware::memory();
        let mut first = context::Enqueue::new(Request::new("https://example.com"));
        let mut second = context::Enqueue::new(Request::new("https://example.com"));

        assert!(matches!(
            block_on(middleware.before_enqueue(&mut first)).unwrap(),
            flow::Enqueue::Continue
        ));
        assert!(matches!(
            block_on(middleware.before_enqueue(&mut second)).unwrap(),
            flow::Enqueue::Drop { reason } if reason == DEDUP
        ));
    }
}
