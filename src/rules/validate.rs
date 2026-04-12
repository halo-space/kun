use crate::error::SpiderError;
use crate::value::Value;
use std::collections::{BTreeMap, BTreeSet};

pub fn validate_rules(value: &Value) -> Result<(), SpiderError> {
    let root = expect_object(value, "rules")?;

    validate_spider(root.get("spider"))?;
    validate_engine(root.get("engine"))?;
    validate_sinks(root.get("sinks"))?;

    let steps = root
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| SpiderError::rules("rules.steps must be an array"))?;
    let seeds = root
        .get("seeds")
        .and_then(Value::as_array)
        .ok_or_else(|| SpiderError::rules("rules.seeds must be an array"))?;

    let step_ids = collect_step_ids(steps)?;
    validate_steps(steps, &step_ids, root.get("engine"), root.get("sinks"))?;
    validate_seeds(seeds, &step_ids, root.get("engine"))?;

    Ok(())
}

fn validate_spider(value: Option<&Value>) -> Result<(), SpiderError> {
    let spider = value
        .ok_or_else(|| SpiderError::rules("rules.spider is required"))
        .and_then(|value| expect_object(value, "rules.spider"))?;

    require_non_empty_string(spider.get("name"), "rules.spider.name")?;

    if let Some(clock) = spider.get("clock") {
        let clock = expect_object(clock, "rules.spider.clock")?;
        if let Some(timezone) = clock.get("timezone") {
            require_non_empty_string(Some(timezone), "rules.spider.clock.timezone")?;
        }
    }

    Ok(())
}

fn validate_engine(value: Option<&Value>) -> Result<(), SpiderError> {
    let Some(engine) = value else {
        return Ok(());
    };

    let engine = expect_object(engine, "rules.engine")?;
    for removed in ["schedule", "limits", "retry"] {
        if engine.contains_key(removed) {
            return Err(SpiderError::rules(format!(
                "rules.engine.{removed} has been removed; use the concrete middleware registry names"
            )));
        }
    }

    for key in [
        "dedup",
        "concurrency",
        "interval",
        "rate_limit",
        "auto_throttle",
        "retry_by_status",
        "retry_by_error",
    ] {
        if let Some(registry) = engine.get(key) {
            let registry = expect_object(registry, &format!("rules.engine.{key}"))?;
            for (name, config) in registry {
                if name.trim().is_empty() {
                    return Err(SpiderError::rules(format!(
                        "rules.engine.{key} keys must not be empty"
                    )));
                }
                expect_object(config, &format!("rules.engine.{key}.{name}"))?;
            }
        }
    }

    Ok(())
}

fn validate_sinks(value: Option<&Value>) -> Result<(), SpiderError> {
    let Some(sinks) = value else {
        return Ok(());
    };

    let sinks = expect_object(sinks, "rules.sinks")?;
    for (name, sink) in sinks {
        let sink = expect_object(sink, &format!("rules.sinks.{name}"))?;
        require_non_empty_string(sink.get("type"), &format!("rules.sinks.{name}.type"))?;
    }

    Ok(())
}

fn collect_step_ids(steps: &[Value]) -> Result<BTreeSet<String>, SpiderError> {
    let mut ids = BTreeSet::new();

    for step in steps {
        let step = expect_object(step, "rules.steps[*]")?;
        let id = required_string(step, "id", "rules.steps[*].id")?.to_string();
        if !ids.insert(id.clone()) {
            return Err(SpiderError::rules(format!("duplicate step id: {id}")));
        }
    }

    Ok(ids)
}

fn validate_steps(
    steps: &[Value],
    step_ids: &BTreeSet<String>,
    engine: Option<&Value>,
    sinks: Option<&Value>,
) -> Result<(), SpiderError> {
    for step in steps {
        let step = expect_object(step, "rules.steps[*]")?;
        reject_old_step_fields(step)?;
        let id = required_string(step, "id", "rules.steps[*].id")?;

        if let Some(callback) = step.get("callback") {
            require_non_empty_string(Some(callback), &format!("step {id} callback"))?;
        }

        if let Some(fields) = step.get("fields") {
            validate_fields(expect_object(fields, &format!("step {id} fields"))?, id)?;
        }

        if let Some(bind) = step.get("bind") {
            validate_value_map(
                expect_object(bind, &format!("step {id} bind"))?,
                &format!("step {id} bind"),
            )?;
        }

        let has_follow = step.get("follow").is_some();
        let has_output = step.get("output").is_some();
        let has_callback = step.get("callback").is_some();
        if !has_follow && !has_output && !has_callback {
            return Err(SpiderError::rules(format!(
                "step {id} must declare follow or output"
            )));
        }

        if let Some(follow) = step.get("follow") {
            validate_follow_list(
                expect_array(follow, &format!("step {id} follow"))?,
                id,
                step_ids,
                engine,
            )?;
        }

        if let Some(output) = step.get("output") {
            validate_output(
                expect_object(output, &format!("step {id} output"))?,
                id,
                sinks,
            )?;
        }
    }

    Ok(())
}

