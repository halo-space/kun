#[derive(Debug, Clone, Default)]
pub struct JsonQuery {
    pub selector: Option<String>,
}

impl JsonQuery {
    pub fn new(selector: Option<String>) -> Self {
        Self { selector }
    }
}
