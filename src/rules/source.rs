use crate::error::SpiderError;
use crate::rules::schema::RulesConfig;
use crate::value::Value;

pub trait RulesSource {
    fn load(&self, config: &RulesConfig) -> Result<Value, SpiderError>;
}
