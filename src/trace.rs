use std::borrow::Cow;

use fastrace::collector::{Reporter, SpanRecord};

pub use fastrace::collector::Config;
pub use fastrace::prelude::{Span, SpanContext};
pub use fastrace::{flush, set_reporter};

pub type Property = (Cow<'static, str>, Cow<'static, str>);
pub type Properties = Vec<Property>;

/// Initialize a compact single-line console reporter for `fastrace`.
///
/// Call this once during application startup if you want runtime logs printed
/// to stderr in a concise `level name key=value` format. For OpenTelemetry
/// Collector deployment, use
/// `trace::set_reporter(...)` with a `fastrace-opentelemetry` reporter instead.
pub fn init_console() {
    set_reporter(CompactReporter, Config::default());
}

pub fn prop(key: &'static str, value: impl ToString) -> Property {
    (Cow::Borrowed(key), Cow::Owned(value.to_string()))
}

pub fn log(level: &'static str, name: &'static str, mut properties: Properties) {
    properties.push(prop("level", level));
    let _span = Span::root(name, SpanContext::random()).with_properties(|| properties);
}

pub fn info(name: &'static str, properties: Properties) {
    log("info", name, properties);
}

pub fn warn(name: &'static str, properties: Properties) {
    log("warn", name, properties);
}

pub fn error(name: &'static str, properties: Properties) {
    log("error", name, properties);
}

struct CompactReporter;

impl Reporter for CompactReporter {
    fn report(&mut self, spans: Vec<SpanRecord>) {
        for span in spans {
            eprintln!("{}", format_span_line(&span));
        }
    }
}

fn format_span_line(span: &SpanRecord) -> String {
    let level = property(span.properties.as_slice(), "level").unwrap_or("info");
    let fields = format_properties(span.properties.as_slice());
    if fields.is_empty() {
        format!("{level} {}", span.name)
    } else {
        format!("{level} {} {fields}", span.name)
    }
}

fn format_properties(properties: &[(Cow<'static, str>, Cow<'static, str>)]) -> String {
    properties
        .iter()
        .filter(|(key, _)| key.as_ref() != "level")
        .map(|(key, value)| format!("{}={}", key, format_value(value)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn property<'a>(
    properties: &'a [(Cow<'static, str>, Cow<'static, str>)],
    key: &str,
) -> Option<&'a str> {
    properties
        .iter()
        .find(|(current, _)| current.as_ref() == key)
        .map(|(_, value)| value.as_ref())
}

fn format_value(value: &str) -> Cow<'_, str> {
    if value.is_empty()
        || value
            .chars()
            .any(|char| char.is_whitespace() || matches!(char, '"' | '\'' | '='))
    {
        Cow::Owned(format!("{value:?}"))
    } else {
        Cow::Borrowed(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastrace::collector::{SpanContext, SpanId, TraceId};

    fn span(name: &'static str, properties: &[(&'static str, &'static str)]) -> SpanRecord {
        SpanRecord {
            trace_id: TraceId(1),
            span_id: SpanId(2),
            parent_id: SpanId(0),
            begin_time_unix_ns: 0,
            duration_ns: 0,
            name: Cow::Borrowed(name),
            properties: properties
                .iter()
                .map(|(key, value)| (Cow::Borrowed(*key), Cow::Borrowed(*value)))
                .collect(),
            events: Vec::new(),
            links: vec![SpanContext::random()],
        }
    }

    #[test]
    fn formats_compact_line() {
        let line = format_span_line(&span(
            "request.ok",
            &[
                ("url", "https://example.com"),
                ("status", "200"),
                ("level", "info"),
            ],
        ));
        assert_eq!(line, "info request.ok url=https://example.com status=200");
    }

    #[test]
    fn quotes_values_with_spaces() {
        let line = format_span_line(&span(
            "request.fail",
            &[("error", "connection reset by peer"), ("level", "warn")],
        ));
        assert_eq!(line, "warn request.fail error=\"connection reset by peer\"");
    }
}
