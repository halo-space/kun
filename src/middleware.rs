pub mod chain;
pub mod cookies;
pub mod dedup;
pub mod interval_gate;
pub mod proxy;
pub mod rate_limit;
pub mod retry_by_error;
pub mod retry_by_status;
pub mod traits;
pub mod types;

use crate::error::SpiderError;
use crate::value::Value;
use std::collections::BTreeMap;

pub use chain::{MiddlewareChain, MiddlewareEntry};
pub use cookies::CookiesMiddleware;
pub use dedup::DedupMiddleware;
pub use interval_gate::IntervalGateMiddleware;
pub use proxy::ProxyMiddleware;
pub use rate_limit::RateLimitMiddleware;
pub use retry_by_error::RetryByErrorMiddleware;
pub use retry_by_status::RetryByStatusMiddleware;
pub use traits::Middleware;
pub use types::{MiddlewareConfig, MiddlewareType};

pub type Map = BTreeMap<String, MiddlewareConfig>;

/// Factory function: takes options from middleware config, returns a middleware instance.
pub type Factory = Box<dyn Fn(&BTreeMap<String, Value>) -> Result<Box<dyn Middleware>, SpiderError> + Send + Sync>;

/// Registry of custom middleware factories keyed by middleware name.
#[derive(Default)]
pub struct FactoryRegistry {
    factories: BTreeMap<String, Factory>,
}

impl FactoryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        key: impl Into<String>,
        factory: impl Fn(&BTreeMap<String, Value>) -> Result<Box<dyn Middleware>, SpiderError> + Send + Sync + 'static,
    ) {
        self.factories.insert(key.into(), Box::new(factory));
    }

    pub fn has(&self, key: &str) -> bool {
        self.factories.contains_key(key)
    }
}

pub fn build(configs: &Map, custom: &FactoryRegistry) -> Result<MiddlewareChain, SpiderError> {
    let mut chain = MiddlewareChain::default();

    for (key, config) in configs {
        chain.push(key.clone(), config.clone(), instantiate(key, configs, custom)?);
    }

    Ok(chain)
}

fn instantiate(
    key: &str,
    configs: &Map,
    custom: &FactoryRegistry,
) -> Result<Box<dyn Middleware>, SpiderError> {
    let options = &configs[key].options;

    let middleware: Box<dyn Middleware> = match key {
        "retry_by_status" => Box::new(RetryByStatusMiddleware::new(options)),
        "retry_by_error" => Box::new(RetryByErrorMiddleware::new(options)),
        "dedup" => Box::new(DedupMiddleware::new(options)),
        "interval_gate" => Box::new(IntervalGateMiddleware::new(options)),
        "rate_limit" => Box::new(RateLimitMiddleware::new(options)),
        "cookies" => Box::new(CookiesMiddleware::new(options)),
        "proxy" => Box::new(ProxyMiddleware::new(options)),
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
