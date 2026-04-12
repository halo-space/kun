/// Control flow for enqueue admission hooks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Enqueue {
    #[default]
    Continue,
    Drop {
        reason: String,
    },
}

/// Control flow for download lifecycle hooks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Download {
    #[default]
    Continue,
    Drop {
        reason: String,
    },
    Delay {
        reason: String,
        millis: u64,
    },
    Retry {
        reason: String,
        backoff: Option<u64>,
    },
}

/// Control flow for parse lifecycle hooks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Parse {
    #[default]
    Continue,
    Drop {
        reason: String,
    },
}

/// Control flow for item lifecycle hooks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Item {
    #[default]
    Continue,
    Drop {
        reason: String,
    },
}
