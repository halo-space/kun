pub mod context;
pub mod types;

use crate::downloader::traits::Downloader;
use crate::error::SpiderError;
use crate::middleware::{build as build_middleware, MiddlewareChain, MiddlewareType};
use crate::request::RequestMode;
use crate::response::Response;
use crate::rules::Compiled;
use crate::runtime::compile::{compile as compile_runtime, merge as merge_middleware};
use crate::scheduler::traits::Scheduler;
use crate::spider::{Output as SpiderOutput, Spider};

pub struct Engine<S, H, B> {
    pub scheduler: S,
    pub http: H,
    pub browser: B,
    pub middleware: MiddlewareChain,
    pub spider_middleware: MiddlewareChain,
}

impl<S, H, B> Engine<S, H, B>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
{
    pub fn new(scheduler: S, http: H, browser: B) -> Self {
        Self {
            scheduler,
            http,
            browser,
            middleware: MiddlewareChain::default(),
            spider_middleware: MiddlewareChain::default(),
        }
    }

    pub fn with_middleware(mut self, middleware: MiddlewareChain) -> Self {
        self.middleware = middleware;
        self
    }

    pub async fn execute_once(&mut self) -> Result<Option<Response>, SpiderError> {
        let Some(task) = self.scheduler.lease().await? else {
            return Ok(None);
        };

        self.middleware
            .process_request(MiddlewareType::Download)
            .await?;

        let result = match task.request.mode {
            RequestMode::Http => self.http.fetch(&task.request).await,
            RequestMode::Browser => self.browser.fetch(&task.request).await,
        };

        match result {
            Ok(response) => {
                self.middleware
                    .process_response(MiddlewareType::Download)
                    .await?;
                self.scheduler.ack().await?;
                Ok(Some(response))
            }
            Err(error) => {
                self.middleware
                    .process_exception(MiddlewareType::Download, &error)
                    .await?;
                self.scheduler.nack().await?;
                Err(error)
            }
        }
    }

    pub async fn execute_spider_once<P>(
        &mut self,
        spider: &P,
        compiled: Option<&Compiled>,
    ) -> Result<Option<SpiderOutput>, SpiderError>
    where
        P: Spider,
    {
        self.refresh_spider_middleware(spider)?;

        let Some(task) = self.scheduler.lease().await? else {
            return Ok(None);
        };

        self.process_request(MiddlewareType::Download).await?;

        let response = match task.request.mode {
            RequestMode::Http => self.http.fetch(&task.request).await,
            RequestMode::Browser => self.browser.fetch(&task.request).await,
        };

        match response {
            Ok(response) => {
                self.process_response(MiddlewareType::Download).await?;
                self.process_request(MiddlewareType::Spider).await?;
                let output = spider.dispatch(&response, compiled).await;
                match output {
                    Ok(output) => {
                        self.process_response(MiddlewareType::Spider).await?;
                        for request in output.requests.iter().cloned() {
                            self.scheduler
                                .enqueue(crate::scheduler::types::ScheduledTask { request })
                                .await?;
                        }
                        self.scheduler.ack().await?;
                        Ok(Some(output))
                    }
                    Err(error) => {
                        self.process_exception(MiddlewareType::Spider, &error).await?;
                        self.scheduler.nack().await?;
                        Err(error)
                    }
                }
            }
            Err(error) => {
                self.process_exception(MiddlewareType::Download, &error).await?;
                self.scheduler.nack().await?;
                Err(error)
            }
        }
    }

    fn refresh_spider_middleware<P: Spider>(&mut self, spider: &P) -> Result<(), SpiderError> {
        let defaults = compile_runtime(&spider.runtime())?;
        let merged = merge_middleware(defaults, spider.middlewares());
        self.spider_middleware = build_middleware(&merged)?;
        Ok(())
    }

    async fn process_request(&self, kind: MiddlewareType) -> Result<(), SpiderError> {
        self.middleware.process_request(kind).await?;
        self.spider_middleware.process_request(kind).await?;
        Ok(())
    }

    async fn process_response(&self, kind: MiddlewareType) -> Result<(), SpiderError> {
        self.middleware.process_response(kind).await?;
        self.spider_middleware.process_response(kind).await?;
        Ok(())
    }

