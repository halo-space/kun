use crate::value::Value;
pub mod output;

#[derive(Debug, Clone, Default)]
pub struct Item {
    pub value: Value,
}
