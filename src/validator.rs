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
pub struct Validation {
    pub field: String,
    pub value_type: ValidationType,
    pub rule: ValidationRule,
}

impl Validation {
    pub fn new(field: impl Into<String>, value_type: ValidationType) -> Self {
        Self {
            field: field.into(),
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
    validations: &[Validation],
) -> Result<(), SpiderError> {
    for validation in validations {
        let resolved_values = resolve_field_values(fields, validation.field.as_str())?;
        validate_field(validation, &resolved_values)?;
    }

    Ok(())
}

pub fn validate_item(item: &Item, validations: &[Validation]) -> Result<(), SpiderError> {
    validate_fields(&item.fields, validations)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldPathStep {
    Field(String),
    Index(usize),
    Each,
}

#[derive(Clone, Copy)]
enum FieldPathNode<'a> {
    Root(&'a BTreeMap<String, Value>),
    Value(&'a Value),
    Missing,
}

#[derive(Clone)]
struct ResolvedFieldValue<'a> {
    field: String,
    value: Option<&'a Value>,
}

struct FieldPathCursor<'a> {
    field: String,
    node: FieldPathNode<'a>,
}

fn validate_field(
    validation: &Validation,
    resolved_values: &[ResolvedFieldValue<'_>],
) -> Result<(), SpiderError> {
    if resolved_values.is_empty() {
        if validation.rule.required {
            return Err(missing_value_error(validation.field.as_str()));
        }
        return Ok(());
    }

    for resolved in resolved_values {
        let Some(value) = resolved.value.filter(|value| !matches!(value, Value::Null)) else {
            if validation.rule.required {
                return Err(missing_value_error(resolved.field.as_str()));
            }
            continue;
        };

        if validation.value_type.matches(value) {
            validate_rule(validation, resolved.field.as_str(), value)?;
            continue;
        }

        return Err(SpiderError::parse(format!(
            "validation failed for field {}: expected {}",
            resolved.field,
            validation.value_type.as_str()
        )));
    }

    Ok(())
}

fn validate_rule(validation: &Validation, field: &str, value: &Value) -> Result<(), SpiderError> {
    if let Some(pattern) = validation.rule.regex.as_deref() {
        validate_regex(field, value, pattern)?;
    }

    if let Some(min) = validation.rule.min {
        validate_min(field, value, min)?;
    }

    if let Some(max) = validation.rule.max {
        validate_max(field, value, max)?;
    }

    if !validation.rule.enum_values.is_empty()
        && !validation.rule.enum_values.iter().any(|item| item == value)
    {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: value is not in enum set",
            field
        )));
    }

    Ok(())
}

fn validate_regex(field: &str, value: &Value, pattern: &str) -> Result<(), SpiderError> {
    let Value::String(text) = value else {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: regex rule only supports text values",
            field
        )));
    };

    let regex = Regex::new(pattern).map_err(|error| {
        SpiderError::parse(format!(
            "validation failed for field {}: invalid regex pattern {pattern}: {error}",
            field
        ))
    })?;

    if regex.is_match(text) {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: value does not match regex {pattern}",
        field
    )))
}

fn validate_min(field: &str, value: &Value, min: f64) -> Result<(), SpiderError> {
    let (label, actual) = comparable_metric(field, value, "min")?;
    if actual >= min {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: {label} must be >= {}",
        field,
        format_number(min)
    )))
}

fn validate_max(field: &str, value: &Value, max: f64) -> Result<(), SpiderError> {
    let (label, actual) = comparable_metric(field, value, "max")?;
    if actual <= max {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: {label} must be <= {}",
        field,
        format_number(max)
    )))
}

fn comparable_metric<'a>(
    field: &'a str,
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
            field
        ))),
        Value::Null => Err(SpiderError::parse(format!(
            "validation failed for field {}: {rule_name} rule cannot be applied to null",
            field
        ))),
    }
}

