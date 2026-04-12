use crate::middleware::{
    AUTO_THROTTLE, CONCURRENCY, HTTP_CACHE, INTERVAL, RATE_LIMIT, RETRY_BY_ERROR, RETRY_BY_STATUS,
    Stage,
};
use crate::value::Value;
use jiff::SignedDuration;
use std::collections::BTreeMap;

/// Global runtime configuration.
///
/// `Config` only groups global knobs. Engine scheduling controls live under
/// `engine`, default request middleware sources live under `request`, and
/// shared service integrations live under `robots` and `openai`.
///
/// ```rust,ignore
/// let config = Config::default()
///     .with_download_delay(SignedDuration::from_millis(200))
///     .with_concurrent_requests(16)
///     .with_retry_times(3)
///     .with_retry_http_codes(vec![500, 502, 503]);
///
/// let engine = Engine::new().with_config(config);
/// ```
#[derive(Debug, Clone)]
pub struct Config {
    pub engine: EngineConfig,
    pub request: RequestConfig,
    pub robots: Robots,
    pub openai: Openai,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub requests: usize,
    pub requests_per_domain: usize,
    pub idle_timeout: SignedDuration,
}

#[derive(Debug, Clone)]
pub struct RequestConfig {
    pub middleware: BTreeMap<String, crate::middleware::Config>,
}

#[derive(Debug, Clone)]
pub struct Robots {
    pub obey: bool,
    pub user_agent: Option<String>,
    pub sitemap_seeds: bool,
    pub sitemap_seed_priority: i32,
    pub sitemap_seed_depth: u32,
}

#[derive(Debug, Clone)]
pub struct Openai {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine: EngineConfig::default(),
            request: RequestConfig::default(),
            robots: Robots::default(),
            openai: Openai::default(),
        }
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            requests: 16,
            requests_per_domain: 8,
            idle_timeout: SignedDuration::from_secs(5),
        }
    }
}

impl Default for RequestConfig {
    fn default() -> Self {
        Self {
            middleware: BTreeMap::new(),
        }
    }
}

impl Default for Robots {
    fn default() -> Self {
        Self {
            obey: false,
            user_agent: None,
            sitemap_seeds: false,
            sitemap_seed_priority: 0,
            sitemap_seed_depth: 0,
        }
    }
}

impl Default for Openai {
    fn default() -> Self {
        Self {
            api_key: std::env::var("OPENAI_API_KEY").ok(),
            base_url: std::env::var("OPENAI_BASE_URL").ok(),
            model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string()),
        }
    }
}

impl Config {
    pub fn with_download_delay(mut self, delay: SignedDuration) -> Self {
        apply_download_delay(&mut self.request.middleware, delay);
        self
    }

    pub fn with_auto_throttle(mut self, enabled: bool) -> Self {
        apply_auto_throttle_enabled(&mut self.request.middleware, enabled);
        self
    }

    pub fn with_auto_throttle_target_concurrency(mut self, target: f64) -> Self {
        if target.is_finite() && target > 0.0 {
            let config = ensure_auto_throttle_config(&mut self.request.middleware);
            config
                .options
                .insert("target_concurrency".to_string(), Value::Number(target));
            self.request.middleware.remove(INTERVAL);
        }
        self
    }

    pub fn with_auto_throttle_max_delay(mut self, delay: SignedDuration) -> Self {
        let config = ensure_auto_throttle_config(&mut self.request.middleware);
        config.options.insert(
            "max_interval".to_string(),
            Value::Number(non_negative_duration(delay).as_millis() as f64),
        );
        self.request.middleware.remove(INTERVAL);
        self
    }

    pub fn with_http_cache(mut self, enabled: bool) -> Self {
        if enabled {
            ensure_http_cache_config(&mut self.request.middleware);
        } else {
            self.request.middleware.remove(HTTP_CACHE);
        }
        self
    }

    pub fn with_http_cache_ttl(mut self, ttl: SignedDuration) -> Self {
        let config = ensure_http_cache_config(&mut self.request.middleware);
        config.options.insert(
            "ttl".to_string(),
            Value::Number(non_negative_duration(ttl).as_millis() as f64),
        );
        self
    }

    pub fn without_http_cache_ttl(mut self) -> Self {
        let config = ensure_http_cache_config(&mut self.request.middleware);
        config.options.insert("ttl".to_string(), Value::Null);
        self
    }

