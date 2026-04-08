use crate::error::SpiderError;
use crate::item::Item;
use crate::request::Request;
use crate::response::Response;
use crate::rules::schema::{
    Compiled, CompiledStep, FieldPlan, LinkPlan, ParsePlan, SelectorKind, SourceKind,
};
use crate::value::Value;
use regex::Regex;
use std::collections::{BTreeMap, HashSet};

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
    let parsed_fields = build_parsed_fields(response, &step.parse).await?;
    crate::validator::validate_fields(&parsed_fields, &step.validate)?;
    let item = build_item_from_fields(&parsed_fields);

    match step.step_type.as_deref() {
        Some("node") => {
            // 只生成 requests，不生成 items
            let requests = build_request_from_next_url_config(
                response,
                &step.parse,
                step,
                &compiled.steps,
                &parsed_fields,
            )
            .await?;
            Ok(Output {
                items: vec![],
                requests,
            })
        }
        Some("end") => {
            // 只生成 items，不生成 requests
            Ok(Output {
                items: item.into_iter().collect(),
                requests: vec![],
            })
        }
        _ => {
            // 默认行为：使用 links
            let requests = build_requests(
                response,
                &step.parse.links,
                step,
                &compiled.steps,
                &parsed_fields,
            )
            .await?;
            Ok(Output {
                items: item.into_iter().collect(),
                requests,
            })
        }
    }
}

async fn build_parsed_fields(
    response: &Response,
    parse: &ParsePlan,
) -> Result<BTreeMap<String, Value>, SpiderError> {
    let mut parsed_fields = BTreeMap::new();
    for field in &parse.fields {
        parsed_fields.insert(field.name.clone(), resolve_field(response, field).await?);
    }
    Ok(parsed_fields)
}

fn build_item_from_fields(parsed_fields: &BTreeMap<String, Value>) -> Option<Item> {
    if parsed_fields.is_empty() {
        return None;
    }

    let mut item = Item::new();
    for (key, value) in parsed_fields {
        item.insert(key.clone(), value.clone());
    }
    Some(item)
}

async fn build_requests(
    response: &Response,
    links: &[LinkPlan],
    current_step: &CompiledStep,
    all_steps: &[CompiledStep],
    parsed_fields: &BTreeMap<String, Value>,
) -> Result<Vec<Request>, SpiderError> {
    let mut requests = Vec::new();

    for link in links {
        let values = resolve_values(
            response,
            link.source,
            &link.source_ref,
            link.selector_type,
            &link.selector,
            &link.attribute,
            link.multiple,
        )
        .await?;
        let urls = filter_urls(values, &link.allow, &link.deny)?;

        if urls.is_empty() {
            if link.required {
                return Err(SpiderError::parse(format!(
                    "required link missing: {}",
                    link.name
                )));
            }
            continue;
        }

        for url in urls {
            let meta = build_request_meta(
                current_step,
                parsed_fields,
                &link.meta,
                link.next_step.as_deref(),
            );
            let request = response.follow_with_meta(url, &meta);
            requests.push(apply_target_step_fetch(request, all_steps)?);
        }
    }

    Ok(requests)
}

async fn build_request_from_next_url_config(
    response: &Response,
    parse: &ParsePlan,
    current_step: &CompiledStep,
    all_steps: &[CompiledStep],
    parsed_fields: &BTreeMap<String, Value>,
) -> Result<Vec<Request>, SpiderError> {
    if parse.next_url_config.is_empty() {
        return Ok(vec![]);
    }

    // 构造 URLs
    let urls = build_next_urls(response, &parse.next_url_config, parsed_fields)?;

    // 找到下一个 step
    let current_idx = all_steps.iter().position(|s| s.id == current_step.id);
    let next_step_id = current_idx
        .and_then(|idx| all_steps.get(idx + 1))
        .map(|s| s.id.clone());

    let next_step = next_step_id.as_deref();

    Ok(urls
        .into_iter()
        .map(|url| {
            let meta = build_request_meta(current_step, parsed_fields, &BTreeMap::new(), next_step);
            let request = response.follow_with_meta(url, &meta);
            apply_target_step_fetch(request, all_steps)
        })
        .collect::<Result<Vec<_>, _>>()?)
}

