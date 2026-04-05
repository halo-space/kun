/// Middleware control flow returned by middleware hooks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Flow {
    #[default]
    Continue,
    Drop(String),
    Retry {
        reason: String,
        backoff: Option<u64>,
    },
}
