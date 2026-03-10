use crate::error::SpiderError;
use crate::value::Value;
use std::collections::BTreeSet;

pub fn validate_rules(value: &Value) -> Result<(), SpiderError> {
    let root = value
        .as_object()
        .ok_or_else(|| SpiderError::rules("rules dsl must be an object"))?;
    let steps = root
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| SpiderError::rules("rules.steps must be an array"))?;

    let mut ids = BTreeSet::new();
    for step in steps {
        let step = step
            .as_object()
            .ok_or_else(|| SpiderError::rules("rules.steps[*] must be an object"))?;

        let id = step
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| SpiderError::rules("rules.steps[*].id is required"))?;

        if !ids.insert(id.to_string()) {
            return Err(SpiderError::rules(format!("duplicate step id: {id}")));
        }

        let step_impl = step
            .get("impl")
            .and_then(Value::as_str)
            .ok_or_else(|| SpiderError::rules(format!("step {id} missing impl")))?;

        match step_impl {
            "dsl" => {
                if step.get("callback").is_some() {
                    return Err(SpiderError::rules(format!(
                        "step {id} with impl=dsl must not define callback"
                    )));
                }
            }
            "code" => {
                if step.get("callback").and_then(Value::as_str).is_none() {
                    return Err(SpiderError::rules(format!(
                        "step {id} with impl=code must define callback"
                    )));
                }
            }
            other => {
                return Err(SpiderError::rules(format!(
                    "step {id} has unsupported impl: {other}"
                )));
            }
        }

        if let Some(fetch) = step.get("fetch") {
            let fetch = fetch
                .as_object()
                .ok_or_else(|| SpiderError::rules(format!("step {id} fetch must be an object")))?;

            if let Some(mode) = fetch.get("mode").and_then(Value::as_str) {
                match mode {
                    "http" | "browser" => {}
                    other => {
                        return Err(SpiderError::rules(format!(
                            "step {id} has unsupported fetch.mode: {other}"
                        )));
                    }
                }
            }
        }

        if let Some(parse) = step.get("parse") {
            let parse = parse
                .as_object()
                .ok_or_else(|| SpiderError::rules(format!("step {id} parse must be an object")))?;

            if let Some(fields) = parse.get("fields") {
                for field in expect_array(fields, &format!("step {id} parse.fields"))? {
                    let field = field.as_object().ok_or_else(|| {
                        SpiderError::rules(format!("step {id} parse.fields[*] must be an object"))
                    })?;
                    require_non_empty_string(field.get("name"), &format!("step {id} field.name"))?;
                    require_non_empty_string(
                        field.get("source"),
                        &format!("step {id} field.source"),
                    )?;
                    require_non_empty_string(
                        field.get("selector_type"),
                        &format!("step {id} field.selector_type"),
                    )?;
                    require_string_array(
                        field.get("selector"),
                        &format!("step {id} field.selector"),
                    )?;
                }
            }

            if let Some(links) = parse.get("links") {
                for link in expect_array(links, &format!("step {id} parse.links"))? {
                    let link = link.as_object().ok_or_else(|| {
                        SpiderError::rules(format!("step {id} parse.links[*] must be an object"))
                    })?;
                    require_non_empty_string(link.get("name"), &format!("step {id} link.name"))?;
                    require_non_empty_string(
                        link.get("source"),
                        &format!("step {id} link.source"),
                    )?;
                    require_non_empty_string(
                        link.get("selector_type"),
                        &format!("step {id} link.selector_type"),
                    )?;
                    require_string_array(
                        link.get("selector"),
                        &format!("step {id} link.selector"),
                    )?;
                    if let Some(target) = link.get("to") {
                        let target = target.as_object().ok_or_else(|| {
                            SpiderError::rules(format!("step {id} link.to must be an object"))
                        })?;
                        if target.get("next_step").and_then(Value::as_str).is_none() {
                            return Err(SpiderError::rules(format!(
                                "step {id} link.to.next_step is required"
                            )));
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn expect_array<'a>(value: &'a Value, label: &str) -> Result<&'a [Value], SpiderError> {
    value
        .as_array()
        .ok_or_else(|| SpiderError::rules(format!("{label} must be an array")))
}

fn require_non_empty_string(value: Option<&Value>, label: &str) -> Result<(), SpiderError> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(|_| ())
        .ok_or_else(|| SpiderError::rules(format!("{label} is required")))
}

fn require_string_array(value: Option<&Value>, label: &str) -> Result<(), SpiderError> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| SpiderError::rules(format!("{label} must be an array")))?;

    for value in values {
        if value.as_str().is_none() {
            return Err(SpiderError::rules(format!(
                "{label} entries must be strings"
            )));
        }
    }

    Ok(())
}
