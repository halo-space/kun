use crate::dedup::{Dedup, Key, fingerprint};
use crate::error::SpiderError;
use crate::request::Request;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

const DEFAULT_EXPECTED_ITEMS: usize = 100_000;
const DEFAULT_FALSE_POSITIVE_RATE: f64 = 0.01;
const MIN_FALSE_POSITIVE_RATE: f64 = 1e-9;
const MAX_FALSE_POSITIVE_RATE: f64 = 0.5;

/// Bloom-filter-based request deduplication.
///
/// This implementation trades exactness for bounded memory usage: once the
/// filter is saturated enough, it can report false positives and drop a
/// request that has not actually been seen before.
#[derive(Debug)]
pub struct Bloom {
    keys: Vec<Key>,
    expected_items: usize,
    false_positive_rate: f64,
    bit_len: usize,
    hash_count: u32,
    bits: Vec<u8>,
}

impl Bloom {
    pub fn new() -> Self {
        Self::default()
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

    pub fn with_expected_items(mut self, expected_items: usize) -> Self {
        self.expected_items = expected_items.max(1);
        self.reset_filter();
        self
    }

    pub fn with_false_positive_rate(mut self, rate: f64) -> Self {
        self.false_positive_rate = clamp_false_positive_rate(rate);
        self.reset_filter();
        self
    }

    pub fn expected_items(&self) -> usize {
        self.expected_items
    }

    pub fn false_positive_rate(&self) -> f64 {
        self.false_positive_rate
    }

    pub fn bit_len(&self) -> usize {
        self.bit_len
    }

    pub fn hash_count(&self) -> u32 {
        self.hash_count
    }

    fn reset_filter(&mut self) {
        let (bit_len, hash_count) = bloom_shape(self.expected_items, self.false_positive_rate);
        self.bit_len = bit_len;
        self.hash_count = hash_count;
        self.bits = vec![0; bit_len.div_ceil(8)];
    }

    fn bit(&self, index: usize) -> bool {
        let byte = self.bits[index / 8];
        let mask = 1_u8 << (index % 8);
        (byte & mask) != 0
    }

    fn set_bit(&mut self, index: usize) {
        let byte = &mut self.bits[index / 8];
        *byte |= 1_u8 << (index % 8);
    }
}

impl Default for Bloom {
    fn default() -> Self {
        let expected_items = DEFAULT_EXPECTED_ITEMS;
        let false_positive_rate = DEFAULT_FALSE_POSITIVE_RATE;
        let (bit_len, hash_count) = bloom_shape(expected_items, false_positive_rate);

        Self {
            keys: vec![Key::Url],
            expected_items,
            false_positive_rate,
            bit_len,
            hash_count,
            bits: vec![0; bit_len.div_ceil(8)],
        }
    }
}

impl Dedup for Bloom {
    async fn check_and_insert(&mut self, request: &Request) -> Result<bool, SpiderError> {
        let fingerprint = fingerprint(request, &self.keys);
        let indexes = bloom_indexes(fingerprint.as_str(), self.bit_len, self.hash_count);

        if indexes.iter().all(|index| self.bit(*index)) {
            return Ok(false);
        }

        for index in indexes {
            self.set_bit(index);
        }

        Ok(true)
    }
}

fn bloom_shape(expected_items: usize, false_positive_rate: f64) -> (usize, u32) {
    let expected_items = expected_items.max(1) as f64;
    let false_positive_rate = clamp_false_positive_rate(false_positive_rate);
    let bit_len =
        (-(expected_items * false_positive_rate.ln()) / std::f64::consts::LN_2.powi(2)).ceil();
    let hash_count = ((bit_len / expected_items) * std::f64::consts::LN_2).ceil();

    (bit_len.max(64.0) as usize, hash_count.max(1.0) as u32)
}

fn clamp_false_positive_rate(rate: f64) -> f64 {
    if !rate.is_finite() {
        return DEFAULT_FALSE_POSITIVE_RATE;
    }

    rate.clamp(MIN_FALSE_POSITIVE_RATE, MAX_FALSE_POSITIVE_RATE)
}

fn bloom_indexes(fingerprint: &str, bit_len: usize, hash_count: u32) -> Vec<usize> {
    let first = hash_with_seed(0, fingerprint);
    let second = hash_with_seed(1, fingerprint) | 1;

    (0..hash_count)
        .map(|offset| {
            first
                .wrapping_add(u64::from(offset).wrapping_mul(second))
                .rem_euclid(bit_len as u64) as usize
        })
        .collect()
}

fn hash_with_seed(seed: u64, value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::value::Value;
    use futures::executor::block_on;

    #[test]
    fn bloom_dedup_rejects_exact_duplicates() {
        let mut dedup = Bloom::default();

        let first = block_on(dedup.check_and_insert(&Request::new("https://example.com"))).unwrap();
        let second =
            block_on(dedup.check_and_insert(&Request::new("https://example.com"))).unwrap();

        assert!(first);
        assert!(!second);
    }

    #[test]
    fn bloom_dedup_can_include_method_body_and_meta_fields() {
        let mut dedup = Bloom::new().with_keys([
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

    #[test]
    fn bloom_dedup_exposes_shape_configuration() {
        let dedup = Bloom::new()
            .with_expected_items(10_000)
            .with_false_positive_rate(0.005);

        assert_eq!(dedup.expected_items(), 10_000);
        assert_eq!(dedup.false_positive_rate(), 0.005);
        assert!(dedup.bit_len() > 0);
        assert!(dedup.hash_count() > 0);
    }
}
