use crate::error::SpiderError;
use crate::item::Item;
use crate::value::Value;
use jiff::{
    Timestamp, Zoned,
    civil::{Date, DateTime},
};
use regex::Regex;
use std::collections::BTreeMap;

const KNOWN_CIVIL_DATETIME_FORMATS: &[&str] = &[
    "%F %H:%M:%S",
    "%F %H:%M",
    "%Y/%m/%d %H:%M:%S",
    "%Y/%m/%d %H:%M",
];

const KNOWN_CIVIL_DATE_FORMATS: &[&str] = &["%Y/%m/%d", "%Y.%m.%d"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Text,
    Number,
    Bool,
    List,
    Object,
}

impl Type {
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

impl TryFrom<&str> for Type {
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
pub struct Rule {
    pub required: bool,
    pub regex: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub min_items: Option<usize>,
    pub max_items: Option<usize>,
    pub min_fields: Option<usize>,
    pub max_fields: Option<usize>,
    pub required_fields: Vec<String>,
    pub enum_values: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    Trim,
    NormalizeWhitespace,
    ParseNumber,
    ParseBool,
    ParseDatetime,
}

impl Transform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trim => "trim",
            Self::NormalizeWhitespace => "normalize_whitespace",
            Self::ParseNumber => "parse_number",
            Self::ParseBool => "parse_bool",
            Self::ParseDatetime => "parse_datetime",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionEnum {
    Exists,
    Missing,
    Equals,
    NotEquals,
}

impl ConditionEnum {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exists => "exists",
            Self::Missing => "missing",
            Self::Equals => "equals",
            Self::NotEquals => "not_equals",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub field: String,
    pub kind: ConditionEnum,
    pub value: Option<Value>,
}

impl Condition {
    pub fn exists(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            kind: ConditionEnum::Exists,
            value: None,
        }
    }

    pub fn missing(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            kind: ConditionEnum::Missing,
            value: None,
        }
    }

    pub fn equals(field: impl Into<String>, value: Value) -> Self {
        Self {
            field: field.into(),
            kind: ConditionEnum::Equals,
            value: Some(value),
        }
    }

