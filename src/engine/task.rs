use crate::download::traits::Downloader;
use crate::engine::context::EngineContext;
use crate::engine::flow::Flow;
use crate::error::SpiderError;
use crate::middleware::{Chain, Stage};
use crate::request::{Request, RequestMode};
use crate::response::Response;
use crate::rules::Compiled;
use crate::scheduler::{Scheduler, Task, TaskId, TaskLease};
use crate::spider::{Failure, Output as SpiderOutput, Spider};
use std::sync::Arc;

pub(super) async fn enqueue_request<S, D>(
    scheduler: &S,
    dedup: &mut D,
    request: Request,
    allowed_domains: &[String],
    stats: Option<&crate::stats::Tracker>,
) -> Result<bool, SpiderError>
where
    S: Scheduler,
    D: crate::dedup::Dedup,
{
    enqueue_task(scheduler, dedup, Task::new(request), allowed_domains, stats).await
}

pub(super) async fn enqueue_task<S, D>(
    scheduler: &S,
    dedup: &mut D,
    task: Task,
    allowed_domains: &[String],
    stats: Option<&crate::stats::Tracker>,
) -> Result<bool, SpiderError>
where
    S: Scheduler,
    D: crate::dedup::Dedup,
{
    let Some(task) = prepare_task_for_enqueue(dedup, task, allowed_domains, stats).await? else {
        return Ok(false);
    };

    scheduler.enqueue(task).await?;
    Ok(true)
}

pub(super) async fn prepare_task_for_enqueue<D>(
    dedup: &mut D,
    task: Task,
    allowed_domains: &[String],
    stats: Option<&crate::stats::Tracker>,
) -> Result<Option<Task>, SpiderError>
where
    D: crate::dedup::Dedup,
{
    let request = &task.request;

    if !request.dont_filter && !super::is_domain_allowed(&request.url, allowed_domains) {
        return Ok(None);
    }

    if !request.dont_filter && !dedup.check_and_insert(&request).await? {
        if let Some(stats) = stats {
            stats.record_dedup_reject();
        }
        return Ok(None);
    }

    Ok(Some(task))
}

pub(super) async fn record_scheduler_event(
    spider_name: &str,
    event: crate::signals::SchedulerEventKind,
    lease: &TaskLease,
    url: &str,
    stats: &crate::stats::Tracker,
    signals: &crate::signals::Bus,
    error: Option<SpiderError>,
) {
    match event {
        crate::signals::SchedulerEventKind::Claimed => stats.record_scheduler_claim(),
        crate::signals::SchedulerEventKind::Completed => stats.record_scheduler_complete(),
        crate::signals::SchedulerEventKind::Requeued => stats.record_scheduler_requeue(),
        crate::signals::SchedulerEventKind::Heartbeat => stats.record_scheduler_heartbeat(),
        crate::signals::SchedulerEventKind::LeaseLost => stats.record_scheduler_lease_lost(),
    }

    signals
        .emit(crate::signals::Signal::scheduler_event(
            spider_name,
            event,
            lease,
            url,
            error,
        ))
        .await;
}

