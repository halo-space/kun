use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SpiderError {
    #[error("request build error: {0}")]
    RequestBuild(String),
    #[error("download error: {0}")]
    Download(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("rules error: {0}")]
    Rules(String),
    #[error("plugin error: {0}")]
    Plugin(String),
    #[error("scheduler error: {0}")]
    Scheduler(String),
    #[error("engine error: {0}")]
    Engine(String),
}

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
