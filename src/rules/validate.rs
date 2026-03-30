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

        if let Some(callback) = step.get("callback") {
            require_non_empty_string(Some(callback), &format!("step {id} callback"))?;
        }

        // 如果有 type，验证其值
        if let Some(step_type) = step.get("type").and_then(Value::as_str) {
            match step_type {
                "node" | "end" => {}
                other => {
                    return Err(SpiderError::rules(format!(
                        "step {id} has unsupported type: {other}"
                    )));
                }
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
                    if link.get("to").is_some() {
                        return Err(SpiderError::rules(format!(
                            "step {id} link.to has been removed; use link.next_step and link.meta"
                        )));
                    }
                    if let Some(next_step) = link.get("next_step") {
                        require_non_empty_string(
                            Some(next_step),
                            &format!("step {id} link.next_step"),
                        )?;
                    }
                    if let Some(meta) = link.get("meta") {
                        meta.as_object().ok_or_else(|| {
                            SpiderError::rules(format!("step {id} link.meta must be an object"))
                        })?;
                    }
                }
            }

            if let Some(next_url_config) = parse.get("next_url_config") {
                validate_next_url_config(next_url_config, id)?;
            }
        }

        if let Some(validate) = step.get("validate") {
            validate_step_validate(validate, id)?;
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

fn validate_next_url_config(value: &Value, step_id: &str) -> Result<(), SpiderError> {
    let config = value.as_object().ok_or_else(|| {
        SpiderError::rules(format!("step {step_id} next_url_config must be an object"))
    })?;
    let mode = config.get("mode").and_then(Value::as_str).ok_or_else(|| {
        SpiderError::rules(format!("step {step_id} next_url_config.mode is required"))
    })?;

    match mode {
        "FIELD" => {
            let from = config
                .get("from")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    SpiderError::rules(format!(
                        "step {step_id} next_url_config.from must be an array"
                    ))
                })?;

            if from.len() != 1 {
                return Err(SpiderError::rules(format!(
                    "step {step_id} FIELD next_url_config requires exactly one from entry"
                )));
            }

            for (index, entry) in from.iter().enumerate() {
                require_non_empty_string(
                    Some(entry),
                    &format!("step {step_id} next_url_config.from[{index}]"),
                )?;
            }
        }
        "TEMPLATE" => {
            require_non_empty_string(
                config.get("template"),
                &format!("step {step_id} next_url_config.template"),
            )?;
        }
        "JOIN" => {
            let from = config
                .get("from")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    SpiderError::rules(format!(
                        "step {step_id} next_url_config.from must be an array"
                    ))
                })?;

            if from.len() < 2 {
                return Err(SpiderError::rules(format!(
                    "step {step_id} JOIN next_url_config requires at least two from entries"
                )));
            }

            for (index, entry) in from.iter().enumerate() {
                require_non_empty_string(
                    Some(entry),
                    &format!("step {step_id} next_url_config.from[{index}]"),
                )?;
            }
        }
        "FUNCTION" => validate_function_config(config, &format!("step {step_id} next_url_config"))?,
        other => {
            return Err(SpiderError::rules(format!(
                "step {step_id} has unsupported next_url_config.mode: {other}"
            )));
        }
    }

    Ok(())
}

fn validate_step_validate(value: &Value, step_id: &str) -> Result<(), SpiderError> {
    for entry in expect_array(value, &format!("step {step_id} validate"))? {
        let entry = entry.as_object().ok_or_else(|| {
            SpiderError::rules(format!("step {step_id} validate[*] must be an object"))
        })?;

        require_non_empty_string(entry.get("name"), &format!("step {step_id} validate.name"))?;
        let value_type = entry
            .get("type")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                SpiderError::rules(format!("step {step_id} validate.type is required"))
            })?;

        match value_type {
            "text" | "number" | "bool" | "list" | "object" => {}
            other => {
                return Err(SpiderError::rules(format!(
                    "step {step_id} validate.type has unsupported value: {other}"
                )));
            }
        }

        if let Some(rule) = entry.get("rule") {
            let rule = rule.as_object().ok_or_else(|| {
                SpiderError::rules(format!("step {step_id} validate.rule must be an object"))
            })?;
            if let Some(required) = rule.get("required")
                && required.as_bool().is_none()
            {
                return Err(SpiderError::rules(format!(
                    "step {step_id} validate.rule.required must be a boolean"
                )));
            }
        }
    }

    Ok(())
}

