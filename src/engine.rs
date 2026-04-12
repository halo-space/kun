pub mod context;
pub mod flow;
mod step;
mod task;

use crate::download::traits::Downloader;
use crate::engine::step::StepExecute;
use crate::engine::task::{
    TaskExecutor, TaskRun, TaskRunReservation, apply_task_run, enqueue_request_with_middleware,
    enqueue_task_with_middleware, record_scheduler_event,
};
use crate::error::SpiderError;
use crate::middleware::{
    Chain, DEDUP, DedupMiddleware, Registry, SharedState, Stage, build_with_shared,
};
use crate::request::{Request, RequestMode};
use crate::rules::Compiled;
use crate::scheduler::{Scheduler, Task};
use crate::settings::Config;
use crate::spider::Spider;
use crate::store::{DEFAULT_STORE_KEY, StoreEntry};
use crate::validator::StepValidator;
use futures::stream::{FuturesUnordered, StreamExt};
use jiff::SignedDuration;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use url::Url;

#[cfg(test)]
use crate::engine::task::{
    TaskOutcome, prepare_task_for_enqueue_with_middleware, resolve_scheduler_transition,
    run_middleware_after_download, run_middleware_before_download, run_middleware_before_parse,
    run_middleware_download_error,
};

pub struct Engine<S, H, B, P = ()> {
    pub scheduler: S,
    pub http: H,
    pub browser: B,
    pub pipeline: P,
    stores: BTreeMap<String, crate::store::SharedStore>,
    robots: Arc<dyn crate::robots::Robot>,
    stats: Arc<crate::stats::Tracker>,
    signals: Arc<crate::signals::Bus>,
    pub config: Config,
    pub middleware: Chain,
    pub plugins: Registry,
    prepared: bool,
    shutdown: Arc<AtomicBool>,
}

fn to_std_duration(duration: SignedDuration) -> Result<std::time::Duration, String> {
    std::time::Duration::try_from(duration).map_err(|error| error.to_string())
}

fn default_dedup_config() -> crate::middleware::Config {
    crate::middleware::Config {
        enabled: true,
        stage: Stage::Enqueue,
        order: 100,
        options: BTreeMap::new(),
    }
}

fn default_engine_middleware() -> Chain {
    let mut chain = Chain::default();
    chain.push(DEDUP, default_dedup_config(), DedupMiddleware::memory());
    chain
}

