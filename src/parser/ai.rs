#[derive(Debug, Clone, Default)]
pub struct AiQuery {
    pub prompt: String,
    pub source: Option<String>,
}

impl AiQuery {
    pub fn new(prompt: impl Into<String>, source: Option<String>) -> Self {
        Self {
            prompt: prompt.into(),
            source,
        }
    }
}
