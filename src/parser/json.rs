use crate::parser::query::{Kind, ValueQuery};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JsonQuery {
    pub value: ValueQuery,
}

impl JsonQuery {
    pub fn new(selector: Option<String>) -> Self {
        let source = selector.unwrap_or_else(|| "$".to_string());
        Self {
            value: ValueQuery::new(Kind::Structured, source),
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
        assert_eq!(JsonQuery::new(None).value.source, "$");
        assert_eq!(JsonQuery::new(Some("$.data.id".to_string())).value.source, "$.data.id");
    }
}
