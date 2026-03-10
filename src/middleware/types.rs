use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiddlewareType {
    Download,
    Spider,
}

#[derive(Debug, Clone)]
pub struct MiddlewareConfig {
    pub enabled: bool,
    pub r#type: MiddlewareType,
    pub order: i32,
    pub options: BTreeMap<String, Value>,
}
