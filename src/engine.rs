pub mod context;
pub mod flow;
mod task;

use crate::download::traits::Downloader;
use crate::engine::task::{TaskExecutor, TaskRun, TaskRunReservation, apply_task_run};
use crate::error::SpiderError;
use crate::middleware::{Chain, Config, Registry, build as build_middleware};
use crate::plugins::types::{
    PluginKind, engine_reserved_plugin_kind_names, engine_supported_plugin_kind_names,
};
use crate::rules::Compiled;
use crate::runtime::compile::{compile as compile_runtime, merge as merge_middleware};
use crate::runtime::{Config as RuntimeConfig, merge as merge_runtime};
use crate::scheduler::{Scheduler, Task};
use crate::settings::Settings;
use crate::spider::{Output as SpiderOutput, Spider};
use futures::stream::{FuturesUnordered, StreamExt};
use jiff::SignedDuration;
use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
use crate::engine::context::EngineContext;
#[cfg(test)]
use crate::engine::task::{TaskOutcome, run_middleware_request, run_middleware_response};
#[cfg(test)]
use crate::middleware::Stage;
#[cfg(test)]
use crate::request::RequestMode;

pub struct Engine<S, H, B, P = (), St = crate::store::File> {
    pub scheduler: S,
    pub http: H,
    pub browser: B,
    pub pipeline: P,
    pub store: St,
    robots: Arc<crate::robots::Robot>,
    stats: Arc<crate::stats::Tracker>,
    pub settings: Settings,
    pub middleware: Chain,
    pub plugins: Registry,
    prepared: bool,
    shutdown: Arc<AtomicBool>,
}

fn to_std_duration(duration: SignedDuration) -> Result<std::time::Duration, String> {
    std::time::Duration::try_from(duration).map_err(|error| error.to_string())
}