pub(super) async fn apply_task_run<S, D>(
    run: TaskRun,
    scheduler: &S,
    dedup: &mut D,
    allowed_domains: &[String],
    outputs: &mut Vec<SpiderOutput>,
    round: &mut usize,
    spider_name: &str,
    stats: &crate::stats::Tracker,
    signals: &crate::signals::Bus,
) -> Result<(), SpiderError>
where
    S: Scheduler,
    D: crate::dedup::Dedup,
{
    let lease = run.lease;
    let url = run.url;
    match run.outcome {
        TaskOutcome::Success(output) => {
            *round += 1;
            let store_committed = !output.items.is_empty();
            let mut scheduled_follows = Vec::new();
            let mut follow_tasks = Vec::new();
            for follow in &output.follows {
                let task = prepare_task_for_enqueue(
                    dedup,
                    Task::new(follow.clone()),
                    allowed_domains,
                    Some(stats),
                )
                .await?;

                if let Some(task) = task {
                    scheduled_follows.push(follow.clone());
                    follow_tasks.push(task);
                }
            }
            let committed = resolve_scheduler_transition(
                scheduler.complete_and_enqueue(&lease, follow_tasks),
                &lease,
                crate::signals::SchedulerEventKind::Completed,
                url.as_str(),
                spider_name,
                stats,
                signals,
                "complete_and_enqueue",
                store_committed,
                scheduled_follows.len(),
            )
            .await?;
            if committed {
                for follow in &scheduled_follows {
                    signals
                        .emit(crate::signals::Signal::request_scheduled(
                            spider_name,
                            follow.clone(),
                        ))
                        .await;
                }
            }
            outputs.push(SpiderOutput {
                items: output.items,
                requests: output.follows,
            });
        }
        TaskOutcome::Retry(retry_task) => {
            stats.record_retry();
            let retry_task = *retry_task;
            let request = retry_task.request.clone();
            let retry_tasks =
                prepare_task_for_enqueue(dedup, retry_task, allowed_domains, Some(stats))
                    .await?
                    .into_iter()
                    .collect::<Vec<_>>();
            let queued_retry_tasks = retry_tasks.len();
            let committed = resolve_scheduler_transition(
                scheduler.complete_and_enqueue(&lease, retry_tasks),
                &lease,
                crate::signals::SchedulerEventKind::Completed,
                url.as_str(),
                spider_name,
                stats,
                signals,
                "complete_and_enqueue",
                false,
                queued_retry_tasks,
            )
            .await?;
            if committed {
                signals
                    .emit(crate::signals::Signal::request_scheduled(
                        spider_name,
                        request,
                    ))
                    .await;
            }
        }
        TaskOutcome::Drop => {
            resolve_scheduler_transition(
                scheduler.complete(&lease),
                &lease,
                crate::signals::SchedulerEventKind::Completed,
                url.as_str(),
                spider_name,
                stats,
                signals,
                "complete",
                false,
                0,
            )
            .await?;
        }
        TaskOutcome::Error(error) => {
            stats.record_error();
            crate::trace::error(
                "task.fail",
                vec![
                    crate::trace::prop("spider", spider_name),
                    crate::trace::prop("url", url.as_str()),
                    crate::trace::prop("error", &error),
                ],
            );
            resolve_scheduler_transition(
                scheduler.requeue(&lease),
                &lease,
                crate::signals::SchedulerEventKind::Requeued,
                url.as_str(),
                spider_name,
                stats,
                signals,
                "requeue",
                false,
                0,
            )
            .await?;
        }
        TaskOutcome::LeaseLost(error) => {
            stats.record_error();
            crate::trace::warn(
                "task.lease_lost",
                vec![
                    crate::trace::prop("spider", spider_name),
                    crate::trace::prop("task", lease.task_id().as_str()),
                    crate::trace::prop("worker", lease.worker_id()),
                    crate::trace::prop("url", url.as_str()),
                    crate::trace::prop("error", &error),
                ],
            );
            record_scheduler_event(
                spider_name,
                crate::signals::SchedulerEventKind::LeaseLost,
                &lease,
                url.as_str(),
                stats,
                signals,
                Some(error),
            )
            .await;
        }
    }
    Ok(())
}

