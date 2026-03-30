use crate::error::SpiderError;
use crate::middleware::{Map as MiddlewareMap, MiddlewareConfig, MiddlewareType};
use crate::request::browser::{Config as BrowserConfig, Driver, Engine, Viewport};
use crate::request::http::Config as HttpConfig;
use crate::request::{Headers, RequestMode};
use crate::rules::schema::{
    Compiled, CompiledStep, Dsl, FetchConfig, FetchPlan, FieldConfig, FieldPlan, LinkConfig,
    LinkPlan, ParseConfig, ParsePlan, SelectorKind, SourceKind, StepConfig,
};
use crate::rules::validate::validate_rules;
use crate::runtime::{Config as RuntimeConfig, merge as merge_runtime};
use crate::validator::{ValidationPlan, ValidationRule, ValidationType};
use crate::value::Value;
use std::collections::BTreeMap;

pub fn compile_rules(value: Value) -> Result<Compiled, SpiderError> {
    let normalized = normalize(value)?;
    validate_rules(&normalized)?;
    let dsl = parse_dsl(&normalized)?;

    Ok(Compiled {
        steps: dsl
            .steps
            .into_iter()
            .map(compile_step)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn normalize(value: Value) -> Result<Value, SpiderError> {
    match value {
        Value::String(content) => serde_json::from_str::<serde_json::Value>(&content)
            .map(Value::from)
            .map_err(|error| SpiderError::rules(format!("invalid rules json: {error}"))),
        other => Ok(other),
    }
}

fn parse_dsl(value: &Value) -> Result<Dsl, SpiderError> {
    let root = value
        .as_object()
        .ok_or_else(|| SpiderError::rules("rules dsl must be an object"))?;
    let steps = root
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| SpiderError::rules("rules.steps must be an array"))?;

    Ok(Dsl {
        steps: steps
            .iter()
            .map(parse_step)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn parse_step(value: &Value) -> Result<StepConfig, SpiderError> {
    let step = value
        .as_object()
        .ok_or_else(|| SpiderError::rules("rules.steps[*] must be an object"))?;

    Ok(StepConfig {
        id: required_string(step, "id")?.to_string(),
        r#type: optional_string(step, "type").map(str::to_string),
        callback: optional_string(step, "callback").map(str::to_string),
        fetch: parse_fetch(step.get("fetch"))?,
        parse: parse_parse(step.get("parse"))?,
        validate: parse_list(step.get("validate"), parse_validation)?,
        route: optional_map(step, "route"),
        output: optional_map(step, "output"),
        runtime: optional_map(step, "runtime"),
        middlewares: parse_step_middlewares(step.get("MIDDLEWARES"))?,
        meta: step.get("meta").and_then(|v| v.as_object().cloned()),
        dedup: parse_dedup(step.get("dedup"))?,
        schedule: parse_schedule(step.get("schedule"))?,
        retry: parse_retry(step.get("retry"))?,
    })
}

fn parse_fetch(value: Option<&Value>) -> Result<FetchConfig, SpiderError> {
    let Some(value) = value else {
        return Ok(FetchConfig::default());
    };
    let fetch = value
        .as_object()
        .ok_or_else(|| SpiderError::rules("step fetch must be an object"))?;

    Ok(FetchConfig {
        mode: optional_string(fetch, "mode").map(str::to_string),
        request: optional_map_value(fetch, "request"),
        browser: optional_map_value(fetch, "browser"),
    })
}

fn parse_parse(value: Option<&Value>) -> Result<ParseConfig, SpiderError> {
    let Some(value) = value else {
        return Ok(ParseConfig::default());
    };
    let parse = value
        .as_object()
        .ok_or_else(|| SpiderError::rules("step parse must be an object"))?;

    Ok(ParseConfig {
        fields: parse_list(parse.get("fields"), parse_field)?,
        links: parse_list(parse.get("links"), parse_link)?,
        next_url_config: optional_map(parse, "next_url_config"),
    })
}

fn parse_field(value: &Value) -> Result<FieldConfig, SpiderError> {
    let field = value
        .as_object()
        .ok_or_else(|| SpiderError::rules("parse.fields[*] must be an object"))?;

    Ok(FieldConfig {
        name: required_string(field, "name")?.to_string(),
        source: required_string(field, "source")?.to_string(),
        selector_type: required_string(field, "selector_type")?.to_string(),
        selector: string_list(field.get("selector"), "parse.fields[*].selector")?,
        attribute: optional_string(field, "attribute")
            .unwrap_or("text")
            .to_string(),
        required: field
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        default: field.get("default").cloned().unwrap_or(Value::Null),
        multiple: field
            .get("multiple")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        options: optional_map_value(field, "options"),
    })
}

fn parse_link(value: &Value) -> Result<LinkConfig, SpiderError> {
    let link = value
        .as_object()
        .ok_or_else(|| SpiderError::rules("parse.links[*] must be an object"))?;

    Ok(LinkConfig {
        name: required_string(link, "name")?.to_string(),
        source: required_string(link, "source")?.to_string(),
        selector_type: required_string(link, "selector_type")?.to_string(),
        selector: string_list(link.get("selector"), "parse.links[*].selector")?,
        attribute: optional_string(link, "attribute")
            .unwrap_or("attr:href")
            .to_string(),
        required: link
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        default: link
            .get("default")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
        multiple: link
            .get("multiple")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        allow: string_list_optional(link.get("allow"), "parse.links[*].allow")?,
        deny: string_list_optional(link.get("deny"), "parse.links[*].deny")?,
        next_step: optional_string(link, "next_step").map(str::to_string),
        meta: optional_map_value(link, "meta"),
        options: optional_map_value(link, "options"),
    })
}

fn compile_step(step: StepConfig) -> Result<CompiledStep, SpiderError> {
    let derived_runtime = runtime_from_step_options(
        step.dedup.as_ref(),
        step.schedule.as_ref(),
        step.retry.as_ref(),
    );
    let explicit_runtime = compile_runtime(step.runtime)?;

    Ok(CompiledStep {
        id: step.id,
        step_type: step.r#type,
        callback: step.callback,
        fetch: compile_fetch(step.fetch)?,
        parse: compile_parse(step.parse)?,
        validate: step.validate,
        runtime: merge_runtime(&derived_runtime, &explicit_runtime),
        middlewares: compile_middlewares(step.middlewares)?,
        meta: step.meta,
        dedup: step.dedup,
        schedule: step.schedule,
        retry: step.retry,
    })
}

fn compile_fetch(fetch: FetchConfig) -> Result<FetchPlan, SpiderError> {
    let mode = RequestMode::try_from(fetch.mode.as_deref().unwrap_or("http"))
        .map_err(SpiderError::rules)?;
    let method = fetch
        .request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_string();
    let headers = parse_headers(fetch.request.get("headers"))?;
    let body = parse_body(fetch.request.get("body"))?;
    let http = if mode == RequestMode::Http {
        Some(parse_http_config(&fetch.request)?)
    } else {
        None
    };
    let browser = if mode == RequestMode::Browser {
        Some(parse_browser_config(&fetch.browser)?)
    } else {
        None
    };

    Ok(FetchPlan {
        mode,
        method,
        headers,
        body,
        http,
        browser,
    })
}

fn compile_parse(parse: ParseConfig) -> Result<ParsePlan, SpiderError> {
    Ok(ParsePlan {
        fields: parse
            .fields
            .into_iter()
            .map(compile_field)
            .collect::<Result<Vec<_>, _>>()?,
        links: parse
            .links
            .into_iter()
            .map(compile_link)
            .collect::<Result<Vec<_>, _>>()?,
        next_url_config: parse.next_url_config,
    })
}

fn compile_runtime(runtime: BTreeMap<String, Value>) -> Result<RuntimeConfig, SpiderError> {
    Ok(RuntimeConfig {
        schedule: section_map(&runtime, "schedule", "step.runtime.schedule")?,
        retry: section_map(&runtime, "retry", "step.runtime.retry")?,
        dedup: section_map(&runtime, "dedup", "step.runtime.dedup")?,
    })
}

fn parse_validation(value: &Value) -> Result<ValidationPlan, SpiderError> {
    let entry = value
        .as_object()
        .ok_or_else(|| SpiderError::rules("step validate entry must be an object"))?;
    let name = required_string(entry, "name")?.to_string();
    let value_type =
        ValidationType::try_from(required_string(entry, "type")?).map_err(SpiderError::rules)?;
    let rule = entry.get("rule").and_then(Value::as_object);
    let required = rule
        .and_then(|rule| rule.get("required"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(ValidationPlan {
        name,
        value_type,
        rule: ValidationRule { required },
    })
}

fn runtime_from_step_options(
    dedup: Option<&crate::rules::schema::DedupConfig>,
    schedule: Option<&crate::rules::schema::ScheduleConfig>,
    retry: Option<&crate::rules::schema::RetryConfig>,
) -> RuntimeConfig {
    RuntimeConfig {
        schedule: schedule.map(schedule_runtime_map).unwrap_or_default(),
        retry: retry.map(retry_runtime_map).unwrap_or_default(),
        dedup: dedup.map(dedup_runtime_map).unwrap_or_default(),
    }
}

fn dedup_runtime_map(dedup: &crate::rules::schema::DedupConfig) -> BTreeMap<String, Value> {
    let mut map = BTreeMap::new();
    map.insert("enabled".to_string(), Value::Bool(dedup.enabled));
    if !dedup.key.is_empty() {
        map.insert(
            "key".to_string(),
            Value::Array(dedup.key.iter().cloned().map(Value::String).collect()),
        );
    }
    map.insert("ttl".to_string(), Value::Number(dedup.ttl as f64));
    map.insert("scope".to_string(), Value::String(dedup.scope.clone()));
    if let Some(namespace) = &dedup.namespace {
        map.insert("namespace".to_string(), Value::String(namespace.clone()));
    }
    map
}

fn schedule_runtime_map(
    schedule: &crate::rules::schema::ScheduleConfig,
) -> BTreeMap<String, Value> {
    let mut map = BTreeMap::new();
    if let Some(concurrency) = schedule.concurrency {
        map.insert("concurrency".to_string(), Value::Number(concurrency as f64));
    }
    if let Some(interval) = schedule.interval {
        map.insert("interval_ms".to_string(), Value::Number(interval as f64));
    }
    map
}

fn retry_runtime_map(retry: &crate::rules::schema::RetryConfig) -> BTreeMap<String, Value> {
    let mut map = BTreeMap::new();
    map.insert("count".to_string(), Value::Number(retry.count as f64));
    if !retry.http_status.is_empty() {
        map.insert(
            "http_status".to_string(),
            Value::Array(
                retry
                    .http_status
                    .iter()
                    .map(|status| Value::Number(*status as f64))
                    .collect(),
            ),
        );
    }
    if !retry.backoff.is_empty() {
        map.insert(
            "backoff_ms".to_string(),
            Value::Array(
                retry
                    .backoff
                    .iter()
                    .map(|backoff| Value::Number(*backoff as f64))
                    .collect(),
            ),
        );
    }
    map
}

fn compile_field(field: FieldConfig) -> Result<FieldPlan, SpiderError> {
    Ok(FieldPlan {
        source_ref: field.source.clone(),
        name: field.name,
        source: compile_source(&field.source)?,
        selector_type: compile_selector_type(&field.selector_type)?,
        selector: field.selector,
        attribute: field.attribute,
        required: field.required,
        default: field.default,
        multiple: field.multiple,
        options: field.options,
    })
}

fn compile_link(link: LinkConfig) -> Result<LinkPlan, SpiderError> {
    Ok(LinkPlan {
        source_ref: link.source.clone(),
        name: link.name,
        source: compile_source(&link.source)?,
        selector_type: compile_selector_type(&link.selector_type)?,
        selector: link.selector,
        attribute: link.attribute,
        required: link.required,
        default: link.default,
        multiple: link.multiple,
        allow: link.allow,
        deny: link.deny,
        next_step: link.next_step,
        meta: link.meta,
        options: link.options,
    })
}

fn compile_source(value: &str) -> Result<SourceKind, SpiderError> {
    match value {
        "html" => Ok(SourceKind::Html),
        "text" => Ok(SourceKind::Text),
        "json" => Ok(SourceKind::Json),
        "xml" => Ok(SourceKind::Xml),
        "headers" => Ok(SourceKind::Headers),
        "final_url" | "url" => Ok(SourceKind::Url),
        value if value.starts_with("meta.") => Ok(SourceKind::Meta),
        other => Err(SpiderError::rules(format!(
            "unsupported parse source: {other}"
        ))),
    }
}

fn compile_selector_type(value: &str) -> Result<SelectorKind, SpiderError> {
    match value {
        "css" => Ok(SelectorKind::Css),
        "xpath" => Ok(SelectorKind::XPath),
        "json" => Ok(SelectorKind::Json),
        "xml" => Ok(SelectorKind::Xml),
        "regex" => Ok(SelectorKind::Regex),
        "ai" => Ok(SelectorKind::Ai),
        "ocr" => Ok(SelectorKind::Ocr),
        other => Err(SpiderError::rules(format!(
            "unsupported selector_type: {other}"
        ))),
    }
}

fn parse_http_config(value: &BTreeMap<String, Value>) -> Result<HttpConfig, SpiderError> {
    let mut config = HttpConfig::default();

    if let Some(query) = value.get("query") {
        for (key, value) in expect_object(query, "fetch.request.query")? {
            let value = value.as_str().ok_or_else(|| {
                SpiderError::rules(format!("query value for {key} must be string"))
            })?;
            config = config.with_query(key.clone(), value.to_string());
        }
    }

    if let Some(cookies) = value.get("cookies") {
        for (key, value) in expect_object(cookies, "fetch.request.cookies")? {
            let value = value.as_str().ok_or_else(|| {
                SpiderError::rules(format!("cookie value for {key} must be string"))
            })?;
            config = config.with_cookie(key.clone(), value.to_string());
        }
    }

    if let Some(allow_redirects) = value.get("allow_redirects").and_then(Value::as_bool) {
        config = config.with_redirects(allow_redirects);
    }

    Ok(config)
}

fn parse_browser_config(value: &BTreeMap<String, Value>) -> Result<BrowserConfig, SpiderError> {
    let mut config = BrowserConfig::default();

    if let Some(driver) = value.get("driver").and_then(Value::as_str) {
        config = config.with_driver(Driver::try_from(driver).map_err(SpiderError::rules)?);
    }
    if let Some(engine) = value.get("engine").and_then(Value::as_str) {
        config = config.with_engine(Engine::try_from(engine).map_err(SpiderError::rules)?);
    }
    if let Some(headless) = value.get("headless").and_then(Value::as_bool) {
        config = config.with_headless(headless);
    }
    if let Some(stealth) = value.get("stealth").and_then(Value::as_bool) {
        config = config.with_stealth(stealth);
    }
    if let Some(profile) = value.get("fingerprint_profile").and_then(Value::as_str) {
        config = config.with_fingerprint_profile(profile.to_string());
    }
    if let Some(wait_for) = value.get("wait_for").and_then(Value::as_str) {
        config = config.with_wait_for(wait_for.to_string());
    }
    if let Some(viewport) = value.get("viewport") {
        let viewport = expect_object(viewport, "fetch.browser.viewport")?;
        let width = viewport
            .get("width")
            .and_then(Value::as_f64)
            .map(|value| value as u32)
            .unwrap_or(1280);
        let height = viewport
            .get("height")
            .and_then(Value::as_f64)
            .map(|value| value as u32)
            .unwrap_or(720);
        config.viewport = Viewport { width, height };
    }

    Ok(config)
}

fn parse_headers(value: Option<&Value>) -> Result<Headers, SpiderError> {
    let mut headers = Headers::new();
    let Some(value) = value else {
        return Ok(headers);
    };

    for (key, value) in expect_object(value, "fetch.request.headers")? {
        let header_value = value
            .as_str()
            .ok_or_else(|| SpiderError::rules(format!("header value for {key} must be string")))?;
        headers
            .entry(key.clone())
            .or_default()
            .push(header_value.to_string());
    }

    Ok(headers)
}

fn parse_body(value: Option<&Value>) -> Result<Option<Vec<u8>>, SpiderError> {
    let Some(value) = value else {
        return Ok(None);
    };

    match value {
        Value::Null => Ok(None),
        Value::String(value) => Ok(Some(value.as_bytes().to_vec())),
        _ => Err(SpiderError::rules(
            "fetch.request.body must be string or null",
        )),
    }
}

fn required_string<'a>(
    value: &'a BTreeMap<String, Value>,
    key: &str,
) -> Result<&'a str, SpiderError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| SpiderError::rules(format!("missing required field: {key}")))
}

fn optional_string<'a>(value: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn optional_map(value: &BTreeMap<String, Value>, key: &str) -> BTreeMap<String, Value> {
    optional_map_value(value, key)
}

fn optional_map_value(value: &BTreeMap<String, Value>, key: &str) -> BTreeMap<String, Value> {
    value
        .get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn section_map(
    value: &BTreeMap<String, Value>,
    key: &str,
    label: &str,
) -> Result<BTreeMap<String, Value>, SpiderError> {
    let Some(value) = value.get(key) else {
        return Ok(BTreeMap::new());
    };

    expect_object(value, label).cloned()
}

fn parse_step_middlewares(value: Option<&Value>) -> Result<BTreeMap<String, Value>, SpiderError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    value
        .as_object()
        .cloned()
        .ok_or_else(|| SpiderError::rules("step MIDDLEWARES must be an object"))
}

fn compile_middlewares(raw: BTreeMap<String, Value>) -> Result<MiddlewareMap, SpiderError> {
    let mut map = MiddlewareMap::new();

    for (key, value) in raw {
        let entry = value
            .as_object()
            .ok_or_else(|| SpiderError::rules(format!("MIDDLEWARES.{key} must be an object")))?;

        let enabled = entry
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let r#type = match entry
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("download")
        {
            "download" => MiddlewareType::Download,
            "spider" => MiddlewareType::Spider,
            other => {
                return Err(SpiderError::rules(format!(
                    "MIDDLEWARES.{key}.type: unsupported {other}"
                )));
            }
        };
        let order = entry.get("order").and_then(Value::as_f64).unwrap_or(100.0) as i32;
        let options = entry
            .get("options")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        map.insert(
            key,
            MiddlewareConfig {
                enabled,
                r#type,
                order,
                options,
            },
        );
    }

    Ok(map)
}

fn parse_list<T>(
    value: Option<&Value>,
    parse: impl Fn(&Value) -> Result<T, SpiderError>,
) -> Result<Vec<T>, SpiderError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };

    value
        .as_array()
        .ok_or_else(|| SpiderError::rules("parse list must be an array"))?
        .iter()
        .map(parse)
        .collect()
}

fn string_list(value: Option<&Value>, label: &str) -> Result<Vec<String>, SpiderError> {
    let Some(value) = value else {
        return Err(SpiderError::rules(format!("{label} is required")));
    };
    string_list_optional(Some(value), label)
}

fn string_list_optional(value: Option<&Value>, label: &str) -> Result<Vec<String>, SpiderError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };

    value
        .as_array()
        .ok_or_else(|| SpiderError::rules(format!("{label} must be an array")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| SpiderError::rules(format!("{label} entries must be strings")))
        })
        .collect()
}

