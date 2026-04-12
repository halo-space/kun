use crate::error::SpiderError;
use crate::item::Item;
use crate::middleware::Map as MiddlewareMap;
use crate::parser::{CssQuery, XPathQuery};
use crate::request::{Headers, Request, RequestMode, SessionConfig};
use crate::response::Response;
use crate::rules::compile::detect_selector_kind;
use crate::rules::schema::{
    BodyConfig, Compiled, CompiledSeed, CompiledStep, ExtractKind, FieldPlan, FollowPlan,
    OutputPlan, RequestPlan, SelectorKind, SelectorValueKind, ValueExpr, ValueSource,
};
use crate::value::Value;
use jiff::{
    Timestamp, Zoned,
    civil::{Date, DateTime},
    tz::TimeZone,
};
use regex::Regex;
use std::collections::{BTreeMap, HashSet};
use url::Url;

#[derive(Debug, Default)]
pub struct Output {
    pub items: Vec<Item>,
    pub requests: Vec<Request>,
}

pub async fn apply(
    response: &Response,
    step: &CompiledStep,
    compiled: &Compiled,
) -> Result<Output, SpiderError> {
    let fields = resolve_fields(response, &step.fields)?;
    let bind = BindResolver::new(response, compiled, &fields, &step.bind).resolve_all()?;

    let requests = build_follow_requests(response, step, compiled, &fields, &bind)?;
    let items = build_output_items(response, step.output.as_ref(), compiled, &fields, &bind)?;

    Ok(Output { items, requests })
}

pub fn build_seed_requests(compiled: &Compiled) -> Result<Vec<Request>, SpiderError> {
    let mut requests = Vec::new();
    let empty_fields = BTreeMap::new();
    let empty_bind = BTreeMap::new();
    let seed_response = Response::default();
    let context = EvalContext {
        response: &seed_response,
        compiled,
        fields: &empty_fields,
        bind: &empty_bind,
        scope: None,
    };

    for seed in &compiled.seeds {
        if let Some(request) = build_seed_request(seed, compiled, &context)? {
            requests.push(request);
        }
    }

    Ok(requests)
}

#[derive(Debug, Clone)]
struct SelectorScope {
    input: String,
}

#[derive(Clone, Copy)]
struct EvalContext<'a> {
    response: &'a Response,
    compiled: &'a Compiled,
    fields: &'a BTreeMap<String, Value>,
    bind: &'a BTreeMap<String, Value>,
    scope: Option<&'a SelectorScope>,
}

struct BindResolver<'a> {
    response: &'a Response,
    compiled: &'a Compiled,
    fields: &'a BTreeMap<String, Value>,
    definitions: &'a BTreeMap<String, ValueExpr>,
    values: BTreeMap<String, Value>,
    resolving: HashSet<String>,
}

impl<'a> BindResolver<'a> {
    fn new(
        response: &'a Response,
        compiled: &'a Compiled,
        fields: &'a BTreeMap<String, Value>,
        definitions: &'a BTreeMap<String, ValueExpr>,
    ) -> Self {
        Self {
            response,
            compiled,
            fields,
            definitions,
            values: BTreeMap::new(),
            resolving: HashSet::new(),
        }
    }

    fn resolve_all(mut self) -> Result<BTreeMap<String, Value>, SpiderError> {
        for key in self.definitions.keys() {
            let _ = self.resolve_key(key)?;
        }
        Ok(self.values)
    }

    fn resolve_key(&mut self, key: &str) -> Result<Value, SpiderError> {
        if let Some(value) = self.values.get(key) {
            return Ok(value.clone());
        }

        if !self.resolving.insert(key.to_string()) {
            return Err(SpiderError::rules(format!(
                "circular bind reference detected: {key}"
            )));
        }

        let expr = self
            .definitions
            .get(key)
            .ok_or_else(|| SpiderError::rules(format!("bind reference not found: {key}")))?;
        let value = self.evaluate(expr)?;
        self.resolving.remove(key);
        self.values.insert(key.to_string(), value.clone());
        Ok(value)
    }

    fn evaluate(&mut self, expr: &ValueExpr) -> Result<Value, SpiderError> {
        let base = match &expr.source {
            ValueSource::Literal(value) => value.clone(),
            ValueSource::From(path) => self.resolve_reference(path)?,
            ValueSource::Template { template, vars } => {
                let vars = evaluate_value_map(
                    vars,
                    &EvalContext {
                        response: self.response,
                        compiled: self.compiled,
                        fields: self.fields,
                        bind: &self.values,
                        scope: None,
                    },
                )?;
                Value::String(render_template(template, &vars))
            }
            ValueSource::Selector { selector, kind } => {
                let input = &self.response.text;
                select_value(input, detect_selector_kind(selector), selector, kind)
                    .map(Value::String)
                    .unwrap_or(Value::Null)
            }
        };

        let value = apply_transforms(base, &expr.transforms, self.response, self.compiled)?;
        if should_use_fallback(&value)
            && let Some(fallback) = &expr.fallback
        {
            return self.evaluate(fallback);
        }

        Ok(value)
    }

