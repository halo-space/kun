pub mod compile;

use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    pub schedule: BTreeMap<String, Value>,
    pub retry: BTreeMap<String, Value>,
    pub dedup: BTreeMap<String, Value>,
}
