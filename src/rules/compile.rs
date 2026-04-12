use crate::error::SpiderError;
use crate::middleware::{
    AUTO_THROTTLE, CONCURRENCY, DEDUP, INTERVAL, Map as MiddlewareMap, RATE_LIMIT, RETRY_BY_ERROR,
    RETRY_BY_STATUS, Stage,
};
use crate::request::RequestMode;
use crate::rules::schema::{
    BodyConfig, ClockConfig, Compiled, CompiledSeed, CompiledStep, Dsl, EngineRefs,
    EngineRegistryConfig, ExtractKind, FieldConfig, FieldPlan, FollowConfig, FollowPlan,
    OutputConfig, OutputFieldValidatorConfig, OutputPlan, OutputValidatorConfig, RequestConfig,
    RequestPlan, SeedConfig, SelectorKind, SelectorValueKind, SinkConfig, SpiderConfig, StepConfig,
    TransformConfig, ValueExpr, ValueSource,
};
use crate::rules::validate::validate_rules;
use crate::validator::{FieldValidator, Transform, Type, field as validator_field};
use crate::value::Value;
use std::collections::BTreeMap;

pub fn compile_rules(value: Value) -> Result<Compiled, SpiderError> {
    let normalized = normalize(value)?;
    validate_rules(&normalized)?;
    let dsl = parse_dsl(&normalized)?;
    compile_dsl(dsl)
}

fn normalize(value: Value) -> Result<Value, SpiderError> {
    match value {
        Value::String(content) => serde_json::from_str::<serde_json::Value>(&content)
            .map(Value::from)
            .map_err(|error| SpiderError::rules(format!("invalid rules json: {error}"))),
        other => Ok(other),
    }
}

fn compile_dsl(dsl: Dsl) -> Result<Compiled, SpiderError> {
    let steps = dsl
        .steps
        .into_iter()
        .map(|step| compile_step(step, &dsl.engine, &dsl.sinks))
        .collect::<Result<Vec<_>, _>>()?;
    let seeds = dsl
        .seeds
        .into_iter()
        .map(|seed| compile_seed(seed, &dsl.engine))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Compiled {
        spider: dsl.spider,
        engine: dsl.engine,
        sinks: dsl.sinks,
        seeds,
        steps,
    })
}

