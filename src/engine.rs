pub mod context;
pub mod types;

use crate::download::traits::Downloader;
use crate::engine::context::EngineContext;
use crate::engine::types::Flow;
use crate::error::SpiderError;
use crate::middleware::{build as build_middleware, FactoryRegistry, MiddlewareChain, MiddlewareType};
use crate::request::RequestMode;
use crate::rules::Compiled;
use crate::runtime::compile::{compile as compile_runtime, merge as merge_middleware};
use crate::runtime::{merge as merge_runtime, Config as RuntimeConfig};
use crate::scheduler::traits::Scheduler;
use crate::settings::Settings;
use crate::spider::{Output as SpiderOutput, Spider};
use futures::stream::{FuturesUnordered, StreamExt};
use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct Engine<S, H, B, P = ()> {
    pub scheduler: S,
    pub http: H,
    pub browser: B,
    pub pipeline: P,
    pub settings: Settings,
    pub middleware: MiddlewareChain,
    pub spider_middleware: MiddlewareChain,
    pub custom_factories: FactoryRegistry,
    step_middlewares: BTreeMap<String, MiddlewareChain>,
    active_spider: Option<String>,
    active_step: String,
    allowed_domains: Vec<String>,
    shutdown: Arc<AtomicBool>,
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
            pipeline: (),
            settings: Settings::default(),
            middleware: MiddlewareChain::default(),
            spider_middleware: MiddlewareChain::default(),
            custom_factories: FactoryRegistry::new(),
            step_middlewares: BTreeMap::new(),
            active_spider: None,
            active_step: "parse".to_string(),
            allowed_domains: Vec::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl<S, H, B, P> Engine<S, H, B, P>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
{
    pub fn with_settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    pub fn with_middleware(mut self, middleware: MiddlewareChain) -> Self {
        self.middleware = middleware;
        self
    }

    pub fn with_pipeline<P2: crate::pipeline::Pipeline>(self, pipeline: P2) -> Engine<S, H, B, P2> {
        Engine {
            scheduler: self.scheduler,
            http: self.http,
            browser: self.browser,
            pipeline,
            settings: self.settings,
            middleware: self.middleware,
            spider_middleware: self.spider_middleware,
            custom_factories: self.custom_factories,
            step_middlewares: self.step_middlewares,
            active_spider: self.active_spider,
            active_step: self.active_step,
            allowed_domains: self.allowed_domains,
            shutdown: self.shutdown,
        }
    }

    /// 直接注册一个自定义中间件实例到引擎级中间件链。
    ///
    /// 这个中间件对所有请求/响应生效。
    ///
    /// ```ignore
    /// engine.add_middleware(
    ///     "custom_ua",
    ///     MiddlewareConfig { enabled: true, r#type: MiddlewareType::Download, order: 50, .. },
    ///     Box::new(MyUaMiddleware),
    /// );
    /// ```
    pub fn add_middleware(
        mut self,
        key: impl Into<String>,
        config: crate::middleware::MiddlewareConfig,
        middleware: Box<dyn crate::middleware::Middleware>,
    ) -> Self {
        self.middleware.push(key, config, middleware);
        self
    }

    /// 注册一个自定义中间件工厂。
    ///
    /// 注册后，可以在 `Settings::middlewares` 或 DSL 规则的 `MIDDLEWARES` 中
    /// 用同名 key 引用，引擎会自动调用工厂创建实例。
    ///
    /// ```ignore
    /// engine.register_middleware("custom_ua", |options| {
    ///     Ok(Box::new(MyUaMiddleware::new(options)))
    /// });
    /// ```
    pub fn register_middleware(
        mut self,
        key: impl Into<String>,
        factory: impl Fn(&std::collections::BTreeMap<String, crate::value::Value>) -> Result<Box<dyn crate::middleware::Middleware>, SpiderError> + Send + Sync + 'static,
    ) -> Self {
        self.custom_factories.register(key, factory);
        self
    }

    /// 获取一个可 Clone 的停止句柄。
    ///
    /// 典型用法：
    /// ```ignore
    /// let handle = engine.shutdown_handle();
    /// tokio::spawn(async move {
    ///     tokio::signal::ctrl_c().await.ok();
    ///     handle.stop();
    /// });
    /// engine.run(&spider).await?;
    /// ```
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            flag: self.shutdown.clone(),
        }
    }

    /// 启动引擎，**持续运行**直到收到 stop 信号。
    ///
    /// 支持并发下载：
    /// - `settings.concurrent_requests` 控制全局并发上限（默认 16）
    /// - `settings.concurrent_requests_per_domain` 控制每域名并发上限（默认 8）
    ///
    /// 引擎不会因为队列为空而自动退出。只有两种方式退出：
    /// 1. 调用 `engine.stop()` 或 `shutdown_handle().stop()`
    /// 2. Ctrl+C（配合 tokio::signal 调 stop）
    pub async fn run<Sp: Spider>(
        &mut self,
        spider: &Sp,
    ) -> Result<Vec<SpiderOutput>, SpiderError> {
        let spider_name = spider.name();
        tracing::info!(spider = spider_name, "引擎启动");

        self.allowed_domains = spider.allowed_domains();
        if !self.allowed_domains.is_empty() {
            tracing::info!(spider = spider_name, domains = ?self.allowed_domains, "域名过滤已启用");
        }

        self.pipeline.open(spider_name).await?;

        let compiled = match spider.rules() {
            Some(config) => {
                tracing::info!(spider = spider_name, "加载 DSL 规则");
                Some(crate::rules::load(&config).await?)
            }
            None => None,
        };

        self.prepare_all_step_middlewares(spider, compiled.as_ref())?;

        let start_urls = spider.start_urls();
        tracing::info!(spider = spider_name, count = start_urls.len(), "入队起始 URL");
        for url in start_urls {
            let request = crate::request::Request::new(url);
            self.scheduler
                .enqueue(crate::scheduler::types::ScheduledTask::new(request))
                .await?;
        }

        let max_concurrent = self.settings.concurrent_requests;
        let per_domain_limit = self.settings.concurrent_requests_per_domain;
        let idle_timeout = self.settings.idle_timeout;

        tracing::info!(
            spider = spider_name,
            concurrent = max_concurrent,
            per_domain = per_domain_limit,
            "并发配置"
        );

        let global_sem = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let mut domain_sems: BTreeMap<String, Arc<tokio::sync::Semaphore>> = BTreeMap::new();

        let default_step_mw = MiddlewareChain::default();

        type TaskFuture<'a> = Pin<Box<dyn std::future::Future<Output = TaskResult> + 'a>>;
        let mut inflight: FuturesUnordered<TaskFuture<'_>> = FuturesUnordered::new();
        let mut outputs = Vec::new();
        let mut round = 0usize;

        let scheduler = &mut self.scheduler;
        let http = &self.http;
        let browser = &self.browser;
        let pipeline = &self.pipeline;
        let engine_mw = &self.middleware;
        let step_mws = &self.step_middlewares;
        let allowed_domains = &self.allowed_domains;
        let shutdown = &self.shutdown;

        loop {
            if shutdown.load(Ordering::Relaxed) {
                tracing::info!(spider = spider_name, "收到 stop 信号，等待 {} 个进行中任务完成...", inflight.len());
                while let Some(result) = inflight.next().await {
                    handle_task_result(result, scheduler, allowed_domains, &mut outputs, &mut round, spider_name).await?;
                }
                break;
            }

            while inflight.len() < max_concurrent {
                let Ok(global_permit) = global_sem.clone().try_acquire_owned() else {
                    break;
                };
                let Some(task) = scheduler.lease().await? else {
                    drop(global_permit);
                    break;
                };

                let domain = extract_domain(&task.request.url)
                    .unwrap_or("unknown")
                    .to_string();
                let domain_sem = domain_sems
                    .entry(domain)
                    .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(per_domain_limit)))
                    .clone();

                let step_id = step_id_from_request(&task.request);
                let step_mw = step_mws.get(&step_id).unwrap_or(&default_step_mw);

                inflight.push(Box::pin(execute_task(
                    task.request,
                    http,
                    browser,
                    pipeline,
                    engine_mw,
                    step_mw,
                    spider,
                    compiled.as_ref(),
                    allowed_domains,
                    spider_name,
                    global_permit,
                    domain_sem,
                )));
            }

            if inflight.is_empty() {
                if idle_timeout.is_zero() {
                    tokio::task::yield_now().await;
                } else {
                    tracing::debug!(
                        spider = spider_name,
                        idle_ms = idle_timeout.as_millis(),
                        "队列为空，等待新任务..."
                    );
                    tokio::time::sleep(idle_timeout).await;
                }
                continue;
            }

            if let Some(result) = inflight.next().await {
                handle_task_result(result, scheduler, allowed_domains, &mut outputs, &mut round, spider_name).await?;
            }
        }

        self.pipeline.close(spider_name).await?;

        let total_items: usize = outputs.iter().map(|o| o.items.len()).sum();
        tracing::info!(
            spider = spider_name,
            rounds = round,
            total_items,
            "引擎已停止"
        );

        Ok(outputs)
    }

    /// 主动停止引擎。当前轮次结束后优雅退出。
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    fn prepare_all_step_middlewares<Sp: Spider>(
        &mut self,
        spider: &Sp,
        compiled: Option<&Compiled>,
    ) -> Result<(), SpiderError> {
        if self.active_spider.is_some() {
            return Ok(());
        }
        self.active_spider = Some(spider.name().to_string());

        let base_runtime = self.settings.to_runtime_config();
        let defaults = compile_runtime(&base_runtime)?;
        let merged_base = merge_middleware(defaults, self.settings.middlewares.clone());

        let base_mw = build_middleware(&merged_base, &self.custom_factories)?;
        self.step_middlewares.insert("parse".to_string(), base_mw);

        if let Some(compiled) = compiled {
            for step in &compiled.steps {
                if self.step_middlewares.contains_key(&step.id) {
                    continue;
                }
                let runtime = effective_runtime(base_runtime.clone(), Some(compiled), &step.id)?;
                let step_defaults = compile_runtime(&runtime)?;
                let step_overrides = step_middlewares(Some(compiled), &step.id);
                let merged = merge_middleware(
                    merge_middleware(step_defaults, self.settings.middlewares.clone()),
                    step_overrides,
                );
                let mw = build_middleware(&merged, &self.custom_factories)?;
                self.step_middlewares.insert(step.id.clone(), mw);
            }
        }

        Ok(())
    }
}

