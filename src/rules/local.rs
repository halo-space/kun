use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::rules::schema::Config;
use crate::value::Value;

pub struct Source;

impl crate::rules::source::Source for Source {
    fn load<'a>(&'a self, config: &'a Config) -> BoxFuture<'a, Result<Value, SpiderError>> {
        Box::pin(async move {
            let Some(path) = config.options.get("path") else {
                return Err(SpiderError::Rules("missing local rules path".to_string()));
            };

            let Value::String(path) = path else {
                return Err(SpiderError::Rules("missing local rules path".to_string()));
            };

            let content = std::fs::read_to_string(path)
                .map_err(|err| SpiderError::Rules(format!("failed reading rules file: {err}")))?;

            Ok(Value::String(content))
        })
    }
}
