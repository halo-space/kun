use crate::middleware::Map as MiddlewareMap;
use crate::runtime::Config as RuntimeConfig;
use crate::value::Value;
use jiff::SignedDuration;
use std::collections::BTreeMap;

/// Engine-level global configuration, similar to Scrapy's `settings.py`.
///
/// The spider itself does not own these values and stays focused on parsing.
/// Runtime parameters such as rate limits, retries, concurrency, and timeouts
/// all live in `Settings`.
///
/// ```rust,ignore
/// let settings = Settings::default()
///     .download_delay(SignedDuration::from_millis(200))
///     .concurrent_requests(16)
///     .retry_times(3)
///     .retry_http_codes(vec![500, 502, 503]);
///
/// let engine = Engine::new().with_settings(settings);
/// ```
#[derive(Debug, Clone)]
pub struct Settings {
    pub download_delay: SignedDuration,
    pub auto_throttle: bool,
    pub auto_throttle_target_concurrency: f64,
    pub auto_throttle_max_delay: SignedDuration,
    pub concurrent_requests: usize,
    pub concurrent_requests_per_domain: usize,
    pub retry_times: u32,
    pub retry_http_codes: Vec<u16>,
    pub idle_timeout: SignedDuration,
    pub middlewares: MiddlewareMap,
    pub runtime_override: Option<RuntimeConfig>,
    pub connection_pool_size: usize,
    pub robots_obey: bool,
    pub robots_user_agent: Option<String>,
    pub robots_sitemap_seeds: bool,
    pub openai_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_model: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_delay: SignedDuration::from_millis(0),
            auto_throttle: false,
            auto_throttle_target_concurrency: 1.0,
            auto_throttle_max_delay: SignedDuration::from_secs(60),
            concurrent_requests: 16,
            concurrent_requests_per_domain: 8,
            retry_times: 2,
            retry_http_codes: vec![500, 502, 503, 504, 408],
            idle_timeout: SignedDuration::from_secs(5),
            middlewares: MiddlewareMap::new(),
            runtime_override: None,
            connection_pool_size: 100,
            robots_obey: false,
            robots_user_agent: None,
            robots_sitemap_seeds: false,
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            openai_base_url: std::env::var("OPENAI_BASE_URL").ok(),
            openai_model: std::env::var("OPENAI_MODEL")
                .unwrap_or_else(|_| "gpt-4o-mini".to_string()),
        }
    }
}

impl Settings {
    pub fn with_download_delay(mut self, delay: SignedDuration) -> Self {
        self.download_delay = non_negative_duration(delay);
        self
    }

    pub fn with_auto_throttle(mut self, enabled: bool) -> Self {
        self.auto_throttle = enabled;
        self
    }

    pub fn with_auto_throttle_target_concurrency(mut self, target: f64) -> Self {
        if target.is_finite() && target > 0.0 {
            self.auto_throttle_target_concurrency = target;
        }
        self
    }

    pub fn with_auto_throttle_max_delay(mut self, delay: SignedDuration) -> Self {
        self.auto_throttle_max_delay = non_negative_duration(delay);
        self
    }

    pub fn with_http_cache(mut self, enabled: bool) -> Self {
        if enabled {
            ensure_http_cache_config(&mut self);
        } else {
            self.middlewares.remove("http_cache");
        }
        self
    }

    pub fn with_http_cache_ttl(mut self, ttl: SignedDuration) -> Self {
        let config = ensure_http_cache_config(&mut self);
        config.options.insert(
            "ttl".to_string(),
            Value::Number(non_negative_duration(ttl).as_millis() as f64),
        );
        self
    }

    pub fn without_http_cache_ttl(mut self) -> Self {
        let config = ensure_http_cache_config(&mut self);
        config.options.insert("ttl".to_string(), Value::Null);
        self
    }

    pub fn with_http_cache_strategy(
        mut self,
        strategy: crate::middleware::http_cache::Strategy,
    ) -> Self {
        let config = ensure_http_cache_config(&mut self);
        config.options.insert(
            "strategy".to_string(),
            Value::String(strategy.as_str().to_string()),
        );
        self
    }

    pub fn with_http_cache_file(mut self, path: impl Into<String>) -> Self {
        let config = ensure_http_cache_config(&mut self);
        config
            .options
            .insert("backend".to_string(), Value::String("file".to_string()));
        config
            .options
            .insert("path".to_string(), Value::String(path.into()));
        self
    }

    pub fn with_concurrent_requests(mut self, n: usize) -> Self {
        self.concurrent_requests = n;
        self
    }

    pub fn with_concurrent_requests_per_domain(mut self, n: usize) -> Self {
        self.concurrent_requests_per_domain = n;
        self
    }

    pub fn with_retry_times(mut self, n: u32) -> Self {
        self.retry_times = n;
        self
    }

    pub fn with_retry_http_codes(mut self, codes: Vec<u16>) -> Self {
        self.retry_http_codes = codes;
        self
    }