fn apply_target_step_fetch(
    request: Request,
    all_steps: &[CompiledStep],
) -> Result<Request, SpiderError> {
    let step_id = request
        .meta
        .get("next_step")
        .and_then(Value::as_str)
        .unwrap_or("parse");
    let step = all_steps
        .iter()
        .find(|step| step.id == step_id)
        .ok_or_else(|| SpiderError::engine(format!("step not found: {step_id}")))?;
    Ok(step.fetch.apply_to_request(request))
}

fn build_request_meta(
    current_step: &CompiledStep,
    parsed_fields: &BTreeMap<String, Value>,
    extra_meta: &BTreeMap<String, Value>,
    next_step: Option<&str>,
) -> BTreeMap<String, Value> {
    let mut meta = BTreeMap::new();

    if let Some(step_meta) = &current_step.meta {
        meta.extend(step_meta.clone());
    }

    meta.extend(parsed_fields.clone());
    meta.extend(extra_meta.clone());

    if let Some(next_step) = next_step {
        meta.insert(
            "next_step".to_string(),
            Value::String(next_step.to_string()),
        );
    }

    meta
}

async fn resolve_field(response: &Response, field: &FieldPlan) -> Result<Value, SpiderError> {
    let values = resolve_values(
        response,
        field.source,
        &field.source_ref,
        field.selector_type,
        &field.selector,
        &field.attribute,
        field.multiple,
    )
    .await?;

    if field.multiple {
        if values.is_empty() {
            return fallback(&field.default, field.required, &field.name, true);
        }
        return Ok(Value::Array(
            values.into_iter().map(Value::String).collect(),
        ));
    }

    if let Some(value) = values.into_iter().next() {
        return Ok(Value::String(value));
    }

    fallback(&field.default, field.required, &field.name, false)
}

fn fallback(
    default: &Value,
    required: bool,
    name: &str,
    multiple: bool,
) -> Result<Value, SpiderError> {
    if !matches!(default, Value::Null) {
        return Ok(default.clone());
    }

    if required {
        return Err(SpiderError::parse(format!(
            "required field missing: {name}"
        )));
    }

    if multiple {
        Ok(Value::Array(Vec::new()))
    } else {
        Ok(Value::Null)
    }
}

async fn resolve_values(
    response: &Response,
    source: SourceKind,
    source_ref: &str,
    selector_type: SelectorKind,
    selectors: &[String],
    attribute: &str,
    multiple: bool,
) -> Result<Vec<String>, SpiderError> {
    let mut values = Vec::new();

    for selector in selectors {
        let current = match (source, selector_type) {
            (SourceKind::Html, SelectorKind::Css) => select_css(response, selector, attribute),
            (SourceKind::Html, SelectorKind::XPath) => select_xpath(response, selector, attribute),
            (SourceKind::Html, SelectorKind::Regex) | (SourceKind::Text, SelectorKind::Regex) => {
                select_regex(response, selector, attribute)
            }
            (SourceKind::Html, SelectorKind::Ai) => select_ai(response, selector).await?,
            (SourceKind::Json, SelectorKind::Json) => select_json(response, selector, multiple),
            (SourceKind::Xml, SelectorKind::Xml) => select_xml(response, selector, attribute),
            (SourceKind::Xml, SelectorKind::XPath) => select_xpath(response, selector, attribute),
            (SourceKind::Headers, _) => select_headers(response, selector),
            (SourceKind::Url, _) => vec![response.url.clone()],
            (SourceKind::Meta, _) => select_meta(response, source_ref),
            _ => {
                return Err(SpiderError::parse(format!(
                    "unsupported source/selector_type combination: {:?}/{:?}",
                    source, selector_type
                )));
            }
        };

        if multiple {
            values.extend(current);
        } else if let Some(value) = current.into_iter().next() {
            return Ok(vec![value]);
        }
    }

    Ok(values)
}

fn select_css(response: &Response, selector: &str, attribute: &str) -> Vec<String> {
    match attribute {
        "text" => response.css(selector).text().all(),
        "html" => response.css(selector).html().all(),
        value if value.starts_with("attr:") => response.css(selector).attr(&value[5..]).all(),
        _ => response.css(selector).all(),
    }
}

fn select_xpath(response: &Response, selector: &str, attribute: &str) -> Vec<String> {
    match attribute {
        "text" => response.xpath(selector).text().all(),
        "html" => response.xpath(selector).html().all(),
        value if value.starts_with("attr:") => response.xpath(selector).attr(&value[5..]).all(),
        _ => response.xpath(selector).all(),
    }
}