pub(super) async fn resolve_scheduler_transition(
    transition: impl std::future::Future<Output = Result<(), SpiderError>>,
    lease: &TaskLease,
    event: crate::signals::SchedulerEventKind,
    url: &str,
    spider_name: &str,
    stats: &crate::stats::Tracker,
    signals: &crate::signals::Bus,
    action: &'static str,
    store_committed: bool,
    queued_task_count: usize,
) -> Result<bool, SpiderError> {
    match transition.await {
        Ok(()) => {
            crate::trace::info(
                "engine.commit.scheduler_resolve.ok",
                vec![
                    crate::trace::prop("spider", spider_name),
                    crate::trace::prop("action", action),
                    crate::trace::prop("task", lease.task_id().as_str()),
                    crate::trace::prop("worker", lease.worker_id()),
                    crate::trace::prop("lease", lease.lease_id()),
                    crate::trace::prop("url", url),
                    crate::trace::prop("store_committed", store_committed),
                    crate::trace::prop("queued_tasks", queued_task_count),
                ],
            );
            record_scheduler_event(spider_name, event, lease, url, stats, signals, None).await;
            Ok(true)
        }
        Err(error) if error.is_scheduler_lease_resolution_error() => {
            stats.record_error();
            record_scheduler_event(
                spider_name,
                crate::signals::SchedulerEventKind::LeaseLost,
                lease,
                url,
                stats,
                signals,
                Some(error.clone()),
            )
            .await;
            crate::trace::warn(
                "task.lease_resolve_fail",
                vec![
                    crate::trace::prop("spider", spider_name),
                    crate::trace::prop("task", lease.task_id().as_str()),
                    crate::trace::prop("worker", lease.worker_id()),
                    crate::trace::prop("lease", lease.lease_id()),
                    crate::trace::prop("action", action),
                    crate::trace::prop("url", url),
                    crate::trace::prop("store_committed", store_committed),
                    crate::trace::prop("queued_tasks", queued_task_count),
                    crate::trace::prop("error", &error),
                ],
            );
            Ok(false)
        }
        Err(error) => {
            crate::trace::warn(
                "engine.commit.scheduler_resolve.fail",
                vec![
                    crate::trace::prop("spider", spider_name),
                    crate::trace::prop("action", action),
                    crate::trace::prop("task", lease.task_id().as_str()),
                    crate::trace::prop("worker", lease.worker_id()),
                    crate::trace::prop("lease", lease.lease_id()),
                    crate::trace::prop("url", url),
                    crate::trace::prop("store_committed", store_committed),
                    crate::trace::prop("queued_tasks", queued_task_count),
                    crate::trace::prop("error", &error),
                ],
            );
            Err(error)
        }
    }
}

pub(super) struct TaskRunReservation {
    lease: TaskLease,
    request: Request,
    global_permit_guard: tokio::sync::OwnedSemaphorePermit,
    domain_semaphore: Arc<tokio::sync::Semaphore>,
}

impl TaskRunReservation {
    pub(super) fn new(
        lease: TaskLease,
        request: Request,
        global_permit_guard: tokio::sync::OwnedSemaphorePermit,
        domain_semaphore: Arc<tokio::sync::Semaphore>,
    ) -> Self {
        Self {
            lease,
            request,
            global_permit_guard,
            domain_semaphore,
        }
    }
}

#[derive(Clone)]
pub(super) struct TaskExecutor<'a, S, H, B, P, St, Sp> {
    pub(super) scheduler: &'a S,
    pub(super) http: &'a H,
    pub(super) browser: &'a B,
    pub(super) pipeline: &'a P,
    pub(super) store: &'a St,
    pub(super) robots: &'a dyn crate::robots::Robot,
    pub(super) settings: &'a crate::settings::Settings,
    pub(super) stats: Arc<crate::stats::Tracker>,
    pub(super) signals: Arc<crate::signals::Bus>,
    pub(super) engine_chain: &'a Chain,
    pub(super) step_chain: &'a Chain,
    pub(super) spider: &'a Sp,
    pub(super) compiled: Option<&'a Compiled>,
    pub(super) allowed_domains: &'a [String],
    pub(super) spider_name: &'a str,
}