    fn resolve_reference(&mut self, path: &str) -> Result<Value, SpiderError> {
        if path == "$now" {
            return Ok(Value::String(current_now(self.compiled)?));
        }

        if let Some(name) = path.strip_prefix("$bind.") {
            return self.resolve_key(name);
        }

        if let Some(name) = path.strip_prefix("$fields.") {
            return Ok(lookup_map_value(self.fields, name).unwrap_or(Value::Null));
        }

        if let Some(name) = path.strip_prefix("$meta.") {
            return Ok(lookup_map_value(&self.response.meta, name).unwrap_or(Value::Null));
        }

        if let Some(name) = path.strip_prefix("$env.") {
            return Ok(std::env::var(name)
                .map(Value::String)
                .unwrap_or(Value::Null));
        }

        resolve_request_or_response_reference(self.response, path)
    }
}

fn resolve_fields(
    response: &Response,
    fields: &BTreeMap<String, FieldPlan>,
) -> Result<BTreeMap<String, Value>, SpiderError> {
    let mut resolved = BTreeMap::new();
    for (name, field) in fields {
        resolved.insert(name.clone(), resolve_field(response, field));
    }
    Ok(resolved)
}

fn resolve_field(response: &Response, field: &FieldPlan) -> Value {
    let value = match &field.kind {
        ExtractKind::Text => select_value(
            &response.text,
            field.selector_kind,
            &field.selector,
            &SelectorValueKind::Text,
        ),
        ExtractKind::Html => select_value(
            &response.text,
            field.selector_kind,
            &field.selector,
            &SelectorValueKind::Html,
        ),
        ExtractKind::Attribute(attr) => select_value(
            &response.text,
            field.selector_kind,
            &field.selector,
            &SelectorValueKind::Attribute(attr.clone()),
        ),
    };

    value.map(Value::String).unwrap_or(Value::Null)
}

fn build_follow_requests(
    response: &Response,
    step: &CompiledStep,
    compiled: &Compiled,
    fields: &BTreeMap<String, Value>,
    bind: &BTreeMap<String, Value>,
) -> Result<Vec<Request>, SpiderError> {
    let mut requests = Vec::new();

    for follow in &step.follow {
        for scope in build_follow_scopes(response, follow) {
            let context = EvalContext {
                response,
                compiled,
                fields,
                bind,
                scope: Some(&scope),
            };

            let Some(raw_url) =
                value_as_non_empty_string(evaluate_value_expr(&follow.request.url, &context)?)
            else {
                continue;
            };

            let absolute_url = resolve_url(&response.url, &raw_url)?;
            if !url_allowed(&absolute_url, &follow.allow_url_pattern)? {
                continue;
            }

            let mut request = build_request_from_plan(&absolute_url, &follow.request, &context)?;
            request = request.with_meta_map(evaluate_value_map(&follow.meta, &context)?);
            request = request.with_meta("next_step", Value::String(follow.next_step.clone()));
            request = apply_request_middleware_plan(request, &follow.middleware);
            if !follow.request.skip.is_empty() {
                request = request.skip(follow.request.skip.iter().map(String::as_str));
            }
            request = apply_target_step_fetch(request, compiled, &follow.next_step)?;
            requests.push(request);
        }
    }

    Ok(requests)
}

fn build_follow_scopes(response: &Response, follow: &FollowPlan) -> Vec<SelectorScope> {
    let Some(selector) = &follow.item else {
        return vec![SelectorScope {
            input: response.text.clone(),
        }];
    };

    let selector_kind = follow
        .item_selector_kind
        .unwrap_or_else(|| detect_selector_kind(selector));
    select_markup_all(&response.text, selector_kind, selector)
        .into_iter()
        .map(|input| SelectorScope { input })
        .collect()
}

fn build_output_items(
    response: &Response,
    output: Option<&OutputPlan>,
    compiled: &Compiled,
    fields: &BTreeMap<String, Value>,
    bind: &BTreeMap<String, Value>,
) -> Result<Vec<Item>, SpiderError> {
    let Some(output) = output else {
        return Ok(Vec::new());
    };

    let context = EvalContext {
        response,
        compiled,
        fields,
        bind,
        scope: None,
    };

    let item = Item::from_fields(evaluate_value_map(&output.item, &context)?);
    Ok(vec![item])
}

