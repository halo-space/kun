use crate::{error::SpiderError, value::Value};
use jiff::{
    Timestamp, Zoned,
    civil::{Date, DateTime},
};
use std::collections::BTreeMap;
use url::Url;

const KNOWN_CIVIL_DATETIME_FORMATS: &[&str] = &[
    "%F %H:%M:%S",
    "%F %H:%M",
    "%Y/%m/%d %H:%M:%S",
    "%Y/%m/%d %H:%M",
];

const KNOWN_CIVIL_DATE_FORMATS: &[&str] = &["%Y/%m/%d", "%Y.%m.%d"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Nodes,
    Text,
    Html,
    Attribute,
    Structured,
    RegexGroup,
    Ai,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeQuery {
    pub selector: String,
    pub trim: bool,
}

impl Default for NodeQuery {
    fn default() -> Self {
        Self {
            selector: String::new(),
            trim: true,
        }
    }
}

impl NodeQuery {
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            ..Self::default()
        }
    }

    pub fn with_trim(mut self, trim: bool) -> Self {
        self.trim = trim;
        self
    }

    pub fn one(&self) -> Option<String> {
        None
    }

    pub fn all(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn text(&self) -> ValueQuery {
        ValueQuery::new(Kind::Text, self.selector.clone()).with_trim(self.trim)
    }

    pub fn html(&self) -> ValueQuery {
        ValueQuery::new(Kind::Html, self.selector.clone()).with_trim(false)
    }

    pub fn attr(&self, name: impl Into<String>) -> ValueQuery {
        ValueQuery::new(
            Kind::Attribute,
            format!("{}::attr({})", self.selector, name.into()),
        )
        .with_trim(self.trim)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueQuery {
    pub kind: Kind,
    pub source: String,
    pub trim: bool,
    pub values: Vec<Value>,
}

impl Default for ValueQuery {
    fn default() -> Self {
        Self {
            kind: Kind::Structured,
            source: String::new(),
            trim: false,
            values: Vec::new(),
        }
    }
}

impl ValueQuery {
    pub fn new(kind: Kind, source: impl Into<String>) -> Self {
        Self {
            kind,
            source: source.into(),
            trim: matches!(
                kind,
                Kind::Text | Kind::Attribute | Kind::RegexGroup | Kind::Ai
            ),
            values: Vec::new(),
        }
    }

    pub fn with_trim(mut self, trim: bool) -> Self {
        self.trim = trim;
        self
    }

    pub fn with_values(mut self, values: Vec<Value>) -> Self {
        self.values = values;
        self
    }

    pub fn fallback(self, fallback: Self) -> Self {
        if self.has_non_empty_values() {
            self
        } else {
            fallback
        }
    }

    pub fn fallback_many(self, fallbacks: impl IntoIterator<Item = Self>) -> Self {
        fallbacks
            .into_iter()
            .fold(self, |current, fallback| current.fallback(fallback))
    }

    pub fn join(mut self, delimiter: impl AsRef<str>) -> Self {
        if self.values.is_empty() {
            return self;
        }

        let joined = self
            .values
            .iter()
            .filter_map(|value| stringify(value, self.trim))
            .collect::<Vec<_>>()
            .join(delimiter.as_ref());

        self.kind = Kind::Text;
        self.trim = false;
        self.values = vec![Value::String(joined)];
        self
    }

    pub fn compact(mut self) -> Self {
        let trim = self.trim;
        self.values = self
            .values
            .into_iter()
            .filter(|value| query_value_is_present(value, trim))
            .collect();
        self
    }

    pub fn trim(mut self) -> Self {
        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .map(|value| map_query_value_strings(value, &|text| text.trim().to_string()))
            .collect();
        self
    }

    pub fn first_non_empty(mut self) -> Self {
        let trim = self.trim;
        self.values = self
            .values
            .into_iter()
            .find(|value| query_value_is_present(value, trim))
            .into_iter()
            .collect();
        self
    }

    pub fn require_non_empty(mut self) -> Result<Self, SpiderError> {
        let trim = self.trim;
        self.values = self
            .values
            .into_iter()
            .filter(|value| query_value_is_present(value, trim))
            .collect();

        if self.values.is_empty() {
            return Err(SpiderError::parse(format!(
                "query {} expected at least one non-empty value",
                self.source
            )));
        }

        Ok(self)
    }

    pub fn require_one(mut self) -> Result<Self, SpiderError> {
        let trim = self.trim;
        self.values = self
            .values
            .into_iter()
            .filter(|value| query_value_is_present(value, trim))
            .collect();

        match self.values.len() {
            1 => Ok(self),
            0 => Err(SpiderError::parse(format!(
                "query {} expected exactly one non-empty value, got 0",
                self.source
            ))),
            count => Err(SpiderError::parse(format!(
                "query {} expected exactly one non-empty value, got {}",
                self.source, count
            ))),
        }
    }

    pub fn field(mut self, name: impl AsRef<str>) -> Self {
        let name = name.as_ref().to_string();
        self.kind = Kind::Structured;
        self.trim = false;
        self.source = format!("{}.{}", self.source, name);
        self.values = self
            .values
            .into_iter()
            .flat_map(|value| extract_query_value_field(value, &name))
            .collect();
        self
    }

    pub fn index(mut self, index: usize) -> Self {
        self.kind = Kind::Structured;
        self.trim = false;
        self.source = format!("{}[{index}]", self.source);
        self.values = self
            .values
            .into_iter()
            .filter_map(|value| extract_query_value_index(value, index))
            .collect();
        self
    }

    pub fn flatten(mut self) -> Self {
        self.kind = Kind::Structured;
        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .flat_map(flatten_query_value)
            .collect();
        self
    }

    pub fn skip(mut self, count: usize) -> Self {
        self.values = self.values.into_iter().skip(count).collect();
        self
    }

    pub fn take(mut self, limit: usize) -> Self {
        self.values.truncate(limit);
        self
    }

    pub fn last(mut self) -> Self {
        self.values = self.values.pop().into_iter().collect();
        self
    }

    pub fn dedup(mut self) -> Self {
        let mut unique_values = Vec::with_capacity(self.values.len());

        for value in self.values {
            if !unique_values.iter().any(|existing| existing == &value) {
                unique_values.push(value);
            }
        }

        self.values = unique_values;
        self
    }

    pub fn normalize_whitespace(mut self) -> Self {
        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .map(|value| map_query_value_strings(value, &normalize_whitespace_text))
            .collect();
        self
    }

    pub fn split(mut self, delimiter: impl AsRef<str>) -> Result<Self, SpiderError> {
        let source = self.source.clone();
        let trim = self.trim;
        let delimiter = delimiter.as_ref().to_string();
        if delimiter.is_empty() {
            return Err(SpiderError::parse(format!(
                "query {source} split delimiter cannot be empty"
            )));
        }

        self.kind = Kind::Text;
        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .enumerate()
            .map(|(index, value)| split_query_value_string(value, &source, index, trim, &delimiter))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(self)
    }

    pub fn replace(mut self, from: impl AsRef<str>, to: impl AsRef<str>) -> Self {
        let from = from.as_ref().to_string();
        let to = to.as_ref().to_string();
        let transform = |text: &str| text.replace(&from, &to);

        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .map(|value| map_query_value_strings(value, &transform))
            .collect();
        self
    }

    pub fn resolve_url(mut self, base_url: impl AsRef<str>) -> Result<Self, SpiderError> {
        let source = self.source.clone();
        let trim = self.trim;
        let base_url_text = base_url.as_ref().to_string();
        let base_url = Url::parse(&base_url_text).map_err(|error| {
            SpiderError::parse(format!(
                "query {source} cannot resolve URLs against invalid base {base_url_text:?}: {error}"
            ))
        })?;

        self.kind = Kind::Text;
        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .enumerate()
            .map(|(index, value)| resolve_query_value_url(value, &base_url, &source, index, trim))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self)
    }

    pub fn parse_number(mut self) -> Result<Self, SpiderError> {
        let source = self.source.clone();
        self.kind = Kind::Structured;
        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .enumerate()
            .map(|(index, value)| parse_query_value_number(value, &source, index))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self)
    }

    pub fn parse_bool(mut self) -> Result<Self, SpiderError> {
        let source = self.source.clone();
        self.kind = Kind::Structured;
        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .enumerate()
            .map(|(index, value)| parse_query_value_bool(value, &source, index))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self)
    }

    pub fn parse_json(mut self) -> Result<Self, SpiderError> {
        let source = self.source.clone();
        let trim = self.trim;
        self.kind = Kind::Structured;
        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .enumerate()
            .map(|(index, value)| parse_query_value_json(value, &source, index, trim))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self)
    }

    pub fn parse_datetime(mut self) -> Result<Self, SpiderError> {
        let source = self.source.clone();
        let trim = self.trim;
        self.kind = Kind::Text;
        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .enumerate()
            .map(|(index, value)| parse_query_value_datetime(value, &source, index, trim))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self)
    }

    pub fn parse_datetime_with_format(
        mut self,
        format: impl AsRef<str>,
    ) -> Result<Self, SpiderError> {
        let source = self.source.clone();
        let trim = self.trim;
        let format = format.as_ref().to_string();
        self.kind = Kind::Text;
        self.trim = false;
        self.values = self
            .values
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                parse_query_value_datetime_with_format(value, &source, index, trim, &format)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self)
    }

    pub fn one(&self) -> Option<String> {
        self.values
            .first()
            .and_then(|value| stringify(value, self.trim))
    }

    pub fn all(&self) -> Vec<String> {
        self.values
            .iter()
            .filter_map(|value| stringify(value, self.trim))
            .collect()
    }

    pub fn value(&self) -> Option<Value> {
        self.values.first().cloned()
    }

    pub fn group(&self, index: usize) -> Option<String> {
        self.values
            .get(index)
            .and_then(|value| stringify(value, self.trim))
    }

    fn has_non_empty_values(&self) -> bool {
        self.values
            .iter()
            .any(|value| query_value_is_present(value, self.trim))
    }
}

