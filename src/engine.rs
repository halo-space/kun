pub mod context;
pub mod flow;
mod task;

use crate::download::traits::Downloader;
use crate::engine::task::{
    TaskExecutor, TaskRun, TaskRunReservation, apply_task_run, enqueue_request, enqueue_task,
};
use crate::error::SpiderError;
use crate::middleware::{Chain, Config, Registry, build as build_middleware};
use crate::plugins::types::{
    PluginKind, engine_deferred_plugin_kind_names, engine_supported_plugin_kind_names,
};
use crate::request::{Request, RequestMode};
use crate::rules::Compiled;
use crate::runtime::compile::{compile as compile_runtime, merge as merge_middleware};
use crate::runtime::{Config as RuntimeConfig, merge as merge_runtime};
use crate::scheduler::{Scheduler, Task};
use crate::settings::Settings;
use crate::spider::{Output as SpiderOutput, Spider};
use futures::stream::{FuturesUnordered, StreamExt};
use jiff::SignedDuration;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use url::Url;

#[cfg(test)]
use crate::engine::context::EngineContext;
#[cfg(test)]
use crate::engine::task::{TaskOutcome, run_middleware_request, run_middleware_response};
#[cfg(test)]
use crate::middleware::Stage;

pub struct Engine<S, H, B, D = crate::dedup::Memory, P = (), St = crate::store::File> {
    pub scheduler: S,
    pub http: H,
    pub browser: B,
    pub dedup: D,
    pub pipeline: P,
    pub store: St,
    robots: Arc<dyn crate::robots::Robot>,
    stats: Arc<crate::stats::Tracker>,
    signals: Arc<crate::signals::Bus>,
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
            dedup: crate::dedup::Memory::default(),
            pipeline: (),
            store: crate::store::File::default(),
            robots: Arc::new(crate::robots::Memory::default()),
            stats: Arc::new(crate::stats::Tracker::default()),
            signals: Arc::new(crate::signals::Bus::default()),
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
    /// - dedup: `dedup::Memory`
    /// - robots: `robots::Memory`
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
    ///
    /// This is a convenience constructor for the common case of keeping the
    /// default scheduler while replacing both downloader components together.
    /// If you only need to replace one downloader, prefer `Engine::new()
    /// .with_http(...)` or `Engine::new().with_browser(...)`.
    pub fn with_downloaders(http: H, browser: B) -> Self {
        Self::from_parts(crate::scheduler::Memory::default(), http, browser)
    }
}