async fn handle_task_result<S: Scheduler>(
    result: TaskResult,
    scheduler: &mut S,
    allowed_domains: &[String],
    outputs: &mut Vec<SpiderOutput>,
    round: &mut usize,
    spider_name: &str,
) -> Result<(), SpiderError> {
    let url = result.url;
    match result.outcome {
        TaskOutcome::Success(output) => {
            *round += 1;
            tracing::info!(
                spider = spider_name,
                round = *round,
                items = output.items.len(),
                follows = output.follows.len(),
                "完成第 {} 轮解析",
                round,
            );
            for follow in &output.follows {
                if follow.dont_filter || is_domain_allowed(&follow.url, allowed_domains) {
                    scheduler
                        .enqueue(crate::scheduler::types::ScheduledTask::new(follow.clone()))
                        .await?;
                }
            }
            scheduler.ack(&url).await?;
            outputs.push(SpiderOutput {
                items: output.items,
                requests: output.follows,
            });
        }
        TaskOutcome::Retry(retry_task) => {
            scheduler.enqueue(retry_task).await?;
            scheduler.ack(&url).await?;
        }
        TaskOutcome::Drop => {
            scheduler.ack(&url).await?;
        }
        TaskOutcome::Error(e) => {
            tracing::error!(spider = spider_name, url = url.as_str(), error = %e, "任务出错");
            scheduler.nack(&url).await?;
        }
    }
    Ok(())
}

