pub mod context;
pub mod types;

use crate::download::traits::Downloader;
use crate::engine::context::EngineContext;
use crate::engine::types::Flow;
use crate::error::SpiderError;
use crate::middleware::{build as build_middleware, MiddlewareChain, MiddlewareType};
use crate::request::RequestMode;
use crate::response::Response;
use crate::rules::Compiled;
use crate::runtime::compile::{compile as compile_runtime, merge as merge_middleware};
use crate::runtime::{merge as merge_runtime, Config as RuntimeConfig};
use crate::scheduler::traits::Scheduler;
use crate::spider::{Output as SpiderOutput, Spider};
use std::collections::BTreeMap;

pub struct Engine<S, H, B> {
    pub scheduler: S,
    pub http: H,
    pub browser: B,
    pub middleware: MiddlewareChain,
    pub spider_middleware: MiddlewareChain,
    step_middlewares: BTreeMap<String, MiddlewareChain>,
    active_spider: Option<String>,
    active_step: String,
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
            step_middlewares: BTreeMap::new(),
            active_spider: None,
            active_step: "parse".to_string(),
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
        let mut context = EngineContext::new(task.request);

        let flow = self.process_request(MiddlewareType::Download, &mut context).await?;
        if self.apply_flow(flow, &context).await? {
            return Ok(None);
        }

        let result = match context.request.mode {
            RequestMode::Http => self.http.fetch(&context.request).await,
            RequestMode::Browser => self.browser.fetch(&context.request).await,
        };