impl<S, H, B, D, P, St> Engine<S, H, B, D, P, St>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
    D: crate::dedup::Dedup,
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
    pub fn with_scheduler<S2: Scheduler>(self, scheduler: S2) -> Engine<S2, H, B, D, P, St> {
        Engine {
            scheduler,
            http: self.http,
            browser: self.browser,
            dedup: self.dedup,
            pipeline: self.pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            settings: self.settings,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Replace only the HTTP downloader while keeping the current engine
    /// configuration.
    pub fn with_http<H2: Downloader>(self, http: H2) -> Engine<S, H2, B, D, P, St> {
        Engine {
            scheduler: self.scheduler,
            http,
            browser: self.browser,
            dedup: self.dedup,
            pipeline: self.pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            settings: self.settings,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Replace only the browser downloader while keeping the current engine
    /// configuration.
    pub fn with_browser<B2: Downloader>(self, browser: B2) -> Engine<S, H, B2, D, P, St> {
        Engine {
            scheduler: self.scheduler,
            http: self.http,
            browser,
            dedup: self.dedup,
            pipeline: self.pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            settings: self.settings,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Replace the request dedup component while keeping the current engine
    /// configuration.
    pub fn with_dedup<D2: crate::dedup::Dedup>(self, dedup: D2) -> Engine<S, H, B, D2, P, St> {
        Engine {
            scheduler: self.scheduler,
            http: self.http,
            browser: self.browser,
            dedup,
            pipeline: self.pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
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
    ) -> Engine<S, H, B, D, P2, St> {
        Engine {
            scheduler: self.scheduler,
            http: self.http,
            browser: self.browser,
            dedup: self.dedup,
            pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            settings: self.settings,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Replace the robots policy component while keeping the current engine
    /// configuration.
    pub fn with_robots(mut self, robots: impl crate::robots::Robot + 'static) -> Self {
        self.robots = Arc::new(robots);
        self
    }

    /// Replace the final item store while keeping the current engine
    /// configuration.
    pub fn with_store<St2: crate::store::Store>(self, store: St2) -> Engine<S, H, B, D, P, St2> {
        Engine {
            scheduler: self.scheduler,
            http: self.http,
            browser: self.browser,
            dedup: self.dedup,
            pipeline: self.pipeline,
            store,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
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

    /// Enqueue one request through the engine's dedup component before it
    /// reaches the scheduler.
    pub async fn enqueue(&mut self, request: crate::request::Request) -> Result<bool, SpiderError> {
        let enqueued = enqueue_request(
            &mut self.scheduler,
            &mut self.dedup,
            request.clone(),
            &[],
            Some(self.stats.as_ref()),
        )
        .await?;

        if enqueued {
            self.signals
                .emit(crate::signals::Signal::request_scheduled("manual", request))
                .await;
        }

        Ok(enqueued)
    }

    /// Register a lightweight runtime stats reporter.
    ///
    /// `engine.stats()` remains the primary read API. Reporters are an
    /// observation hook for streaming counter updates to custom telemetry.
    pub fn with_stats_reporter(self, reporter: impl crate::stats::Reporter + 'static) -> Self {
        self.stats.add_reporter(Arc::new(reporter));
        self
    }

    /// Register an async signal listener for engine lifecycle and runtime
    /// events.
    pub fn with_signal_listener(self, listener: impl crate::signals::Listener + 'static) -> Self {
        self.signals.add_listener(Arc::new(listener));
        self
    }

    /// Register an extension through the engine signal bus.
    pub fn with_extension(self, extension: impl crate::extensions::Extension + 'static) -> Self {
        self.with_signal_listener(extension)
    }

    /// Load plugin manifests and verify that every declared middleware plugin
    /// has a registered factory.
    ///
    /// Before calling this method, register each middleware factory with
    /// `register_middleware()`. `load_plugins()` currently auto-loads only
    /// `kind = "middleware"` plugins; other known component kinds are kept as
    /// explicit future extension points and are not auto-wired by the engine.
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
                "engine currently only auto-loads plugin kinds [{}]; unsupported manifests: {}; known but not auto-loadable yet: [{}]",
                engine_supported_plugin_kind_names().join(", "),
                unsupported_manifests.join(", "),
                engine_deferred_plugin_kind_names().join(", ")
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

    async fn enqueue_start_requests<Sp: Spider>(
        &mut self,
        spider: &Sp,
        allowed_domains: &[String],
    ) -> Result<(), SpiderError> {
        let start_requests = spider.build_start_requests();

        tracing::info!(
            spider = spider.name(),
            count = start_requests.len(),
            "enqueueing start URLs"
        );

        for request in &start_requests {
            let enqueued = enqueue_request(
                &mut self.scheduler,
                &mut self.dedup,
                request.clone(),
                &[],
                Some(self.stats.as_ref()),
            )
            .await?;

            if enqueued {
                self.signals
                    .emit(crate::signals::Signal::request_scheduled(
                        spider.name(),
                        request.clone(),
                    ))
                    .await;
            }
        }

        if !self.settings.robots_sitemap_seeds {
            return Ok(());
        }

        self.enqueue_robots_sitemap_seeds(spider.name(), allowed_domains, &start_requests)
            .await
    }

    async fn enqueue_robots_sitemap_seeds(
        &mut self,
        spider_name: &str,
        allowed_domains: &[String],
        start_requests: &[crate::request::Request],
    ) -> Result<(), SpiderError> {
        let mut origin_requests = BTreeMap::new();
        for request in start_requests {
            let Some(origin) = request_origin(request.url.as_str()) else {
                continue;
            };
            origin_requests
                .entry(origin)
                .or_insert_with(|| request.clone());
        }

        if origin_requests.is_empty() {
            return Ok(());
        }

        let mut seen_sitemaps = BTreeSet::new();
        let mut pending_sitemaps = VecDeque::new();

        for request in origin_requests.values() {
            match self.robots.sitemaps(request).await {
                Ok(sitemaps) => {
                    for sitemap in sitemaps {
                        let Some(resolved) = resolve_url(request.url.as_str(), sitemap.as_str())
                        else {
                            tracing::warn!(
                                spider = spider_name,
                                sitemap = sitemap.as_str(),
                                "skipping invalid sitemap URL declared by robots"
                            );
                            continue;
                        };

                        if seen_sitemaps.insert(resolved.clone()) {
                            pending_sitemaps.push_back((resolved, request.clone()));
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        spider = spider_name,
                        url = request.url.as_str(),
                        error = %error,
                        "failed to read sitemap URLs from robots"
                    );
                }
            }
        }

        if pending_sitemaps.is_empty() {
            return Ok(());
        }

        let mut seed_count = 0usize;

        while let Some((sitemap_url, representative_request)) = pending_sitemaps.pop_front() {
            let sitemap_request =
                build_robots_sitemap_fetch_request(&representative_request, sitemap_url.clone());

            let response = match self.http.fetch(&sitemap_request).await {
                Ok(response) => response,
                Err(error) => {
                    tracing::warn!(
                        spider = spider_name,
                        sitemap = sitemap_url.as_str(),
                        error = %error,
                        "failed to fetch sitemap"
                    );
                    continue;
                }
            };

            if !(200..300).contains(&response.status) {
                tracing::warn!(
                    spider = spider_name,
                    sitemap = sitemap_url.as_str(),
                    status = response.status,
                    "skipping sitemap because fetch did not return success"
                );
                continue;
            }

            let entries = response.sitemap().entries();

            for nested_sitemap in entries.sitemaps {
                let Some(resolved) = resolve_url(sitemap_url.as_str(), nested_sitemap.as_str())
                else {
                    tracing::warn!(
                        spider = spider_name,
                        sitemap = nested_sitemap.as_str(),
                        parent = sitemap_url.as_str(),
                        "skipping invalid nested sitemap URL"
                    );
                    continue;
                };

                if seen_sitemaps.insert(resolved.clone()) {
                    pending_sitemaps.push_back((resolved, representative_request.clone()));
                }
            }

            for page_url in entries.urls {
                let Some(resolved) = resolve_url(sitemap_url.as_str(), page_url.as_str()) else {
                    tracing::warn!(
                        spider = spider_name,
                        url = page_url.as_str(),
                        sitemap = sitemap_url.as_str(),
                        "skipping invalid sitemap URL entry"
                    );
                    continue;
                };

                let sitemap_seed_request =
                    build_robots_sitemap_seed_request(&representative_request, resolved);
                let sitemap_seed_task =
                    build_robots_sitemap_seed_task(sitemap_seed_request.clone(), &self.settings);

                if enqueue_task(
                    &mut self.scheduler,
                    &mut self.dedup,
                    sitemap_seed_task,
                    allowed_domains,
                    Some(self.stats.as_ref()),
                )
                .await?
                {
                    seed_count += 1;
                    self.signals
                        .emit(crate::signals::Signal::request_scheduled(
                            spider_name,
                            sitemap_seed_request,
                        ))
                        .await;
                }
            }
        }

        tracing::info!(
            spider = spider_name,
            count = seed_count,
            "enqueueing robots sitemap seed URLs"
        );

        Ok(())
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
        self.signals
            .emit(crate::signals::Signal::spider_opened(spider_name))
            .await;

        let compiled = match spider.rules() {
            Some(config) => {
                tracing::info!(spider = spider_name, "loading DSL rules");
                Some(crate::rules::load(&config).await?)
            }
            None => None,
        };

        let step_middlewares = self.build_step_middlewares(compiled.as_ref())?;

        self.enqueue_start_requests(spider, &allowed_domains)
            .await?;

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

        let scheduler = &self.scheduler;
        let dedup = &mut self.dedup;
        let http = &self.http;
        let browser = &self.browser;
        let pipeline = &self.pipeline;
        let store = &self.store;
        let robots = self.robots.as_ref();
        let stats = self.stats.clone();
        let signals = self.signals.clone();
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
                        dedup,
                        allowed_domains,
                        &mut outputs,
                        &mut round,
                        spider_name,
                        stats.as_ref(),
                        signals.as_ref(),
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

                let domain = extract_domain(&task.task.request.url)
                    .unwrap_or("unknown")
                    .to_string();
                let domain_semaphore = domain_semaphores
                    .entry(domain)
                    .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(per_domain_limit)))
                    .clone();

                let step_id = step_id_from_request(&task.task.request);
                let step_chain = step_chains.get(&step_id).unwrap_or(&default_step_chain);

                let task_executor = TaskExecutor {
                    scheduler,
                    http,
                    browser,
                    pipeline,
                    store,
                    robots,
                    settings: &self.settings,
                    stats: stats.clone(),
                    signals: signals.clone(),
                    engine_chain,
                    step_chain,
                    spider,
                    compiled: compiled.as_ref(),
                    allowed_domains,
                    spider_name,
                };
                let task_run_reservation = TaskRunReservation::new(
                    task.lease,
                    task.task.request,
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
                        idle_timeout = idle_timeout.as_millis(),
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
                    dedup,
                    allowed_domains,
                    &mut outputs,
                    &mut round,
                    spider_name,
                    stats.as_ref(),
                    signals.as_ref(),
                )
                .await?;
            }
        }

        self.store.close(spider_name).await?;
        self.pipeline.close(spider_name).await?;
        self.signals
            .emit(crate::signals::Signal::spider_closed(
                spider_name,
                stats.snapshot(),
            ))
            .await;

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

fn build_robots_sitemap_fetch_request(parent: &Request, url: String) -> Request {
    Request::from_parent_for_follow(parent, url).with_mode(RequestMode::Http)
}

fn build_robots_sitemap_seed_request(parent: &Request, url: String) -> Request {
    Request::from_parent_for_follow(parent, url)
}

fn build_robots_sitemap_seed_task(request: Request, settings: &Settings) -> Task {
    Task::new(request)
        .with_priority(settings.robots_sitemap_seed_priority)
        .with_depth(settings.robots_sitemap_seed_depth)
}

impl<H, B, D, P, St> Engine<crate::scheduler::Memory, H, B, D, P, St>
where
    H: Downloader,
    B: Downloader,
    D: crate::dedup::Dedup,
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
    ) -> Engine<crate::scheduler::checkpoint::Memory<Persist>, H, B, D, P, St>
    where
        Persist: crate::scheduler::checkpoint::Persist,
    {
        let scheduler = crate::scheduler::checkpoint::Memory::from_parts(self.scheduler, persist);

        Engine {
            scheduler,
            http: self.http,
            browser: self.browser,
            dedup: self.dedup,
            pipeline: self.pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
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
    ) -> Result<Engine<crate::scheduler::checkpoint::Memory<Persist>, H, B, D, P, St>, SpiderError>
    where
        Persist: crate::scheduler::checkpoint::Persist,
    {
        let scheduler = crate::scheduler::checkpoint::Memory::load(persist).await?;

        Ok(Engine {
            scheduler,
            http: self.http,
            browser: self.browser,
            dedup: self.dedup,
            pipeline: self.pipeline,
            store: self.store,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
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

fn request_origin(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    Some(parsed.origin().ascii_serialization())
}

fn resolve_url(base_url: &str, raw_url: &str) -> Option<String> {
    let raw_url = raw_url.trim();
    if raw_url.is_empty() {
        return None;
    }

    if let Ok(url) = Url::parse(raw_url) {
        return Some(url.to_string());
    }

    let base = Url::parse(base_url).ok()?;
    base.join(raw_url).ok().map(|url| url.to_string())
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
impl<S, H, B, D, P, St> Engine<S, H, B, D, P, St>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
    D: crate::dedup::Dedup,
    P: crate::pipeline::Pipeline,
    St: crate::store::Store,
{
    async fn execute_once(&mut self) -> Result<Option<crate::response::Response>, SpiderError> {
        let Some(task) = self.scheduler.take_ready().await? else {
            return Ok(None);
        };
        let task_id = task.lease.task_id().clone();
        let mut context = EngineContext::new(task.task.request)
            .with_task_id(task_id)
            .with_stats(self.stats.clone());
        let lease = task.lease;

        let default_chain = Chain::default();
        let step_chain = &default_chain;

        match run_middleware_request(&self.middleware, step_chain, Stage::Download, &mut context)
            .await
        {
            Ok(crate::engine::flow::Flow::Continue) => {}
            Ok(_) => {
                self.scheduler.complete(&lease).await?;
                return Ok(None);
            }
            Err(e) => {
                self.scheduler.requeue(&lease).await?;
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
                self.scheduler.requeue(&lease).await?;
                return Err(e);
            }
        };

        context.response = Some(response.clone());

        match run_middleware_response(&self.middleware, step_chain, Stage::Download, &mut context)
            .await
        {
            Ok(crate::engine::flow::Flow::Continue) => {}
            Ok(_) => {
                self.scheduler.complete(&lease).await?;
                return Ok(None);
            }
            Err(e) => {
                self.scheduler.requeue(&lease).await?;
                return Err(e);
            }
        }

        let response = context.response.clone().unwrap_or(response);

        self.scheduler.complete(&lease).await?;
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
        let task_id = task.lease.task_id().clone();
        let lease = task.lease.clone();

        let step_id = step_id_from_request(&task.task.request);
        let default_chain = Chain::default();
        let step_chain = step_chains.get(&step_id).unwrap_or(&default_chain);

        let task_executor = TaskExecutor {
            scheduler: &self.scheduler,
            http: &self.http,
            browser: &self.browser,
            pipeline: &self.pipeline,
            store: &self.store,
            robots: self.robots.as_ref(),
            settings: &self.settings,
            stats: self.stats.clone(),
            signals: self.signals.clone(),
            engine_chain: &self.middleware,
            step_chain,
            spider,
            compiled,
            allowed_domains: &[],
            spider_name: spider.name(),
        };

        let outcome = task_executor.run(task_id, task.task.request).await;

        match outcome {
            TaskOutcome::Success(output) => {
                for follow in &output.follows {
                    let enqueued = enqueue_request(
                        &mut self.scheduler,
                        &mut self.dedup,
                        follow.clone(),
                        &[],
                        Some(self.stats.as_ref()),
                    )
                    .await?;

                    if enqueued {
                        self.signals
                            .emit(crate::signals::Signal::request_scheduled(
                                spider.name(),
                                follow.clone(),
                            ))
                            .await;
                    }
                }
                self.scheduler.complete(&lease).await?;
                Ok(Some(crate::spider::Output {
                    items: output.items,
                    requests: output.follows,
                }))
            }
            TaskOutcome::Retry(retry_task) => {
                self.stats.record_retry();
                let request = retry_task.request.clone();
                self.scheduler.enqueue(*retry_task).await?;
                self.signals
                    .emit(crate::signals::Signal::request_scheduled(
                        spider.name(),
                        request,
                    ))
                    .await;
                self.scheduler.complete(&lease).await?;
                Ok(None)
            }
            TaskOutcome::Drop => {
                self.scheduler.complete(&lease).await?;
                Ok(None)
            }
            TaskOutcome::Error(e) => {
                self.stats.record_error();
                self.scheduler.requeue(&lease).await?;
                Err(e)
            }
            TaskOutcome::LeaseLost(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::context::EngineContext;
    use crate::engine::flow::Flow;
    use crate::future::BoxFuture;
    use crate::middleware::Config;
    use crate::middleware::traits::Middleware;
    use crate::pipeline::Pipeline;
    use crate::plugins::{PluginManifest, PluginRegistry};
    use crate::request::{Headers, Request};
    use crate::response::Response;
    use crate::scheduler::checkpoint::{Checkpoint, Persist};
    use crate::scheduler::memory::Memory;
    use crate::scheduler::{Scheduler, Task};
    use crate::signals::{Kind as SignalKind, Listener as SignalListener, Signal};
    use crate::spider::{Failure, Output as SpiderOutput, Spider};
    use crate::stats::Snapshot as StatsSnapshot;
    use crate::stats::{Event as StatsEvent, Reporter as StatsReporter};
    use crate::store::Memory as MemoryStore;
    use crate::test_support::redis::spawn_redis_server;
    use crate::value::Value;
    use jiff::SignedDuration;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn engine_executes_http_task_once() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser);
        let response = block_on(engine.execute_once()).unwrap().unwrap();

        assert_eq!(response.url, "https://example.com");
        assert_eq!(response.protocol.as_deref(), Some("HTTP/1.1"));
    }

    #[test]
    fn engine_executes_browser_task_once() {
        let scheduler = Memory::default();
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
    fn engine_with_downloaders_keeps_default_dedup_behavior() {
        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine =
            Engine::with_downloaders(downloader, StubBrowser).with_store(MemoryStore::default());

        assert!(
            block_on(engine.enqueue(Request::new("https://example.com/dedup-shortcut"))).unwrap()
        );
        assert!(
            !block_on(engine.enqueue(Request::new("https://example.com/dedup-shortcut"))).unwrap()
        );

        let mut step_chains = BTreeMap::new();
        let output = block_on(engine.execute_spider_once(
            &SimpleSpider("with_downloaders"),
            None,
            &mut step_chains,
        ))
        .unwrap();

        assert!(output.is_some());
        assert_eq!(*fetches.lock().unwrap(), 1);
    }

    #[test]
    fn engine_with_http_replaces_only_http_downloader() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/http-only"))))
            .unwrap();

        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser).with_http(AltHttp);
        let response = block_on(engine.execute_once()).unwrap().unwrap();

        assert_eq!(response.protocol.as_deref(), Some("alt-http"));
    }

    #[test]
    fn engine_with_browser_replaces_only_browser_downloader() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::browser(
            "https://example.com/browser-only",
        ))))
        .unwrap();

        let mut engine =
            Engine::from_parts(scheduler, StubHttp, StubBrowser).with_browser(AltBrowser);
        let response = block_on(engine.execute_once()).unwrap().unwrap();

        assert_eq!(response.protocol.as_deref(), Some("alt-browser"));
    }

    #[test]
    fn engine_default_is_zero_config_memory_engine() {
        let engine = Engine::default();

        assert!(!block_on(engine.scheduler.has_pending()).unwrap());
    }

    #[test]
    fn engine_with_checkpoint_wraps_default_memory_scheduler() {
        let persist = TestCheckpointPersist::default();
        let engine =
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

    #[tokio::test]
    async fn engine_run_renews_redis_task_leases_while_long_tasks_are_running() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "engine_heartbeat";
        let scheduler = crate::scheduler::Redis::new(format!("redis://{url}"), namespace)
            .with_worker_id("engine-worker")
            .with_lease_timeout(SignedDuration::from_millis(20))
            .with_heartbeat_interval(SignedDuration::from_millis(10));
        let mut engine = Engine::from_parts(scheduler, AsyncDelayedHttp { delay: 60 }, StubBrowser)
            .with_settings(Settings::default().with_idle_timeout(SignedDuration::from_millis(5)))
            .with_store(MemoryStore::default());
        let shutdown = engine.shutdown_handle();
        let observer = crate::scheduler::Redis::new(format!("redis://{url}"), namespace)
            .with_worker_id("observer")
            .with_lease_timeout(SignedDuration::from_millis(20));
        let observer_task = async move {
            tokio::time::sleep(to_std_duration(SignedDuration::from_millis(30)).unwrap()).await;
            let checkpoint = observer.checkpoint().await.unwrap();

            assert!(checkpoint.ready.is_empty());
            assert_eq!(checkpoint.inflight.len(), 1);
            assert_eq!(
                checkpoint.inflight[0].request.url,
                "https://example.com/start"
            );

            tokio::time::sleep(to_std_duration(SignedDuration::from_millis(80)).unwrap()).await;
            shutdown.stop();
            observer
        };
        let (run_result, observer) = tokio::join!(engine.run(&StartUrlSpider), observer_task);
        let outputs = run_result.unwrap();
        assert_eq!(outputs.len(), 1);

        let final_checkpoint = observer.checkpoint().await.unwrap();
        assert!(!final_checkpoint.has_pending());

        observer.close().await.unwrap();
        engine.scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[test]
    fn engine_runs_download_middlewares_around_fetch() {
        let scheduler = Memory::default();
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
    fn engine_load_plugins_rejects_deferred_plugin_kinds_explicitly() {
        let mut registry = PluginRegistry::new();
        registry
            .register(PluginManifest {
                name: "sqlite".to_string(),
                kind: "store".to_string(),
                entry: "plugins_demo::SqliteStore".to_string(),
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
                .contains("only auto-loads plugin kinds [middleware]")
        );
        assert!(error.to_string().contains("(store, sqlite)"));
        assert!(error.to_string().contains(
            "known but not auto-loadable yet: [store, scheduler, dedup, robots, http, browser]"
        ));
    }

    #[test]
    fn engine_loads_runtime_middlewares_and_applies_explicit_overrides() {
        let scheduler = Memory::default();
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
        assert!(!keys.contains(&"dedup"));
    }

    #[test]
    fn engine_loads_auto_throttle_from_settings() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, HtmlHttp, StubBrowser)
            .with_settings(auto_throttle_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();
        block_on(engine.execute_spider_once(
            &SimpleSpider("auto_throttle_runtime"),
            None,
            &mut step_chains,
        ))
        .unwrap()
        .unwrap();

        let keys = step_chains
            .get("parse")
            .unwrap()
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>();

        assert!(keys.contains(&"auto_throttle"));
        assert!(!keys.contains(&"interval_gate"));
    }

    #[test]
    fn engine_loads_http_cache_from_settings() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, HtmlHttp, StubBrowser)
            .with_settings(http_cache_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();
        block_on(engine.execute_spider_once(
            &SimpleSpider("http_cache_runtime"),
            None,
            &mut step_chains,
        ))
        .unwrap()
        .unwrap();

        let keys = step_chains
            .get("parse")
            .unwrap()
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>();

        assert!(keys.contains(&"http_cache"));
    }

    #[test]
    fn engine_enqueue_dedups_duplicate_requests_before_scheduler() {
        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::from_parts(Memory::default(), downloader, StubBrowser)
            .with_store(MemoryStore::default());

        assert!(block_on(engine.enqueue(Request::new("https://example.com/dedup"))).unwrap());
        assert!(!block_on(engine.enqueue(Request::new("https://example.com/dedup"))).unwrap());

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
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                dedup_reject_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_with_noop_dedup_accepts_duplicate_requests() {
        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::from_parts(Memory::default(), downloader, StubBrowser)
            .with_dedup(crate::dedup::Noop)
            .with_store(MemoryStore::default());

        assert!(block_on(engine.enqueue(Request::new("https://example.com/dedup"))).unwrap());
        assert!(block_on(engine.enqueue(Request::new("https://example.com/dedup"))).unwrap());

        let mut step_chains = BTreeMap::new();
        let first =
            block_on(engine.execute_spider_once(&SimpleSpider("dedup"), None, &mut step_chains))
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&SimpleSpider("dedup"), None, &mut step_chains))
                .unwrap();

        assert!(first.is_some());
        assert!(second.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_with_bloom_dedup_rejects_duplicate_requests() {
        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::from_parts(Memory::default(), downloader, StubBrowser)
            .with_dedup(crate::dedup::Bloom::default())
            .with_store(MemoryStore::default());

        assert!(block_on(engine.enqueue(Request::new("https://example.com/dedup-bloom"))).unwrap());
        assert!(
            !block_on(engine.enqueue(Request::new("https://example.com/dedup-bloom"))).unwrap()
        );

        let mut step_chains = BTreeMap::new();
        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("dedup_bloom"),
            None,
            &mut step_chains,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("dedup_bloom"),
            None,
            &mut step_chains,
        ))
        .unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(*fetches.lock().unwrap(), 1);
    }

    #[test]
    fn engine_download_error_dispatches_request_errback() {
        let scheduler = Memory::default();
        block_on(
            scheduler.enqueue(Task::new(
                Request::new("https://example.com/fail")
                    .with_kwarg("page", Value::Number(3.0))
                    .with_errback("handle_failure"),
            )),
        )
        .unwrap();

        let store = MemoryStore::default();
        let mut engine = Engine::from_parts(scheduler, ErrorHttp, StubBrowser)
            .with_settings(Settings::default().with_middleware(
                "retry_by_error",
                Config {
                    enabled: false,
                    stage: Stage::Download,
                    order: 210,
                    options: BTreeMap::new(),
                },
            ))
            .with_store(store.clone());

        let mut step_chains = BTreeMap::new();
        let output = block_on(engine.execute_spider_once(&ErrbackSpider, None, &mut step_chains))
            .unwrap()
            .unwrap();

        assert_eq!(output.items.len(), 1);
        assert_eq!(
            store.items()[0].fields.get("kind"),
            Some(&Value::String("download".to_string()))
        );
        assert_eq!(
            store.items()[0].fields.get("page"),
            Some(&Value::Number(3.0))
        );
        assert_eq!(
            store.items()[0].fields.get("has_response"),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn engine_callback_error_dispatches_request_errback_with_response_context() {
        let scheduler = Memory::default();
        block_on(
            scheduler.enqueue(Task::new(
                Request::new("https://example.com/parse-error")
                    .with_kwarg("source", Value::String("detail".to_string()))
                    .with_errback("handle_failure"),
            )),
        )
        .unwrap();

        let store = MemoryStore::default();
        let mut engine =
            Engine::from_parts(scheduler, StubHttp, StubBrowser).with_store(store.clone());

        let mut step_chains = BTreeMap::new();
        let output =
            block_on(engine.execute_spider_once(&ParseErrorSpider, None, &mut step_chains))
                .unwrap()
                .unwrap();

        assert_eq!(output.items.len(), 1);
        assert_eq!(
            store.items()[0].fields.get("kind"),
            Some(&Value::String("parse".to_string()))
        );
        assert_eq!(
            store.items()[0].fields.get("source"),
            Some(&Value::String("detail".to_string()))
        );
        assert_eq!(
            store.items()[0].fields.get("has_response"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn engine_retries_on_configured_status() {
        let scheduler = Memory::default();
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
        let scheduler = Memory::default();
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
        std::thread::sleep(to_std_duration(SignedDuration::from_millis(30)).unwrap());
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
        let scheduler = Memory::default();
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
        let scheduler = Memory::default();
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
    fn engine_respects_auto_throttle_delay() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/auto/1")))).unwrap();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/auto/2")))).unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = DelayedCountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
            delays: vec![12, 0],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_settings(auto_throttle_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("auto_throttle"),
            None,
            &mut step_chains,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("auto_throttle"),
            None,
            &mut step_chains,
        ))
        .unwrap();
        let mut third = None;

        for _ in 0..10 {
            std::thread::sleep(to_std_duration(SignedDuration::from_millis(10)).unwrap());
            third = block_on(engine.execute_spider_once(
                &SimpleSpider("auto_throttle"),
                None,
                &mut step_chains,
            ))
            .unwrap();

            if third.is_some() {
                break;
            }
        }

        assert!(first.is_some());
        assert!(second.is_none());
        assert!(third.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_http_cache_reuses_cached_response_on_not_modified() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/cache")))).unwrap();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/cache")))).unwrap();

        let seen_headers = Arc::new(Mutex::new(Vec::<Headers>::new()));
        let downloader = ConditionalCacheHttp {
            seen_headers: seen_headers.clone(),
            fetches: Arc::new(Mutex::new(0)),
        };

        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_settings(http_cache_settings())
            .with_store(MemoryStore::default());
        let mut step_chains = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&ResponseInspectSpider, None, &mut step_chains))
                .unwrap()
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&ResponseInspectSpider, None, &mut step_chains))
                .unwrap()
                .unwrap();

        assert_eq!(
            first.items[0].fields.get("status"),
            Some(&Value::Number(200.0))
        );
        assert_eq!(
            second.items[0].fields.get("status"),
            Some(&Value::Number(200.0))
        );
        assert_eq!(
            second.items[0].fields.get("text"),
            Some(&Value::String("cached-body".to_string()))
        );
        assert_eq!(
            second.items[0].fields.get("flags"),
            Some(&Value::Array(vec![Value::String("http_cache".to_string())]))
        );

        let headers = seen_headers.lock().unwrap();
        assert_eq!(
            headers[1]
                .get("If-None-Match")
                .and_then(|values| values.first())
                .map(String::as_str),
            Some("v1")
        );
        assert_eq!(
            headers[1]
                .get("If-Modified-Since")
                .and_then(|values| values.first())
                .map(String::as_str),
            Some("Wed, 21 Oct 2015 07:28:00 GMT")
        );
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 2,
                response_count: 2,
                item_count: 2,
                http_cache_hit_count: 1,
                http_cache_store_count: 1,
                http_cache_miss_count: 1,
                http_cache_revalidate_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_pipeline_keeps_items_and_memory_store_writes_them() {
        let scheduler = Memory::default();
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
                item_count: 1,
                response_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_store_prefers_batch_write_for_kept_items() {
        let scheduler = Memory::default();
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
        let scheduler = Memory::default();
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
                pipeline_drop_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_pipeline_error_fails_task_explicitly() {
        let scheduler = Memory::default();
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
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_store_error_increments_stats() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_store(FailStore)
            .with_pipeline(PassPipeline);
        let mut step_chains = BTreeMap::new();

        let error =
            block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_chains)).unwrap_err();

        assert!(error.to_string().contains("store failed"));
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                error_count: 1,
                store_error_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_with_stats_reporter_receives_runtime_events() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let reporter = RecordingReporter::default();
        let recorded = reporter.events.clone();
        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_stats_reporter(reporter);
        let mut step_chains = BTreeMap::new();

        block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_chains)).unwrap();

        let events = recorded.lock().unwrap().clone();
        assert_eq!(
            events,
            vec![
                (
                    StatsEvent::Request,
                    StatsSnapshot {
                        request_count: 1,
                        ..StatsSnapshot::default()
                    },
                ),
                (
                    StatsEvent::Response,
                    StatsSnapshot {
                        request_count: 1,
                        response_count: 1,
                        ..StatsSnapshot::default()
                    },
                ),
                (
                    StatsEvent::Item,
                    StatsSnapshot {
                        request_count: 1,
                        response_count: 1,
                        item_count: 1,
                        ..StatsSnapshot::default()
                    },
                ),
            ]
        );
    }

    #[test]
    fn engine_with_signal_listener_receives_request_response_item_signals() {
        let listener = RecordingSignalListener::default();
        let recorded_events = listener.events.clone();
        let recorded_urls = listener.request_urls.clone();
        let recorded_titles = listener.item_titles.clone();

        let mut engine = Engine::from_parts(Memory::default(), StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_signal_listener(listener);

        block_on(engine.enqueue(Request::new("https://example.com/item"))).unwrap();

        let mut step_chains = BTreeMap::new();
        block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_chains)).unwrap();

        assert_eq!(
            recorded_events.lock().unwrap().clone(),
            vec![
                SignalKind::RequestScheduled,
                SignalKind::ResponseReceived,
                SignalKind::ItemScraped,
            ]
        );
        assert_eq!(
            recorded_urls.lock().unwrap().clone(),
            vec!["https://example.com/item".to_string()]
        );
        assert_eq!(
            recorded_titles.lock().unwrap().clone(),
            vec!["post".to_string()]
        );
    }

    #[test]
    fn engine_with_signal_listener_receives_spider_error_signal() {
        let listener = RecordingSignalListener::default();
        let recorded_events = listener.events.clone();
        let recorded_errors = listener.errors.clone();

        let mut engine = Engine::from_parts(Memory::default(), StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_signal_listener(listener);

        block_on(engine.enqueue(Request::new("https://example.com/error"))).unwrap();

        let mut step_chains = BTreeMap::new();
        let error = block_on(engine.execute_spider_once(&FailingSpider, None, &mut step_chains))
            .unwrap_err();

        assert_eq!(error, SpiderError::parse("parse failed"));
        assert_eq!(
            recorded_events.lock().unwrap().clone(),
            vec![
                SignalKind::RequestScheduled,
                SignalKind::ResponseReceived,
                SignalKind::SpiderError,
            ]
        );
        assert_eq!(
            recorded_errors.lock().unwrap().clone(),
            vec!["parse error: parse failed".to_string()]
        );
    }

    #[test]
    fn engine_with_extension_registers_extension_on_signal_bus() {
        let extension = RecordingExtension::default();
        let recorded_events = extension.events.clone();

        let mut engine = Engine::from_parts(Memory::default(), StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_extension(extension);

        block_on(engine.enqueue(Request::new("https://example.com/item"))).unwrap();

        let mut step_chains = BTreeMap::new();
        block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_chains)).unwrap();

        assert_eq!(
            recorded_events.lock().unwrap().clone(),
            vec![
                SignalKind::RequestScheduled,
                SignalKind::ResponseReceived,
                SignalKind::ItemScraped,
            ]
        );
    }

    #[tokio::test]
    async fn engine_run_emits_spider_opened_and_closed_signals() {
        let listener = RecordingSignalListener::default();
        let recorded_events = listener.events.clone();

        let mut engine = Engine::from_parts(Memory::default(), StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_signal_listener(listener)
            .with_settings(Settings::default().with_idle_timeout(SignedDuration::from_millis(5)));

        let shutdown = engine.shutdown_handle();
        tokio::spawn(async move {
            tokio::time::sleep(to_std_duration(SignedDuration::from_millis(20)).unwrap()).await;
            shutdown.stop();
        });

        let outputs = engine.run(&IdleSpider).await.unwrap();

        assert!(outputs.is_empty());
        assert_eq!(
            recorded_events.lock().unwrap().clone(),
            vec![SignalKind::SpiderOpened, SignalKind::SpiderClosed]
        );
    }

    #[test]
    fn engine_stats_track_retries_across_attempts() {
        let scheduler = Memory::default();
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
                retry_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_skips_disallowed_request_when_robots_obey_is_enabled() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/private/page"))))
            .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_robots(BlockPrivate)
            .with_settings(
                Settings::default()
                    .with_robots_obey(true)
                    .with_robots_user_agent("kun-bot"),
            )
            .with_store(MemoryStore::default());

        let mut step_chains = BTreeMap::new();
        let output =
            block_on(engine.execute_spider_once(&SimpleSpider("robots"), None, &mut step_chains))
                .unwrap();

        assert!(output.is_none());
        assert_eq!(*fetches.lock().unwrap(), 0);
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                robots_disallow_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_respects_robots_crawl_delay_when_robots_obey_is_enabled() {
        let robot = crate::robots::Memory::default();
        block_on(robot.seed_from_body(
            "https://example.com/news/1",
            "User-agent: *\nAllow: /\nCrawl-delay: 0.01\n",
        ));

        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/news/1")))).unwrap();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/news/2")))).unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_robots(robot)
            .with_settings(
                Settings::default()
                    .with_robots_obey(true)
                    .with_robots_user_agent("kun-bot"),
            )
            .with_store(MemoryStore::default());

        let mut step_chains = BTreeMap::new();
        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("robots_crawl_delay"),
            None,
            &mut step_chains,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("robots_crawl_delay"),
            None,
            &mut step_chains,
        ))
        .unwrap();
        std::thread::sleep(to_std_duration(SignedDuration::from_millis(15)).unwrap());
        let third = block_on(engine.execute_spider_once(
            &SimpleSpider("robots_crawl_delay"),
            None,
            &mut step_chains,
        ))
        .unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert!(third.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 2,
                response_count: 2,
                retry_count: 1,
                robots_delay_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_enqueues_robots_sitemap_seed_requests_through_dedup() {
        let mut engine = Engine::from_parts(
            Memory::default(),
            SitemapHttp::new([(
                "https://example.com/sitemap.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://example.com/start</loc></url>
  <url><loc>https://example.com/from-sitemap</loc></url>
</urlset>"#,
            )]),
            StubBrowser,
        )
        .with_robots(StaticSitemaps::new(["https://example.com/sitemap.xml"]))
        .with_settings(Settings::default().with_robots_sitemap_seeds(true))
        .with_store(MemoryStore::default());

        block_on(engine.enqueue_start_requests(&StartUrlSpider, &[])).unwrap();

        let checkpoint = engine.scheduler.checkpoint();
        let urls = checkpoint
            .ready
            .iter()
            .map(|task| task.request.url.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            urls,
            vec![
                "https://example.com/start".to_string(),
                "https://example.com/from-sitemap".to_string(),
            ]
        );
        assert!(checkpoint.ready.iter().all(|task| task.priority == 0));
        assert!(checkpoint.ready.iter().all(|task| task.depth == 0));
    }

    #[test]
    fn engine_enqueues_nested_robots_sitemaps_as_seed_requests() {
        let mut engine = Engine::from_parts(
            Memory::default(),
            SitemapHttp::new([
                (
                    "https://example.com/root.xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap><loc>https://example.com/news.xml</loc></sitemap>
</sitemapindex>"#,
                ),
                (
                    "https://example.com/news.xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://example.com/news/1</loc></url>
</urlset>"#,
                ),
            ]),
            StubBrowser,
        )
        .with_robots(StaticSitemaps::new(["https://example.com/root.xml"]))
        .with_settings(Settings::default().with_robots_sitemap_seeds(true))
        .with_store(MemoryStore::default());

        block_on(engine.enqueue_start_requests(&StartUrlSpider, &[])).unwrap();

        let checkpoint = engine.scheduler.checkpoint();
        let urls = checkpoint
            .ready
            .iter()
            .map(|task| task.request.url.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            urls,
            vec![
                "https://example.com/start".to_string(),
                "https://example.com/news/1".to_string(),
            ]
        );
    }

    #[test]
    fn engine_applies_configured_priority_and_depth_to_robots_sitemap_seed_requests() {
        let mut engine = Engine::from_parts(
            Memory::default(),
            SitemapHttp::new([(
                "https://example.com/sitemap.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://example.com/from-sitemap</loc></url>
</urlset>"#,
            )]),
            StubBrowser,
        )
        .with_robots(StaticSitemaps::new(["https://example.com/sitemap.xml"]))
        .with_settings(
            Settings::default()
                .with_robots_sitemap_seeds(true)
                .with_robots_sitemap_seed_priority(12)
                .with_robots_sitemap_seed_depth(2),
        )
        .with_store(MemoryStore::default());

        block_on(engine.enqueue_start_requests(&StartUrlSpider, &[])).unwrap();

        let checkpoint = engine.scheduler.checkpoint();
        let sitemap_task = checkpoint
            .ready
            .iter()
            .find(|task| task.request.url == "https://example.com/from-sitemap")
            .unwrap();

        assert_eq!(sitemap_task.priority, 12);
        assert_eq!(sitemap_task.depth, 2);
    }

    #[test]
    fn engine_enqueues_custom_start_requests_with_full_request_semantics() {
        let mut engine = Engine::from_parts(Memory::default(), HtmlHttp, StubBrowser)
            .with_store(MemoryStore::default());

        block_on(engine.enqueue_start_requests(&CustomStartRequestSpider, &[])).unwrap();

        let checkpoint = engine.scheduler.checkpoint();
        let request = &checkpoint.ready[0].request;

        assert_eq!(request.url, "https://example.com/start");
        assert_eq!(request.mode, RequestMode::Browser);
        assert_eq!(
            request.headers.get("x-token"),
            Some(&vec!["abc".to_string()])
        );
        assert_eq!(
            request.cookies.get("sid").map(String::as_str),
            Some("cookie-1")
        );
        assert_eq!(request.timeout, Some(SignedDuration::from_secs(5)));
        assert_eq!(
            request.proxy.as_ref().map(|proxy| proxy.url.as_str()),
            Some("http://proxy.internal:8080")
        );
        assert_eq!(
            request.session.as_ref().map(|session| session.id.as_str()),
            Some("shared-session")
        );
    }

    #[test]
    fn engine_enqueues_robots_sitemap_seed_requests_inheriting_start_request_semantics() {
        let mut engine = Engine::from_parts(
            Memory::default(),
            SitemapHttp::new([(
                "https://example.com/sitemap.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://example.com/from-sitemap</loc></url>
</urlset>"#,
            )]),
            StubBrowser,
        )
        .with_robots(StaticSitemaps::new(["https://example.com/sitemap.xml"]))
        .with_settings(Settings::default().with_robots_sitemap_seeds(true))
        .with_store(MemoryStore::default());

        block_on(engine.enqueue_start_requests(&CustomStartRequestSpider, &[])).unwrap();

        let checkpoint = engine.scheduler.checkpoint();
        let request = checkpoint
            .ready
            .iter()
            .find(|task| task.request.url == "https://example.com/from-sitemap")
            .map(|task| &task.request)
            .unwrap();

        assert_eq!(request.mode, RequestMode::Browser);
        assert_eq!(
            request.headers.get("x-token"),
            Some(&vec!["abc".to_string()])
        );
        assert_eq!(
            request.cookies.get("sid").map(String::as_str),
            Some("cookie-1")
        );
        assert_eq!(request.timeout, Some(SignedDuration::from_secs(5)));
        assert_eq!(
            request.proxy.as_ref().map(|proxy| proxy.url.as_str()),
            Some("http://proxy.internal:8080")
        );
        assert_eq!(
            request.session.as_ref().map(|session| session.id.as_str()),
            Some("shared-session")
        );
    }

    #[test]
    fn engine_fetches_robots_sitemap_with_http_mode_and_inherited_shared_request_semantics() {
        let recorder = Arc::new(Mutex::new(None));
        let mut engine = Engine::from_parts(
            Memory::default(),
            InspectingSitemapHttp::new(
                recorder.clone(),
                r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://example.com/from-sitemap</loc></url>
</urlset>"#,
            ),
            StubBrowser,
        )
        .with_robots(StaticSitemaps::new(["https://example.com/sitemap.xml"]))
        .with_settings(Settings::default().with_robots_sitemap_seeds(true))
        .with_store(MemoryStore::default());

        block_on(engine.enqueue_start_requests(&CustomStartRequestSpider, &[])).unwrap();

        let request = recorder.lock().unwrap().clone().unwrap();
        assert_eq!(request.url, "https://example.com/sitemap.xml");
        assert_eq!(request.mode, RequestMode::Http);
        assert_eq!(
            request.headers.get("x-token"),
            Some(&vec!["abc".to_string()])
        );
        assert_eq!(
            request.cookies.get("sid").map(String::as_str),
            Some("cookie-1")
        );
        assert_eq!(request.timeout, Some(SignedDuration::from_secs(5)));
        assert_eq!(
            request.proxy.as_ref().map(|proxy| proxy.url.as_str()),
            Some("http://proxy.internal:8080")
        );
        assert_eq!(
            request.session.as_ref().map(|session| session.id.as_str()),
            Some("shared-session")
        );
        assert!(request.http.is_some());
        assert!(request.browser.is_none());
    }

    #[derive(Clone, Copy)]
    struct BlockPrivate;

    impl crate::robots::Robot for BlockPrivate {
        fn is_allowed<'a>(
            &'a self,
            request: &'a Request,
            _user_agent: &'a str,
        ) -> BoxFuture<'a, Result<bool, SpiderError>> {
            Box::pin(async move { Ok(!request.url.contains("/private")) })
        }
    }

    struct StaticSitemaps {
        urls: Vec<String>,
    }

    impl StaticSitemaps {
        fn new<const N: usize>(urls: [&str; N]) -> Self {
            Self {
                urls: urls.into_iter().map(str::to_string).collect(),
            }
        }
    }

    impl crate::robots::Robot for StaticSitemaps {
        fn is_allowed<'a>(
            &'a self,
            _request: &'a Request,
            _user_agent: &'a str,
        ) -> BoxFuture<'a, Result<bool, SpiderError>> {
            Box::pin(async { Ok(true) })
        }

        fn sitemaps<'a>(
            &'a self,
            _request: &'a Request,
        ) -> BoxFuture<'a, Result<Vec<String>, SpiderError>> {
            Box::pin(async move { Ok(self.urls.clone()) })
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

    struct AltHttp;

    impl crate::download::traits::Downloader for AltHttp {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            let mut response =
                Response::from_request(request.clone(), 200, Default::default(), Vec::new());
            response.protocol = Some("alt-http".to_string());
            Ok(response)
        }
    }

    struct AltBrowser;

    impl crate::download::traits::Downloader for AltBrowser {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            let mut response =
                Response::from_request(request.clone(), 200, Default::default(), Vec::new());
            response.protocol = Some("alt-browser".to_string());
            response.flags.push("browser".to_string());
            Ok(response)
        }
    }

    struct ErrorHttp;

    impl crate::download::traits::Downloader for ErrorHttp {
        async fn fetch(&self, _request: &Request) -> Result<Response, SpiderError> {
            Err(SpiderError::download("network down"))
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

    struct SitemapHttp {
        bodies: BTreeMap<String, Vec<u8>>,
    }

    impl SitemapHttp {
        fn new<I, K, V>(entries: I) -> Self
        where
            I: IntoIterator<Item = (K, V)>,
            K: Into<String>,
            V: AsRef<[u8]>,
        {
            Self {
                bodies: entries
                    .into_iter()
                    .map(|(url, body)| (url.into(), body.as_ref().to_vec()))
                    .collect(),
            }
        }
    }

    impl crate::download::traits::Downloader for SitemapHttp {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            let body = self.bodies.get(&request.url).cloned().unwrap_or_default();

            Ok(Response::from_request(
                request.clone(),
                200,
                [(
                    "content-type".to_string(),
                    vec!["application/xml; charset=utf-8".to_string()],
                )]
                .into_iter()
                .collect(),
                body,
            ))
        }
    }

    struct InspectingSitemapHttp {
        request: Arc<Mutex<Option<Request>>>,
        body: Vec<u8>,
    }

    impl InspectingSitemapHttp {
        fn new(request: Arc<Mutex<Option<Request>>>, body: impl AsRef<[u8]>) -> Self {
            Self {
                request,
                body: body.as_ref().to_vec(),
            }
        }
    }

    impl crate::download::traits::Downloader for InspectingSitemapHttp {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            *self.request.lock().unwrap() = Some(request.clone());
            Ok(Response::from_request(
                request.clone(),
                200,
                [(
                    "content-type".to_string(),
                    vec!["application/xml; charset=utf-8".to_string()],
                )]
                .into_iter()
                .collect(),
                self.body.clone(),
            ))
        }
    }

    #[test]
    fn engine_enqueues_gzipped_robots_sitemap_seed_requests() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://example.com/from-gzip</loc></url>