struct TaskSuccess {
    items: Vec<crate::item::Item>,
    follows: Vec<crate::request::Request>,
}

enum TaskOutcome {
    Success(TaskSuccess),
    Retry(crate::scheduler::types::ScheduledTask),
    Drop,
    Error(SpiderError),
}

struct TaskResult {
    url: String,
    outcome: TaskOutcome,
}

async fn execute_task<'a, H, B, P, Sp>(
    request: crate::request::Request,
    http: &'a H,
    browser: &'a B,
    pipeline: &'a P,
    engine_mw: &'a MiddlewareChain,
    step_mw: &'a MiddlewareChain,
    spider: &'a Sp,
    compiled: Option<&'a Compiled>,
    allowed_domains: &'a [String],
    spider_name: &'a str,
    _global_permit: tokio::sync::OwnedSemaphorePermit,
    domain_sem: Arc<tokio::sync::Semaphore>,
) -> TaskResult
where
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
    Sp: Spider,
{
    let url = request.url.clone();
    let _domain_permit = match domain_sem.acquire().await {
        Ok(p) => p,
        Err(_) => {
            return TaskResult {
                url,
                outcome: TaskOutcome::Error(SpiderError::engine("domain semaphore closed")),
            };
        }
    };

    let outcome = execute_task_inner(
        request, http, browser, pipeline, engine_mw, step_mw, spider, compiled, allowed_domains, spider_name,
    )
    .await;

    TaskResult { url, outcome }
}

