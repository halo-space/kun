#[derive(Debug, Clone, Default)]
pub struct CssQuery {
    pub selector: String,
}

impl CssQuery {
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
        }
    }
}