fn parse_dsl(value: &Value) -> Result<Dsl, SpiderError> {
    let root = expect_object(value, "rules")?;

    Ok(Dsl {
        spider: parse_spider(root.get("spider"))?,
        engine: parse_engine(root.get("engine"))?,
        sinks: parse_sinks(root.get("sinks"))?,
        seeds: root
            .get("seeds")
            .and_then(Value::as_array)
            .ok_or_else(|| SpiderError::rules("rules.seeds must be an array"))?
            .iter()
            .map(parse_seed)
            .collect::<Result<Vec<_>, _>>()?,
        steps: root
            .get("steps")
            .and_then(Value::as_array)
            .ok_or_else(|| SpiderError::rules("rules.steps must be an array"))?
            .iter()
            .map(parse_step)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn parse_spider(value: Option<&Value>) -> Result<SpiderConfig, SpiderError> {
    let spider = value
        .ok_or_else(|| SpiderError::rules("rules.spider is required"))
        .and_then(|value| expect_object(value, "rules.spider"))?;

    Ok(SpiderConfig {
        name: required_string(spider, "name", "rules.spider.name")?.to_string(),
        clock: parse_clock(spider.get("clock"))?,
    })
}

fn parse_clock(value: Option<&Value>) -> Result<ClockConfig, SpiderError> {
    let Some(clock) = value else {
        return Ok(ClockConfig::default());
    };
    let clock = expect_object(clock, "rules.spider.clock")?;

    Ok(ClockConfig {
        timezone: optional_string(clock, "timezone").map(str::to_string),
    })
}

fn parse_engine(value: Option<&Value>) -> Result<EngineRegistryConfig, SpiderError> {
    let Some(engine) = value else {
        return Ok(EngineRegistryConfig::default());
    };
    let engine = expect_object(engine, "rules.engine")?;

    Ok(EngineRegistryConfig {
        dedup: parse_named_registry(engine.get("dedup"), "rules.engine.dedup")?,
        concurrency: parse_named_registry(engine.get("concurrency"), "rules.engine.concurrency")?,
        interval: parse_named_registry(engine.get("interval"), "rules.engine.interval")?,
        rate_limit: parse_named_registry(engine.get("rate_limit"), "rules.engine.rate_limit")?,
        auto_throttle: parse_named_registry(
            engine.get("auto_throttle"),
            "rules.engine.auto_throttle",
        )?,
        retry_by_status: parse_named_registry(
            engine.get("retry_by_status"),
            "rules.engine.retry_by_status",
        )?,
        retry_by_error: parse_named_registry(
            engine.get("retry_by_error"),
            "rules.engine.retry_by_error",
        )?,
    })
}

fn parse_named_registry(
    value: Option<&Value>,
    label: &str,
) -> Result<BTreeMap<String, BTreeMap<String, Value>>, SpiderError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let registry = expect_object(value, label)?;
    let mut parsed = BTreeMap::new();

    for (name, config) in registry {
        parsed.insert(
            name.clone(),
            expect_object(config, &format!("{label}.{name}"))?.clone(),
        );
    }

    Ok(parsed)
}

fn parse_sinks(value: Option<&Value>) -> Result<BTreeMap<String, SinkConfig>, SpiderError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let sinks = expect_object(value, "rules.sinks")?;
    let mut parsed = BTreeMap::new();

    for (name, sink) in sinks {
        let sink = expect_object(sink, &format!("rules.sinks.{name}"))?;
        let mut options = sink.clone();
        let kind = required_string(sink, "type", &format!("rules.sinks.{name}.type"))?.to_string();
        options.remove("type");
        parsed.insert(name.clone(), SinkConfig { kind, options });
    }

    Ok(parsed)
}

fn parse_seed(value: &Value) -> Result<SeedConfig, SpiderError> {
    let seed = expect_object(value, "rules.seeds[*]")?;

    Ok(SeedConfig {
        id: required_string(seed, "id", "rules.seeds[*].id")?.to_string(),
        request: parse_request(
            seed.get("request")
                .ok_or_else(|| SpiderError::rules("seed request is required"))?,
            "seed request",
        )?,
        meta: parse_value_map(seed.get("meta"), "seed meta")?,
        allow_url_pattern: string_list(seed.get("allow_url_pattern"), "seed allow_url_pattern")?,
        engine: parse_engine_refs(seed.get("engine"))?,
        next_step: required_string(seed, "next_step", "seed next_step")?.to_string(),
    })
}

fn parse_step(value: &Value) -> Result<StepConfig, SpiderError> {
    let step = expect_object(value, "rules.steps[*]")?;

    Ok(StepConfig {
        id: required_string(step, "id", "rules.steps[*].id")?.to_string(),
        callback: optional_string(step, "callback").map(str::to_string),
        fields: parse_fields(step.get("fields"))?,
        bind: parse_value_map(step.get("bind"), "step bind")?,
        follow: parse_follow_list(step.get("follow"))?,
        output: step.get("output").map(parse_output).transpose()?,
    })
}

fn parse_fields(value: Option<&Value>) -> Result<BTreeMap<String, FieldConfig>, SpiderError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let fields = expect_object(value, "step fields")?;
    let mut parsed = BTreeMap::new();

    for (name, field) in fields {
        let field = expect_object(field, &format!("step fields.{name}"))?;
        parsed.insert(
            name.clone(),
            FieldConfig {
                selector: required_string(
                    field,
                    "selector",
                    &format!("step fields.{name}.selector"),
                )?
                .to_string(),
                kind: extract_kind_from_object(field),
            },
        );
    }

    Ok(parsed)
}

fn parse_follow_list(value: Option<&Value>) -> Result<Vec<FollowConfig>, SpiderError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let list = expect_array(value, "step follow")?;
    list.iter().map(parse_follow).collect()
}

