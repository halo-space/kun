#[derive(Debug, Clone, Default)]
pub struct XPathQuery {
    pub selector: String,
}

impl XPathQuery {
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
        }
    }
}
