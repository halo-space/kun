use super::{Dedup, Key, fingerprint};
use crate::error::SpiderError;
use crate::request::Request;
use std::collections::HashSet;
use std::future;

/// Exact in-memory request deduplication.
///
/// By default it fingerprints requests by URL only, which matches the previous
/// built-in runtime dedup behavior. Callers can opt into richer fingerprints by
/// selecting more [`Key`] parts.
#[derive(Debug)]
pub struct Memory {
    keys: Vec<Key>,
    seen: HashSet<String>,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            keys: vec![Key::Url],
            seen: HashSet::new(),
        }
    }

    pub fn with_key(mut self, key: Key) -> Self {
        self.keys.push(key);
        self
    }

    pub fn with_keys(mut self, keys: impl IntoIterator<Item = Key>) -> Self {
        let keys = keys.into_iter().collect::<Vec<_>>();
        self.keys = if keys.is_empty() {
            vec![Key::Url]
        } else {
            keys
        };
        self
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl Dedup for Memory {
    fn check_and_insert(
        &mut self,
        request: &Request,
    ) -> impl std::future::Future<Output = Result<bool, SpiderError>> + Send {
        let accepted = self.seen.insert(fingerprint(request, &self.keys));
        future::ready(Ok(accepted))
    }

    fn check_and_insert_with_keys(
        &mut self,
        request: &Request,
        keys: &[Key],
    ) -> impl std::future::Future<Output = Result<bool, SpiderError>> + Send {
        let accepted = self.seen.insert(fingerprint(request, keys));
        future::ready(Ok(accepted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::value::Value;
    use futures::executor::block_on;

    #[test]
    fn memory_dedup_uses_url_only_by_default() {
        let mut dedup = Memory::default();

        let first = block_on(dedup.check_and_insert(&Request::new("https://example.com"))).unwrap();
        let second =
            block_on(dedup.check_and_insert(&Request::new("https://example.com"))).unwrap();

        assert!(first);
        assert!(!second);
    }

    #[test]
    fn memory_dedup_can_include_method_body_and_meta_fields() {
        let mut dedup = Memory::new().with_keys([
            Key::Url,
            Key::Method,
            Key::Body,
            Key::Meta("page".to_string()),
        ]);

        let first = Request::new("https://example.com")
            .with_method("POST")
            .with_body("hello".as_bytes().to_vec())
            .with_meta("page", Value::Number(1.0));
        let second = Request::new("https://example.com")
            .with_method("POST")
            .with_body("hello".as_bytes().to_vec())
            .with_meta("page", Value::Number(1.0));
        let third = Request::new("https://example.com")
            .with_method("POST")
            .with_body("hello".as_bytes().to_vec())
            .with_meta("page", Value::Number(2.0));

        assert!(block_on(dedup.check_and_insert(&first)).unwrap());
        assert!(!block_on(dedup.check_and_insert(&second)).unwrap());
        assert!(block_on(dedup.check_and_insert(&third)).unwrap());
    }
}