    pub fn with_http_cache_strategy(
        mut self,
        strategy: crate::middleware::http_cache::Strategy,
    ) -> Self {
        let config = ensure_http_cache_config(&mut self.request.middleware);
        config.options.insert(
            "strategy".to_string(),
            Value::String(strategy.as_str().to_string()),
        );
        self
    }

    pub fn with_http_cache_file(mut self, path: impl Into<String>) -> Self {
        let config = ensure_http_cache_config(&mut self.request.middleware);
        config
            .options
            .insert("backend".to_string(), Value::String("file".to_string()));
        config
            .options
            .insert("path".to_string(), Value::String(path.into()));
        self
    }

    pub fn with_concurrent_requests(mut self, n: usize) -> Self {
        self.engine.requests = n;
        self
    }

    pub fn with_concurrent_requests_per_domain(mut self, n: usize) -> Self {
        self.engine.requests_per_domain = n;
        self
    }

    pub fn with_retry_times(mut self, n: u32) -> Self {
        set_retry_times(&mut self.request.middleware, n);
        self
    }

    pub fn with_retry_http_codes(mut self, codes: Vec<u16>) -> Self {
        set_retry_http_codes(&mut self.request.middleware, &codes);
        self
    }

    pub fn with_idle_timeout(mut self, timeout: SignedDuration) -> Self {
        self.engine.idle_timeout = non_negative_duration(timeout);
        self
    }

    pub fn with_request_middlewares(
        mut self,
        middlewares: BTreeMap<String, crate::middleware::Config>,
    ) -> Self {
        self.request.middleware = middlewares;
        self
    }

    pub fn with_request_middleware(
        mut self,
        key: impl Into<String>,
        config: crate::middleware::Config,
    ) -> Self {
        self.request.middleware.insert(key.into(), config);
        self
    }

    pub fn with_robots_obey(mut self, obey: bool) -> Self {
        self.robots.obey = obey;
        self
    }

    pub fn with_robots_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.robots.user_agent = Some(user_agent.into());
        self
    }

    pub fn with_robots_sitemap_seeds(mut self, enabled: bool) -> Self {
        self.robots.sitemap_seeds = enabled;
        self
    }

    pub fn with_robots_sitemap_seed_priority(mut self, priority: i32) -> Self {
        self.robots.sitemap_seed_priority = priority;
        self
    }

    pub fn with_robots_sitemap_seed_depth(mut self, depth: u32) -> Self {
        self.robots.sitemap_seed_depth = depth;
        self
    }

    pub fn with_openai_api_key(mut self, key: impl Into<String>) -> Self {
        self.openai.api_key = Some(key.into());
        self
    }

    pub fn with_openai_base_url(mut self, url: impl Into<String>) -> Self {
        self.openai.base_url = Some(url.into());
        self
    }

    pub fn with_openai_model(mut self, model: impl Into<String>) -> Self {
        self.openai.model = model.into();
        self
    }
}

impl RequestConfig {
    pub(crate) fn merged_middleware(&self) -> BTreeMap<String, crate::middleware::Config> {
        let mut defaults = default_request_middleware();
        for (key, config) in &self.middleware {
            defaults.insert(key.clone(), config.clone());
        }
        defaults
    }
}

impl Robots {
    pub(crate) fn resolved_user_agent(&self, spider_name: &str) -> String {
        self.user_agent
            .clone()
            .unwrap_or_else(|| spider_name.to_string())
    }
}

fn default_request_middleware() -> BTreeMap<String, crate::middleware::Config> {
    let mut defaults = BTreeMap::new();

    defaults.insert(CONCURRENCY.to_string(), middleware_config(225, Vec::new()));
    defaults.insert(INTERVAL.to_string(), middleware_config(120, Vec::new()));
    defaults.insert(
        AUTO_THROTTLE.to_string(),
        middleware_config(120, Vec::new()),
    );
    defaults.insert(RATE_LIMIT.to_string(), middleware_config(130, Vec::new()));

    let retry_count = Value::Number(2.0);
    defaults.insert(
        RETRY_BY_STATUS.to_string(),
        middleware_config(
            200,
            vec![
                ("count".to_string(), Value::Array(vec![retry_count.clone()])),
                (
                    "status".to_string(),
                    Value::Array(
                        [500_u16, 502, 503, 504, 408]
                            .into_iter()
                            .map(|status| Value::Number(status as f64))
                            .collect(),
                    ),
                ),
            ],
        ),
    );

    defaults.insert(
        RETRY_BY_ERROR.to_string(),
        middleware_config(
            210,
            vec![("count".to_string(), Value::Array(vec![retry_count]))],
        ),
    );

    defaults
}