fn parse_follow(value: &Value) -> Result<FollowConfig, SpiderError> {
    let follow = expect_object(value, "step follow[*]")?;

    Ok(FollowConfig {
        item: optional_string(follow, "item").map(str::to_string),
        next_step: required_string(follow, "next_step", "follow next_step")?.to_string(),
        request: parse_request(
            follow
                .get("request")
                .ok_or_else(|| SpiderError::rules("follow request is required"))?,
            "follow request",
        )?,
        meta: parse_value_map(follow.get("meta"), "follow meta")?,
        allow_url_pattern: string_list(
            follow.get("allow_url_pattern"),
            "follow allow_url_pattern",
        )?,
        engine: parse_engine_refs(follow.get("engine"))?,
    })
}

fn parse_output(value: &Value) -> Result<OutputConfig, SpiderError> {
    let output = expect_object(value, "step output")?;

    Ok(OutputConfig {
        item: parse_value_map(output.get("item"), "step output.item")?,
        validator: output
            .get("validate")
            .map(parse_output_validator)
            .transpose()?,
        sinks: string_list(output.get("sinks"), "step output.sinks")?,
    })
}

fn parse_output_validator(value: &Value) -> Result<OutputValidatorConfig, SpiderError> {
    let validate = expect_object(value, "step output.validate")?;
    let mut fields = BTreeMap::new();

    if let Some(value) = validate.get("fields") {
        let configured = expect_object(value, "step output.validate.fields")?;
        for (name, config) in configured {
            let config = expect_object(config, &format!("step output.validate.fields.{name}"))?;
            fields.insert(
                name.clone(),
                OutputFieldValidatorConfig {
                    value_type: required_string(
                        config,
                        "type",
                        &format!("step output.validate.fields.{name}.type"),
                    )?
                    .to_string(),
                    min_length: config
                        .get("min_length")
                        .and_then(Value::as_f64)
                        .map(|value| value as usize),
                    max_length: config
                        .get("max_length")
                        .and_then(Value::as_f64)
                        .map(|value| value as usize),
                    format: optional_string(config, "format").map(str::to_string),
                    pattern: optional_string(config, "pattern").map(str::to_string),
                    enum_values: config
                        .get("enum")
                        .and_then(Value::as_array)
                        .map(|values| values.to_vec())
                        .unwrap_or_default(),
                },
            );
        }
    }

    Ok(OutputValidatorConfig {
        required: string_list(validate.get("required"), "step output.validate.required")?,
        fields,
    })
}

fn parse_request(value: &Value, label: &str) -> Result<RequestConfig, SpiderError> {
    let request = expect_object(value, label)?;

    Ok(RequestConfig {
        mode: optional_string(request, "mode").map(str::to_string),
        method: optional_string(request, "method").map(str::to_string),
        url: parse_value_expr(
            request
                .get("url")
                .ok_or_else(|| SpiderError::rules(format!("{label}.url is required")))?,
        )?,
        query: parse_value_map(request.get("query"), &format!("{label}.query"))?,
        headers: parse_value_map(request.get("headers"), &format!("{label}.headers"))?,
        cookies: parse_value_map(request.get("cookies"), &format!("{label}.cookies"))?,
        timeout: request.get("timeout").map(parse_value_expr).transpose()?,
        proxy: request.get("proxy").map(parse_value_expr).transpose()?,
        session: request.get("session").map(parse_value_expr).transpose()?,
        encoding: request.get("encoding").map(parse_value_expr).transpose()?,
        priority: request.get("priority").map(parse_value_expr).transpose()?,
        flags: request
            .get("flags")
            .and_then(Value::as_array)
            .map(|values| values.iter().map(parse_value_expr).collect())
            .transpose()?
            .unwrap_or_default(),
        cb_kwargs: parse_value_map(request.get("cb_kwargs"), &format!("{label}.cb_kwargs"))?,
        errback: optional_string(request, "errback").map(str::to_string),
        body: request.get("body").map(parse_body).transpose()?,
        allow_redirects: request.get("allow_redirects").and_then(Value::as_bool),
        skip: string_list(request.get("skip"), &format!("{label}.skip"))?,
    })
}

