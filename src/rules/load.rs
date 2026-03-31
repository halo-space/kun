use crate::error::SpiderError;
use crate::rules::compile::compile_rules;
use crate::rules::schema::{Compiled, Config};
use crate::rules::source::Source;

pub async fn load(config: &Config) -> Result<Compiled, SpiderError> {
    let source: Box<dyn Source> = match config.r#type.as_str() {
        "local" => Box::new(crate::rules::local::Source),
        "inline" => Box::new(crate::rules::inline::Source),
        other => {
            return Err(SpiderError::rules(format!(
                "unsupported rules source type: {other}"
            )));
        }
    };

    let value = source.load(config).await?;
    compile_rules(value)
}
