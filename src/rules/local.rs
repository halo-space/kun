use crate::error::SpiderError;
use crate::rules::schema::RulesConfig;
use crate::rules::source::RulesSource;
use crate::value::Value;

pub struct LocalRulesSource;

impl RulesSource for LocalRulesSource {
    fn load(&self, config: &RulesConfig) -> Result<Value, SpiderError> {
        let Some(path) = config.options.get("path") else {
            return Err(SpiderError::Rules("missing local rules path".to_string()));
        };

        let Value::String(path) = path else {
            return Err(SpiderError::Rules("missing local rules path".to_string()));
        };

        let content = std::fs::read_to_string(path)
            .map_err(|err| SpiderError::Rules(format!("failed reading rules file: {err}")))?;

        Ok(Value::String(content))
    }
}