</urlset>"#;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        use std::io::Write as _;
        encoder.write_all(xml.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut engine = Engine::from_parts(
            Memory::default(),
            SitemapHttp::new([("https://example.com/sitemap.xml.gz", compressed)]),
            StubBrowser,
        )
        .with_robots(StaticSitemaps::new(["https://example.com/sitemap.xml.gz"]))
        .with_settings(Settings::default().with_robots_sitemap_seeds(true))
        .with_store(MemoryStore::default());

        block_on(engine.enqueue_start_requests(&StartUrlSpider, &[])).unwrap();

        let checkpoint = engine.scheduler.checkpoint();
        let urls = checkpoint
            .ready
            .iter()
            .map(|task| task.request.url.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            urls,
            vec![
                "https://example.com/start".to_string(),
                "https://example.com/from-gzip".to_string(),
            ]
        );
    }

    struct DelayedCountHttp {
        fetches: Arc<Mutex<usize>>,
        statuses: Vec<u16>,
        delays: Vec<i64>,
    }

    impl crate::download::traits::Downloader for DelayedCountHttp {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            let mut fetches = self.fetches.lock().unwrap();
            let index = *fetches;
            *fetches += 1;
            drop(fetches);

            let delay = self.delays.get(index).copied().unwrap_or_default();
            if delay > 0 {
                std::thread::sleep(to_std_duration(SignedDuration::from_millis(delay)).unwrap());
            }

            let status = self.statuses.get(index).copied().unwrap_or(200);
            Ok(Response::from_request(
                request.clone(),
                status,
                Default::default(),
                Vec::new(),
            ))
        }
    }

    struct AsyncDelayedHttp {
        delay: i64,
    }

    impl crate::download::traits::Downloader for AsyncDelayedHttp {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            if self.delay > 0 {
                tokio::time::sleep(
                    to_std_duration(SignedDuration::from_millis(self.delay)).unwrap(),
                )
                .await;
            }

            Ok(Response::from_request(
                request.clone(),
                200,
                Default::default(),
                Vec::new(),
            ))
        }
    }

    struct ConditionalCacheHttp {
        seen_headers: Arc<Mutex<Vec<Headers>>>,
        fetches: Arc<Mutex<usize>>,
    }

    impl crate::download::traits::Downloader for ConditionalCacheHttp {
        async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
            self.seen_headers
                .lock()
                .unwrap()
                .push(request.headers.clone());

            let mut fetches = self.fetches.lock().unwrap();
            let index = *fetches;
            *fetches += 1;
            drop(fetches);

            if index == 0 {
                return Ok(Response::from_request(
                    request.clone(),
                    200,
                    [
                        ("ETag".to_string(), vec!["v1".to_string()]),
                        (
                            "Last-Modified".to_string(),
                            vec!["Wed, 21 Oct 2015 07:28:00 GMT".to_string()],
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    b"cached-body".to_vec(),
                ));
            }

            Ok(Response::from_request(
                request.clone(),
                304,
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

    struct StartUrlSpider;

    impl Spider for StartUrlSpider {
        fn name(&self) -> &str {
            "start_url_spider"
        }

        fn start_urls(&self) -> Vec<String> {
            vec!["https://example.com/start".to_string()]
        }
    }

    struct CustomStartRequestSpider;

    impl Spider for CustomStartRequestSpider {
        fn name(&self) -> &str {
            "custom_start_request_spider"
        }

        fn build_start_requests(&self) -> Vec<Request> {
            vec![
                Request::browser("https://example.com/start")
                    .with_header("x-token", "abc")
                    .with_cookie("sid", "cookie-1")
                    .with_timeout(SignedDuration::from_secs(5))
                    .with_proxy("http://proxy.internal:8080")
                    .with_session("shared-session"),
            ]
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

    struct FailingSpider;

    impl Spider for FailingSpider {
        fn name(&self) -> &str {
            "failing_spider"
        }

        async fn parse(&self, _response: &Response) -> Result<SpiderOutput, SpiderError> {
            Err(SpiderError::parse("parse failed"))
        }
    }

    struct IdleSpider;

    impl Spider for IdleSpider {
        fn name(&self) -> &str {
            "idle_spider"
        }
    }

    struct ResponseInspectSpider;

    impl Spider for ResponseInspectSpider {
        fn name(&self) -> &str {
            "response_inspect_spider"
        }

        async fn parse(&self, response: &Response) -> Result<SpiderOutput, SpiderError> {
            Ok(SpiderOutput {
                items: vec![
                    crate::item::Item::new()
                        .with_field("status", Value::Number(response.status as f64))
                        .with_field("text", Value::String(response.text.clone()))
                        .with_field(
                            "flags",
                            Value::Array(
                                response.flags.iter().cloned().map(Value::String).collect(),
                            ),
                        ),
                ],
                requests: Vec::new(),
            })
        }
    }

    #[derive(Clone, Default)]
    struct RecordingReporter {
        events: Arc<Mutex<Vec<(StatsEvent, StatsSnapshot)>>>,
    }

    impl StatsReporter for RecordingReporter {
        fn report(&self, event: StatsEvent, snapshot: StatsSnapshot) {
            self.events.lock().unwrap().push((event, snapshot));
        }
    }

    #[derive(Clone, Default)]
    struct RecordingSignalListener {
        events: Arc<Mutex<Vec<SignalKind>>>,
        request_urls: Arc<Mutex<Vec<String>>>,
        item_titles: Arc<Mutex<Vec<String>>>,
        errors: Arc<Mutex<Vec<String>>>,
    }

    impl SignalListener for RecordingSignalListener {
        fn on_signal<'a>(&'a self, signal: &'a Signal) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                self.events.lock().unwrap().push(signal.kind());

                match signal {
                    Signal::RequestScheduled(signal) => {
                        self.request_urls
                            .lock()
                            .unwrap()
                            .push(signal.request.url.clone());
                    }
                    Signal::ItemScraped(signal) => {
                        if let Some(Value::String(title)) = signal.item.get("title") {
                            self.item_titles.lock().unwrap().push(title.clone());
                        }
                    }
                    Signal::SpiderError(signal) => {
                        self.errors.lock().unwrap().push(signal.error.to_string());
                    }
                    _ => {}
                }
            })
        }
    }

    #[derive(Clone, Default)]
    struct RecordingExtension {
        events: Arc<Mutex<Vec<SignalKind>>>,
    }

    impl crate::extensions::Extension for RecordingExtension {
        fn on_signal<'a>(&'a self, signal: &'a Signal) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                self.events.lock().unwrap().push(signal.kind());
            })
        }
    }

    struct ErrbackSpider;

    impl Spider for ErrbackSpider {
        fn name(&self) -> &str {
            "errback_spider"
        }

        async fn handle_error(
            &self,
            name: &str,
            failure: &Failure,
        ) -> Result<SpiderOutput, SpiderError> {
            match name {
                "handle_failure" => Ok(SpiderOutput {
                    items: vec![
                        crate::item::Item::new()
                            .with_field(
                                "kind",
                                Value::String(match failure.error {
                                    SpiderError::Download(_) => "download".to_string(),
                                    SpiderError::Parse(_) => "parse".to_string(),
                                    _ => "other".to_string(),
                                }),
                            )
                            .with_field("has_response", Value::Bool(failure.response.is_some()))
                            .with_field(
                                "page",
                                failure.kwarg("page").cloned().unwrap_or(Value::Null),
                            )
                            .with_field(
                                "source",
                                failure.kwarg("source").cloned().unwrap_or(Value::Null),
                            ),
                    ],
                    requests: Vec::new(),
                }),
                other => Err(SpiderError::engine(format!("unknown errback: {other}"))),
            }
        }
    }

    struct ParseErrorSpider;

    impl Spider for ParseErrorSpider {
        fn name(&self) -> &str {
            "parse_error_spider"
        }

        async fn parse(&self, _response: &Response) -> Result<SpiderOutput, SpiderError> {
            Err(SpiderError::parse("parse exploded"))
        }

        async fn handle_error(
            &self,
            name: &str,
            failure: &Failure,
        ) -> Result<SpiderOutput, SpiderError> {
            ErrbackSpider.handle_error(name, failure).await
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

    struct PassPipeline;

    #[derive(Clone, Copy)]
    struct FailStore;

    impl Pipeline for FailPipeline {
        async fn process(
            &self,
            _item: &mut crate::item::Item,
            _spider_name: &str,
        ) -> Result<bool, SpiderError> {
            Err(SpiderError::engine("pipeline failed"))
        }
    }

    impl Pipeline for PassPipeline {
        async fn process(
            &self,
            _item: &mut crate::item::Item,
            _spider_name: &str,
        ) -> Result<bool, SpiderError> {
            Ok(true)
        }
    }

    impl crate::store::Store for FailStore {
        async fn write(
            &self,
            _item: &crate::item::Item,
            _spider_name: &str,
        ) -> Result<(), SpiderError> {
            Err(SpiderError::engine("store failed"))
        }

        async fn batch_write(
            &self,
            _items: &[crate::item::Item],
            _spider_name: &str,
        ) -> Result<(), SpiderError> {
            Err(SpiderError::engine("store failed"))
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
                    ("interval".to_string(), Value::Number(1000.0)),
                    ("rate_per_minute".to_string(), Value::Number(60.0)),
                ]
                .into_iter()
                .collect(),
                retry: [("count".to_string(), Value::Number(3.0))]
                    .into_iter()
                    .collect(),
                dedup: BTreeMap::new(),
            })
            .with_middlewares(BTreeMap::new())
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
                    "backoff".to_string(),
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
            schedule: [("interval".to_string(), Value::Number(10.0))]
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

    fn auto_throttle_settings() -> Settings {
        Settings::default()
            .with_auto_throttle(true)
            .with_auto_throttle_target_concurrency(1.0)
            .with_download_delay(SignedDuration::from_millis(0))
            .with_auto_throttle_max_delay(SignedDuration::from_millis(500))
    }

    fn http_cache_settings() -> Settings {
        Settings::default().with_http_cache(true)
    }
}