fn reject_old_step_fields(step: &BTreeMap<String, Value>) -> Result<(), SpiderError> {
    for key in [
        "type",
        "fetch",
        "parse",
        "validate",
        "route",
        "engine",
        "middlewares",
        "MIDDLEWARES",
        "meta",
        "dedup",
        "schedule",
        "retry",
    ] {
        if step.contains_key(key) {
            return Err(SpiderError::rules(format!(
                "step.{key} has been removed; use the v1 fields/bind/follow/output structure"
            )));
        }
    }

    Ok(())
}

fn validate_fields(fields: &BTreeMap<String, Value>, step_id: &str) -> Result<(), SpiderError> {
    for (name, field) in fields {
        let field = expect_object(field, &format!("step {step_id} fields.{name}"))?;
        require_non_empty_string(
            field.get("selector"),
            &format!("step {step_id} fields.{name}.selector"),
        )?;

        if field.contains_key("kind") {
            return Err(SpiderError::rules(format!(
                "step {step_id} fields.{name}.kind has been removed; use follow.item for node scope"
            )));
        }

        let mut projection_count = 0;
        projection_count += usize::from(field.get("text").and_then(Value::as_bool) == Some(true));
        projection_count += usize::from(field.get("html").and_then(Value::as_bool) == Some(true));
        projection_count += usize::from(field.contains_key("attr"));

        if projection_count > 1 {
            return Err(SpiderError::rules(format!(
                "step {step_id} fields.{name} may only declare one of text/html/attr"
            )));
        }

        if let Some(attr) = field.get("attr") {
            require_non_empty_string(Some(attr), &format!("step {step_id} fields.{name}.attr"))?;
        }
    }

    Ok(())
}

fn validate_follow_list(
    follow: &[Value],
    step_id: &str,
    step_ids: &BTreeSet<String>,
    engine_registry: Option<&Value>,
) -> Result<(), SpiderError> {
    for (index, entry) in follow.iter().enumerate() {
        let follow = expect_object(entry, &format!("step {step_id} follow[{index}]"))?;
        if let Some(item) = follow.get("item") {
            require_non_empty_string(Some(item), &format!("step {step_id} follow[{index}].item"))?;
        }
        let next_step = required_string(
            follow,
            "next_step",
            &format!("step {step_id} follow[{index}].next_step"),
        )?;
        if !step_ids.contains(next_step) {
            return Err(SpiderError::rules(format!(
                "step {step_id} follow[{index}] references unknown next_step: {next_step}"
            )));
        }

        let request = follow
            .get("request")
            .ok_or_else(|| {
                SpiderError::rules(format!(
                    "step {step_id} follow[{index}].request is required"
                ))
            })
            .and_then(|value| {
                expect_object(value, &format!("step {step_id} follow[{index}].request"))
            })?;
        validate_request(request, &format!("step {step_id} follow[{index}].request"))?;

        if let Some(meta) = follow.get("meta") {
            validate_value_map(
                expect_object(meta, &format!("step {step_id} follow[{index}].meta"))?,
                &format!("step {step_id} follow[{index}].meta"),
            )?;
        }

        if let Some(patterns) = follow.get("allow_url_pattern") {
            require_string_array(
                Some(patterns),
                &format!("step {step_id} follow[{index}].allow_url_pattern"),
            )?;
            validate_regex_array(
                patterns,
                &format!("step {step_id} follow[{index}].allow_url_pattern"),
            )?;
        }

        if let Some(engine) = follow.get("engine") {
            let refs = expect_object(engine, &format!("step {step_id} follow[{index}].engine"))?;
            validate_engine_refs(refs, &format!("step {step_id} follow[{index}].engine"))?;
            validate_engine_references_exist(
                refs,
                engine_registry,
                &format!("step {step_id} follow[{index}].engine"),
            )?;
        }
    }

    Ok(())
}

fn validate_output(
    output: &BTreeMap<String, Value>,
    step_id: &str,
    sinks: Option<&Value>,
) -> Result<(), SpiderError> {
    let item = output
        .get("item")
        .ok_or_else(|| SpiderError::rules(format!("step {step_id} output.item is required")))
        .and_then(|value| expect_object(value, &format!("step {step_id} output.item")))?;
    validate_value_map(item, &format!("step {step_id} output.item"))?;

    if let Some(validate) = output.get("validate") {
        validate_output_validator(
            expect_object(validate, &format!("step {step_id} output.validate"))?,
            step_id,
        )?;
    }

    let sink_value = output
        .get("sinks")
        .ok_or_else(|| SpiderError::rules(format!("step {step_id} output.sinks is required")))?;
    require_string_array(Some(sink_value), &format!("step {step_id} output.sinks"))?;
    let defined_sinks = sinks.and_then(Value::as_object).ok_or_else(|| {
        SpiderError::rules(format!(
            "step {step_id} output references sinks but rules.sinks is missing"
        ))
    })?;
    for sink in expect_array(sink_value, &format!("step {step_id} output.sinks"))? {
        let sink = sink.as_str().ok_or_else(|| {
            SpiderError::rules(format!(
                "step {step_id} output.sinks entries must be strings"
            ))
        })?;
        if !defined_sinks.contains_key(sink) {
            return Err(SpiderError::rules(format!(
                "step {step_id} output references unknown sink: {sink}"
            )));
        }
    }

    Ok(())
}

