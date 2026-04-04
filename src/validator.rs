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
pub enum ValidationTransform {
    Trim,
    NormalizeWhitespace,
    ParseNumber,
    ParseBool,
    ParseDatetime,
}

impl ValidationTransform {
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
pub enum ValidationConditionKind {
    Exists,
    Missing,
    Equals,
    NotEquals,
}

impl ValidationConditionKind {
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
pub struct ValidationCondition {
    pub field: String,
    pub kind: ValidationConditionKind,
    pub value: Option<Value>,
}

impl ValidationCondition {
    pub fn exists(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            kind: ValidationConditionKind::Exists,
            value: None,
        }
    }

    pub fn missing(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            kind: ValidationConditionKind::Missing,
            value: None,
        }
    }

    pub fn equals(field: impl Into<String>, value: Value) -> Self {
        Self {
            field: field.into(),
            kind: ValidationConditionKind::Equals,
            value: Some(value),
        }
    }

    pub fn not_equals(field: impl Into<String>, value: Value) -> Self {
        Self {
            field: field.into(),
            kind: ValidationConditionKind::NotEquals,
            value: Some(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationGroupKind {
    AllOf,
    AnyOf,
    OneOf,
    MutuallyExclusive,
}

impl ValidationGroupKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllOf => "all_of",
            Self::AnyOf => "any_of",
            Self::OneOf => "one_of",
            Self::MutuallyExclusive => "mutually_exclusive",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidationGroup {
    pub kind: ValidationGroupKind,
    pub validations: Vec<Validation>,
}

impl ValidationGroup {
    fn new(kind: ValidationGroupKind, validations: impl IntoIterator<Item = Validation>) -> Self {
        Self {
            kind,
            validations: validations.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Validation {
    pub field: String,
    pub value_type: ValidationType,
    pub transforms: Vec<ValidationTransform>,
    pub conditions: Vec<ValidationCondition>,
    pub object_validations: Vec<Validation>,
    pub each_validations: Vec<Validation>,
    pub groups: Vec<ValidationGroup>,
    pub rule: ValidationRule,
}

impl Validation {
    pub fn root() -> Self {
        Self::new("", ValidationType::Object)
    }

    pub fn new(field: impl Into<String>, value_type: ValidationType) -> Self {
        Self {
            field: field.into(),
            value_type,
            transforms: Vec::new(),
            conditions: Vec::new(),
            object_validations: Vec::new(),
            each_validations: Vec::new(),
            groups: Vec::new(),
            rule: ValidationRule::default(),
        }
    }

    pub fn with_transform(mut self, transform: ValidationTransform) -> Self {
        self.transforms.push(transform);
        self
    }

    pub fn with_transforms(
        mut self,
        transforms: impl IntoIterator<Item = ValidationTransform>,
    ) -> Self {
        self.transforms.extend(transforms);
        self
    }

    pub fn with_condition(mut self, condition: ValidationCondition) -> Self {
        self.conditions.push(condition);
        self
    }

    pub fn with_conditions(
        mut self,
        conditions: impl IntoIterator<Item = ValidationCondition>,
    ) -> Self {
        self.conditions.extend(conditions);
        self
    }

    pub fn with_when_exists(mut self, field: impl Into<String>) -> Self {
        self.conditions.push(ValidationCondition::exists(field));
        self
    }

    pub fn with_when_missing(mut self, field: impl Into<String>) -> Self {
        self.conditions.push(ValidationCondition::missing(field));
        self
    }

    pub fn with_when_equals(mut self, field: impl Into<String>, value: Value) -> Self {
        self.conditions
            .push(ValidationCondition::equals(field, value));
        self
    }

    pub fn with_when_not_equals(mut self, field: impl Into<String>, value: Value) -> Self {
        self.conditions
            .push(ValidationCondition::not_equals(field, value));
        self
    }

    pub fn with_required_when_exists(self, field: impl Into<String>) -> Self {
        self.with_required(true).with_when_exists(field)
    }

    pub fn with_required_when_missing(self, field: impl Into<String>) -> Self {
        self.with_required(true).with_when_missing(field)
    }

    pub fn with_required_when_equals(self, field: impl Into<String>, value: Value) -> Self {
        self.with_required(true).with_when_equals(field, value)
    }

    pub fn with_required_when_not_equals(self, field: impl Into<String>, value: Value) -> Self {
        self.with_required(true).with_when_not_equals(field, value)
    }

    pub fn with_optional_when_exists(self, field: impl Into<String>) -> Self {
        self.with_when_exists(field)
    }

    pub fn with_optional_when_missing(self, field: impl Into<String>) -> Self {
        self.with_when_missing(field)
    }

    pub fn with_optional_when_equals(self, field: impl Into<String>, value: Value) -> Self {
        self.with_when_equals(field, value)
    }

    pub fn with_optional_when_not_equals(self, field: impl Into<String>, value: Value) -> Self {
        self.with_when_not_equals(field, value)
    }

    pub fn with_object_validations(
        mut self,
        validations: impl IntoIterator<Item = Validation>,
    ) -> Self {
        self.object_validations.extend(validations);
        self
    }

    pub fn with_each_validations(
        mut self,
        validations: impl IntoIterator<Item = Validation>,
    ) -> Self {
        self.each_validations.extend(validations);
        self
    }

    pub fn with_all_of(mut self, validations: impl IntoIterator<Item = Validation>) -> Self {
        self.groups.push(ValidationGroup::new(
            ValidationGroupKind::AllOf,
            validations,
        ));
        self
    }

    pub fn with_any_of(mut self, validations: impl IntoIterator<Item = Validation>) -> Self {
        self.groups.push(ValidationGroup::new(
            ValidationGroupKind::AnyOf,
            validations,
        ));
        self
    }

    pub fn with_one_of(mut self, validations: impl IntoIterator<Item = Validation>) -> Self {
        self.groups.push(ValidationGroup::new(
            ValidationGroupKind::OneOf,
            validations,
        ));
        self
    }

    pub fn with_mutually_exclusive(
        mut self,
        validations: impl IntoIterator<Item = Validation>,
    ) -> Self {
        self.groups.push(ValidationGroup::new(
            ValidationGroupKind::MutuallyExclusive,
            validations,
        ));
        self
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

    pub fn with_min_length(mut self, min_length: usize) -> Self {
        self.rule.min_length = Some(min_length);
        self
    }

    pub fn with_max_length(mut self, max_length: usize) -> Self {
        self.rule.max_length = Some(max_length);
        self
    }

    pub fn with_min_items(mut self, min_items: usize) -> Self {
        self.rule.min_items = Some(min_items);
        self
    }

    pub fn with_max_items(mut self, max_items: usize) -> Self {
        self.rule.max_items = Some(max_items);
        self
    }

    pub fn with_min_fields(mut self, min_fields: usize) -> Self {
        self.rule.min_fields = Some(min_fields);
        self
    }

    pub fn with_max_fields(mut self, max_fields: usize) -> Self {
        self.rule.max_fields = Some(max_fields);
        self
    }

    pub fn with_required_fields<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.rule.required_fields = fields.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_enum(mut self, values: impl IntoIterator<Item = Value>) -> Self {
        self.rule.enum_values = values.into_iter().collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub field: String,
    pub message: String,
}

impl ValidationIssue {
    fn from_error(error: SpiderError) -> Self {
        let text = match error {
            SpiderError::Parse(message) => message,
            other => other.to_string(),
        };

        let prefix = "validation failed for field ";
        if let Some(rest) = text.strip_prefix(prefix)
            && let Some((field, message)) = rest.split_once(": ")
        {
            return Self {
                field: field.to_string(),
                message: message.to_string(),
            };
        }

        Self {
            field: String::new(),
            message: text,
        }
    }

    fn to_error(&self) -> SpiderError {
        if self.field.is_empty() {
            return SpiderError::parse(self.message.clone());
        }

        SpiderError::parse(format!(
            "validation failed for field {}: {}",
            self.field, self.message
        ))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }

    pub fn first_error(&self) -> Option<SpiderError> {
        self.issues.first().map(ValidationIssue::to_error)
    }

    pub fn into_result(self) -> Result<(), SpiderError> {
        if let Some(issue) = self.issues.first() {
            return Err(issue.to_error());
        }

        Ok(())
    }
}

pub fn validate_fields(
    fields: &BTreeMap<String, Value>,
    validations: &[Validation],
) -> Result<(), SpiderError> {
    let mut collector = ValidationCollector::fail_fast();
    validate_fields_internal(fields, validations, &mut collector)
}

pub fn validate_item(item: &Item, validations: &[Validation]) -> Result<(), SpiderError> {
    validate_fields(&item.fields, validations)
}

pub fn validate_fields_report(
    fields: &BTreeMap<String, Value>,
    validations: &[Validation],
) -> ValidationReport {
    let mut collector = ValidationCollector::collect_all();
    let _ = validate_fields_internal(fields, validations, &mut collector);
    collector.into_report()
}

pub fn validate_item_report(item: &Item, validations: &[Validation]) -> ValidationReport {
    validate_fields_report(&item.fields, validations)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationRunMode {
    FailFast,
    CollectAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationStatus {
    Skipped,
    Applied,
}

struct ValidationCollector {
    mode: ValidationRunMode,
    issues: Vec<ValidationIssue>,
}

impl ValidationCollector {
    fn fail_fast() -> Self {
        Self {
            mode: ValidationRunMode::FailFast,
            issues: Vec::new(),
        }
    }

    fn collect_all() -> Self {
        Self {
            mode: ValidationRunMode::CollectAll,
            issues: Vec::new(),
        }
    }

    fn record_error(&mut self, error: SpiderError) -> Result<(), SpiderError> {
        match self.mode {
            ValidationRunMode::FailFast => Err(error),
            ValidationRunMode::CollectAll => {
                self.issues.push(ValidationIssue::from_error(error));
                Ok(())
            }
        }
    }

    fn into_report(self) -> ValidationReport {
        ValidationReport {
            issues: self.issues,
        }
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

struct ScopedValidationCheck {
    status: ValidationStatus,
    report: ValidationReport,
}

struct FieldPathCursor<'a> {
    field: String,
    node: FieldPathNode<'a>,
}

fn validate_fields_internal(
    fields: &BTreeMap<String, Value>,
    validations: &[Validation],
    collector: &mut ValidationCollector,
) -> Result<(), SpiderError> {
    let root_value = Value::Object(fields.clone());

    for validation in validations {
        if validation.field.is_empty() {
            let resolved_values = vec![ResolvedFieldValue {
                field: display_validation_field("").to_string(),
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
    validation: &Validation,
    scope_field: &str,
    scope_value: &Value,
    resolved_values: &[ResolvedFieldValue<'_>],
    collector: &mut ValidationCollector,
) -> Result<ValidationStatus, SpiderError> {
    match validation_conditions_match(scope_field, scope_value, &validation.conditions) {
        Ok(false) => return Ok(ValidationStatus::Skipped),
        Ok(true) => {}
        Err(error) => {
            collector.record_error(error)?;
            return Ok(ValidationStatus::Applied);
        }
    }

    if resolved_values.is_empty() {
        if validation.rule.required {
            collector.record_error(missing_value_error(validation.field.as_str()))?;
            return Ok(ValidationStatus::Applied);
        }
        return Ok(ValidationStatus::Skipped);
    }

    let mut status = ValidationStatus::Skipped;
    for resolved in resolved_values {
        let resolved_status = validate_resolved_value(
            validation,
            resolved.field.as_str(),
            resolved.value,
            collector,
        )?;
        if resolved_status == ValidationStatus::Applied {
            status = ValidationStatus::Applied;
        }
    }

    Ok(status)
}

fn validate_resolved_value(
    validation: &Validation,
    resolved_field: &str,
    resolved_value: Option<&Value>,
    collector: &mut ValidationCollector,
) -> Result<ValidationStatus, SpiderError> {
    let Some(value) = resolved_value.filter(|value| !matches!(value, Value::Null)) else {
        if validation.rule.required {
            collector.record_error(missing_value_error(resolved_field))?;
            return Ok(ValidationStatus::Applied);
        }
        return Ok(ValidationStatus::Skipped);
    };

    let transformed_value = if validation.transforms.is_empty() {
        None
    } else {
        match apply_validation_transforms(resolved_field, value.clone(), &validation.transforms) {
            Ok(value) => Some(value),
            Err(error) => {
                collector.record_error(error)?;
                return Ok(ValidationStatus::Applied);
            }
        }
    };
    let value = transformed_value.as_ref().unwrap_or(value);

    if !validation.value_type.matches(value) {
        collector.record_error(SpiderError::parse(format!(
            "validation failed for field {}: expected {}",
            display_validation_field(resolved_field),
            validation.value_type.as_str()
        )))?;
        return Ok(ValidationStatus::Applied);
    }

    if let Err(error) = validate_rule(validation, resolved_field, value) {
        collector.record_error(error)?;
        return Ok(ValidationStatus::Applied);
    }

    validate_nested_validations(validation, resolved_field, value, collector)?;
    Ok(ValidationStatus::Applied)
}

fn validate_nested_validations(
    validation: &Validation,
    field: &str,
    value: &Value,
    collector: &mut ValidationCollector,
) -> Result<(), SpiderError> {
    if !validation.object_validations.is_empty() {
        validate_object_validations(field, value, &validation.object_validations, collector)?;
    }

    if !validation.each_validations.is_empty() {
        validate_each_validations(field, value, &validation.each_validations, collector)?;
    }

    if !validation.groups.is_empty() {
        validate_validation_groups(field, value, &validation.groups, collector)?;
    }

    Ok(())
}

fn validate_object_validations(
    field: &str,
    value: &Value,
    validations: &[Validation],
    collector: &mut ValidationCollector,
) -> Result<(), SpiderError> {
    validate_scoped_validations(
        field,
        value,
        validations,
        collector,
        "object validations only support object values",
    )
}

fn validate_each_validations(
    field: &str,
    value: &Value,
    validations: &[Validation],
    collector: &mut ValidationCollector,
) -> Result<(), SpiderError> {
    let Value::Array(items) = value else {
        collector.record_error(SpiderError::parse(format!(
            "validation failed for field {}: each validations only support list values",
            field
        )))?;
        return Ok(());
    };

    for (index, item) in items.iter().enumerate() {
        let item_field = append_index_path(field, index);
        for validation in validations {
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

fn validate_scoped_validations(
    scope_field: &str,
    scope_value: &Value,
    validations: &[Validation],
    collector: &mut ValidationCollector,
    non_object_message: &str,
) -> Result<(), SpiderError> {
    let Value::Object(_) = scope_value else {
        collector.record_error(SpiderError::parse(format!(
            "validation failed for field {}: {}",
            display_validation_field(scope_field),
            non_object_message
        )))?;
        return Ok(());
    };

    for validation in validations {
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

fn validate_validation_groups(
    scope_field: &str,
    scope_value: &Value,
    groups: &[ValidationGroup],
    collector: &mut ValidationCollector,
) -> Result<(), SpiderError> {
    let Value::Object(_) = scope_value else {
        collector.record_error(SpiderError::parse(format!(
            "validation failed for field {}: validation groups only support object values",
            display_validation_field(scope_field)
        )))?;
        return Ok(());
    };

    for group in groups {
        validate_validation_group(scope_field, scope_value, group, collector)?;
    }

    Ok(())
}

fn validate_validation_group(
    scope_field: &str,
    scope_value: &Value,
    group: &ValidationGroup,
    collector: &mut ValidationCollector,
) -> Result<(), SpiderError> {
    if group.validations.is_empty() {
        return Ok(());
    }

    match group.kind {
        ValidationGroupKind::AllOf => validate_scoped_validations(
            scope_field,
            scope_value,
            &group.validations,
            collector,
            "validation groups only support object values",
        ),
        ValidationGroupKind::AnyOf
        | ValidationGroupKind::OneOf
        | ValidationGroupKind::MutuallyExclusive => {
            let checks = group
                .validations
                .iter()
                .map(|validation| {
                    validate_scoped_validation_check(scope_field, scope_value, validation)
                })
                .collect::<Vec<_>>();
            let passed = checks
                .iter()
                .filter(|check| check.status == ValidationStatus::Applied && check.report.is_ok())
                .count();
            let message = match group.kind {
                ValidationGroupKind::AnyOf if passed >= 1 => return Ok(()),
                ValidationGroupKind::OneOf if passed == 1 => return Ok(()),
                ValidationGroupKind::MutuallyExclusive if passed <= 1 => return Ok(()),
                ValidationGroupKind::AnyOf => format!(
                    "group {} expected at least one validation to pass, but got 0 ({})",
                    group.kind.as_str(),
                    summarize_validation_checks(&checks)
                ),
                ValidationGroupKind::OneOf => format!(
                    "group {} expected exactly one validation to pass, but got {}",
                    group.kind.as_str(),
                    passed
                ),
                ValidationGroupKind::MutuallyExclusive => format!(
                    "group {} expected at most one validation to pass, but got {}",
                    group.kind.as_str(),
                    passed
                ),
                ValidationGroupKind::AllOf => unreachable!(),
            };

            collector.record_error(SpiderError::parse(format!(
                "validation failed for field {}: {}",
                display_validation_field(scope_field),
                message
            )))?;
            Ok(())
        }
    }
}

fn validate_scoped_validation_check(
    scope_field: &str,
    scope_value: &Value,
    validation: &Validation,
) -> ScopedValidationCheck {
    let mut collector = ValidationCollector::collect_all();
    let status =
        match resolve_scoped_field_values(scope_field, scope_value, validation.field.as_str()) {
            Ok(resolved_values) => match validate_field(
                validation,
                scope_field,
                scope_value,
                &resolved_values,
                &mut collector,
            ) {
                Ok(status) => status,
                Err(error) => {
                    let _ = collector.record_error(error);
                    ValidationStatus::Applied
                }
            },
            Err(error) => {
                let _ = collector.record_error(error);
                ValidationStatus::Applied
            }
        };

    ScopedValidationCheck {
        status,
        report: collector.into_report(),
    }
}

fn summarize_validation_checks(checks: &[ScopedValidationCheck]) -> String {
    let parts = checks
        .iter()
        .flat_map(|check| check.report.issues.iter().take(1))
        .map(|issue| {
            if issue.field.is_empty() {
                issue.message.clone()
            } else {
                format!(
                    "{}: {}",
                    summarize_issue_field(issue.field.as_str()),
                    issue.message
                )
            }
        })
        .collect::<Vec<_>>();

    if parts.is_empty() {
        return "all validation branches were skipped".to_string();
    }

    parts.join("; ")
}

fn summarize_issue_field(field: &str) -> &str {
    field.strip_prefix("$.").unwrap_or(field)
}

fn validation_conditions_match(
    scope_field: &str,
    scope_value: &Value,
    conditions: &[ValidationCondition],
) -> Result<bool, SpiderError> {
    for condition in conditions {
        if !validation_condition_matches(scope_field, scope_value, condition)? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn validation_condition_matches(
    scope_field: &str,
    scope_value: &Value,
    condition: &ValidationCondition,
) -> Result<bool, SpiderError> {
    let resolved_values =
        resolve_scoped_field_values(scope_field, scope_value, condition.field.as_str())?;
    let present_values = resolved_values
        .iter()
        .filter_map(|resolved| resolved.value)
        .filter(|value| !matches!(value, Value::Null))
        .collect::<Vec<_>>();

    match condition.kind {
        ValidationConditionKind::Exists => Ok(!present_values.is_empty()),
        ValidationConditionKind::Missing => Ok(present_values.is_empty()),
        ValidationConditionKind::Equals => {
            let expected = validation_condition_expected_value(condition)?;
            Ok(present_values.iter().any(|value| *value == expected))
        }
        ValidationConditionKind::NotEquals => {
            let expected = validation_condition_expected_value(condition)?;
            Ok(!present_values.is_empty() && present_values.iter().all(|value| *value != expected))
        }
    }
}

fn validation_condition_expected_value(
    condition: &ValidationCondition,
) -> Result<&Value, SpiderError> {
    condition.value.as_ref().ok_or_else(|| {
        SpiderError::parse(format!(
            "validation condition {} on field {} requires a comparison value",
            condition.kind.as_str(),
            display_validation_field(condition.field.as_str())
        ))
    })
}

fn apply_validation_transforms(
    field: &str,
    mut value: Value,
    transforms: &[ValidationTransform],
) -> Result<Value, SpiderError> {
    for transform in transforms {
        value = apply_validation_transform(field, value, *transform)?;
    }

    Ok(value)
}

fn apply_validation_transform(
    field: &str,
    value: Value,
    transform: ValidationTransform,
) -> Result<Value, SpiderError> {
    match transform {
        ValidationTransform::Trim => trim_validation_value(field, value),
        ValidationTransform::NormalizeWhitespace => {
            normalize_whitespace_validation_value(field, value)
        }
        ValidationTransform::ParseNumber => parse_number_validation_value(field, value),
        ValidationTransform::ParseBool => parse_bool_validation_value(field, value),
        ValidationTransform::ParseDatetime => parse_datetime_validation_value(field, value),
    }
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

fn trim_validation_value(field: &str, value: Value) -> Result<Value, SpiderError> {
    let Value::String(text) = value else {
        return Err(transform_type_error(
            field,
            ValidationTransform::Trim,
            &value,
            "text",
        ));
    };

    Ok(Value::String(text.trim().to_string()))
}

fn normalize_whitespace_validation_value(field: &str, value: Value) -> Result<Value, SpiderError> {
    let Value::String(text) = value else {
        return Err(transform_type_error(
            field,
            ValidationTransform::NormalizeWhitespace,
            &value,
            "text",
        ));
    };

    Ok(Value::String(normalize_whitespace_text(&text)))
}

fn parse_number_validation_value(field: &str, value: Value) -> Result<Value, SpiderError> {
    match value {
        Value::Number(value) => Ok(Value::Number(value)),
        Value::String(text) => {
            let normalized = text.trim();
            if normalized.is_empty() {
                return Err(transform_error(
                    field,
                    ValidationTransform::ParseNumber,
                    "empty string",
                ));
            }

            normalized
                .parse::<f64>()
                .map(Value::Number)
                .map_err(|error| {
                    transform_error(field, ValidationTransform::ParseNumber, &error.to_string())
                })
        }
        other => Err(transform_type_error(
            field,
            ValidationTransform::ParseNumber,
            &other,
            "string or number",
        )),
    }
}

fn parse_bool_validation_value(field: &str, value: Value) -> Result<Value, SpiderError> {
    match value {
        Value::Bool(value) => Ok(Value::Bool(value)),
        Value::String(text) => {
            let normalized = text.trim();
            if normalized.is_empty() {
                return Err(transform_error(
                    field,
                    ValidationTransform::ParseBool,
                    "empty string",
                ));
            }

            parse_bool_text(normalized).map(Value::Bool).ok_or_else(|| {
                transform_error(
                    field,
                    ValidationTransform::ParseBool,
                    &format!("expected true/false/1/0, got {normalized:?}"),
                )
            })
        }
        other => Err(transform_type_error(
            field,
            ValidationTransform::ParseBool,
            &other,
            "string or bool",
        )),
    }
}

fn parse_datetime_validation_value(field: &str, value: Value) -> Result<Value, SpiderError> {
    let Value::String(text) = value else {
        return Err(transform_type_error(
            field,
            ValidationTransform::ParseDatetime,
            &value,
            "text",
        ));
    };

    let normalized = text.trim();
    if normalized.is_empty() {
        return Err(transform_error(
            field,
            ValidationTransform::ParseDatetime,
            "empty string",
        ));
    }

    parse_validation_datetime_text(normalized)
        .map(Value::String)
        .map_err(|message| transform_error(field, ValidationTransform::ParseDatetime, &message))
}

fn parse_validation_datetime_text(text: &str) -> Result<String, String> {
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

fn transform_error(field: &str, transform: ValidationTransform, detail: &str) -> SpiderError {
    SpiderError::parse(format!(
        "validation failed for field {}: transform {} failed: {}",
        field,
        transform.as_str(),
        detail
    ))
}

fn transform_type_error(
    field: &str,
    transform: ValidationTransform,
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
            field: display_validation_field(scope_field).to_string(),
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

fn display_validation_field(field: &str) -> &str {
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
    fn validate_fields_skips_missing_optional_field() {
        let fields = BTreeMap::new();
        let validations = vec![Validation::new("title", ValidationType::Text).with_min_length(3)];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_requires_field_when_required_when_equals_matches() {
        let fields = BTreeMap::from([("type".to_string(), Value::String("video".to_string()))]);
        let validations = vec![
            Validation::new("duration", ValidationType::Number)
                .with_required_when_equals("type", Value::String("video".to_string())),
        ];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
        let validations = vec![
            Validation::new("duration", ValidationType::Number)
                .with_required_when_equals("type", Value::String("video".to_string())),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_applies_optional_validation_when_exists_condition_matches() {
        let fields = BTreeMap::from([
            ("title".to_string(), Value::String("Kun".to_string())),
            ("summary".to_string(), Value::String("bad".to_string())),
        ]);
        let validations = vec![
            Validation::new("summary", ValidationType::Text)
                .with_optional_when_exists("title")
                .with_min_length(5),
        ];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
        let validations = vec![
            Validation::new("summary", ValidationType::Text)
                .with_optional_when_exists("title")
                .with_min_length(5),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
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
        let validations = vec![
            Validation::new("asset", ValidationType::Object)
                .with_object_validations([Validation::new("checksum", ValidationType::Text)
                    .with_required_when_missing("signature")]),
        ];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
        let validations = vec![
            Validation::new("asset", ValidationType::Object)
                .with_object_validations([Validation::new("checksum", ValidationType::Text)
                    .with_required_when_missing("signature")]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_requires_field_when_required_when_not_equals_matches() {
        let fields = BTreeMap::from([("kind".to_string(), Value::String("news".to_string()))]);
        let validations = vec![
            Validation::new("summary", ValidationType::Text)
                .with_required_when_not_equals("kind", Value::String("redirect".to_string())),
        ];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
        let validations = vec![
            Validation::new("summary", ValidationType::Text)
                .with_required_when_not_equals("kind", Value::String("redirect".to_string())),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_accepts_number_transform_before_numeric_rules() {
        let fields = BTreeMap::from([("count".to_string(), Value::String(" 42 ".to_string()))]);
        let validations = vec![
            Validation::new("count", ValidationType::Number)
                .with_transform(ValidationTransform::ParseNumber)
                .with_min(10.0)
                .with_max(100.0),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_accepts_bool_transform_before_type_check() {
        let fields =
            BTreeMap::from([("published".to_string(), Value::String(" true ".to_string()))]);
        let validations = vec![
            Validation::new("published", ValidationType::Bool)
                .with_transform(ValidationTransform::ParseBool),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_accepts_datetime_transform_before_enum_check() {
        let fields = BTreeMap::from([(
            "published_at".to_string(),
            Value::String("2026-04-01T08:30:45+08:00".to_string()),
        )]);
        let validations = vec![
            Validation::new("published_at", ValidationType::Text)
                .with_transform(ValidationTransform::ParseDatetime)
                .with_enum([Value::String("2026-04-01T00:30:45Z".to_string())]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_accepts_trim_and_normalize_whitespace_before_regex() {
        let fields = BTreeMap::from([(
            "title".to_string(),
            Value::String("  Hello   Kun  ".to_string()),
        )]);
        let validations = vec![
            Validation::new("title", ValidationType::Text)
                .with_transforms([
                    ValidationTransform::Trim,
                    ValidationTransform::NormalizeWhitespace,
                ])
                .with_regex(r"^Hello Kun$"),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_rejects_failed_number_transform() {
        let fields =
            BTreeMap::from([("count".to_string(), Value::String("forty-two".to_string()))]);
        let validations = vec![
            Validation::new("count", ValidationType::Number)
                .with_transform(ValidationTransform::ParseNumber),
        ];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
        let validations = vec![
            Validation::new("count", ValidationType::Number)
                .with_transform(ValidationTransform::ParseNumber),
        ];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field count: transform parse_number failed: expected string or number, got bool"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_fields_accepts_object_validations() {
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
        let validations = vec![
            Validation::new("meta", ValidationType::Object)
                .with_required_fields(["title", "published_at"])
                .with_object_validations([
                    Validation::new("title", ValidationType::Text)
                        .with_transform(ValidationTransform::Trim)
                        .with_min_length(3),
                    Validation::new("published_at", ValidationType::Text)
                        .with_transform(ValidationTransform::ParseDatetime)
                        .with_enum([Value::String("2026-04-01T00:30:45Z".to_string())]),
                ]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_skips_missing_optional_nested_object_field() {
        let fields = BTreeMap::from([("meta".to_string(), Value::Object(BTreeMap::new()))]);
        let validations = vec![
            Validation::new("meta", ValidationType::Object).with_object_validations([
                Validation::new("subtitle", ValidationType::Text).with_min_length(3),
            ]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
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
        let validations = vec![
            Validation::new("meta", ValidationType::Object)
                .with_object_validations([Validation::new("duration", ValidationType::Number)
                    .with_required_when_equals("type", Value::String("video".to_string()))]),
        ];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field meta.duration: value is required".to_string()
            )
        );
    }

    #[test]
    fn validate_fields_accepts_each_validations_for_object_items() {
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
        let validations = vec![
            Validation::new("articles", ValidationType::List)
                .with_min_items(2)
                .with_each_validations([
                    Validation::new("title", ValidationType::Text)
                        .with_transform(ValidationTransform::Trim)
                        .with_min_length(5),
                    Validation::new("score", ValidationType::Number)
                        .with_transform(ValidationTransform::ParseNumber)
                        .with_min(10.0),
                ]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_accepts_each_validations_for_scalar_items() {
        let fields = BTreeMap::from([(
            "tags".to_string(),
            Value::Array(vec![
                Value::String("  news  ".to_string()),
                Value::String("policy".to_string()),
            ]),
        )]);
        let validations = vec![
            Validation::new("tags", ValidationType::List).with_each_validations([Validation::new(
                "",
                ValidationType::Text,
            )
            .with_transform(ValidationTransform::Trim)
            .with_min_length(4)]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_report_collects_multiple_issues() {
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
        let validations = vec![
            Validation::new("title", ValidationType::Text)
                .with_transform(ValidationTransform::Trim)
                .with_min_length(2),
            Validation::new("count", ValidationType::Number)
                .with_transform(ValidationTransform::ParseNumber),
            Validation::new("articles", ValidationType::List).with_each_validations([
                Validation::new("title", ValidationType::Text).with_required(true),
            ]),
        ];

        let report = validate_fields_report(&fields, &validations);

        assert!(report.is_err());
        assert_eq!(
            report.issues,
            vec![
                ValidationIssue {
                    field: "title".to_string(),
                    message: "text length must be >= 2".to_string(),
                },
                ValidationIssue {
                    field: "count".to_string(),
                    message: "transform parse_number failed: invalid float literal".to_string(),
                },
                ValidationIssue {
                    field: "articles[0].title".to_string(),
                    message: "value is required".to_string(),
                },
                ValidationIssue {
                    field: "articles[1].title".to_string(),
                    message: "cannot access field `title` from text at articles[1]".to_string(),
                },
            ]
        );
        assert_eq!(
            report.first_error(),
            Some(SpiderError::Parse(
                "validation failed for field title: text length must be >= 2".to_string()
            ))
        );
    }

    #[test]
    fn validate_fields_accepts_root_any_of_group() {
        let fields = BTreeMap::from([(
            "headline".to_string(),
            Value::String("Kun update".to_string()),
        )]);
        let validations = vec![Validation::root().with_any_of([
            Validation::new("title", ValidationType::Text).with_required(true),
            Validation::new("headline", ValidationType::Text).with_required(true),
        ])];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_reject_root_any_of_group_when_none_match() {
        let fields = BTreeMap::new();
        let validations = vec![Validation::root().with_any_of([
            Validation::new("title", ValidationType::Text).with_required(true),
            Validation::new("headline", ValidationType::Text).with_required(true),
        ])];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field $: group any_of expected at least one validation to pass, but got 0 (title: value is required; headline: value is required)"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_fields_reject_root_any_of_group_when_all_optional_branches_are_skipped() {
        let fields = BTreeMap::new();
        let validations = vec![Validation::root().with_any_of([
            Validation::new("title", ValidationType::Text),
            Validation::new("headline", ValidationType::Text),
        ])];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field $: group any_of expected at least one validation to pass, but got 0 (all validation branches were skipped)"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_fields_accepts_root_one_of_group_with_one_optional_present_branch() {
        let fields = BTreeMap::from([("headline".to_string(), Value::String("Kun".to_string()))]);
        let validations = vec![Validation::root().with_one_of([
            Validation::new("title", ValidationType::Text),
            Validation::new("headline", ValidationType::Text),
        ])];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_accepts_object_one_of_group() {
        let fields = BTreeMap::from([(
            "contact".to_string(),
            Value::Object(BTreeMap::from([(
                "email".to_string(),
                Value::String("kun@example.com".to_string()),
            )])),
        )]);
        let validations = vec![
            Validation::new("contact", ValidationType::Object).with_one_of([
                Validation::new("email", ValidationType::Text).with_required(true),
                Validation::new("phone", ValidationType::Text).with_required(true),
            ]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_rejects_object_one_of_group_when_multiple_match() {
        let fields = BTreeMap::from([(
            "contact".to_string(),
            Value::Object(BTreeMap::from([
                (
                    "email".to_string(),
                    Value::String("kun@example.com".to_string()),
                ),
                ("phone".to_string(), Value::String("123".to_string())),
            ])),
        )]);
        let validations = vec![
            Validation::new("contact", ValidationType::Object).with_one_of([
                Validation::new("email", ValidationType::Text).with_required(true),
                Validation::new("phone", ValidationType::Text).with_required(true),
            ]),
        ];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field contact: group one_of expected exactly one validation to pass, but got 2"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_fields_accepts_mutually_exclusive_group_when_zero_or_one_match() {
        let fields = BTreeMap::from([(
            "asset".to_string(),
            Value::Object(BTreeMap::from([(
                "url".to_string(),
                Value::String("https://example.com".to_string()),
            )])),
        )]);
        let validations = vec![
            Validation::new("asset", ValidationType::Object).with_mutually_exclusive([
                Validation::new("url", ValidationType::Text).with_required(true),
                Validation::new("file_path", ValidationType::Text).with_required(true),
            ]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_rejects_mutually_exclusive_group_when_multiple_match() {
        let fields = BTreeMap::from([(
            "asset".to_string(),
            Value::Object(BTreeMap::from([
                (
                    "url".to_string(),
                    Value::String("https://example.com".to_string()),
                ),
                (
                    "file_path".to_string(),
                    Value::String("/tmp/file".to_string()),
                ),
            ])),
        )]);
        let validations = vec![
            Validation::new("asset", ValidationType::Object).with_mutually_exclusive([
                Validation::new("url", ValidationType::Text).with_required(true),
                Validation::new("file_path", ValidationType::Text).with_required(true),
            ]),
        ];

        let error = validate_fields(&fields, &validations).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Parse(
                "validation failed for field asset: group mutually_exclusive expected at most one validation to pass, but got 2"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_fields_accepts_all_of_group() {
        let fields = BTreeMap::from([(
            "meta".to_string(),
            Value::Object(BTreeMap::from([
                ("title".to_string(), Value::String("Kun".to_string())),
                (
                    "url".to_string(),
                    Value::String("https://example.com".to_string()),
                ),
            ])),
        )]);
        let validations = vec![
            Validation::new("meta", ValidationType::Object).with_all_of([
                Validation::new("title", ValidationType::Text).with_required(true),
                Validation::new("url", ValidationType::Text).with_required(true),
            ]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_item_report_uses_same_collect_all_semantics() {
        let item = Item::new()
            .with_field("count", Value::String("oops".to_string()))
            .with_field("published", Value::String("maybe".to_string()));
        let validations = vec![
            Validation::new("count", ValidationType::Number)
                .with_transform(ValidationTransform::ParseNumber),
            Validation::new("published", ValidationType::Bool)
                .with_transform(ValidationTransform::ParseBool),
        ];

        let report = validate_item_report(&item, &validations);

        assert_eq!(report.issues.len(), 2);
        assert_eq!(report.issues[0].field, "count");
        assert_eq!(report.issues[1].field, "published");
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
    fn validate_fields_accepts_explicit_text_length_rules() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("news".to_string()))]);
        let validations = vec![
            Validation::new("title", ValidationType::Text)
                .with_min_length(2)
                .with_max_length(8),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_rejects_text_below_min_length() {
        let fields = BTreeMap::from([("title".to_string(), Value::String("a".to_string()))]);
        let validations = vec![Validation::new("title", ValidationType::Text).with_min_length(2)];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
        let validations = vec![
            Validation::new("tags", ValidationType::List)
                .with_min_items(1)
                .with_max_items(3),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
    }

    #[test]
    fn validate_fields_rejects_list_below_min_items() {
        let fields = BTreeMap::from([("tags".to_string(), Value::Array(vec![]))]);
        let validations = vec![Validation::new("tags", ValidationType::List).with_min_items(1)];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
        let validations = vec![
            Validation::new("meta", ValidationType::Object)
                .with_min_fields(2)
                .with_max_fields(4)
                .with_required_fields(["title", "url"]),
        ];

        assert!(validate_fields(&fields, &validations).is_ok());
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
        let validations = vec![
            Validation::new("meta", ValidationType::Object).with_required_fields(["title", "url"]),
        ];

        let error = validate_fields(&fields, &validations).unwrap_err();

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
