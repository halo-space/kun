#[derive(Debug, Clone, Default)]
pub struct RegexQuery {
    pub pattern: String,
    pub source: Option<String>,
}

impl RegexQuery {
    pub fn new(pattern: impl Into<String>, source: Option<String>) -> Self {
        Self {
            pattern: pattern.into(),
            source,
        }
    }
}