fn resolve_field_values<'a>(
    fields: &'a BTreeMap<String, Value>,
    field: &str,
) -> Result<Vec<ResolvedFieldValue<'a>>, SpiderError> {
    let steps = parse_field_path(field)?;
    let mut cursors = vec![FieldPathCursor {
        field: String::new(),
        node: FieldPathNode::Root(fields),
    }];

    for step in &steps {
        cursors = apply_field_path_step(field, cursors, step)?;
    }

    Ok(cursors
        .into_iter()
        .map(|cursor| ResolvedFieldValue {
            field: if cursor.field.is_empty() {
                field.to_string()
            } else {
                cursor.field
            },
            value: match cursor.node {
                FieldPathNode::Value(value) => Some(value),
                FieldPathNode::Root(_) | FieldPathNode::Missing => None,
            },
        })
        .collect())
}

fn parse_field_path(field: &str) -> Result<Vec<FieldPathStep>, SpiderError> {
    let bytes = field.as_bytes();
    let mut index = 0;
    let mut steps = Vec::new();

    while index < bytes.len() {
        if bytes[index] == b'.' {
            return Err(invalid_field_path_error(
                field,
                "unexpected `.`; field path segments cannot be empty",
            ));
        }

        let start = index;
        while index < bytes.len() && bytes[index] != b'.' && bytes[index] != b'[' {
            index += 1;
        }

        if start == index {
            return Err(invalid_field_path_error(
                field,
                "expected field name before array selector",
            ));
        }

        steps.push(FieldPathStep::Field(field[start..index].to_string()));

        while index < bytes.len() && bytes[index] == b'[' {
            index += 1;

            if index < bytes.len() && bytes[index] == b']' {
                steps.push(FieldPathStep::Each);
                index += 1;
                continue;
            }

            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }

            if start == index {
                return Err(invalid_field_path_error(
                    field,
                    "array selector must be `[]` or `[index]`",
                ));
            }

            if index >= bytes.len() || bytes[index] != b']' {
                return Err(invalid_field_path_error(
                    field,
                    "array selector is missing closing `]`",
                ));
            }

            let array_index = field[start..index].parse::<usize>().map_err(|error| {
                invalid_field_path_error(field, &format!("invalid array index: {error}"))
            })?;
            steps.push(FieldPathStep::Index(array_index));
            index += 1;
        }

        if index == bytes.len() {
            break;
        }

        if bytes[index] != b'.' {
            return Err(invalid_field_path_error(
                field,
                "field path contains unsupported syntax",
            ));
        }

        index += 1;
        if index == bytes.len() {
            return Err(invalid_field_path_error(
                field,
                "field path cannot end with `.`",
            ));
        }
    }

    Ok(steps)
}