impl<'a, S, H, B, P, St, Sp> TaskExecutor<'a, S, H, B, P, St, Sp>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
    St: crate::store::Store,
    Sp: Spider,
{
    async fn error_outcome(
        &self,
        request: &Request,
        response: Option<Response>,
        error: SpiderError,
    ) -> TaskOutcome {
        self.signals
            .emit(crate::signals::Signal::spider_error(
                self.spider_name,
                request.clone(),
                response,
                error.clone(),
            ))
            .await;
        TaskOutcome::Error(error)
    }

    async fn error_outcome_from_context(
        &self,
        context: &EngineContext,
        error: SpiderError,
    ) -> TaskOutcome {
        self.error_outcome(&context.request, context.response.clone(), error)
            .await
    }

    async fn process_spider_output(
        self,
        mut output: SpiderOutput,
        request: &Request,
        response: Option<Response>,
    ) -> TaskOutcome {
        let mut kept_items = Vec::with_capacity(output.items.len());
        for mut item in output.items.drain(..) {
            match self.pipeline.process(&mut item, self.spider_name).await {
                Ok(true) => {
                    kept_items.push(item);
                }
                Ok(false) => {
                    self.stats.record_pipeline_drop();
                }
                Err(error) => {
                    crate::trace::warn(
                        "pipeline.fail",
                        vec![
                            crate::trace::prop("spider", self.spider_name),
                            crate::trace::prop("error", &error),
                        ],
                    );
                    return self.error_outcome(request, response.clone(), error).await;
                }
            }
        }

        if !kept_items.is_empty() {
            match self.store.batch_write(&kept_items, self.spider_name).await {
                Ok(()) => {
                    crate::trace::info(
                        "engine.commit.store.ok",
                        vec![
                            crate::trace::prop("spider", self.spider_name),
                            crate::trace::prop("url", request.url.as_str()),
                            crate::trace::prop("items", kept_items.len()),
                        ],
                    );
                    for item in &kept_items {
                        self.stats.record_item();
                        self.signals
                            .emit(crate::signals::Signal::item_scraped(
                                self.spider_name,
                                item.clone(),
                            ))
                            .await;
                    }
                }
                Err(error) => {
                    self.stats.record_store_error();
                    crate::trace::warn(
                        "engine.commit.store.fail",
                        vec![
                            crate::trace::prop("spider", self.spider_name),
                            crate::trace::prop("url", request.url.as_str()),
                            crate::trace::prop("items", kept_items.len()),
                            crate::trace::prop("scheduler_resolve", "skipped"),
                            crate::trace::prop("error", &error),
                        ],
                    );
                    return self.error_outcome(request, response.clone(), error).await;
                }
            }
        }

        let mut follows = Vec::new();
        for request in output.requests {
            if request.dont_filter || super::is_domain_allowed(&request.url, self.allowed_domains) {
                follows.push(request);
            }
        }

        TaskOutcome::Success(TaskOutput {
            items: kept_items,
            follows,
        })
    }

    async fn run_errback(self, context: &EngineContext, error: SpiderError) -> TaskOutcome {
        let Some(errback) = context.request.errback.as_ref() else {
            return self.error_outcome_from_context(context, error).await;
        };

        let failure = Failure::new(context.request.clone(), context.response.clone(), error);

        match self.spider.handle_error(&errback.name, &failure).await {
            Ok(output) => {
                self.process_spider_output(output, &context.request, context.response.clone())
                    .await
            }
            Err(errback_error) => {
                self.error_outcome_from_context(context, errback_error)
                    .await
            }
        }
    }

    pub(super) async fn run_with_reservation(self, reservation: TaskRunReservation) -> TaskRun {
        let TaskRunReservation {
            lease,
            request,
            global_permit_guard,
            domain_semaphore,
        } = reservation;

        let url = request.url.clone();
        let task_id = lease.task_id().clone();
        let scheduler = self.scheduler;
        let spider_name = self.spider_name;
        let stats = self.stats.clone();
        let signals = self.signals.clone();
        let task_future = async move {
            let _global_permit_guard = global_permit_guard;
            let _domain_permit_guard = match domain_semaphore.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    return self
                        .error_outcome(
                            &request,
                            None,
                            SpiderError::engine("domain semaphore closed"),
                        )
                        .await;
                }
            };

            self.run(task_id, request).await
        };

        let outcome = run_task_with_heartbeat(
            scheduler,
            &lease,
            spider_name,
            url.as_str(),
            stats.as_ref(),
            signals.as_ref(),
            task_future,
        )
        .await;

        TaskRun {
            lease,
            url,
            outcome,
        }
    }

    pub(super) async fn run(self, task_id: TaskId, request: Request) -> TaskOutcome {
        let mut context = EngineContext::new(request)
            .with_task_id(task_id)
            .with_stats(self.stats.clone());

        match run_middleware_request(
            self.engine_chain,
            self.step_chain,
            Stage::Download,
            &mut context,
        )
        .await
        {
            Ok(Flow::Continue) => {}
            Ok(flow) => return map_flow_to_task_outcome(flow, &context),
            Err(error) => return self.error_outcome_from_context(&context, error).await,
        }

        if self.settings.robots_obey {
            let user_agent = self.settings.resolved_robots_user_agent(self.spider_name);
            match self
                .robots
                .check(&context.request, user_agent.as_str())
                .await
            {
                Ok(crate::robots::Decision::Allow) => {}
                Ok(crate::robots::Decision::Disallow) => {
                    self.stats.record_robots_disallow();
                    crate::trace::warn(
                        "robots.blocked",
                        vec![
                            crate::trace::prop("spider", self.spider_name),
                            crate::trace::prop("url", context.request.url.as_str()),
                        ],
                    );
                    return TaskOutcome::Drop;
                }
                Ok(crate::robots::Decision::Delay(delay)) => {
                    self.stats.record_robots_delay();
                    let backoff = u64::try_from(delay.as_millis()).unwrap_or_default().max(1);
                    crate::trace::warn(
                        "robots.delayed",
                        vec![
                            crate::trace::prop("spider", self.spider_name),
                            crate::trace::prop("url", context.request.url.as_str()),
                            crate::trace::prop("backoff", backoff),
                        ],
                    );
                    return map_flow_to_task_outcome(
                        Flow::Retry {
                            reason: "robots crawl delay".to_string(),
                            backoff: Some(backoff),
                        },
                        &context,
                    );
                }
                Err(error) => return self.error_outcome_from_context(&context, error).await,
            }
        }

        self.stats.record_request();
        crate::trace::info(
            "request.start",
            vec![
                crate::trace::prop("spider", self.spider_name),
                crate::trace::prop("url", context.request.url.as_str()),
                crate::trace::prop("method", context.request.method.as_str()),
                crate::trace::prop(
                    "mode",
                    match context.request.mode {
                        RequestMode::Http => "http",
                        RequestMode::Browser => "browser",
                    },
                ),
            ],
        );
        let response = match context.request.mode {
            RequestMode::Http => self.http.fetch(&context.request).await,
            RequestMode::Browser => self.browser.fetch(&context.request).await,
        };

        let response = match response {
            Ok(response) => {
                self.stats.record_response();
                crate::trace::info(
                    "request.ok",
                    vec![
                        crate::trace::prop("spider", self.spider_name),
                        crate::trace::prop("url", context.request.url.as_str()),
                        crate::trace::prop("status", response.status),
                        crate::trace::prop("bytes", response.body.len()),
                    ],
                );
                response
            }
            Err(error) => {
                crate::trace::warn(
                    "request.fail",
                    vec![
                        crate::trace::prop("spider", self.spider_name),
                        crate::trace::prop("url", context.request.url.as_str()),
                        crate::trace::prop("error", &error),
                    ],
                );
                match run_middleware_exception(
                    self.engine_chain,
                    self.step_chain,
                    Stage::Download,
                    &mut context,
                    &error,
                )
                .await
                {
                    Ok(Flow::Continue) => return self.run_errback(&context, error).await,
                    Ok(flow) => return map_flow_to_task_outcome(flow, &context),
                    Err(middleware_error) => {
                        return self
                            .error_outcome_from_context(&context, middleware_error)
                            .await;
                    }
                }
            }
        };

        context.response = Some(response.clone());

        match run_middleware_response(
            self.engine_chain,
            self.step_chain,
            Stage::Download,
            &mut context,
        )
        .await
        {
            Ok(Flow::Continue) => {}
            Ok(flow) => return map_flow_to_task_outcome(flow, &context),
            Err(error) => return self.error_outcome_from_context(&context, error).await,
        }

        let response = context.response.clone().unwrap_or(response);
        self.signals
            .emit(crate::signals::Signal::response_received(
                self.spider_name,
                response.clone(),
            ))
            .await;

        match run_middleware_request(
            self.engine_chain,
            self.step_chain,
            Stage::Spider,
            &mut context,
        )
        .await
        {
            Ok(Flow::Continue) => {}
            Ok(flow) => return map_flow_to_task_outcome(flow, &context),
            Err(error) => return self.error_outcome_from_context(&context, error).await,
        }

        let output = self.spider.dispatch(&response, self.compiled).await;

        match output {
            Ok(output) => {
                match run_middleware_response(
                    self.engine_chain,
                    self.step_chain,
                    Stage::Spider,
                    &mut context,
                )
                .await
                {
                    Ok(Flow::Continue) => {}
                    Ok(flow) => return map_flow_to_task_outcome(flow, &context),
                    Err(error) => return self.error_outcome_from_context(&context, error).await,
                }

                self.process_spider_output(output, &context.request, context.response.clone())
                    .await
            }
            Err(error) => {
                crate::trace::error(
                    "spider.fail",
                    vec![
                        crate::trace::prop("spider", self.spider_name),
                        crate::trace::prop("url", context.request.url.as_str()),
                        crate::trace::prop("error", &error),
                    ],
                );
                match run_middleware_exception(
                    self.engine_chain,
                    self.step_chain,
                    Stage::Spider,
                    &mut context,
                    &error,
                )
                .await
                {
                    Ok(Flow::Continue) => self.run_errback(&context, error).await,
                    Ok(flow) => map_flow_to_task_outcome(flow, &context),
                    Err(middleware_error) => {
                        self.error_outcome_from_context(&context, middleware_error)
                            .await
                    }
                }
            }
        }
    }
}

