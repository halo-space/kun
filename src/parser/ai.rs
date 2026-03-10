use crate::parser::query::{Kind, ValueQuery};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AiQuery {
    pub input: String,
    pub prompt: String,
    pub source: Option<String>,
    pub value: ValueQuery,
}

impl AiQuery {
    pub fn new(input: impl Into<String>, prompt: impl Into<String>, source: Option<String>) -> Self {
        let source_value = source.unwrap_or_else(|| "html".to_string());
        Self {
            input: input.into(),
            prompt: prompt.into(),
            source: Some(source_value.clone()),
            value: ValueQuery::new(Kind::Ai, source_value),
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
    fn ai_query_uses_html_source_by_default() {
        let query = AiQuery::new("<h1>Title</h1>", "extract title", None);

        assert_eq!(query.source.as_deref(), Some("html"));
        assert!(query.value.trim);
    }
}
