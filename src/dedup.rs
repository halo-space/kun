pub mod bloom;
pub mod memory;
pub mod noop;

use crate::error::SpiderError;
use crate::request::Request;

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
#[allow(async_fn_in_trait)]
pub trait Dedup: Send + Sync {
    /// Returns `true` when the request should be accepted and recorded as seen.
    ///
    /// Returning `false` means the request is considered a duplicate and will
    /// not be enqueued into the scheduler.
    async fn check_and_insert(&mut self, request: &Request) -> Result<bool, SpiderError>;
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
