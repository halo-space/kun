#[derive(Debug, Clone, Default)]
pub struct XmlQuery {
    pub selector: String,
}

impl XmlQuery {
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
        }
    }
}