fn apply_download_delay(
    middleware: &mut BTreeMap<String, crate::middleware::Config>,
    delay: SignedDuration,
) {
    let millis = non_negative_duration(delay).as_millis() as f64;
    if auto_throttle_is_enabled(middleware) {
        let config = ensure_auto_throttle_config(middleware);
        config
            .options
            .insert("start_interval".to_string(), Value::Number(millis));
        config
            .options
            .insert("min_interval".to_string(), Value::Number(millis));
    } else if millis > 0.0 {
        let config = ensure_interval_config(middleware);
        config
            .options
            .insert("interval".to_string(), Value::Number(millis));
    } else {
        middleware.remove(INTERVAL);
    }
}

fn apply_auto_throttle_enabled(
    middleware: &mut BTreeMap<String, crate::middleware::Config>,
    enabled: bool,
) {
    if enabled {
        let delay = interval_delay(middleware);
        let config = ensure_auto_throttle_config(middleware);
        config
            .options
            .entry("target_concurrency".to_string())
            .or_insert_with(|| Value::Number(1.0));
        config
            .options
            .entry("start_interval".to_string())
            .or_insert_with(|| Value::Number(delay));
        config
            .options
            .entry("min_interval".to_string())
            .or_insert_with(|| Value::Number(delay));
        config
            .options
            .entry("max_interval".to_string())
            .or_insert_with(|| Value::Number(60_000.0));
        middleware.remove(INTERVAL);
    } else if let Some(config) = middleware.remove(AUTO_THROTTLE) {
        let delay = config
            .options
            .get("min_interval")
            .or_else(|| config.options.get("start_interval"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if delay > 0.0 {
            let interval = ensure_interval_config(middleware);
            interval
                .options
                .insert("interval".to_string(), Value::Number(delay));
        }
    }
}

fn set_retry_times(middleware: &mut BTreeMap<String, crate::middleware::Config>, count: u32) {
    let retry_count = Value::Array(vec![Value::Number(count as f64)]);

    ensure_retry_by_status_config(middleware)
        .options
        .insert("count".to_string(), retry_count.clone());
    ensure_retry_by_error_config(middleware)
        .options
        .insert("count".to_string(), retry_count);
}

fn set_retry_http_codes(
    middleware: &mut BTreeMap<String, crate::middleware::Config>,
    codes: &[u16],
) {
    ensure_retry_by_status_config(middleware).options.insert(
        "status".to_string(),
        Value::Array(
            codes
                .iter()
                .map(|status| Value::Number(*status as f64))
                .collect(),
        ),
    );
}

fn auto_throttle_is_enabled(middleware: &BTreeMap<String, crate::middleware::Config>) -> bool {
    middleware
        .get(AUTO_THROTTLE)
        .map(|config| !config.options.is_empty())
        .unwrap_or(false)
}

fn interval_delay(middleware: &BTreeMap<String, crate::middleware::Config>) -> f64 {
    middleware
        .get(INTERVAL)
        .and_then(|config| config.options.get("interval"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

fn ensure_interval_config(
    middleware: &mut BTreeMap<String, crate::middleware::Config>,
) -> &mut crate::middleware::Config {
    middleware
        .entry(INTERVAL.to_string())
        .or_insert_with(|| middleware_config(120, Vec::new()))
}

fn ensure_auto_throttle_config(
    middleware: &mut BTreeMap<String, crate::middleware::Config>,
) -> &mut crate::middleware::Config {
    let delay = interval_delay(middleware);
    middleware
        .entry(AUTO_THROTTLE.to_string())
        .or_insert_with(|| {
            let mut config = middleware_config(120, Vec::new());
            config
                .options
                .insert("target_concurrency".to_string(), Value::Number(1.0));
            config
                .options
                .insert("start_interval".to_string(), Value::Number(delay));
            config
                .options
                .insert("min_interval".to_string(), Value::Number(delay));
            config
                .options
                .insert("max_interval".to_string(), Value::Number(60_000.0));
            config
        })
}

fn ensure_retry_by_status_config(
    middleware: &mut BTreeMap<String, crate::middleware::Config>,
) -> &mut crate::middleware::Config {
    middleware
        .entry(RETRY_BY_STATUS.to_string())
        .or_insert_with(|| {
            middleware_config(
                200,
                vec![
                    ("count".to_string(), Value::Array(vec![Value::Number(2.0)])),
                    (
                        "status".to_string(),
                        Value::Array(
                            [500_u16, 502, 503, 504, 408]
                                .into_iter()
                                .map(|status| Value::Number(status as f64))
                                .collect(),
                        ),
                    ),
                ],
            )
        })
}

fn ensure_retry_by_error_config(
    middleware: &mut BTreeMap<String, crate::middleware::Config>,
) -> &mut crate::middleware::Config {
    middleware
        .entry(RETRY_BY_ERROR.to_string())
        .or_insert_with(|| {
            middleware_config(
                210,
                vec![("count".to_string(), Value::Array(vec![Value::Number(2.0)]))],
            )
        })
}

fn ensure_http_cache_config(
    middleware: &mut BTreeMap<String, crate::middleware::Config>,
) -> &mut crate::middleware::Config {
    middleware
        .entry(HTTP_CACHE.to_string())
        .or_insert_with(|| crate::middleware::Config {
            enabled: true,
            stage: Stage::Download,
            order: 110,
            options: BTreeMap::new(),
        })
}

fn middleware_config(order: i32, options: Vec<(String, Value)>) -> crate::middleware::Config {
    crate::middleware::Config {
        enabled: true,
        stage: Stage::Download,
        order,
        options: options.into_iter().collect(),
    }
}

fn non_negative_duration(duration: SignedDuration) -> SignedDuration {
    if duration.is_negative() {
        SignedDuration::ZERO
    } else {
        duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::{HTTP_CACHE, INTERVAL, RETRY_BY_STATUS};

    #[test]
    fn http_cache_builders_fill_middleware_options() {
        let config = Config::default()
            .with_http_cache_ttl(SignedDuration::from_secs(30))
            .with_http_cache_strategy(crate::middleware::http_cache::Strategy::Validators)
            .with_http_cache_file("output/custom-http-cache.json");

        let middleware = config.request.middleware.get(HTTP_CACHE).unwrap();

        assert_eq!(
            middleware.options.get("ttl"),
            Some(&Value::Number(30_000.0))
        );
        assert_eq!(
            middleware.options.get("strategy"),
            Some(&Value::String("validators".to_string()))
        );
        assert_eq!(
            middleware.options.get("backend"),
            Some(&Value::String("file".to_string()))
        );
        assert_eq!(
            middleware.options.get("path"),
            Some(&Value::String("output/custom-http-cache.json".to_string()))
        );
    }

    #[test]
    fn http_cache_builders_can_disable_ttl_explicitly() {
        let config = Config::default().without_http_cache_ttl();

        let middleware = config.request.middleware.get(HTTP_CACHE).unwrap();
        assert_eq!(middleware.options.get("ttl"), Some(&Value::Null));
    }

    #[test]
    fn robots_builders_override_defaults() {
        let config = Config::default()
            .with_robots_sitemap_seeds(true)
            .with_robots_sitemap_seed_priority(42)
            .with_robots_sitemap_seed_depth(3);

        assert!(config.robots.sitemap_seeds);
        assert_eq!(config.robots.sitemap_seed_priority, 42);
        assert_eq!(config.robots.sitemap_seed_depth, 3);
    }

    #[test]
    fn merged_middleware_keeps_runtime_defaults_and_explicit_overrides() {
        let config = Config::default()
            .with_retry_times(3)
            .with_retry_http_codes(vec![429, 500])
            .with_request_middleware(
                INTERVAL,
                middleware_config(120, vec![("interval".to_string(), Value::Number(250.0))]),
            );

        let middleware = config.request.merged_middleware();

        assert!(middleware.contains_key(RETRY_BY_STATUS));
        assert_eq!(
            middleware
                .get(INTERVAL)
                .and_then(|config| config.options.get("interval")),
            Some(&Value::Number(250.0))
        );
    }
}
