use std::borrow::Cow;

pub use fastrace::collector::{Config, ConsoleReporter};
pub use fastrace::prelude::{Span, SpanContext};
pub use fastrace::{flush, set_reporter};

pub type Property = (Cow<'static, str>, Cow<'static, str>);
pub type Properties = Vec<Property>;

/// Initialize the built-in console reporter for `fastrace`.
///
/// Call this once during application startup if you want trace output printed
/// to stderr. For OpenTelemetry Collector deployment, use
/// `trace::set_reporter(...)` with a `fastrace-opentelemetry` reporter instead.
pub fn init_console() {
    set_reporter(ConsoleReporter, Config::default());
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

pub fn debug(name: &'static str, properties: Properties) {
    log("debug", name, properties);
}

pub fn warn(name: &'static str, properties: Properties) {
    log("warn", name, properties);
}

pub fn error(name: &'static str, properties: Properties) {
    log("error", name, properties);
}