fn parse_body(value: &Value) -> Result<BodyConfig, SpiderError> {
    let body = expect_object(value, "request body")?;

    if let Some(json) = body.get("json") {
        return Ok(BodyConfig::Json(parse_value_map(
            Some(json),
            "request body.json",
        )?));
    }

    if let Some(form) = body.get("form") {
        return Ok(BodyConfig::Form(parse_value_map(
            Some(form),
            "request body.form",
        )?));
    }

    if let Some(raw) = body.get("raw") {
        return Ok(BodyConfig::Raw(parse_value_expr(raw)?));
    }

    Err(SpiderError::rules(
        "request body must declare exactly one of json/form/raw".to_string(),
    ))
}

fn parse_engine_refs(value: Option<&Value>) -> Result<EngineRefs, SpiderError> {
    let Some(value) = value else {
        return Ok(EngineRefs::default());
    };
    let refs = expect_object(value, "engine refs")?;

    Ok(EngineRefs {
        dedup: optional_string(refs, "dedup").map(str::to_string),
        concurrency: optional_string(refs, "concurrency").map(str::to_string),
        interval: optional_string(refs, "interval").map(str::to_string),
        rate_limit: optional_string(refs, "rate_limit").map(str::to_string),
        auto_throttle: optional_string(refs, "auto_throttle").map(str::to_string),
        retry_by_status: optional_string(refs, "retry_by_status").map(str::to_string),
        retry_by_error: optional_string(refs, "retry_by_error").map(str::to_string),
    })
}

fn parse_value_map(
    value: Option<&Value>,
    label: &str,
) -> Result<BTreeMap<String, ValueExpr>, SpiderError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let map = expect_object(value, label)?;
    let mut parsed = BTreeMap::new();

    for (key, value) in map {
        parsed.insert(key.clone(), parse_value_expr(value)?);
    }

    Ok(parsed)
}

fn parse_value_expr(value: &Value) -> Result<ValueExpr, SpiderError> {
    let Some(object) = value.as_object() else {
        return Ok(ValueExpr::literal(value.clone()));
    };

    let mut expr = if let Some(from) = object.get("from") {
        ValueExpr::from_ref(
            from.as_str()
                .ok_or_else(|| SpiderError::rules("value.from must be a string"))?,
        )
    } else if let Some(template) = object.get("template") {
        ValueExpr {
            source: ValueSource::Template {
                template: template
                    .as_str()
                    .ok_or_else(|| SpiderError::rules("value.template must be a string"))?
                    .to_string(),
                vars: parse_value_map(object.get("vars"), "value.vars")?,
            },
            transforms: Vec::new(),
            fallback: None,
        }
    } else if let Some(selector) = object.get("selector") {
        ValueExpr {
            source: ValueSource::Selector {
                selector: selector
                    .as_str()
                    .ok_or_else(|| SpiderError::rules("value.selector must be a string"))?
                    .to_string(),
                kind: selector_value_kind_from_object(object)?,
            },
            transforms: Vec::new(),
            fallback: None,
        }
    } else {
        ValueExpr::literal(value.clone())
    };

    expr.transforms = parse_transforms(object.get("transforms"))?;
    expr.fallback = object
        .get("fallback")
        .map(parse_value_expr)
        .transpose()?
        .map(Box::new);

    Ok(expr)
}

fn parse_transforms(value: Option<&Value>) -> Result<Vec<TransformConfig>, SpiderError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let transforms = expect_array(value, "value.transforms")?;
    transforms
        .iter()
        .map(|value| {
            let object = expect_object(value, "value.transforms[*]")?;
            let mut options = object.clone();
            let kind = required_string(object, "type", "value.transforms[*].type")?.to_string();
            options.remove("type");
            Ok(TransformConfig { kind, options })
        })
        .collect()
}

fn compile_seed(
    seed: SeedConfig,
    registry: &EngineRegistryConfig,
) -> Result<CompiledSeed, SpiderError> {
    Ok(CompiledSeed {
        id: seed.id,
        request: compile_request_plan(seed.request)?,
        meta: seed.meta,
        allow_url_pattern: seed.allow_url_pattern,
        middleware: lower_engine_refs(&seed.engine, registry)?,
        next_step: seed.next_step,
    })
}

