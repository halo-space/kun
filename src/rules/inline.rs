use crate::error::SpiderError;
use crate::rules::schema::RulesConfig;
use crate::rules::source::RulesSource;
use crate::value::Value;

pub struct InlineRulesSource;

impl RulesSource for InlineRulesSource {
    fn load(&self, config: &RulesConfig) -> Result<Value, SpiderError> {
        config
            .options
            .get("value")
            .cloned()
            .ok_or_else(|| SpiderError::Rules("missing inline rules value".to_string()))
    }
}