fn apply_field_path_step<'a>(
    original_field: &str,
    cursors: Vec<FieldPathCursor<'a>>,
    step: &FieldPathStep,
) -> Result<Vec<FieldPathCursor<'a>>, SpiderError> {
    let mut next = Vec::new();

    for cursor in cursors {
        match step {
            FieldPathStep::Field(name) => match cursor.node {
                FieldPathNode::Root(fields) => next.push(FieldPathCursor {
                    field: append_field_path(cursor.field.as_str(), name),
                    node: fields
                        .get(name)
                        .map(FieldPathNode::Value)
                        .unwrap_or(FieldPathNode::Missing),
                }),
                FieldPathNode::Value(Value::Object(fields)) => next.push(FieldPathCursor {
                    field: append_field_path(cursor.field.as_str(), name),
                    node: fields
                        .get(name)
                        .map(FieldPathNode::Value)
                        .unwrap_or(FieldPathNode::Missing),
                }),
                FieldPathNode::Value(Value::Null) | FieldPathNode::Missing => {
                    next.push(FieldPathCursor {
                        field: append_field_path(cursor.field.as_str(), name),
                        node: FieldPathNode::Missing,
                    });
                }
                FieldPathNode::Value(value) => {
                    return Err(invalid_field_navigation_error(
                        original_field,
                        cursor.field.as_str(),
                        &format!("field `{name}`"),
                        value,
                    ));
                }
            },
            FieldPathStep::Index(array_index) => match cursor.node {
                FieldPathNode::Value(Value::Array(values)) => next.push(FieldPathCursor {
                    field: append_index_path(cursor.field.as_str(), *array_index),
                    node: values
                        .get(*array_index)
                        .map(FieldPathNode::Value)
                        .unwrap_or(FieldPathNode::Missing),
                }),
                FieldPathNode::Value(Value::Null) | FieldPathNode::Missing => {
                    next.push(FieldPathCursor {
                        field: append_index_path(cursor.field.as_str(), *array_index),
                        node: FieldPathNode::Missing,
                    });
                }
                FieldPathNode::Root(_) => {
                    return Err(invalid_field_path_error(
                        original_field,
                        "array selector must follow a field name",
                    ));
                }
                FieldPathNode::Value(value) => {
                    return Err(invalid_field_navigation_error(
                        original_field,
                        cursor.field.as_str(),
                        &format!("index [{}]", array_index),
                        value,
                    ));
                }
            },
            FieldPathStep::Each => match cursor.node {
                FieldPathNode::Value(Value::Array(values)) => {
                    for (array_index, value) in values.iter().enumerate() {
                        next.push(FieldPathCursor {
                            field: append_index_path(cursor.field.as_str(), array_index),
                            node: FieldPathNode::Value(value),
                        });
                    }
                }
                FieldPathNode::Value(Value::Null) | FieldPathNode::Missing => {
                    next.push(FieldPathCursor {
                        field: append_each_path(cursor.field.as_str()),
                        node: FieldPathNode::Missing,
                    });
                }
                FieldPathNode::Root(_) => {
                    return Err(invalid_field_path_error(
                        original_field,
                        "array selector must follow a field name",
                    ));
                }
                FieldPathNode::Value(value) => {
                    return Err(invalid_field_navigation_error(
                        original_field,
                        cursor.field.as_str(),
                        "items `[]`",
                        value,
                    ));
                }
            },
        }
    }

    Ok(next)
}

fn append_field_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        return name.to_string();
    }

    format!("{base}.{name}")
}

fn append_index_path(base: &str, index: usize) -> String {
    format!("{base}[{index}]")
}

fn append_each_path(base: &str) -> String {
    format!("{base}[]")
}

fn missing_value_error(field: &str) -> SpiderError {
    SpiderError::parse(format!(
        "validation failed for field {}: value is required",
        field
    ))
}

fn invalid_field_path_error(field: &str, detail: &str) -> SpiderError {
    SpiderError::parse(format!(
        "validation failed for field {}: invalid field path: {}",
        field, detail
    ))
}

fn invalid_field_navigation_error(
    original_field: &str,
    current_field: &str,
    next_step: &str,
    value: &Value,
) -> SpiderError {
    SpiderError::parse(format!(
        "validation failed for field {}: cannot access {} from {} at {}",
        original_field,
        next_step,
        value_kind_name(value),
        current_field
    ))
}

