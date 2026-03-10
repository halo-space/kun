use crate::parser::query::{Kind, ValueQuery};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegexQuery {
    pub pattern: String,
    pub source: Option<String>,
    pub value: ValueQuery,
}

impl RegexQuery {
    pub fn new(pattern: impl Into<String>, source: Option<String>) -> Self {
        let source_value = source.unwrap_or_else(|| "text".to_string());
        Self {
            pattern: pattern.into(),
            source: Some(source_value.clone()),
            value: ValueQuery::new(Kind::RegexGroup, source_value),
        }
    }

    pub fn one(&self) -> Option<String> {
        self.value.one()
    }

    pub fn all(&self) -> Vec<String> {
        self.value.all()
    }

    pub fn group(&self, index: usize) -> Option<String> {
        self.value.group(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_query_uses_text_source_by_default() {
        let query = RegexQuery::new(r"\d+", None);

        assert_eq!(query.source.as_deref(), Some("text"));
        assert!(query.value.trim);
    }
}