fn validate_output_validator(
    validate: &BTreeMap<String, Value>,
    step_id: &str,
) -> Result<(), SpiderError> {
    if let Some(required) = validate.get("required") {
        require_string_array(
            Some(required),
            &format!("step {step_id} output.validate.required"),
        )?;
    }

    if let Some(fields) = validate.get("fields") {
        let fields = expect_object(fields, &format!("step {step_id} output.validate.fields"))?;
        for (name, field) in fields {
            let field = expect_object(
                field,
                &format!("step {step_id} output.validate.fields.{name}"),
            )?;
            let value_type = required_string(
                field,
                "type",
                &format!("step {step_id} output.validate.fields.{name}.type"),
            )?;
            match value_type {
                "string" | "text" | "number" | "bool" | "list" | "object" => {}
                other => {
                    return Err(SpiderError::rules(format!(
                        "step {step_id} output.validate.fields.{name}.type has unsupported value: {other}"
                    )));
                }
            }
            if let Some(min_length) = field.get("min_length") {
                require_non_negative_integer(
                    min_length,
                    &format!("step {step_id} output.validate.fields.{name}.min_length"),
                )?;
            }
            if let Some(max_length) = field.get("max_length") {
                require_non_negative_integer(
                    max_length,
                    &format!("step {step_id} output.validate.fields.{name}.max_length"),
                )?;
            }
            if let Some(format) = field.get("format") {
                require_non_empty_string(
                    Some(format),
                    &format!("step {step_id} output.validate.fields.{name}.format"),
                )?;
                match format.as_str() {
                    Some("url" | "datetime" | "date") => {}
                    Some(other) => {
                        return Err(SpiderError::rules(format!(
                            "step {step_id} output.validate.fields.{name}.format has unsupported value: {other}"
                        )));
                    }
                    None => unreachable!("format was already validated as non-empty string"),
                }
            }
            if let Some(pattern) = field.get("pattern") {
                require_non_empty_string(
                    Some(pattern),
                    &format!("step {step_id} output.validate.fields.{name}.pattern"),
                )?;
                validate_regex(
                    pattern
                        .as_str()
                        .expect("pattern was already validated as non-empty string"),
                    &format!("step {step_id} output.validate.fields.{name}.pattern"),
                )?;
            }
            if let Some(enum_values) = field.get("enum") {
                expect_array(
                    enum_values,
                    &format!("step {step_id} output.validate.fields.{name}.enum"),
                )?;
            }
        }
    }

    if validate.get("rule").is_some() {
        return Err(SpiderError::rules(format!(
            "step {step_id} output.validate.rule has been removed"
        )));
    }

    Ok(())
}

fn validate_seeds(
    seeds: &[Value],
    step_ids: &BTreeSet<String>,
    engine: Option<&Value>,
) -> Result<(), SpiderError> {
    let mut ids = BTreeSet::new();

    for (index, seed) in seeds.iter().enumerate() {
        let seed = expect_object(seed, "rules.seeds[*]")?;
        let id = required_string(seed, "id", &format!("rules.seeds[{index}].id"))?.to_string();
        if !ids.insert(id.clone()) {
            return Err(SpiderError::rules(format!("duplicate seed id: {id}")));
        }

        let request = seed
            .get("request")
            .ok_or_else(|| SpiderError::rules(format!("seed {id} request is required")))
            .and_then(|value| expect_object(value, &format!("seed {id} request")))?;
        validate_request(request, &format!("seed {id} request"))?;

        if let Some(meta) = seed.get("meta") {
            validate_value_map(
                expect_object(meta, &format!("seed {id} meta"))?,
                &format!("seed {id} meta"),
            )?;
        }

        if let Some(patterns) = seed.get("allow_url_pattern") {
            require_string_array(Some(patterns), &format!("seed {id} allow_url_pattern"))?;
            validate_regex_array(patterns, &format!("seed {id} allow_url_pattern"))?;
        }

        if let Some(engine_refs) = seed.get("engine") {
            validate_engine_refs(
                expect_object(engine_refs, &format!("seed {id} engine"))?,
                &format!("seed {id} engine"),
            )?;
            validate_engine_references_exist(
                expect_object(engine_refs, &format!("seed {id} engine"))?,
                engine,
                &format!("seed {id} engine"),
            )?;
        }

        let next_step = required_string(seed, "next_step", &format!("seed {id} next_step"))?;
        if !step_ids.contains(next_step) {
            return Err(SpiderError::rules(format!(
                "seed {id} references unknown next_step: {next_step}"
            )));
        }
    }

    Ok(())
}