    pub fn not_equals(field: impl Into<String>, value: Value) -> Self {
        Self {
            field: field.into(),
            kind: ConditionEnum::NotEquals,
            value: Some(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationEnum {
    And,
    Or,
    One,
}

impl RelationEnum {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::And => "and",
            Self::Or => "or",
            Self::One => "one",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub fields: Vec<String>,
    pub kind: RelationEnum,
}

impl Relation {
    pub fn new<I, S>(fields: I, kind: RelationEnum) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            kind,
            fields: fields.into_iter().map(Into::into).collect(),
        }
    }

    pub fn and<I, S>(fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(fields, RelationEnum::And)
    }

    pub fn or<I, S>(fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(fields, RelationEnum::Or)
    }

    pub fn one<I, S>(fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(fields, RelationEnum::One)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldValidator {
    pub field: String,
    pub value_type: Type,
    pub transforms: Vec<Transform>,
    pub conditions: Vec<Condition>,
    pub object_fields: Vec<FieldValidator>,
    pub each_fields: Vec<FieldValidator>,
    pub rule: Rule,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepValidator {
    pub fields: Vec<FieldValidator>,
    pub relations: Vec<Relation>,
}

impl StepValidator {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_fields(fields: Vec<FieldValidator>) -> Self {
        fields.into_iter().fold(Self::new(), |step, field| {
            let name = field.field.clone();
            let value_type = field.value_type;
            step.field(name, value_type, |_| field)
        })
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.relations.is_empty()
    }

    pub fn field<F>(mut self, name: impl Into<String>, value_type: Type, build: F) -> Self
    where
        F: FnOnce(FieldValidator) -> FieldValidator,
    {
        self.fields
            .push(build(FieldValidator::new(name, value_type)));
        self
    }

    pub fn and<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.relations
            .push(Relation::new(fields, RelationEnum::And));
        self
    }

    pub fn or<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.relations.push(Relation::new(fields, RelationEnum::Or));
        self
    }

    pub fn one<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.relations
            .push(Relation::new(fields, RelationEnum::One));
        self
    }

    pub async fn validate(&self, item: &Item) -> Result<(), SpiderError> {
        if !self.fields.is_empty() {
            validate_item(item, &self.fields)?;
        }

        if !self.relations.is_empty() {
            validate_item_relations(item, &self.relations)?;
        }

        Ok(())
    }
}

pub fn field(field: impl Into<String>, value_type: Type) -> FieldValidator {
    FieldValidator::new(field, value_type)
}

impl FieldValidator {
    pub fn root() -> Self {
        Self::new("", Type::Object)
    }

    pub fn new(field: impl Into<String>, value_type: Type) -> Self {
        Self {
            field: field.into(),
            value_type,
            transforms: Vec::new(),
            conditions: Vec::new(),
            object_fields: Vec::new(),
            each_fields: Vec::new(),
            rule: Rule::default(),
        }
    }

    pub fn transform(mut self, transform: Transform) -> Self {
        self.transforms.push(transform);
        self
    }

    pub fn transforms(mut self, transforms: impl IntoIterator<Item = Transform>) -> Self {
        self.transforms.extend(transforms);
        self
    }

    pub fn condition(mut self, condition: Condition) -> Self {
        self.conditions.push(condition);
        self
    }

    pub fn conditions(mut self, conditions: impl IntoIterator<Item = Condition>) -> Self {
        self.conditions.extend(conditions);
        self
    }

    pub fn apply_when_exists(mut self, field: impl Into<String>) -> Self {
        self.conditions.push(Condition::exists(field));
        self
    }

    pub fn apply_when_missing(mut self, field: impl Into<String>) -> Self {
        self.conditions.push(Condition::missing(field));
        self
    }

    pub fn apply_when_equals(mut self, field: impl Into<String>, value: Value) -> Self {
        self.conditions.push(Condition::equals(field, value));
        self
    }

    pub fn apply_when_not_equals(mut self, field: impl Into<String>, value: Value) -> Self {
        self.conditions.push(Condition::not_equals(field, value));
        self
    }

    pub fn object_fields(mut self, fields: impl IntoIterator<Item = FieldValidator>) -> Self {
        self.object_fields.extend(fields);
        self
    }

    pub fn each_fields(mut self, fields: impl IntoIterator<Item = FieldValidator>) -> Self {
        self.each_fields.extend(fields);
        self
    }

    pub fn required(mut self) -> Self {
        self.rule.required = true;
        self
    }

    pub fn optional(mut self) -> Self {
        self.rule.required = false;
        self
    }

    pub fn regex(mut self, pattern: impl Into<String>) -> Self {
        self.rule.regex = Some(pattern.into());
        self
    }

    pub fn min(mut self, min: f64) -> Self {
        self.rule.min = Some(min);
        self
    }

    pub fn max(mut self, max: f64) -> Self {
        self.rule.max = Some(max);
        self
    }

    pub fn min_length(mut self, min_length: usize) -> Self {
        self.rule.min_length = Some(min_length);
        self
    }

    pub fn max_length(mut self, max_length: usize) -> Self {
        self.rule.max_length = Some(max_length);
        self
    }

    pub fn min_items(mut self, min_items: usize) -> Self {
        self.rule.min_items = Some(min_items);
        self
    }

    pub fn max_items(mut self, max_items: usize) -> Self {
        self.rule.max_items = Some(max_items);
        self
    }

    pub fn min_fields(mut self, min_fields: usize) -> Self {
        self.rule.min_fields = Some(min_fields);
        self
    }

    pub fn max_fields(mut self, max_fields: usize) -> Self {
        self.rule.max_fields = Some(max_fields);
        self
    }

    pub fn required_fields<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.rule.required_fields = fields.into_iter().map(Into::into).collect();
        self
    }

    pub fn enum_values(mut self, values: impl IntoIterator<Item = Value>) -> Self {
        self.rule.enum_values = values.into_iter().collect();
        self
    }
}

pub fn validate_fields(
    fields: &BTreeMap<String, Value>,
    validators: &[FieldValidator],
) -> Result<(), SpiderError> {
    let mut collector = ValidatorCollector::fail_fast();
    validate_fields_internal(fields, validators, &mut collector)
}

pub fn validate_item(item: &Item, validators: &[FieldValidator]) -> Result<(), SpiderError> {
    validate_fields(&item.fields, validators)
}

pub fn validate_field_relations(
    fields: &BTreeMap<String, Value>,
    relations: &[Relation],
) -> Result<(), SpiderError> {
    for relation in relations {
        validate_relation(fields, relation)?;
    }

    Ok(())
}

pub fn validate_item_relations(item: &Item, relations: &[Relation]) -> Result<(), SpiderError> {
    validate_field_relations(&item.fields, relations)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidatorStatus {
    Skipped,
    Applied,
}

struct ValidatorCollector;

impl ValidatorCollector {
    fn fail_fast() -> Self {
        Self
    }

    fn record_error(&mut self, error: SpiderError) -> Result<(), SpiderError> {
        Err(error)
    }
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

fn validate_fields_internal(
    fields: &BTreeMap<String, Value>,
    validators: &[FieldValidator],
    collector: &mut ValidatorCollector,
) -> Result<(), SpiderError> {
    let root_value = Value::Object(fields.clone());

    for validation in validators {
        if validation.field.is_empty() {
            let resolved_values = vec![ResolvedFieldValue {
                field: display_validator_field("").to_string(),
                value: Some(&root_value),
            }];
            validate_field(validation, "", &root_value, &resolved_values, collector)?;
            continue;
        }

        let resolved_values = match resolve_field_values(fields, validation.field.as_str()) {
            Ok(resolved_values) => resolved_values,
            Err(error) => {
                collector.record_error(error)?;
                continue;
            }
        };
        validate_field(validation, "", &root_value, &resolved_values, collector)?;
    }

    Ok(())
}

fn validate_field(
    validation: &FieldValidator,
    scope_field: &str,
    scope_value: &Value,
    resolved_values: &[ResolvedFieldValue<'_>],
    collector: &mut ValidatorCollector,
) -> Result<ValidatorStatus, SpiderError> {
    match validator_conditions_match(scope_field, scope_value, &validation.conditions) {
        Ok(false) => return Ok(ValidatorStatus::Skipped),
        Ok(true) => {}
        Err(error) => {
            collector.record_error(error)?;
            return Ok(ValidatorStatus::Applied);
        }
    }

    if resolved_values.is_empty() {
        if validation.rule.required {
            collector.record_error(missing_value_error(validation.field.as_str()))?;
            return Ok(ValidatorStatus::Applied);
        }
        return Ok(ValidatorStatus::Skipped);
    }

    let mut status = ValidatorStatus::Skipped;
    for resolved in resolved_values {
        let resolved_status = validate_resolved_value(
            validation,
            resolved.field.as_str(),
            resolved.value,
            collector,
        )?;
        if resolved_status == ValidatorStatus::Applied {
            status = ValidatorStatus::Applied;
        }
    }

    Ok(status)
}

fn validate_resolved_value(
    validation: &FieldValidator,
    resolved_field: &str,
    resolved_value: Option<&Value>,
    collector: &mut ValidatorCollector,
) -> Result<ValidatorStatus, SpiderError> {
    let Some(value) = resolved_value.filter(|value| !matches!(value, Value::Null)) else {
        if validation.rule.required {
            collector.record_error(missing_value_error(resolved_field))?;
            return Ok(ValidatorStatus::Applied);
        }
        return Ok(ValidatorStatus::Skipped);
    };

    let transformed_value = if validation.transforms.is_empty() {
        None
    } else {
        match apply_validator_transforms(resolved_field, value.clone(), &validation.transforms) {
            Ok(value) => Some(value),
            Err(error) => {
                collector.record_error(error)?;
                return Ok(ValidatorStatus::Applied);
            }
        }
    };
    let value = transformed_value.as_ref().unwrap_or(value);

    if !validation.value_type.matches(value) {
        collector.record_error(SpiderError::parse(format!(
            "validation failed for field {}: expected {}",
            display_validator_field(resolved_field),
            validation.value_type.as_str()
        )))?;
        return Ok(ValidatorStatus::Applied);
    }

    if let Err(error) = validate_rule(validation, resolved_field, value) {
        collector.record_error(error)?;
        return Ok(ValidatorStatus::Applied);
    }

    validate_nested_validators(validation, resolved_field, value, collector)?;
    Ok(ValidatorStatus::Applied)
}

fn validate_nested_validators(
    validation: &FieldValidator,
    field: &str,
    value: &Value,
    collector: &mut ValidatorCollector,
) -> Result<(), SpiderError> {
    if !validation.object_fields.is_empty() {
        validate_object_fields(field, value, &validation.object_fields, collector)?;
    }

    if !validation.each_fields.is_empty() {
        validate_each_fields(field, value, &validation.each_fields, collector)?;
    }

    Ok(())
}

fn validate_object_fields(
    field: &str,
    value: &Value,
    fields: &[FieldValidator],
    collector: &mut ValidatorCollector,
) -> Result<(), SpiderError> {
    validate_scoped_validators(
        field,
        value,
        fields,
        collector,
        "object fields only support object values",
    )
}

fn validate_each_fields(
    field: &str,
    value: &Value,
    fields: &[FieldValidator],
    collector: &mut ValidatorCollector,
) -> Result<(), SpiderError> {
    let Value::Array(items) = value else {
        collector.record_error(SpiderError::parse(format!(
            "validation failed for field {}: each fields only support list values",
            field
        )))?;
        return Ok(());
    };

    for (index, item) in items.iter().enumerate() {
        let item_field = append_index_path(field, index);
        for validation in fields {
            let resolved_values = match resolve_nested_field_values(
                item_field.as_str(),
                item,
                validation.field.as_str(),
            ) {
                Ok(resolved_values) => resolved_values,
                Err(error) => {
                    collector.record_error(error)?;
                    continue;
                }
            };
            validate_field(
                validation,
                item_field.as_str(),
                item,
                &resolved_values,
                collector,
            )?;
        }
    }

    Ok(())
}

fn validate_scoped_validators(
    scope_field: &str,
    scope_value: &Value,
    validators: &[FieldValidator],
    collector: &mut ValidatorCollector,
    non_object_message: &str,
) -> Result<(), SpiderError> {
    let Value::Object(_) = scope_value else {
        collector.record_error(SpiderError::parse(format!(
            "validation failed for field {}: {}",
            display_validator_field(scope_field),
            non_object_message
        )))?;
        return Ok(());
    };

    for validation in validators {
        let resolved_values = match resolve_scoped_field_values(
            scope_field,
            scope_value,
            validation.field.as_str(),
        ) {
            Ok(resolved_values) => resolved_values,
            Err(error) => {
                collector.record_error(error)?;
                continue;
            }
        };
        validate_field(
            validation,
            scope_field,
            scope_value,
            &resolved_values,
            collector,
        )?;
    }

    Ok(())
}

fn validate_relation(
    fields: &BTreeMap<String, Value>,
    relation: &Relation,
) -> Result<(), SpiderError> {
    if relation.fields.is_empty() {
        return Ok(());
    }

    let mut present = Vec::new();
    let mut missing = Vec::new();

    for field in &relation.fields {
        let resolved_values = resolve_field_values(fields, field)?;
        let has_value = resolved_values.iter().any(|resolved| {
            resolved
                .value
                .is_some_and(|value| !matches!(value, Value::Null))
        });

        if has_value {
            present.push(field.as_str());
        } else {
            missing.push(field.as_str());
        }
    }

    let count = present.len();
    let total = relation.fields.len();
    match relation.kind {
        RelationEnum::And if count == total => Ok(()),
        RelationEnum::Or if count >= 1 => Ok(()),
        RelationEnum::One if count == 1 => Ok(()),
        RelationEnum::And => Err(relation_error(
            relation,
            format!(
                "expected all fields to be present, missing: {}",
                missing.join(", ")
            ),
        )),
        RelationEnum::Or => Err(relation_error(
            relation,
            "expected at least one field to be present".to_string(),
        )),
        RelationEnum::One => Err(relation_error(
            relation,
            format!("expected exactly one field to be present, but got {count}"),
        )),
    }
}

fn relation_error(relation: &Relation, message: String) -> SpiderError {
    SpiderError::parse(format!(
        "validation failed for field {}: relation {} {}",
        relation_label(relation),
        relation.kind.as_str(),
        message
    ))
}

fn relation_label(relation: &Relation) -> String {
    format!("[{}]", relation.fields.join(", "))
}

fn validator_conditions_match(
    scope_field: &str,
    scope_value: &Value,
    conditions: &[Condition],
) -> Result<bool, SpiderError> {
    for condition in conditions {
        if !validator_condition_matches(scope_field, scope_value, condition)? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn validator_condition_matches(
    scope_field: &str,
    scope_value: &Value,
    condition: &Condition,
) -> Result<bool, SpiderError> {
    let resolved_values =
        resolve_scoped_field_values(scope_field, scope_value, condition.field.as_str())?;
    let present_values = resolved_values
        .iter()
        .filter_map(|resolved| resolved.value)
        .filter(|value| !matches!(value, Value::Null))
        .collect::<Vec<_>>();

    match condition.kind {
        ConditionEnum::Exists => Ok(!present_values.is_empty()),
        ConditionEnum::Missing => Ok(present_values.is_empty()),
        ConditionEnum::Equals => {
            let expected = validator_condition_expected_value(condition)?;
            Ok(present_values.iter().any(|value| *value == expected))
        }
        ConditionEnum::NotEquals => {
            let expected = validator_condition_expected_value(condition)?;
            Ok(!present_values.is_empty() && present_values.iter().all(|value| *value != expected))
        }
    }
}

fn validator_condition_expected_value(condition: &Condition) -> Result<&Value, SpiderError> {
    condition.value.as_ref().ok_or_else(|| {
        SpiderError::parse(format!(
            "validation condition {} on field {} requires a comparison value",
            condition.kind.as_str(),
            display_validator_field(condition.field.as_str())
        ))
    })
}

fn apply_validator_transforms(
    field: &str,
    mut value: Value,
    transforms: &[Transform],
) -> Result<Value, SpiderError> {
    for transform in transforms {
        value = apply_validator_transform(field, value, *transform)?;
    }

    Ok(value)
}

fn apply_validator_transform(
    field: &str,
    value: Value,
    transform: Transform,
) -> Result<Value, SpiderError> {
    match transform {
        Transform::Trim => trim_validator_value(field, value),
        Transform::NormalizeWhitespace => normalize_whitespace_validator_value(field, value),
        Transform::ParseNumber => parse_number_validator_value(field, value),
        Transform::ParseBool => parse_bool_validator_value(field, value),
        Transform::ParseDatetime => parse_datetime_validator_value(field, value),
    }
}

fn validate_rule(
    validation: &FieldValidator,
    field: &str,
    value: &Value,
) -> Result<(), SpiderError> {
    if let Some(pattern) = validation.rule.regex.as_deref() {
        validate_regex(field, value, pattern)?;
    }

    if let Some(min) = validation.rule.min {
        validate_min(field, value, min)?;
    }

    if let Some(max) = validation.rule.max {
        validate_max(field, value, max)?;
    }

    if let Some(min_length) = validation.rule.min_length {
        validate_min_length(field, value, min_length)?;
    }

    if let Some(max_length) = validation.rule.max_length {
        validate_max_length(field, value, max_length)?;
    }

    if let Some(min_items) = validation.rule.min_items {
        validate_min_items(field, value, min_items)?;
    }

    if let Some(max_items) = validation.rule.max_items {
        validate_max_items(field, value, max_items)?;
    }

    if let Some(min_fields) = validation.rule.min_fields {
        validate_min_fields(field, value, min_fields)?;
    }

    if let Some(max_fields) = validation.rule.max_fields {
        validate_max_fields(field, value, max_fields)?;
    }

    if !validation.rule.required_fields.is_empty() {
        validate_required_fields(field, value, &validation.rule.required_fields)?;
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

fn validate_min_length(field: &str, value: &Value, min_length: usize) -> Result<(), SpiderError> {
    let Value::String(text) = value else {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: min_length rule only supports text values",
            field
        )));
    };

    let length = text.chars().count();
    if length >= min_length {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: text length must be >= {}",
        field, min_length
    )))
}

fn validate_max_length(field: &str, value: &Value, max_length: usize) -> Result<(), SpiderError> {
    let Value::String(text) = value else {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: max_length rule only supports text values",
            field
        )));
    };

