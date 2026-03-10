use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::rules::schema::Config;
use crate::value::Value;

pub trait Source: Send + Sync {
    fn load<'a>(&'a self, config: &'a Config) -> BoxFuture<'a, Result<Value, SpiderError>>;
}