fn validate_request(request: &BTreeMap<String, Value>, label: &str) -> Result<(), SpiderError> {
    if let Some(mode) = request.get("mode") {
        let mode = mode
            .as_str()
            .ok_or_else(|| SpiderError::rules(format!("{label}.mode must be a string")))?;
        match mode {
            "http" | "browser" => {}
            other => {
                return Err(SpiderError::rules(format!(
                    "{label}.mode has unsupported value: {other}"
                )));
            }
        }
    }

    validate_scalar_value_expr(
        request
            .get("url")
            .ok_or_else(|| SpiderError::rules(format!("{label}.url is required")))?,
        &format!("{label}.url"),
        "string",
    )?;

    if let Some(method) = request.get("method") {
        require_non_empty_string(Some(method), &format!("{label}.method"))?;
    }

    for key in ["query", "headers", "cookies", "cb_kwargs"] {
        if let Some(map) = request.get(key) {
            validate_value_map(
                expect_object(map, &format!("{label}.{key}"))?,
                &format!("{label}.{key}"),
            )?;
        }
    }

    if let Some(timeout) = request.get("timeout") {
        validate_scalar_value_expr(timeout, &format!("{label}.timeout"), "number")?;
    }

    if let Some(proxy) = request.get("proxy").and_then(Value::as_object)
        && proxy.contains_key("ref")
    {
        return Err(SpiderError::rules(format!(
            "{label}.proxy.ref is unsupported; use a direct proxy url or a value expression like {label}.proxy.from"
        )));
    }

    if let Some(proxy) = request.get("proxy") {
        validate_scalar_value_expr(proxy, &format!("{label}.proxy"), "string")?;
    }

    for key in ["session", "encoding"] {
        if let Some(value) = request.get(key) {
            validate_scalar_value_expr(value, &format!("{label}.{key}"), "string")?;
        }
    }

    if let Some(priority) = request.get("priority") {
        validate_scalar_value_expr(priority, &format!("{label}.priority"), "number")?;
    }

    if let Some(errback) = request.get("errback") {
        require_non_empty_string(Some(errback), &format!("{label}.errback"))?;
    }

    if let Some(flags) = request.get("flags") {
        let flags = expect_array(flags, &format!("{label}.flags"))?;
        for (index, flag) in flags.iter().enumerate() {
            validate_value_expr(flag, &format!("{label}.flags[{index}]"))?;
        }
    }

    if let Some(body) = request.get("body") {
        let body = expect_object(body, &format!("{label}.body"))?;
        let mut body_keys = 0;
        body_keys += usize::from(body.contains_key("json"));
        body_keys += usize::from(body.contains_key("form"));
        body_keys += usize::from(body.contains_key("raw"));
        if body_keys != 1 {
            return Err(SpiderError::rules(format!(
                "{label}.body must declare exactly one of json/form/raw"
            )));
        }
        if let Some(json) = body.get("json") {
            validate_value_map(
                expect_object(json, &format!("{label}.body.json"))?,
                &format!("{label}.body.json"),
            )?;
        }
        if let Some(form) = body.get("form") {
            validate_value_map(
                expect_object(form, &format!("{label}.body.form"))?,
                &format!("{label}.body.form"),
            )?;
        }
        if let Some(raw) = body.get("raw") {
            validate_scalar_value_expr(raw, &format!("{label}.body.raw"), "string")?;
        }
    }

    if let Some(allow_redirects) = request.get("allow_redirects")
        && allow_redirects.as_bool().is_none()
    {
        return Err(SpiderError::rules(format!(
            "{label}.allow_redirects must be a boolean"
        )));
    }

    if request.contains_key("dont_filter") {
        return Err(SpiderError::rules(format!(
            "{label}.dont_filter has been removed; use {label}.skip: [\"dedup\"]"
        )));
    }

    if let Some(skip) = request.get("skip") {
        require_string_array(Some(skip), &format!("{label}.skip"))?;
    }

    Ok(())
}

fn validate_engine_refs(refs: &BTreeMap<String, Value>, label: &str) -> Result<(), SpiderError> {
    for key in refs.keys() {
        match key.as_str() {
            "dedup" | "concurrency" | "interval" | "rate_limit" | "auto_throttle"
            | "retry_by_status" | "retry_by_error" => {}
            other => {
                return Err(SpiderError::rules(format!(
                    "{label}.{other} is unsupported"
                )));
            }
        }
    }

    for (key, value) in refs {
        require_non_empty_string(Some(value), &format!("{label}.{key}"))?;
    }

    Ok(())
}

