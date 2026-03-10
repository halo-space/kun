use crate::parser::query::{Kind, ValueQuery};
use crate::value::Value;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JsonQuery {
    pub input: String,
    pub value: ValueQuery,
}

impl JsonQuery {
    pub fn new(input: impl Into<String>, selector: Option<String>) -> Self {
        let input = input.into();
        let source = selector.unwrap_or_else(|| "$".to_string());
        let values = select(&input, &source)
            .into_iter()
            .map(to_value)
            .collect();

        Self {
            input,
            value: ValueQuery::new(Kind::Structured, source).with_values(values),
        }
    }

    pub fn one(&self) -> Option<String> {
        self.value.one()
    }

    pub fn all(&self) -> Vec<String> {
        self.value.all()
    }

    pub fn value(&self) -> Option<crate::value::Value> {
        self.value.value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_query_supports_optional_selector() {
        assert_eq!(JsonQuery::new("{}", None).value.source, "$");
        assert_eq!(
            JsonQuery::new("{}", Some("$.data.id".to_string())).value.source,
            "$.data.id"
        );
    }

    #[test]
    fn json_query_reads_scalar_values() {
        let query = JsonQuery::new(r#"{"data":{"id":42}}"#, Some("$.data.id".to_string()));

        assert_eq!(query.one().as_deref(), Some("42"));
    }

    #[test]
    fn json_query_returns_structured_value() {
        let query = JsonQuery::new(r#"{"data":{"title":"post"}}"#, Some("$.data".to_string()));

        assert_eq!(
            query.value(),
            Some(Value::Object(
                [("title".to_string(), Value::String("post".to_string()))]
                    .into_iter()
                    .collect()
            ))
        );
    }
}

fn select(input: &str, selector: &str) -> Vec<JsonValue> {
    let Ok(root) = serde_json::from_str::<JsonValue>(input) else {
        return Vec::new();
    };

    let Some(segments) = parse(selector) else {
        return Vec::new();
    };

    let mut current = vec![&root];
    for segment in segments {
        current = match segment {
            Segment::Key(key) => current
                .into_iter()
                .filter_map(|value| value.as_object().and_then(|object| object.get(&key)))
                .collect(),
            Segment::Index(index) => current
                .into_iter()
                .filter_map(|value| value.as_array().and_then(|array| array.get(index)))
                .collect(),
        };
    }

    current.into_iter().cloned().collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Key(String),
    Index(usize),
}

fn parse(selector: &str) -> Option<Vec<Segment>> {
    if selector == "$" {
        return Some(Vec::new());
    }

    let mut chars = selector.chars().peekable();
    if chars.next()? != '$' {
        return None;
    }

    let mut segments = Vec::new();
    while let Some(ch) = chars.peek().copied() {
        match ch {
            '.' => {
                chars.next();
                let mut key = String::new();
                while let Some(ch) = chars.peek().copied() {
                    if ch == '.' || ch == '[' {
                        break;
                    }
                    key.push(ch);
                    chars.next();
                }
                if key.is_empty() {
                    return None;
                }
                segments.push(Segment::Key(key));
            }
            '[' => {
                chars.next();
                let mut index = String::new();
                while let Some(ch) = chars.peek().copied() {
                    if ch == ']' {
                        break;
                    }
                    index.push(ch);
                    chars.next();
                }
                if chars.next()? != ']' {
                    return None;
                }
                segments.push(Segment::Index(index.parse().ok()?));
            }
            _ => return None,
        }
    }

    Some(segments)
}

fn to_value(value: JsonValue) -> Value {
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(value) => Value::Bool(value),
        JsonValue::Number(value) => Value::Number(value.as_f64().unwrap_or_default()),
        JsonValue::String(value) => Value::String(value),
        JsonValue::Array(values) => Value::Array(values.into_iter().map(to_value).collect()),
        JsonValue::Object(values) => {
            Value::Object(values.into_iter().map(|(key, value)| (key, to_value(value))).collect())
        }
    }
}
