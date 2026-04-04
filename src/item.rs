use crate::value::Value;
use std::collections::BTreeMap;

pub type Fields = BTreeMap<String, Value>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    pub fields: Fields,
}

impl Item {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_fields(fields: Fields) -> Self {
        Self { fields }
    }

    pub fn with_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.fields.insert(key.into(), value);
        self
    }

    pub fn insert(&mut self, key: impl Into<String>, value: Value) -> Option<Value> {
        self.fields.insert(key.into(), value)
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.fields
                .iter()
                .map(|(key, value)| (key.clone(), value.to_json()))
                .collect(),
        )
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_can_hold_flat_fields() {
        let item = Item::new()
            .with_field("title", Value::String("hello".to_string()))
            .with_field("published", Value::Bool(true));

        assert_eq!(item.len(), 2);
        assert_eq!(item.get("title"), Some(&Value::String("hello".to_string())));
    }

    #[test]
    fn item_can_convert_to_json_object() {
        let item = Item::new()
            .with_field("title", Value::String("hello".to_string()))
            .with_field("published", Value::Bool(true));

        assert_eq!(
            item.to_json(),
            serde_json::json!({
                "published": true,
                "title": "hello"
            })
        );
    }
}