fn build_seed_request(
    seed: &CompiledSeed,
    compiled: &Compiled,
    context: &EvalContext<'_>,
) -> Result<Option<Request>, SpiderError> {
    let Some(raw_url) = value_as_non_empty_string(evaluate_value_expr(&seed.request.url, context)?)
    else {
        return Ok(None);
    };

    if !url_allowed(&raw_url, &seed.allow_url_pattern)? {
        return Ok(None);
    }

    let mut request = build_request_from_plan(&raw_url, &seed.request, context)?;
    request = request.with_meta_map(evaluate_value_map(&seed.meta, context)?);
    request = request.with_meta("next_step", Value::String(seed.next_step.clone()));
    request = apply_request_middleware_plan(request, &seed.middleware);
    if !seed.request.skip.is_empty() {
        request = request.skip(seed.request.skip.iter().map(String::as_str));
    }
    request = apply_target_step_fetch(request, compiled, &seed.next_step)?;

    Ok(Some(request))
}

fn evaluate_value_map(
    values: &BTreeMap<String, ValueExpr>,
    context: &EvalContext<'_>,
) -> Result<BTreeMap<String, Value>, SpiderError> {
    let mut evaluated = BTreeMap::new();
    for (key, expr) in values {
        evaluated.insert(key.clone(), evaluate_value_expr(expr, context)?);
    }
    Ok(evaluated)
}

fn evaluate_value_expr(expr: &ValueExpr, context: &EvalContext<'_>) -> Result<Value, SpiderError> {
    let base = match &expr.source {
        ValueSource::Literal(value) => value.clone(),
        ValueSource::From(path) => resolve_reference(path, context)?,
        ValueSource::Template { template, vars } => {
            let vars = evaluate_value_map(vars, context)?;
            Value::String(render_template(template, &vars))
        }
        ValueSource::Selector { selector, kind } => {
            let input = context
                .scope
                .map(|scope| scope.input.as_str())
                .unwrap_or(context.response.text.as_str());
            select_value(input, detect_selector_kind(selector), selector, kind)
                .map(Value::String)
                .unwrap_or(Value::Null)
        }
    };

    let value = apply_transforms(base, &expr.transforms, context.response, context.compiled)?;
    if should_use_fallback(&value)
        && let Some(fallback) = &expr.fallback
    {
        return evaluate_value_expr(fallback, context);
    }

    Ok(value)
}

fn resolve_reference(path: &str, context: &EvalContext<'_>) -> Result<Value, SpiderError> {
    if path == "$now" {
        return Ok(Value::String(current_now(context.compiled)?));
    }

    if let Some(name) = path.strip_prefix("$env.") {
        return Ok(std::env::var(name)
            .map(Value::String)
            .unwrap_or(Value::Null));
    }

    if let Some(name) = path.strip_prefix("$fields.") {
        return Ok(lookup_map_value(context.fields, name).unwrap_or(Value::Null));
    }

    if let Some(name) = path.strip_prefix("$bind.") {
        return Ok(lookup_map_value(context.bind, name).unwrap_or(Value::Null));
    }

    if let Some(name) = path.strip_prefix("$meta.") {
        return Ok(lookup_map_value(&context.response.meta, name).unwrap_or(Value::Null));
    }

    resolve_request_or_response_reference(context.response, path)
}

fn resolve_request_or_response_reference(
    response: &Response,
    path: &str,
) -> Result<Value, SpiderError> {
    if path == "$response.url" {
        return Ok(Value::String(response.url.clone()));
    }

    if path == "$response.status" {
        return Ok(Value::Number(response.status as f64));
    }

    if path == "$request.url" {
        return Ok(response
            .request
            .as_deref()
            .map(|request| Value::String(request.url.clone()))
            .unwrap_or(Value::Null));
    }

    if path == "$request.method" {
        return Ok(response
            .request
            .as_deref()
            .map(|request| Value::String(request.method.clone()))
            .unwrap_or(Value::Null));
    }

    Ok(Value::Null)
}

fn lookup_map_value(map: &BTreeMap<String, Value>, path: &str) -> Option<Value> {
    let mut segments = path.split('.');
    let first = segments.next()?;
    let mut current = map.get(first)?;

    for segment in segments {
        current = navigate_value(current, segment)?;
    }

    Some(current.clone())
}

fn navigate_value<'a>(value: &'a Value, segment: &str) -> Option<&'a Value> {
    if let Some((name, index)) = parse_indexed_segment(segment) {
        let value = if name.is_empty() {
            value
        } else {
            value.as_object()?.get(name)?
        };
        return value.as_array()?.get(index);
    }

    value.as_object()?.get(segment)
}

fn parse_indexed_segment(segment: &str) -> Option<(&str, usize)> {
    let (name, rest) = segment.split_once('[')?;
    let index = rest.strip_suffix(']')?.parse::<usize>().ok()?;
    Some((name, index))
}

