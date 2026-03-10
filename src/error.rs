use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
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

impl SpiderError {
    pub fn request_build(message: impl Into<String>) -> Self {
        Self::RequestBuild(message.into())
    }

    pub fn download(message: impl Into<String>) -> Self {
        Self::Download(message.into())
    }

    pub fn parse(message: impl Into<String>) -> Self {
        Self::Parse(message.into())
    }

    pub fn rules(message: impl Into<String>) -> Self {
        Self::Rules(message.into())
    }

    pub fn plugin(message: impl Into<String>) -> Self {
        Self::Plugin(message.into())
    }

    pub fn scheduler(message: impl Into<String>) -> Self {
        Self::Scheduler(message.into())
    }

    pub fn engine(message: impl Into<String>) -> Self {
        Self::Engine(message.into())
    }
}