fn compile_step(
    step: StepConfig,
    registry: &EngineRegistryConfig,
    sinks: &BTreeMap<String, SinkConfig>,
) -> Result<CompiledStep, SpiderError> {
    Ok(CompiledStep {
        id: step.id,
        callback: step.callback,
        fetch: crate::rules::schema::FetchPlan::default(),
        fields: step
            .fields
            .into_iter()
            .map(|(name, field)| compile_field(name, field))
            .collect::<Result<BTreeMap<_, _>, _>>()?,
        bind: step.bind,
        follow: step
            .follow
            .into_iter()
            .map(|follow| compile_follow(follow, registry))
            .collect::<Result<Vec<_>, _>>()?,
        output: step
            .output
            .map(|output| compile_output(output, sinks))
            .transpose()?,
        default_middlewares: MiddlewareMap::new(),
        middlewares: MiddlewareMap::new(),
    })
}

fn compile_field(name: String, field: FieldConfig) -> Result<(String, FieldPlan), SpiderError> {
    Ok((
        name.clone(),
        FieldPlan {
            name,
            selector_kind: detect_selector_kind(&field.selector),
            selector: field.selector,
            kind: field.kind,
        },
    ))
}

fn compile_follow(
    follow: FollowConfig,
    registry: &EngineRegistryConfig,
) -> Result<FollowPlan, SpiderError> {
    Ok(FollowPlan {
        item_selector_kind: follow
            .item
            .as_ref()
            .map(|selector| detect_selector_kind(selector)),
        item: follow.item,
        next_step: follow.next_step,
        request: compile_request_plan(follow.request)?,
        meta: follow.meta,
        allow_url_pattern: follow.allow_url_pattern,
        middleware: lower_engine_refs(&follow.engine, registry)?,
    })
}

fn compile_output(
    output: OutputConfig,
    _sinks: &BTreeMap<String, SinkConfig>,
) -> Result<OutputPlan, SpiderError> {
    let validators = compile_output_validator(output.validator)?;

    Ok(OutputPlan {
        item: output.item,
        validators,
        sinks: output.sinks,
    })
}

fn compile_output_validator(
    validator: Option<OutputValidatorConfig>,
) -> Result<Vec<FieldValidator>, SpiderError> {
    let Some(validator) = validator else {
        return Ok(Vec::new());
    };

    let mut validators = BTreeMap::new();

    for required in &validator.required {
        validators
            .entry(required.clone())
            .or_insert_with(|| validator_field(required.clone(), Type::Text).required());
    }

    for (field, config) in validator.fields {
        let compiled = compile_field_validator(field.clone(), config)?;
        let compiled = if validator.required.iter().any(|required| required == &field) {
            compiled.required()
        } else {
            compiled
        };
        validators.insert(field, compiled);
    }

    Ok(validators.into_values().collect())
}

fn compile_field_validator(
    field: String,
    config: OutputFieldValidatorConfig,
) -> Result<FieldValidator, SpiderError> {
    let value_type = match config.value_type.as_str() {
        "string" | "text" => Type::Text,
        "number" => Type::Number,
        "bool" => Type::Bool,
        "list" => Type::List,
        "object" => Type::Object,
        other => {
            return Err(SpiderError::rules(format!(
                "unsupported output validation type: {other}"
            )));
        }
    };

    let mut validation = validator_field(field, value_type);

    if let Some(min_length) = config.min_length {
        validation = validation.min_length(min_length);
    }
    if let Some(max_length) = config.max_length {
        validation = validation.max_length(max_length);
    }
    if let Some(pattern) = config.pattern {
        validation = validation.regex(pattern);
    }
    if !config.enum_values.is_empty() {
        validation = validation.enum_values(config.enum_values);
    }
    if let Some(format) = config.format {
        validation = apply_validator_format(validation, &format)?;
    }

    Ok(validation)
}