fn build_request_from_plan(
    url: &str,
    plan: &RequestPlan,
    context: &EvalContext<'_>,
) -> Result<Request, SpiderError> {
    let mut request = match plan.mode.unwrap_or(RequestMode::Http) {
        RequestMode::Http => Request::new(url.to_string()),
        RequestMode::Browser => Request::browser(url.to_string()),
    };

    if let Some(method) = &plan.method {
        request = request.with_method(method.clone());
    }

    if let Some(encoding) = plan.encoding.as_ref() {
        if let Some(value) = value_as_non_empty_string(evaluate_value_expr(encoding, context)?) {
            request = request.with_encoding(value);
        }
    }

    if let Some(priority) = plan.priority.as_ref() {
        if let Some(value) = evaluate_value_expr(priority, context)?.as_f64() {
            request = request.with_priority(value as i32);
        }
    }

    if let Some(timeout) = plan.timeout.as_ref() {
        if let Some(value) = evaluate_value_expr(timeout, context)?.as_f64() {
            request = request.with_timeout(jiff::SignedDuration::from_millis(value as i64));
        }
    }

    if let Some(proxy) = plan.proxy.as_ref()
        && let Some(value) = value_as_non_empty_string(evaluate_value_expr(proxy, context)?)
    {
        request = request.with_proxy(value);
    }

    if let Some(session) = plan.session.as_ref()
        && let Some(value) = value_as_non_empty_string(evaluate_value_expr(session, context)?)
    {
        request = request.with_session_config(SessionConfig::new(value));
    }

    let query = evaluate_string_map(&plan.query, context)?;
    if !query.is_empty() {
        request.url = append_query(&request.url, &query)?;
    }

    if let Some(allow_redirects) = plan.allow_redirects {
        let mut http = request.http.take().unwrap_or_default();
        http.allow_redirects = allow_redirects;
        request.http = Some(http);
    }

    let headers = evaluate_multi_header_map(&plan.headers, context)?;
    if !headers.is_empty() {
        request.headers.extend(headers);
    }

    let cookies = evaluate_string_map(&plan.cookies, context)?;
    if !cookies.is_empty() {
        request.cookies.extend(cookies);
    }

    let cb_kwargs = evaluate_value_map(&plan.cb_kwargs, context)?;
    if !cb_kwargs.is_empty() {
        request = request.with_cb_kwargs(cb_kwargs);
    }

    for flag in &plan.flags {
        if let Some(value) = value_as_non_empty_string(evaluate_value_expr(flag, context)?) {
            request = request.with_flag(value);
        }
    }

    if let Some(errback) = &plan.errback {
        request = request.with_errback(errback.clone());
    }

    if let Some(body) = &plan.body {
        request = apply_body(request, body, context)?;
    }

    Ok(request)
}

fn apply_body(
    mut request: Request,
    body: &BodyConfig,
    context: &EvalContext<'_>,
) -> Result<Request, SpiderError> {
    match body {
        BodyConfig::Json(values) => {
            let value = Value::Object(evaluate_value_map(values, context)?);
            let encoded = serde_json::to_vec(&value.to_json()).map_err(|error| {
                SpiderError::rules(format!("failed to encode request json body: {error}"))
            })?;
            if !request.headers.contains_key("Content-Type") {
                request = request.with_header("Content-Type", "application/json");
            }
            request = request.with_body(encoded);
        }
        BodyConfig::Form(values) => {
            let values = evaluate_string_map(values, context)?;
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            for (key, value) in values {
                serializer.append_pair(&key, &value);
            }
            if !request.headers.contains_key("Content-Type") {
                request = request.with_header("Content-Type", "application/x-www-form-urlencoded");
            }
            request = request.with_body(serializer.finish());
        }
        BodyConfig::Raw(value) => {
            let value = evaluate_value_expr(value, context)?;
            if let Some(text) = value_as_string(value) {
                request = request.with_body(text);
            }
        }
    }

    Ok(request)
}

fn apply_request_middleware_plan(mut request: Request, middleware: &MiddlewareMap) -> Request {
    for (name, config) in middleware {
        if !config.enabled {
            request = request.skip([name.as_str()]);
            continue;
        }

        request = if config.order == 0 {
            request.with_middleware_options(name.clone(), config.options.clone())
        } else {
            request.with_middleware_options_ordered(
                name.clone(),
                config.options.clone(),
                config.order,
            )
        };
    }

    request
}

fn apply_target_step_fetch(
    request: Request,
    compiled: &Compiled,
    next_step: &str,
) -> Result<Request, SpiderError> {
    let step = compiled
        .steps
        .iter()
        .find(|step| step.id == next_step)
        .ok_or_else(|| SpiderError::engine(format!("step not found: {next_step}")))?;
    Ok(step.fetch.apply_to_request(request))
}

fn evaluate_string_map(
    values: &BTreeMap<String, ValueExpr>,
    context: &EvalContext<'_>,
) -> Result<BTreeMap<String, String>, SpiderError> {
    let mut evaluated = BTreeMap::new();
    for (key, expr) in values {
        if let Some(value) = value_as_non_empty_string(evaluate_value_expr(expr, context)?) {
            evaluated.insert(key.clone(), value);
        }
    }
    Ok(evaluated)
}

