pub mod compile;

use crate::middleware::Map;
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub schedule: BTreeMap<String, Value>,
    pub retry: BTreeMap<String, Value>,
    pub dedup: BTreeMap<String, Value>,
}

pub type MiddlewareMap = Map;

pub fn merge(base: &Config, overlay: &Config) -> Config {
    Config {
        schedule: merge_map(&base.schedule, &overlay.schedule),
        retry: merge_map(&base.retry, &overlay.retry),
        dedup: merge_map(&base.dedup, &overlay.dedup),
    }
}

fn merge_map(
    base: &BTreeMap<String, Value>,
    overlay: &BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    let mut merged = base.clone();

    for (key, value) in overlay {
        merged.insert(key.clone(), value.clone());
    }

    merged
}