fn validate_engine_references_exist(
    refs: &BTreeMap<String, Value>,
    engine: Option<&Value>,
    label: &str,
) -> Result<(), SpiderError> {
    let engine = engine
        .ok_or_else(|| {
            SpiderError::rules(format!(
                "{label} references engine rules but rules.engine is missing"
            ))
        })
        .and_then(|value| expect_object(value, "rules.engine"))?;

    for (key, value) in refs {
        let Some(name) = value.as_str() else {
            continue;
        };
        let registry = engine
            .get(key)
            .and_then(Value::as_object)
            .ok_or_else(|| SpiderError::rules(format!("rules.engine.{key} is missing")))?;
        if !registry.contains_key(name) {
            return Err(SpiderError::rules(format!(
                "{label}.{key} references unknown engine rule: {name}"
            )));
        }
    }

    Ok(())
}

fn validate_value_map(map: &BTreeMap<String, Value>, label: &str) -> Result<(), SpiderError> {
    for (key, value) in map {
        validate_value_expr(value, &format!("{label}.{key}"))?;
    }

    Ok(())
}

fn validate_scalar_value_expr(
    value: &Value,
    label: &str,
    expected_literal: &str,
) -> Result<(), SpiderError> {
    validate_value_expr(value, label)?;

    if let Some(object) = value.as_object()
        && !is_value_expr_object(object)
    {
        return Err(SpiderError::rules(format!(
            "{label} must be a {expected_literal} or a value expression object"
        )));
    }

    Ok(())
}

fn validate_value_expr(value: &Value, label: &str) -> Result<(), SpiderError> {
    if let Some(values) = value.as_array() {
        reject_nested_value_expr_in_literal_array(values, label)?;
        return Ok(());
    }

    let Some(object) = value.as_object() else {
        return Ok(());
    };

    let source_keys = ["from", "template", "selector"];
    let has_expr_keys = is_value_expr_object(object);

    if !has_expr_keys {
        reject_nested_value_expr_in_literal_object(object, label)?;
        return Ok(());
    }

    if object.contains_key("vars") && !object.contains_key("template") {
        return Err(SpiderError::rules(format!(
            "{label}.vars requires {label}.template"
        )));
    }

    let has_selector_projection =
        object.contains_key("text") || object.contains_key("html") || object.contains_key("attr");
    if has_selector_projection && !object.contains_key("selector") {
        return Err(SpiderError::rules(format!(
            "{label}.text/html/attr requires {label}.selector"
        )));
    }

    let source_count = source_keys
        .iter()
        .filter(|key| object.contains_key(**key))
        .count();
    if source_count > 1 {
        return Err(SpiderError::rules(format!(
            "{label} may only declare one of from/template/selector"
        )));
    }

    if let Some(from) = object.get("from") {
        require_non_empty_string(Some(from), &format!("{label}.from"))?;
    }

    if let Some(template) = object.get("template") {
        require_non_empty_string(Some(template), &format!("{label}.template"))?;
    }

    if let Some(vars) = object.get("vars") {
        validate_value_map(
            expect_object(vars, &format!("{label}.vars"))?,
            &format!("{label}.vars"),
        )?;
    }

    if let Some(selector) = object.get("selector") {
        require_non_empty_string(Some(selector), &format!("{label}.selector"))?;
        let mut projection_count = 0;
        projection_count += usize::from(object.get("text").and_then(Value::as_bool) == Some(true));
        projection_count += usize::from(object.get("html").and_then(Value::as_bool) == Some(true));
        projection_count += usize::from(object.contains_key("attr"));
        if projection_count > 1 {
            return Err(SpiderError::rules(format!(
                "{label} selector extraction may only declare one of text/html/attr"
            )));
        }
        if let Some(attr) = object.get("attr") {
            require_non_empty_string(Some(attr), &format!("{label}.attr"))?;
        }
    }

    if let Some(transforms) = object.get("transforms") {
        let transforms = expect_array(transforms, &format!("{label}.transforms"))?;
        validate_transforms(transforms, label)?;
    }

    if let Some(fallback) = object.get("fallback") {
        validate_value_expr(fallback, &format!("{label}.fallback"))?;
    }

    Ok(())
}

fn is_value_expr_object(object: &BTreeMap<String, Value>) -> bool {
    object.contains_key("from")
        || object.contains_key("template")
        || object.contains_key("selector")
        || object.contains_key("transforms")
        || object.contains_key("fallback")
}

fn reject_nested_value_expr_in_literal_object(
    object: &BTreeMap<String, Value>,
    label: &str,
) -> Result<(), SpiderError> {
    for (key, value) in object {
        reject_nested_value_expr_in_literal_value(value, &format!("{label}.{key}"))?;
    }
    Ok(())
}

