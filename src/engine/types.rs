#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Flow {
    #[default]
    Continue,
    Drop(String),
    Retry {
        reason: String,
        backoff_ms: Option<u64>,
    },
}