        match result {
            Ok(response) => {
                context.response = Some(response.clone());
                let flow = self
                    .process_response(MiddlewareType::Download, &mut context)
                    .await?;
                if self.apply_flow(flow, &context).await? {
                    return Ok(None);
                }
                self.scheduler.ack().await?;
                Ok(Some(response))
            }
            Err(error) => {
                let flow = self
                    .process_exception(MiddlewareType::Download, &mut context, &error)
                    .await?;
                if self.apply_error_flow(flow, &context).await? {
                    return Ok(None);
                }
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
        let Some(task) = self.scheduler.lease().await? else {
            return Ok(None);
        };
        let step_id = step_id_from_request(&task.request);
        self.prepare_spider_middleware(spider, compiled, &step_id)?;
        let mut context = EngineContext::new(task.request);

        let flow = self.process_request(MiddlewareType::Download, &mut context).await?;
        if self.apply_flow(flow, &context).await? {
            return Ok(None);
        }

        let response = match context.request.mode {
            RequestMode::Http => self.http.fetch(&context.request).await,
            RequestMode::Browser => self.browser.fetch(&context.request).await,
        };

        match response {
            Ok(response) => {
                context.response = Some(response.clone());

                let flow = self
                    .process_response(MiddlewareType::Download, &mut context)
                    .await?;
                if self.apply_flow(flow, &context).await? {
                    return Ok(None);
                }

                let flow = self.process_request(MiddlewareType::Spider, &mut context).await?;
                if self.apply_flow(flow, &context).await? {
                    return Ok(None);
                }

                let output = spider.dispatch(&response, compiled).await;
                match output {
                    Ok(output) => {
                        let flow = self
                            .process_response(MiddlewareType::Spider, &mut context)
                            .await?;
                        if self.apply_flow(flow, &context).await? {
                            return Ok(None);
                        }
                        for request in output.requests.iter().cloned() {
                            self.scheduler
                                .enqueue(crate::scheduler::types::ScheduledTask::new(request))
                                .await?;
                        }
                        self.scheduler.ack().await?;
                        Ok(Some(output))
                    }
                    Err(error) => {
                        let flow = self
                            .process_exception(MiddlewareType::Spider, &mut context, &error)
                            .await?;
                        if self.apply_error_flow(flow, &context).await? {
                            return Ok(None);
                        }
                        self.scheduler.nack().await?;
                        Err(error)
                    }
                }
            }
            Err(error) => {
                let flow = self
                    .process_exception(MiddlewareType::Download, &mut context, &error)
                    .await?;
                if self.apply_error_flow(flow, &context).await? {
                    return Ok(None);
                }
                self.scheduler.nack().await?;
                Err(error)
            }
        }
    }

    fn refresh_spider_middleware<P: Spider>(&mut self, spider: &P) -> Result<(), SpiderError> {
        if self.active_spider.as_deref() != Some(spider.name()) {
            self.spider_middleware = MiddlewareChain::default();
            self.step_middlewares.clear();
            self.active_spider = Some(spider.name().to_string());
        }

        Ok(())
    }

    fn prepare_spider_middleware<P: Spider>(
        &mut self,
        spider: &P,
        compiled: Option<&Compiled>,
        step_id: &str,
    ) -> Result<(), SpiderError> {
        self.refresh_spider_middleware(spider)?;

        if !self.step_middlewares.contains_key(step_id) {
            let runtime = effective_runtime(spider.runtime(), compiled, step_id)?;
            let defaults = compile_runtime(&runtime)?;
            let merged = merge_middleware(defaults, spider.middlewares());
            let middleware = build_middleware(&merged)?;
            self.step_middlewares.insert(step_id.to_string(), middleware);
        }

        self.active_step = step_id.to_string();
        Ok(())
    }

    async fn process_request(
        &self,
        kind: MiddlewareType,
        context: &mut EngineContext,
    ) -> Result<Flow, SpiderError> {
        let flow = self.middleware.process_request(kind, context).await?;
        if !matches!(flow, Flow::Continue) {
            return Ok(flow);
        }
        self.active_spider_middleware().process_request(kind, context).await
    }

    async fn process_response(
        &self,
        kind: MiddlewareType,
        context: &mut EngineContext,
    ) -> Result<Flow, SpiderError> {
        let flow = self.middleware.process_response(kind, context).await?;
        if !matches!(flow, Flow::Continue) {
            return Ok(flow);
        }
        self.active_spider_middleware()
            .process_response(kind, context)
            .await
    }

    async fn process_exception(
        &self,
        kind: MiddlewareType,
        context: &mut EngineContext,
        error: &SpiderError,
    ) -> Result<Flow, SpiderError> {
        let flow = self.middleware.process_exception(kind, context, error).await?;
        if !matches!(flow, Flow::Continue) {
            return Ok(flow);
        }
        self.active_spider_middleware()
            .process_exception(kind, context, error)
            .await
    }

    fn active_spider_middleware(&self) -> &MiddlewareChain {
        self.step_middlewares
            .get(&self.active_step)
            .unwrap_or(&self.spider_middleware)
    }

    async fn apply_flow(
        &mut self,
        flow: Flow,
        context: &EngineContext,
    ) -> Result<bool, SpiderError> {
        match flow {
            Flow::Continue => Ok(false),
            Flow::Drop(_) => {
                self.scheduler.ack().await?;
                Ok(true)
            }
            Flow::Retry { .. } => {
                self.enqueue_retry_request(context, flow).await?;
                self.scheduler.ack().await?;
                Ok(true)
            }
        }
    }

    async fn apply_error_flow(
        &mut self,
        flow: Flow,
        context: &EngineContext,
    ) -> Result<bool, SpiderError> {
        match flow {
            Flow::Continue => Ok(false),
            Flow::Drop(_) => {
                self.scheduler.ack().await?;
                Ok(true)
            }
            Flow::Retry { .. } => {
                self.enqueue_retry_request(context, flow).await?;
                self.scheduler.ack().await?;
                Ok(true)
            }
        }
    }

    async fn enqueue_retry_request(
        &mut self,
        context: &EngineContext,
        flow: Flow,
    ) -> Result<(), SpiderError> {
        let Flow::Retry { reason, backoff_ms } = flow else {
            return Ok(());
        };

        let retries = context
            .request
            .meta
            .get("_retry_times")
            .and_then(crate::value::Value::as_f64)
            .unwrap_or(0.0)
            + 1.0;

        let mut request = context.request.clone();
        request.dont_filter = true;
        request.meta.insert(
            "_retry_times".to_string(),
            crate::value::Value::Number(retries),
        );
        request.meta.insert(
            "_retry_reason".to_string(),
            crate::value::Value::String(reason),
        );
        if let Some(backoff_ms) = backoff_ms {
            request.meta.insert(
                "_retry_backoff_ms".to_string(),
                crate::value::Value::Number(backoff_ms as f64),
            );
        }

        let task = match backoff_ms {
            Some(backoff_ms) if backoff_ms > 0 => {
                crate::scheduler::types::ScheduledTask::with_delay_ms(request, backoff_ms)
            }
            _ => crate::scheduler::types::ScheduledTask::new(request),
        };

        self.scheduler.enqueue(task).await
    }
}

fn step_id_from_request(request: &crate::request::Request) -> String {
    request
        .meta
        .get("next_step")
        .and_then(crate::value::Value::as_str)
        .unwrap_or("parse")
        .to_string()
}

fn effective_runtime(
    spider_runtime: RuntimeConfig,
    compiled: Option<&Compiled>,
    step_id: &str,
) -> Result<RuntimeConfig, SpiderError> {
    let Some(compiled) = compiled else {
        return Ok(spider_runtime);
    };

    let step = compiled
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .ok_or_else(|| SpiderError::engine(format!("step not found: {step_id}")))?;

    Ok(merge_runtime(&spider_runtime, &step.runtime))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::{BrowserDownloader, HttpDownloader};
    use crate::engine::context::EngineContext;
    use crate::engine::types::Flow;
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
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com",
        ))))
        .unwrap();

        let mut engine = Engine::new(scheduler, HttpDownloader, BrowserDownloader);
        let response = block_on(engine.execute_once()).unwrap().unwrap();

        assert_eq!(response.url, "https://example.com");
        assert_eq!(response.protocol.as_deref(), Some("HTTP/1.1"));
    }

    #[test]
    fn engine_executes_browser_task_once() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::browser(
            "https://example.com/browser",
        ))))
        .unwrap();

        let mut engine = Engine::new(scheduler, HttpDownloader, BrowserDownloader);
        let response = block_on(engine.execute_once()).unwrap().unwrap();

        assert_eq!(response.url, "https://example.com/browser");
        assert_eq!(response.protocol.as_deref(), Some("browser"));
    }

    #[test]
    fn engine_runs_download_middlewares_around_fetch() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com",
        ))))
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
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com",
        ))))
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
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/list",
        ))))
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
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com",
        ))))
        .unwrap();

        let mut engine = Engine::new(scheduler, HtmlHttpDownloader, BrowserDownloader);
        block_on(engine.execute_spider_once(&RuntimeSpider, None))
            .unwrap()
            .unwrap();

        let keys = engine
            .step_middlewares
            .get("parse")
            .unwrap()
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>();

        assert!(keys.contains(&"retry_by_error"));
        assert!(keys.contains(&"interval_gate"));
        assert!(keys.contains(&"rate_limit"));
        assert!(keys.contains(&"dedup"));

        let dedup = engine
            .step_middlewares
            .get("parse")
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.key == "dedup")
            .unwrap();
        assert!(!dedup.config.enabled);
        assert_eq!(dedup.config.order, 999);
    }

    #[test]
    fn engine_dedups_duplicate_requests_before_fetch() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/dedup",
        ))))
        .unwrap();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/dedup",
        ))))
        .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttpDownloader {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::new(scheduler, downloader, BrowserDownloader);

        let first = block_on(engine.execute_spider_once(&DedupSpider, None)).unwrap();
        let second = block_on(engine.execute_spider_once(&DedupSpider, None)).unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(*fetches.lock().unwrap(), 1);
    }

    #[test]
    fn engine_retries_on_configured_status() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/retry",
        ))))
        .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttpDownloader {
            fetches: fetches.clone(),
            statuses: vec![500, 200],
        };
        let mut engine = Engine::new(scheduler, downloader, BrowserDownloader);

        let first = block_on(engine.execute_spider_once(&RetrySpider, None)).unwrap();
        let second = block_on(engine.execute_spider_once(&RetrySpider, None)).unwrap();

        assert!(first.is_none());
        assert!(second.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_respects_retry_backoff_delay() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/retry-backoff",
        ))))
        .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttpDownloader {
            fetches: fetches.clone(),
            statuses: vec![500, 200],
        };
        let mut engine = Engine::new(scheduler, downloader, BrowserDownloader);

        let first = block_on(engine.execute_spider_once(&RetryBackoffSpider, None)).unwrap();
        let second = block_on(engine.execute_spider_once(&RetryBackoffSpider, None)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
        let third = block_on(engine.execute_spider_once(&RetryBackoffSpider, None)).unwrap();

        assert!(first.is_none());
        assert!(second.is_none());
        assert!(third.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_respects_interval_gate_delay() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/interval/1",
        ))))
        .unwrap();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/interval/2",
        ))))
        .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttpDownloader {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::new(scheduler, downloader, BrowserDownloader);

        let first = block_on(engine.execute_spider_once(&IntervalSpider, None)).unwrap();
        let second = block_on(engine.execute_spider_once(&IntervalSpider, None)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
        let third = block_on(engine.execute_spider_once(&IntervalSpider, None)).unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert!(third.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_respects_rate_limit_delay() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/rate/1",
        ))))
        .unwrap();
        block_on(scheduler.enqueue(ScheduledTask::new(Request::new(
            "https://example.com/rate/2",
        ))))
        .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttpDownloader {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::new(scheduler, downloader, BrowserDownloader);

        let first = block_on(engine.execute_spider_once(&RateLimitSpider, None)).unwrap();
        let second = block_on(engine.execute_spider_once(&RateLimitSpider, None)).unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(*fetches.lock().unwrap(), 1);
    }

    #[test]
    fn engine_applies_step_runtime_before_download() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(
            Request::new("https://example.com/detail/1")
                .with_meta("next_step", Value::String("detail".to_string())),
        )))
        .unwrap();
        block_on(scheduler.enqueue(ScheduledTask::new(
            Request::new("https://example.com/detail/2")
                .with_meta("next_step", Value::String("detail".to_string())),
        )))
        .unwrap();

        let compiled = compile_rules(Value::String(
            r#"{
                "steps":[
                    {
                        "id":"detail",
                        "impl":"code",
                        "callback":"parse_detail",
                        "runtime":{
                            "schedule":{"interval_ms":10}
                        }
                    }
                ]
            }"#
            .to_string(),
        ))
        .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttpDownloader {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::new(scheduler, downloader, BrowserDownloader);

        let first = block_on(engine.execute_spider_once(&FlowSpider, Some(&compiled))).unwrap();
        let second = block_on(engine.execute_spider_once(&FlowSpider, Some(&compiled))).unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(*fetches.lock().unwrap(), 1);
        assert!(engine.step_middlewares.contains_key("detail"));
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
        fn process_request<'a>(
            &'a self,
            _context: &'a mut EngineContext,
        ) -> crate::future::BoxFuture<'a, Result<Flow, SpiderError>> {
            Box::pin(async move {
                self.log.lock().unwrap().push("request".to_string());
                Ok(Flow::Continue)
            })
        }

        fn process_response<'a>(
            &'a self,
            _context: &'a mut EngineContext,
        ) -> crate::future::BoxFuture<'a, Result<Flow, SpiderError>> {
            Box::pin(async move {
                self.log.lock().unwrap().push("response".to_string());
                Ok(Flow::Continue)
            })
        }
    }

    struct HtmlHttpDownloader;

    impl crate::download::traits::Downloader for HtmlHttpDownloader {
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

    impl crate::download::traits::Downloader for FlowHttpDownloader {
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

    struct CountHttpDownloader {
        fetches: Arc<Mutex<usize>>,
        statuses: Vec<u16>,
    }

    impl crate::download::traits::Downloader for CountHttpDownloader {
        fn fetch<'a>(
            &'a self,
            request: &'a Request,
        ) -> crate::future::BoxFuture<'a, Result<Response, SpiderError>> {
            Box::pin(async move {
                let mut fetches = self.fetches.lock().unwrap();
                let index = *fetches;
                *fetches += 1;
                let status = self.statuses.get(index).copied().unwrap_or(200);

                Ok(Response::from_request(
                    request.clone(),
                    status,
                    Default::default(),
                    Vec::new(),
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

    struct DedupSpider;

    impl Spider for DedupSpider {
        fn name(&self) -> &str {
            "dedup"
        }

        fn runtime(&self) -> crate::runtime::Config {
            crate::runtime::Config {
                schedule: BTreeMap::new(),
                retry: BTreeMap::new(),
                dedup: [
                    ("enabled".to_string(), Value::Bool(true)),
                    ("key".to_string(), Value::String("url".to_string())),
                ]
                .into_iter()
                .collect(),
            }
        }

        fn parse<'a>(
            &'a self,
            _response: &'a Response,
        ) -> crate::future::BoxFuture<'a, Result<SpiderOutput, SpiderError>> {
            Box::pin(async { Ok(SpiderOutput::empty()) })
        }
    }

    struct RetrySpider;

    impl Spider for RetrySpider {
        fn name(&self) -> &str {
            "retry"
        }

        fn runtime(&self) -> crate::runtime::Config {
            crate::runtime::Config {
                schedule: BTreeMap::new(),
                retry: [
                    ("count".to_string(), Value::Number(1.0)),
                    (
                        "http_status".to_string(),
                        Value::Array(vec![Value::Number(500.0)]),
                    ),
                ]
                .into_iter()
                .collect(),
                dedup: BTreeMap::new(),
            }
        }

        fn parse<'a>(
            &'a self,
            _response: &'a Response,
        ) -> crate::future::BoxFuture<'a, Result<SpiderOutput, SpiderError>> {
            Box::pin(async { Ok(SpiderOutput::empty()) })
        }
    }

    struct RetryBackoffSpider;

    impl Spider for RetryBackoffSpider {
        fn name(&self) -> &str {
            "retry_backoff"
        }

        fn runtime(&self) -> crate::runtime::Config {
            crate::runtime::Config {
                schedule: BTreeMap::new(),
                retry: [
                    ("count".to_string(), Value::Number(1.0)),
                    (
                        "http_status".to_string(),
                        Value::Array(vec![Value::Number(500.0)]),
                    ),
                    (
                        "backoff_ms".to_string(),
                        Value::Array(vec![Value::Number(10.0)]),
                    ),
                ]
                .into_iter()
                .collect(),
                dedup: BTreeMap::new(),
            }
        }

        fn parse<'a>(
            &'a self,
            _response: &'a Response,
        ) -> crate::future::BoxFuture<'a, Result<SpiderOutput, SpiderError>> {
            Box::pin(async { Ok(SpiderOutput::empty()) })
        }
    }

    struct IntervalSpider;

    impl Spider for IntervalSpider {
        fn name(&self) -> &str {
            "interval"
        }

        fn runtime(&self) -> crate::runtime::Config {
            crate::runtime::Config {
                schedule: [("interval_ms".to_string(), Value::Number(10.0))]
                    .into_iter()
                    .collect(),
                retry: BTreeMap::new(),
                dedup: BTreeMap::new(),
            }
        }

        fn parse<'a>(
            &'a self,
            _response: &'a Response,
        ) -> crate::future::BoxFuture<'a, Result<SpiderOutput, SpiderError>> {
            Box::pin(async { Ok(SpiderOutput::empty()) })
        }
    }

    struct RateLimitSpider;

    impl Spider for RateLimitSpider {
        fn name(&self) -> &str {
            "rate_limit"
        }

        fn runtime(&self) -> crate::runtime::Config {
            crate::runtime::Config {
                schedule: [("rate_per_minute".to_string(), Value::Number(1.0))]
                    .into_iter()
                    .collect(),
                retry: BTreeMap::new(),
                dedup: BTreeMap::new(),
            }
        }

        fn parse<'a>(
            &'a self,
            _response: &'a Response,
        ) -> crate::future::BoxFuture<'a, Result<SpiderOutput, SpiderError>> {
            Box::pin(async { Ok(SpiderOutput::empty()) })
        }
    }
}