fn reject_nested_value_expr_in_literal_array(
    values: &[Value],
    label: &str,
) -> Result<(), SpiderError> {
    for (index, value) in values.iter().enumerate() {
        reject_nested_value_expr_in_literal_value(value, &format!("{label}[{index}]"))?;
    }
    Ok(())
}

fn reject_nested_value_expr_in_literal_value(
    value: &Value,
    label: &str,
) -> Result<(), SpiderError> {
    match value {
        Value::Object(object) => {
            if is_value_expr_object(object) {
                return Err(SpiderError::rules(format!(
                    "{label} uses a nested value expression inside a literal object/array, which is unsupported in v1"
                )));
            }

            reject_nested_value_expr_in_literal_object(object, label)
        }
        Value::Array(values) => reject_nested_value_expr_in_literal_array(values, label),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

fn expect_object<'a>(
    value: &'a Value,
    label: &str,
) -> Result<&'a BTreeMap<String, Value>, SpiderError> {
    value
        .as_object()
        .ok_or_else(|| SpiderError::rules(format!("{label} must be an object")))
}

fn expect_array<'a>(value: &'a Value, label: &str) -> Result<&'a [Value], SpiderError> {
    value
        .as_array()
        .ok_or_else(|| SpiderError::rules(format!("{label} must be an array")))
}

fn required_string<'a>(
    value: &'a BTreeMap<String, Value>,
    key: &str,
    label: &str,
) -> Result<&'a str, SpiderError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SpiderError::rules(format!("{label} is required")))
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

fn validate_regex_array(value: &Value, label: &str) -> Result<(), SpiderError> {
    for pattern in expect_array(value, label)? {
        let pattern = pattern
            .as_str()
            .ok_or_else(|| SpiderError::rules(format!("{label} entries must be strings")))?;
        validate_regex(pattern, label)?;
    }

    Ok(())
}

fn validate_regex(pattern: &str, label: &str) -> Result<(), SpiderError> {
    regex::Regex::new(pattern).map(|_| ()).map_err(|error| {
        SpiderError::rules(format!(
            "{label} contains invalid regex {pattern:?}: {error}"
        ))
    })
}

fn validate_transforms(transforms: &[Value], label: &str) -> Result<(), SpiderError> {
    for (index, transform) in transforms.iter().enumerate() {
        let transform = expect_object(transform, &format!("{label}.transforms[{index}]"))?;
        let transform_type = required_string(
            transform,
            "type",
            &format!("{label}.transforms[{index}].type"),
        )?;

        match transform_type {
            "trim" | "resolve_url" => {}
            "replace" => {
                require_non_empty_string(
                    transform.get("from"),
                    &format!("{label}.transforms[{index}].from"),
                )?;
                if let Some(to) = transform.get("to")
                    && to.as_str().is_none()
                {
                    return Err(SpiderError::rules(format!(
                        "{label}.transforms[{index}].to must be a string"
                    )));
                }
            }
            "regex" => {
                let pattern = transform
                    .get("pattern")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| {
                        transform
                            .get("expr")
                            .and_then(Value::as_str)
                            .filter(|value| !value.trim().is_empty())
                    })
                    .ok_or_else(|| {
                        SpiderError::rules(format!(
                            "{label}.transforms[{index}] regex requires pattern or expr"
                        ))
                    })?;
                validate_regex(pattern, &format!("{label}.transforms[{index}]"))?;
                if let Some(group) = transform.get("group") {
                    require_non_negative_integer(
                        group,
                        &format!("{label}.transforms[{index}].group"),
                    )?;
                }
                if let Some(replace) = transform.get("replace")
                    && replace.as_str().is_none()
                {
                    return Err(SpiderError::rules(format!(
                        "{label}.transforms[{index}].replace must be a string"
                    )));
                }
            }
            "split" | "join" => {
                option_non_empty_string_any(
                    transform,
                    &["delimiter", "sep"],
                    &format!("{label}.transforms[{index}]"),
                )?;
            }
            "pick" => {
                if let Some(index_value) = transform.get("index") {
                    require_non_negative_integer(
                        index_value,
                        &format!("{label}.transforms[{index}].index"),
                    )?;
                }
            }
            "date_format" => {
                require_non_empty_string(
                    transform.get("format"),
                    &format!("{label}.transforms[{index}].format"),
                )?;
                if let Some(input_format) = transform.get("input_format")
                    && input_format.as_str().is_none()
                {
                    return Err(SpiderError::rules(format!(
                        "{label}.transforms[{index}].input_format must be a string"
                    )));
                }
            }
            other => {
                return Err(SpiderError::rules(format!(
                    "{label}.transforms[{index}].type has unsupported value: {other}"
                )));
            }
        }
    }

    Ok(())
}