pub(crate) fn stringify(value: &Value, trim: bool) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => render_array(values),
        Value::Object(values) => render_object(values),
    };

    Some(trim_text(&text, trim))
}

pub(crate) fn trim_text(text: &str, trim: bool) -> String {
    if trim {
        text.trim().to_string()
    } else {
        text.to_string()
    }
}

fn query_value_is_present(value: &Value, trim: bool) -> bool {
    stringify(value, trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false)
}

fn normalize_whitespace_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn map_query_value_strings(value: Value, transform: &dyn Fn(&str) -> String) -> Value {
    match value {
        Value::String(text) => Value::String(transform(&text)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| map_query_value_strings(value, transform))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, map_query_value_strings(value, transform)))
                .collect(),
        ),
        other => other,
    }
}

fn extract_query_value_field(value: Value, name: &str) -> Vec<Value> {
    match value {
        Value::Object(values) => values.get(name).cloned().into_iter().collect(),
        Value::Array(values) => values
            .into_iter()
            .filter_map(|value| match value {
                Value::Object(values) => values.get(name).cloned(),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn extract_query_value_index(value: Value, index: usize) -> Option<Value> {
    match value {
        Value::Array(values) => values.into_iter().nth(index),
        _ => None,
    }
}

fn flatten_query_value(value: Value) -> Vec<Value> {
    match value {
        Value::Array(values) => values,
        other => vec![other],
    }
}

fn parse_query_value_number(
    value: Value,
    source: &str,
    index: usize,
) -> Result<Value, SpiderError> {
    match value {
        Value::Number(value) => Ok(Value::Number(value)),
        Value::String(text) => {
            let normalized = text.trim();
            if normalized.is_empty() {
                return Err(SpiderError::parse(format!(
                    "failed to parse query value from {source}[{index}] as number: empty string"
                )));
            }

            normalized
                .parse::<f64>()
                .map(Value::Number)
                .map_err(|error| {
                    SpiderError::parse(format!(
                        "failed to parse query value from {source}[{index}] as number: {error}"
                    ))
                })
        }
        other => Err(SpiderError::parse(format!(
            "failed to parse query value from {source}[{index}] as number: expected string or number, got {}",
            value_type_name(&other)
        ))),
    }
}

fn parse_query_value_bool(value: Value, source: &str, index: usize) -> Result<Value, SpiderError> {
    match value {
        Value::Bool(value) => Ok(Value::Bool(value)),
        Value::String(text) => {
            let normalized = text.trim();
            if normalized.is_empty() {
                return Err(SpiderError::parse(format!(
                    "failed to parse query value from {source}[{index}] as bool: empty string"
                )));
            }

            parse_bool_text(normalized)
                .map(Value::Bool)
                .ok_or_else(|| {
                    SpiderError::parse(format!(
                        "failed to parse query value from {source}[{index}] as bool: expected true/false/1/0, got {normalized:?}"
                    ))
                })
        }
        other => Err(SpiderError::parse(format!(
            "failed to parse query value from {source}[{index}] as bool: expected string or bool, got {}",
            value_type_name(&other)
        ))),
    }
}

fn resolve_query_value_url(
    value: Value,
    base_url: &Url,
    source: &str,
    index: usize,
    trim: bool,
) -> Result<Value, SpiderError> {
    let Value::String(text) = value else {
        return Err(SpiderError::parse(format!(
            "failed to resolve query value from {source}[{index}] as URL: expected string, got {}",
            value_type_name(&value)
        )));
    };

    let candidate = trim_text(&text, trim);
    if candidate.is_empty() {
        return Err(SpiderError::parse(format!(
            "failed to resolve query value from {source}[{index}] as URL: empty string"
        )));
    }

    if let Ok(url) = Url::parse(&candidate) {
        return Ok(Value::String(url.to_string()));
    }

    base_url
        .join(&candidate)
        .map(|url| Value::String(url.to_string()))
        .map_err(|error| {
            SpiderError::parse(format!(
                "failed to resolve query value from {source}[{index}] against base {:?}: {error}",
                base_url.as_str()
            ))
        })
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

fn split_query_value_string(
    value: Value,
    source: &str,
    index: usize,
    trim: bool,
    delimiter: &str,
) -> Result<Vec<Value>, SpiderError> {
    let text = match value {
        Value::String(text) => text,
        other => {
            return Err(SpiderError::parse(format!(
                "failed to split query value from {source}[{index}]: expected string, got {}",
                value_type_name(&other)
            )));
        }
    };

    let candidate = trim_text(&text, trim);
    Ok(candidate
        .split(delimiter)
        .map(|part| Value::String(trim_text(part, trim)))
        .collect())
}

fn parse_query_value_json(
    value: Value,
    source: &str,
    index: usize,
    trim: bool,
) -> Result<Value, SpiderError> {
    let text = match value {
        Value::String(text) => text,
        other => return Ok(other),
    };

    let candidate = trim_text(&text, trim);
    if candidate.is_empty() {
        return Err(SpiderError::parse(format!(
            "failed to parse query value from {source}[{index}] as json: empty string"
        )));
    }

    serde_json::from_str::<serde_json::Value>(&candidate)
        .map(Value::from)
        .map_err(|error| {
            SpiderError::parse(format!(
                "failed to parse query value from {source}[{index}] as json: {error}"
            ))
        })
}

fn parse_query_value_datetime(
    value: Value,
    source: &str,
    index: usize,
    trim: bool,
) -> Result<Value, SpiderError> {
    let Value::String(text) = value else {
        return Err(SpiderError::parse(format!(
            "failed to parse query value from {source}[{index}] as datetime: expected string, got {}",
            value_type_name(&value)
        )));
    };

    let candidate = trim_text(&text, trim);
    if candidate.is_empty() {
        return Err(SpiderError::parse(format!(
            "failed to parse query value from {source}[{index}] as datetime: empty string"
        )));
    }

    parse_datetime_text(&candidate)
        .map(Value::String)
        .map_err(|message| {
            SpiderError::parse(format!(
                "failed to parse query value from {source}[{index}] as datetime: {message}"
            ))
        })
}

fn parse_query_value_datetime_with_format(
    value: Value,
    source: &str,
    index: usize,
    trim: bool,
    format: &str,
) -> Result<Value, SpiderError> {
    let Value::String(text) = value else {
        return Err(SpiderError::parse(format!(
            "failed to parse query value from {source}[{index}] as datetime with format {format:?}: expected string, got {}",
            value_type_name(&value)
        )));
    };

    let candidate = trim_text(&text, trim);
    if candidate.is_empty() {
        return Err(SpiderError::parse(format!(
            "failed to parse query value from {source}[{index}] as datetime with format {format:?}: empty string"
        )));
    }

    parse_datetime_text_with_format(&candidate, format)
        .map(Value::String)
        .map_err(|message| {
            SpiderError::parse(format!(
                "failed to parse query value from {source}[{index}] as datetime with format {format:?}: {message}"
            ))
        })
}

fn parse_datetime_text(text: &str) -> Result<String, String> {
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

fn parse_datetime_text_with_format(text: &str, format: &str) -> Result<String, String> {
    if let Ok(zoned) = Zoned::strptime(format, text) {
        return Ok(zoned.timestamp().to_string());
    }

    if let Ok(datetime) = DateTime::strptime(format, text) {
        return Ok(datetime.to_string());
    }

    if let Ok(date) = Date::strptime(format, text) {
        return Ok(date.to_string());
    }

    Err(format!("input did not match format {format:?}"))
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn render_array(values: &[Value]) -> String {
    let rendered = values
        .iter()
        .filter_map(|value| stringify(value, false))
        .map(|value| format!("\"{}\"", escape_json(&value)))
        .collect::<Vec<_>>()
        .join(",");

    format!("[{rendered}]")
}

fn render_object(values: &BTreeMap<String, Value>) -> String {
    let rendered = values
        .iter()
        .filter_map(|(key, value)| stringify(value, false).map(|value| (key, value)))
        .map(|(key, value)| format!("\"{}\":\"{}\"", escape_json(key), escape_json(&value)))
        .collect::<Vec<_>>()
        .join(",");

    format!("{{{rendered}}}")
}

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_query_trims_by_default() {
        let query = NodeQuery::new("h1.title")
            .text()
            .with_values(vec![Value::String("  hello  ".to_string())]);

        assert_eq!(query.one().as_deref(), Some("hello"));
    }

    #[test]
    fn structured_values_do_not_trim_by_default() {
        let query = ValueQuery::new(Kind::Structured, "$.data")
            .with_values(vec![Value::String("  raw  ".to_string())]);

        assert_eq!(query.one().as_deref(), Some("  raw  "));
    }

    #[test]
    fn node_query_attr_keeps_trim_hook() {
        let query = NodeQuery::new("a.link")
            .attr("href")
            .with_values(vec![Value::String(" /detail ".to_string())]);

        assert_eq!(query.one().as_deref(), Some("/detail"));
    }

    #[test]
    fn value_query_join_concatenates_stringified_values() {
        let query = ValueQuery::new(Kind::Text, "p.content").with_values(vec![
            Value::String("Hello".to_string()),
            Value::String("World".to_string()),
        ]);

        assert_eq!(query.clone().join("").one().as_deref(), Some("HelloWorld"));
        assert_eq!(query.join(" ").one().as_deref(), Some("Hello World"));
    }

    #[test]
    fn value_query_compact_drops_empty_values() {
        let query = ValueQuery::new(Kind::Text, "p.content").with_values(vec![
            Value::Null,
            Value::String("   ".to_string()),
            Value::String("A".to_string()),
            Value::String(" B ".to_string()),
        ]);

        assert_eq!(
            query.compact().all(),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn value_query_trim_trims_nested_string_values() {
        let query = ValueQuery::new(Kind::Structured, "$.data").with_values(vec![Value::Object(
            [(
                "tags".to_string(),
                Value::Array(vec![
                    Value::String("  first  ".to_string()),
                    Value::String(" second ".to_string()),
                ]),
            )]
            .into_iter()
            .collect(),
        )]);

        assert_eq!(
            query.trim().value(),
            Some(Value::Object(
                [(
                    "tags".to_string(),
                    Value::Array(vec![
                        Value::String("first".to_string()),
                        Value::String("second".to_string()),
                    ]),
                )]
                .into_iter()
                .collect()
            ))
        );
    }

    #[test]
    fn value_query_first_non_empty_skips_empty_values() {
        let query = ValueQuery::new(Kind::Text, "p.content").with_values(vec![
            Value::Null,
            Value::String("  ".to_string()),
            Value::String("Headline".to_string()),
            Value::String("Backup".to_string()),
        ]);

        assert_eq!(query.first_non_empty().one().as_deref(), Some("Headline"));
    }

    #[test]
    fn value_query_require_non_empty_returns_compacted_values() {
        let query = ValueQuery::new(Kind::Text, "p.content").with_values(vec![
            Value::Null,
            Value::String("  ".to_string()),
            Value::String("Body".to_string()),
        ]);

        assert_eq!(
            query.require_non_empty().unwrap().all(),
            vec!["Body".to_string()]
        );
    }

    #[test]
    fn value_query_require_non_empty_returns_error_when_all_values_are_empty() {
        let query = ValueQuery::new(Kind::Text, "p.content")
            .with_values(vec![Value::Null, Value::String(" ".to_string())]);

        assert_eq!(
            query.require_non_empty().unwrap_err(),
            SpiderError::parse("query p.content expected at least one non-empty value".to_string())
        );
    }

    #[test]
    fn value_query_require_one_keeps_the_only_non_empty_value() {
        let query = ValueQuery::new(Kind::Text, "h1.title")
            .with_values(vec![Value::Null, Value::String(" Headline ".to_string())]);

        assert_eq!(
            query.require_one().unwrap().one().as_deref(),
            Some("Headline")
        );
    }

    #[test]
    fn value_query_require_one_returns_error_when_no_values_match() {
        let query =
            ValueQuery::new(Kind::Text, "h1.title").with_values(vec![Value::String(" ".into())]);

        assert_eq!(
            query.require_one().unwrap_err(),
            SpiderError::parse(
                "query h1.title expected exactly one non-empty value, got 0".to_string()
            )
        );
    }

    #[test]
    fn value_query_require_one_returns_error_when_multiple_values_match() {
        let query = ValueQuery::new(Kind::Text, "h1.title").with_values(vec![
            Value::String("Headline".to_string()),
            Value::String("Backup".to_string()),
        ]);

        assert_eq!(
            query.require_one().unwrap_err(),
            SpiderError::parse(
                "query h1.title expected exactly one non-empty value, got 2".to_string()
            )
        );
    }

    #[test]
    fn value_query_field_extracts_object_fields() {
        let query = ValueQuery::new(Kind::Structured, "$.data").with_values(vec![Value::Object(
            [("title".to_string(), Value::String("Post".to_string()))]
                .into_iter()
                .collect(),
        )]);
        let title = query.field("title");

        assert_eq!(title.one().as_deref(), Some("Post"));
        assert_eq!(title.source, "$.data.title");
    }

    #[test]
    fn value_query_field_extracts_fields_from_array_of_objects() {
        let query =
            ValueQuery::new(Kind::Structured, "$.items").with_values(vec![Value::Array(vec![
                Value::Object(
                    [("title".to_string(), Value::String("First".to_string()))]
                        .into_iter()
                        .collect(),
                ),
                Value::Object(
                    [("title".to_string(), Value::String("Second".to_string()))]
                        .into_iter()
                        .collect(),
                ),
            ])]);

        assert_eq!(
            query.field("title").all(),
            vec!["First".to_string(), "Second".to_string()]
        );
    }

    #[test]
    fn value_query_index_extracts_array_item() {
        let query =
            ValueQuery::new(Kind::Structured, "$.items").with_values(vec![Value::Array(vec![
                Value::String("First".to_string()),
                Value::String("Second".to_string()),
            ])]);
        let item = query.index(1);

        assert_eq!(item.one().as_deref(), Some("Second"));
        assert_eq!(item.source, "$.items[1]");
    }

    #[test]
    fn value_query_index_can_chain_into_field() {
        let query =
            ValueQuery::new(Kind::Structured, "$.items").with_values(vec![Value::Array(vec![
                Value::Object(
                    [("title".to_string(), Value::String("First".to_string()))]
                        .into_iter()
                        .collect(),
                ),
                Value::Object(
                    [("title".to_string(), Value::String("Second".to_string()))]
                        .into_iter()
                        .collect(),
                ),
            ])]);

        assert_eq!(
            query.index(1).field("title").one().as_deref(),
            Some("Second")
        );
    }

    #[test]
    fn value_query_skip_take_and_last_slice_results_in_order() {
        let query = ValueQuery::new(Kind::Text, "$.items").with_values(vec![
            Value::String("first".to_string()),
            Value::String("second".to_string()),
            Value::String("third".to_string()),
            Value::String("fourth".to_string()),
        ]);

        assert_eq!(
            query.clone().skip(1).take(2).all(),
            vec!["second".to_string(), "third".to_string()]
        );
        assert_eq!(query.last().one().as_deref(), Some("fourth"));
    }

    #[test]
    fn value_query_dedup_preserves_first_occurrence_order() {
        let query = ValueQuery::new(Kind::Text, "$.items").with_values(vec![
            Value::String("alpha".to_string()),
            Value::String("beta".to_string()),
            Value::String("alpha".to_string()),
            Value::String("gamma".to_string()),
            Value::String("beta".to_string()),
        ]);

        assert_eq!(
            query.dedup().all(),
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
    }

    #[test]
    fn value_query_flatten_expands_top_level_arrays() {
        let query =
            ValueQuery::new(Kind::Structured, "$.items").with_values(vec![Value::Array(vec![
                Value::String("First".to_string()),
                Value::String("Second".to_string()),
            ])]);

        assert_eq!(
            query.flatten().all(),
            vec!["First".to_string(), "Second".to_string()]
        );
    }

    #[test]
    fn value_query_flatten_can_chain_into_parse_number() {
        let query =
            ValueQuery::new(Kind::Structured, "$.counts").with_values(vec![Value::Array(vec![
                Value::String("1".to_string()),
                Value::String("2".to_string()),
            ])]);

        assert_eq!(
            query.flatten().parse_number().unwrap().values,
            vec![Value::Number(1.0), Value::Number(2.0)]
        );
    }

    #[test]
    fn value_query_normalize_whitespace_collapses_runs() {
        let query = ValueQuery::new(Kind::Text, "p.content")
            .with_trim(false)
            .with_values(vec![Value::String(" A \n\t  B   C ".to_string())]);

        assert_eq!(query.normalize_whitespace().one().as_deref(), Some("A B C"));
    }

    #[test]
    fn value_query_replace_rewrites_string_values() {
        let query = ValueQuery::new(Kind::Text, "p.content")
            .with_values(vec![Value::String("line1<br>line2<br>line3".to_string())]);

        assert_eq!(
            query.replace("<br>", "\n").one().as_deref(),
            Some("line1\nline2\nline3")
        );
    }

    #[test]
    fn value_query_split_breaks_string_values_into_segments() {
        let query = ValueQuery::new(Kind::Text, "meta.keywords").with_values(vec![Value::String(
            " news, politics , economy ".to_string(),
        )]);

        assert_eq!(
            query.split(",").unwrap().all(),
            vec![
                "news".to_string(),
                "politics".to_string(),
                "economy".to_string()
            ]
        );
    }

    #[test]
    fn value_query_split_rejects_non_string_values() {
        let query = ValueQuery::new(Kind::Structured, "$.keywords")
            .with_values(vec![Value::Array(vec![Value::String("news".to_string())])]);

        assert_eq!(
            query.split(",").unwrap_err(),
            SpiderError::parse(
                "failed to split query value from $.keywords[0]: expected string, got array"
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_resolve_url_joins_relative_strings_against_base() {
        let query = ValueQuery::new(Kind::Attribute, "a::attr(href)")
            .with_values(vec![Value::String(" /2026/03/detail.html ".to_string())]);

        assert_eq!(
            query
                .resolve_url("https://ep.shxwcb.com/2026/03/period.xml")
                .unwrap()
                .one()
                .as_deref(),
            Some("https://ep.shxwcb.com/2026/03/detail.html")
        );
    }

    #[test]
    fn value_query_resolve_url_keeps_absolute_urls() {
        let query =
            ValueQuery::new(Kind::Attribute, "a::attr(href)").with_values(vec![Value::String(
                "https://example.com/news/1".to_string(),
            )]);

        assert_eq!(
            query
                .resolve_url("https://ep.shxwcb.com/2026/03/period.xml")
                .unwrap()
                .one()
                .as_deref(),
            Some("https://example.com/news/1")
        );
    }

    #[test]
    fn value_query_resolve_url_returns_error_for_invalid_base_url() {
        let query = ValueQuery::new(Kind::Attribute, "a::attr(href)")
            .with_values(vec![Value::String("/detail".to_string())]);

        assert_eq!(
            query.resolve_url("::bad-base::").unwrap_err(),
            SpiderError::parse(
                "query a::attr(href) cannot resolve URLs against invalid base \"::bad-base::\": relative URL without a base"
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_resolve_url_returns_error_for_empty_string() {
        let query = ValueQuery::new(Kind::Attribute, "a::attr(href)")
            .with_values(vec![Value::String("   ".to_string())]);

        assert_eq!(
            query
                .resolve_url("https://ep.shxwcb.com/2026/03/period.xml")
                .unwrap_err(),
            SpiderError::parse(
                "failed to resolve query value from a::attr(href)[0] as URL: empty string"
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_resolve_url_returns_error_for_non_string_values() {
        let query =
            ValueQuery::new(Kind::Structured, "$.links").with_values(vec![Value::Array(vec![
                Value::String("/detail".to_string()),
            ])]);

        assert_eq!(
            query
                .resolve_url("https://ep.shxwcb.com/2026/03/period.xml")
                .unwrap_err(),
            SpiderError::parse(
                "failed to resolve query value from $.links[0] as URL: expected string, got array"
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_parse_number_parses_trimmed_strings() {
        let query = ValueQuery::new(Kind::Text, "$.count")
            .with_values(vec![Value::String(" 42 ".to_string())]);

        assert_eq!(
            query.parse_number().unwrap().value(),
            Some(Value::Number(42.0))
        );
    }

    #[test]
    fn value_query_parse_number_keeps_existing_numbers() {
        let query =
            ValueQuery::new(Kind::Structured, "$.score").with_values(vec![Value::Number(3.5)]);

        assert_eq!(
            query.parse_number().unwrap().value(),
            Some(Value::Number(3.5))
        );
    }

    #[test]
    fn value_query_parse_number_returns_error_for_invalid_strings() {
        let query = ValueQuery::new(Kind::Text, "$.count")
            .with_values(vec![Value::String("forty-two".to_string())]);

        assert_eq!(
            query.parse_number().unwrap_err(),
            SpiderError::parse(
                "failed to parse query value from $.count[0] as number: invalid float literal"
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_parse_number_returns_error_for_non_scalar_values() {
        let query = ValueQuery::new(Kind::Structured, "$.count")
            .with_values(vec![Value::Array(vec![Value::String("42".to_string())])]);

        assert_eq!(
            query.parse_number().unwrap_err(),
            SpiderError::parse(
                "failed to parse query value from $.count[0] as number: expected string or number, got array"
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_parse_bool_parses_trimmed_strings() {
        let query = ValueQuery::new(Kind::Text, "$.published")
            .with_values(vec![Value::String(" TRUE ".to_string())]);

        assert_eq!(query.parse_bool().unwrap().value(), Some(Value::Bool(true)));
    }

    #[test]
    fn value_query_parse_bool_keeps_existing_bools() {
        let query =
            ValueQuery::new(Kind::Structured, "$.enabled").with_values(vec![Value::Bool(false)]);

        assert_eq!(
            query.parse_bool().unwrap().value(),
            Some(Value::Bool(false))
        );
    }

    #[test]
    fn value_query_parse_bool_returns_error_for_invalid_strings() {
        let query = ValueQuery::new(Kind::Text, "$.published")
            .with_values(vec![Value::String("maybe".to_string())]);

        assert_eq!(
            query.parse_bool().unwrap_err(),
            SpiderError::parse(
                "failed to parse query value from $.published[0] as bool: expected true/false/1/0, got \"maybe\""
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_parse_bool_returns_error_for_non_scalar_values() {
        let query = ValueQuery::new(Kind::Structured, "$.published")
            .with_values(vec![Value::Object(BTreeMap::new())]);

        assert_eq!(
            query.parse_bool().unwrap_err(),
            SpiderError::parse(
                "failed to parse query value from $.published[0] as bool: expected string or bool, got object"
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_parse_json_parses_embedded_json_text() {
        let query =
            ValueQuery::new(Kind::Text, "script#__NEXT_DATA__").with_values(vec![Value::String(
                r#"{"article":{"title":"Kun","tags":["rust","crawler"]}}"#.to_string(),
            )]);

        assert_eq!(
            query
                .parse_json()
                .unwrap()
                .field("article")
                .field("title")
                .one()
                .as_deref(),
            Some("Kun")
        );
    }

    #[test]
    fn value_query_parse_json_keeps_existing_structured_values() {
        let query =
            ValueQuery::new(Kind::Structured, "$.payload").with_values(vec![Value::Object(
                [("title".to_string(), Value::String("Kun".to_string()))]
                    .into_iter()
                    .collect(),
            )]);

        assert_eq!(
            query.parse_json().unwrap().field("title").one().as_deref(),
            Some("Kun")
        );
    }

    #[test]
    fn value_query_parse_json_returns_error_for_invalid_json_text() {
        let query = ValueQuery::new(Kind::Text, "script.data")
            .with_values(vec![Value::String("{not-json}".to_string())]);

        assert_eq!(
            query.parse_json().unwrap_err(),
            SpiderError::parse(
                "failed to parse query value from script.data[0] as json: key must be a string at line 1 column 2"
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_parse_datetime_normalizes_rfc3339_timestamp_to_utc() {
        let query = ValueQuery::new(Kind::Text, "$.published_at")
            .with_values(vec![Value::String("2026-04-01T08:30:45+08:00".to_string())]);

        assert_eq!(
            query.parse_datetime().unwrap().value(),
            Some(Value::String("2026-04-01T00:30:45Z".to_string()))
        );
    }

    #[test]
    fn value_query_parse_datetime_accepts_common_spaced_datetime_format() {
        let query = ValueQuery::new(Kind::Text, "$.published_at")
            .with_values(vec![Value::String(" 2026-04-01 08:30 ".to_string())]);

        assert_eq!(
            query.parse_datetime().unwrap().value(),
            Some(Value::String("2026-04-01T08:30:00".to_string()))
        );
    }

    #[test]
    fn value_query_parse_datetime_with_format_accepts_custom_layout() {
        let query = ValueQuery::new(Kind::Text, "$.published_at")
            .with_values(vec![Value::String("2026/04/01 08:30".to_string())]);

        assert_eq!(
            query
                .parse_datetime_with_format("%Y/%m/%d %H:%M")
                .unwrap()
                .value(),
            Some(Value::String("2026-04-01T08:30:00".to_string()))
        );
    }

    #[test]
    fn value_query_parse_datetime_returns_error_for_invalid_string() {
        let query = ValueQuery::new(Kind::Text, "$.published_at")
            .with_values(vec![Value::String("tomorrow morning maybe".to_string())]);

        assert_eq!(
            query.parse_datetime().unwrap_err(),
            SpiderError::parse(
                "failed to parse query value from $.published_at[0] as datetime: unsupported datetime format, got \"tomorrow morning maybe\""
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_parse_datetime_returns_error_for_non_string_values() {
        let query = ValueQuery::new(Kind::Structured, "$.published_at")
            .with_values(vec![Value::Number(1_775_004_645.0)]);

        assert_eq!(
            query.parse_datetime().unwrap_err(),
            SpiderError::parse(
                "failed to parse query value from $.published_at[0] as datetime: expected string, got number"
                    .to_string()
            )
        );
    }

    #[test]
    fn value_query_fallback_uses_second_query_when_first_is_empty() {
        let primary = ValueQuery::new(Kind::Text, "h1.title")
            .with_values(vec![Value::String("".to_string())]);
        let fallback = ValueQuery::new(Kind::Text, "title")
            .with_values(vec![Value::String("Document Title".to_string())]);

        assert_eq!(
            primary.fallback(fallback).one().as_deref(),
            Some("Document Title")
        );
    }

    #[test]
    fn value_query_fallback_keeps_first_query_when_non_empty() {
        let primary = ValueQuery::new(Kind::Text, "h1.title")
            .with_values(vec![Value::String("Headline".to_string())]);
        let fallback = ValueQuery::new(Kind::Text, "title")
            .with_values(vec![Value::String("Document Title".to_string())]);

        assert_eq!(
            primary.fallback(fallback).one().as_deref(),
            Some("Headline")
        );
    }

    #[test]
    fn value_query_fallback_many_uses_first_non_empty_query_in_order() {
        let primary = ValueQuery::new(Kind::Text, "h1.title")
            .with_values(vec![Value::String("".to_string())]);
        let secondary =
            ValueQuery::new(Kind::Text, "title").with_values(vec![Value::String("".to_string())]);
        let tertiary = ValueQuery::new(Kind::Text, "meta[property='og:title']")
            .with_values(vec![Value::String("Document Title".to_string())]);

        assert_eq!(
            primary
                .fallback_many([secondary, tertiary])
                .one()
                .as_deref(),
            Some("Document Title")
        );
    }

    #[test]
    fn value_query_fallback_many_keeps_last_query_when_all_are_empty() {
        let primary = ValueQuery::new(Kind::Text, "h1.title")
            .with_values(vec![Value::String("".to_string())]);
        let secondary =
            ValueQuery::new(Kind::Text, "title").with_values(vec![Value::String("".to_string())]);
        let tertiary = ValueQuery::new(Kind::Text, "meta[property='og:title']")
            .with_values(vec![Value::String("".to_string())]);

        assert_eq!(
            primary.fallback_many([secondary, tertiary]).source,
            "meta[property='og:title']"
        );
    }
}
