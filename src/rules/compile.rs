use crate::error::SpiderError;
use crate::middleware::{Config as MiddlewareConfig, Map as MiddlewareMap, Stage};
use crate::request::browser::{
    Config as BrowserConfig, DeviceProfile, Driver, Engine, FingerprintProfile, KeepAlive,
    KeepAliveScope, ScreenProfile, Size,
};
use crate::request::http::Config as HttpConfig;
use crate::request::{Headers, ProxyConfig, RequestMode, SessionConfig};
use crate::rules::schema::{
    Compiled, CompiledStep, Dsl, FetchConfig, FetchPlan, FieldConfig, FieldPlan, LinkConfig,
    LinkPlan, ParseConfig, ParsePlan, SelectorKind, SourceKind, StepConfig,
};
use crate::rules::validate::validate_rules;
use crate::runtime::{Config as RuntimeConfig, merge as merge_runtime};
use crate::validator::{Validation, ValidationRule, ValidationType};
use crate::value::Value;
use jiff::SignedDuration;
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
    let mode = fetch
        .mode
        .as_deref()
        .map(RequestMode::try_from)
        .transpose()
        .map_err(SpiderError::rules)?;
    let method = fetch
        .request
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let headers = parse_headers(fetch.request.get("headers"))?;
    let body = parse_body(fetch.request.get("body"))?;
    let cookies = parse_request_cookies(&fetch.request)?;
    let timeout = parse_request_timeout(fetch.request.get("timeout"))?;
    let proxy = parse_request_proxy(fetch.request.get("proxy"))?;
    let session = parse_request_session(fetch.request.get("session"))?;
    let dont_filter = fetch.request.get("dont_filter").and_then(Value::as_bool);
    let http = parse_http_config(&fetch.request)?;
    let browser = parse_browser_config(&fetch.browser)?;

    Ok(FetchPlan {
        mode,
        method,
        headers,
        body,
        cookies,
        timeout,
        proxy,
        session,
        dont_filter,
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

fn parse_validation(value: &Value) -> Result<Validation, SpiderError> {
    let entry = value
        .as_object()
        .ok_or_else(|| SpiderError::rules("step validate entry must be an object"))?;
    let field = required_string(entry, "name")?.to_string();
    let value_type =
        ValidationType::try_from(required_string(entry, "type")?).map_err(SpiderError::rules)?;
    let rule = entry.get("rule").and_then(Value::as_object);
    let required = rule
        .and_then(|rule| rule.get("required"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let regex = rule
        .and_then(|rule| rule.get("regex"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let min = rule
        .and_then(|rule| rule.get("min"))
        .and_then(Value::as_f64);
    let max = rule
        .and_then(|rule| rule.get("max"))
        .and_then(Value::as_f64);
    let enum_values = rule
        .and_then(|rule| rule.get("enum"))
        .and_then(Value::as_array)
        .map(|values| values.to_vec())
        .unwrap_or_default();

    Ok(Validation {
        field,
        value_type,
        transforms: Vec::new(),
        conditions: Vec::new(),
        object_validations: Vec::new(),
        each_validations: Vec::new(),
        groups: Vec::new(),
        rule: ValidationRule {
            required,
            regex,
            min,
            max,
            enum_values,
            ..ValidationRule::default()
        },
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
        map.insert("interval".to_string(), Value::Number(interval as f64));
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
            "backoff".to_string(),
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
        other => Err(SpiderError::rules(format!(
            "unsupported selector_type: {other}"
        ))),
    }
}

fn parse_http_config(value: &BTreeMap<String, Value>) -> Result<Option<HttpConfig>, SpiderError> {
    let has_http_fields = value.contains_key("query") || value.contains_key("allow_redirects");
    if !has_http_fields {
        return Ok(None);
    }

    let mut config = HttpConfig::default();

    if let Some(query) = value.get("query") {
        for (key, value) in expect_object(query, "fetch.request.query")? {
            let value = value.as_str().ok_or_else(|| {
                SpiderError::rules(format!("query value for {key} must be string"))
            })?;
            config = config.with_query(key.clone(), value.to_string());
        }
    }

    if let Some(allow_redirects) = value.get("allow_redirects").and_then(Value::as_bool) {
        config = config.with_redirects(allow_redirects);
    }

    Ok(Some(config))
}

fn parse_request_cookies(
    value: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, String>, SpiderError> {
    let Some(cookies) = value.get("cookies") else {
        return Ok(BTreeMap::new());
    };

    let mut parsed = BTreeMap::new();
    for (key, value) in expect_object(cookies, "fetch.request.cookies")? {
        let value = value
            .as_str()
            .ok_or_else(|| SpiderError::rules(format!("cookie value for {key} must be string")))?;
        parsed.insert(key.clone(), value.to_string());
    }

    Ok(parsed)
}

fn parse_browser_config(
    value: &BTreeMap<String, Value>,
) -> Result<Option<BrowserConfig>, SpiderError> {
    if value.is_empty() {
        return Ok(None);
    }

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
    if let Some(device_profile) = value.get("device_profile") {
        let device_profile = expect_object(device_profile, "fetch.browser.device_profile")?;
        let mut resolved = DeviceProfile::new();

        if let Some(fingerprint) = device_profile.get("fingerprint") {
            let fingerprint =
                expect_object(fingerprint, "fetch.browser.device_profile.fingerprint")?;
            let mut profile = FingerprintProfile::new();

            if let Some(user_agent) = fingerprint.get("user_agent").and_then(Value::as_str) {
                profile = profile.with_user_agent(user_agent.to_string());
            }
            if let Some(locale) = fingerprint.get("locale").and_then(Value::as_str) {
                profile = profile.with_locale(locale.to_string());
            }
            if let Some(timezone) = fingerprint.get("timezone").and_then(Value::as_str) {
                profile = profile.with_timezone(timezone.to_string());
            }
            if let Some(accept_language) =
                fingerprint.get("accept_language").and_then(Value::as_str)
            {
                profile = profile.with_accept_language(accept_language.to_string());
            }
            if let Some(languages) = fingerprint.get("languages") {
                let values = languages
                    .as_array()
                    .ok_or_else(|| {
                        SpiderError::rules(
                            "fetch.browser.device_profile.fingerprint.languages must be an array",
                        )
                    })?
                    .iter()
                    .map(|value| {
                        value.as_str().map(str::to_string).ok_or_else(|| {
                            SpiderError::rules(
                                "fetch.browser.device_profile.fingerprint.languages must be string[]",
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                profile = profile.with_languages(values);
            }
            if let Some(platform) = fingerprint.get("platform").and_then(Value::as_str) {
                profile = profile.with_platform(platform.to_string());
            }
            if let Some(mobile) = fingerprint.get("mobile").and_then(Value::as_bool) {
                profile = profile.with_mobile(mobile);
            }
            if let Some(device_memory) = fingerprint.get("device_memory").and_then(Value::as_f64) {
                profile = profile.with_device_memory(device_memory as u8);
            }

            resolved = resolved.with_fingerprint(profile);
        }

        if let Some(screen) = device_profile.get("screen") {
            let screen = expect_object(screen, "fetch.browser.device_profile.screen")?;
            let mut profile = ScreenProfile::new();

            if let Some(viewport) = screen.get("viewport") {
                profile = profile.with_viewport_size(parse_browser_size(
                    viewport,
                    "fetch.browser.device_profile.screen.viewport",
                )?);
            }
            if let Some(screen_size) = screen.get("screen") {
                profile = profile.with_screen_size(parse_browser_size(
                    screen_size,
                    "fetch.browser.device_profile.screen.screen",
                )?);
            }
            if let Some(avail) = screen.get("avail") {
                profile = profile.with_avail_size(parse_browser_size(
                    avail,
                    "fetch.browser.device_profile.screen.avail",
                )?);
            }
            if let Some(color_depth) = screen.get("color_depth").and_then(Value::as_f64) {
                profile = profile.with_color_depth(color_depth as u8);
            }
            if let Some(pixel_depth) = screen.get("pixel_depth").and_then(Value::as_f64) {
                profile = profile.with_pixel_depth(pixel_depth as u8);
            }
            if let Some(device_scale_factor) =
                screen.get("device_scale_factor").and_then(Value::as_f64)
            {
                profile = profile.with_device_scale_factor(device_scale_factor as u32);
            }

            resolved = resolved.with_screen(profile);
        }

        config = config.with_device_profile(resolved);
    }
    if let Some(wait_for_selector) = value.get("wait_for_selector").and_then(Value::as_str) {
        config = config.with_wait_for_selector(wait_for_selector.to_string());
    }
    if let Some(keep_alive) = value.get("keep_alive").and_then(Value::as_str) {
        config =
            config.with_keep_alive(KeepAlive::try_from(keep_alive).map_err(SpiderError::rules)?);
    }
    if let Some(keep_alive_scope) = value.get("keep_alive_scope").and_then(Value::as_str) {
        config = config.with_keep_alive_scope(
            KeepAliveScope::try_from(keep_alive_scope).map_err(SpiderError::rules)?,
        );
    }

    Ok(Some(config))
}

fn parse_browser_size(value: &Value, field: &str) -> Result<Size, SpiderError> {
    let size = expect_object(value, field)?;
    let width = size
        .get("width")
        .and_then(Value::as_f64)
        .map(|value| value as u32)
        .ok_or_else(|| SpiderError::rules(format!("{field}.width must be a number")))?;
    let height = size
        .get("height")
        .and_then(Value::as_f64)
        .map(|value| value as u32)
        .ok_or_else(|| SpiderError::rules(format!("{field}.height must be a number")))?;

    Ok(Size::new(width, height))
}

fn parse_request_timeout(value: Option<&Value>) -> Result<Option<SignedDuration>, SpiderError> {
    let Some(value) = value else {
        return Ok(None);
    };

    let Some(milliseconds) = value.as_f64() else {
        return Err(SpiderError::rules("fetch.request.timeout must be a number"));
    };

    Ok(Some(SignedDuration::from_millis(
        milliseconds.max(0.0) as i64
    )))
}

fn parse_request_proxy(value: Option<&Value>) -> Result<Option<ProxyConfig>, SpiderError> {
    let Some(value) = value else {
        return Ok(None);
    };

    let Some(url) = value.as_str() else {
        return Err(SpiderError::rules("fetch.request.proxy must be a string"));
    };

    Ok(Some(ProxyConfig::new(url)))
}

fn parse_request_session(value: Option<&Value>) -> Result<Option<SessionConfig>, SpiderError> {
    let Some(value) = value else {
        return Ok(None);
    };

    let Some(id) = value.as_str() else {
        return Err(SpiderError::rules("fetch.request.session must be a string"));
    };

    Ok(Some(SessionConfig::new(id)))
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
        let stage = match entry
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("download")
        {
            "download" => Stage::Download,
            "spider" => Stage::Spider,
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
                stage,
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
