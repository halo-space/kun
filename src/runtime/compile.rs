use crate::error::SpiderError;
use crate::middleware::{MiddlewareConfig, MiddlewareType};
use crate::runtime::{Config, MiddlewareMap};
use crate::value::Value;
use std::collections::BTreeMap;

pub fn compile(runtime: &Config) -> Result<MiddlewareMap, SpiderError> {
    let mut middleware = MiddlewareMap::new();

    compile_retry(runtime, &mut middleware)?;
    compile_dedup(runtime, &mut middleware)?;
    compile_schedule(runtime, &mut middleware)?;

    Ok(middleware)
}

pub fn merge(defaults: MiddlewareMap, explicit: MiddlewareMap) -> MiddlewareMap {
    let mut merged = defaults;

    for (key, config) in explicit {
        merged.insert(key, config);
    }

    merged
}

fn compile_retry(runtime: &Config, middleware: &mut MiddlewareMap) -> Result<(), SpiderError> {
    if runtime.retry.is_empty() {
        return Ok(());
    }

    let count = optional_number(&runtime.retry, "count");
    let backoff = optional_array(&runtime.retry, "backoff_ms");
    let statuses = optional_array(&runtime.retry, "http_status");

    if count.is_some() || backoff.is_some() || statuses.is_some() {
        if let Some(statuses) = statuses {
            middleware.insert(
                "retry_by_status".to_string(),
                config(
                    200,
                    vec![
                        optional_value("count", count.clone()),
                        optional_value("backoff_ms", backoff.clone()),
                        Some(("status".to_string(), Value::Array(statuses))),
                    ],
                ),
            );
        }

        middleware.insert(
            "retry_by_error".to_string(),
            config(
                210,
                vec![
                    optional_value("count", count),
                    optional_value("backoff_ms", optional_array(&runtime.retry, "backoff_ms")),
                ],
            ),
        );
    }

    Ok(())
}

fn compile_dedup(runtime: &Config, middleware: &mut MiddlewareMap) -> Result<(), SpiderError> {
    if runtime.dedup.is_empty() {
        return Ok(());
    }

    let enabled = runtime
        .dedup
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    if enabled {
        middleware.insert(
            "dedup".to_string(),
            MiddlewareConfig {
                enabled: true,
                r#type: MiddlewareType::Download,
                order: 220,
                options: runtime.dedup.clone(),
            },
        );
    }

    Ok(())
}

fn compile_schedule(runtime: &Config, middleware: &mut MiddlewareMap) -> Result<(), SpiderError> {
    if runtime.schedule.is_empty() {
        return Ok(());
    }

    if let Some(interval_ms) = runtime.schedule.get("interval_ms").cloned() {
        middleware.insert(
            "interval_gate".to_string(),
            config(120, vec![Some(("interval_ms".to_string(), interval_ms))]),
        );
    }

    if let Some(rate_per_minute) = runtime.schedule.get("rate_per_minute").cloned() {
        middleware.insert(
            "rate_limit".to_string(),
            config(130, vec![Some(("rate_per_minute".to_string(), rate_per_minute))]),
        );
    }

    Ok(())
}

fn config(order: i32, options: Vec<Option<(String, Value)>>) -> MiddlewareConfig {
    MiddlewareConfig {
        enabled: true,
        r#type: MiddlewareType::Download,
        order,
        options: options.into_iter().flatten().collect(),
    }
}

fn optional_value(key: &str, value: Option<Vec<Value>>) -> Option<(String, Value)> {
    value.map(|value| (key.to_string(), Value::Array(value)))
}

fn optional_number(map: &BTreeMap<String, Value>, key: &str) -> Option<Vec<Value>> {
    map.get(key)
        .and_then(Value::as_f64)
        .map(Value::Number)
        .map(|value| vec![value])
}

fn optional_array(map: &BTreeMap<String, Value>, key: &str) -> Option<Vec<Value>> {
    map.get(key).and_then(Value::as_array).map(|values| values.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_generates_default_runtime_middlewares() {
        let runtime = Config {
            schedule: [
                ("interval_ms".to_string(), Value::Number(1000.0)),
                ("rate_per_minute".to_string(), Value::Number(120.0)),
            ]
            .into_iter()
            .collect(),
            retry: [
                ("count".to_string(), Value::Number(3.0)),
                (
                    "http_status".to_string(),
                    Value::Array(vec![Value::Number(429.0), Value::Number(500.0)]),
                ),
                (
                    "backoff_ms".to_string(),
                    Value::Array(vec![Value::Number(1000.0), Value::Number(3000.0)]),
                ),
            ]
            .into_iter()
            .collect(),
            dedup: [
                ("enabled".to_string(), Value::Bool(true)),
                ("key".to_string(), Value::String("url".to_string())),
            ]
            .into_iter()
            .collect(),
        };

        let compiled = compile(&runtime).unwrap();

        assert!(compiled.contains_key("retry_by_status"));
        assert!(compiled.contains_key("retry_by_error"));
        assert!(compiled.contains_key("dedup"));
        assert!(compiled.contains_key("interval_gate"));
        assert!(compiled.contains_key("rate_limit"));
    }

    #[test]
    fn merge_prefers_explicit_middleware() {
        let defaults = [(
            "rate_limit".to_string(),
            MiddlewareConfig {
                enabled: true,
                r#type: MiddlewareType::Download,
                order: 130,
                options: BTreeMap::new(),
            },
        )]
        .into_iter()
        .collect();

        let explicit = [(
            "rate_limit".to_string(),
            MiddlewareConfig {
                enabled: false,
                r#type: MiddlewareType::Download,
                order: 999,
                options: BTreeMap::new(),
            },
        )]
        .into_iter()
        .collect();

        let merged = merge(defaults, explicit);

        assert_eq!(merged["rate_limit"].enabled, false);
        assert_eq!(merged["rate_limit"].order, 999);
    }
}