fn default_stores() -> BTreeMap<String, crate::store::SharedStore> {
    let mut stores = BTreeMap::new();
    stores.insert(
        DEFAULT_STORE_KEY.to_string(),
        crate::store::shared_store(crate::store::File::default()),
    );
    stores
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
            stores: default_stores(),
            robots: Arc::new(crate::robots::Memory::default()),
            stats: Arc::new(crate::stats::Tracker::default()),
            signals: Arc::new(crate::signals::Bus::default()),
            config: Config::default(),
            middleware: default_engine_middleware(),
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
    /// - enqueue dedup middleware: `dedup::Memory`
    /// - robots: `robots::Memory`
    /// - default store (`default`): `store::File`
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

impl<S, H, B, P> Engine<S, H, B, P>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
{
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// Replace the middleware chain while keeping the current engine
    /// configuration.
    pub fn with_middleware(mut self, middleware: Chain) -> Self {
        self.middleware = middleware;
        self
    }

    /// Replace the scheduler while keeping the current engine configuration.
    pub fn with_scheduler<S2: Scheduler>(self, scheduler: S2) -> Engine<S2, H, B, P> {
        Engine {
            scheduler,
            http: self.http,
            browser: self.browser,
            pipeline: self.pipeline,
            stores: self.stores,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            config: self.config,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Replace only the HTTP downloader while keeping the current engine
    /// configuration.
    pub fn with_http<H2: Downloader>(self, http: H2) -> Engine<S, H2, B, P> {
        Engine {
            scheduler: self.scheduler,
            http,
            browser: self.browser,
            pipeline: self.pipeline,
            stores: self.stores,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            config: self.config,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Replace only the browser downloader while keeping the current engine
    /// configuration.
    pub fn with_browser<B2: Downloader>(self, browser: B2) -> Engine<S, H, B2, P> {
        Engine {
            scheduler: self.scheduler,
            http: self.http,
            browser,
            pipeline: self.pipeline,
            stores: self.stores,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            config: self.config,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Replace the enqueue-stage request dedup middleware.
    pub fn with_dedup(mut self, dedup: impl crate::middleware::dedup::Dedup + 'static) -> Self {
        self.middleware
            .upsert(DEDUP, default_dedup_config(), DedupMiddleware::new(dedup));
        self
    }

    /// Replace the item pipeline while keeping the current engine
    /// configuration.
    pub fn with_pipeline<P2: crate::pipeline::Pipeline>(self, pipeline: P2) -> Engine<S, H, B, P2> {
        Engine {
            scheduler: self.scheduler,
            http: self.http,
            browser: self.browser,
            pipeline,
            stores: self.stores,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            config: self.config,
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

    /// Replace the default final item store under the `default` key while
    /// keeping the current engine configuration.
    pub fn with_store(mut self, store: impl crate::store::Store + 'static) -> Self {
        self.stores.insert(
            DEFAULT_STORE_KEY.to_string(),
            crate::store::shared_store(store),
        );
        self
    }

    /// Register additional keyed item stores in the engine store registry.
    pub fn with_stores<I>(mut self, stores: I) -> Self
    where
        I: IntoIterator<Item = StoreEntry>,
    {
        for entry in stores {
            self.stores.insert(entry.key, entry.store);
        }
        self
    }

    /// Register a custom middleware instance directly on the engine-level chain.
    ///
    /// This middleware applies to every request and response.
    ///
    /// ```ignore
    /// engine.add_middleware(
    ///     "custom_ua",
    ///     middleware::Config { enabled: true, stage: Stage::Download, order: 50, .. },
    ///     Box::new(MyUaMiddleware),
    /// );
    /// ```
    pub fn add_middleware(
        mut self,
        key: impl Into<String>,
        config: crate::middleware::Config,
        middleware: impl crate::middleware::Middleware + 'static,
    ) -> Self {
        self.middleware.push(key, config, middleware);
        self
    }

    /// Register a custom middleware factory.
    ///
    /// After registration, the same key can be referenced from
    /// `Config::request.middleware` or a DSL `MIDDLEWARES` section, and the engine
    /// will call the factory automatically to create the instance.
    ///
    /// ```ignore
    /// engine.register_middleware("custom_ua", |options| {
    ///     Ok(Box::new(MyUaMiddleware::new(options)))
    /// });
    /// ```
    pub fn register_middleware<M>(
        mut self,
        key: impl Into<String>,
        factory: impl Fn(
            &std::collections::BTreeMap<String, crate::value::Value>,
        ) -> Result<M, SpiderError>
        + Send
        + Sync
        + 'static,
    ) -> Self
    where
        M: crate::middleware::Middleware + 'static,
    {
        self.plugins.register(key, factory);
        self
    }

    /// Enqueue one request through the engine middleware admission chain
    /// before it reaches the scheduler.
    pub async fn enqueue(&mut self, request: crate::request::Request) -> Result<bool, SpiderError> {
        let default_chain = Chain::default();
        let enqueued = enqueue_request_with_middleware(
            &mut self.scheduler,
            &self.middleware,
            &default_chain,
            request.clone(),
            &[],
            Some("manual"),
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

    /// Attach one unified telemetry exporter to both engine stats updates and
    /// scheduler runtime events.
    ///
    /// This automatically wraps the current scheduler in
    /// `scheduler::Observed`, so one exporter can observe:
    ///
    /// - `Engine::stats()` counter updates
    /// - scheduler claim / complete / requeue / heartbeat / reclaim events
    ///
    /// ```ignore
    /// let telemetry = halo_spider::telemetry::Collector::default();
    /// let engine = halo_spider::engine::Engine::new().with_telemetry(telemetry.clone());
    /// ```
    pub fn with_telemetry<T>(self, exporter: T) -> Engine<crate::scheduler::Observed<S>, H, B, P>
    where
        T: crate::telemetry::Exporter + 'static,
    {
        let exporter = Arc::new(exporter);
        self.stats.add_reporter(exporter.clone());

        let scheduler = crate::scheduler::Observed::new(self.scheduler);
        scheduler.add_reporter(exporter.clone());

        Engine {
            scheduler,
            http: self.http,
            browser: self.browser,
            pipeline: self.pipeline,
            stores: self.stores,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            config: self.config,
            middleware: self.middleware,
            plugins: self.plugins,
            prepared: self.prepared,
            shutdown: self.shutdown,
        }
    }

    /// Register an async signal listener for engine lifecycle and runtime
    /// events.
    pub fn with_signal_listener(self, listener: impl crate::signals::Listener + 'static) -> Self {
        self.signals.add_listener(Arc::new(listener));
        self
    }

    /// Register an async signal listener for a selected subset of signal
    /// kinds.
    pub fn with_signal_listener_for<I>(
        self,
        kinds: I,
        listener: impl crate::signals::Listener + 'static,
    ) -> Self
    where
        I: IntoIterator<Item = crate::signals::Kind>,
    {
        self.signals.add_listener_for(kinds, Arc::new(listener));
        self
    }

    /// Register an extension through the engine signal bus.
    pub fn with_extension(self, extension: impl crate::extensions::Extension + 'static) -> Self {
        self.with_signal_listener(extension)
    }

    /// Register an extension for a selected subset of signal kinds.
    pub fn with_extension_for<I>(
        self,
        kinds: I,
        extension: impl crate::extensions::Extension + 'static,
    ) -> Self
    where
        I: IntoIterator<Item = crate::signals::Kind>,
    {
        self.with_signal_listener_for(kinds, extension)
    }

    /// Load plugin manifests and verify that every declared middleware plugin
    /// has a registered factory.
    ///
    /// Before calling this method, register each middleware factory with
    /// `register_middleware()`. `load_plugins()` only wires middleware plugins;
    /// core components such as scheduler/store/http/browser stay on the
    /// explicit trait + engine injection path.
    ///
    /// It verifies that every middleware declared in the plugin manifest file
    /// has a matching engine factory and returns an error otherwise.
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
        for manifest in registry.all() {
            if !self.plugins.has(&manifest.name) {
                return Err(SpiderError::plugin(format!(
                    "middleware plugin '{}' declared in plugin manifest (entry: {}) but no factory registered; \
                     call register_middleware(\"{}\", ...) before load_plugins()",
                    manifest.name, manifest.entry, manifest.name
                )));
            }
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

    async fn open_stores(&self, spider_name: &str) -> Result<(), SpiderError> {
        for store in self.stores.values() {
            store.open(spider_name).await?;
        }
        Ok(())
    }

    async fn close_stores(&self, spider_name: &str) -> Result<(), SpiderError> {
        for store in self.stores.values() {
            store.close(spider_name).await?;
        }
        Ok(())
    }

    async fn enqueue_start_requests<Sp: Spider>(
        &mut self,
        spider: &Sp,
        allowed_domains: &[String],
        compiled: Option<&Compiled>,
        step_executes: &BTreeMap<String, StepExecute>,
    ) -> Result<(), SpiderError> {
        let start_requests = if let Some(compiled) = compiled
            && !compiled.seeds.is_empty()
        {
            crate::rules::build_seed_requests(compiled)?
        } else {
            spider
                .build_start_requests()
                .into_iter()
                .map(|request| apply_compiled_fetch_to_request(request, compiled))
                .collect::<Result<Vec<_>, _>>()?
        };

        for request in &start_requests {
            let step_middleware = &step_execute_for_request(step_executes, request).chain;
            let enqueued = enqueue_request_with_middleware(
                &mut self.scheduler,
                &self.middleware,
                step_middleware,
                request.clone(),
                &[],
                Some(spider.name()),
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

        if !self.config.robots.sitemap_seeds {
            return Ok(());
        }

        self.enqueue_robots_sitemap_seeds(
            spider.name(),
            allowed_domains,
            &start_requests,
            step_executes,
        )
        .await
    }

    async fn enqueue_robots_sitemap_seeds(
        &mut self,
        spider_name: &str,
        allowed_domains: &[String],
        start_requests: &[crate::request::Request],
        step_executes: &BTreeMap<String, StepExecute>,
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
                            crate::trace::warn(
                                "sitemap.invalid",
                                vec![
                                    crate::trace::prop("spider", spider_name),
                                    crate::trace::prop("sitemap", sitemap.as_str()),
                                ],
                            );
                            continue;
                        };

                        if seen_sitemaps.insert(resolved.clone()) {
                            pending_sitemaps.push_back((resolved, request.clone()));
                        }
                    }
                }
                Err(error) => {
                    crate::trace::warn(
                        "sitemap.read_fail",
                        vec![
                            crate::trace::prop("spider", spider_name),
                            crate::trace::prop("url", request.url.as_str()),
                            crate::trace::prop("error", error),
                        ],
                    );
                }
            }
        }

        if pending_sitemaps.is_empty() {
            return Ok(());
        }

        while let Some((sitemap_url, representative_request)) = pending_sitemaps.pop_front() {
            let sitemap_request =
                build_robots_sitemap_fetch_request(&representative_request, sitemap_url.clone());

            let response = match self.http.fetch(&sitemap_request).await {
                Ok(response) => response,
                Err(error) => {
                    crate::trace::warn(
                        "sitemap.fetch_fail",
                        vec![
                            crate::trace::prop("spider", spider_name),
                            crate::trace::prop("sitemap", sitemap_url.as_str()),
                            crate::trace::prop("error", error),
                        ],
                    );
                    continue;
                }
            };

            if !(200..300).contains(&response.status) {
                crate::trace::warn(
                    "sitemap.bad_status",
                    vec![
                        crate::trace::prop("spider", spider_name),
                        crate::trace::prop("sitemap", sitemap_url.as_str()),
                        crate::trace::prop("status", response.status),
                    ],
                );
                continue;
            }

            let entries = response.sitemap().entries();

            for nested_sitemap in entries.sitemaps {
                let Some(resolved) = resolve_url(sitemap_url.as_str(), nested_sitemap.as_str())
                else {
                    crate::trace::warn(
                        "sitemap.nested_invalid",
                        vec![
                            crate::trace::prop("spider", spider_name),
                            crate::trace::prop("sitemap", nested_sitemap.as_str()),
                            crate::trace::prop("parent", sitemap_url.as_str()),
                        ],
                    );
                    continue;
                };

                if seen_sitemaps.insert(resolved.clone()) {
                    pending_sitemaps.push_back((resolved, representative_request.clone()));
                }
            }

            for page_url in entries.urls {
                let Some(resolved) = resolve_url(sitemap_url.as_str(), page_url.as_str()) else {
                    crate::trace::warn(
                        "sitemap.entry_invalid",
                        vec![
                            crate::trace::prop("spider", spider_name),
                            crate::trace::prop("url", page_url.as_str()),
                            crate::trace::prop("sitemap", sitemap_url.as_str()),
                        ],
                    );
                    continue;
                };

                let sitemap_seed_request =
                    build_robots_sitemap_seed_request(&representative_request, resolved);
                let sitemap_seed_task = build_robots_sitemap_seed_task(
                    sitemap_seed_request.clone(),
                    &self.config.robots,
                );

                let step_middleware =
                    &step_execute_for_request(step_executes, &sitemap_seed_request).chain;
                if enqueue_task_with_middleware(
                    &mut self.scheduler,
                    &self.middleware,
                    step_middleware,
                    sitemap_seed_task,
                    allowed_domains,
                    Some(spider_name),
                    Some(self.stats.as_ref()),
                )
                .await?
                {
                    self.signals
                        .emit(crate::signals::Signal::request_scheduled(
                            spider_name,
                            sitemap_seed_request,
                        ))
                        .await;
                }
            }
        }

        Ok(())
    }

    /// Run the engine continuously until a stop signal is received.
    ///
    /// Concurrent downloads are controlled by:
    /// - `config.engine.requests` for the global concurrency limit
    /// - `config.engine.requests_per_domain` for the per-domain limit
    ///
    /// The engine does not exit automatically when the queue becomes empty.
    /// It exits only when:
    /// 1. `engine.stop()` or `shutdown_handle().stop()` is called
    /// 2. Ctrl+C triggers a stop signal
    pub async fn run<Sp: Spider>(&mut self, spider: &Sp) -> Result<(), SpiderError> {
        let spider_name = spider.name();
        crate::trace::info(
            "engine.start",
            vec![crate::trace::prop("spider", spider_name)],
        );

        let allowed_domains = spider.allowed_domains();

        self.pipeline.open(spider_name).await?;
        self.open_stores(spider_name).await?;
        self.signals
            .emit(crate::signals::Signal::spider_opened(spider_name))
            .await;

        let compiled = match spider.rules() {
            Some(config) => Some(crate::rules::load(&config).await?),
            None => None,
        };

        let step_executes = self.build_step_executes(compiled.as_ref(), spider.validator())?;

        self.enqueue_start_requests(spider, &allowed_domains, compiled.as_ref(), &step_executes)
            .await?;

        let max_concurrent = self.config.engine.requests;
        let per_domain_limit = self.config.engine.requests_per_domain;
        let idle_timeout = self.config.engine.idle_timeout;
        let idle_timeout_std =
            if idle_timeout.is_zero() {
                None
            } else {
                Some(to_std_duration(idle_timeout).map_err(|error| {
                    SpiderError::engine(format!("invalid idle_timeout: {error}"))
                })?)
            };

        let global_semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let mut domain_semaphores: BTreeMap<String, Arc<tokio::sync::Semaphore>> = BTreeMap::new();

        type TaskFuture<'a> = Pin<Box<dyn std::future::Future<Output = TaskRun> + 'a>>;
        let mut inflight: FuturesUnordered<TaskFuture<'_>> = FuturesUnordered::new();
        let mut round = 0usize;

        let scheduler = &self.scheduler;
        let http = &self.http;
        let browser = &self.browser;
        let pipeline = &self.pipeline;
        let robots = self.robots.as_ref();
        let stats = self.stats.clone();
        let signals = self.signals.clone();
        let engine_middleware = &self.middleware;
        let step_executes = &step_executes;
        let allowed_domains = &allowed_domains;
        let shutdown = &self.shutdown;

        loop {
            if shutdown.load(Ordering::Relaxed) {
                while let Some(result) = inflight.next().await {
                    apply_task_run(
                        result,
                        scheduler,
                        engine_middleware,
                        step_executes,
                        allowed_domains,
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
                let available_slots = max_concurrent - inflight.len();
                let mut permits = Vec::with_capacity(available_slots);
                while permits.len() < available_slots {
                    let Ok(global_permit_guard) = global_semaphore.clone().try_acquire_owned()
                    else {
                        break;
                    };
                    permits.push(global_permit_guard);
                }
                if permits.is_empty() {
                    break;
                }

                let requested = permits.len();
                let claimed = scheduler.take_batch_ready(requested).await?;
                let claimed_count = claimed.len();
                if claimed_count == 0 {
                    drop(permits);
                    break;
                }
                permits.truncate(claimed_count);

                for (task, global_permit_guard) in claimed.into_iter().zip(permits.into_iter()) {
                    record_scheduler_event(
                        spider_name,
                        crate::signals::SchedulerEventKind::Claimed,
                        &task.lease,
                        task.task.request.url.as_str(),
                        stats.as_ref(),
                        signals.as_ref(),
                        None,
                    )
                    .await;

                    let domain = extract_domain(&task.task.request.url)
                        .unwrap_or("unknown")
                        .to_string();
                    let domain_semaphore = domain_semaphores
                        .entry(domain)
                        .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(per_domain_limit)))
                        .clone();

                    let step_execute = step_execute_for_request(step_executes, &task.task.request);
                    let task_executor = TaskExecutor {
                        scheduler,
                        http,
                        browser,
                        pipeline,
                        robots,
                        config: &self.config,
                        stats: stats.clone(),
                        signals: signals.clone(),
                        engine_middleware,
                        step_execute,
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

                if claimed_count < requested {
                    break;
                }
            }

            if inflight.is_empty() {
                if let Some(idle_timeout_std) = idle_timeout_std {
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
                    engine_middleware,
                    step_executes,
                    allowed_domains,
                    &mut round,
                    spider_name,
                    stats.as_ref(),
                    signals.as_ref(),
                )
                .await?;
            }
        }

        self.close_stores(spider_name).await?;
        self.pipeline.close(spider_name).await?;
        self.signals
            .emit(crate::signals::Signal::spider_closed(
                spider_name,
                stats.snapshot(),
            ))
            .await;

        let snapshot = stats.snapshot();
        crate::trace::info(
            "engine.stop",
            vec![
                crate::trace::prop("spider", spider_name),
                crate::trace::prop("rounds", round),
                crate::trace::prop("items", snapshot.item_count),
                crate::trace::prop("requests", snapshot.request_count),
                crate::trace::prop("responses", snapshot.response_count),
                crate::trace::prop("errors", snapshot.error_count),
                crate::trace::prop("retries", snapshot.retry_count),
                crate::trace::prop("dropped", snapshot.pipeline_drop_count),
            ],
        );

        self.scheduler.close().await?;

        Ok(())
    }

    /// Signal the engine to stop and exit gracefully after the current loop.
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    fn build_step_executes(
        &self,
        compiled: Option<&Compiled>,
        spider_validator: Option<crate::validator::StepValidator>,
    ) -> Result<BTreeMap<String, StepExecute>, SpiderError> {
        let base_defaults = self.config.request.merged_middleware();
        let shared = SharedState::default();

        let mut out = BTreeMap::new();
        let base_chain = build_with_shared(&base_defaults, &self.plugins, &shared)?;
        out.insert(
            "parse".to_string(),
            StepExecute::new(
                base_chain,
                default_step_stores(&self.stores)?,
                spider_validator.unwrap_or_default(),
            ),
        );

        if let Some(compiled) = compiled {
            for step in &compiled.steps {
                let merged = merge_middlewares(
                    merge_middlewares(
                        base_defaults.clone(),
                        step_default_middlewares(Some(compiled), &step.id),
                    ),
                    step_middlewares(Some(compiled), &step.id),
                );
                let chain = build_with_shared(&merged, &self.plugins, &shared)?;
                let stores = resolve_step_stores(
                    &self.stores,
                    step.output
                        .as_ref()
                        .map(|output| output.sinks.as_slice())
                        .unwrap_or(&[]),
                )?;
                let step_validator = match &step.output {
                    Some(output) => StepValidator::from_fields(output.validators.clone()),
                    None => StepValidator::default(),
                };
                out.insert(
                    step.id.clone(),
                    StepExecute::new(chain, stores, step_validator),
                );
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

fn build_robots_sitemap_seed_task(request: Request, robots: &crate::settings::Robots) -> Task {
    Task::new(request)
        .with_priority(robots.sitemap_seed_priority)
        .with_depth(robots.sitemap_seed_depth)
}

impl<H, B, P> Engine<crate::scheduler::Memory, H, B, P>
where
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
{
    /// Attach checkpoint persistence to the current default memory scheduler.
    ///
    /// This keeps in-memory scheduling semantics, but saves every state change
    /// through the provided `scheduler::checkpoint::Persist` backend.
    pub fn with_checkpoint<Persist>(
        self,
        persist: Persist,
    ) -> Engine<crate::scheduler::checkpoint::Memory<Persist>, H, B, P>
    where
        Persist: crate::scheduler::checkpoint::Persist,
    {
        let scheduler = crate::scheduler::checkpoint::Memory::from_parts(self.scheduler, persist);

        Engine {
            scheduler,
            http: self.http,
            browser: self.browser,
            pipeline: self.pipeline,
            stores: self.stores,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            config: self.config,
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
    ) -> Result<Engine<crate::scheduler::checkpoint::Memory<Persist>, H, B, P>, SpiderError>
    where
        Persist: crate::scheduler::checkpoint::Persist,
    {
        let scheduler = crate::scheduler::checkpoint::Memory::load(persist).await?;

        Ok(Engine {
            scheduler,
            http: self.http,
            browser: self.browser,
            pipeline: self.pipeline,
            stores: self.stores,
            robots: self.robots,
            stats: self.stats,
            signals: self.signals,
            config: self.config,
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

fn step_execute_for_request<'a>(
    step_executes: &'a BTreeMap<String, StepExecute>,
    request: &crate::request::Request,
) -> &'a StepExecute {
    let step_id = step_id_from_request(request);
    step_executes
        .get(&step_id)
        .or_else(|| step_executes.get("parse"))
        .expect("step executes must always contain the default parse step")
}

fn apply_compiled_fetch_to_request(
    request: crate::request::Request,
    compiled: Option<&Compiled>,
) -> Result<crate::request::Request, SpiderError> {
    let Some(compiled) = compiled else {
        return Ok(request);
    };

    let step = compiled.step_from_meta(&request.meta)?;
    Ok(step.fetch.apply_to_request(request))
}

fn step_default_middlewares(compiled: Option<&Compiled>, step_id: &str) -> crate::middleware::Map {
    let Some(compiled) = compiled else {
        return crate::middleware::Map::new();
    };

    compiled
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .map(|step| step.default_middlewares.clone())
        .unwrap_or_default()
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

fn merge_middlewares(
    mut defaults: crate::middleware::Map,
    explicit: crate::middleware::Map,
) -> crate::middleware::Map {
    for (key, config) in explicit {
        defaults.insert(key, config);
    }

    defaults
}

fn default_step_stores(
    stores: &BTreeMap<String, crate::store::SharedStore>,
) -> Result<Vec<crate::store::SharedStore>, SpiderError> {
    stores
        .get(DEFAULT_STORE_KEY)
        .cloned()
        .map(|store| vec![store])
        .ok_or_else(|| SpiderError::engine("default store not found".to_string()))
}

fn resolve_step_stores(
    stores: &BTreeMap<String, crate::store::SharedStore>,
    sinks: &[String],
) -> Result<Vec<crate::store::SharedStore>, SpiderError> {
    if sinks.is_empty() {
        return default_step_stores(stores);
    }

    let mut seen = BTreeSet::new();
    let mut resolved = Vec::new();
    for sink in sinks {
        if !seen.insert(sink.clone()) {
            continue;
        }

        let store = stores
            .get(sink)
            .ok_or_else(|| SpiderError::engine(format!("store not found: {sink}")))?;
        resolved.push(store.clone());
    }

    Ok(resolved)
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
        let Some(task) = self.scheduler.take_ready().await? else {
            return Ok(None);
        };
        record_scheduler_event(
            "manual",
            crate::signals::SchedulerEventKind::Claimed,
            &task.lease,
            task.task.request.url.as_str(),
            self.stats.as_ref(),
            self.signals.as_ref(),
            None,
        )
        .await;
        let task_id = task.lease.task_id().clone();
        let attempt = task
            .task
            .request
            .meta
            .get("_retry_times")
            .and_then(crate::value::Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0) as u32
            + 1;
        let mut context = context::Download::new(task.task.request)
            .with_task_id(task_id)
            .with_spider_name("manual")
            .with_attempt(attempt)
            .with_stats(self.stats.clone());
        let lease = task.lease;

        let default_chain = Chain::default();
        let step_middleware = &default_chain;

        match run_middleware_before_download(&self.middleware, step_middleware, &mut context).await
        {
            Ok(flow::Download::Continue) => {}
            Ok(_) => {
                self.scheduler.complete(&lease).await?;
                record_scheduler_event(
                    "manual",
                    crate::signals::SchedulerEventKind::Completed,
                    &lease,
                    context.request.url.as_str(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    None,
                )
                .await;
                return Ok(None);
            }
            Err(e) => {
                self.scheduler.requeue(&lease).await?;
                record_scheduler_event(
                    "manual",
                    crate::signals::SchedulerEventKind::Requeued,
                    &lease,
                    context.request.url.as_str(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    None,
                )
                .await;
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
                let _ = run_middleware_download_error(
                    &self.middleware,
                    step_middleware,
                    &mut context,
                    &e,
                )
                .await;
                self.stats.record_error();
                self.scheduler.requeue(&lease).await?;
                record_scheduler_event(
                    "manual",
                    crate::signals::SchedulerEventKind::Requeued,
                    &lease,
                    context.request.url.as_str(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    None,
                )
                .await;
                return Err(e);
            }
        };

        let mut response = response;

        match run_middleware_after_download(
            &self.middleware,
            step_middleware,
            &mut context,
            &mut response,
        )
        .await
        {
            Ok(flow::Download::Continue) => {}
            Ok(_) => {
                self.scheduler.complete(&lease).await?;
                record_scheduler_event(
                    "manual",
                    crate::signals::SchedulerEventKind::Completed,
                    &lease,
                    context.request.url.as_str(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    None,
                )
                .await;
                return Ok(None);
            }
            Err(e) => {
                self.scheduler.requeue(&lease).await?;
                record_scheduler_event(
                    "manual",
                    crate::signals::SchedulerEventKind::Requeued,
                    &lease,
                    context.request.url.as_str(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    None,
                )
                .await;
                return Err(e);
            }
        }

        let mut parse = context::Parse::new(context.request.clone(), response.clone())
            .with_task_id(context.task_id.clone())
            .with_spider_name("manual");

        match run_middleware_before_parse(&self.middleware, step_middleware, &mut parse).await {
            Ok(flow::Parse::Continue) => {}
            Ok(_) => {
                self.scheduler.complete(&lease).await?;
                record_scheduler_event(
                    "manual",
                    crate::signals::SchedulerEventKind::Completed,
                    &lease,
                    parse.request.url.as_str(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    None,
                )
                .await;
                return Ok(None);
            }
            Err(e) => {
                self.scheduler.requeue(&lease).await?;
                record_scheduler_event(
                    "manual",
                    crate::signals::SchedulerEventKind::Requeued,
                    &lease,
                    parse.request.url.as_str(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    None,
                )
                .await;
                return Err(e);
            }
        }

        self.scheduler.complete(&lease).await?;
        record_scheduler_event(
            "manual",
            crate::signals::SchedulerEventKind::Completed,
            &lease,
            parse.request.url.as_str(),
            self.stats.as_ref(),
            self.signals.as_ref(),
            None,
        )
        .await;
        Ok(Some(response))
    }

    async fn execute_spider_once<Sp: Spider>(
        &mut self,
        spider: &Sp,
        compiled: Option<&Compiled>,
        step_executes: &mut BTreeMap<String, StepExecute>,
    ) -> Result<Option<crate::engine::task::TaskOutput>, SpiderError> {
        if !self.prepared {
            *step_executes = self.build_step_executes(compiled, spider.validator())?;
            self.pipeline.open(spider.name()).await?;
            self.open_stores(spider.name()).await?;
            self.prepared = true;
        }

        let Some(task) = self.scheduler.take_ready().await? else {
            return Ok(None);
        };
        record_scheduler_event(
            spider.name(),
            crate::signals::SchedulerEventKind::Claimed,
            &task.lease,
            task.task.request.url.as_str(),
            self.stats.as_ref(),
            self.signals.as_ref(),
            None,
        )
        .await;
        let task_id = task.lease.task_id().clone();
        let lease = task.lease.clone();
        let task_url = task.task.request.url.clone();

        let step_execute = step_execute_for_request(step_executes, &task.task.request);

        let task_executor = TaskExecutor {
            scheduler: &self.scheduler,
            http: &self.http,
            browser: &self.browser,
            pipeline: &self.pipeline,
            robots: self.robots.as_ref(),
            config: &self.config,
            stats: self.stats.clone(),
            signals: self.signals.clone(),
            engine_middleware: &self.middleware,
            step_execute,
            spider,
            compiled,
            allowed_domains: &[],
            spider_name: spider.name(),
        };

        let outcome = task_executor.run(task_id, task.task.request).await;

        match outcome {
            TaskOutcome::Success(output) => {
                let store_committed = !output.items.is_empty();
                let mut scheduled_follows = Vec::new();
                let mut follow_tasks = Vec::new();
                for follow in &output.follows {
                    let step_middleware = &step_execute_for_request(step_executes, follow).chain;
                    let task = prepare_task_for_enqueue_with_middleware(
                        &self.middleware,
                        step_middleware,
                        Task::new(follow.clone()),
                        &[],
                        Some(spider.name()),
                        Some(self.stats.as_ref()),
                    )
                    .await?;

                    if let Some(task) = task {
                        scheduled_follows.push((follow.clone(), task.id.clone()));
                        follow_tasks.push(task);
                    }
                }
                let committed = resolve_scheduler_transition(
                    self.scheduler.complete_and_enqueue(&lease, follow_tasks),
                    &lease,
                    crate::signals::SchedulerEventKind::Completed,
                    task_url.as_str(),
                    spider.name(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    "complete_and_enqueue",
                    store_committed,
                    scheduled_follows.len(),
                )
                .await?;
                if committed {
                    for (follow, task_id) in &scheduled_follows {
                        let step_middleware =
                            &step_execute_for_request(step_executes, follow).chain;
                        crate::engine::task::run_middleware_after_enqueue(
                            &self.middleware,
                            step_middleware,
                            follow.clone(),
                            task_id.clone(),
                            Some(spider.name()),
                        )
                        .await?;
                        self.signals
                            .emit(crate::signals::Signal::request_scheduled(
                                spider.name(),
                                follow.clone(),
                            ))
                            .await;
                    }
                }
                Ok(Some(output))
            }
            TaskOutcome::Delay(delayed_task) => {
                let delayed_task = *delayed_task;
                let delayed_task_id = delayed_task.id.clone();
                let request = delayed_task.request.clone();
                let step_middleware = &step_execute_for_request(step_executes, &request).chain;
                let delayed_tasks = prepare_task_for_enqueue_with_middleware(
                    &self.middleware,
                    step_middleware,
                    delayed_task,
                    &[],
                    Some(spider.name()),
                    Some(self.stats.as_ref()),
                )
                .await?
                .into_iter()
                .collect::<Vec<_>>();
                let queued_delayed_tasks = delayed_tasks.len();
                let committed = resolve_scheduler_transition(
                    self.scheduler.complete_and_enqueue(&lease, delayed_tasks),
                    &lease,
                    crate::signals::SchedulerEventKind::Completed,
                    task_url.as_str(),
                    spider.name(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    "complete_and_enqueue",
                    false,
                    queued_delayed_tasks,
                )
                .await?;
                if committed {
                    crate::engine::task::run_middleware_after_enqueue(
                        &self.middleware,
                        step_middleware,
                        request.clone(),
                        delayed_task_id,
                        Some(spider.name()),
                    )
                    .await?;
                    self.signals
                        .emit(crate::signals::Signal::request_scheduled(
                            spider.name(),
                            request,
                        ))
                        .await;
                }
                Ok(None)
            }
            TaskOutcome::Retry(retry_task) => {
                self.stats.record_retry();
                let retry_task = *retry_task;
                let retry_task_id = retry_task.id.clone();
                let request = retry_task.request.clone();
                let step_middleware = &step_execute_for_request(step_executes, &request).chain;
                let retry_tasks = prepare_task_for_enqueue_with_middleware(
                    &self.middleware,
                    step_middleware,
                    retry_task,
                    &[],
                    Some(spider.name()),
                    Some(self.stats.as_ref()),
                )
                .await?
                .into_iter()
                .collect::<Vec<_>>();
                let queued_retry_tasks = retry_tasks.len();
                let committed = resolve_scheduler_transition(
                    self.scheduler.complete_and_enqueue(&lease, retry_tasks),
                    &lease,
                    crate::signals::SchedulerEventKind::Completed,
                    task_url.as_str(),
                    spider.name(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    "complete_and_enqueue",
                    false,
                    queued_retry_tasks,
                )
                .await?;
                if committed {
                    crate::engine::task::run_middleware_after_enqueue(
                        &self.middleware,
                        step_middleware,
                        request.clone(),
                        retry_task_id,
                        Some(spider.name()),
                    )
                    .await?;
                    self.signals
                        .emit(crate::signals::Signal::request_scheduled(
                            spider.name(),
                            request,
                        ))
                        .await;
                }
                Ok(None)
            }
            TaskOutcome::Drop => {
                resolve_scheduler_transition(
                    self.scheduler.complete(&lease),
                    &lease,
                    crate::signals::SchedulerEventKind::Completed,
                    task_url.as_str(),
                    spider.name(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    "complete",
                    false,
                    0,
                )
                .await?;
                Ok(None)
            }
            TaskOutcome::Error(e) => {
                self.stats.record_error();
                resolve_scheduler_transition(
                    self.scheduler.requeue(&lease),
                    &lease,
                    crate::signals::SchedulerEventKind::Requeued,
                    task_url.as_str(),
                    spider.name(),
                    self.stats.as_ref(),
                    self.signals.as_ref(),
                    "requeue",
                    false,
                    0,
                )
                .await?;
                Err(e)
            }
            TaskOutcome::LeaseLost(error) => Err(error),
        }
    }
}

#[cfg(test)]
#[allow(refining_impl_trait)]
mod tests {
    use super::*;
    use crate::engine::{context, flow};
    use crate::future::BoxFuture;
    use crate::middleware::traits::Middleware;
    use crate::middleware::{
        AUTO_THROTTLE, HTTP_CACHE, INTERVAL, RATE_LIMIT, RETRY_BY_ERROR, RETRY_BY_STATUS,
    };
    use crate::pipeline::Pipeline;
    use crate::plugins::{PluginManifest, PluginRegistry};
    use crate::request::{Headers, Request};
    use crate::response::Response;
    use crate::rules::Config as RulesConfig;
    use crate::scheduler::checkpoint::{Checkpoint, Persist};
    use crate::scheduler::memory::Memory;
    use crate::scheduler::{Scheduler, Task};
    use crate::signals::{
        Kind as SignalKind, Listener as SignalListener, SchedulerEventKind, Signal,
    };
    use crate::spider::{Failure, Spider};
    use crate::stats::Snapshot as StatsSnapshot;
    use crate::stats::{Event as StatsEvent, Reporter as StatsReporter};
    use crate::store::Memory as MemoryStore;
    use crate::test_support::redis::spawn_redis_server;
    use crate::validator;
    use crate::value::Value;
    use jiff::SignedDuration;
    use serde_json::json;
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

        let mut step_executes = BTreeMap::new();
        let output = block_on(engine.execute_spider_once(
            &SimpleSpider("with_downloaders"),
            None,
            &mut step_executes,
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
            .with_worker(
                crate::scheduler::Worker::new("engine-worker")
                    .with_lease_timeout(SignedDuration::from_millis(20))
                    .with_heartbeat_interval(SignedDuration::from_millis(10)),
            );
        let mut engine = Engine::from_parts(scheduler, AsyncDelayedHttp { delay: 60 }, StubBrowser)
            .with_config(Config::default().with_idle_timeout(SignedDuration::from_millis(5)))
            .with_store(MemoryStore::default());
        let shutdown = engine.shutdown_handle();
        let observer = crate::scheduler::Redis::new(format!("redis://{url}"), namespace)
            .with_worker(
                crate::scheduler::Worker::new("observer")
                    .with_lease_timeout(SignedDuration::from_millis(20)),
            );
        let observer_task = async move {
            let mut checkpoint = observer.checkpoint().await.unwrap();
            for _ in 0..20 {
                let has_expected_inflight = checkpoint.ready.is_empty()
                    && checkpoint.inflight.len() == 1
                    && checkpoint.inflight[0].request.url == "https://example.com/start";
                if has_expected_inflight {
                    break;
                }
                tokio::time::sleep(to_std_duration(SignedDuration::from_millis(5)).unwrap()).await;
                checkpoint = observer.checkpoint().await.unwrap();
            }

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
        run_result.unwrap();
        assert_eq!(engine.stats().response_count, 1);

        let final_checkpoint = observer.checkpoint().await.unwrap();
        assert!(!final_checkpoint.has_pending());

        observer.close().await.unwrap();
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
            crate::middleware::Config {
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
    fn engine_load_plugins_requires_registered_factory() {
        let mut registry = PluginRegistry::new();
        registry
            .register(PluginManifest {
                name: "stats".to_string(),
                entry: "plugins::StatsMiddleware".to_string(),
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
                .contains("middleware plugin 'stats' declared in plugin manifest")
        );
        assert!(
            error
                .to_string()
                .contains("call register_middleware(\"stats\", ...) before load_plugins()")
        );
    }

    #[test]
    fn engine_loads_default_request_middlewares_and_applies_explicit_overrides() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, HtmlHttp, StubBrowser)
            .with_config(default_request_middleware_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();
        block_on(engine.execute_spider_once(
            &SimpleSpider("request_middleware"),
            None,
            &mut step_executes,
        ))
        .unwrap()
        .unwrap();

        let keys = step_executes
            .get("parse")
            .unwrap()
            .chain
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>();

        assert!(keys.contains(&RETRY_BY_ERROR));
        assert!(keys.contains(&INTERVAL));
        assert!(keys.contains(&RATE_LIMIT));
        assert!(!keys.contains(&"dedup"));
    }

    #[test]
    fn engine_loads_auto_throttle_from_settings() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, HtmlHttp, StubBrowser)
            .with_config(auto_throttle_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();
        block_on(engine.execute_spider_once(
            &SimpleSpider("auto_throttle_runtime"),
            None,
            &mut step_executes,
        ))
        .unwrap()
        .unwrap();

        let keys = step_executes
            .get("parse")
            .unwrap()
            .chain
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>();

        assert!(keys.contains(&AUTO_THROTTLE));
        let interval_entry = step_executes
            .get("parse")
            .unwrap()
            .chain
            .entries
            .iter()
            .find(|entry| entry.key == INTERVAL)
            .unwrap();
        assert!(interval_entry.config.options.is_empty());
    }

    #[test]
    fn engine_loads_http_cache_from_settings() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, HtmlHttp, StubBrowser)
            .with_config(http_cache_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();
        block_on(engine.execute_spider_once(
            &SimpleSpider("http_cache_runtime"),
            None,
            &mut step_executes,
        ))
        .unwrap()
        .unwrap();

        let keys = step_executes
            .get("parse")
            .unwrap()
            .chain
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>();

        assert!(keys.contains(&HTTP_CACHE));
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

        let mut step_executes = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&SimpleSpider("dedup"), None, &mut step_executes))
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&SimpleSpider("dedup"), None, &mut step_executes))
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
                scheduler_claim_count: 1,
                scheduler_complete_count: 1,
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
            .with_dedup(crate::middleware::dedup::Noop)
            .with_store(MemoryStore::default());

        assert!(block_on(engine.enqueue(Request::new("https://example.com/dedup"))).unwrap());
        assert!(block_on(engine.enqueue(Request::new("https://example.com/dedup"))).unwrap());

        let mut step_executes = BTreeMap::new();
        let first =
            block_on(engine.execute_spider_once(&SimpleSpider("dedup"), None, &mut step_executes))
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&SimpleSpider("dedup"), None, &mut step_executes))
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
            .with_dedup(crate::middleware::dedup::Bloom::default())
            .with_store(MemoryStore::default());

        assert!(block_on(engine.enqueue(Request::new("https://example.com/dedup-bloom"))).unwrap());
        assert!(
            !block_on(engine.enqueue(Request::new("https://example.com/dedup-bloom"))).unwrap()
        );

        let mut step_executes = BTreeMap::new();
        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("dedup_bloom"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("dedup_bloom"),
            None,
            &mut step_executes,
        ))
        .unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(*fetches.lock().unwrap(), 1);
    }

    #[test]
    fn engine_enqueue_request_can_skip_dedup_for_one_request() {
        let mut engine = Engine::from_parts(Memory::default(), StubHttp, StubBrowser)
            .with_store(MemoryStore::default());

        assert!(block_on(engine.enqueue(Request::new("https://example.com/dedup-skip"))).unwrap());
        assert!(
            block_on(engine.enqueue(
                Request::new("https://example.com/dedup-skip").skip([crate::middleware::DEDUP]),
            ))
            .unwrap()
        );
    }

    #[test]
    fn admission_skip_dedup_still_respects_allowed_domains() {
        let task = block_on(prepare_task_for_enqueue_with_middleware(
            &default_engine_middleware(),
            &Chain::default(),
            Task::new(
                Request::new("https://outside.example.net/page").skip([crate::middleware::DEDUP]),
            ),
            &["example.com".to_string()],
            Some("manual"),
            None,
        ))
        .unwrap();

        assert!(task.is_none());
    }

    #[test]
    fn admission_skip_domain_filter_still_respects_dedup() {
        let engine_middleware = default_engine_middleware();
        let request = Request::new("https://outside.example.net/page").skip_domain_filter();

        let first = block_on(prepare_task_for_enqueue_with_middleware(
            &engine_middleware,
            &Chain::default(),
            Task::new(request.clone()),
            &["example.com".to_string()],
            Some("manual"),
            None,
        ))
        .unwrap();
        let second = block_on(prepare_task_for_enqueue_with_middleware(
            &engine_middleware,
            &Chain::default(),
            Task::new(request),
            &["example.com".to_string()],
            Some("manual"),
            None,
        ))
        .unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
    }

    #[test]
    fn engine_enqueue_request_can_override_dedup_keys() {
        let mut engine = Engine::from_parts(Memory::default(), StubHttp, StubBrowser)
            .with_store(MemoryStore::default());

        let dedup = BTreeMap::from([(
            "key".to_string(),
            Value::Array(vec![
                Value::String("url".to_string()),
                Value::String("meta.page".to_string()),
            ]),
        )]);

        assert!(
            block_on(
                engine.enqueue(
                    Request::new("https://example.com/dedup-keys")
                        .with_meta("page", Value::Number(1.0))
                        .with_dedup(dedup.clone()),
                )
            )
            .unwrap()
        );
        assert!(
            block_on(
                engine.enqueue(
                    Request::new("https://example.com/dedup-keys")
                        .with_meta("page", Value::Number(2.0))
                        .with_dedup(dedup),
                )
            )
            .unwrap()
        );
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
            .with_config(Config::default().with_request_middleware(
                RETRY_BY_ERROR,
                crate::middleware::Config {
                    enabled: false,
                    stage: Stage::Download,
                    order: 210,
                    options: BTreeMap::new(),
                },
            ))
            .with_store(store.clone());

        let mut step_executes = BTreeMap::new();
        let output = block_on(engine.execute_spider_once(&ErrbackSpider, None, &mut step_executes))
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

        let mut step_executes = BTreeMap::new();
        let output =
            block_on(engine.execute_spider_once(&ParseErrorSpider, None, &mut step_executes))
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
            .with_config(retry_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&SimpleSpider("retry"), None, &mut step_executes))
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&SimpleSpider("retry"), None, &mut step_executes))
                .unwrap();

        assert!(first.is_none());
        assert!(second.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_request_retry_override_can_enable_retry_without_global_defaults() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(
            Request::new("https://example.com/request-retry").with_retry_by_status(
                BTreeMap::from([
                    ("count".to_string(), Value::Number(1.0)),
                    (
                        "http_status".to_string(),
                        Value::Array(vec![Value::Number(500.0)]),
                    ),
                ]),
                200,
            ),
        )))
        .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![500, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_config(
                Config::default()
                    .with_retry_times(0)
                    .with_retry_http_codes(Vec::new()),
            )
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("request_retry"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("request_retry"),
            None,
            &mut step_executes,
        ))
        .unwrap();

        assert!(first.is_none());
        assert!(second.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_request_interval_override_can_enable_download_before_middleware_without_global_defaults()
     {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(
            Request::new("https://example.com/request-interval-1").with_interval(
                BTreeMap::from([("interval".to_string(), Value::Number(20.0))]),
                120,
            ),
        )))
        .unwrap();
        block_on(scheduler.enqueue(Task::new(
            Request::new("https://example.com/request-interval-2").with_interval(
                BTreeMap::from([("interval".to_string(), Value::Number(20.0))]),
                120,
            ),
        )))
        .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_config(Config::default().with_download_delay(SignedDuration::from_millis(0)))
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("request_interval"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("request_interval"),
            None,
            &mut step_executes,
        ))
        .unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(*fetches.lock().unwrap(), 1);
    }

    #[test]
    fn engine_request_can_skip_retry_middleware() {
        let scheduler = Memory::default();
        block_on(
            scheduler.enqueue(Task::new(
                Request::new("https://example.com/request-skip-retry")
                    .skip([RETRY_BY_STATUS, RETRY_BY_ERROR]),
            )),
        )
        .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![500],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_config(retry_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("request_skip_retry"),
            None,
            &mut step_executes,
        ))
        .unwrap();

        assert!(first.is_some());
        assert_eq!(*fetches.lock().unwrap(), 1);
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
            .with_config(retry_backoff_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("retry_backoff"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("retry_backoff"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        std::thread::sleep(to_std_duration(SignedDuration::from_millis(30)).unwrap());
        let third = block_on(engine.execute_spider_once(
            &SimpleSpider("retry_backoff"),
            None,
            &mut step_executes,
        ))
        .unwrap();

        assert!(first.is_none());
        assert!(second.is_none());
        assert!(third.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_respects_interval_delay() {
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
            .with_config(interval_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("interval"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("interval"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        std::thread::sleep(to_std_duration(SignedDuration::from_millis(15)).unwrap());
        let third = block_on(engine.execute_spider_once(
            &SimpleSpider("interval"),
            None,
            &mut step_executes,
        ))
        .unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert!(third.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_request_can_skip_download_before_middleware() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new(
            "https://example.com/interval-skip/1",
        ))))
        .unwrap();
        block_on(scheduler.enqueue(Task::new(
            Request::new("https://example.com/interval-skip/2").skip([INTERVAL]),
        )))
        .unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_config(interval_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("interval_skip"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("interval_skip"),
            None,
            &mut step_executes,
        ))
        .unwrap();

        assert!(first.is_some());
        assert!(second.is_some());
        assert_eq!(*fetches.lock().unwrap(), 2);
    }

    #[test]
    fn engine_callback_output_requests_reenter_admission_dedup() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/start")))).unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200],
        };
        let store = MemoryStore::default();
        let mut engine =
            Engine::from_parts(scheduler, downloader, StubBrowser).with_store(store.clone());
        let mut step_executes = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&CallbackDedupSpider, None, &mut step_executes))
                .unwrap()
                .unwrap();

        let checkpoint = engine.scheduler.checkpoint();
        let ready_urls = checkpoint
            .ready
            .iter()
            .map(|task| task.request.url.clone())
            .collect::<Vec<_>>();

        assert_eq!(first.items.len(), 1);
        assert_eq!(first.follows.len(), 2);
        assert_eq!(ready_urls, vec!["https://example.com/detail".to_string()]);
        assert_eq!(
            store.items(),
            vec![crate::item::Item::new().with_field("title", Value::String("root".to_string()))]
        );

        let second =
            block_on(engine.execute_spider_once(&CallbackDedupSpider, None, &mut step_executes))
                .unwrap()
                .unwrap();

        assert_eq!(*fetches.lock().unwrap(), 2);
        assert_eq!(second.items.len(), 1);
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 2,
                response_count: 2,
                item_count: 2,
                dedup_reject_count: 1,
                scheduler_claim_count: 2,
                scheduler_complete_count: 2,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_callback_output_requests_reenter_download_before_middleware() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/start")))).unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 200, 200],
        };
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_config(Config::default().with_download_delay(SignedDuration::from_millis(0)))
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&CallbackIntervalSpider, None, &mut step_executes))
                .unwrap()
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&CallbackIntervalSpider, None, &mut step_executes))
                .unwrap()
                .unwrap();
        let third =
            block_on(engine.execute_spider_once(&CallbackIntervalSpider, None, &mut step_executes))
                .unwrap();

        let delayed_checkpoint = engine.scheduler.checkpoint();

        assert_eq!(first.follows.len(), 2);
        assert_eq!(second.items.len(), 1);
        assert!(third.is_none());
        assert_eq!(*fetches.lock().unwrap(), 2);
        assert_eq!(delayed_checkpoint.delayed.len(), 1);

        std::thread::sleep(to_std_duration(SignedDuration::from_millis(25)).unwrap());

        let fourth =
            block_on(engine.execute_spider_once(&CallbackIntervalSpider, None, &mut step_executes))
                .unwrap()
                .unwrap();

        assert_eq!(fourth.items.len(), 1);
        assert_eq!(*fetches.lock().unwrap(), 3);
    }

    #[test]
    fn engine_callback_output_requests_reenter_retry_middleware() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/start")))).unwrap();

        let fetches = Arc::new(Mutex::new(0usize));
        let downloader = CountHttp {
            fetches: fetches.clone(),
            statuses: vec![200, 500, 200],
        };
        let store = MemoryStore::default();
        let mut engine = Engine::from_parts(scheduler, downloader, StubBrowser)
            .with_config(
                Config::default()
                    .with_retry_times(0)
                    .with_retry_http_codes(Vec::new()),
            )
            .with_store(store.clone());
        let mut step_executes = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&CallbackRetrySpider, None, &mut step_executes))
                .unwrap()
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&CallbackRetrySpider, None, &mut step_executes))
                .unwrap();

        let checkpoint = engine.scheduler.checkpoint();
        let retry_task = checkpoint
            .ready
            .first()
            .expect("retry task should be ready");

        assert_eq!(first.follows.len(), 1);
        assert!(second.is_none());
        assert_eq!(
            retry_task.request.meta.get("_retry_times"),
            Some(&Value::Number(1.0))
        );
        assert_eq!(
            retry_task.request.meta.get("_retry_reason"),
            Some(&Value::String(RETRY_BY_STATUS.to_string()))
        );
        assert!(
            retry_task
                .request
                .middleware_skips(crate::middleware::DEDUP)
        );

        let third =
            block_on(engine.execute_spider_once(&CallbackRetrySpider, None, &mut step_executes))
                .unwrap()
                .unwrap();

        assert_eq!(third.items.len(), 1);
        assert_eq!(*fetches.lock().unwrap(), 3);
        assert_eq!(
            store.items(),
            vec![crate::item::Item::new().with_field("title", Value::String("detail".to_string()))]
        );
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 3,
                response_count: 3,
                item_count: 1,
                retry_count: 1,
                scheduler_claim_count: 3,
                scheduler_complete_count: 3,
                ..StatsSnapshot::default()
            }
        );
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
            .with_config(rate_limit_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("rate_limit"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("rate_limit"),
            None,
            &mut step_executes,
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
            .with_config(auto_throttle_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("auto_throttle"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("auto_throttle"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        let mut third = None;

        for _ in 0..10 {
            std::thread::sleep(to_std_duration(SignedDuration::from_millis(10)).unwrap());
            third = block_on(engine.execute_spider_once(
                &SimpleSpider("auto_throttle"),
                None,
                &mut step_executes,
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
            .with_config(http_cache_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&ResponseInspectSpider, None, &mut step_executes))
                .unwrap()
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&ResponseInspectSpider, None, &mut step_executes))
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
            Some(&Value::Array(vec![Value::String(HTTP_CACHE.to_string())]))
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
                scheduler_claim_count: 2,
                scheduler_complete_count: 2,
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
        let mut step_executes = BTreeMap::new();

        let output = block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes))
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
                scheduler_claim_count: 1,
                scheduler_complete_count: 1,
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
        let mut step_executes = BTreeMap::new();

        let output = block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes))
            .unwrap()
            .unwrap();

        assert_eq!(output.items, store.items());
    }

    #[test]
    fn engine_dispatches_items_to_selected_stores() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let default_store = MemoryStore::default();
        let article_db = MemoryStore::default();
        let article_file = MemoryStore::default();
        let compiled = load_test_rules(&RoutedItemSpider);
        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_store(default_store.clone())
            .with_stores([
                StoreEntry::new("article_db", article_db.clone()),
                StoreEntry::new("article_file", article_file.clone()),
            ]);
        let mut step_executes = BTreeMap::new();

        let output = block_on(engine.execute_spider_once(
            &RoutedItemSpider,
            Some(&compiled),
            &mut step_executes,
        ))
        .unwrap()
        .unwrap();

        let expected =
            vec![crate::item::Item::new().with_field("title", Value::String("post".to_string()))];

        assert_eq!(output.items, expected);
        assert!(default_store.items().is_empty());
        assert_eq!(article_db.items(), expected);
        assert_eq!(article_file.items(), expected);
    }

    #[test]
    fn engine_errors_when_store_key_is_missing() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let default_store = MemoryStore::default();
        let compiled = load_test_rules(&MissingStoreSpider);
        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_store(default_store.clone())
            .with_pipeline(PassPipeline);
        let mut step_executes = BTreeMap::new();

        let error = block_on(engine.execute_spider_once(
            &MissingStoreSpider,
            Some(&compiled),
            &mut step_executes,
        ))
        .unwrap_err();

        assert!(error.to_string().contains("store not found: missing"));
        assert!(default_store.items().is_empty());
    }

    #[test]
    fn engine_pipeline_can_drop_items_explicitly() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_pipeline(DropPipeline)
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let output = block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes))
            .unwrap()
            .unwrap();

        assert!(output.items.is_empty());
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                pipeline_drop_count: 1,
                scheduler_claim_count: 1,
                scheduler_complete_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_validator_drops_invalid_items_before_store() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let store = MemoryStore::default();
        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_pipeline(PassPipeline)
            .with_store(store.clone());
        let mut step_executes = BTreeMap::new();

        let output = block_on(engine.execute_spider_once(
            &InvalidValidatedItemSpider,
            None,
            &mut step_executes,
        ))
        .unwrap()
        .unwrap();

        assert!(output.items.is_empty());
        assert!(store.items().is_empty());
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                scheduler_claim_count: 1,
                scheduler_complete_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_validator_allows_valid_items_into_store() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let store = MemoryStore::default();
        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_pipeline(PassPipeline)
            .with_store(store.clone());
        let mut step_executes = BTreeMap::new();

        let output =
            block_on(engine.execute_spider_once(&ValidatedItemSpider, None, &mut step_executes))
                .unwrap()
                .unwrap();

        assert_eq!(output.items.len(), 1);
        assert_eq!(store.items(), output.items);
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                item_count: 1,
                scheduler_claim_count: 1,
                scheduler_complete_count: 1,
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
        let mut step_executes = BTreeMap::new();

        let error = block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes))
            .unwrap_err();

        assert!(error.to_string().contains("pipeline failed"));
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                error_count: 1,
                scheduler_claim_count: 1,
                scheduler_requeue_count: 1,
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
        let mut step_executes = BTreeMap::new();

        let error = block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes))
            .unwrap_err();

        assert!(error.to_string().contains("store failed"));
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                error_count: 1,
                store_error_count: 1,
                scheduler_claim_count: 1,
                scheduler_requeue_count: 1,
                ..StatsSnapshot::default()
            }
        );
    }

    #[test]
    fn engine_keeps_boundary_explicit_when_scheduler_resolve_fails_after_store_commit() {
        let scheduler = FailCompleteAndEnqueueScheduler::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let store = BatchOnlyStore::default();
        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_store(store.clone())
            .with_pipeline(PassPipeline);
        let mut step_executes = BTreeMap::new();

        let error = block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes))
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("complete_and_enqueue failed after store commit")
        );
        assert_eq!(store.items().len(), 1);

        let checkpoint = block_on(Scheduler::checkpoint(&engine.scheduler)).unwrap();
        assert!(checkpoint.ready.is_empty());
        assert!(checkpoint.delayed.is_empty());
        assert_eq!(checkpoint.inflight.len(), 1);
        assert_eq!(
            checkpoint.inflight[0].request.url,
            "https://example.com/item"
        );
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                item_count: 1,
                scheduler_claim_count: 1,
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
        let mut step_executes = BTreeMap::new();

        block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes)).unwrap();

        let events = recorded.lock().unwrap().clone();
        assert_eq!(
            events,
            vec![
                (
                    StatsEvent::SchedulerClaim,
                    StatsSnapshot {
                        scheduler_claim_count: 1,
                        ..StatsSnapshot::default()
                    },
                ),
                (
                    StatsEvent::Request,
                    StatsSnapshot {
                        request_count: 1,
                        scheduler_claim_count: 1,
                        ..StatsSnapshot::default()
                    },
                ),
                (
                    StatsEvent::Response,
                    StatsSnapshot {
                        request_count: 1,
                        response_count: 1,
                        scheduler_claim_count: 1,
                        ..StatsSnapshot::default()
                    },
                ),
                (
                    StatsEvent::Item,
                    StatsSnapshot {
                        request_count: 1,
                        response_count: 1,
                        item_count: 1,
                        scheduler_claim_count: 1,
                        ..StatsSnapshot::default()
                    },
                ),
                (
                    StatsEvent::SchedulerComplete,
                    StatsSnapshot {
                        request_count: 1,
                        response_count: 1,
                        item_count: 1,
                        scheduler_claim_count: 1,
                        scheduler_complete_count: 1,
                        ..StatsSnapshot::default()
                    },
                ),
            ]
        );
    }

    #[test]
    fn engine_with_telemetry_collects_stats_and_scheduler_runtime() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/item")))).unwrap();

        let telemetry = crate::telemetry::Collector::default();
        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_telemetry(telemetry.clone());
        let mut step_executes = BTreeMap::new();

        block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes)).unwrap();

        let snapshot = telemetry.snapshot();
        assert_eq!(
            snapshot.stats,
            StatsSnapshot {
                request_count: 1,
                response_count: 1,
                item_count: 1,
                scheduler_claim_count: 1,
                scheduler_complete_count: 1,
                ..StatsSnapshot::default()
            }
        );
        assert_eq!(snapshot.scheduler.totals.claimed_total, 1);
        assert_eq!(snapshot.scheduler.totals.completed_total, 1);
        assert!(
            snapshot
                .recent_events
                .iter()
                .any(|event| matches!(event, crate::telemetry::Event::Scheduler(_)))
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
            .with_signal_listener_for(
                [
                    SignalKind::RequestScheduled,
                    SignalKind::ResponseReceived,
                    SignalKind::ItemScraped,
                ],
                listener,
            );

        block_on(engine.enqueue(Request::new("https://example.com/item"))).unwrap();

        let mut step_executes = BTreeMap::new();
        block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes)).unwrap();

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
            .with_signal_listener_for(
                [
                    SignalKind::RequestScheduled,
                    SignalKind::ResponseReceived,
                    SignalKind::SpiderError,
                ],
                listener,
            );

        block_on(engine.enqueue(Request::new("https://example.com/error"))).unwrap();

        let mut step_executes = BTreeMap::new();
        let error = block_on(engine.execute_spider_once(&FailingSpider, None, &mut step_executes))
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
    fn engine_with_filtered_signal_listener_only_receives_selected_events() {
        let listener = RecordingSignalListener::default();
        let recorded_events = listener.events.clone();
        let recorded_titles = listener.item_titles.clone();
        let recorded_urls = listener.request_urls.clone();

        let mut engine = Engine::from_parts(Memory::default(), StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_signal_listener_for([SignalKind::ItemScraped], listener);

        block_on(engine.enqueue(Request::new("https://example.com/item"))).unwrap();

        let mut step_executes = BTreeMap::new();
        block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes)).unwrap();

        assert_eq!(
            recorded_events.lock().unwrap().clone(),
            vec![SignalKind::ItemScraped]
        );
        assert!(recorded_urls.lock().unwrap().is_empty());
        assert_eq!(
            recorded_titles.lock().unwrap().clone(),
            vec!["post".to_string()]
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

        let mut step_executes = BTreeMap::new();
        block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes)).unwrap();

        assert_eq!(
            recorded_events.lock().unwrap().clone(),
            vec![
                SignalKind::RequestScheduled,
                SignalKind::SchedulerEvent,
                SignalKind::ResponseReceived,
                SignalKind::ItemScraped,
                SignalKind::SchedulerEvent,
            ]
        );
    }

    #[test]
    fn engine_with_signal_listener_receives_scheduler_signals() {
        let listener = RecordingSignalListener::default();
        let recorded_events = listener.events.clone();
        let recorded_scheduler_events = listener.scheduler_events.clone();

        let mut engine = Engine::from_parts(Memory::default(), StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_signal_listener_for([SignalKind::SchedulerEvent], listener);

        block_on(engine.enqueue(Request::new("https://example.com/item"))).unwrap();

        let mut step_executes = BTreeMap::new();
        block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes)).unwrap();

        assert_eq!(
            recorded_events.lock().unwrap().clone(),
            vec![SignalKind::SchedulerEvent, SignalKind::SchedulerEvent]
        );
        assert_eq!(
            recorded_scheduler_events.lock().unwrap().clone(),
            vec![SchedulerEventKind::Claimed, SchedulerEventKind::Completed]
        );
    }

    #[test]
    fn engine_with_filtered_extension_only_receives_selected_events() {
        let extension = RecordingExtension::default();
        let recorded_events = extension.events.clone();

        let mut engine = Engine::from_parts(Memory::default(), StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_extension_for([SignalKind::ResponseReceived], extension);

        block_on(engine.enqueue(Request::new("https://example.com/item"))).unwrap();

        let mut step_executes = BTreeMap::new();
        block_on(engine.execute_spider_once(&ItemSpider, None, &mut step_executes)).unwrap();

        assert_eq!(
            recorded_events.lock().unwrap().clone(),
            vec![SignalKind::ResponseReceived]
        );
    }

    #[tokio::test]
    async fn engine_run_emits_spider_opened_and_closed_signals() {
        let listener = RecordingSignalListener::default();
        let recorded_events = listener.events.clone();

        let mut engine = Engine::from_parts(Memory::default(), StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_signal_listener(listener)
            .with_config(Config::default().with_idle_timeout(SignedDuration::from_millis(5)));

        let shutdown = engine.shutdown_handle();
        tokio::spawn(async move {
            tokio::time::sleep(to_std_duration(SignedDuration::from_millis(20)).unwrap()).await;
            shutdown.stop();
        });

        engine.run(&IdleSpider).await.unwrap();
        assert_eq!(
            recorded_events.lock().unwrap().clone(),
            vec![SignalKind::SpiderOpened, SignalKind::SpiderClosed]
        );
    }

    #[tokio::test]
    async fn engine_run_uses_scheduler_batch_claim_path() {
        let scheduler = BatchRecordingScheduler::default();
        let batch_calls = scheduler.batch_calls.clone();
        let single_calls = scheduler.single_calls.clone();

        let mut engine = Engine::from_parts(scheduler, StubHttp, StubBrowser)
            .with_store(MemoryStore::default())
            .with_config(
                Config::default()
                    .with_concurrent_requests(3)
                    .with_idle_timeout(SignedDuration::from_millis(5)),
            );

        let shutdown = engine.shutdown_handle();
        tokio::spawn(async move {
            tokio::time::sleep(to_std_duration(SignedDuration::from_millis(40)).unwrap()).await;
            shutdown.stop();
        });

        engine.run(&BatchStartSpider).await.unwrap();
        assert!(*batch_calls.lock().unwrap() > 0);
        assert_eq!(*single_calls.lock().unwrap(), 0);
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 3,
                response_count: 3,
                scheduler_claim_count: 3,
                scheduler_complete_count: 3,
                ..StatsSnapshot::default()
            }
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
            .with_config(retry_settings())
            .with_store(MemoryStore::default());
        let mut step_executes = BTreeMap::new();

        let first =
            block_on(engine.execute_spider_once(&SimpleSpider("retry"), None, &mut step_executes))
                .unwrap();
        let second =
            block_on(engine.execute_spider_once(&SimpleSpider("retry"), None, &mut step_executes))
                .unwrap();

        assert!(first.is_none());
        assert!(second.is_some());
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                request_count: 2,
                response_count: 2,
                retry_count: 1,
                scheduler_claim_count: 2,
                scheduler_complete_count: 2,
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
            .with_config(
                Config::default()
                    .with_robots_obey(true)
                    .with_robots_user_agent("kun-bot"),
            )
            .with_store(MemoryStore::default());

        let mut step_executes = BTreeMap::new();
        let output =
            block_on(engine.execute_spider_once(&SimpleSpider("robots"), None, &mut step_executes))
                .unwrap();

        assert!(output.is_none());
        assert_eq!(*fetches.lock().unwrap(), 0);
        assert_eq!(
            engine.stats(),
            StatsSnapshot {
                robots_disallow_count: 1,
                scheduler_claim_count: 1,
                scheduler_complete_count: 1,
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
            .with_config(
                Config::default()
                    .with_robots_obey(true)
                    .with_robots_user_agent("kun-bot"),
            )
            .with_store(MemoryStore::default());

        let mut step_executes = BTreeMap::new();
        let first = block_on(engine.execute_spider_once(
            &SimpleSpider("robots_crawl_delay"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        let second = block_on(engine.execute_spider_once(
            &SimpleSpider("robots_crawl_delay"),
            None,
            &mut step_executes,
        ))
        .unwrap();
        std::thread::sleep(to_std_duration(SignedDuration::from_millis(15)).unwrap());
        let third = block_on(engine.execute_spider_once(
            &SimpleSpider("robots_crawl_delay"),
            None,
            &mut step_executes,
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
                retry_count: 0,
                robots_delay_count: 1,
                scheduler_claim_count: 3,
                scheduler_complete_count: 3,
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
        .with_config(Config::default().with_robots_sitemap_seeds(true))
        .with_store(MemoryStore::default());
        let step_executes = build_test_step_executes(&engine, &StartUrlSpider, None);

        block_on(engine.enqueue_start_requests(&StartUrlSpider, &[], None, &step_executes))
            .unwrap();

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
        .with_config(Config::default().with_robots_sitemap_seeds(true))
        .with_store(MemoryStore::default());
        let step_executes = build_test_step_executes(&engine, &StartUrlSpider, None);

        block_on(engine.enqueue_start_requests(&StartUrlSpider, &[], None, &step_executes))
            .unwrap();

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
        .with_config(
            Config::default()
                .with_robots_sitemap_seeds(true)
                .with_robots_sitemap_seed_priority(12)
                .with_robots_sitemap_seed_depth(2),
        )
        .with_store(MemoryStore::default());
        let step_executes = build_test_step_executes(&engine, &StartUrlSpider, None);

        block_on(engine.enqueue_start_requests(&StartUrlSpider, &[], None, &step_executes))
            .unwrap();

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
        let step_executes = build_test_step_executes(&engine, &CustomStartRequestSpider, None);

        block_on(engine.enqueue_start_requests(
            &CustomStartRequestSpider,
            &[],
            None,
            &step_executes,
        ))
        .unwrap();

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
    fn engine_uses_rules_seeds_as_start_requests_when_present() {
        let mut engine = Engine::from_parts(Memory::default(), HtmlHttp, StubBrowser)
            .with_store(MemoryStore::default());
        let compiled = load_test_rules(&RulesStartRequestSpider);
        let step_executes =
            build_test_step_executes(&engine, &RulesStartRequestSpider, Some(&compiled));

        block_on(engine.enqueue_start_requests(
            &RulesStartRequestSpider,
            &[],
            Some(&compiled),
            &step_executes,
        ))
        .unwrap();

        let checkpoint = engine.scheduler.checkpoint();
        let request = &checkpoint.ready[0].request;

        assert_eq!(request.url, "https://example.com/from-rules-seed");
        assert_eq!(request.mode, RequestMode::Http);
        assert!(request.headers.is_empty());
        assert!(request.cookies.is_empty());
        assert_eq!(request.timeout, None);
        assert!(request.proxy.is_none());
        assert!(request.session.is_none());
        assert!(request.browser.is_none());
        assert!(request.http.is_some());
        assert_eq!(
            request.meta.get("next_step"),
            Some(&Value::String("parse".to_string()))
        );
    }

    #[test]
    fn engine_falls_back_to_spider_start_requests_when_rules_seeds_are_empty() {
        let mut engine = Engine::from_parts(Memory::default(), HtmlHttp, StubBrowser)
            .with_store(MemoryStore::default());
        let compiled = load_test_rules(&NoSeedRulesStartRequestSpider);
        let step_executes =
            build_test_step_executes(&engine, &NoSeedRulesStartRequestSpider, Some(&compiled));

        block_on(engine.enqueue_start_requests(
            &NoSeedRulesStartRequestSpider,
            &[],
            Some(&compiled),
            &step_executes,
        ))
        .unwrap();

        let checkpoint = engine.scheduler.checkpoint();
        let request = &checkpoint.ready[0].request;

        assert_eq!(request.url, "https://example.com/fallback-start");
        assert_eq!(request.mode, RequestMode::Http);
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
        .with_config(Config::default().with_robots_sitemap_seeds(true))
        .with_store(MemoryStore::default());
        let step_executes = build_test_step_executes(&engine, &CustomStartRequestSpider, None);

        block_on(engine.enqueue_start_requests(
            &CustomStartRequestSpider,
            &[],
            None,
            &step_executes,
        ))
        .unwrap();

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
        .with_config(Config::default().with_robots_sitemap_seeds(true))
        .with_store(MemoryStore::default());
        let step_executes = build_test_step_executes(&engine, &CustomStartRequestSpider, None);

        block_on(engine.enqueue_start_requests(
            &CustomStartRequestSpider,
            &[],
            None,
            &step_executes,
        ))
        .unwrap();

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

    fn load_test_rules<Sp: Spider>(spider: &Sp) -> Compiled {
        let rules = spider.rules().expect("rules should exist");
        block_on(crate::rules::load(&rules)).expect("rules should load")
    }

    fn build_test_step_executes<S, H, B, P, Sp>(
        engine: &Engine<S, H, B, P>,
        spider: &Sp,
        compiled: Option<&Compiled>,
    ) -> BTreeMap<String, StepExecute>
    where
        S: Scheduler,
        H: Downloader,
        B: Downloader,
        P: Pipeline,
        Sp: Spider,
    {
        engine
            .build_step_executes(compiled, spider.validator())
            .expect("step executes should build")
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    struct RecordMiddleware {
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Middleware for RecordMiddleware {
        async fn before_download(
            &self,
            _context: &mut context::Download,
        ) -> Result<flow::Download, SpiderError> {
            self.log.lock().unwrap().push("request".to_string());
            Ok(flow::Download::Continue)
        }

        async fn after_download(
            &self,
            _context: &mut context::Download,
            _response: &mut Response,
        ) -> Result<flow::Download, SpiderError> {
            self.log.lock().unwrap().push("response".to_string());
            Ok(flow::Download::Continue)
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
        .with_config(Config::default().with_robots_sitemap_seeds(true))
        .with_store(MemoryStore::default());
        let step_executes = build_test_step_executes(&engine, &StartUrlSpider, None);

        block_on(engine.enqueue_start_requests(&StartUrlSpider, &[], None, &step_executes))
            .unwrap();

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

        async fn parse(&self, _response: &Response) -> Result<(), SpiderError> {
            Ok(())
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

    struct BatchStartSpider;

    impl Spider for BatchStartSpider {
        fn name(&self) -> &str {
            "batch_start_spider"
        }

        fn start_urls(&self) -> Vec<String> {
            vec![
                "https://example.com/batch/a".to_string(),
                "https://example.com/batch/b".to_string(),
                "https://example.com/batch/c".to_string(),
            ]
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

    struct RulesStartRequestSpider;

    impl Spider for RulesStartRequestSpider {
        fn name(&self) -> &str {
            "rules_start_request_spider"
        }

        fn start_urls(&self) -> Vec<String> {
            vec!["https://example.com/fallback-start".to_string()]
        }

        fn rules(&self) -> Option<RulesConfig> {
            Some(RulesConfig::inline(Value::from(json!({
                "spider": {
                    "name": "rules_start_request_spider"
                },
                "sinks": {
                    "default": {
                        "type": "memory"
                    }
                },
                "seeds": [
                    {
                        "id": "start",
                        "request": {
                            "url": "https://example.com/from-rules-seed"
                        },
                        "next_step": "parse"
                    }
                ],
                "steps": [
                    {
                        "id": "parse",
                        "output": {
                            "item": {
                                "url": { "from": "$response.url" }
                            },
                            "sinks": ["default"]
                        },
                    }
                ]
            }))))
        }
    }

    struct NoSeedRulesStartRequestSpider;

    impl Spider for NoSeedRulesStartRequestSpider {
        fn name(&self) -> &str {
            "no_seed_rules_start_request_spider"
        }

        fn start_urls(&self) -> Vec<String> {
            vec!["https://example.com/fallback-start".to_string()]
        }

        fn rules(&self) -> Option<RulesConfig> {
            Some(RulesConfig::inline(Value::from(json!({
                "spider": {
                    "name": "no_seed_rules_start_request_spider"
                },
                "sinks": {
                    "default": {
                        "type": "memory"
                    }
                },
                "seeds": [],
                "steps": [
                    {
                        "id": "parse",
                        "output": {
                            "item": {
                                "url": { "from": "$response.url" }
                            },
                            "sinks": ["default"]
                        },
                    }
                ]
            }))))
        }
    }

    struct ItemSpider;

    impl Spider for ItemSpider {
        fn name(&self) -> &str {
            "item_spider"
        }

        async fn parse(&self, _response: &Response) -> Result<crate::item::Item, SpiderError> {
            Ok(crate::item::Item::new().with_field("title", Value::String("post".to_string())))
        }
    }

    struct RoutedItemSpider;

    impl Spider for RoutedItemSpider {
        fn name(&self) -> &str {
            "routed_item_spider"
        }

        fn rules(&self) -> Option<RulesConfig> {
            Some(RulesConfig::inline(Value::from(json!({
                "spider": {
                    "name": "routed_item_spider"
                },
                "sinks": {
                    "default": {
                        "type": "memory"
                    },
                    "article_db": {
                        "type": "memory"
                    },
                    "article_file": {
                        "type": "memory"
                    }
                },
                "seeds": [],
                "steps": [
                    {
                        "id": "parse",
                        "callback": "parse",
                        "output": {
                            "item": {
                                "title": { "from": "$response.url" }
                            },
                            "sinks": ["article_db", "article_file"]
                        }
                    }
                ]
            }))))
        }

        async fn parse(&self, _response: &Response) -> Result<crate::item::Item, SpiderError> {
            Ok(crate::item::Item::new().with_field("title", Value::String("post".to_string())))
        }
    }

    struct MissingStoreSpider;

    impl Spider for MissingStoreSpider {
        fn name(&self) -> &str {
            "missing_store_spider"
        }

        fn rules(&self) -> Option<RulesConfig> {
            Some(RulesConfig::inline(Value::from(json!({
                "spider": {
                    "name": "missing_store_spider"
                },
                "sinks": {
                    "default": {
                        "type": "memory"
                    },
                    "missing": {
                        "type": "memory"
                    }
                },
                "seeds": [],
                "steps": [
                    {
                        "id": "parse",
                        "callback": "parse",
                        "output": {
                            "item": {
                                "title": { "from": "$response.url" }
                            },
                            "sinks": ["missing"]
                        }
                    }
                ]
            }))))
        }

        async fn parse(&self, _response: &Response) -> Result<crate::item::Item, SpiderError> {
            Ok(crate::item::Item::new().with_field("title", Value::String("post".to_string())))
        }
    }

    struct ValidatedItemSpider;

    impl Spider for ValidatedItemSpider {
        fn name(&self) -> &str {
            "validated_item_spider"
        }

        fn validator(&self) -> Option<validator::StepValidator> {
            Some(
                validator::StepValidator::new()
                    .field("title", validator::Type::Text, |field| field.required())
                    .field("published_at", validator::Type::Text, |field| {
                        field
                            .transform(validator::Transform::ParseDatetime)
                            .required()
                    }),
            )
        }

        async fn parse(&self, _response: &Response) -> Result<crate::item::Item, SpiderError> {
            Ok(crate::item::Item::new()
                .with_field("title", Value::String("post".to_string()))
                .with_field(
                    "published_at",
                    Value::String("2026-04-08 10:00:00".to_string()),
                ))
        }
    }

    struct InvalidValidatedItemSpider;

    impl Spider for InvalidValidatedItemSpider {
        fn name(&self) -> &str {
            "invalid_validated_item_spider"
        }

        fn validator(&self) -> Option<validator::StepValidator> {
            Some(
                validator::StepValidator::new()
                    .field("title", validator::Type::Text, |field| field.required())
                    .field("published_at", validator::Type::Text, |field| {
                        field
                            .transform(validator::Transform::ParseDatetime)
                            .required()
                    }),
            )
        }

        async fn parse(&self, _response: &Response) -> Result<crate::item::Item, SpiderError> {
            Ok(crate::item::Item::new()
                .with_field("title", Value::String("post".to_string()))
                .with_field("published_at", Value::String("not-a-datetime".to_string())))
        }
    }

    struct FailingSpider;

    impl Spider for FailingSpider {
        fn name(&self) -> &str {
            "failing_spider"
        }

        async fn parse(&self, _response: &Response) -> Result<(), SpiderError> {
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

        async fn parse(&self, response: &Response) -> Result<crate::item::Item, SpiderError> {
            Ok(crate::item::Item::new()
                .with_field("status", Value::Number(response.status as f64))
                .with_field("text", Value::String(response.text.clone()))
                .with_field(
                    "flags",
                    Value::Array(response.flags.iter().cloned().map(Value::String).collect()),
                ))
        }
    }

    struct CallbackDedupSpider;

    impl Spider for CallbackDedupSpider {
        fn name(&self) -> &str {
            "callback_dedup_spider"
        }

        async fn parse(
            &self,
            response: &Response,
        ) -> Result<impl crate::spider::IntoSpiderResultParts, SpiderError> {
            if response.url.ends_with("/start") {
                return Ok(crate::spider::into_spider_result_parts((
                    crate::item::Item::new().with_field("title", Value::String("root".to_string())),
                    vec![
                        Request::new("https://example.com/detail"),
                        Request::new("https://example.com/detail"),
                    ],
                )));
            }

            Ok(crate::spider::into_spider_result_parts(
                crate::item::Item::new().with_field("title", Value::String("detail".to_string())),
            ))
        }
    }

    struct CallbackIntervalSpider;

    impl Spider for CallbackIntervalSpider {
        fn name(&self) -> &str {
            "callback_interval_spider"
        }

        async fn parse(
            &self,
            response: &Response,
        ) -> Result<impl crate::spider::IntoSpiderResultParts, SpiderError> {
            if response.url.ends_with("/start") {
                let interval = BTreeMap::from([("interval".to_string(), Value::Number(20.0))]);
                return Ok(crate::spider::into_spider_result_parts(vec![
                    Request::new("https://example.com/detail/1")
                        .with_interval(interval.clone(), 120),
                    Request::new("https://example.com/detail/2").with_interval(interval, 120),
                ]));
            }

            Ok(crate::spider::into_spider_result_parts(
                crate::item::Item::new().with_field("title", Value::String(response.url.clone())),
            ))
        }
    }

    struct CallbackRetrySpider;

    impl Spider for CallbackRetrySpider {
        fn name(&self) -> &str {
            "callback_retry_spider"
        }

        async fn parse(
            &self,
            response: &Response,
        ) -> Result<impl crate::spider::IntoSpiderResultParts, SpiderError> {
            if response.url.ends_with("/start") {
                return Ok(crate::spider::into_spider_result_parts(vec![
                    Request::new("https://example.com/detail").with_retry_by_status(
                        BTreeMap::from([
                            ("count".to_string(), Value::Number(1.0)),
                            (
                                "http_status".to_string(),
                                Value::Array(vec![Value::Number(500.0)]),
                            ),
                        ]),
                        200,
                    ),
                ]));
            }

            Ok(crate::spider::into_spider_result_parts(
                crate::item::Item::new().with_field("title", Value::String("detail".to_string())),
            ))
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
        scheduler_events: Arc<Mutex<Vec<SchedulerEventKind>>>,
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
                    Signal::SchedulerEvent(signal) => {
                        self.scheduler_events.lock().unwrap().push(signal.event);
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

    fn failure_item(failure: &Failure) -> crate::item::Item {
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
                failure
                    .cb_kwargs()
                    .get("page")
                    .cloned()
                    .unwrap_or(Value::Null),
            )
            .with_field(
                "source",
                failure
                    .cb_kwargs()
                    .get("source")
                    .cloned()
                    .unwrap_or(Value::Null),
            )
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
        ) -> Result<crate::item::Item, SpiderError> {
            match name {
                "handle_failure" => Ok(failure_item(failure)),
                other => Err(SpiderError::engine(format!("unknown errback: {other}"))),
            }
        }
    }

    struct ParseErrorSpider;

    impl Spider for ParseErrorSpider {
        fn name(&self) -> &str {
            "parse_error_spider"
        }

        async fn parse(&self, _response: &Response) -> Result<(), SpiderError> {
            Err(SpiderError::parse("parse exploded"))
        }

        async fn handle_error(
            &self,
            name: &str,
            failure: &Failure,
        ) -> Result<crate::item::Item, SpiderError> {
            match name {
                "handle_failure" => Ok(failure_item(failure)),
                other => Err(SpiderError::engine(format!("unknown errback: {other}"))),
            }
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

    #[derive(Default)]
    struct FailCompleteAndEnqueueScheduler {
        inner: Memory,
    }

    impl Scheduler for FailCompleteAndEnqueueScheduler {
        async fn enqueue(&self, task: Task) -> Result<(), SpiderError> {
            self.inner.enqueue(task).await
        }

        async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
            Scheduler::checkpoint(&self.inner).await
        }

        async fn counts(&self) -> Result<crate::scheduler::checkpoint::Counts, SpiderError> {
            Scheduler::counts(&self.inner).await
        }

        async fn snapshot(&self) -> Result<crate::scheduler::Snapshot, SpiderError> {
            self.inner.snapshot().await
        }

        async fn take_ready(&self) -> Result<Option<crate::scheduler::ClaimedTask>, SpiderError> {
            self.inner.take_ready().await
        }

        async fn complete(&self, lease: &crate::scheduler::TaskLease) -> Result<(), SpiderError> {
            self.inner.complete(lease).await
        }

        async fn complete_and_enqueue(
            &self,
            _lease: &crate::scheduler::TaskLease,
            _tasks: Vec<Task>,
        ) -> Result<(), SpiderError> {
            Err(SpiderError::scheduler(
                "complete_and_enqueue failed after store commit",
            ))
        }

        async fn requeue(&self, lease: &crate::scheduler::TaskLease) -> Result<(), SpiderError> {
            self.inner.requeue(lease).await
        }

        async fn has_pending(&self) -> Result<bool, SpiderError> {
            self.inner.has_pending().await
        }
    }

    #[derive(Clone, Default)]
    struct BatchRecordingScheduler {
        inner: Arc<Memory>,
        batch_calls: Arc<Mutex<usize>>,
        single_calls: Arc<Mutex<usize>>,
    }

    impl Scheduler for BatchRecordingScheduler {
        async fn enqueue(&self, task: Task) -> Result<(), SpiderError> {
            self.inner.enqueue(task).await
        }

        async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
            Scheduler::checkpoint(self.inner.as_ref()).await
        }

        async fn counts(&self) -> Result<crate::scheduler::checkpoint::Counts, SpiderError> {
            Scheduler::counts(self.inner.as_ref()).await
        }

        async fn snapshot(&self) -> Result<crate::scheduler::Snapshot, SpiderError> {
            self.inner.snapshot().await
        }

        async fn take_ready(&self) -> Result<Option<crate::scheduler::ClaimedTask>, SpiderError> {
            *self.single_calls.lock().unwrap() += 1;
            self.inner.take_ready().await
        }

        async fn take_batch_ready(
            &self,
            limit: usize,
        ) -> Result<Vec<crate::scheduler::ClaimedTask>, SpiderError> {
            *self.batch_calls.lock().unwrap() += 1;
            self.inner.take_batch_ready(limit).await
        }

        async fn complete(&self, lease: &crate::scheduler::TaskLease) -> Result<(), SpiderError> {
            self.inner.complete(lease).await
        }

        async fn complete_and_enqueue(
            &self,
            lease: &crate::scheduler::TaskLease,
            tasks: Vec<Task>,
        ) -> Result<(), SpiderError> {
            self.inner.complete_and_enqueue(lease, tasks).await
        }

        async fn requeue(&self, lease: &crate::scheduler::TaskLease) -> Result<(), SpiderError> {
            self.inner.requeue(lease).await
        }

        async fn has_pending(&self) -> Result<bool, SpiderError> {
            self.inner.has_pending().await
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

    fn default_request_middleware_settings() -> Config {
        Config::default()
            .with_request_middleware(
                INTERVAL,
                download_middleware(120, [("interval".to_string(), Value::Number(1000.0))]),
            )
            .with_request_middleware(
                RATE_LIMIT,
                download_middleware(130, [("rate_per_minute".to_string(), Value::Number(60.0))]),
            )
            .with_request_middleware(
                RETRY_BY_ERROR,
                download_middleware(
                    210,
                    [("count".to_string(), Value::Array(vec![Value::Number(3.0)]))],
                ),
            )
    }

    fn retry_settings() -> Config {
        Config::default()
            .with_retry_times(1)
            .with_retry_http_codes(vec![500])
    }

    fn retry_backoff_settings() -> Config {
        Config::default().with_request_middleware(
            RETRY_BY_STATUS,
            download_middleware(
                200,
                [
                    ("count".to_string(), Value::Array(vec![Value::Number(1.0)])),
                    (
                        "backoff".to_string(),
                        Value::Array(vec![Value::Number(10.0)]),
                    ),
                    (
                        "status".to_string(),
                        Value::Array(vec![Value::Number(500.0)]),
                    ),
                ],
            ),
        )
    }

    fn interval_settings() -> Config {
        Config::default().with_download_delay(SignedDuration::from_millis(10))
    }

    fn rate_limit_settings() -> Config {
        Config::default().with_request_middleware(
            RATE_LIMIT,
            download_middleware(130, [("rate_per_minute".to_string(), Value::Number(1.0))]),
        )
    }

    fn auto_throttle_settings() -> Config {
        Config::default()
            .with_auto_throttle(true)
            .with_auto_throttle_target_concurrency(1.0)
            .with_download_delay(SignedDuration::from_millis(0))
            .with_auto_throttle_max_delay(SignedDuration::from_millis(500))
    }

    fn http_cache_settings() -> Config {
        Config::default().with_http_cache(true)
    }

    fn download_middleware<const N: usize>(
        order: i32,
        options: [(String, Value); N],
    ) -> crate::middleware::Config {
        crate::middleware::Config {
            enabled: true,
            stage: Stage::Download,
            order,
            options: options.into_iter().collect(),
        }
    }
}
