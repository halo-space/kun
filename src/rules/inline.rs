use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::rules::schema::Config;
use crate::value::Value;

pub struct Source;

impl crate::rules::source::Source for Source {
    fn load<'a>(&'a self, config: &'a Config) -> BoxFuture<'a, Result<Value, SpiderError>> {
        Box::pin(async move {
            config
                .options
                .get("value")
                .cloned()
                .ok_or_else(|| SpiderError::Rules("missing inline rules value".to_string()))
        })
    }
}
