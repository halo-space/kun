#[derive(Debug, Clone, Default)]
pub struct NodeQuery {
    pub selector: String,
}

impl NodeQuery {
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ValueQuery {
    pub source: String,
}

impl ValueQuery {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
        }
    }
}