fn evaluate_multi_header_map(
    values: &BTreeMap<String, ValueExpr>,
    context: &EvalContext<'_>,
) -> Result<Headers, SpiderError> {
    let mut headers = Headers::new();

    for (key, expr) in values {
        match evaluate_value_expr(expr, context)? {
            Value::Array(values) => {
                let collected = values
                    .into_iter()
                    .filter_map(value_as_string)
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>();
                if !collected.is_empty() {
                    headers.insert(key.clone(), collected);
                }
            }
            value => {
                if let Some(value) = value_as_non_empty_string(value) {
                    headers.insert(key.clone(), vec![value]);
                }
            }
        }
    }

    Ok(headers)
}

fn select_value(
    input: &str,
    selector_kind: SelectorKind,
    selector: &str,
    value_kind: &SelectorValueKind,
) -> Option<String> {
    match selector_kind {
        SelectorKind::Css => {
            let query = CssQuery::new(input, selector);
            match value_kind {
                SelectorValueKind::Text => query.text().one(),
                SelectorValueKind::Html => query.html().one(),
                SelectorValueKind::Attribute(attr) => query.attr(attr).one(),
            }
        }
        SelectorKind::XPath => {
            let query = XPathQuery::new(input, selector);
            match value_kind {
                SelectorValueKind::Text => query.text().one(),
                SelectorValueKind::Html => query.html().one(),
                SelectorValueKind::Attribute(attr) => query.attr(attr).one(),
            }
        }
    }
}

fn select_markup_all(input: &str, selector_kind: SelectorKind, selector: &str) -> Vec<String> {
    match selector_kind {
        SelectorKind::Css => CssQuery::new(input, selector).html().all(),
        SelectorKind::XPath => XPathQuery::new(input, selector).html().all(),
    }
}

fn should_use_fallback(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(value) => value.trim().is_empty(),
        Value::Array(values) => values.is_empty(),
        Value::Object(values) => values.is_empty(),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

fn render_template(template: &str, vars: &BTreeMap<String, Value>) -> String {
    let mut rendered = template.to_string();
    for (key, value) in vars {
        rendered = rendered.replace(
            &format!("{{{key}}}"),
            &value_as_string(value.clone()).unwrap_or_default(),
        );
    }
    rendered
}

fn value_as_non_empty_string(value: Value) -> Option<String> {
    value_as_string(value).filter(|value| !value.trim().is_empty())
}

fn value_as_string(value: Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(format_number(value)),
        Value::String(value) => Some(value),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(&value.to_json()).ok(),
    }
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

fn apply_transforms(
    mut value: Value,
    transforms: &[crate::rules::schema::TransformConfig],
    response: &Response,
    compiled: &Compiled,
) -> Result<Value, SpiderError> {
    for transform in transforms {
        value = apply_transform(value, transform, response, compiled)?;
    }
    Ok(value)
}

fn apply_transform(
    value: Value,
    transform: &crate::rules::schema::TransformConfig,
    response: &Response,
    compiled: &Compiled,
) -> Result<Value, SpiderError> {
    match transform.kind.as_str() {
        "trim" => Ok(map_string_values(value, |text| text.trim().to_string())),
        "replace" => {
            let from = required_option_string(&transform.options, "from", "replace")?;
            let to = transform
                .options
                .get("to")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Ok(map_string_values(value, |text| text.replace(&from, &to)))
        }
        "regex" => apply_regex_transform(value, &transform.options),
        "split" => {
            let delimiter = option_string_any(&transform.options, &["delimiter", "sep"], "split")?;
            match value {
                Value::String(text) => Ok(Value::Array(
                    text.split(&delimiter)
                        .map(|part| Value::String(part.to_string()))
                        .collect(),
                )),
                other => Ok(other),
            }
        }
        "join" => {
            let delimiter = option_string_any(&transform.options, &["delimiter", "sep"], "join")?;
            match value {
                Value::Array(values) => Ok(Value::String(
                    values
                        .into_iter()
                        .filter_map(value_as_string)
                        .collect::<Vec<_>>()
                        .join(&delimiter),
                )),
                other => Ok(other),
            }
        }
        "pick" => {
            let index = transform
                .options
                .get("index")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as usize;
            match value {
                Value::Array(values) => Ok(values.get(index).cloned().unwrap_or(Value::Null)),
                other => Ok(other),
            }
        }
        "date_format" => {
            let format = required_option_string(&transform.options, "format", "date_format")?;
            let input_format = transform
                .options
                .get("input_format")
                .and_then(Value::as_str);
            match value {
                Value::String(text) => Ok(Value::String(format_datetime(
                    &text,
                    input_format,
                    &format,
                    compiled,
                )?)),
                other => Ok(other),
            }
        }
        "resolve_url" => match value {
            Value::String(text) => Ok(Value::String(resolve_url(&response.url, &text)?)),
            other => Ok(other),
        },
        other => Err(SpiderError::rules(format!(
            "unsupported value transform: {other}"
        ))),
    }
}

fn apply_regex_transform(
    value: Value,
    options: &BTreeMap<String, Value>,
) -> Result<Value, SpiderError> {
    let pattern = option_string_any(options, &["pattern", "expr"], "regex")?;
    let compiled = Regex::new(&pattern)
        .map_err(|error| SpiderError::rules(format!("invalid regex transform pattern: {error}")))?;
    let group = options.get("group").and_then(Value::as_f64).unwrap_or(1.0) as usize;

    if let Some(replace) = options.get("replace").and_then(Value::as_str) {
        return Ok(map_string_values(value, |text| {
            compiled.replace_all(&text, replace).to_string()
        }));
    }

    match value {
        Value::String(text) => Ok(compiled
            .captures(&text)
            .and_then(|captures| captures.get(group))
            .map(|value| Value::String(value.as_str().to_string()))
            .unwrap_or(Value::Null)),
        other => Ok(other),
    }
}

fn map_string_values(value: Value, transform: impl Fn(String) -> String + Copy) -> Value {
    match value {
        Value::String(text) => Value::String(transform(text)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| map_string_values(value, transform))
                .collect(),
        ),
        other => other,
    }
}

