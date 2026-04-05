use crate::error::SpiderError;
use crate::middleware::{Config, Stage};
use crate::runtime::{Config as RuntimeConfig, MiddlewareMap};
use crate::value::Value;
use std::collections::BTreeMap;

pub fn compile(runtime: &RuntimeConfig) -> Result<MiddlewareMap, SpiderError> {
    let mut middleware = MiddlewareMap::new();

    compile_retry(runtime, &mut middleware)?;
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

fn compile_retry(
    runtime: &RuntimeConfig,
    middleware: &mut MiddlewareMap,
) -> Result<(), SpiderError> {
    if runtime.retry.is_empty() {
        return Ok(());
    }

    let count = optional_number(&runtime.retry, "count");
    let backoff = optional_array(&runtime.retry, "backoff");
    let statuses = optional_array(&runtime.retry, "http_status");

    if count.is_some() || backoff.is_some() || statuses.is_some() {
        if let Some(statuses) = statuses {
            middleware.insert(
                "retry_by_status".to_string(),
                config(
                    200,
                    vec![
                        optional_value("count", count.clone()),
                        optional_value("backoff", backoff.clone()),
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
                    optional_value("backoff", optional_array(&runtime.retry, "backoff")),
                ],
            ),
        );
    }

    Ok(())
}

fn compile_schedule(
    runtime: &RuntimeConfig,
    middleware: &mut MiddlewareMap,
) -> Result<(), SpiderError> {
    if runtime.schedule.is_empty() {
        return Ok(());
    }

    if let Some(concurrency) = runtime.schedule.get("concurrency").cloned() {
        middleware.insert(
            "concurrency_gate".to_string(),
            config(225, vec![Some(("concurrency".to_string(), concurrency))]),
        );
    }

    let auto_throttle_enabled = runtime
        .schedule
        .get("auto_throttle")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if auto_throttle_enabled {
        middleware.insert(
            "auto_throttle".to_string(),
            config(
                120,
                vec![
                    Some(("auto_throttle".to_string(), Value::Bool(true))),
                    runtime
                        .schedule
                        .get("target_concurrency")
                        .cloned()
                        .map(|value| ("target_concurrency".to_string(), value)),
                    runtime
                        .schedule
                        .get("start_interval")
                        .cloned()
                        .or_else(|| runtime.schedule.get("interval").cloned())
                        .map(|value| ("start_interval".to_string(), value)),
                    runtime
                        .schedule
                        .get("min_interval")
                        .cloned()
                        .or_else(|| runtime.schedule.get("interval").cloned())
                        .map(|value| ("min_interval".to_string(), value)),
                    runtime
                        .schedule
                        .get("max_interval")
                        .cloned()
                        .map(|value| ("max_interval".to_string(), value)),
                    runtime
                        .schedule
                        .get("error_backoff_ratio")
                        .cloned()
                        .map(|value| ("error_backoff_ratio".to_string(), value)),
                ],
            ),
        );
    } else if let Some(interval) = runtime.schedule.get("interval").cloned() {
        middleware.insert(
            "interval_gate".to_string(),
            config(120, vec![Some(("interval".to_string(), interval))]),
        );
    }

    if let Some(rate_per_minute) = runtime.schedule.get("rate_per_minute").cloned() {
        middleware.insert(
            "rate_limit".to_string(),
            config(
                130,
                vec![Some(("rate_per_minute".to_string(), rate_per_minute))],
            ),
        );
    }

    Ok(())
}

fn config(order: i32, options: Vec<Option<(String, Value)>>) -> Config {
    Config {
        enabled: true,
        stage: Stage::Download,
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
    map.get(key)
        .and_then(Value::as_array)
        .map(|values| values.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_generates_default_runtime_middlewares() {
        let runtime = RuntimeConfig {
            schedule: [
                ("concurrency".to_string(), Value::Number(2.0)),
                ("interval".to_string(), Value::Number(1000.0)),
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
                    "backoff".to_string(),
                    Value::Array(vec![Value::Number(1000.0), Value::Number(3000.0)]),
                ),
            ]
            .into_iter()
            .collect(),
            dedup: BTreeMap::new(),
        };

        let compiled = compile(&runtime).unwrap();

        assert!(compiled.contains_key("retry_by_status"));
        assert!(compiled.contains_key("retry_by_error"));
        assert!(compiled.contains_key("concurrency_gate"));
        assert!(compiled.contains_key("interval_gate"));
        assert!(compiled.contains_key("rate_limit"));
    }

    #[test]
    fn compile_prefers_auto_throttle_over_interval_gate() {
        let runtime = RuntimeConfig {
            schedule: [
                ("auto_throttle".to_string(), Value::Bool(true)),
                ("interval".to_string(), Value::Number(200.0)),
                ("target_concurrency".to_string(), Value::Number(2.0)),
                ("max_interval".to_string(), Value::Number(5_000.0)),
            ]
            .into_iter()
            .collect(),
            retry: BTreeMap::new(),
            dedup: BTreeMap::new(),
        };

        let compiled = compile(&runtime).unwrap();

        assert!(compiled.contains_key("auto_throttle"));
        assert!(!compiled.contains_key("interval_gate"));
        assert_eq!(
            compiled["auto_throttle"].options.get("min_interval"),
            Some(&Value::Number(200.0))
        );
    }

    #[test]
    fn merge_prefers_explicit_middleware() {
        let defaults = [(
            "rate_limit".to_string(),
            Config {
                enabled: true,
                stage: Stage::Download,
                order: 130,
                options: BTreeMap::new(),
            },
        )]
        .into_iter()
        .collect();

        let explicit = [(
            "rate_limit".to_string(),
            Config {
                enabled: false,
                stage: Stage::Download,
                order: 999,
                options: BTreeMap::new(),
            },
        )]
        .into_iter()
        .collect();

        let merged = merge(defaults, explicit);

        assert!(!merged["rate_limit"].enabled);
        assert_eq!(merged["rate_limit"].order, 999);
    }
}
