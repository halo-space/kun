use crate::error::SpiderError;
use crate::request::browser::{Config as BrowserConfig, Driver, Engine, Viewport};
use crate::request::http::Config as HttpConfig;
use crate::request::{Headers, RequestMode};
use crate::runtime::Config as RuntimeConfig;
use crate::rules::schema::{
    Compiled, CompiledStep, Dsl, FetchConfig, FetchPlan, FieldConfig, FieldPlan, LinkConfig,
    LinkPlan, LinkTargetConfig, LinkTargetPlan, ParseConfig, ParsePlan, SelectorKind, SourceKind,
    StepConfig, StepImpl,
};
use crate::rules::validate::validate_rules;
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
        r#impl: required_string(step, "impl")?.to_string(),
        callback: optional_string(step, "callback").map(str::to_string),
        fetch: parse_fetch(step.get("fetch"))?,
        parse: parse_parse(step.get("parse"))?,
        route: optional_map(step, "route"),
        output: optional_map(step, "output"),
        runtime: optional_map(step, "runtime"),
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
        attribute: optional_string(field, "attribute").unwrap_or("text").to_string(),
        required: field.get("required").and_then(Value::as_bool).unwrap_or(false),
        default: field.get("default").cloned().unwrap_or(Value::Null),
        multiple: field.get("multiple").and_then(Value::as_bool).unwrap_or(false),
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
        required: link.get("required").and_then(Value::as_bool).unwrap_or(false),
        default: link
            .get("default")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
        multiple: link.get("multiple").and_then(Value::as_bool).unwrap_or(true),
        allow: string_list_optional(link.get("allow"), "parse.links[*].allow")?,
        deny: string_list_optional(link.get("deny"), "parse.links[*].deny")?,
        to: parse_link_target(link.get("to"))?,
        options: optional_map_value(link, "options"),
    })
}

fn parse_link_target(value: Option<&Value>) -> Result<LinkTargetConfig, SpiderError> {
    let Some(value) = value else {
        return Ok(LinkTargetConfig::default());
    };
    let target = value
        .as_object()
        .ok_or_else(|| SpiderError::rules("parse.links[*].to must be an object"))?;

    Ok(LinkTargetConfig {
        next_step: optional_string(target, "next_step").map(str::to_string),
        meta_patch: optional_map_value(target, "meta_patch"),
    })
}

fn compile_step(step: StepConfig) -> Result<CompiledStep, SpiderError> {
    let step_impl = match step.r#impl.as_str() {
        "dsl" => StepImpl::Dsl,
        "code" => StepImpl::Code,
        other => return Err(SpiderError::rules(format!("unsupported step impl: {other}"))),
    };

    Ok(CompiledStep {
        id: step.id,
        r#impl: step_impl,
        callback: step.callback,
        fetch: compile_fetch(step.fetch)?,
        parse: compile_parse(step.parse)?,
        runtime: compile_runtime(step.runtime)?,
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
    })
}

fn compile_runtime(runtime: BTreeMap<String, Value>) -> Result<RuntimeConfig, SpiderError> {
    Ok(RuntimeConfig {
        schedule: section_map(&runtime, "schedule", "step.runtime.schedule")?,
        retry: section_map(&runtime, "retry", "step.runtime.retry")?,
        dedup: section_map(&runtime, "dedup", "step.runtime.dedup")?,
    })
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
        to: LinkTargetPlan {
            next_step: link.to.next_step,
            meta_patch: link.to.meta_patch,
        },
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
        "final_url" => Ok(SourceKind::FinalUrl),
        value if value.starts_with("meta.") => Ok(SourceKind::Meta),
        other => Err(SpiderError::rules(format!("unsupported parse source: {other}"))),
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
            let value = value
                .as_str()
                .ok_or_else(|| SpiderError::rules(format!("query value for {key} must be string")))?;
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
        let header_value = value.as_str().ok_or_else(|| {
            SpiderError::rules(format!("header value for {key} must be string"))
        })?;
        headers.entry(key.clone()).or_default().push(header_value.to_string());
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
        _ => Err(SpiderError::rules("fetch.request.body must be string or null")),
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
                        ("impl".to_string(), Value::String("dsl".to_string())),
                        (
                            "fetch".to_string(),
                            Value::Object(
                                [(
                                    "request".to_string(),
                                    Value::Object(
                                        [
                                            ("method".to_string(), Value::String("POST".to_string())),
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
                                            ("name".to_string(), Value::String("title".to_string())),
                                            ("source".to_string(), Value::String("html".to_string())),
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
        assert_eq!(compiled.steps[0].parse.fields[0].selector_type, SelectorKind::Css);
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
                        "impl":"code",
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
                                    "to":{
                                        "next_step":"detail_fetch",
                                        "meta_patch":{
                                            "from_list":true
                                        }
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
            compiled.steps[0].parse.links[0].to.next_step.as_deref(),
            Some("detail_fetch")
        );
        assert_eq!(
            compiled.steps[0]
                .parse
                .links[0]
                .to
                .meta_patch
                .get("from_list"),
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
    fn compile_rules_rejects_invalid_selector_type() {
        let rules = Value::Object(
            [(
                "steps".to_string(),
                Value::Array(vec![Value::Object(
                    [
                        ("id".to_string(), Value::String("parse".to_string())),
                        ("impl".to_string(), Value::String("dsl".to_string())),
                        (
                            "parse".to_string(),
                            Value::Object(
                                [(
                                    "fields".to_string(),
                                    Value::Array(vec![Value::Object(
                                        [
                                            ("name".to_string(), Value::String("title".to_string())),
                                            ("source".to_string(), Value::String("html".to_string())),
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
}