fn required_option_string(
    options: &BTreeMap<String, Value>,
    key: &str,
    transform: &str,
) -> Result<String, SpiderError> {
    options
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            SpiderError::rules(format!(
                "transform {transform} requires a non-empty `{key}` option"
            ))
        })
}

fn option_string_any(
    options: &BTreeMap<String, Value>,
    keys: &[&str],
    transform: &str,
) -> Result<String, SpiderError> {
    keys.iter()
        .find_map(|key| {
            options
                .get(*key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| {
            SpiderError::rules(format!(
                "transform {transform} requires one of: {}",
                keys.join(", ")
            ))
        })
}

fn current_now(compiled: &Compiled) -> Result<String, SpiderError> {
    let now = Timestamp::now();
    if let Some(timezone) = compiled.spider.clock.timezone.as_deref() {
        let timezone = TimeZone::get(timezone).map_err(|error| {
            SpiderError::rules(format!(
                "invalid spider.clock.timezone {timezone:?}: {error}"
            ))
        })?;
        Ok(now.to_zoned(timezone).to_string())
    } else {
        Ok(now.to_string())
    }
}

fn format_datetime(
    text: &str,
    input_format: Option<&str>,
    format: &str,
    compiled: &Compiled,
) -> Result<String, SpiderError> {
    if let Some(input_format) = input_format {
        if let Ok(value) = Zoned::strptime(input_format, text) {
            return Ok(value.strftime(format).to_string());
        }
        if let Ok(value) = DateTime::strptime(input_format, text) {
            return Ok(value.strftime(format).to_string());
        }
        if let Ok(value) = Date::strptime(input_format, text) {
            return Ok(value.strftime(format).to_string());
        }

        return Err(SpiderError::rules(format!(
            "date_format input did not match format {input_format:?}"
        )));
    }

    if let Ok(value) = text.parse::<Timestamp>() {
        let timezone = compiled
            .spider
            .clock
            .timezone
            .as_deref()
            .map(TimeZone::get)
            .transpose()
            .map_err(|error| {
                SpiderError::rules(format!("invalid spider.clock.timezone: {error}"))
            })?;
        if let Some(timezone) = timezone {
            return Ok(value.to_zoned(timezone).strftime(format).to_string());
        }
        return Ok(value.strftime(format).to_string());
    }

    if let Ok(value) = text.parse::<Zoned>() {
        return Ok(value.strftime(format).to_string());
    }
    if let Ok(value) = text.parse::<DateTime>() {
        return Ok(value.strftime(format).to_string());
    }
    if let Ok(value) = text.parse::<Date>() {
        return Ok(value.strftime(format).to_string());
    }

    Err(SpiderError::rules(format!(
        "date_format could not parse datetime value {text:?}"
    )))
}

fn url_allowed(url: &str, patterns: &[String]) -> Result<bool, SpiderError> {
    if patterns.is_empty() {
        return Ok(true);
    }

    for pattern in patterns {
        let regex = Regex::new(pattern).map_err(|error| {
            SpiderError::rules(format!("invalid allow_url_pattern {pattern:?}: {error}"))
        })?;
        if regex.is_match(url) {
            return Ok(true);
        }
    }

    Ok(false)
}

fn resolve_url(base: &str, url: &str) -> Result<String, SpiderError> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return Ok(url.to_string());
    }

    let base = Url::parse(base)
        .map_err(|error| SpiderError::rules(format!("invalid base url {base:?}: {error}")))?;
    base.join(url)
        .map(|url| url.to_string())
        .map_err(|error| SpiderError::rules(format!("invalid relative url {url:?}: {error}")))
}

