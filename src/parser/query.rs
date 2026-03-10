use crate::value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Nodes,
    Text,
    Html,
    Attribute,
    Structured,
    RegexGroup,
    Ai,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeQuery {
    pub selector: String,
    pub trim: bool,
}

impl Default for NodeQuery {
    fn default() -> Self {
        Self {
            selector: String::new(),
            trim: true,
        }
    }
}

impl NodeQuery {
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            ..Self::default()
        }
    }

    pub fn with_trim(mut self, trim: bool) -> Self {
        self.trim = trim;
        self
    }

    pub fn one(&self) -> Option<String> {
        None
    }

    pub fn all(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn text(&self) -> ValueQuery {
        ValueQuery::new(Kind::Text, self.selector.clone()).with_trim(self.trim)
    }

    pub fn html(&self) -> ValueQuery {
        ValueQuery::new(Kind::Html, self.selector.clone()).with_trim(false)
    }

    pub fn attr(&self, name: impl Into<String>) -> ValueQuery {
        ValueQuery::new(Kind::Attribute, format!("{}::attr({})", self.selector, name.into()))
            .with_trim(self.trim)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueQuery {
    pub kind: Kind,
    pub source: String,
    pub trim: bool,
    pub values: Vec<Value>,
}

impl Default for ValueQuery {
    fn default() -> Self {
        Self {
            kind: Kind::Structured,
            source: String::new(),
            trim: false,
            values: Vec::new(),
        }
    }
}

impl ValueQuery {
    pub fn new(kind: Kind, source: impl Into<String>) -> Self {
        Self {
            kind,
            source: source.into(),
            trim: matches!(kind, Kind::Text | Kind::Attribute | Kind::RegexGroup | Kind::Ai),
            values: Vec::new(),
        }
    }

    pub fn with_trim(mut self, trim: bool) -> Self {
        self.trim = trim;
        self
    }

    pub fn with_values(mut self, values: Vec<Value>) -> Self {
        self.values = values;
        self
    }

    pub fn one(&self) -> Option<String> {
        self.values.first().and_then(|value| stringify(value, self.trim))
    }

    pub fn all(&self) -> Vec<String> {
        self.values
            .iter()
            .filter_map(|value| stringify(value, self.trim))
            .collect()
    }

    pub fn value(&self) -> Option<Value> {
        self.values.first().cloned()
    }

    pub fn group(&self, index: usize) -> Option<String> {
        self.values.get(index).and_then(|value| stringify(value, self.trim))
    }
}

fn stringify(value: &Value, trim: bool) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
    };

    if trim {
        Some(text.trim().to_string())
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_query_trims_by_default() {
        let query = NodeQuery::new("h1.title")
            .text()
            .with_values(vec![Value::String("  hello  ".to_string())]);

        assert_eq!(query.one().as_deref(), Some("hello"));
    }

    #[test]
    fn structured_values_do_not_trim_by_default() {
        let query = ValueQuery::new(Kind::Structured, "$.data")
            .with_values(vec![Value::String("  raw  ".to_string())]);

        assert_eq!(query.one().as_deref(), Some("  raw  "));
    }

    #[test]
    fn node_query_attr_keeps_trim_hook() {
        let query = NodeQuery::new("a.link")
            .attr("href")
            .with_values(vec![Value::String(" /detail ".to_string())]);

        assert_eq!(query.one().as_deref(), Some("/detail"));
    }
}