fn option_non_empty_string_any(
    object: &BTreeMap<String, Value>,
    keys: &[&str],
    label: &str,
) -> Result<(), SpiderError> {
    if keys.iter().any(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        return Ok(());
    }

    Err(SpiderError::rules(format!(
        "{label} requires one of: {}",
        keys.join(", ")
    )))
}

fn require_non_negative_integer(value: &Value, label: &str) -> Result<(), SpiderError> {
    let Some(number) = value.as_f64() else {
        return Err(SpiderError::rules(format!(
            "{label} must be a non-negative integer"
        )));
    };

    if number < 0.0 || number.fract() != 0.0 {
        return Err(SpiderError::rules(format!(
            "{label} must be a non-negative integer"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_rules() -> Value {
        Value::from(json!({
            "spider": {
                "name": "demo"
            },
            "engine": {
                "concurrency": {
                    "detail_serial": {
                        "bucket": "origin",
                        "concurrency": 1
                    }
                },
                "dedup": {
                    "detail_dedup": {
                        "backend": "memory",
                        "key": ["url"]
                    }
                }
            },
            "sinks": {
                "file": {
                    "type": "file",
                    "path": "./out.jsonl"
                }
            },
            "seeds": [
                {
                    "id": "start",
                    "request": {
                        "url": "https://example.com/start"
                    },
                    "next_step": "list"
                }
            ],
            "steps": [
                {
                    "id": "list",
                    "fields": {
                        "title": {
                            "selector": ".title",
                            "text": true
                        }
                    },
                    "bind": {
                        "title_trimmed": {
                            "from": "$fields.title",
                            "transforms": [{ "type": "trim" }]
                        }
                    },
                    "follow": [
                        {
                            "item": ".row",
                            "next_step": "detail",
                            "request": {
                                "url": {
                                    "selector": "a",
                                    "attr": "href"
                                }
                            },
                            "engine": {
                                "concurrency": "detail_serial",
                                "dedup": "detail_dedup"
                            }
                        }
                    ]
                },
                {
                    "id": "detail",
                    "output": {
                        "item": {
                            "title": {
                                "from": "$fields.title"
                            }
                        },
                        "sinks": ["file"]
                    }
                }
            ]
        }))
    }

    #[test]
    fn validate_accepts_v1_rules_shape() {
        assert!(validate_rules(&valid_rules()).is_ok());
    }

    #[test]
    fn validate_rejects_old_step_parse_shape() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{ "id": "start", "request": { "url": "https://example.com" }, "next_step": "parse" }],
            "steps": [
                {
                    "id": "parse",
                    "parse": {}
                }
            ]
        }));

        let error = validate_rules(&rules).expect_err("old parse shape should fail");
        assert!(error.to_string().contains("step.parse has been removed"));
    }

    #[test]
    fn validate_rejects_removed_fields_kind_node() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{ "id": "start", "request": { "url": "https://example.com" }, "next_step": "parse" }],
            "steps": [
                {
                    "id": "parse",
                    "fields": {
                        "node": {
                            "selector": "//div",
                            "kind": "node"
                        }
                    },
                    "output": {
                        "item": { "title": "ok" },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("fields.kind should fail");
        assert!(
            error
                .to_string()
                .contains("fields.node.kind has been removed")
        );
    }

    #[test]
    fn validate_rejects_removed_output_validate_rule() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{ "id": "start", "request": { "url": "https://example.com" }, "next_step": "parse" }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": { "title": "ok" },
                        "validate": {
                            "rule": "article_guard"
                        },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("output.validate.rule should fail");
        assert!(
            error
                .to_string()
                .contains("output.validate.rule has been removed")
        );
    }

    #[test]
    fn validate_rejects_removed_request_dont_filter() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com",
                    "dont_filter": true
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": { "title": "ok" },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("request.dont_filter should fail");
        assert!(
            error
                .to_string()
                .contains("request.dont_filter has been removed")
        );
    }

    #[test]
    fn validate_rejects_unsupported_request_proxy_ref() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com",
                    "proxy": {
                        "ref": "default_proxy"
                    }
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": { "title": "ok" },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("request.proxy.ref should fail");
        assert!(
            error
                .to_string()
                .contains("request.proxy.ref is unsupported")
        );
    }

    #[test]
    fn validate_rejects_request_url_literal_object_shape() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": {
                        "value": "https://example.com"
                    }
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": { "title": "ok" },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("request.url literal object should fail");
        assert!(
            error
                .to_string()
                .contains("request.url must be a string or a value expression object")
        );
    }

    #[test]
    fn validate_rejects_request_timeout_literal_object_shape() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com",
                    "timeout": {
                        "millis": 1000
                    }
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": { "title": "ok" },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("request.timeout literal object should fail");
        assert!(
            error
                .to_string()
                .contains("request.timeout must be a number or a value expression object")
        );
    }

    #[test]
    fn validate_rejects_invalid_seed_allow_url_pattern_regex() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com"
                },
                "allow_url_pattern": ["("],
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": { "title": "ok" },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("invalid allow_url_pattern should fail");
        assert!(
            error
                .to_string()
                .contains("seed start allow_url_pattern contains invalid regex")
        );
    }

    #[test]
    fn validate_rejects_invalid_output_validate_pattern_regex() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com"
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": { "title": "ok" },
                        "validate": {
                            "fields": {
                                "title": {
                                    "type": "string",
                                    "pattern": "("
                                }
                            }
                        },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("invalid output pattern regex should fail");
        assert!(
            error
                .to_string()
                .contains("output.validate.fields.title.pattern contains invalid regex")
        );
    }

    #[test]
    fn validate_rejects_fractional_output_validate_min_length() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com"
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": { "title": "ok" },
                        "validate": {
                            "fields": {
                                "title": {
                                    "type": "string",
                                    "min_length": 1.5
                                }
                            }
                        },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("fractional min_length should fail");
        assert!(
            error
                .to_string()
                .contains("output.validate.fields.title.min_length must be a non-negative integer")
        );
    }

    #[test]
    fn validate_rejects_unsupported_output_validate_format() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com"
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": { "title": "ok" },
                        "validate": {
                            "fields": {
                                "title": {
                                    "type": "string",
                                    "format": "email"
                                }
                            }
                        },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("unsupported output format should fail");
        assert!(
            error
                .to_string()
                .contains("output.validate.fields.title.format has unsupported value: email")
        );
    }

    #[test]
    fn validate_rejects_nested_output_item_value_expression() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com"
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": {
                            "author": {
                                "name": {
                                    "from": "$fields.author_name"
                                }
                            }
                        },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules)
            .expect_err("nested value expression inside literal object should fail");
        assert!(
            error
                .to_string()
                .contains("uses a nested value expression inside a literal object/array")
        );
    }

    #[test]
    fn validate_rejects_value_expr_vars_without_template() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": {
                        "from": "$meta.detail_url",
                        "vars": {
                            "id": 1
                        }
                    }
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "output": {
                        "item": { "title": "ok" },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("vars without template should fail");
        assert!(error.to_string().contains("request.url.vars requires"));
    }

    #[test]
    fn validate_rejects_unknown_transform_type() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com"
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "bind": {
                        "title_trimmed": {
                            "from": "$fields.title",
                            "transforms": [{ "type": "unknown_transform" }]
                        }
                    },
                    "output": {
                        "item": { "title": "ok" },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error = validate_rules(&rules).expect_err("unknown transform should fail");
        assert!(
            error
                .to_string()
                .contains("bind.title_trimmed.transforms[0].type has unsupported value")
        );
    }

    #[test]
    fn validate_rejects_regex_transform_without_pattern() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com"
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "bind": {
                        "slug": {
                            "from": "$fields.title",
                            "transforms": [{ "type": "regex" }]
                        }
                    },
                    "output": {
                        "item": { "title": "ok" },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error =
            validate_rules(&rules).expect_err("regex transform without pattern should fail");
        assert!(
            error
                .to_string()
                .contains("bind.slug.transforms[0] regex requires pattern or expr")
        );
    }

    #[test]
    fn validate_rejects_pick_transform_non_integer_index() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "seeds": [{
                "id": "start",
                "request": {
                    "url": "https://example.com"
                },
                "next_step": "parse"
            }],
            "steps": [
                {
                    "id": "parse",
                    "bind": {
                        "first_tag": {
                            "from": "$fields.tags",
                            "transforms": [{ "type": "pick", "index": 1.5 }]
                        }
                    },
                    "output": {
                        "item": { "title": "ok" },
                        "sinks": ["file"]
                    }
                }
            ],
            "sinks": {
                "file": { "type": "file" }
            }
        }));

        let error =
            validate_rules(&rules).expect_err("pick transform non-integer index should fail");
        assert!(
            error
                .to_string()
                .contains("bind.first_tag.transforms[0].index must be a non-negative integer")
        );
    }

    #[test]
    fn validate_rejects_unknown_engine_reference() {
        let rules = Value::from(json!({
            "spider": { "name": "demo" },
            "engine": {
                "concurrency": {
                    "detail_serial": { "bucket": "origin", "concurrency": 1 }
                }
            },
            "seeds": [{ "id": "start", "request": { "url": "https://example.com" }, "next_step": "parse" }],
            "steps": [
                {
                    "id": "parse",
                    "follow": [
                        {
                            "next_step": "parse",
                            "request": { "url": "https://example.com/detail" },
                            "engine": {
                                "dedup": "missing_rule"
                            }
                        }
                    ]
                }
            ]
        }));

        let error = validate_rules(&rules).expect_err("unknown engine reference should fail");
        assert!(matches!(error, SpiderError::Rules(_)));
    }
}