fn expect_object<'a>(
    value: &'a Value,
    label: &str,
) -> Result<&'a BTreeMap<String, Value>, SpiderError> {
    value
        .as_object()
        .ok_or_else(|| SpiderError::rules(format!("{label} must be an object")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_rules_supports_http_step() {
        let rules = Value::Object(
            [(
                "steps".to_string(),
                Value::Array(vec![Value::Object(
                    [
                        ("id".to_string(), Value::String("parse".to_string())),
                        (
                            "fetch".to_string(),
                            Value::Object(
                                [(
                                    "request".to_string(),
                                    Value::Object(
                                        [
                                            (
                                                "method".to_string(),
                                                Value::String("POST".to_string()),
                                            ),
                                            (
                                                "headers".to_string(),
                                                Value::Object(
                                                    [(
                                                        "x-token".to_string(),
                                                        Value::String("abc".to_string()),
                                                    )]
                                                    .into_iter()
                                                    .collect(),
                                                ),
                                            ),
                                        ]
                                        .into_iter()
                                        .collect(),
                                    ),
                                )]
                                .into_iter()
                                .collect(),
                            ),
                        ),
                        (
                            "parse".to_string(),
                            Value::Object(
                                [(
                                    "fields".to_string(),
                                    Value::Array(vec![Value::Object(
                                        [
                                            (
                                                "name".to_string(),
                                                Value::String("title".to_string()),
                                            ),
                                            (
                                                "source".to_string(),
                                                Value::String("html".to_string()),
                                            ),
                                            (
                                                "selector_type".to_string(),
                                                Value::String("css".to_string()),
                                            ),
                                            (
                                                "selector".to_string(),
                                                Value::Array(vec![Value::String(
                                                    "h1.title".to_string(),
                                                )]),
                                            ),
                                            (
                                                "attribute".to_string(),
                                                Value::String("text".to_string()),
                                            ),
                                            ("required".to_string(), Value::Bool(true)),
                                            ("default".to_string(), Value::Null),
                                            ("multiple".to_string(), Value::Bool(false)),
                                        ]
                                        .into_iter()
                                        .collect(),
                                    )]),
                                )]
                                .into_iter()
                                .collect(),
                            ),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                )]),
            )]
            .into_iter()
            .collect(),
        );

        let compiled = compile_rules(rules).unwrap();

        assert_eq!(compiled.steps[0].id, "parse");
        assert_eq!(compiled.steps[0].fetch.mode, RequestMode::Http);
        assert_eq!(compiled.steps[0].fetch.method, "POST");
        assert_eq!(compiled.steps[0].parse.fields.len(), 1);
        assert_eq!(compiled.steps[0].parse.fields[0].name, "title");
        assert_eq!(compiled.steps[0].parse.fields[0].source, SourceKind::Html);
        assert_eq!(
            compiled.steps[0].parse.fields[0].selector_type,
            SelectorKind::Css
        );
        assert_eq!(
            compiled.steps[0].fetch.headers.get("x-token"),
            Some(&vec!["abc".to_string()])
        );
    }

    #[test]
    fn compile_rules_supports_browser_step() {
        let rules = Value::String(
            r#"{
                "steps":[
                    {
                        "id":"detail",
                        "callback":"parse_detail",
                        "fetch":{
                            "mode":"browser",
                            "browser":{
                                "driver":"playwright",
                                "engine":"chromium",
                                "stealth":true,
                                "fingerprint_profile":"desktop_zh_cn"
                            }
                        },
                        "runtime":{
                            "schedule":{
                                "interval_ms":1000
                            }
                        },
                        "parse":{
                            "links":[
                                {
                                    "name":"detail_links",
                                    "source":"html",
                                    "selector_type":"css",
                                    "selector":["a.detail"],
                                    "attribute":"attr:href",
                                    "required":false,
                                    "default":[],
                                    "multiple":true,
                                    "allow":["^https://example.com/detail/\\d+$"],
                                    "deny":[],
                                    "next_step":"detail_fetch",
                                    "meta":{
                                        "from_list":true
                                    }
                                }
                            ]
                        }
                    }
                ]
            }"#
            .to_string(),
        );

        let compiled = compile_rules(rules).unwrap();

        assert_eq!(compiled.steps[0].id, "detail");
        assert_eq!(compiled.steps[0].callback.as_deref(), Some("parse_detail"));
        assert_eq!(compiled.steps[0].fetch.mode, RequestMode::Browser);
        assert_eq!(compiled.steps[0].parse.links.len(), 1);
        assert_eq!(compiled.steps[0].parse.links[0].name, "detail_links");
        assert_eq!(
            compiled.steps[0].parse.links[0].next_step.as_deref(),
            Some("detail_fetch")
        );
        assert_eq!(
            compiled.steps[0].parse.links[0].meta.get("from_list"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            compiled.steps[0]
                .fetch
                .browser
                .as_ref()
                .and_then(|config| config.fingerprint_profile.as_deref()),
            Some("desktop_zh_cn")
        );
        assert_eq!(
            compiled.steps[0].runtime.schedule.get("interval_ms"),
            Some(&Value::Number(1000.0))
        );
    }

    #[test]
    fn compile_rules_compiles_step_validate_into_shared_plans() {
        let rules = Value::String(
            r#"{
                "steps":[
                    {
                        "id":"parse",
                        "validate":[
                            {
                                "name":"title",
                                "type":"text",
                                "rule":{"required":true}
                            },
                            {
                                "name":"tags",
                                "type":"list"
                            }
                        ],
                        "parse":{"fields":[]}
                    }
                ]
            }"#
            .to_string(),
        );

        let compiled = compile_rules(rules).unwrap();

        assert_eq!(
            compiled.steps[0].validate,
            vec![
                ValidationPlan::new("title", ValidationType::Text).with_required(true),
                ValidationPlan::new("tags", ValidationType::List),
            ]
        );
    }

    #[test]
    fn compile_rules_rejects_invalid_selector_type() {
        let rules = Value::Object(
            [(
                "steps".to_string(),
                Value::Array(vec![Value::Object(
                    [
                        ("id".to_string(), Value::String("parse".to_string())),
                        (
                            "parse".to_string(),
                            Value::Object(
                                [(
                                    "fields".to_string(),
                                    Value::Array(vec![Value::Object(
                                        [
                                            (
                                                "name".to_string(),
                                                Value::String("title".to_string()),
                                            ),
                                            (
                                                "source".to_string(),
                                                Value::String("html".to_string()),
                                            ),
                                            (
                                                "selector_type".to_string(),
                                                Value::String("unknown".to_string()),
                                            ),
                                            (
                                                "selector".to_string(),
                                                Value::Array(vec![Value::String("h1".to_string())]),
                                            ),
                                        ]
                                        .into_iter()
                                        .collect(),
                                    )]),
                                )]
                                .into_iter()
                                .collect(),
                            ),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                )]),
            )]
            .into_iter()
            .collect(),
        );

        assert_eq!(
            compile_rules(rules).unwrap_err(),
            SpiderError::Rules("unsupported selector_type: unknown".to_string())
        );
    }

    #[test]
    fn compile_rules_supports_step_middlewares() {
        let rules = Value::String(
            r#"{
                "steps":[
                    {
                        "id":"parse",
                        "fetch":{"request":{}},
                        "parse":{"fields":[],"links":[]},
                        "MIDDLEWARES":{
                            "retry_by_status":{
                                "enabled":true,
                                "type":"download",
                                "order":200,
                                "options":{"count":5,"status":[429,503]}
                            },
                            "dedup":{"enabled":false}
                        }
                    }
                ]
            }"#
            .to_string(),
        );

        let compiled = compile_rules(rules).unwrap();

        assert_eq!(compiled.steps[0].id, "parse");
        assert!(
            compiled.steps[0]
                .middlewares
                .contains_key("retry_by_status")
        );
        assert!(compiled.steps[0].middlewares.contains_key("dedup"));
        assert_eq!(
            compiled.steps[0].middlewares["retry_by_status"].enabled,
            true
        );
        assert_eq!(compiled.steps[0].middlewares["retry_by_status"].order, 200);
        assert_eq!(
            compiled.steps[0].middlewares["retry_by_status"]
                .options
                .get("count")
                .and_then(Value::as_f64),
            Some(5.0)
        );
        assert_eq!(compiled.steps[0].middlewares["dedup"].enabled, false);
    }

    #[test]
    fn compile_rules_projects_step_configs_into_runtime() {
        let rules = Value::String(
            r#"{
                "steps":[
                    {
                        "id":"detail",
                        "dedup":{
                            "enabled":true,
                            "key":["product_id","meta.category"],
                            "ttl":86400,
                            "scope":"STEP"
                        },
                        "schedule":{
                            "concurrency":2,
                            "interval":1000
                        },
                        "retry":{
                            "count":3,
                            "http_status":[500,502],
                            "backoff":[1000,2000]
                        }
                    }
                ]
            }"#
            .to_string(),
        );

        let compiled = compile_rules(rules).unwrap();
        let step = &compiled.steps[0];

        assert_eq!(
            step.runtime.schedule.get("concurrency"),
            Some(&Value::Number(2.0))
        );
        assert_eq!(
            step.runtime.schedule.get("interval_ms"),
            Some(&Value::Number(1000.0))
        );
        assert_eq!(step.runtime.retry.get("count"), Some(&Value::Number(3.0)));
        assert_eq!(
            step.runtime.retry.get("backoff_ms"),
            Some(&Value::Array(vec![
                Value::Number(1000.0),
                Value::Number(2000.0)
            ]))
        );
        assert_eq!(
            step.runtime.dedup.get("key"),
            Some(&Value::Array(vec![
                Value::String("product_id".to_string()),
                Value::String("meta.category".to_string()),
            ]))
        );
        assert_eq!(
            step.runtime.dedup.get("scope"),
            Some(&Value::String("STEP".to_string()))
        );
    }
}

fn parse_dedup(
    value: Option<&Value>,
) -> Result<Option<crate::rules::schema::DedupConfig>, SpiderError> {
    let Some(v) = value else { return Ok(None) };
    let obj = v
        .as_object()
        .ok_or_else(|| SpiderError::rules("dedup must be an object"))?;

    Ok(Some(crate::rules::schema::DedupConfig {
        enabled: obj.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        key: obj
            .get("key")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        ttl: obj
            .get("ttl")
            .and_then(Value::as_f64)
            .map(|n| n as u64)
            .unwrap_or(86400),
        scope: obj
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("TASK")
            .to_string(),
        namespace: obj
            .get("namespace")
            .and_then(Value::as_str)
            .map(str::to_string),
    }))
}

fn parse_schedule(
    value: Option<&Value>,
) -> Result<Option<crate::rules::schema::ScheduleConfig>, SpiderError> {
    let Some(v) = value else { return Ok(None) };
    let obj = v
        .as_object()
        .ok_or_else(|| SpiderError::rules("schedule must be an object"))?;

    Ok(Some(crate::rules::schema::ScheduleConfig {
        concurrency: obj
            .get("concurrency")
            .and_then(Value::as_f64)
            .map(|n| n as u32),
        interval: obj
            .get("interval")
            .and_then(Value::as_f64)
            .map(|n| n as u64),
    }))
}

fn parse_retry(
    value: Option<&Value>,
) -> Result<Option<crate::rules::schema::RetryConfig>, SpiderError> {
    let Some(v) = value else { return Ok(None) };
    let obj = v
        .as_object()
        .ok_or_else(|| SpiderError::rules("retry must be an object"))?;

    Ok(Some(crate::rules::schema::RetryConfig {
        count: obj
            .get("count")
            .and_then(Value::as_f64)
            .map(|n| n as u32)
            .unwrap_or(3),
        http_status: obj
            .get("http_status")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_f64)
                    .map(|n| n as u16)
                    .collect()
            })
            .unwrap_or_default(),
        backoff: obj
            .get("backoff")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_f64)
                    .map(|n| n as u64)
                    .collect()
            })
            .unwrap_or_default(),
    }))
}