fn validate_function_config(
    config: &std::collections::BTreeMap<String, Value>,
    label: &str,
) -> Result<(), SpiderError> {
    let function_name = config
        .get("fn")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SpiderError::rules(format!("{label}.fn is required")))?;
    let args = config
        .get("args")
        .and_then(Value::as_array)
        .ok_or_else(|| SpiderError::rules(format!("{label}.args must be an array")))?;

    match function_name {
        "concat" | "coalesce" => {
            if args.is_empty() {
                return Err(SpiderError::rules(format!(
                    "{label}.args must contain at least one entry for {function_name}"
                )));
            }
        }
        "replace" => {
            if args.len() != 3 {
                return Err(SpiderError::rules(format!(
                    "{label}.args must contain exactly 3 entries for replace"
                )));
            }
        }
        other => {
            return Err(SpiderError::rules(format!(
                "{label}.fn has unsupported value: {other}"
            )));
        }
    }

    for (index, arg) in args.iter().enumerate() {
        validate_function_arg(arg, &format!("{label}.args[{index}]"))?;
    }

    Ok(())
}

fn validate_function_arg(value: &Value, label: &str) -> Result<(), SpiderError> {
    match value {
        Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(()),
        Value::Object(object) => {
            let mut branch_count = 0;
            branch_count += usize::from(object.contains_key("value"));
            branch_count += usize::from(object.contains_key("field"));
            branch_count += usize::from(object.contains_key("meta"));
            branch_count += usize::from(object.contains_key("fn"));

            if branch_count != 1 {
                return Err(SpiderError::rules(format!(
                    "{label} must contain exactly one of value, field, meta, fn"
                )));
            }

            if let Some(value) = object.get("value") {
                match value {
                    Value::Null | Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(()),
                    Value::Array(_) | Value::Object(_) => Err(SpiderError::rules(format!(
                        "{label}.value must be a scalar or null"
                    ))),
                }?;
            }

            if object.contains_key("field") {
                require_non_empty_string(object.get("field"), &format!("{label}.field"))?;
            }

            if object.contains_key("meta") {
                require_non_empty_string(object.get("meta"), &format!("{label}.meta"))?;
            }

            if object.contains_key("fn") {
                validate_function_config(object, label)?;
            }

            Ok(())
        }
        Value::Null | Value::Array(_) => Err(SpiderError::rules(format!(
            "{label} must be a scalar literal or object expression"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rules_accepts_function_next_url_config() {
        let rules = Value::String(
            r#"{
                "steps":[
                    {
                        "id":"parse",
                        "type":"node",
                        "parse":{
                            "next_url_config":{
                                "mode":"FUNCTION",
                                "fn":"concat",
                                "args":[
                                    "https://example.com/",
                                    {
                                        "fn":"replace",
                                        "args":[
                                            {"meta":"period_date"},
                                            "-",
                                            "/"
                                        ]
                                    },
                                    "/",
                                    {
                                        "fn":"coalesce",
                                        "args":[
                                            {"field":"front_page"},
                                            {"meta":"front_page"}
                                        ]
                                    }
                                ]
                            }
                        }
                    }
                ]
            }"#
            .to_string(),
        );

        assert!(crate::rules::compile::compile_rules(rules).is_ok());
    }

    #[test]
    fn validate_rules_rejects_invalid_function_next_url_config() {
        let rules = Value::String(
            r#"{
                "steps":[
                    {
                        "id":"parse",
                        "type":"node",
                        "parse":{
                            "next_url_config":{
                                "mode":"FUNCTION",
                                "fn":"replace",
                                "args":[{"meta":"period_date"},"-"]
                            }
                        }
                    }
                ]
            }"#
            .to_string(),
        );

        let error = crate::rules::compile::compile_rules(rules).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Rules(
                "step parse next_url_config.args must contain exactly 3 entries for replace"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_rules_accepts_step_validate_config() {
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
                            }
                        ],
                        "parse":{"fields":[]}
                    }
                ]
            }"#
            .to_string(),
        );

        assert!(crate::rules::compile::compile_rules(rules).is_ok());
    }

    #[test]
    fn validate_rules_rejects_invalid_step_validate_type() {
        let rules = Value::String(
            r#"{
                "steps":[
                    {
                        "id":"parse",
                        "validate":[
                            {
                                "name":"title",
                                "type":"date"
                            }
                        ],
                        "parse":{"fields":[]}
                    }
                ]
            }"#
            .to_string(),
        );

        let error = crate::rules::compile::compile_rules(rules).unwrap_err();

        assert_eq!(
            error,
            SpiderError::Rules("step parse validate.type has unsupported value: date".to_string())
        );
    }
}