async fn execute_task_inner<'a, H, B, P, Sp>(
    request: crate::request::Request,
    http: &'a H,
    browser: &'a B,
    pipeline: &'a P,
    engine_mw: &'a MiddlewareChain,
    step_mw: &'a MiddlewareChain,
    spider: &'a Sp,
    compiled: Option<&'a Compiled>,
    allowed_domains: &'a [String],
    spider_name: &'a str,
) -> TaskOutcome
where
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
    Sp: Spider,
{
    let mut context = EngineContext::new(request);

    // Download middleware: process_request
    match run_middleware_request(engine_mw, step_mw, MiddlewareType::Download, &mut context).await {
        Ok(Flow::Continue) => {}
        Ok(flow) => return flow_to_outcome(flow, &context),
        Err(e) => return TaskOutcome::Error(e),
    }

    // Download
    let response = match context.request.mode {
        RequestMode::Http => http.fetch(&context.request).await,
        RequestMode::Browser => browser.fetch(&context.request).await,
    };

    let response = match response {
        Ok(r) => r,
        Err(error) => {
            tracing::warn!(url = context.request.url.as_str(), error = %error, "下载失败");
            match run_middleware_exception(engine_mw, step_mw, MiddlewareType::Download, &mut context, &error).await {
                Ok(Flow::Continue) => return TaskOutcome::Error(error),
                Ok(flow) => return flow_to_outcome(flow, &context),
                Err(e) => return TaskOutcome::Error(e),
            }
        }
    };

    context.response = Some(response.clone());

    // Download middleware: process_response
    match run_middleware_response(engine_mw, step_mw, MiddlewareType::Download, &mut context).await {
        Ok(Flow::Continue) => {}
        Ok(flow) => return flow_to_outcome(flow, &context),
        Err(e) => return TaskOutcome::Error(e),
    }

    // Spider middleware: process_request
    match run_middleware_request(engine_mw, step_mw, MiddlewareType::Spider, &mut context).await {
        Ok(Flow::Continue) => {}
        Ok(flow) => return flow_to_outcome(flow, &context),
        Err(e) => return TaskOutcome::Error(e),
    }

    // Dispatch: callback or DSL
    let output = spider.dispatch(&response, compiled).await;

    match output {
        Ok(mut output) => {
            // Spider middleware: process_response
            match run_middleware_response(engine_mw, step_mw, MiddlewareType::Spider, &mut context).await {
                Ok(Flow::Continue) => {}
                Ok(flow) => return flow_to_outcome(flow, &context),
                Err(e) => return TaskOutcome::Error(e),
            }

            // Pipeline
            let mut kept_items = Vec::with_capacity(output.items.len());
            for mut item in output.items.drain(..) {
                match pipeline.process(&mut item, spider_name).await {
                    Ok(true) => kept_items.push(item),
                    Ok(false) => {
                        tracing::debug!(spider = spider_name, "pipeline 丢弃 item");
                    }
                    Err(e) => {
                        tracing::warn!(spider = spider_name, error = %e, "pipeline 处理 item 出错");
                    }
                }
            }

            // Domain filter (filter only, don't enqueue — main loop does that)
            let mut follows = Vec::new();
            for req in output.requests {
                if req.dont_filter || is_domain_allowed(&req.url, allowed_domains) {
                    follows.push(req);
                } else {
                    tracing::debug!(url = req.url.as_str(), "域名不在 allowed_domains 内，已过滤");
                }
            }

            TaskOutcome::Success(TaskSuccess {
                items: kept_items,
                follows,
            })
        }
        Err(error) => {
            tracing::error!(
                spider = spider_name,
                url = context.request.url.as_str(),
                error = %error,
                "解析回调执行失败"
            );
            match run_middleware_exception(engine_mw, step_mw, MiddlewareType::Spider, &mut context, &error).await {
                Ok(Flow::Continue) => TaskOutcome::Error(error),
                Ok(flow) => flow_to_outcome(flow, &context),
                Err(e) => TaskOutcome::Error(e),
            }
        }
    }
}

