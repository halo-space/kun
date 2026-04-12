use crate::value::Value;
use std::collections::BTreeMap;

/// Execution stage where a middleware hook applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Enqueue,
    Download,
    Spider,
    Item,
}

/// Runtime configuration for a middleware entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub enabled: bool,
    pub stage: Stage,
    pub order: i32,
    pub options: BTreeMap<String, Value>,
}
