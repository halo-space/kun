use crate::error::SpiderError;
use crate::item::Item;
use crate::request::Request;
use crate::response::Response;
use crate::rules::schema::{CompiledStep, FieldPlan, LinkPlan, ParsePlan, SelectorKind, SourceKind};
use crate::value::Value;
use regex::Regex;

#[derive(Debug, Default)]
pub struct Output {
    pub items: Vec<Item>,
    pub requests: Vec<Request>,
}

pub fn apply(response: &Response, step: &CompiledStep) -> Result<Output, SpiderError> {
    let item = build_item(response, &step.parse)?;
    let requests = build_requests(response, &step.parse.links)?;

    Ok(Output {
        items: item.into_iter().collect(),
        requests,
    })
}

fn build_item(response: &Response, parse: &ParsePlan) -> Result<Option<Item>, SpiderError> {
    if parse.fields.is_empty() {
        return Ok(None);
    }

    let mut item = Item::new();
    for field in &parse.fields {
        item.insert(field.name.clone(), resolve_field(response, field)?);
    }

    Ok(Some(item))
}

fn build_requests(response: &Response, links: &[LinkPlan]) -> Result<Vec<Request>, SpiderError> {
    let mut requests = Vec::new();

    for link in links {
        let values = resolve_values(response, link.source, &link.source_ref, link.selector_type, &link.selector, &link.attribute, link.multiple)?;
        let urls = filter_urls(values, &link.allow, &link.deny)?;

        if urls.is_empty() {
            if link.required {
                return Err(SpiderError::parse(format!("required link missing: {}", link.name)));
            }
            continue;
        }

        for url in urls {
            let mut meta = link.to.meta_patch.clone();
            if let Some(next_step) = &link.to.next_step {
                meta.insert("next_step".to_string(), Value::String(next_step.clone()));
            }

            requests.push(response.follow_with_meta(url, &meta));
        }
    }

    Ok(requests)
}

fn resolve_field(response: &Response, field: &FieldPlan) -> Result<Value, SpiderError> {
    let values = resolve_values(
        response,
        field.source,
        &field.source_ref,
        field.selector_type,
        &field.selector,
        &field.attribute,
        field.multiple,
    )?;

    if field.multiple {
        if values.is_empty() {
            return fallback(&field.default, field.required, &field.name, true);
        }
        return Ok(Value::Array(values.into_iter().map(Value::String).collect()));
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
        return Err(SpiderError::parse(format!("required field missing: {name}")));
    }

    if multiple {
        Ok(Value::Array(Vec::new()))
    } else {
        Ok(Value::Null)
    }
}

fn resolve_values(
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
            (SourceKind::Html, SelectorKind::Ai) => select_ai(response, selector),
            (SourceKind::Json, SelectorKind::Json) => select_json(response, selector, multiple),
            (SourceKind::Xml, SelectorKind::Xml) => select_xml(response, selector, attribute),
            (SourceKind::Xml, SelectorKind::XPath) => select_xpath(response, selector, attribute),
            (SourceKind::Headers, _) => select_headers(response, selector),
            (SourceKind::FinalUrl, _) => vec![response.url.clone()],
            (SourceKind::Meta, _) => select_meta(response, source_ref),
            _ => {
                return Err(SpiderError::parse(format!(
                    "unsupported source/selector_type combination: {:?}/{:?}",
                    source, selector_type
                )))
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
    if let Some(index) = attribute.strip_prefix("group:") {
        if let Ok(index) = index.parse::<usize>() {
            return query.group(index).into_iter().collect();
        }
    }
    query.all()
}

fn select_ai(response: &Response, prompt: &str) -> Vec<String> {
    response.ai(prompt).all()
}

fn select_headers(response: &Response, selector: &str) -> Vec<String> {
    response
        .headers
        .get(selector)
        .cloned()
        .unwrap_or_default()
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

fn filter_urls(values: Vec<String>, allow: &[String], deny: &[String]) -> Result<Vec<String>, SpiderError> {
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
            Regex::new(pattern)
                .map_err(|error| SpiderError::parse(format!("invalid regex pattern {pattern}: {error}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::rules::compile::compile_rules;
    use crate::value::Value;

    #[test]
    fn apply_builds_item_from_field_rules() {
        let compiled = compile_rules(Value::Object(
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
                                                Value::String("css".to_string()),
                                            ),
                                            (
                                                "selector".to_string(),
                                                Value::Array(vec![Value::String("h1.title".to_string())]),
                                            ),
                                            (
                                                "attribute".to_string(),
                                                Value::String("text".to_string()),
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
        ))
        .unwrap();

        let response = Response::new(
            "https://example.com",
            200,
            Default::default(),
            b"<h1 class='title'>Hello</h1>".to_vec(),
        );
        let output = apply(&response, &compiled.steps[0]).unwrap();

        assert_eq!(output.items.len(), 1);
        assert_eq!(
            output.items[0].get("title"),
            Some(&Value::String("Hello".to_string()))
        );
    }

    #[test]
    fn apply_builds_requests_from_link_rules() {
        let compiled = compile_rules(Value::String(
            r#"{
                "steps":[
                    {
                        "id":"parse",
                        "impl":"dsl",
                        "parse":{
                            "links":[
                                {
                                    "name":"detail",
                                    "source":"html",
                                    "selector_type":"css",
                                    "selector":["a.detail"],
                                    "attribute":"attr:href",
                                    "allow":["^https://example.com/detail/\\d+$"],
                                    "deny":["2$"],
                                    "to":{
                                        "next_step":"detail",
                                        "meta_patch":{"from_list":true}
                                    }
                                }
                            ]
                        }
                    }
                ]
            }"#
            .to_string(),
        ))
        .unwrap();

        let response = Response::from_request(
            Request::new("https://example.com/list").with_meta("page", Value::Number(1.0)),
            200,
            Default::default(),
            br#"<a class="detail" href="https://example.com/detail/1">1</a><a class="detail" href="https://example.com/detail/2">2</a>"#.to_vec(),
        );

        let output = apply(&response, &compiled.steps[0]).unwrap();

        assert_eq!(output.requests.len(), 1);
        assert_eq!(output.requests[0].url, "https://example.com/detail/1");
        assert_eq!(output.requests[0].meta.get("page"), Some(&Value::Number(1.0)));
        assert_eq!(
            output.requests[0].meta.get("from_list"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            output.requests[0].meta.get("next_step"),
            Some(&Value::String("detail".to_string()))
        );
    }
}
