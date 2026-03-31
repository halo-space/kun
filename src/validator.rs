use crate::error::SpiderError;
use crate::item::Item;
use crate::value::Value;
use regex::Regex;
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationRule {
    pub required: bool,
    pub regex: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub enum_values: Vec<Value>,
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

    pub fn with_regex(mut self, pattern: impl Into<String>) -> Self {
        self.rule.regex = Some(pattern.into());
        self
    }

    pub fn with_min(mut self, min: f64) -> Self {
        self.rule.min = Some(min);
        self
    }

    pub fn with_max(mut self, max: f64) -> Self {
        self.rule.max = Some(max);
        self
    }

    pub fn with_enum(mut self, values: impl IntoIterator<Item = Value>) -> Self {
        self.rule.enum_values = values.into_iter().collect();
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
        return validate_rule(plan, value);
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: expected {}",
        plan.name,
        plan.value_type.as_str()
    )))
}

fn validate_rule(plan: &ValidationPlan, value: &Value) -> Result<(), SpiderError> {
    if let Some(pattern) = plan.rule.regex.as_deref() {
        validate_regex(plan, value, pattern)?;
    }

    if let Some(min) = plan.rule.min {
        validate_min(plan, value, min)?;
    }

    if let Some(max) = plan.rule.max {
        validate_max(plan, value, max)?;
    }

    if !plan.rule.enum_values.is_empty() && !plan.rule.enum_values.iter().any(|item| item == value)
    {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: value is not in enum set",
            plan.name
        )));
    }

    Ok(())
}

fn validate_regex(plan: &ValidationPlan, value: &Value, pattern: &str) -> Result<(), SpiderError> {
    let Value::String(text) = value else {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: regex rule only supports text values",
            plan.name
        )));
    };

    let regex = Regex::new(pattern).map_err(|error| {
        SpiderError::parse(format!(
            "validation failed for field {}: invalid regex pattern {pattern}: {error}",
            plan.name
        ))
    })?;

    if regex.is_match(text) {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: value does not match regex {pattern}",
        plan.name
    )))
}

fn validate_min(plan: &ValidationPlan, value: &Value, min: f64) -> Result<(), SpiderError> {
    let (label, actual) = comparable_metric(plan, value, "min")?;
    if actual >= min {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: {label} must be >= {}",
        plan.name,
        format_number(min)
    )))
}

fn validate_max(plan: &ValidationPlan, value: &Value, max: f64) -> Result<(), SpiderError> {
    let (label, actual) = comparable_metric(plan, value, "max")?;
    if actual <= max {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: {label} must be <= {}",
        plan.name,
        format_number(max)
    )))
}

fn comparable_metric<'a>(
    plan: &'a ValidationPlan,
    value: &'a Value,
    rule_name: &str,
) -> Result<(&'static str, f64), SpiderError> {
    match value {
        Value::Number(value) => Ok(("value", *value)),
        Value::String(value) => Ok(("text length", value.chars().count() as f64)),
        Value::Array(value) => Ok(("list length", value.len() as f64)),
        Value::Object(value) => Ok(("object size", value.len() as f64)),
        Value::Bool(_) => Err(SpiderError::parse(format!(
            "validation failed for field {}: {rule_name} rule is not supported for bool values",
            plan.name
        ))),
        Value::Null => Err(SpiderError::parse(format!(
            "validation failed for field {}: {rule_name} rule cannot be applied to null",
            plan.name
        ))),
    }
}

fn format_number(value: f64) -> String {
    let mut text = value.to_string();
    if text.ends_with(".0") {
        text.truncate(text.len() - 2);
    }
    text
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

    #[test]
    fn validate_fields_accepts_regex_min_max_and_enum_rules() {
        let fields = BTreeMap::from([
            ("title".to_string(), Value::String("post-2026".to_string())),
            ("count".to_string(), Value::Number(5.0)),
            ("kind".to_string(), Value::String("news".to_string())),
        ]);
        let plans = vec![
            ValidationPlan::new("title", ValidationType::Text)
                .with_regex(r"^post-\d{4}$")
                .with_min(4.0)
                .with_max(16.0),
            ValidationPlan::new("count", ValidationType::Number)
                .with_min(1.0)
                .with_max(10.0),
            ValidationPlan::new("kind", ValidationType::Text).with_enum([
                Value::String("news".to_string()),
                Value::String("notice".to_string()),
            ]),
        ];

        assert!(validate_fields(&fields, &plans).is_ok());
    }

    #[test]
    fn validate_fields_rejects_regex_mismatch() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("bad".to_string()))]);
        let plans =
            vec![ValidationPlan::new("title", ValidationType::Text).with_regex(r"^post-\d+$")];

        let error = validate_fields(&fields, &plans).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field title: value does not match regex ^post-\\d+$"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_fields_rejects_number_below_min() {
        let fields = BTreeMap::from([("count".to_string(), Value::Number(1.0))]);
        let plans = vec![ValidationPlan::new("count", ValidationType::Number).with_min(3.0)];

        let error = validate_fields(&fields, &plans).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse("validation failed for field count: value must be >= 3".to_string())
        );
    }

    #[test]
    fn validate_fields_rejects_text_above_max_length() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("abcdef".to_string()))]);
        let plans = vec![ValidationPlan::new("title", ValidationType::Text).with_max(3.0)];

        let error = validate_fields(&fields, &plans).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field title: text length must be <= 3".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_rejects_value_outside_enum() {
        let fields = BTreeMap::from([("kind".to_string(), Value::String("blog".to_string()))]);
        let plans = vec![
            ValidationPlan::new("kind", ValidationType::Text).with_enum([
                Value::String("news".to_string()),
                Value::String("notice".to_string()),
            ]),
        ];

        let error = validate_fields(&fields, &plans).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field kind: value is not in enum set".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_rejects_invalid_regex_pattern() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("post-1".to_string()))]);
        let plans = vec![ValidationPlan::new("title", ValidationType::Text).with_regex("(")];

        let error = validate_fields(&fields, &plans).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("validation failed for field title: invalid regex pattern")
        );
    }
}
