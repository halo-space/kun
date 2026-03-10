use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum SpiderError {
    RequestBuild(String),
    Download(String),
    Parse(String),
    Rules(String),
    Plugin(String),
    Scheduler(String),
    Engine(String),
}

impl Display for SpiderError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequestBuild(msg) => write!(f, "request build error: {msg}"),
            Self::Download(msg) => write!(f, "download error: {msg}"),
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::Rules(msg) => write!(f, "rules error: {msg}"),
            Self::Plugin(msg) => write!(f, "plugin error: {msg}"),
            Self::Scheduler(msg) => write!(f, "scheduler error: {msg}"),
            Self::Engine(msg) => write!(f, "engine error: {msg}"),
        }
    }
}

impl std::error::Error for SpiderError {}