    let length = text.chars().count();
    if length <= max_length {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: text length must be <= {}",
        field, max_length
    )))
}

fn validate_min_items(field: &str, value: &Value, min_items: usize) -> Result<(), SpiderError> {
    let Value::Array(items) = value else {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: min_items rule only supports list values",
            field
        )));
    };

    if items.len() >= min_items {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: list length must be >= {}",
        field, min_items
    )))
}

fn validate_max_items(field: &str, value: &Value, max_items: usize) -> Result<(), SpiderError> {
    let Value::Array(items) = value else {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: max_items rule only supports list values",
            field
        )));
    };

    if items.len() <= max_items {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: list length must be <= {}",
        field, max_items
    )))
}

fn validate_min_fields(field: &str, value: &Value, min_fields: usize) -> Result<(), SpiderError> {
    let Value::Object(fields) = value else {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: min_fields rule only supports object values",
            field
        )));
    };

    if fields.len() >= min_fields {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: object size must be >= {}",
        field, min_fields
    )))
}

fn validate_max_fields(field: &str, value: &Value, max_fields: usize) -> Result<(), SpiderError> {
    let Value::Object(fields) = value else {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: max_fields rule only supports object values",
            field
        )));
    };

    if fields.len() <= max_fields {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: object size must be <= {}",
        field, max_fields
    )))
}

