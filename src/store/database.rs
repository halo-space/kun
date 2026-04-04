use crate::error::SpiderError;
use crate::item::Item;
use crate::value::Value;
use std::collections::BTreeSet;

pub(crate) const RESERVED_FIELD_COLUMN_NAMES: &[&str] = &["id", "spider_name", "item_json"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldColumnType {
    Text,
    Integer,
    Real,
    Bool,
    Json,
}

impl FieldColumnType {
    pub(crate) fn sqlite_type(self) -> &'static str {
        match self {
            Self::Text | Self::Json => "TEXT",
            Self::Integer | Self::Bool => "INTEGER",
            Self::Real => "REAL",
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Real => "real",
            Self::Bool => "bool",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FieldColumn {
    pub(crate) field: String,
    pub(crate) name: String,
    pub(crate) column_type: FieldColumnType,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FieldColumnValue {
    Null,
    Text(String),
    Integer(i64),
    Real(f64),
    Bool(bool),
    Json(serde_json::Value),
}

pub(crate) fn build_field_column(
    field: impl Into<String>,
    name: impl Into<String>,
    column_type: FieldColumnType,
) -> FieldColumn {
    FieldColumn {
        field: field.into(),
        name: name.into(),
        column_type,
    }
}

pub(crate) fn map_field_column_value(
    backend_name: &str,
    item: &Item,
    column: &FieldColumn,
) -> Result<FieldColumnValue, SpiderError> {
    let Some(value) = item.get(&column.field) else {
        return Ok(FieldColumnValue::Null);
    };

    if matches!(value, Value::Null) {
        return Ok(FieldColumnValue::Null);
    }

    match column.column_type {
        FieldColumnType::Text => match value {
            Value::String(value) => Ok(FieldColumnValue::Text(value.clone())),
            other => Err(field_column_type_error(backend_name, column, other)),
        },
        FieldColumnType::Integer => match value {
            Value::Number(value) => {
                if !value.is_finite() || value.fract() != 0.0 {
                    return Err(SpiderError::engine(format!(
                        "{backend_name} store field `{}` cannot be stored in column `{}` as integer: expected a whole number",
                        column.field, column.name
                    )));
                }

                if *value < i64::MIN as f64 || *value > i64::MAX as f64 {
                    return Err(SpiderError::engine(format!(
                        "{backend_name} store field `{}` cannot be stored in column `{}` as integer: value is out of range",
                        column.field, column.name
                    )));
                }

                Ok(FieldColumnValue::Integer(*value as i64))
            }
            other => Err(field_column_type_error(backend_name, column, other)),
        },
        FieldColumnType::Real => match value {
            Value::Number(value) if value.is_finite() => Ok(FieldColumnValue::Real(*value)),
            Value::Number(_) => Err(SpiderError::engine(format!(
                "{backend_name} store field `{}` cannot be stored in column `{}` as real: value must be finite",
                column.field, column.name
            ))),
            other => Err(field_column_type_error(backend_name, column, other)),
        },
        FieldColumnType::Bool => match value {
            Value::Bool(value) => Ok(FieldColumnValue::Bool(*value)),
            other => Err(field_column_type_error(backend_name, column, other)),
        },
        FieldColumnType::Json => Ok(FieldColumnValue::Json(value.to_json())),
    }
}

pub(crate) fn validate_identifier(
    backend_name: &str,
    kind: &str,
    identifier: &str,
) -> Result<(), SpiderError> {
    let mut chars = identifier.chars();
    let Some(first) = chars.next() else {
        return Err(SpiderError::engine(format!(
            "{backend_name} store {kind} name cannot be empty"
        )));
    };

    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|char| char.is_ascii_alphanumeric() || char == '_')
    {
        return Err(SpiderError::engine(format!(
            "{backend_name} store {kind} name must use only ASCII letters, digits, and underscores, and cannot start with a digit: {identifier}"
        )));
    }

    Ok(())
}

pub(crate) fn validate_field_columns(
    backend_name: &str,
    columns: &[FieldColumn],
) -> Result<(), SpiderError> {
    let mut names = BTreeSet::new();

    for column in columns {
        validate_identifier(backend_name, "field column", &column.name)?;
        if RESERVED_FIELD_COLUMN_NAMES.contains(&column.name.as_str()) {
            return Err(SpiderError::engine(format!(
                "{backend_name} store field column name `{}` is reserved",
                column.name
            )));
        }

        if !names.insert(column.name.clone()) {
            return Err(SpiderError::engine(format!(
                "{backend_name} store field column name `{}` is duplicated",
                column.name
            )));
        }
    }

    Ok(())
}

pub(crate) fn quote_identifier(identifier: &str) -> String {
    format!("\"{identifier}\"")
}

fn field_column_type_error(backend_name: &str, column: &FieldColumn, value: &Value) -> SpiderError {
    SpiderError::engine(format!(
        "{backend_name} store field `{}` cannot be stored in column `{}` as {}: got {}",
        column.field,
        column.name,
        column.column_type.as_str(),
        value_type_name(value)
    ))
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "text",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
    }
}