impl<S, H, B> Engine<S, H, B>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
{
    /// Build an engine from fully explicit core parts.
    ///
    /// This is the lowest-level constructor for callers that want to replace
    /// the default scheduler or downloaders.
    pub fn from_parts(scheduler: S, http: H, browser: B) -> Self {
        Self {
            scheduler,
            http,
            browser,
            pipeline: (),
            store: crate::store::File::default(),
            robots: Arc::new(crate::robots::Robot::default()),
            stats: Arc::new(crate::stats::Tracker::default()),
            settings: Settings::default(),
            middleware: Chain::default(),
            plugins: Registry::new(),
            prepared: false,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Engine<crate::scheduler::Memory, crate::download::Http, crate::download::Browser>
where
    crate::download::Http: Downloader,
    crate::download::Browser: Downloader,
{
    /// Build the zero-config default engine.
    ///
    /// Defaults:
    /// - scheduler: `scheduler::Memory`
    /// - http downloader: `download::Http`
    /// - browser downloader: `download::Browser`
    /// - store: `store::File`
    pub fn new() -> Self {
        Self::from_parts(
            crate::scheduler::Memory::default(),
            crate::download::Http::default(),
            crate::download::Browser::default(),
        )
    }
}

impl Default for Engine<crate::scheduler::Memory, crate::download::Http, crate::download::Browser>
where
    crate::download::Http: Downloader,
    crate::download::Browser: Downloader,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<H, B> Engine<crate::scheduler::Memory, H, B>
where
    H: Downloader,
    B: Downloader,
{
    /// Build an engine with the default memory scheduler and custom
    /// downloaders.
    pub fn with_downloaders(http: H, browser: B) -> Self {
        Self::from_parts(crate::scheduler::Memory::default(), http, browser)
    }
}

impl<S, H, B, P, St> Engine<S, H, B, P, St>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
    St: crate::store::Store,
{
    pub fn with_settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    /// Replace the middleware chain while keeping the current engine
    /// configuration.
    pub fn with_middleware(mut self, middleware: Chain) -> Self {
        self.middleware = middleware;
        self
    }

    /// Replace the scheduler while keeping the current engine configuration.
    pub fn with_scheduler<S2: Scheduler>(self, scheduler: S2) -> Engine<S2, H, B, P, St> {
        Engine {
            scheduler,
            http: self.http,
            browser: self.browser,
            pipeline: self.pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            settings: self.settings,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Replace the item pipeline while keeping the current engine
    /// configuration.
    pub fn with_pipeline<P2: crate::pipeline::Pipeline>(
        self,
        pipeline: P2,
    ) -> Engine<S, H, B, P2, St> {
        Engine {
            scheduler: self.scheduler,
            http: self.http,
            browser: self.browser,
            pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            settings: self.settings,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Replace the final item store while keeping the current engine
    /// configuration.
    pub fn with_store<St2: crate::store::Store>(self, store: St2) -> Engine<S, H, B, P, St2> {
        Engine {
            scheduler: self.scheduler,
            http: self.http,
            browser: self.browser,
            pipeline: self.pipeline,
            store,
            robots: self.robots,
            stats: self.stats,
            settings: self.settings,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Register a custom middleware instance directly on the engine-level chain.
    ///
    /// This middleware applies to every request and response.
    ///
    /// ```ignore
    /// engine.add_middleware(
    ///     "custom_ua",
    ///     Config { enabled: true, stage: Stage::Download, order: 50, .. },
    ///     Box::new(MyUaMiddleware),
    /// );
    /// ```
    pub fn add_middleware(
        mut self,
        key: impl Into<String>,
        config: Config,
        middleware: Box<dyn crate::middleware::Middleware>,
    ) -> Self {
        self.middleware.push(key, config, middleware);
        self
    }

    /// Register a custom middleware factory.
    ///
    /// After registration, the same key can be referenced from
    /// `Settings::middlewares` or a DSL `MIDDLEWARES` section, and the engine
    /// will call the factory automatically to create the instance.
    ///
    /// ```ignore
    /// engine.register_middleware("custom_ua", |options| {
    ///     Ok(Box::new(MyUaMiddleware::new(options)))
    /// });
    /// ```
    pub fn register_middleware(
        mut self,
        key: impl Into<String>,
        factory: impl Fn(
            &std::collections::BTreeMap<String, crate::value::Value>,
        ) -> Result<Box<dyn crate::middleware::Middleware>, SpiderError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.plugins.register(key, factory);
        self
    }

    /// Load plugin manifests and verify that every declared middleware plugin
    /// has a registered factory.
    ///
    /// Before calling this method, register each middleware factory with
    /// `register_middleware()`. `load_plugins()` currently supports only
    /// `kind = "middleware"` plugins; other known kinds are kept as
    /// namespaces only and are not auto-wired by the engine.
    ///
    /// It verifies that every middleware declared in `plugins.toml` has a
    /// matching engine factory and returns an error otherwise.
    ///
    /// ```ignore
    /// let manifests = load_plugin_manifest("plugins.toml")?;
    /// let mut registry = PluginRegistry::new();
    /// registry.register_all(manifests)?;
    ///
    /// let engine = Engine::from_parts(scheduler, http, browser)
    ///     .register_middleware("custom_signature", |opts| {
    ///         Ok(Box::new(CustomSignatureMiddleware::new(opts)))
    ///     })
    ///     .load_plugins(&registry)?;
    /// ```
    pub fn load_plugins(
        self,
        registry: &crate::plugins::PluginRegistry,
    ) -> Result<Self, SpiderError> {
        let mut unsupported_manifests = Vec::new();
        for manifest in registry.all() {
            let kind = PluginKind::try_from(manifest.kind.as_str()).map_err(SpiderError::plugin)?;
            if !kind.is_engine_supported() {
                unsupported_manifests.push(format!("({}, {})", manifest.kind, manifest.name));
            }
        }

        if !unsupported_manifests.is_empty() {
            return Err(SpiderError::plugin(format!(
                "engine currently only supports plugin kinds [{}]; unsupported manifests: {}; reserved but not loadable yet: [{}]",
                engine_supported_plugin_kind_names().join(", "),
                unsupported_manifests.join(", "),
                engine_reserved_plugin_kind_names().join(", ")
            )));
        }

        for manifest in registry.by_kind("middleware") {
            if !self.plugins.has(&manifest.name) {
                return Err(SpiderError::plugin(format!(
                    "middleware plugin '{}' declared in plugins.toml (entry: {}) but no factory registered; \
                     call register_middleware(\"{}\", ...) before load_plugins()",
                    manifest.name, manifest.entry, manifest.name
                )));
            }
            tracing::info!(
                plugin = manifest.name.as_str(),
                kind = "middleware",
                entry = manifest.entry.as_str(),
                "plugin loaded"
            );
        }
        Ok(self)
    }

    /// Get a clonable shutdown handle.
    ///
    /// Example:
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

    /// Return a cumulative runtime stats snapshot for this engine instance.
    pub fn stats(&self) -> crate::stats::Snapshot {
        self.stats.snapshot()
    }

    /// Run the engine continuously until a stop signal is received.
    ///
    /// Concurrent downloads are controlled by:
    /// - `settings.concurrent_requests` for the global concurrency limit
    /// - `settings.concurrent_requests_per_domain` for the per-domain limit
    ///
    /// The engine does not exit automatically when the queue becomes empty.
    /// It exits only when:
    /// 1. `engine.stop()` or `shutdown_handle().stop()` is called
    /// 2. Ctrl+C triggers a stop signal
    pub async fn run<Sp: Spider>(&mut self, spider: &Sp) -> Result<Vec<SpiderOutput>, SpiderError> {
        let spider_name = spider.name();
        tracing::info!(spider = spider_name, "engine started");

        let allowed_domains = spider.allowed_domains();
        if !allowed_domains.is_empty() {
            tracing::info!(
                spider = spider_name,
                domains = ?allowed_domains,
                "allowed domain filter enabled"
            );
        }

        self.pipeline.open(spider_name).await?;
        self.store.open(spider_name).await?;

        let compiled = match spider.rules() {
            Some(config) => {
                tracing::info!(spider = spider_name, "loading DSL rules");
                Some(crate::rules::load(&config).await?)
            }
            None => None,
        };

        let step_middlewares = self.build_step_middlewares(compiled.as_ref())?;

        let start_urls = spider.build_start_urls();
        tracing::info!(
            spider = spider_name,
            count = start_urls.len(),
            "enqueueing start URLs"
        );
        for url in start_urls {
            let request = crate::request::Request::new(url);
            self.scheduler.enqueue(Task::new(request)).await?;
        }

        let max_concurrent = self.settings.concurrent_requests;
        let per_domain_limit = self.settings.concurrent_requests_per_domain;
        let idle_timeout = self.settings.idle_timeout;
        let idle_timeout_std =
            if idle_timeout.is_zero() {
                None
            } else {
                Some(to_std_duration(idle_timeout).map_err(|error| {
                    SpiderError::engine(format!("invalid idle_timeout: {error}"))
                })?)
            };

        tracing::info!(
            spider = spider_name,
            concurrent = max_concurrent,
            per_domain = per_domain_limit,
            "concurrency settings"
        );

        let global_semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let mut domain_semaphores: BTreeMap<String, Arc<tokio::sync::Semaphore>> = BTreeMap::new();

        let default_step_chain = Chain::default();

        type TaskFuture<'a> = Pin<Box<dyn std::future::Future<Output = TaskRun> + 'a>>;
        let mut inflight: FuturesUnordered<TaskFuture<'_>> = FuturesUnordered::new();
        let mut outputs = Vec::new();
        let mut round = 0usize;

        let scheduler = &mut self.scheduler;
        let http = &self.http;
        let browser = &self.browser;
        let pipeline = &self.pipeline;
        let store = &self.store;
        let robots = &self.robots;
        let stats = &self.stats;
        let engine_chain = &self.middleware;
        let step_chains = &step_middlewares;
        let allowed_domains = &allowed_domains;
        let shutdown = &self.shutdown;

        loop {
            if shutdown.load(Ordering::Relaxed) {
                tracing::info!(
                    spider = spider_name,
                    "received stop signal, waiting for {} in-flight tasks to finish...",
                    inflight.len()
                );
                while let Some(result) = inflight.next().await {
                    apply_task_run(
                        result,
                        scheduler,
                        allowed_domains,
                        &mut outputs,
                        &mut round,
                        spider_name,
                        stats.as_ref(),
                    )
                    .await?;
                }
                break;
            }

            while inflight.len() < max_concurrent {
                let Ok(global_permit_guard) = global_semaphore.clone().try_acquire_owned() else {
                    break;
                };
                let Some(task) = scheduler.take_ready().await? else {
                    drop(global_permit_guard);
                    break;
                };

                let domain = extract_domain(&task.request.url)
                    .unwrap_or("unknown")
                    .to_string();
                let domain_semaphore = domain_semaphores
                    .entry(domain)
                    .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(per_domain_limit)))
                    .clone();

                let step_id = step_id_from_request(&task.request);
                let step_chain = step_chains.get(&step_id).unwrap_or(&default_step_chain);

                let task_executor = TaskExecutor {
                    http,
                    browser,
                    pipeline,
                    store,
                    robots,
                    settings: &self.settings,
                    stats,
                    engine_chain,
                    step_chain,
                    spider,
                    compiled: compiled.as_ref(),
                    allowed_domains,
                    spider_name,
                };
                let task_run_reservation = TaskRunReservation::new(
                    task.id.clone(),
                    task.request,
                    global_permit_guard,
                    domain_semaphore,
                );

                inflight.push(Box::pin(
                    task_executor.run_with_reservation(task_run_reservation),
                ));
            }

            if inflight.is_empty() {
                if let Some(idle_timeout_std) = idle_timeout_std {
                    tracing::debug!(
                        spider = spider_name,
                        idle_ms = idle_timeout.as_millis(),
                        "queue is empty, waiting for new tasks..."
                    );
                    tokio::time::sleep(idle_timeout_std).await;
                } else {
                    tokio::task::yield_now().await;
                }
                continue;
            }

            if let Some(result) = inflight.next().await {
                apply_task_run(
                    result,
                    scheduler,
                    allowed_domains,
                    &mut outputs,
                    &mut round,
                    spider_name,
                    stats.as_ref(),
                )
                .await?;
            }
        }

        self.store.close(spider_name).await?;
        self.pipeline.close(spider_name).await?;

        let total_items: usize = outputs.iter().map(|o| o.items.len()).sum();
        tracing::info!(
            spider = spider_name,
            rounds = round,
            total_items,
            request_count = stats.snapshot().request_count,
            response_count = stats.snapshot().response_count,
            error_count = stats.snapshot().error_count,
            retry_count = stats.snapshot().retry_count,
            pipeline_drop_count = stats.snapshot().pipeline_drop_count,
            "engine stopped"
        );

        Ok(outputs)
    }

    /// Signal the engine to stop and exit gracefully after the current loop.
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    fn build_step_middlewares(
        &self,
        compiled: Option<&Compiled>,
    ) -> Result<BTreeMap<String, Chain>, SpiderError> {
        let base_runtime = self.settings.to_runtime_config();
        let defaults = compile_runtime(&base_runtime)?;
        let merged_base = merge_middleware(defaults, self.settings.middlewares.clone());

        let mut out = BTreeMap::new();
        let base_chain = build_middleware(&merged_base, &self.plugins)?;
        out.insert("parse".to_string(), base_chain);

        if let Some(compiled) = compiled {
            for step in &compiled.steps {
                if out.contains_key(&step.id) {
                    continue;
                }
                let runtime = effective_runtime(base_runtime.clone(), Some(compiled), &step.id)?;
                let step_defaults = compile_runtime(&runtime)?;
                let step_overrides = step_middlewares(Some(compiled), &step.id);
                let merged = merge_middleware(
                    merge_middleware(step_defaults, self.settings.middlewares.clone()),
                    step_overrides,
                );
                let chain = build_middleware(&merged, &self.plugins)?;
                out.insert(step.id.clone(), chain);
            }
        }

        Ok(out)
    }
}

impl<H, B, P, St> Engine<crate::scheduler::Memory, H, B, P, St>
where
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
    St: crate::store::Store,
{
    /// Attach checkpoint persistence to the current default memory scheduler.
    ///
    /// This keeps in-memory scheduling semantics, but saves every state change
    /// through the provided `scheduler::checkpoint::Persist` backend.
    pub fn with_checkpoint<Persist>(
        self,
        persist: Persist,
    ) -> Engine<crate::scheduler::checkpoint::Memory<Persist>, H, B, P, St>
    where
        Persist: crate::scheduler::checkpoint::Persist,
    {
        let scheduler = crate::scheduler::checkpoint::Memory::from_parts(self.scheduler, persist);

        Engine {
            scheduler,
            http: self.http,
            browser: self.browser,
            pipeline: self.pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            settings: self.settings,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Restore the default memory scheduler from checkpoint persistence and
    /// attach the same persistence backend for future updates.
    pub async fn load_checkpoint<Persist>(
        self,
        persist: Persist,
    ) -> Result<Engine<crate::scheduler::checkpoint::Memory<Persist>, H, B, P, St>, SpiderError>
    where
        Persist: crate::scheduler::checkpoint::Persist,
    {
        let scheduler = crate::scheduler::checkpoint::Memory::load(persist).await?;

        Ok(Engine {
            scheduler,
            http: self.http,
            browser: self.browser,
            pipeline: self.pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            settings: self.settings,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        })
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

/// Clonable engine shutdown handle.
///
/// Calling `stop()` tells the engine to exit gracefully after the current loop.
/// Typical usage is wiring it to `tokio::signal::ctrl_c()`.
#[derive(Clone)]
pub struct ShutdownHandle {
    flag: Arc<AtomicBool>,
}

impl ShutdownHandle {
    /// Notify the engine to stop after the current run loop iteration.
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
impl<S, H, B, P, St> Engine<S, H, B, P, St>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
    St: crate::store::Store,
{
    async fn execute_once(&mut self) -> Result<Option<crate::response::Response>, SpiderError> {
        let Some(task) = self.scheduler.take_ready().await? else {
            return Ok(None);
        };
        let task_id = task.id.clone();
        let mut context = EngineContext::new(task.request).with_task_id(task_id.clone());

        let default_chain = Chain::default();
        let step_chain = &default_chain;

        match run_middleware_request(&self.middleware, step_chain, Stage::Download, &mut context)
            .await
        {
            Ok(crate::engine::flow::Flow::Continue) => {}
            Ok(_) => {
                self.scheduler.complete(&task_id).await?;
                return Ok(None);
            }
            Err(e) => {
                self.scheduler.requeue(&task_id).await?;
                return Err(e);
            }
        }

        self.stats.record_request();
        let response = match context.request.mode {
            RequestMode::Http => self.http.fetch(&context.request).await,
            RequestMode::Browser => self.browser.fetch(&context.request).await,
        };

        let response = match response {
            Ok(r) => {
                self.stats.record_response();
                r
            }
            Err(e) => {
                self.stats.record_error();
                self.scheduler.requeue(&task_id).await?;
                return Err(e);
            }
        };

        context.response = Some(response.clone());

        match run_middleware_response(&self.middleware, step_chain, Stage::Download, &mut context)
            .await
        {
            Ok(crate::engine::flow::Flow::Continue) => {}
            Ok(_) => {
                self.scheduler.complete(&task_id).await?;
                return Ok(None);
            }
            Err(e) => {
                self.scheduler.requeue(&task_id).await?;
                return Err(e);
            }
        }

        self.scheduler.complete(&task_id).await?;
        Ok(Some(response))
    }

    async fn execute_spider_once<Sp: Spider>(
        &mut self,
        spider: &Sp,
        compiled: Option<&Compiled>,
        step_chains: &mut BTreeMap<String, Chain>,
    ) -> Result<Option<crate::spider::Output>, SpiderError> {
        if !self.prepared {
            *step_chains = self.build_step_middlewares(compiled)?;
            self.pipeline.open(spider.name()).await?;
            self.store.open(spider.name()).await?;
            self.prepared = true;
        }

        let Some(task) = self.scheduler.take_ready().await? else {
            return Ok(None);
        };
        let task_id = task.id.clone();

        let step_id = step_id_from_request(&task.request);
        let default_chain = Chain::default();
        let step_chain = step_chains.get(&step_id).unwrap_or(&default_chain);

        let task_executor = TaskExecutor {
            http: &self.http,
            browser: &self.browser,
            pipeline: &self.pipeline,
            store: &self.store,
            robots: &self.robots,
            settings: &self.settings,
            stats: &self.stats,
            engine_chain: &self.middleware,
            step_chain,
            spider,
            compiled,
            allowed_domains: &[],
            spider_name: spider.name(),
        };

        let outcome = task_executor.run(task_id.clone(), task.request).await;

        match outcome {
            TaskOutcome::Success(output) => {
                for follow in &output.follows {
                    if follow.dont_filter || is_domain_allowed(&follow.url, &[]) {
                        self.scheduler.enqueue(Task::new(follow.clone())).await?;
                    }
                }
                self.scheduler.complete(&task_id).await?;
                Ok(Some(crate::spider::Output {
                    items: output.items,
                    requests: output.follows,
                }))
            }
            TaskOutcome::Retry(retry_task) => {
                self.stats.record_retry();
                self.scheduler.enqueue(*retry_task).await?;
                self.scheduler.complete(&task_id).await?;
                Ok(None)
            }
            TaskOutcome::Drop => {
                self.scheduler.complete(&task_id).await?;
                Ok(None)
            }
            TaskOutcome::Error(e) => {
                self.stats.record_error();
                self.scheduler.requeue(&task_id).await?;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::context::EngineContext;
    use crate::engine::flow::Flow;
    use crate::middleware::Config;
    use crate::middleware::traits::Middleware;
    use crate::pipeline::Pipeline;
    use crate::plugins::{PluginManifest, PluginRegistry};
    use crate::request::Request;
    use crate::response::Response;
    use crate::scheduler::checkpoint::{Checkpoint, Persist};
    use crate::scheduler::memory::Memory;
    use crate::scheduler::{Scheduler, Task};
    use crate::spider::{Output as SpiderOutput, Spider};
    use crate::stats::Snapshot as StatsSnapshot;
    use crate::store::Memory as MemoryStore;
    use crate::value::Value;
    use jiff::SignedDuration;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn engine_executes_http_task_once() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser);
        let response = block_on(engine.execute_once()).unwrap().unwrap();

        assert_eq!(response.url, "https://example.com");
        assert_eq!(response.protocol.as_deref(), Some("HTTP/1.1"));
    }

    #[test]
    fn engine_executes_browser_task_once() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::browser("https://example.com/browser"))))
            .unwrap();

        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser);
        let response = block_on(engine.execute_once()).unwrap().unwrap();

        assert_eq!(response.url, "https://example.com/browser");
        assert_eq!(response.protocol.as_deref(), Some("browser"));
    }

    #[test]
    fn engine_with_downloaders_uses_default_memory_scheduler() {
        let mut engine = Engine::with_downloaders(StubHttp, StubBrowser);
        block_on(engine.scheduler.enqueue(Task::new(Request::new(
            "https://example.com/default-engine",
        ))))
        .unwrap();

        let response = block_on(engine.execute_once()).unwrap().unwrap();

        assert_eq!(response.url, "https://example.com/default-engine");
    }

    #[test]
    fn engine_default_is_zero_config_memory_engine() {
        let engine = Engine::default();

        assert!(!block_on(engine.scheduler.has_pending()).unwrap());
    }

    #[test]
    fn engine_with_checkpoint_wraps_default_memory_scheduler() {
        let persist = TestCheckpointPersist::default();
        let mut engine =
            Engine::with_downloaders(StubHttp, StubBrowser).with_checkpoint(persist.clone());

        block_on(
            engine
                .scheduler
                .enqueue(Task::new(Request::new("https://example.com/checkpoint"))),
        )
        .unwrap();

        let checkpoint = block_on(persist.load()).unwrap();

        assert_eq!(checkpoint.ready.len(), 1);
        assert_eq!(
            checkpoint.ready[0].request.url,
            "https://example.com/checkpoint"
        );
    }

    #[test]
    fn engine_load_checkpoint_restores_memory_scheduler_from_persist() {
        let persist = TestCheckpointPersist::default();

        block_on(persist.save(&Checkpoint {
            ready: vec![Task::new(Request::new("https://example.com/restored"))],
            delayed: Vec::new(),
            inflight: Vec::new(),
        }))
        .unwrap();

        let engine = block_on(
            Engine::with_downloaders(StubHttp, StubBrowser).load_checkpoint(persist.clone()),
        )
        .unwrap();

        assert_eq!(engine.scheduler.counts().ready, 1);
        assert_eq!(
            engine.scheduler.checkpoint().ready[0].request.url,
            "https://example.com/restored"
        );
    }

    #[test]
    fn engine_runs_download_middlewares_around_fetch() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com")))).unwrap();

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut middleware = Chain::default();
        middleware.push(
            "recorder",
            Config {
                enabled: true,
                stage: Stage::Download,
                order: 100,
                options: BTreeMap::<String, Value>::new(),
            },
            Box::new(RecordMiddleware { log: log.clone() }),
        );

        let mut engine =
            Engine::from_parts(scheduler, StubHttp, StubBrowser).with_middleware(middleware);
        block_on(engine.execute_once()).unwrap();

        assert_eq!(
            *log.lock().unwrap(),
            vec!["request".to_string(), "response".to_string()]
        );
    }

    #[test]
    fn engine_load_plugins_rejects_unsupported_plugin_kinds_explicitly() {
        let mut registry = PluginRegistry::new();
        registry
            .register(PluginManifest {
                name: "json_storage".to_string(),
                kind: "storage".to_string(),
                entry: "plugins_demo::JsonStoragePipeline".to_string(),
                r#override: false,
            })
            .unwrap();

        let result =
            Engine::from_parts(Memory::default(), StubHttp, StubBrowser).load_plugins(&registry);
        assert!(result.is_err());
        let error = result.err().unwrap();

        assert!(
            error
                .to_string()
                .contains("only supports plugin kinds [middleware]")
        );
        assert!(error.to_string().contains("(storage, json_storage)"));
    }

    #[test]
    fn engine_loads_runtime_middlewares_and_applies_explicit_overrides() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, HtmlHttp, StubBrowser)
            .with_settings(runtime_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();
        block_on(engine.execute_spider_once(&SimpleSpider("runtime"), None, &mut step_chains))
            .unwrap()
            .unwrap();

        let keys = step_chains
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

        let dedup = step_chains
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
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/dedup")))).unwrap();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/dedup")))).unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_settings(dedup_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&SimpleSpider("dedup"), None, &mut step_chains))
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&SimpleSpider("dedup"), None, &mut step_chains))
                .unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(*fetches.lock().unwrap(), 1);
    }

    #[test]
    fn engine_retries_on_configured_status() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/retry")))).unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![500, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_settings(retry_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&SimpleSpider("retry"), None, &mut step_chains))
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&SimpleSpider("retry"), None, &mut step_chains))
                .unwrap();

        assert!(first.is_none());
        assert!(second.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_respects_retry_backoff_delay() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/retry-backoff"))))
            .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![500, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_settings(retry_backoff_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("retry_backoff"),
            None,
            &mut step_chains,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("retry_backoff"),
            None,
            &mut step_chains,
        ))
        .unwrap();
        std::thread::sleep(to_std_duration(SignedDuration::from_millis(15)).unwrap());
        let third = block_on(engine.execute_spider_once(
            &SimpleSpider("retry_backoff"),
            None,
            &mut step_chains,
        ))
        .unwrap();

        assert!(first.is_none());
        assert!(second.is_none());
        assert!(third.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_respects_interval_gate_delay() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/interval/1"))))
            .unwrap();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/interval/2"))))
            .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_settings(interval_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&SimpleSpider("interval"), None, &mut step_chains))
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&SimpleSpider("interval"), None, &mut step_chains))
                .unwrap();
        std::thread::sleep(to_std_duration(SignedDuration::from_millis(15)).unwrap());
        let third =
            block_on(engine.execute_spider_once(&SimpleSpider("interval"), None, &mut step_chains))
                .unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert!(third.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_respects_rate_limit_delay() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/rate/1")))).unwrap();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/rate/2")))).unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_settings(rate_limit_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("rate_limit"),
            None,
            &mut step_chains,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("rate_limit"),
            None,
            &mut step_chains,
        ))
        .unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(*fetches.lock().unwrap(), 1);
    }

    #[test]
    fn engine_pipeline_keeps_items_and_memory_store_writes_them() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let store = MemoryStore::default();
        let mut engine =
            Engine::from_parts(scheduler, StubHttp, StubBrowser).with_store(store.clone());
        let mut step_chains = BTreeMap::new();

        let output = block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_chains))
            .unwrap()
            .unwrap();

        let expected =
            vec![crate::item::Item::new().with_field("title", Value::String("post".to_string()))];

        assert_eq!(output.items, expected);
        assert_eq!(store.items(), expected);
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                error_count: 0,
                retry_count: 0,
                item_count: 1,
                pipeline_drop_count: 0,
            }
        );
    }

    #[test]
    fn engine_store_prefers_batch_write_for_kept_items() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let store = BatchOnlyStore::default();
        let mut engine =
            Engine::from_parts(scheduler, StubHttp, StubBrowser).with_store(store.clone());
        let mut step_chains = BTreeMap::new();

        let output = block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_chains))
            .unwrap()
            .unwrap();

        assert_eq!(output.items, store.items());
    }

    #[test]
    fn engine_pipeline_can_drop_items_explicitly() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_pipeline(DropPipeline)
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();

        let output = block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_chains))
            .unwrap()
            .unwrap();

        assert!(output.items.is_empty());
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                error_count: 0,
                retry_count: 0,
                item_count: 0,
                pipeline_drop_count: 1,
            }
        );
    }

    #[test]
    fn engine_pipeline_error_fails_task_explicitly() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_pipeline(FailPipeline)
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();

        let error =
            block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_chains)).unwrap_err();

        assert!(error.to_string().contains("pipeline failed"));
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                error_count: 1,
                retry_count: 0,
                item_count: 0,
                pipeline_drop_count: 0,
            }
        );
    }

    #[test]
    fn engine_stats_track_retries_across_attempts() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/retry")))).unwrap();

        let downloader = CountHttp {
            fetches: Arc::new(Mutex::new(0usize)),
            statuses: vec![500, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_settings(retry_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&SimpleSpider("retry"), None, &mut step_chains))
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&SimpleSpider("retry"), None, &mut step_chains))
                .unwrap();

        assert!(first.is_none());
        assert!(second.is_some());
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 2,
                response_count: 2,
                error_count: 0,
                retry_count: 1,
                item_count: 0,
                pipeline_drop_count: 0,
            }
        );
    }

    #[test]
    fn engine_skips_disallowed_request_when_robots_obey_is_enabled() {
        let mut scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/private/page"))))
            .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_settings(
                Settings::default()
                    .with_robots_obey(true)
                    .with_robots_user_agent("kun-bot"),
            )
            .with_store(MemoryStore::default());
        block_on(engine.robots.seed_from_body(
            "https://example.com/private/page",
            "User-agent: kun-bot\nDisallow: /private\n",
        ));

        let mut step_chains = BTreeMap::new();
        let output =
            block_on(engine.execute_spider_once(&SimpleSpider("robots"), None, &mut step_chains))
                .unwrap();

        assert!(output.is_none());
        assert_eq!(*fetches.lock().unwrap(), 0);
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 0,
                response_count: 0,
                error_count: 0,
                retry_count: 0,
                item_count: 0,
                pipeline_drop_count: 0,
            }
        );
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

    struct StubHttp;

    impl crate::download::traits::Downloader for StubHttp {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            let mut response =
                Response::from_request(request.clone(), 200, Default::default(), Vec::new());
            response.protocol = Some("HTTP/1.1".to_string());
            Ok(response)
        }
    }

    struct StubBrowser;

    impl crate::download::traits::Downloader for StubBrowser {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            let mut response =
                Response::from_request(request.clone(), 200, Default::default(), Vec::new());
            response.protocol = Some("browser".to_string());
            response.flags.push("browser".to_string());
            Ok(response)
        }
    }

    struct HtmlHttp;

    impl crate::download::traits::Downloader for HtmlHttp {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            Ok(Response::from_request(
                request.clone(),
                200,
                Default::default(),
                br#"<h1 class="title">Hello</h1>"#.to_vec(),
            ))
        }
    }

    struct CountHttp {
        fetches: Arc<Mutex<usize>>,
        statuses: Vec<u16>,
    }

    impl crate::download::traits::Downloader for CountHttp {
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

    struct SimpleSpider(&'static str);

    impl Spider for SimpleSpider {
        fn name(&self) -> &str {
            self.0
        }

        async fn parse(&self, _response: &Response) -> Result<SpiderOutput, SpiderError> {
            Ok(SpiderOutput::empty())
        }
    }

    struct ItemSpider;

    impl Spider for ItemSpider {
        fn name(&self) -> &str {
            "item_spider"
        }

        async fn parse(&self, _response: &Response) -> Result<SpiderOutput, SpiderError> {
            Ok(SpiderOutput {
                items: vec![
                    crate::item::Item::new().with_field("title", Value::String("post".to_string())),
                ],
                requests: Vec::new(),
            })
        }
    }

    struct DropPipeline;

    #[derive(Clone, Default)]
    struct BatchOnlyStore {
        items: Arc<Mutex<Vec<crate::item::Item>>>,
    }

    impl BatchOnlyStore {
        fn items(&self) -> Vec<crate::item::Item> {
            self.items.lock().unwrap().clone()
        }
    }

    impl crate::store::Store for BatchOnlyStore {
        async fn write(
            &self,
            _item: &crate::item::Item,
            _spider_name: &str,
        ) -> Result<(), SpiderError> {
            Err(SpiderError::engine(
                "engine should prefer batch_write over write for final store delivery",
            ))
        }

        async fn batch_write(
            &self,
            items: &[crate::item::Item],
            _spider_name: &str,
        ) -> Result<(), SpiderError> {
            self.items.lock().unwrap().extend(items.iter().cloned());
            Ok(())
        }
    }

    impl Pipeline for DropPipeline {
        async fn process(
            &self,
            _item: &mut crate::item::Item,
            _spider_name: &str,
        ) -> Result<bool, SpiderError> {
            Ok(false)
        }
    }

    struct FailPipeline;

    impl Pipeline for FailPipeline {
        async fn process(
            &self,
            _item: &mut crate::item::Item,
            _spider_name: &str,
        ) -> Result<bool, SpiderError> {
            Err(SpiderError::engine("pipeline failed"))
        }
    }

    #[derive(Clone, Default)]
    struct TestCheckpointPersist {
        checkpoint: Arc<Mutex<Checkpoint>>,
    }

    impl Persist for TestCheckpointPersist {
        async fn load(&self) -> Result<Checkpoint, SpiderError> {
            Ok(self.checkpoint.lock().unwrap().clone())
        }

        async fn save(&self, checkpoint: &Checkpoint) -> Result<(), SpiderError> {
            *self.checkpoint.lock().unwrap() = checkpoint.clone();
            Ok(())
        }
    }

    fn runtime_settings() -> Settings {
        Settings::default()
            .with_runtime(crate::runtime::Config {
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
            })
            .with_middlewares(
                [(
                    "dedup".to_string(),
                    Config {
                        enabled: false,
                        stage: Stage::Download,
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
