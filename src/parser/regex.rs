use crate::parser::query::{Kind, ValueQuery};
use crate::parser::query::trim_text;
use crate::value::Value;
use regex::Regex;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegexQuery {
    pub input: String,
    pub pattern: String,
    pub source: Option<String>,
    pub groups: Vec<String>,
    pub value: ValueQuery,
}

impl RegexQuery {
    pub fn new(input: impl Into<String>, pattern: impl Into<String>, source: Option<String>) -> Self {
        let input = input.into();
        let pattern = pattern.into();
        let source_value = source.unwrap_or_else(|| "text".to_string());
        let (matches, groups) = capture(&input, &pattern);

        Self {
            input,
            pattern,
            source: Some(source_value.clone()),
            groups,
            value: ValueQuery::new(Kind::RegexGroup, source_value).with_values(
                matches.into_iter().map(Value::String).collect(),
            ),
        }
    }

    pub fn one(&self) -> Option<String> {
        self.value.one()
    }

    pub fn all(&self) -> Vec<String> {
        self.value.all()
    }

    pub fn group(&self, index: usize) -> Option<String> {
        self.groups
            .get(index)
            .map(|value| trim_text(value, self.value.trim))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_query_uses_text_source_by_default() {
        let query = RegexQuery::new("order id: 42", r"\d+", None);

        assert_eq!(query.source.as_deref(), Some("text"));
        assert!(query.value.trim);
    }

    #[test]
    fn regex_query_supports_group_lookup() {
        let query = RegexQuery::new("article_id=abc123", r"article_id=(\w+)", None);

        assert_eq!(query.one().as_deref(), Some("article_id=abc123"));
        assert_eq!(query.group(1).as_deref(), Some("abc123"));
    }
}

fn capture(input: &str, pattern: &str) -> (Vec<String>, Vec<String>) {
    let Ok(regex) = Regex::new(pattern) else {
        return (Vec::new(), Vec::new());
    };

    let matches = regex
        .captures_iter(input)
        .filter_map(|captures| captures.get(0).map(|capture| capture.as_str().to_string()))
        .collect::<Vec<_>>();

    let groups = regex
        .captures(input)
        .map(|captures| {
            captures
                .iter()
                .filter_map(|capture| capture.map(|capture| capture.as_str().to_string()))
                .collect()
        })
        .unwrap_or_default();

    (matches, groups)
}