fn value_kind_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "text",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
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
        let validations = vec![
            Validation::new("title", ValidationType::Text).with_required(true),
            Validation::new("published", ValidationType::Bool).with_required(true),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_rejects_missing_required_field() {
        let fields = BTreeMap::new();
        let validations = vec![Validation::new("title", ValidationType::Text).with_required(true)];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse("validation failed for field title: value is required".to_string())
        );
    }

    #[test]
    fn validate_fields_rejects_type_mismatch() {
        let fields = BTreeMap::from([("title".to_string(), Value::Number(1.0))]);
        let validations = vec![Validation::new("title", ValidationType::Text).with_required(true)];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse("validation failed for field title: expected text".to_string())
        );
    }

    #[test]
    fn validate_item_uses_same_validation_semantics() {
        let item = Item::new().with_field("count", Value::Number(3.0));
        let validations =
            vec![Validation::new("count", ValidationType::Number).with_required(true)];

        assert!(validate_item(&item, &validations).is_ok());
    }

    #[test]
    fn validate_fields_accepts_regex_min_max_and_enum_rules() {
        let fields = BTreeMap::from([
            ("title".to_string(), Value::String("post-2026".to_string())),
            ("count".to_string(), Value::Number(5.0)),
            ("kind".to_string(), Value::String("news".to_string())),
        ]);
        let validations = vec![
            Validation::new("title", ValidationType::Text)
                .with_regex(r"^post-\d{4}$")
                .with_min(4.0)
                .with_max(16.0),
            Validation::new("count", ValidationType::Number)
                .with_min(1.0)
                .with_max(10.0),
            Validation::new("kind", ValidationType::Text).with_enum([
                Value::String("news".to_string()),
                Value::String("notice".to_string()),
            ]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_rejects_regex_mismatch() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("bad".to_string()))]);
        let validations =
            vec![Validation::new("title", ValidationType::Text).with_regex(r"^post-\d+$")];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
        let validations = vec![Validation::new("count", ValidationType::Number).with_min(3.0)];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse("validation failed for field count: value must be >= 3".to_string())
        );
    }

    #[test]
    fn validate_fields_rejects_text_above_max_length() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("abcdef".to_string()))]);
        let validations = vec![Validation::new("title", ValidationType::Text).with_max(3.0)];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
        let validations = vec![Validation::new("kind", ValidationType::Text).with_enum([
            Value::String("news".to_string()),
            Value::String("notice".to_string()),
        ])];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
        let validations = vec![Validation::new("title", ValidationType::Text).with_regex("(")];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("validation failed for field title: invalid regex pattern")
        );
    }

    #[test]
    fn validate_fields_accepts_nested_object_field_path() {
        let fields = BTreeMap::from([(
            "meta".to_string(),
            Value::Object(BTreeMap::from([(
                "title".to_string(),
                Value::String("hello".to_string()),
            )])),
        )]);
        let validations =
            vec![Validation::new("meta.title", ValidationType::Text).with_required(true)];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_accepts_indexed_array_field_path() {
        let fields = BTreeMap::from([(
            "authors".to_string(),
            Value::Array(vec![
                Value::Object(BTreeMap::from([(
                    "name".to_string(),
                    Value::String("alice".to_string()),
                )])),
                Value::Object(BTreeMap::from([(
                    "name".to_string(),
                    Value::String("bob".to_string()),
                )])),
            ]),
        )]);
        let validations =
            vec![Validation::new("authors[0].name", ValidationType::Text).with_required(true)];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_accepts_each_array_member_path() {
        let fields = BTreeMap::from([(
            "tags".to_string(),
            Value::Array(vec![
                Value::String("news".to_string()),
                Value::String("policy".to_string()),
            ]),
        )]);
        let validations = vec![Validation::new("tags[]", ValidationType::Text).with_required(true)];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_rejects_missing_nested_value_with_concrete_array_path() {
        let fields = BTreeMap::from([(
            "articles".to_string(),
            Value::Array(vec![
                Value::Object(BTreeMap::from([(
                    "title".to_string(),
                    Value::String("first".to_string()),
                )])),
                Value::Object(BTreeMap::new()),
            ]),
        )]);
        let validations =
            vec![Validation::new("articles[].title", ValidationType::Text).with_required(true)];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field articles[1].title: value is required".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_rejects_invalid_field_navigation_on_scalar_value() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("hello".to_string()))]);
        let validations =
            vec![Validation::new("title.name", ValidationType::Text).with_required(true)];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field title.name: cannot access field `name` from text at title"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_fields_rejects_invalid_field_path_syntax() {
        let fields = BTreeMap::from([(
            "meta".to_string(),
            Value::Object(BTreeMap::from([(
                "title".to_string(),
                Value::String("hello".to_string()),
            )])),
        )]);
        let validations = vec![Validation::new("meta..title", ValidationType::Text)];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field meta..title: invalid field path: unexpected `.`; field path segments cannot be empty".to_string()
            )
        );
    }
}