fn validate_required_fields(
    field: &str,
    value: &Value,
    required_fields: &[String],
) -> Result<(), SpiderError> {
    let Value::Object(fields) = value else {
        return Err(SpiderError::parse(format!(
            "validation failed for field {}: required_fields rule only supports object values",
            field
        )));
    };

    let missing = required_fields
        .iter()
        .filter(|required_field| {
            fields
                .get(required_field.as_str())
                .is_none_or(|value| matches!(value, Value::Null))
        })
        .cloned()
        .collect::<Vec<_>>();

    if missing.is_empty() {
        return Ok(());
    }

    Err(SpiderError::parse(format!(
        "validation failed for field {}: object is missing required fields: {}",
        field,
        missing.join(", ")
    )))
}

fn trim_validator_value(field: &str, value: Value) -> Result<Value, SpiderError> {
    let Value::String(text) = value else {
        return Err(transform_type_error(field, Transform::Trim, &value, "text"));
    };

    Ok(Value::String(text.trim().to_string()))
}

fn normalize_whitespace_validator_value(field: &str, value: Value) -> Result<Value, SpiderError> {
    let Value::String(text) = value else {
        return Err(transform_type_error(
            field,
            Transform::NormalizeWhitespace,
            &value,
            "text",
        ));
    };

    Ok(Value::String(normalize_whitespace_text(&text)))
}

fn parse_number_validator_value(field: &str, value: Value) -> Result<Value, SpiderError> {
    match value {
        Value::Number(value) => Ok(Value::Number(value)),
        Value::String(text) => {
            let normalized = text.trim();
            if normalized.is_empty() {
                return Err(transform_error(
                    field,
                    Transform::ParseNumber,
                    "empty string",
                ));
            }

            normalized
                .parse::<f64>()
                .map(Value::Number)
                .map_err(|error| transform_error(field, Transform::ParseNumber, &error.to_string()))
        }
        other => Err(transform_type_error(
            field,
            Transform::ParseNumber,
            &other,
            "string or number",
        )),
    }
}

