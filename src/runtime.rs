pub mod compile;

use crate::middleware::Map;
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub schedule: BTreeMap<String, Value>,
    pub retry: BTreeMap<String, Value>,
    pub dedup: BTreeMap<String, Value>,
}

pub type MiddlewareMap = Map;