async fn run_task_with_heartbeat<S, F>(
    scheduler: &S,
    lease: &TaskLease,
    spider_name: &str,
    url: &str,
    stats: &crate::stats::Tracker,
    signals: &crate::signals::Bus,
    task_future: F,
) -> TaskOutcome
where
    S: Scheduler,
    F: std::future::Future<Output = TaskOutcome>,
{
    let Some(interval) = scheduler
        .heartbeat_interval()
        .and_then(non_negative_std_duration)
    else {
        return task_future.await;
    };

    let mut task_future = std::pin::pin!(task_future);

    loop {
        let sleep = tokio::time::sleep(interval);
        tokio::pin!(sleep);

        tokio::select! {
            outcome = task_future.as_mut() => return outcome,
            _ = &mut sleep => {
                if let Err(error) = scheduler.heartbeat(lease).await {
                    if error.is_scheduler_lease_resolution_error() {
                        return TaskOutcome::LeaseLost(error);
                    }
                    return TaskOutcome::Error(error);
                }
                record_scheduler_event(
                    spider_name,
                    crate::signals::SchedulerEventKind::Heartbeat,
                    lease,
                    url,
                    stats,
                    signals,
                    None,
                )
                .await;
            }
        }
    }
}

fn non_negative_std_duration(duration: jiff::SignedDuration) -> Option<std::time::Duration> {
    std::time::Duration::try_from(duration)
        .ok()
        .filter(|value| !value.is_zero())
}

