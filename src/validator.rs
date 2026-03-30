use crate::error::SpiderError;
use crate::item::Item;
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationType {
    Text,
    Number,
    Bool,
    List,
    Object,
}

impl ValidationType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::List => "list",
            Self::Object => "object",
        }
    }

    fn matches(self, value: &Value) -> bool {
        match self {
            Self::Text => matches!(value, Value::String(_)),
            Self::Number => matches!(value, Value::Number(_)),
            Self::Bool => matches!(value, Value::Bool(_)),
            Self::List => matches!(value, Value::Array(_)),
            Self::Object => matches!(value, Value::Object(_)),
        }
    }
}

impl TryFrom<&str> for ValidationType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "text" => Ok(Self::Text),
            "number" => Ok(Self::Number),
            "bool" => Ok(Self::Bool),
            "list" => Ok(Self::List),
            "object" => Ok(Self::Object),
            other => Err(format!("unsupported validation type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationRule {
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidationPlan {
    pub name: String,
    pub value_type: ValidationType,
    pub rule: ValidationRule,
}

impl ValidationPlan {
    pub fn new(name: impl Into<String>, value_type: ValidationType) -> Self {
        Self {
            name: name.into(),
            value_type,
            rule: ValidationRule::default(),
        }
    }

    pub fn with_required(mut self, required: bool) -> Self {
        self.rule.required = required;
        self
    }
}

pub fn validate_fields(
    fields: &BTreeMap<String, Value>,
    plans: &[ValidationPlan],
) -> Result<(), SpiderError> {
    for plan in plans {
        validate_field(plan, fields.get(plan.name.as_str()))?;
    }

    Ok(())
}

pub fn validate_item(item: &Item, plans: &[ValidationPlan]) -> Result<(), SpiderError> {
    validate_fields(&item.fields, plans)
}

fn validate_field(plan: &ValidationPlan, value: Option<&Value>) -> Result<(), SpiderError> {
    let Some(value) = value.filter(|value| !matches!(value, Value::Null)) else {
        if plan.rule.required {
            return Err(SpiderError::parse(format!(
                "validation failed for field {}: value is required",
                plan.name
            )));
        }
        return Ok(());
    };

    if plan.value_type.matches(value) {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: expected {}",
        plan.name,
        plan.value_type.as_str()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_fields_accepts_matching_values() {
        let fields = BTreeMap::from([
            ("title".to_string(), Value::String("hello".to_string())),
            ("published".to_string(), Value::Bool(true)),
        ]);
        let plans = vec![
            ValidationPlan::new("title", ValidationType::Text).with_required(true),
            ValidationPlan::new("published", ValidationType::Bool).with_required(true),
        ];

        assert!(validate_fields(&fields, &plans).is_ok());
    }

    #[test]
    fn validate_fields_rejects_missing_required_field() {
        let fields = BTreeMap::new();
        let plans = vec![ValidationPlan::new("title", ValidationType::Text).with_required(true)];

        let error = validate_fields(&fields, &plans).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse("validation failed for field title: value is required".to_string())
        );
    }

    #[test]
    fn validate_fields_rejects_type_mismatch() {
        let fields = BTreeMap::from([("title".to_string(), Value::Number(1.0))]);
        let plans = vec![ValidationPlan::new("title", ValidationType::Text).with_required(true)];

        let error = validate_fields(&fields, &plans).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse("validation failed for field title: expected text".to_string())
        );
    }

    #[test]
    fn validate_item_uses_same_validation_semantics() {
        let item = Item::new().with_field("count", Value::Number(3.0));
        let plans = vec![ValidationPlan::new("count", ValidationType::Number).with_required(true)];

        assert!(validate_item(&item, &plans).is_ok());
    }
}