async fn run_middleware_request(
    engine_mw: &MiddlewareChain,
    step_mw: &MiddlewareChain,
    kind: MiddlewareType,
    context: &mut EngineContext,
) -> Result<Flow, SpiderError> {
    let flow = engine_mw.process_request(kind, context).await?;
    if !matches!(flow, Flow::Continue) {
        return Ok(flow);
    }
    step_mw.process_request(kind, context).await
}

async fn run_middleware_response(
    engine_mw: &MiddlewareChain,
    step_mw: &MiddlewareChain,
    kind: MiddlewareType,
    context: &mut EngineContext,
) -> Result<Flow, SpiderError> {
    let flow = engine_mw.process_response(kind, context).await?;
    if !matches!(flow, Flow::Continue) {
        return Ok(flow);
    }
    step_mw.process_response(kind, context).await
}

async fn run_middleware_exception(
    engine_mw: &MiddlewareChain,
    step_mw: &MiddlewareChain,
    kind: MiddlewareType,
    context: &mut EngineContext,
    error: &SpiderError,
) -> Result<Flow, SpiderError> {
    let flow = engine_mw.process_exception(kind, context, error).await?;
    if !matches!(flow, Flow::Continue) {
        return Ok(flow);
    }
    step_mw.process_exception(kind, context, error).await
}

fn flow_to_outcome(flow: Flow, context: &EngineContext) -> TaskOutcome {
    match flow {
        Flow::Continue => unreachable!(),
        Flow::Drop(_) => TaskOutcome::Drop,
        Flow::Retry { reason, backoff_ms } => {
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
            if let Some(ms) = backoff_ms {
                request.meta.insert(
                    "_retry_backoff_ms".to_string(),
                    crate::value::Value::Number(ms as f64),
                );
            }

            let task = match backoff_ms {
                Some(ms) if ms > 0 => {
                    crate::scheduler::types::ScheduledTask::with_delay_ms(request, ms)
                }
                _ => crate::scheduler::types::ScheduledTask::new(request),
            };
            TaskOutcome::Retry(task)
        }
    }
}

fn extract_domain(url: &str) -> Option<&str> {
    let after_scheme = url.split("://").nth(1)?;
    let host_port = after_scheme.split('/').next()?;
    Some(host_port.split(':').next().unwrap_or(host_port))
}

fn is_domain_allowed(url: &str, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let Some(domain) = extract_domain(url) else {
        return false;
    };
    allowed
        .iter()
        .any(|d| domain == d.as_str() || domain.ends_with(&format!(".{d}")))
}

/// 引擎停止句柄。可 Clone，跨线程使用。
///
/// 调用 `stop()` 通知引擎在当前轮次结束后优雅退出。
/// 典型用法：配合 `tokio::signal::ctrl_c()` 在收到 Ctrl+C 时停止引擎。
#[derive(Clone)]
pub struct ShutdownHandle {
    flag: Arc<AtomicBool>,
}