pub(super) async fn run_middleware_request(
    engine_chain: &Chain,
    step_chain: &Chain,
    stage: Stage,
    context: &mut EngineContext,
) -> Result<Flow, SpiderError> {
    let flow = engine_chain.process_request(stage, context).await?;
    if !matches!(flow, Flow::Continue) {
        return Ok(flow);
    }
    step_chain.process_request(stage, context).await
}

pub(super) async fn run_middleware_response(
    engine_chain: &Chain,
    step_chain: &Chain,
    stage: Stage,
    context: &mut EngineContext,
) -> Result<Flow, SpiderError> {
    let flow = engine_chain.process_response(stage, context).await?;
    if !matches!(flow, Flow::Continue) {
        return Ok(flow);
    }
    step_chain.process_response(stage, context).await
}

async fn run_middleware_exception(
    engine_chain: &Chain,
    step_chain: &Chain,
    stage: Stage,
    context: &mut EngineContext,
    error: &SpiderError,
) -> Result<Flow, SpiderError> {
    let flow = engine_chain
        .process_exception(stage, context, error)
        .await?;
    if !matches!(flow, Flow::Continue) {
        return Ok(flow);
    }
    step_chain.process_exception(stage, context, error).await
}

fn map_flow_to_task_outcome(flow: Flow, context: &EngineContext) -> TaskOutcome {
    match flow {
        Flow::Continue => unreachable!(),
        Flow::Drop(_) => TaskOutcome::Drop,
        Flow::Retry { reason, backoff } => {
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
            if let Some(delay) = backoff {
                request.meta.insert(
                    "_retry_backoff".to_string(),
                    crate::value::Value::Number(delay as f64),
                );
            }

            let task = match backoff {
                Some(delay) if delay > 0 => {
                    Task::with_id_and_delay(request, context.task_id.clone(), delay)
                }
                _ => Task::with_id(request, context.task_id.clone()),
            };
            TaskOutcome::Retry(Box::new(task))
        }
    }
}

pub(super) struct TaskRun {
    lease: TaskLease,
    url: String,
    outcome: TaskOutcome,
}

pub(super) enum TaskOutcome {
    Success(TaskOutput),
    Retry(Box<Task>),
    Drop,
    Error(SpiderError),
    LeaseLost(SpiderError),
}

pub(super) struct TaskOutput {
    pub(super) items: Vec<crate::item::Item>,
    pub(super) follows: Vec<Request>,
}