fn append_query(url: &str, query: &BTreeMap<String, String>) -> Result<String, SpiderError> {
    let mut parsed = Url::parse(url)
        .map_err(|error| SpiderError::rules(format!("invalid request url {url:?}: {error}")))?;
    {
        let mut pairs = parsed.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    Ok(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::{DEDUP, RATE_LIMIT};
    use crate::request::Request;
    use crate::rules::compile::compile_rules;
    use serde_json::json;

    fn html_response(url: &str, html: &str) -> Response {
        Response::from_request(
            Request::new(url),
            200,
            Headers::new(),
            html.as_bytes().to_vec(),
        )
    }

    #[tokio::test]
    async fn apply_runs_fields_bind_follow_with_single_chain_meta_binding() {
        let compiled = compile_rules(Value::from(json!({
            "spider": { "name": "demo" },
            "sinks": {
                "default": { "type": "memory" }
            },
            "engine": {
                "dedup": {
                    "request_url": {
                        "backend": "memory",
                        "key": ["url", "meta.channel"]
                    }
                },
                "rate_limit": {
                    "origin_budget": {
                        "bucket": "origin",
                        "rate_per_minute": 120
                    }
                }
            },
            "seeds": [{
                "id": "seed-a",
                "request": { "url": "https://example.com/start" },
                "next_step": "parse_list"
            }],
            "steps": [{
                "id": "parse_list",
                "fields": {
                    "channel": { "selector": "body", "attr": "data-channel" }
                },
                "bind": {
                    "channel_upper": {
                        "template": "{name}",
                        "vars": {
                            "name": { "from": "$fields.channel" }
                        },
                        "transforms": [{
                            "type": "replace",
                            "from": "news",
                            "to": "NEWS"
                        }]
                    }
                },
                "follow": [{
                    "item": ".news li",
                    "next_step": "parse_detail",
                    "request": {
                        "url": { "selector": "a", "attr": "href" },
                        "headers": {
                            "Referer": { "from": "$response.url" }
                        }
                    },
                    "meta": {
                        "title": { "selector": "a", "text": true },
                        "channel": { "from": "$bind.channel_upper" }
                    },
                    "allow_url_pattern": ["^https://example\\.com/detail/"],
                    "engine": {
                        "dedup": "request_url",
                        "rate_limit": "origin_budget"
                    }
                }]
            }, {
                "id": "parse_detail",
                "output": {
                    "item": { "title": { "from": "$meta.title" } },
                    "sinks": ["default"]
                }
            }]
        })))
        .expect("rules should compile");

        let response = html_response(
            "https://example.com/list",
            r#"
            <body data-channel="news">
              <ul class="news">
                <li><a href="/detail/1">First</a></li>
                <li><a href="mailto:test@example.com">Skip</a></li>
              </ul>
            </body>
            "#,
        );
        let step = &compiled.steps[0];

        let output = apply(&response, step, &compiled)
            .await
            .expect("run should work");

        assert!(output.items.is_empty());
        assert_eq!(output.requests.len(), 1);

        let request = &output.requests[0];
        assert_eq!(request.url, "https://example.com/detail/1");
        assert_eq!(
            request.meta.get("title"),
            Some(&Value::String("First".to_string()))
        );
        assert_eq!(
            request.meta.get("channel"),
            Some(&Value::String("NEWS".to_string()))
        );
        assert_eq!(
            request.meta.get("next_step"),
            Some(&Value::String("parse_detail".to_string()))
        );
        assert!(request.middleware_options(DEDUP).is_some());
        assert!(request.middleware_options(RATE_LIMIT).is_some());
    }

    #[test]
    fn build_seed_requests_uses_seed_plan_semantics() {
        let home = std::env::var("HOME").expect("HOME should exist for test");
        let compiled = compile_rules(Value::from(json!({
            "spider": {
                "name": "demo",
                "clock": {
                    "timezone": "Asia/Shanghai"
                }
            },
            "engine": {
                "dedup": {
                    "request_url": {
                        "backend": "memory",
                        "key": ["url"]
                    }
                },
                "rate_limit": {
                    "origin_budget": {
                        "bucket": "origin",
                        "rate_per_minute": 120
                    }
                }
            },
            "sinks": {
                "default": { "type": "memory" }
            },
            "seeds": [{
                "id": "seed-a",
                "request": {
                    "url": {
                        "template": "https://example.com/{year}/start",
                        "vars": {
                            "year": {
                                "from": "$now",
                                "transforms": [{
                                    "type": "date_format",
                                    "format": "%Y"
                                }]
                            }
                        }
                    },
                    "method": "POST",
                    "headers": {
                        "X-Env": { "from": "$env.HOME" }
                    },
                    "cb_kwargs": {
                        "channel": "news"
                    },
                    "flags": ["seeded"],
                    "skip": ["dedup"]
                },
                "meta": {
                    "source": "rules",
                    "env_value": { "from": "$env.HOME" }
                },
                "allow_url_pattern": ["^https://example\\.com/\\d{4}/start$"],
                "engine": {
                    "dedup": "request_url",
                    "rate_limit": "origin_budget"
                },
                "next_step": "parse"
            }],
            "steps": [{
                "id": "parse",
                "output": {
                    "item": { "url": { "from": "$response.url" } },
                    "sinks": ["default"]
                }
            }]
        })))
        .expect("rules should compile");

        let requests = build_seed_requests(&compiled).expect("seed requests should build");

        assert_eq!(requests.len(), 1);

        let request = &requests[0];
        assert_eq!(request.method, "POST");
        assert!(request.url.starts_with("https://example.com/"));
        assert!(request.url.ends_with("/start"));
        assert_eq!(
            request.meta.get("source"),
            Some(&Value::String("rules".to_string()))
        );
        assert_eq!(
            request.meta.get("env_value"),
            Some(&Value::String(home.clone()))
        );
        assert_eq!(
            request.meta.get("next_step"),
            Some(&Value::String("parse".to_string()))
        );
        assert_eq!(request.headers.get("X-Env"), Some(&vec![home]));
        assert_eq!(
            request.cb_kwargs.get("channel"),
            Some(&Value::String("news".to_string()))
        );
        assert_eq!(request.flags, vec!["seeded".to_string()]);
        assert!(request.middleware_skips(DEDUP));
        assert!(request.middleware_options(RATE_LIMIT).is_some());
    }

    #[tokio::test]
    async fn apply_builds_output_item_from_fields_bind_and_meta() {
        let compiled = compile_rules(Value::from(json!({
            "spider": { "name": "demo" },
            "sinks": {
                "default": { "type": "memory" }
            },
            "seeds": [{
                "id": "seed-a",
                "request": { "url": "https://example.com/start" },
                "next_step": "parse_detail"
            }],
            "steps": [{
                "id": "parse_detail",
                "fields": {
                    "title": { "selector": "h1", "text": true },
                    "content": { "selector": ".content", "text": true }
                },
                "bind": {
                    "clean_title": {
                        "from": "$fields.title",
                        "transforms": [{ "type": "trim" }]
                    }
                },
                "output": {
                    "item": {
                        "title": {
                            "from": "$meta.title",
                            "fallback": { "from": "$bind.clean_title" }
                        },
                        "content": { "from": "$fields.content" },
                        "source_url": { "from": "$response.url" }
                    },
                    "validate": {
                        "required": ["title", "content", "source_url"],
                        "fields": {
                            "title": { "type": "string", "min_length": 1 },
                            "content": { "type": "string", "min_length": 5 },
                            "source_url": { "type": "string", "format": "url" }
                        }
                    },
                    "sinks": ["default"]
                }
            }]
        })))
        .expect("rules should compile");

        let response = Response::from_request(
            Request::new("https://example.com/detail/1")
                .with_meta("title", Value::String(String::new())),
            200,
            Headers::new(),
            br#"<html><body><h1>  Detail Title  </h1><div class="content">hello world</div></body></html>"#
                .to_vec(),
        );

        let output = apply(&response, &compiled.steps[0], &compiled)
            .await
            .expect("run should succeed");

        assert_eq!(output.requests.len(), 0);
        assert_eq!(output.items.len(), 1);

        let item = &output.items[0];
        assert_eq!(
            item.get("title"),
            Some(&Value::String("Detail Title".to_string()))
        );
        assert_eq!(
            item.get("source_url"),
            Some(&Value::String("https://example.com/detail/1".to_string()))
        );
    }

    #[tokio::test]
    async fn apply_marks_request_skip_as_dedup_skip() {
        let compiled = compile_rules(Value::from(json!({
            "spider": { "name": "demo" },
            "sinks": {
                "default": { "type": "memory" }
            },
            "engine": {
                "dedup": {
                    "request_url": {
                        "backend": "memory",
                        "key": ["url"]
                    }
                }
            },
            "seeds": [{
                "id": "seed-a",
                "request": { "url": "https://example.com/start" },
                "next_step": "parse_list"
            }],
            "steps": [{
                "id": "parse_list",
                "follow": [{
                    "next_step": "parse_detail",
                    "request": {
                        "url": "https://example.com/detail/1",
                        "skip": ["dedup"]
                    },
                    "engine": {
                        "dedup": "request_url"
                    }
                }]
            }, {
                "id": "parse_detail",
                "output": {
                    "item": { "url": { "from": "$response.url" } },
                    "sinks": ["default"]
                }
            }]
        })))
        .expect("rules should compile");

        let response = html_response("https://example.com/list", "<html></html>");
        let output = apply(&response, &compiled.steps[0], &compiled)
            .await
            .expect("run should succeed");

        assert_eq!(output.requests.len(), 1);
        assert!(output.requests[0].middleware_skips(DEDUP));
    }
}