fn parse_bool_validator_value(field: &str, value: Value) -> Result<Value, SpiderError> {
    match value {
        Value::Bool(value) => Ok(Value::Bool(value)),
        Value::String(text) => {
            let normalized = text.trim();
            if normalized.is_empty() {
                return Err(transform_error(field, Transform::ParseBool, "empty string"));
            }

            parse_bool_text(normalized).map(Value::Bool).ok_or_else(|| {
                transform_error(
                    field,
                    Transform::ParseBool,
                    &format!("expected true/false/1/0, got {normalized:?}"),
                )
            })
        }
        other => Err(transform_type_error(
            field,
            Transform::ParseBool,
            &other,
            "string or bool",
        )),
    }
}

fn parse_datetime_validator_value(field: &str, value: Value) -> Result<Value, SpiderError> {
    let Value::String(text) = value else {
        return Err(transform_type_error(
            field,
            Transform::ParseDatetime,
            &value,
            "text",
        ));
    };

    let normalized = text.trim();
    if normalized.is_empty() {
        return Err(transform_error(
            field,
            Transform::ParseDatetime,
            "empty string",
        ));
    }

    parse_validator_datetime_text(normalized)
        .map(Value::String)
        .map_err(|message| transform_error(field, Transform::ParseDatetime, &message))
}

fn parse_validator_datetime_text(text: &str) -> Result<String, String> {
    if let Ok(timestamp) = text.parse::<Timestamp>() {
        return Ok(timestamp.to_string());
    }

    if let Ok(zoned) = text.parse::<Zoned>() {
        return Ok(zoned.timestamp().to_string());
    }

    if let Ok(datetime) = text.parse::<DateTime>() {
        return Ok(datetime.to_string());
    }

    if let Ok(date) = text.parse::<Date>() {
        return Ok(date.to_string());
    }

    for format in KNOWN_CIVIL_DATETIME_FORMATS {
        if let Ok(datetime) = DateTime::strptime(format, text) {
            return Ok(datetime.to_string());
        }
    }

    for format in KNOWN_CIVIL_DATE_FORMATS {
        if let Ok(date) = Date::strptime(format, text) {
            return Ok(date.to_string());
        }
    }

    Err(format!("unsupported datetime format, got {text:?}"))
}

fn parse_bool_text(text: &str) -> Option<bool> {
    if text.eq_ignore_ascii_case("true") || text == "1" {
        Some(true)
    } else if text.eq_ignore_ascii_case("false") || text == "0" {
        Some(false)
    } else {
        None
    }
}

fn normalize_whitespace_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn transform_error(field: &str, transform: Transform, detail: &str) -> SpiderError {
    SpiderError::parse(format!(
        "validation failed for field {}: transform {} failed: {}",
        field,
        transform.as_str(),
        detail
    ))
}

fn transform_type_error(
    field: &str,
    transform: Transform,
    value: &Value,
    expected: &str,
) -> SpiderError {
    transform_error(
        field,
        transform,
        &format!("expected {expected}, got {}", value_kind_name(value)),
    )
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

fn resolve_nested_field_values<'a>(
    parent_field: &str,
    parent_value: &'a Value,
    child_field: &str,
) -> Result<Vec<ResolvedFieldValue<'a>>, SpiderError> {
    if child_field.is_empty() {
        return Ok(vec![ResolvedFieldValue {
            field: parent_field.to_string(),
            value: Some(parent_value),
        }]);
    }

    let Value::Object(fields) = parent_value else {
        let next_step = child_field
            .split(['.', '['])
            .next()
            .filter(|segment| !segment.is_empty())
            .map(|segment| format!("field `{segment}`"))
            .unwrap_or_else(|| "field".to_string());
        let original_field = prefix_nested_field(parent_field, child_field);

        return Err(invalid_field_navigation_error(
            original_field.as_str(),
            parent_field,
            next_step.as_str(),
            parent_value,
        ));
    };

    let resolved_values = resolve_field_values(fields, child_field)?;
    Ok(resolved_values
        .into_iter()
        .map(|resolved| ResolvedFieldValue {
            field: prefix_nested_field(parent_field, resolved.field.as_str()),
            value: resolved.value,
        })
        .collect())
}

fn resolve_scoped_field_values<'a>(
    scope_field: &str,
    scope_value: &'a Value,
    child_field: &str,
) -> Result<Vec<ResolvedFieldValue<'a>>, SpiderError> {
    if child_field.is_empty() {
        return Ok(vec![ResolvedFieldValue {
            field: display_validator_field(scope_field).to_string(),
            value: Some(scope_value),
        }]);
    }

    resolve_nested_field_values(scope_field, scope_value, child_field)
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

fn prefix_nested_field(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        return child.to_string();
    }

    if child.is_empty() {
        return parent.to_string();
    }

    format!("{parent}.{child}")
}

