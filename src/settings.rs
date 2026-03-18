use crate::middleware::Map as MiddlewareMap;
use crate::runtime::Config as RuntimeConfig;
use crate::value::Value;
use std::time::Duration;

/// 引擎级全局配置，对应 Scrapy 的 settings.py。
///
/// Spider 不持有这些配置 —— Spider 只管解析。
/// 所有运行参数（速率、重试、并发、超时等）都在 Settings 里。
///
/// ```rust,ignore
/// let settings = Settings::default()
///     .download_delay(Duration::from_millis(200))
///     .concurrent_requests(16)
///     .retry_times(3)
///     .retry_http_codes(vec![500, 502, 503]);
///
/// let engine = Engine::new(scheduler, http, browser)
///     .with_settings(settings);
/// ```
#[derive(Debug, Clone)]
pub struct Settings {
    pub download_delay: Duration,
    pub concurrent_requests: usize,
    pub concurrent_requests_per_domain: usize,
    pub retry_times: u32,
    pub retry_http_codes: Vec<u16>,
    pub dedup_enabled: bool,
    pub idle_timeout: Duration,
    pub middlewares: MiddlewareMap,
    pub runtime_override: Option<RuntimeConfig>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_delay: Duration::from_millis(0),
            concurrent_requests: 16,
            concurrent_requests_per_domain: 8,
            retry_times: 2,
            retry_http_codes: vec![500, 502, 503, 504, 408],
            dedup_enabled: true,
            idle_timeout: Duration::from_secs(5),
            middlewares: MiddlewareMap::new(),
            runtime_override: None,
        }
    }
}

impl Settings {
    pub fn download_delay(mut self, delay: Duration) -> Self {
        self.download_delay = delay;
        self
    }

    pub fn concurrent_requests(mut self, n: usize) -> Self {
        self.concurrent_requests = n;
        self
    }

    pub fn concurrent_requests_per_domain(mut self, n: usize) -> Self {
        self.concurrent_requests_per_domain = n;
        self
    }

    pub fn retry_times(mut self, n: u32) -> Self {
        self.retry_times = n;
        self
    }

    pub fn retry_http_codes(mut self, codes: Vec<u16>) -> Self {
        self.retry_http_codes = codes;
        self
    }

    pub fn dedup_enabled(mut self, enabled: bool) -> Self {
        self.dedup_enabled = enabled;
        self
    }

    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    pub fn middlewares(mut self, middlewares: MiddlewareMap) -> Self {
        self.middlewares = middlewares;
        self
    }

    pub fn with_middleware(
        mut self,
        key: impl Into<String>,
        config: crate::middleware::MiddlewareConfig,
    ) -> Self {
        self.middlewares.insert(key.into(), config);
        self
    }

    pub fn with_runtime(mut self, runtime: RuntimeConfig) -> Self {
        self.runtime_override = Some(runtime);
        self
    }

    pub(crate) fn to_runtime_config(&self) -> RuntimeConfig {
        if let Some(ref rt) = self.runtime_override {
            return rt.clone();
        }

        let mut schedule = std::collections::BTreeMap::new();
        let delay_ms = self.download_delay.as_millis() as f64;
        if delay_ms > 0.0 {
            schedule.insert("interval_ms".to_string(), Value::Number(delay_ms));
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

        let mut dedup = std::collections::BTreeMap::new();
        dedup.insert("enabled".to_string(), Value::Bool(self.dedup_enabled));

        RuntimeConfig {
            schedule,
            retry,
            dedup,
        }
    }
}