    async fn process_exception(
        &self,
        kind: MiddlewareType,
        error: &SpiderError,
    ) -> Result<(), SpiderError> {
        self.middleware.process_exception(kind, error).await?;
        self.spider_middleware.process_exception(kind, error).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::{BrowserDownloader, HttpDownloader};
    use crate::middleware::traits::Middleware;
    use crate::middleware::types::MiddlewareConfig;
    use crate::request::Request;
    use crate::rules::compile::compile_rules;
    use crate::scheduler::memory::MemoryScheduler;
    use crate::scheduler::traits::Scheduler;
    use crate::scheduler::types::ScheduledTask;
    use crate::spider::{Output as SpiderOutput, Spider};
    use crate::value::Value;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn engine_executes_http_task_once() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask {
            request: Request::new("https://example.com"),
        }))
        .unwrap();

        let mut engine = Engine::new(scheduler, HttpDownloader, BrowserDownloader);
        let response = block_on(engine.execute_once()).unwrap().unwrap();

        assert_eq!(response.url, "https://example.com");
        assert_eq!(response.protocol.as_deref(), Some("HTTP/1.1"));
    }

    #[test]
    fn engine_executes_browser_task_once() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask {
            request: Request::browser("https://example.com/browser"),
        }))
        .unwrap();

        let mut engine = Engine::new(scheduler, HttpDownloader, BrowserDownloader);
        let response = block_on(engine.execute_once()).unwrap().unwrap();

        assert_eq!(response.url, "https://example.com/browser");
        assert_eq!(response.protocol.as_deref(), Some("browser"));
    }

    #[test]
    fn engine_runs_download_middlewares_around_fetch() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask {
            request: Request::new("https://example.com"),
        }))
        .unwrap();

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut middleware = MiddlewareChain::default();
        middleware.push(
            "recorder",
            MiddlewareConfig {
                enabled: true,
                r#type: MiddlewareType::Download,
                order: 100,
                options: BTreeMap::<String, Value>::new(),
            },
            Box::new(RecordMiddleware { log: log.clone() }),
        );

        let mut engine = Engine::new(scheduler, HttpDownloader, BrowserDownloader)
            .with_middleware(middleware);
        block_on(engine.execute_once()).unwrap();

        assert_eq!(
            *log.lock().unwrap(),
            vec!["request".to_string(), "response".to_string()]
        );
    }

    #[test]
    fn engine_executes_dsl_step_after_download() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask {
            request: Request::new("https://example.com"),
        }))
        .unwrap();

        let compiled = compile_rules(Value::String(
            r#"{
                "steps":[
                    {
                        "id":"parse",
                        "impl":"dsl",
                        "parse":{
                            "fields":[
                                {
                                    "name":"title",
                                    "source":"html",
                                    "selector_type":"css",
                                    "selector":["h1.title"],
                                    "attribute":"text"
                                }
                            ]
                        }
                    }
                ]
            }"#
            .to_string(),
        ))
        .unwrap();

        let mut engine = Engine::new(scheduler, HtmlHttpDownloader, BrowserDownloader);
        let output = block_on(engine.execute_spider_once(&TestSpider, Some(&compiled)))
            .unwrap()
            .unwrap();

        assert_eq!(
            output.items[0].get("title"),
            Some(&Value::String("Hello".to_string()))
        );
    }

    #[test]
    fn engine_enqueues_follow_request_and_runs_next_step() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask {
            request: Request::new("https://example.com/list"),
        }))
        .unwrap();

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
                                    "to":{"next_step":"detail"}
                                }
                            ]
                        }
                    },
                    {
                        "id":"detail",
                        "impl":"code",
                        "callback":"parse_detail"
                    }
                ]
            }"#
            .to_string(),
        ))
        .unwrap();

        let mut engine = Engine::new(scheduler, FlowHttpDownloader, BrowserDownloader);
        let first = block_on(engine.execute_spider_once(&FlowSpider, Some(&compiled)))
            .unwrap()
            .unwrap();
        let second = block_on(engine.execute_spider_once(&FlowSpider, Some(&compiled)))
            .unwrap()
            .unwrap();

        assert_eq!(first.requests.len(), 1);
        assert_eq!(first.requests[0].url, "https://example.com/detail/1");
        assert_eq!(
            first.requests[0].meta.get("next_step"),
            Some(&Value::String("detail".to_string()))
        );
        assert_eq!(
            second.items[0].get("detail"),
            Some(&Value::String("https://example.com/detail/1".to_string()))
        );
    }

    #[test]
    fn engine_loads_runtime_middlewares_and_applies_explicit_overrides() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask {
            request: Request::new("https://example.com"),
        }))
        .unwrap();

        let mut engine = Engine::new(scheduler, HtmlHttpDownloader, BrowserDownloader);
        block_on(engine.execute_spider_once(&RuntimeSpider, None))
            .unwrap()
            .unwrap();

        let keys = engine
            .spider_middleware
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>();

        assert!(keys.contains(&"retry_by_error"));
        assert!(keys.contains(&"interval_gate"));
        assert!(keys.contains(&"rate_limit"));
        assert!(keys.contains(&"dedup"));

        let dedup = engine
            .spider_middleware
            .entries
            .iter()
            .find(|entry| entry.key == "dedup")
            .unwrap();
        assert!(!dedup.config.enabled);
        assert_eq!(dedup.config.order, 999);
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut future = Pin::from(Box::new(future));
        let mut context = Context::from_waker(&waker);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    struct RecordMiddleware {
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Middleware for RecordMiddleware {
        fn process_request<'a>(&'a self) -> crate::future::BoxFuture<'a, Result<(), SpiderError>> {
            Box::pin(async move {
                self.log.lock().unwrap().push("request".to_string());
                Ok(())
            })
        }

        fn process_response<'a>(&'a self) -> crate::future::BoxFuture<'a, Result<(), SpiderError>> {
            Box::pin(async move {
                self.log.lock().unwrap().push("response".to_string());
                Ok(())
            })
        }
    }

    struct HtmlHttpDownloader;

    impl crate::downloader::traits::Downloader for HtmlHttpDownloader {
        fn fetch<'a>(
            &'a self,
            request: &'a Request,
        ) -> crate::future::BoxFuture<'a, Result<Response, SpiderError>> {
            Box::pin(async move {
                Ok(Response::from_request(
                    request.clone(),
                    200,
                    Default::default(),
                    br#"<h1 class="title">Hello</h1>"#.to_vec(),
                ))
            })
        }
    }

    struct FlowHttpDownloader;

    impl crate::downloader::traits::Downloader for FlowHttpDownloader {
        fn fetch<'a>(
            &'a self,
            request: &'a Request,
        ) -> crate::future::BoxFuture<'a, Result<Response, SpiderError>> {
            Box::pin(async move {
                let body = if request.url.ends_with("/list") {
                    br#"<a class="detail" href="https://example.com/detail/1">1</a>"#.to_vec()
                } else {
                    b"<h1>detail</h1>".to_vec()
                };

                Ok(Response::from_request(
                    request.clone(),
                    200,
                    Default::default(),
                    body,
                ))
            })
        }
    }

    struct TestSpider;

    impl Spider for TestSpider {
        fn name(&self) -> &str {
            "test"
        }

        fn parse<'a>(
            &'a self,
            _response: &'a Response,
        ) -> crate::future::BoxFuture<'a, Result<SpiderOutput, SpiderError>> {
            Box::pin(async { Ok(SpiderOutput::empty()) })
        }
    }

    struct FlowSpider;

    impl Spider for FlowSpider {
        fn name(&self) -> &str {
            "flow"
        }

        fn parse<'a>(
            &'a self,
            _response: &'a Response,
        ) -> crate::future::BoxFuture<'a, Result<SpiderOutput, SpiderError>> {
            Box::pin(async { Ok(SpiderOutput::empty()) })
        }

        fn call<'a>(
            &'a self,
            name: &'a str,
            response: &'a Response,
        ) -> crate::future::BoxFuture<'a, Result<SpiderOutput, SpiderError>> {
            match name {
                "parse" => self.parse(response),
                "parse_detail" => Box::pin(async move {
                    Ok(SpiderOutput {
                        items: vec![crate::item::Item::new().with_field(
                            "detail",
                            Value::String(response.url.clone()),
                        )],
                        requests: Vec::new(),
                    })
                }),
                other => Box::pin(async move {
                    Err(SpiderError::engine(format!("unknown callback: {other}")))
                }),
            }
        }
    }

    struct RuntimeSpider;

    impl Spider for RuntimeSpider {
        fn name(&self) -> &str {
            "runtime"
        }

        fn runtime(&self) -> crate::runtime::Config {
            crate::runtime::Config {
                schedule: [
                    ("interval_ms".to_string(), Value::Number(1000.0)),
                    ("rate_per_minute".to_string(), Value::Number(60.0)),
                ]
                .into_iter()
                .collect(),
                retry: [("count".to_string(), Value::Number(3.0))]
                    .into_iter()
                    .collect(),
                dedup: [("enabled".to_string(), Value::Bool(true))]
                    .into_iter()
                    .collect(),
            }
        }

        fn middlewares(&self) -> crate::middleware::Map {
            [(
                "dedup".to_string(),
                MiddlewareConfig {
                    enabled: false,
                    r#type: MiddlewareType::Download,
                    order: 999,
                    options: BTreeMap::new(),
                },
            )]
            .into_iter()
            .collect()
        }

        fn parse<'a>(
            &'a self,
            _response: &'a Response,
        ) -> crate::future::BoxFuture<'a, Result<SpiderOutput, SpiderError>> {
            Box::pin(async { Ok(SpiderOutput::empty()) })
        }
    }
}
