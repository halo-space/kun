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

pub fn build(configs: &Map) -> Result<MiddlewareChain, SpiderError> {
    let mut chain = MiddlewareChain::default();

    for (key, config) in configs {
        chain.push(key.clone(), config.clone(), instantiate(key)?);
    }

    Ok(chain)
}

fn instantiate(key: &str) -> Result<Box<dyn Middleware>, SpiderError> {
    let middleware: Box<dyn Middleware> = match key {
        "retry_by_status" => Box::new(RetryByStatusMiddleware),
        "retry_by_error" => Box::new(RetryByErrorMiddleware),
        "dedup" => Box::new(DedupMiddleware),
        "interval_gate" => Box::new(IntervalGateMiddleware),
        "rate_limit" => Box::new(RateLimitMiddleware),
        "cookies" => Box::new(CookiesMiddleware),
        "proxy" => Box::new(ProxyMiddleware),
        other => {
            return Err(SpiderError::engine(format!(
                "unknown middleware key: {other}"
            )))
        }
    };

    Ok(middleware)
}