fn display_validator_field(field: &str) -> &str {
    if field.is_empty() { "$" } else { field }
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
        let validators = vec![
            FieldValidator::new("title", Type::Text).required(),
            FieldValidator::new("published", Type::Bool).required(),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_rejects_missing_required_field() {
        let fields = BTreeMap::new();
        let validators = vec![FieldValidator::new("title", Type::Text).required()];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse("validation failed for field title: value is required".to_string())
        );
    }

    #[test]
    fn validate_fields_rejects_type_mismatch() {
        let fields = BTreeMap::from([("title".to_string(), Value::Number(1.0))]);
        let validators = vec![FieldValidator::new("title", Type::Text).required()];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse("validation failed for field title: expected text".to_string())
        );
    }

    #[test]
    fn validate_fields_skips_missing_optional_field() {
        let fields = BTreeMap::new();
        let validators = vec![FieldValidator::new("title", Type::Text).min_length(3)];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_requires_field_when_required_when_equals_matches() {
        let fields = BTreeMap::from([("type".to_string(), Value::String("video".to_string()))]);
        let validators = vec![
            FieldValidator::new("duration", Type::Number)
                .required()
                .apply_when_equals("type", Value::String("video".to_string())),
        ];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field duration: value is required".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_skips_required_when_equals_when_condition_does_not_match() {
        let fields = BTreeMap::from([("type".to_string(), Value::String("article".to_string()))]);
        let validators = vec![
            FieldValidator::new("duration", Type::Number)
                .required()
                .apply_when_equals("type", Value::String("video".to_string())),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_applies_optional_validation_when_exists_condition_matches() {
        let fields = BTreeMap::from([
            ("title".to_string(), Value::String("Kun".to_string())),
            ("summary".to_string(), Value::String("bad".to_string())),
        ]);
        let validators = vec![
            FieldValidator::new("summary", Type::Text)
                .apply_when_exists("title")
                .min_length(5),
        ];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field summary: text length must be >= 5".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_skips_optional_validation_when_exists_condition_is_missing() {
        let fields = BTreeMap::from([("summary".to_string(), Value::String("bad".to_string()))]);
        let validators = vec![
            FieldValidator::new("summary", Type::Text)
                .apply_when_exists("title")
                .min_length(5),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_requires_field_when_required_when_missing_matches() {
        let fields = BTreeMap::from([(
            "asset".to_string(),
            Value::Object(BTreeMap::from([(
                "url".to_string(),
                Value::String("https://example.com/video.mp4".to_string()),
            )])),
        )]);
        let validators = vec![
            FieldValidator::new("asset", Type::Object).object_fields([FieldValidator::new(
                "checksum",
                Type::Text,
            )
            .required()
            .apply_when_missing("signature")]),
        ];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field asset.checksum: value is required".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_skips_required_when_missing_when_companion_field_exists() {
        let fields = BTreeMap::from([(
            "asset".to_string(),
            Value::Object(BTreeMap::from([(
                "signature".to_string(),
                Value::String("sha256:abc".to_string()),
            )])),
        )]);
        let validators = vec![
            FieldValidator::new("asset", Type::Object).object_fields([FieldValidator::new(
                "checksum",
                Type::Text,
            )
            .required()
            .apply_when_missing("signature")]),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_requires_field_when_required_when_not_equals_matches() {
        let fields = BTreeMap::from([("kind".to_string(), Value::String("news".to_string()))]);
        let validators = vec![
            FieldValidator::new("summary", Type::Text)
                .required()
                .apply_when_not_equals("kind", Value::String("redirect".to_string())),
        ];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field summary: value is required".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_skips_required_when_not_equals_when_condition_field_is_missing() {
        let fields = BTreeMap::new();
        let validators = vec![
            FieldValidator::new("summary", Type::Text)
                .required()
                .apply_when_not_equals("kind", Value::String("redirect".to_string())),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_accepts_number_transform_before_numeric_rules() {
        let fields = BTreeMap::from([("count".to_string(), Value::String(" 42 ".to_string()))]);
        let validators = vec![
            FieldValidator::new("count", Type::Number)
                .transform(Transform::ParseNumber)
                .min(10.0)
                .max(100.0),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_accepts_bool_transform_before_type_check() {
        let fields =
            BTreeMap::from([("published".to_string(), Value::String(" true ".to_string()))]);
        let validators =
            vec![FieldValidator::new("published", Type::Bool).transform(Transform::ParseBool)];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_accepts_datetime_transform_before_enum_check() {
        let fields = BTreeMap::from([(
            "published_at".to_string(),
            Value::String("2026-04-01T08:30:45+08:00".to_string()),
        )]);
        let validators = vec![
            FieldValidator::new("published_at", Type::Text)
                .transform(Transform::ParseDatetime)
                .enum_values([Value::String("2026-04-01T00:30:45Z".to_string())]),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_accepts_trim_and_normalize_whitespace_before_regex() {
        let fields = BTreeMap::from([(
            "title".to_string(),
            Value::String("  Hello   Kun  ".to_string()),
        )]);
        let validators = vec![
            FieldValidator::new("title", Type::Text)
                .transforms([Transform::Trim, Transform::NormalizeWhitespace])
                .regex(r"^Hello Kun$"),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_rejects_failed_number_transform() {
        let fields =
            BTreeMap::from([("count".to_string(), Value::String("forty-two".to_string()))]);
        let validators =
            vec![FieldValidator::new("count", Type::Number).transform(Transform::ParseNumber)];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field count: transform parse_number failed: invalid float literal"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_fields_rejects_transform_type_mismatch() {
        let fields = BTreeMap::from([("count".to_string(), Value::Bool(true))]);
        let validators =
            vec![FieldValidator::new("count", Type::Number).transform(Transform::ParseNumber)];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field count: transform parse_number failed: expected string or number, got bool"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_fields_accepts_object_fields() {
        let fields = BTreeMap::from([(
            "meta".to_string(),
            Value::Object(BTreeMap::from([
                (
                    "title".to_string(),
                    Value::String("  Kun Weekly  ".to_string()),
                ),
                (
                    "published_at".to_string(),
                    Value::String("2026-04-01T08:30:45+08:00".to_string()),
                ),
            ])),
        )]);
        let validators = vec![
            FieldValidator::new("meta", Type::Object)
                .required_fields(["title", "published_at"])
                .object_fields([
                    FieldValidator::new("title", Type::Text)
                        .transform(Transform::Trim)
                        .min_length(3),
                    FieldValidator::new("published_at", Type::Text)
                        .transform(Transform::ParseDatetime)
                        .enum_values([Value::String("2026-04-01T00:30:45Z".to_string())]),
                ]),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_skips_missing_optional_nested_object_field() {
        let fields = BTreeMap::from([("meta".to_string(), Value::Object(BTreeMap::new()))]);
        let validators = vec![
            FieldValidator::new("meta", Type::Object).object_fields([FieldValidator::new(
                "subtitle",
                Type::Text,
            )
            .min_length(3)]),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_applies_conditions_relative_to_nested_object_scope() {
        let fields = BTreeMap::from([(
            "meta".to_string(),
            Value::Object(BTreeMap::from([(
                "type".to_string(),
                Value::String("video".to_string()),
            )])),
        )]);
        let validators = vec![
            FieldValidator::new("meta", Type::Object).object_fields([FieldValidator::new(
                "duration",
                Type::Number,
            )
            .required()
            .apply_when_equals("type", Value::String("video".to_string()))]),
        ];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field meta.duration: value is required".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_accepts_each_fields_for_object_items() {
        let fields = BTreeMap::from([(
            "articles".to_string(),
            Value::Array(vec![
                Value::Object(BTreeMap::from([
                    (
                        "title".to_string(),
                        Value::String("  First item ".to_string()),
                    ),
                    ("score".to_string(), Value::String("10".to_string())),
                ])),
                Value::Object(BTreeMap::from([
                    (
                        "title".to_string(),
                        Value::String("Second item".to_string()),
                    ),
                    ("score".to_string(), Value::String("20".to_string())),
                ])),
            ]),
        )]);
        let validators = vec![
            FieldValidator::new("articles", Type::List)
                .min_items(2)
                .each_fields([
                    FieldValidator::new("title", Type::Text)
                        .transform(Transform::Trim)
                        .min_length(5),
                    FieldValidator::new("score", Type::Number)
                        .transform(Transform::ParseNumber)
                        .min(10.0),
                ]),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_accepts_each_fields_for_scalar_items() {
        let fields = BTreeMap::from([(
            "tags".to_string(),
            Value::Array(vec![
                Value::String("  news  ".to_string()),
                Value::String("policy".to_string()),
            ]),
        )]);
        let validators = vec![
            FieldValidator::new("tags", Type::List).each_fields([FieldValidator::new(
                "",
                Type::Text,
            )
            .transform(Transform::Trim)
            .min_length(4)]),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_fails_fast_on_first_issue() {
        let fields = BTreeMap::from([
            ("title".to_string(), Value::String(" a ".to_string())),
            ("count".to_string(), Value::String("bad".to_string())),
            (
                "articles".to_string(),
                Value::Array(vec![
                    Value::Object(BTreeMap::new()),
                    Value::String("oops".to_string()),
                ]),
            ),
        ]);
        let validators = vec![
            FieldValidator::new("title", Type::Text)
                .transform(Transform::Trim)
                .min_length(2),
            FieldValidator::new("count", Type::Number).transform(Transform::ParseNumber),
            FieldValidator::new("articles", Type::List).each_fields([FieldValidator::new(
                "title",
                Type::Text,
            )
            .required()]),
        ];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field title: text length must be >= 2".to_string()
            )
        );
    }

    #[test]
    fn validate_field_relations_accepts_and_relation() {
        let fields = BTreeMap::from([
            (
                "start_time".to_string(),
                Value::String("2026-04-01".to_string()),
            ),
            (
                "end_time".to_string(),
                Value::String("2026-04-02".to_string()),
            ),
        ]);

        assert!(
            validate_field_relations(&fields, &[Relation::and(["start_time", "end_time"])]).is_ok()
        );
    }

    #[test]
    fn validate_field_relations_rejects_and_relation_when_field_is_missing() {
        let fields = BTreeMap::from([(
            "start_time".to_string(),
            Value::String("2026-04-01".to_string()),
        )]);

        let error = validate_field_relations(&fields, &[Relation::and(["start_time", "end_time"])])
            .unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field [start_time, end_time]: relation and expected all fields to be present, missing: end_time"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_field_relations_accepts_or_relation() {
        let fields = BTreeMap::from([("phone".to_string(), Value::String("123456".to_string()))]);

        assert!(validate_field_relations(&fields, &[Relation::or(["phone", "email"])]).is_ok());
    }

    #[test]
    fn validate_field_relations_rejects_or_relation_when_none_are_present() {
        let fields = BTreeMap::new();
        let error =
            validate_field_relations(&fields, &[Relation::or(["phone", "email"])]).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field [phone, email]: relation or expected at least one field to be present"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_field_relations_accepts_one_relation() {
        let fields = BTreeMap::from([(
            "cover_url".to_string(),
            Value::String("https://example.com/cover.jpg".to_string()),
        )]);

        assert!(
            validate_field_relations(&fields, &[Relation::one(["cover_url", "cover_file"])])
                .is_ok()
        );
    }

    #[test]
    fn validate_field_relations_rejects_one_relation_when_multiple_are_present() {
        let fields = BTreeMap::from([
            (
                "cover_url".to_string(),
                Value::String("https://example.com/cover.jpg".to_string()),
            ),
            (
                "cover_file".to_string(),
                Value::String("/tmp/cover.jpg".to_string()),
            ),
        ]);

        let error =
            validate_field_relations(&fields, &[Relation::one(["cover_url", "cover_file"])])
                .unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field [cover_url, cover_file]: relation one expected exactly one field to be present, but got 2"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_item_fails_fast_with_same_semantics() {
        let item = Item::new()
            .with_field("count", Value::String("oops".to_string()))
            .with_field("published", Value::String("maybe".to_string()));
        let validators = vec![
            FieldValidator::new("count", Type::Number).transform(Transform::ParseNumber),
            FieldValidator::new("published", Type::Bool).transform(Transform::ParseBool),
        ];

        let error = validate_item(&item, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field count: transform parse_number failed: invalid float literal"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_item_uses_same_validation_semantics() {
        let item = Item::new().with_field("count", Value::Number(3.0));
        let validators = vec![FieldValidator::new("count", Type::Number).required()];

        assert!(validate_item(&item, &validators).is_ok());
    }

    #[test]
    fn validate_fields_accepts_regex_min_max_and_enum_rules() {
        let fields = BTreeMap::from([
            ("title".to_string(), Value::String("post-2026".to_string())),
            ("count".to_string(), Value::Number(5.0)),
            ("kind".to_string(), Value::String("news".to_string())),
        ]);
        let validators = vec![
            FieldValidator::new("title", Type::Text)
                .regex(r"^post-\d{4}$")
                .min(4.0)
                .max(16.0),
            FieldValidator::new("count", Type::Number)
                .min(1.0)
                .max(10.0),
            FieldValidator::new("kind", Type::Text).enum_values([
                Value::String("news".to_string()),
                Value::String("notice".to_string()),
            ]),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_rejects_regex_mismatch() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("bad".to_string()))]);
        let validators = vec![FieldValidator::new("title", Type::Text).regex(r"^post-\d+$")];

        let error = validate_fields(&fields, &validators).unwrap_err();

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
        let validators = vec![FieldValidator::new("count", Type::Number).min(3.0)];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse("validation failed for field count: value must be >= 3".to_string())
        );
    }

    #[test]
    fn validate_fields_rejects_text_above_max_length() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("abcdef".to_string()))]);
        let validators = vec![FieldValidator::new("title", Type::Text).max(3.0)];

        let error = validate_fields(&fields, &validators).unwrap_err();

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
        let validators = vec![FieldValidator::new("kind", Type::Text).enum_values([
            Value::String("news".to_string()),
            Value::String("notice".to_string()),
        ])];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field kind: value is not in enum set".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_accepts_explicit_text_length_rules() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("news".to_string()))]);
        let validators = vec![
            FieldValidator::new("title", Type::Text)
                .min_length(2)
                .max_length(8),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_rejects_text_below_min_length() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("a".to_string()))]);
        let validators = vec![FieldValidator::new("title", Type::Text).min_length(2)];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field title: text length must be >= 2".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_accepts_explicit_list_size_rules() {
        let fields = BTreeMap::from([(
            "tags".to_string(),
            Value::Array(vec![
                Value::String("news".to_string()),
                Value::String("policy".to_string()),
            ]),
        )]);
        let validators = vec![
            FieldValidator::new("tags", Type::List)
                .min_items(1)
                .max_items(3),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_rejects_list_below_min_items() {
        let fields = BTreeMap::from([("tags".to_string(), Value::Array(vec![]))]);
        let validators = vec![FieldValidator::new("tags", Type::List).min_items(1)];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field tags: list length must be >= 1".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_accepts_object_rules_with_required_fields() {
        let fields = BTreeMap::from([(
            "meta".to_string(),
            Value::Object(BTreeMap::from([
                ("title".to_string(), Value::String("hello".to_string())),
                (
                    "url".to_string(),
                    Value::String("https://example.com/post".to_string()),
                ),
            ])),
        )]);
        let validators = vec![
            FieldValidator::new("meta", Type::Object)
                .min_fields(2)
                .max_fields(4)
                .required_fields(["title", "url"]),
        ];

        assert!(validate_fields(&fields, &validators).is_ok());
    }

    #[test]
    fn validate_fields_rejects_missing_required_object_fields() {
        let fields = BTreeMap::from([(
            "meta".to_string(),
            Value::Object(BTreeMap::from([(
                "title".to_string(),
                Value::String("hello".to_string()),
            )])),
        )]);
        let validators =
            vec![FieldValidator::new("meta", Type::Object).required_fields(["title", "url"])];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field meta: object is missing required fields: url"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_fields_rejects_invalid_regex_pattern() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("post-1".to_string()))]);
        let validators = vec![FieldValidator::new("title", Type::Text).regex("(")];

        let error = validate_fields(&fields, &validators).unwrap_err();

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
        let validators = vec![FieldValidator::new("meta.title", Type::Text).required()];

        assert!(validate_fields(&fields, &validators).is_ok());
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
        let validators = vec![FieldValidator::new("authors[0].name", Type::Text).required()];

        assert!(validate_fields(&fields, &validators).is_ok());
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
        let validators = vec![FieldValidator::new("tags[]", Type::Text).required()];

        assert!(validate_fields(&fields, &validators).is_ok());
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
        let validators = vec![FieldValidator::new("articles[].title", Type::Text).required()];

        let error = validate_fields(&fields, &validators).unwrap_err();

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
        let validators = vec![FieldValidator::new("title.name", Type::Text).required()];

        let error = validate_fields(&fields, &validators).unwrap_err();

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
        let validators = vec![FieldValidator::new("meta..title", Type::Text)];

        let error = validate_fields(&fields, &validators).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field meta..title: invalid field path: unexpected `.`; field path segments cannot be empty".to_string()
            )
        );
    }

    #[test]
    fn step_validator_collects_fields_and_relations() {
        let validator = StepValidator::new()
            .field("title", Type::Text, |field| {
                field.required().min_length(1).transform(Transform::Trim)
            })
            .field("start_time", Type::Text, |field| {
                field.transform(Transform::ParseDatetime)
            })
            .field("end_time", Type::Text, |field| {
                field.transform(Transform::ParseDatetime)
            })
            .and(["start_time", "end_time"]);

        assert_eq!(validator.fields.len(), 3);
        assert_eq!(validator.relations.len(), 1);
        assert!(!validator.is_empty());
    }

    #[test]
    fn step_validator_rejects_invalid_relation() {
        let validator = StepValidator::new()
            .field("title", Type::Text, |field| {
                field.required().min_length(3).transform(Transform::Trim)
            })
            .field("start_time", Type::Text, |field| {
                field.transform(Transform::ParseDatetime)
            })
            .field("end_time", Type::Text, |field| {
                field.transform(Transform::ParseDatetime)
            })
            .and(["start_time", "end_time"]);
        let item = Item::new()
            .with_field("title", Value::String("post".to_string()))
            .with_field(
                "start_time",
                Value::String("2026-04-01T08:30:45+08:00".to_string()),
            );

        let error = futures::executor::block_on(validator.validate(&item)).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field [start_time, end_time]: relation and expected all fields to be present, missing: end_time"
                    .to_string()
            )
        );
    }
}