fn select_xml(response: &Response, selector: &str, attribute: &str) -> Vec<String> {
    match attribute {
        "text" => response.xml(selector).text().all(),
        "html" => response.xml(selector).html().all(),
        value if value.starts_with("attr:") => response.xml(selector).attr(&value[5..]).all(),
        _ => response.xml(selector).all(),
    }
}

fn select_json(response: &Response, selector: &str, multiple: bool) -> Vec<String> {
    let query = response.json(Some(selector));
    if multiple {
        query.all()
    } else {
        query.one().into_iter().collect()
    }
}

fn select_regex(response: &Response, selector: &str, attribute: &str) -> Vec<String> {
    let query = response.regex(selector);
    if attribute == "text" {
        return query.all();
    }
    if let Some(index) = attribute.strip_prefix("group:")
        && let Ok(index) = index.parse::<usize>()
    {
        return query.group(index).into_iter().collect();
    }
    query.all()
}

async fn select_ai(response: &Response, prompt: &str) -> Result<Vec<String>, SpiderError> {
    let mut query = response.ai(prompt);
    query.execute().await.map_err(SpiderError::parse)?;
    Ok(query.all())
}

fn select_headers(response: &Response, selector: &str) -> Vec<String> {
    response.headers.get(selector).cloned().unwrap_or_default()
}

fn select_meta(response: &Response, source_ref: &str) -> Vec<String> {
    let Some(key) = source_ref.strip_prefix("meta.") else {
        return Vec::new();
    };
    response
        .meta
        .get(key)
        .and_then(|value| value.as_str().map(str::to_string))
        .into_iter()
        .collect()
}

fn filter_urls(
    values: Vec<String>,
    allow: &[String],
    deny: &[String],
) -> Result<Vec<String>, SpiderError> {
    let allow = compile_patterns(allow)?;
    let deny = compile_patterns(deny)?;

    Ok(values
        .into_iter()
        .filter(|value| allow.is_empty() || allow.iter().any(|pattern| pattern.is_match(value)))
        .filter(|value| !deny.iter().any(|pattern| pattern.is_match(value)))
        .collect())
}

fn compile_patterns(patterns: &[String]) -> Result<Vec<Regex>, SpiderError> {
    patterns
        .iter()
        .map(|pattern| {
            Regex::new(pattern).map_err(|error| {
                SpiderError::parse(format!("invalid regex pattern {pattern}: {error}"))
            })
        })
        .collect()
}

fn build_next_urls(
    response: &Response,
    config: &BTreeMap<String, Value>,
    parsed_fields: &BTreeMap<String, Value>,
) -> Result<Vec<String>, SpiderError> {
    let mode = config
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| SpiderError::parse("next_url_config.mode is required"))?;

    let urls = match mode {
        "FIELD" => build_from_field(config, parsed_fields)?,
        "TEMPLATE" => build_from_template(config, parsed_fields, response)?,
        "JOIN" => build_from_join(config, parsed_fields)?,
        "FUNCTION" => build_from_function(config, parsed_fields, response)?,
        other => return Err(SpiderError::parse(format!("unsupported mode: {}", other))),
    };

    normalize_urls(response, urls)
}

fn build_from_field(
    config: &BTreeMap<String, Value>,
    parsed_fields: &BTreeMap<String, Value>,
) -> Result<Vec<String>, SpiderError> {
    let from = config
        .get("from")
        .and_then(Value::as_array)
        .ok_or_else(|| SpiderError::parse("FIELD mode requires 'from'"))?;

    if from.len() != 1 {
        return Err(SpiderError::parse("FIELD mode requires exactly one field"));
    }

    let field_name = from[0]
        .as_str()
        .ok_or_else(|| SpiderError::parse("from[0] must be string"))?;

    let value = parsed_fields
        .get(field_name)
        .ok_or_else(|| SpiderError::parse(format!("Field '{}' not found", field_name)))?;

    match value {
        Value::String(s) => Ok(vec![s.clone()]),
        Value::Array(arr) => Ok(arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()),
        _ => Err(SpiderError::parse("Field value must be string or array")),
    }
}

