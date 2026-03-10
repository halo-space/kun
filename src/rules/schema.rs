use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct RulesConfig {
    pub r#type: String,
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct StepConfig {
    pub id: String,
    pub r#impl: String,
    pub callback: Option<String>,
}