    pub fn with_idle_timeout(mut self, timeout: SignedDuration) -> Self {
        self.idle_timeout = non_negative_duration(timeout);
        self
    }

    pub fn with_middlewares(mut self, middlewares: MiddlewareMap) -> Self {
        self.middlewares = middlewares;
        self
    }

    pub fn with_middleware(
        mut self,
        key: impl Into<String>,
        config: crate::middleware::Config,
    ) -> Self {
        self.middlewares.insert(key.into(), config);
        self
    }

    pub fn with_runtime(mut self, runtime: RuntimeConfig) -> Self {
        self.runtime_override = Some(runtime);
        self
    }

    pub fn with_connection_pool_size(mut self, size: usize) -> Self {
        self.connection_pool_size = size;
        self
    }

    pub fn with_robots_obey(mut self, obey: bool) -> Self {
        self.robots_obey = obey;
        self
    }

    pub fn with_robots_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.robots_user_agent = Some(user_agent.into());
        self
    }

    pub fn with_robots_sitemap_seeds(mut self, enabled: bool) -> Self {
        self.robots_sitemap_seeds = enabled;
        self
    }

    pub fn with_openai_api_key(mut self, key: impl Into<String>) -> Self {
        self.openai_api_key = Some(key.into());
        self
    }

    pub fn with_openai_base_url(mut self, url: impl Into<String>) -> Self {
        self.openai_base_url = Some(url.into());
        self
    }

    pub fn with_openai_model(mut self, model: impl Into<String>) -> Self {
        self.openai_model = model.into();
        self
    }

    pub(crate) fn to_runtime_config(&self) -> RuntimeConfig {
        if let Some(ref rt) = self.runtime_override {
            return rt.clone();
        }

        let mut schedule = std::collections::BTreeMap::new();
        if self.auto_throttle {
            schedule.insert("auto_throttle".to_string(), Value::Bool(true));
            schedule.insert(
                "target_concurrency".to_string(),
                Value::Number(self.auto_throttle_target_concurrency),
            );
            schedule.insert(
                "start_interval".to_string(),
                Value::Number(self.download_delay.as_millis() as f64),
            );
            schedule.insert(
                "min_interval".to_string(),
                Value::Number(self.download_delay.as_millis() as f64),
            );
            schedule.insert(
                "max_interval".to_string(),
                Value::Number(self.auto_throttle_max_delay.as_millis() as f64),
            );
        } else if self.download_delay.is_positive() {
            schedule.insert(
                "interval".to_string(),
                Value::Number(self.download_delay.as_millis() as f64),
            );
        }

        let mut retry = std::collections::BTreeMap::new();
        retry.insert("count".to_string(), Value::Number(self.retry_times as f64));
        retry.insert(
            "http_status".to_string(),
            Value::Array(
                self.retry_http_codes
                    .iter()
                    .map(|&c| Value::Number(c as f64))
                    .collect(),
            ),
        );

        RuntimeConfig {
            schedule,
            retry,
            dedup: std::collections::BTreeMap::new(),
        }
    }

    pub(crate) fn resolved_robots_user_agent(&self, spider_name: &str) -> String {
        self.robots_user_agent
            .clone()
            .unwrap_or_else(|| spider_name.to_string())
    }
}

fn ensure_http_cache_config(settings: &mut Settings) -> &mut crate::middleware::Config {
    settings
        .middlewares
        .entry("http_cache".to_string())
        .or_insert_with(|| crate::middleware::Config {
            enabled: true,
            stage: crate::middleware::Stage::Download,
            order: 110,
            options: BTreeMap::new(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_cache_settings_builders_fill_middleware_options() {
        let settings = Settings::default()
            .with_http_cache_ttl(SignedDuration::from_secs(30))
            .with_http_cache_strategy(crate::middleware::http_cache::Strategy::Validators)
            .with_http_cache_file("output/custom-http-cache.json");

        let config = settings.middlewares.get("http_cache").unwrap();

        assert_eq!(config.options.get("ttl"), Some(&Value::Number(30_000.0)));
        assert_eq!(
            config.options.get("strategy"),
            Some(&Value::String("validators".to_string()))
        );
        assert_eq!(
            config.options.get("backend"),
            Some(&Value::String("file".to_string()))
        );
        assert_eq!(
            config.options.get("path"),
            Some(&Value::String("output/custom-http-cache.json".to_string()))
        );
    }

    #[test]
    fn http_cache_settings_can_disable_ttl_explicitly() {
        let settings = Settings::default().without_http_cache_ttl();

        let config = settings.middlewares.get("http_cache").unwrap();
        assert_eq!(config.options.get("ttl"), Some(&Value::Null));
    }
}

fn non_negative_duration(duration: SignedDuration) -> SignedDuration {
    if duration.is_negative() {
        SignedDuration::ZERO
    } else {
        duration
    }
}