impl ShutdownHandle {
    /// 通知引擎停止。引擎会在当前轮次结束后退出 run() 循环。
    pub fn stop(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    pub fn is_stopped(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
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

fn step_middlewares(compiled: Option<&Compiled>, step_id: &str) -> crate::middleware::Map {
    let Some(compiled) = compiled else {
        return crate::middleware::Map::new();
    };

    compiled
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .map(|step| step.middlewares.clone())
        .unwrap_or_default()
}

#[cfg(test)]
impl<S, H, B, P> Engine<S, H, B, P>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
{
    async fn execute_once(&mut self) -> Result<Option<crate::response::Response>, SpiderError> {
        let Some(task) = self.scheduler.lease().await? else {
            return Ok(None);
        };
        let url = task.request.url.clone();
        let mut context = EngineContext::new(task.request);

        let step_id = step_id_from_request(&context.request);
        let default_mw = MiddlewareChain::default();
        let step_mw = self.step_middlewares.get(&step_id).unwrap_or(&default_mw);

        match run_middleware_request(&self.middleware, step_mw, MiddlewareType::Download, &mut context).await {
            Ok(crate::engine::types::Flow::Continue) => {}
            Ok(_) => {
                self.scheduler.ack(&url).await?;
                return Ok(None);
            }
            Err(e) => {
                self.scheduler.nack(&url).await?;
                return Err(e);
            }
        }

        let response = match context.request.mode {
            RequestMode::Http => self.http.fetch(&context.request).await,
            RequestMode::Browser => self.browser.fetch(&context.request).await,
        };

        let response = match response {
            Ok(r) => r,
            Err(e) => {
                self.scheduler.nack(&url).await?;
                return Err(e);
            }
        };

        context.response = Some(response.clone());

        match run_middleware_response(&self.middleware, step_mw, MiddlewareType::Download, &mut context).await {
            Ok(crate::engine::types::Flow::Continue) => {}
            Ok(_) => {
                self.scheduler.ack(&url).await?;
                return Ok(None);
            }
            Err(e) => {
                self.scheduler.nack(&url).await?;
                return Err(e);
            }
        }

        self.scheduler.ack(&url).await?;
        Ok(Some(response))
    }

    async fn execute_spider_once<Sp: Spider>(
        &mut self,
        spider: &Sp,
        compiled: Option<&Compiled>,
    ) -> Result<Option<crate::spider::Output>, SpiderError> {
        self.prepare_all_step_middlewares(spider, compiled)?;

        let Some(task) = self.scheduler.lease().await? else {
            return Ok(None);
        };
        let url = task.request.url.clone();

        let step_id = step_id_from_request(&task.request);
        let default_mw = MiddlewareChain::default();
        let step_mw = self.step_middlewares.get(&step_id).unwrap_or(&default_mw);

        let outcome = execute_task_inner(
            task.request,
            &self.http,
            &self.browser,
            &self.pipeline,
            &self.middleware,
            step_mw,
            spider,
            compiled,
            &self.allowed_domains,
            spider.name(),
        )
        .await;

        match outcome {
            TaskOutcome::Success(output) => {
                for follow in &output.follows {
                    if follow.dont_filter || is_domain_allowed(&follow.url, &self.allowed_domains) {
                        self.scheduler
                            .enqueue(crate::scheduler::types::ScheduledTask::new(follow.clone()))
                            .await?;
                    }
                }
                self.scheduler.ack(&url).await?;
                Ok(Some(crate::spider::Output {
                    items: output.items,
                    requests: output.follows,
                }))
            }
            TaskOutcome::Retry(retry_task) => {
                self.scheduler.enqueue(retry_task).await?;
                self.scheduler.ack(&url).await?;
                Ok(None)
            }
            TaskOutcome::Drop => {
                self.scheduler.ack(&url).await?;
                Ok(None)
            }
            TaskOutcome::Error(e) => {
                self.scheduler.nack(&url).await?;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::context::EngineContext;
    use crate::engine::types::Flow;
    use crate::middleware::traits::Middleware;
    use crate::middleware::types::MiddlewareConfig;
    use crate::request::Request;
    use crate::response::Response;
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

        let mut engine = Engine::new(scheduler, StubHttpDownloader, StubBrowserDownloader);
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

        let mut engine = Engine::new(scheduler, StubHttpDownloader, StubBrowserDownloader);
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

        let mut engine = Engine::new(scheduler, StubHttpDownloader, StubBrowserDownloader)
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

        let mut engine = Engine::new(scheduler, HtmlHttpDownloader, StubBrowserDownloader);
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

        let mut engine = Engine::new(scheduler, FlowHttpDownloader, StubBrowserDownloader);
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

        let mut engine = Engine::new(scheduler, HtmlHttpDownloader, StubBrowserDownloader)
            .with_settings(runtime_settings());
        block_on(engine.execute_spider_once(&SimpleSpider("runtime"), None))
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
        let mut engine = Engine::new(scheduler, downloader, StubBrowserDownloader)
            .with_settings(dedup_settings());

        let first = block_on(engine.execute_spider_once(&SimpleSpider("dedup"), None)).unwrap();
        let second = block_on(engine.execute_spider_once(&SimpleSpider("dedup"), None)).unwrap();

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
        let mut engine = Engine::new(scheduler, downloader, StubBrowserDownloader)
            .with_settings(retry_settings());

        let first = block_on(engine.execute_spider_once(&SimpleSpider("retry"), None)).unwrap();
        let second = block_on(engine.execute_spider_once(&SimpleSpider("retry"), None)).unwrap();

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
        let mut engine = Engine::new(scheduler, downloader, StubBrowserDownloader)
            .with_settings(retry_backoff_settings());

        let first = block_on(engine.execute_spider_once(&SimpleSpider("retry_backoff"), None)).unwrap();
        let second = block_on(engine.execute_spider_once(&SimpleSpider("retry_backoff"), None)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
        let third = block_on(engine.execute_spider_once(&SimpleSpider("retry_backoff"), None)).unwrap();

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
        let mut engine = Engine::new(scheduler, downloader, StubBrowserDownloader)
            .with_settings(interval_settings());

        let first = block_on(engine.execute_spider_once(&SimpleSpider("interval"), None)).unwrap();
        let second = block_on(engine.execute_spider_once(&SimpleSpider("interval"), None)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
        let third = block_on(engine.execute_spider_once(&SimpleSpider("interval"), None)).unwrap();

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
        let mut engine = Engine::new(scheduler, downloader, StubBrowserDownloader)
            .with_settings(rate_limit_settings());

        let first = block_on(engine.execute_spider_once(&SimpleSpider("rate_limit"), None)).unwrap();
        let second = block_on(engine.execute_spider_once(&SimpleSpider("rate_limit"), None)).unwrap();

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
        let mut engine = Engine::new(scheduler, downloader, StubBrowserDownloader);

        let first = block_on(engine.execute_spider_once(&FlowSpider, Some(&compiled))).unwrap();
        let second = block_on(engine.execute_spider_once(&FlowSpider, Some(&compiled))).unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(*fetches.lock().unwrap(), 1);
        assert!(engine.step_middlewares.contains_key("detail"));
    }

    #[test]
    fn engine_applies_step_middlewares_override_spider() {
        let mut scheduler = MemoryScheduler::default();
        block_on(scheduler.enqueue(ScheduledTask::new(
            Request::new("https://example.com/detail/1")
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
                        "runtime":{"schedule":{"interval_ms":10}},
                        "MIDDLEWARES":{
                            "dedup":{"enabled":false,"order":999}
                        }
                    }
                ]
            }"#
            .to_string(),
        ))
        .unwrap();

        let mut engine = Engine::new(scheduler, HtmlHttpDownloader, StubBrowserDownloader);
        block_on(engine.execute_spider_once(&FlowSpider, Some(&compiled))).unwrap();

        let dedup = engine
            .step_middlewares
            .get("detail")
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.key == "dedup")
            .unwrap();
        assert!(!dedup.config.enabled);
        assert_eq!(dedup.config.order, 999);
    }

    #[tokio::test]
    async fn engine_run_processes_start_urls_to_completion() {
        let scheduler = MemoryScheduler::default();
        let mut engine = Engine::new(scheduler, HtmlHttpDownloader, StubBrowserDownloader)
            .with_settings(
                Settings::default().idle_timeout(std::time::Duration::from_millis(10)),
            );

        let handle = engine.shutdown_handle();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            handle.stop();
        });

        let outputs = engine.run(&RunSpider).await.unwrap();

        assert_eq!(outputs.len(), 2);
        assert_eq!(
            outputs[0].items[0].get("title"),
            Some(&Value::String("Hello".to_string()))
        );
    }

    struct RunSpider;

    impl Spider for RunSpider {
        fn name(&self) -> &str {
            "run_spider"
        }

        fn start_urls(&self) -> Vec<String> {
            vec![
                "https://example.com/page/1".to_string(),
                "https://example.com/page/2".to_string(),
            ]
        }

        fn rules(&self) -> Option<crate::rules::Config> {
            Some(crate::rules::Config {
                r#type: "inline".to_string(),
                options: [(
                    "value".to_string(),
                    Value::String(
                        r#"{"steps":[{"id":"parse","impl":"dsl","parse":{"fields":[{"name":"title","source":"html","selector_type":"css","selector":["h1.title"],"attribute":"text"}]}}]}"#.to_string(),
                    ),
                )]
                .into_iter()
                .collect(),
            })
        }

        async fn parse(&self, _response: &Response) -> Result<SpiderOutput, SpiderError> {
            Ok(SpiderOutput::empty())
        }
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

    struct StubHttpDownloader;

    impl crate::download::traits::Downloader for StubHttpDownloader {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            let mut response =
                Response::from_request(request.clone(), 200, Default::default(), Vec::new());
            response.protocol = Some("HTTP/1.1".to_string());
            Ok(response)
        }
    }

    struct StubBrowserDownloader;

    impl crate::download::traits::Downloader for StubBrowserDownloader {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            let mut response =
                Response::from_request(request.clone(), 200, Default::default(), Vec::new());
            response.protocol = Some("browser".to_string());
            response.flags.push("browser".to_string());
            Ok(response)
        }
    }

    struct HtmlHttpDownloader;

    impl crate::download::traits::Downloader for HtmlHttpDownloader {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            Ok(Response::from_request(
                request.clone(),
                200,
                Default::default(),
                br#"<h1 class="title">Hello</h1>"#.to_vec(),
            ))
        }
    }