fn apply_validator_format(
    validation: FieldValidator,
    format: &str,
) -> Result<FieldValidator, SpiderError> {
    match format {
        "url" => Ok(validation.regex(r"^https?://\S+$")),
        "datetime" | "date" => Ok(validation.transform(Transform::ParseDatetime)),
        other => Err(SpiderError::rules(format!(
            "unsupported output validation format: {other}"
        ))),
    }
}

fn compile_request_plan(request: RequestConfig) -> Result<RequestPlan, SpiderError> {
    Ok(RequestPlan {
        mode: request
            .mode
            .as_deref()
            .map(RequestMode::try_from)
            .transpose()
            .map_err(SpiderError::rules)?,
        method: request.method,
        url: request.url,
        query: request.query,
        headers: request.headers,
        cookies: request.cookies,
        timeout: request.timeout,
        proxy: request.proxy,
        session: request.session,
        encoding: request.encoding,
        priority: request.priority,
        flags: request.flags,
        cb_kwargs: request.cb_kwargs,
        errback: request.errback,
        body: request.body,
        allow_redirects: request.allow_redirects,
        skip: request.skip,
    })
}

fn lower_engine_refs(
    refs: &EngineRefs,
    registry: &EngineRegistryConfig,
) -> Result<MiddlewareMap, SpiderError> {
    let mut middleware = MiddlewareMap::new();

    lower_registry_ref(
        &mut middleware,
        DEDUP,
        refs.dedup.as_deref(),
        &registry.dedup,
        Stage::Enqueue,
        0,
    )?;
    lower_registry_ref(
        &mut middleware,
        CONCURRENCY,
        refs.concurrency.as_deref(),
        &registry.concurrency,
        Stage::Download,
        225,
    )?;
    lower_registry_ref(
        &mut middleware,
        INTERVAL,
        refs.interval.as_deref(),
        &registry.interval,
        Stage::Download,
        120,
    )?;
    lower_registry_ref(
        &mut middleware,
        RATE_LIMIT,
        refs.rate_limit.as_deref(),
        &registry.rate_limit,
        Stage::Download,
        130,
    )?;
    lower_registry_ref(
        &mut middleware,
        AUTO_THROTTLE,
        refs.auto_throttle.as_deref(),
        &registry.auto_throttle,
        Stage::Download,
        120,
    )?;
    lower_registry_ref(
        &mut middleware,
        RETRY_BY_STATUS,
        refs.retry_by_status.as_deref(),
        &registry.retry_by_status,
        Stage::Download,
        200,
    )?;
    lower_registry_ref(
        &mut middleware,
        RETRY_BY_ERROR,
        refs.retry_by_error.as_deref(),
        &registry.retry_by_error,
        Stage::Download,
        210,
    )?;

    Ok(middleware)
}

fn lower_registry_ref(
    target: &mut MiddlewareMap,
    key: &str,
    reference: Option<&str>,
    registry: &BTreeMap<String, BTreeMap<String, Value>>,
    stage: Stage,
    order: i32,
) -> Result<(), SpiderError> {
    let Some(reference) = reference else {
        return Ok(());
    };
    let options = registry.get(reference).ok_or_else(|| {
        SpiderError::rules(format!(
            "engine middleware reference not found: {key}.{reference}"
        ))
    })?;

    target.insert(
        key.to_string(),
        crate::middleware::Config {
            enabled: true,
            stage,
            order,
            options: options.clone(),
        },
    );

    Ok(())
}

fn selector_value_kind_from_object(
    object: &BTreeMap<String, Value>,
) -> Result<SelectorValueKind, SpiderError> {
    if object.get("html").and_then(Value::as_bool) == Some(true) {
        return Ok(SelectorValueKind::Html);
    }

    if let Some(attr) = object.get("attr") {
        let attr = attr
            .as_str()
            .ok_or_else(|| SpiderError::rules("value.attr must be a string"))?;
        return Ok(SelectorValueKind::Attribute(attr.to_string()));
    }

    Ok(SelectorValueKind::Text)
}