fn build_from_template(
    config: &BTreeMap<String, Value>,
    parsed_fields: &BTreeMap<String, Value>,
    response: &Response,
) -> Result<Vec<String>, SpiderError> {
    let template = config
        .get("template")
        .and_then(Value::as_str)
        .ok_or_else(|| SpiderError::parse("TEMPLATE mode requires 'template'"))?;

    let mut url = template.to_string();

    // 替换 {field}
    for (key, value) in parsed_fields {
        let placeholder = format!("{{{}}}", key);
        if url.contains(&placeholder) {
            let value_str = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            url = url.replace(&placeholder, &value_str);
        }
    }

    // 替换 {meta.xxx}
    for (key, value) in &response.meta {
        let placeholder = format!("{{meta.{}}}", key);
        if url.contains(&placeholder)
            && let Some(s) = value.as_str()
        {
            url = url.replace(&placeholder, s);
        }
    }

    Ok(vec![url])
}

fn build_from_join(
    config: &BTreeMap<String, Value>,
    parsed_fields: &BTreeMap<String, Value>,
) -> Result<Vec<String>, SpiderError> {
    let from = config
        .get("from")
        .and_then(Value::as_array)
        .ok_or_else(|| SpiderError::parse("JOIN mode requires 'from'"))?;

    if from.len() < 2 {
        return Err(SpiderError::parse("JOIN mode requires at least 2 fields"));
    }

    let delimiter = config
        .get("join_delimiter")
        .and_then(Value::as_str)
        .unwrap_or("");

    let parts: Vec<String> = from
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(|field_name| {
            parsed_fields
                .get(field_name)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    if parts.len() != from.len() {
        return Err(SpiderError::parse("Some fields not found for JOIN"));
    }

    Ok(vec![parts.join(delimiter)])
}

fn build_from_function(
    config: &BTreeMap<String, Value>,
    parsed_fields: &BTreeMap<String, Value>,
    response: &Response,
) -> Result<Vec<String>, SpiderError> {
    let function_name = config
        .get("fn")
        .and_then(Value::as_str)
        .ok_or_else(|| SpiderError::parse("FUNCTION mode requires 'fn'"))?;
    let args = config
        .get("args")
        .and_then(Value::as_array)
        .ok_or_else(|| SpiderError::parse("FUNCTION mode requires 'args'"))?;

    Ok(vec![evaluate_function_call(
        function_name,
        args,
        parsed_fields,
        response,
    )?])
}

fn evaluate_function_call(
    function_name: &str,
    args: &[Value],
    parsed_fields: &BTreeMap<String, Value>,
    response: &Response,
) -> Result<String, SpiderError> {
    match function_name {
        "concat" => {
            let parts = args
                .iter()
                .enumerate()
                .map(|(index, arg)| {
                    resolve_function_arg_required(
                        arg,
                        &format!("concat.args[{index}]"),
                        parsed_fields,
                        response,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;

            Ok(parts.concat())
        }
        "replace" => {
            if args.len() != 3 {
                return Err(SpiderError::parse(
                    "FUNCTION replace requires exactly 3 args",
                ));
            }

            let input = resolve_function_arg_required(
                &args[0],
                "replace.args[0]",
                parsed_fields,
                response,
            )?;
            let from = resolve_function_arg_required(
                &args[1],
                "replace.args[1]",
                parsed_fields,
                response,
            )?;
            let to = resolve_function_arg_required(
                &args[2],
                "replace.args[2]",
                parsed_fields,
                response,
            )?;

            if from.is_empty() {
                return Err(SpiderError::parse(
                    "FUNCTION replace.args[1] must not be empty",
                ));
            }

            Ok(input.replace(&from, &to))
        }
        "coalesce" => {
            for (index, arg) in args.iter().enumerate() {
                if let Some(value) = resolve_function_arg(
                    arg,
                    parsed_fields,
                    response,
                    &format!("coalesce.args[{index}]"),
                )? && !value.trim().is_empty()
                {
                    return Ok(value);
                }
            }

            Err(SpiderError::parse(
                "FUNCTION coalesce could not resolve a non-empty value",
            ))
        }
        other => Err(SpiderError::parse(format!(
            "unsupported FUNCTION fn: {other}"
        ))),
    }
}

fn resolve_function_arg_required(
    arg: &Value,
    label: &str,
    parsed_fields: &BTreeMap<String, Value>,
    response: &Response,
) -> Result<String, SpiderError> {
    resolve_function_arg(arg, parsed_fields, response, label)?
        .ok_or_else(|| SpiderError::parse(format!("FUNCTION {label} resolved to no value")))
}

fn resolve_function_arg(
    arg: &Value,
    parsed_fields: &BTreeMap<String, Value>,
    response: &Response,
    label: &str,
) -> Result<Option<String>, SpiderError> {
    match arg {
        Value::Null => Ok(None),
        Value::String(_) | Value::Number(_) | Value::Bool(_) => stringify_scalar_value(arg, label),
        Value::Object(object) => {
            resolve_function_object_arg(object, parsed_fields, response, label)
        }
        Value::Array(_) => Err(SpiderError::parse(format!(
            "FUNCTION {label} must be a scalar, reference, or nested function"
        ))),
    }
}

fn resolve_function_object_arg(
    object: &BTreeMap<String, Value>,
    parsed_fields: &BTreeMap<String, Value>,
    response: &Response,
    label: &str,
) -> Result<Option<String>, SpiderError> {
    let mut branch_count = 0;
    branch_count += usize::from(object.contains_key("value"));
    branch_count += usize::from(object.contains_key("field"));
    branch_count += usize::from(object.contains_key("meta"));
    branch_count += usize::from(object.contains_key("fn"));

    if branch_count != 1 {
        return Err(SpiderError::parse(format!(
            "FUNCTION {label} object must contain exactly one of: value, field, meta, fn"
        )));
    }

    if let Some(value) = object.get("value") {
        return stringify_scalar_value(value, &format!("{label}.value"));
    }

    if let Some(field_name) = object.get("field").and_then(Value::as_str) {
        return stringify_scalar_value(
            parsed_fields.get(field_name).unwrap_or(&Value::Null),
            &format!("{label}.field({field_name})"),
        );
    }

    if let Some(meta_key) = object.get("meta").and_then(Value::as_str) {
        return stringify_scalar_value(
            response.meta.get(meta_key).unwrap_or(&Value::Null),
            &format!("{label}.meta({meta_key})"),
        );
    }

    let function_name = object
        .get("fn")
        .and_then(Value::as_str)
        .ok_or_else(|| SpiderError::parse(format!("FUNCTION {label}.fn is required")))?;
    let args = object
        .get("args")
        .and_then(Value::as_array)
        .ok_or_else(|| SpiderError::parse(format!("FUNCTION {label}.args is required")))?;

    Ok(Some(evaluate_function_call(
        function_name,
        args,
        parsed_fields,
        response,
    )?))
}

fn stringify_scalar_value(value: &Value, label: &str) -> Result<Option<String>, SpiderError> {
    match value {
        Value::Null => Ok(None),
        Value::String(value) => Ok(Some(value.clone())),
        Value::Number(value) => Ok(Some(value.to_string())),
        Value::Bool(value) => Ok(Some(value.to_string())),
        Value::Array(_) | Value::Object(_) => Err(SpiderError::parse(format!(
            "FUNCTION {label} must resolve to a scalar value"
        ))),
    }
}

fn normalize_urls(response: &Response, urls: Vec<String>) -> Result<Vec<String>, SpiderError> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();

    for url in urls {
        let url = url.trim();
        if url.is_empty() {
            continue;
        }

        // 相对路径补全
        let resolved = resolve_url(&response.url, url);

        // 只保留 http/https
        if !resolved.starts_with("http://") && !resolved.starts_with("https://") {
            continue;
        }

        // 去重
        if seen.insert(resolved.clone()) {
            normalized.push(resolved);
        }
    }

    Ok(normalized)
}

fn resolve_url(base: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }

    if let Ok(base_url) = url::Url::parse(base)
        && let Ok(resolved) = base_url.join(url)
    {
        return resolved.to_string();
    }

    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Request, RequestMode};
    use jiff::SignedDuration;
    use serde_json::json;

    fn compile_test_rules() -> Compiled {
        crate::rules::compile::compile_rules(Value::from(json!({
            "steps": [
                {
                    "id": "parse",
                    "fetch": {
                        "mode": "browser",
                        "request": {
                            "headers": { "x-parent": "parent" },
                            "cookies": { "sid": "cookie-1" },
                            "timeout": 1000,
                            "proxy": "http://proxy.parent:8080",
                            "session": "parent-session"
                        },
                        "browser": {
                            "wait_for_selector": "#list"
                        }
                    },
                    "parse": {
                        "links": [
                            {
                                "name": "detail",
                                "source": "html",
                                "selector_type": "css",
                                "selector": [".detail-link"],
                                "attribute": "attr:href",
                                "next_step": "detail"
                            }
                        ]
                    }
                },
                {
                    "id": "detail",
                    "fetch": {
                        "mode": "http",
                        "request": {
                            "method": "POST",
                            "headers": { "x-step": "detail" },
                            "body": "payload",
                            "cookies": { "token": "cookie-2" },
                            "timeout": 5000,
                            "proxy": "http://proxy.detail:8080",
                            "session": "detail-session",
                            "dont_filter": true,
                            "query": { "page": "2" },
                            "allow_redirects": false
                        }
                    },
                    "parse": {}
                }
            ]
        })))
        .expect("rules should compile")
    }

    #[tokio::test]
    async fn dsl_follow_requests_apply_target_step_fetch_overrides() {
        let compiled = compile_test_rules();
        let parse_step = compiled
            .steps
            .iter()
            .find(|step| step.id == "parse")
            .expect("parse step should exist");
        let request = Request::browser("https://example.com/list")
            .with_header("x-parent", "parent")
            .with_cookie("sid", "cookie-1")
            .with_timeout(SignedDuration::from_secs(1))
            .with_proxy("http://proxy.parent:8080")
            .with_session("parent-session");
        let response = Response::from_request(
            request,
            200,
            Default::default(),
            br#"<html><body><a class="detail-link" href="/detail/1">detail</a></body></html>"#
                .to_vec(),
        );

        let output = apply(&response, parse_step, &compiled)
            .await
            .expect("dsl step should succeed");
        let request = output
            .requests
            .first()
            .expect("follow request should exist");

        assert_eq!(request.url, "https://example.com/detail/1");
        assert_eq!(request.mode, RequestMode::Http);
        assert_eq!(request.method, "POST");
        assert_eq!(request.body.as_deref(), Some(&b"payload"[..]));
        assert_eq!(
            request.meta.get("next_step").and_then(Value::as_str),
            Some("detail")
        );
        assert_eq!(
            request.headers.get("x-parent"),
            Some(&vec!["parent".to_string()])
        );
        assert_eq!(
            request.headers.get("x-step"),
            Some(&vec!["detail".to_string()])
        );
        assert_eq!(
            request.cookies.get("sid").map(String::as_str),
            Some("cookie-1")
        );
        assert_eq!(
            request.cookies.get("token").map(String::as_str),
            Some("cookie-2")
        );
        assert_eq!(request.timeout, Some(SignedDuration::from_secs(5)));
        assert_eq!(
            request.proxy.as_ref().map(|proxy| proxy.url.as_str()),
            Some("http://proxy.detail:8080")
        );
        assert_eq!(
            request.session.as_ref().map(|session| session.id.as_str()),
            Some("detail-session")
        );
        assert!(request.dont_filter);
        assert!(request.http.as_ref().is_some_and(
            |http| !http.allow_redirects && http.query.get("page") == Some(&"2".to_string())
        ));
        assert!(request.browser.is_none());
    }

    #[test]
    fn dsl_browser_config_can_compile_mobile_fingerprint_profile() {
        let compiled = crate::rules::compile::compile_rules(Value::from(json!({
            "steps": [
                {
                    "id": "mobile",
                    "fetch": {
                        "mode": "browser",
                        "browser": {
                            "stealth": true,
                            "device_profile": {
                                "fingerprint": {
                                    "mobile": true,
                                    "locale": "en-US"
                                }
                            }
                        }
                    },
                    "parse": {}
                }
            ]
        })))
        .expect("rules should compile");

        let browser = compiled
            .steps
            .first()
            .and_then(|step| step.fetch.browser.as_ref())
            .expect("browser config should exist");
        let fingerprint = browser
            .device_profile
            .as_ref()
            .and_then(|profile| profile.fingerprint.as_ref())
            .expect("fingerprint profile should exist");

        assert!(browser.stealth);
        assert_eq!(fingerprint.mobile, Some(true));
        assert_eq!(fingerprint.locale.as_deref(), Some("en-US"));
    }
}