    struct FlowHttpDownloader;

    impl crate::download::traits::Downloader for FlowHttpDownloader {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
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
        }
    }

    struct CountHttpDownloader {
        fetches: Arc<Mutex<usize>>,
        statuses: Vec<u16>,
    }

    impl crate::download::traits::Downloader for CountHttpDownloader {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
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
        }
    }

    struct TestSpider;

    impl Spider for TestSpider {
        fn name(&self) -> &str {
            "test"
        }

        async fn parse(&self, _response: &Response) -> Result<SpiderOutput, SpiderError> {
            Ok(SpiderOutput::empty())
        }
    }

    struct FlowSpider;

    impl Spider for FlowSpider {
        fn name(&self) -> &str {
            "flow"
        }

        async fn parse(&self, _response: &Response) -> Result<SpiderOutput, SpiderError> {
            Ok(SpiderOutput::empty())
        }

        async fn call(&self, name: &str, response: &Response) -> Result<SpiderOutput, SpiderError> {
            match name {
                "parse" => self.parse(response).await,
                "parse_detail" => Ok(SpiderOutput {
                    items: vec![crate::item::Item::new().with_field(
                        "detail",
                        Value::String(response.url.clone()),
                    )],
                    requests: Vec::new(),
                }),
                other => Err(SpiderError::engine(format!("unknown callback: {other}"))),
            }
        }
    }

    struct SimpleSpider(&'static str);

    impl Spider for SimpleSpider {
        fn name(&self) -> &str {
            self.0
        }

        async fn parse(&self, _response: &Response) -> Result<SpiderOutput, SpiderError> {
            Ok(SpiderOutput::empty())
        }
    }

    fn runtime_settings() -> Settings {
        Settings::default().with_runtime(crate::runtime::Config {
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
        }).middlewares(
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
            .collect(),
        )
    }

    fn dedup_settings() -> Settings {
        Settings::default().with_runtime(crate::runtime::Config {
            schedule: BTreeMap::new(),
            retry: BTreeMap::new(),
            dedup: [
                ("enabled".to_string(), Value::Bool(true)),
                ("key".to_string(), Value::String("url".to_string())),
            ]
            .into_iter()
            .collect(),
        })
    }

    fn retry_settings() -> Settings {
        Settings::default().with_runtime(crate::runtime::Config {
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
        })
    }

    fn retry_backoff_settings() -> Settings {
        Settings::default().with_runtime(crate::runtime::Config {
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
        })
    }

    fn interval_settings() -> Settings {
        Settings::default().with_runtime(crate::runtime::Config {
            schedule: [("interval_ms".to_string(), Value::Number(10.0))]
                .into_iter()
                .collect(),
            retry: BTreeMap::new(),
            dedup: BTreeMap::new(),
        })
    }

    fn rate_limit_settings() -> Settings {
        Settings::default().with_runtime(crate::runtime::Config {
            schedule: [("rate_per_minute".to_string(), Value::Number(1.0))]
                .into_iter()
                .collect(),
            retry: BTreeMap::new(),
            dedup: BTreeMap::new(),
        })
    }
}
