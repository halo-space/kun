pub mod auto_throttle;
pub(crate) mod bucket;
pub mod chain;
pub mod concurrency;
pub mod config;
pub mod cookies;
pub mod dedup;
pub mod http_cache;
pub mod interval;
pub mod proxy;
pub mod rate_limit;
pub mod retry_by_error;
pub mod retry_by_status;
pub mod traits;

use crate::error::SpiderError;
use crate::value::Value;
use std::collections::BTreeMap;

pub use auto_throttle::AutoThrottle;
pub use chain::{Chain, Entry};
pub use concurrency::Concurrency;
pub use config::{Config, Stage};
pub use cookies::Cookies;
pub use dedup::DedupMiddleware;
pub use http_cache::HttpCache;
pub use interval::Interval;
pub use proxy::Proxy;
pub use rate_limit::RateLimit;
pub use retry_by_error::RetryByError;
pub use retry_by_status::RetryByStatus;
pub use traits::Middleware;
use traits::{BoxMiddleware, box_middleware};

pub const DEDUP: &str = "dedup";
pub const AUTO_THROTTLE: &str = "auto_throttle";
pub const RETRY_BY_STATUS: &str = "retry_by_status";
pub const RETRY_BY_ERROR: &str = "retry_by_error";
pub const CONCURRENCY: &str = "concurrency";
pub const INTERVAL: &str = "interval";
pub const RATE_LIMIT: &str = "rate_limit";
pub const COOKIES: &str = "cookies";
pub const HTTP_CACHE: &str = "http_cache";
pub const PROXY: &str = "proxy";

pub type Map = BTreeMap<String, Config>;

#[derive(Default, Clone)]
pub(crate) struct SharedState {
    pub concurrency: concurrency::SharedRegistry,
    pub interval: interval::SharedRegistry,
    pub rate_limit: rate_limit::SharedRegistry,
    pub auto_throttle: auto_throttle::SharedRegistry,
}

/// Factory function: takes options from middleware config, returns a middleware instance.
pub(crate) type Factory =
    Box<dyn Fn(&BTreeMap<String, Value>) -> Result<BoxMiddleware, SpiderError> + Send + Sync>;

/// Registry of custom middleware factories keyed by middleware name.
#[derive(Default)]
pub struct Registry {
    factories: BTreeMap<String, Factory>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<M>(
        &mut self,
        key: impl Into<String>,
        factory: impl Fn(&BTreeMap<String, Value>) -> Result<M, SpiderError> + Send + Sync + 'static,
    ) where
        M: Middleware + 'static,
    {
        self.factories.insert(
            key.into(),
            Box::new(move |options| factory(options).map(box_middleware)),
        );
    }

    pub fn has(&self, key: &str) -> bool {
        self.factories.contains_key(key)
    }
}

pub fn build(configs: &Map, custom: &Registry) -> Result<Chain, SpiderError> {
    build_with_shared(configs, custom, &SharedState::default())
}

pub(crate) fn build_with_shared(
    configs: &Map,
    custom: &Registry,
    shared: &SharedState,
) -> Result<Chain, SpiderError> {
    let mut chain = Chain::default();

    for (key, config) in configs {
        chain.push_boxed(
            key.clone(),
            config.clone(),
            instantiate(key, configs, custom, shared)?,
        );
    }

    Ok(chain)
}

fn instantiate(
    key: &str,
    configs: &Map,
    custom: &Registry,
    shared: &SharedState,
) -> Result<BoxMiddleware, SpiderError> {
    let options = &configs[key].options;

    let middleware = match key {
        DEDUP => dedup::from_options(options)?,
        AUTO_THROTTLE => box_middleware(AutoThrottle::new(options, &shared.auto_throttle)?),
        RETRY_BY_STATUS => box_middleware(RetryByStatus::new(options)),
        RETRY_BY_ERROR => box_middleware(RetryByError::new(options)),
        CONCURRENCY => box_middleware(Concurrency::new(options, &shared.concurrency)?),
        INTERVAL => box_middleware(Interval::new(options, &shared.interval)?),
        RATE_LIMIT => box_middleware(RateLimit::new(options, &shared.rate_limit)?),
        COOKIES => box_middleware(Cookies::new(options)),
        HTTP_CACHE => box_middleware(HttpCache::new(options)),
        PROXY => box_middleware(Proxy::new(options)),
        other => {
            if let Some(factory) = custom.factories.get(other) {
                factory(options)?
            } else {
                return Err(SpiderError::engine(format!(
                    "unknown middleware key: {other}"
                )));
            }
        }
    };

    Ok(middleware)
}
