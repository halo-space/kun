pub mod chain;
pub mod concurrency_gate;
pub mod config;
pub mod cookies;
pub mod dedup;
pub mod interval_gate;
pub mod proxy;
pub mod rate_limit;
pub mod retry_by_error;
pub mod retry_by_status;
pub mod traits;

use crate::error::SpiderError;
use crate::value::Value;
use std::collections::BTreeMap;

pub use chain::{Chain, Entry};
pub use concurrency_gate::ConcurrencyGate;
pub use config::{Config, Stage};
pub use cookies::Cookies;
pub use dedup::Dedup;
pub use interval_gate::IntervalGate;
pub use proxy::Proxy;
pub use rate_limit::RateLimit;
pub use retry_by_error::RetryByError;
pub use retry_by_status::RetryByStatus;
pub use traits::Middleware;

pub type Map = BTreeMap<String, Config>;

/// Factory function: takes options from middleware config, returns a middleware instance.
pub type Factory =
    Box<dyn Fn(&BTreeMap<String, Value>) -> Result<Box<dyn Middleware>, SpiderError> + Send + Sync>;

/// Registry of custom middleware factories keyed by middleware name.
#[derive(Default)]
pub struct Registry {
    factories: BTreeMap<String, Factory>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        key: impl Into<String>,
        factory: impl Fn(&BTreeMap<String, Value>) -> Result<Box<dyn Middleware>, SpiderError>
        + Send
        + Sync
        + 'static,
    ) {
        self.factories.insert(key.into(), Box::new(factory));
    }

    pub fn has(&self, key: &str) -> bool {
        self.factories.contains_key(key)
    }
}

pub fn build(configs: &Map, custom: &Registry) -> Result<Chain, SpiderError> {
    let mut chain = Chain::default();

    for (key, config) in configs {
        chain.push(
            key.clone(),
            config.clone(),
            instantiate(key, configs, custom)?,
        );
    }

    Ok(chain)
}

fn instantiate(
    key: &str,
    configs: &Map,
    custom: &Registry,
) -> Result<Box<dyn Middleware>, SpiderError> {
    let options = &configs[key].options;

    let middleware: Box<dyn Middleware> = match key {
        "retry_by_status" => Box::new(RetryByStatus::new(options)),
        "retry_by_error" => Box::new(RetryByError::new(options)),
        "dedup" => Box::new(Dedup::new(options)),
        "concurrency_gate" => Box::new(ConcurrencyGate::new(options)),
        "interval_gate" => Box::new(IntervalGate::new(options)),
        "rate_limit" => Box::new(RateLimit::new(options)),
        "cookies" => Box::new(Cookies::new(options)),
        "proxy" => Box::new(Proxy::new(options)),
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