fn extract_kind_from_object(object: &BTreeMap<String, Value>) -> ExtractKind {
    if object.get("html").and_then(Value::as_bool) == Some(true) {
        ExtractKind::Html
    } else if let Some(attr) = object.get("attr").and_then(Value::as_str) {
        ExtractKind::Attribute(attr.to_string())
    } else {
        ExtractKind::Text
    }
}

pub(crate) fn detect_selector_kind(selector: &str) -> SelectorKind {
    let selector = selector.trim();
    if selector.starts_with("//")
        || selector.starts_with(".//")
        || selector.starts_with("./")
        || selector.starts_with('/')
        || selector.starts_with('(')
    {
        SelectorKind::XPath
    } else {
        SelectorKind::Css
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
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| SpiderError::rules(format!("{label} is required")))
}

fn optional_string<'a>(value: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn string_list(value: Option<&Value>, label: &str) -> Result<Vec<String>, SpiderError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = expect_array(value, label)?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| SpiderError::rules(format!("{label} entries must be strings")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::{DEDUP, RATE_LIMIT, RETRY_BY_STATUS};
    use serde_json::json;

    #[test]
    fn compile_rules_parses_v1_structure_and_lowers_engine_refs() {
        let compiled = compile_rules(Value::from(json!({
            "spider": { "name": "demo" },
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
                },
                "retry_by_status": {
                    "default_http_retry": {
                        "count": 3,
                        "status": [429, 500],
                        "backoff": [1000, 3000]
                    }
                }
            },
            "sinks": {
                "article_file": {
                    "type": "file",
                    "path": "./output/articles.jsonl"
                }
            },
            "seeds": [{
                "id": "seed-a",
                "request": {
                    "url": "https://example.com/start"
                },
                "engine": {
                    "dedup": "request_url"
                },
                "next_step": "parse_list"
            }],
            "steps": [{
                "id": "parse_list",
                "fields": {
                    "title": {
                        "selector": "h1",
                        "text": true
                    }
                },
                "follow": [{
                    "item": ".news li",
                    "next_step": "parse_detail",
                    "request": {
                        "url": {
                            "selector": "a",
                            "attr": "href"
                        }
                    },
                    "engine": {
                        "rate_limit": "origin_budget",
                        "retry_by_status": "default_http_retry",
                        "dedup": "request_url"
                    }
                }]
            }, {
                "id": "parse_detail",
                "output": {
                    "item": {
                        "title": { "from": "$meta.title" }
                    },
                    "validate": {
                        "required": ["title"],
                        "fields": {
                            "title": {
                                "type": "string",
                                "min_length": 1
                            }
                        }
                    },
                    "sinks": ["article_file"]
                }
            }]
        })))
        .expect("rules should compile");

        assert_eq!(compiled.spider.name, "demo");
        assert_eq!(compiled.seeds.len(), 1);
        assert_eq!(compiled.steps.len(), 2);

        let seed = compiled.seeds.first().expect("seed should exist");
        assert!(seed.middleware.contains_key(DEDUP));

        let follow = compiled.steps[0]
            .follow
            .first()
            .expect("follow should exist");
        assert_eq!(follow.item.as_deref(), Some(".news li"));
        assert_eq!(follow.item_selector_kind, Some(SelectorKind::Css));
        assert!(follow.middleware.contains_key(DEDUP));
        assert!(follow.middleware.contains_key(RATE_LIMIT));
        assert!(follow.middleware.contains_key(RETRY_BY_STATUS));

        let output = compiled.steps[1]
            .output
            .as_ref()
            .expect("output should exist");
        assert_eq!(output.validators.len(), 1);
        assert_eq!(output.sinks, vec!["article_file".to_string()]);
    }

    #[test]
    fn detect_selector_kind_treats_xpath_shapes_as_xpath() {
        assert_eq!(detect_selector_kind("//article/a"), SelectorKind::XPath);
        assert_eq!(detect_selector_kind(".//a"), SelectorKind::XPath);
        assert_eq!(detect_selector_kind(".news a"), SelectorKind::Css);
        assert_eq!(detect_selector_kind("a.detail-link"), SelectorKind::Css);
    }
}
