use crate::download::traits::Downloader;
use crate::engine::step::StepExecute;
use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::Chain;
use crate::middleware::DEDUP;
use crate::request::{Request, RequestMode};
use crate::response::Response;
use crate::rules::Compiled;
use crate::scheduler::{Scheduler, Task, TaskId, TaskLease};
use crate::spider::{CallbackOutput, Failure, IntoSpiderResultParts, Spider};
use futures::future::try_join_all;
use std::collections::BTreeMap;
use std::sync::Arc;

pub(super) async fn enqueue_request_with_middleware<S>(
    scheduler: &S,
    engine_middleware: &Chain,
    step_middleware: &Chain,
    request: Request,
    allowed_domains: &[String],
    spider_name: Option<&str>,
    stats: Option<&crate::stats::Tracker>,
) -> Result<bool, SpiderError>
where
    S: Scheduler,
{
    enqueue_task_with_middleware(
        scheduler,
        engine_middleware,
        step_middleware,
        Task::new(request),
        allowed_domains,
        spider_name,
        stats,
    )
    .await
}

pub(super) async fn enqueue_task_with_middleware<S>(
    scheduler: &S,
    engine_middleware: &Chain,
    step_middleware: &Chain,
    task: Task,
    allowed_domains: &[String],
    spider_name: Option<&str>,
    stats: Option<&crate::stats::Tracker>,
) -> Result<bool, SpiderError>
where
    S: Scheduler,
{
    let Some(task) = prepare_task_for_enqueue_with_middleware(
        engine_middleware,
        step_middleware,
        task,
        allowed_domains,
        spider_name,
        stats,
    )
    .await?
    else {
        return Ok(false);
    };

    let request = task.request.clone();
    let task_id = task.id.clone();
    scheduler.enqueue(task).await?;
    run_middleware_after_enqueue(
        engine_middleware,
        step_middleware,
        request,
        task_id,
        spider_name,
    )
    .await?;
    Ok(true)
}

pub(super) async fn prepare_task_for_enqueue_with_middleware(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    mut task: Task,
    allowed_domains: &[String],
    spider_name: Option<&str>,
    stats: Option<&crate::stats::Tracker>,
) -> Result<Option<Task>, SpiderError> {
    let mut context = context::Enqueue::new(task.request.clone()).with_task_id(task.id.clone());
    if let Some(spider_name) = spider_name {
        context = context.with_spider_name(spider_name);
    }

    let next =
        run_middleware_before_enqueue(engine_middleware, step_middleware, &mut context).await?;
    if !resolve_enqueue_admission_flow(next, stats)? {
        return Ok(None);
    }

    task.request = context.request;
    let request = &task.request;

    if !request.skips_domain_filter() && !super::is_domain_allowed(&request.url, allowed_domains) {
        return Ok(None);
    }

    Ok(Some(task))
}

fn resolve_enqueue_admission_flow(
    next: flow::Enqueue,
    stats: Option<&crate::stats::Tracker>,
) -> Result<bool, SpiderError> {
    match next {
        flow::Enqueue::Continue => Ok(true),
        flow::Enqueue::Drop { reason } => {
            if reason == DEDUP {
                if let Some(stats) = stats {
                    stats.record_dedup_reject();
                }
            }
            Ok(false)
        }
    }
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

pub(super) async fn apply_task_run<S>(
    run: TaskRun,
    scheduler: &S,
    engine_middleware: &Chain,
    step_executes: &BTreeMap<String, StepExecute>,
    allowed_domains: &[String],
    round: &mut usize,
    spider_name: &str,
    stats: &crate::stats::Tracker,
    signals: &crate::signals::Bus,
) -> Result<(), SpiderError>
where
    S: Scheduler,
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
                let step_middleware = &super::step_execute_for_request(step_executes, follow).chain;
                let task = prepare_task_for_enqueue_with_middleware(
                    engine_middleware,
                    step_middleware,
                    Task::new(follow.clone()),
                    allowed_domains,
                    Some(spider_name),
                    Some(stats),
                )
                .await?;

                if let Some(task) = task {
                    scheduled_follows.push((follow.clone(), task.id.clone()));
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
                for (follow, task_id) in &scheduled_follows {
                    let step_middleware =
                        &super::step_execute_for_request(step_executes, follow).chain;
                    run_middleware_after_enqueue(
                        engine_middleware,
                        step_middleware,
                        follow.clone(),
                        task_id.clone(),
                        Some(spider_name),
                    )
                    .await?;
                    signals
                        .emit(crate::signals::Signal::request_scheduled(
                            spider_name,
                            follow.clone(),
                        ))
                        .await;
                }
            }
        }
        TaskOutcome::Retry(retry_task) => {
            stats.record_retry();
            let retry_task = *retry_task;
            let retry_task_id = retry_task.id.clone();
            let request = retry_task.request.clone();
            let step_middleware = &super::step_execute_for_request(step_executes, &request).chain;
            let retry_tasks = prepare_task_for_enqueue_with_middleware(
                engine_middleware,
                step_middleware,
                retry_task,
                allowed_domains,
                Some(spider_name),
                Some(stats),
            )
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
                run_middleware_after_enqueue(
                    engine_middleware,
                    step_middleware,
                    request.clone(),
                    retry_task_id,
                    Some(spider_name),
                )
                .await?;
                signals
                    .emit(crate::signals::Signal::request_scheduled(
                        spider_name,
                        request,
                    ))
                    .await;
            }
        }
        TaskOutcome::Delay(delayed_task) => {
            let delayed_task = *delayed_task;
            let delayed_task_id = delayed_task.id.clone();
            let request = delayed_task.request.clone();
            let step_middleware = &super::step_execute_for_request(step_executes, &request).chain;
            let delayed_tasks = prepare_task_for_enqueue_with_middleware(
                engine_middleware,
                step_middleware,
                delayed_task,
                allowed_domains,
                Some(spider_name),
                Some(stats),
            )
            .await?
            .into_iter()
            .collect::<Vec<_>>();
            let queued_delayed_tasks = delayed_tasks.len();
            let committed = resolve_scheduler_transition(
                scheduler.complete_and_enqueue(&lease, delayed_tasks),
                &lease,
                crate::signals::SchedulerEventKind::Completed,
                url.as_str(),
                spider_name,
                stats,
                signals,
                "complete_and_enqueue",
                false,
                queued_delayed_tasks,
            )
            .await?;
            if committed {
                run_middleware_after_enqueue(
                    engine_middleware,
                    step_middleware,
                    request.clone(),
                    delayed_task_id,
                    Some(spider_name),
                )
                .await?;
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
pub(super) struct TaskExecutor<'a, S, H, B, P, Sp> {
    pub(super) scheduler: &'a S,
    pub(super) http: &'a H,
    pub(super) browser: &'a B,
    pub(super) pipeline: &'a P,
    pub(super) robots: &'a dyn crate::robots::Robot,
    pub(super) config: &'a crate::settings::Config,
    pub(super) stats: Arc<crate::stats::Tracker>,
    pub(super) signals: Arc<crate::signals::Bus>,
    pub(super) engine_middleware: &'a Chain,
    pub(super) step_execute: &'a StepExecute,
    pub(super) spider: &'a Sp,
    pub(super) compiled: Option<&'a Compiled>,
    pub(super) allowed_domains: &'a [String],
    pub(super) spider_name: &'a str,
}

impl<'a, S, H, B, P, Sp> TaskExecutor<'a, S, H, B, P, Sp>
where
    S: Scheduler,
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
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

    async fn process_spider_output(
        self,
        mut output: CallbackOutput,
        task_id: &TaskId,
        request: &Request,
        response: Option<Response>,
    ) -> TaskOutcome {
        let mut kept_items = Vec::with_capacity(output.items.len());
        for item in output.items.drain(..) {
            let mut context = context::Item::new(request.clone(), item)
                .with_task_id(task_id.clone())
                .with_spider_name(self.spider_name);

            if let Some(response) = response.clone() {
                context = context.with_response(response);
            }

            match run_middleware_before_item(
                self.engine_middleware,
                &self.step_execute.chain,
                &mut context,
            )
            .await
            {
                Ok(flow::Item::Continue) => {}
                Ok(flow::Item::Drop { .. }) => continue,
                Err(error) => return self.error_outcome(request, response.clone(), error).await,
            }

            match self
                .pipeline
                .process(&mut context.item, self.spider_name)
                .await
            {
                Ok(true) => {
                    if let Err(error) = self
                        .step_execute
                        .step_validator
                        .validate(&context.item)
                        .await
                    {
                        crate::trace::warn(
                            "validator.drop",
                            vec![
                                crate::trace::prop("stage", "validator"),
                                crate::trace::prop("event", "drop"),
                                crate::trace::prop("reason", "validation_failed"),
                                crate::trace::prop("spider", self.spider_name),
                                crate::trace::prop("url", request.url.as_str()),
                                crate::trace::prop("task_id", task_id.as_str()),
                                crate::trace::prop("error", &error),
                            ],
                        );
                        continue;
                    }

                    if let Err(error) = run_middleware_after_item(
                        self.engine_middleware,
                        &self.step_execute.chain,
                        &mut context,
                    )
                    .await
                    {
                        return self.error_outcome(request, response.clone(), error).await;
                    }

                    kept_items.push(context.item);
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
            match dispatch_items_to_stores(&self.step_execute.stores, &kept_items, self.spider_name)
                .await
            {
                Ok(()) => {
                    crate::trace::info(
                        "engine.commit.store.ok",
                        vec![
                            crate::trace::prop("spider", self.spider_name),
                            crate::trace::prop("url", request.url.as_str()),
                            crate::trace::prop("items", kept_items.len()),
                            crate::trace::prop("stores", self.step_execute.stores.len()),
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
                            crate::trace::prop("stores", self.step_execute.stores.len()),
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
            if super::is_domain_allowed(&request.url, self.allowed_domains) {
                follows.push(request);
            }
        }

        TaskOutcome::Success(TaskOutput {
            items: kept_items,
            follows,
        })
    }

    async fn run_errback(
        self,
        task_id: &TaskId,
        request: &Request,
        response: Option<Response>,
        error: SpiderError,
    ) -> TaskOutcome {
        let Some(errback) = request.errback.as_ref() else {
            return self.error_outcome(request, response, error).await;
        };

        let failure = Failure::new(request.clone(), response.clone(), error);

        match self.spider.handle_error(&errback.name, &failure).await {
            Ok(output) => {
                self.process_spider_output(
                    CallbackOutput::from_parts(output.into_parts()),
                    task_id,
                    request,
                    response,
                )
                .await
            }
            Err(errback_error) => self.error_outcome(request, response, errback_error).await,
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
        let attempt = request_attempt(&request);
        let mut download = context::Download::new(request)
            .with_task_id(task_id)
            .with_spider_name(self.spider_name)
            .with_attempt(attempt)
            .with_stats(self.stats.clone());

        match run_middleware_before_download(
            self.engine_middleware,
            &self.step_execute.chain,
            &mut download,
        )
        .await
        {
            Ok(flow::Download::Continue) => {}
            Ok(next) => return map_download_to_task_outcome(next, &download),
            Err(error) => return self.error_outcome(&download.request, None, error).await,
        }

        if self.config.robots.obey {
            let user_agent = self.config.robots.resolved_user_agent(self.spider_name);
            match self
                .robots
                .check(&download.request, user_agent.as_str())
                .await
            {
                Ok(crate::robots::Decision::Allow) => {}
                Ok(crate::robots::Decision::Disallow) => {
                    self.stats.record_robots_disallow();
                    crate::trace::warn(
                        "robots.blocked",
                        vec![
                            crate::trace::prop("spider", self.spider_name),
                            crate::trace::prop("url", download.request.url.as_str()),
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
                            crate::trace::prop("url", download.request.url.as_str()),
                            crate::trace::prop("backoff", backoff),
                        ],
                    );
                    return map_download_to_task_outcome(
                        flow::Download::Delay {
                            reason: "robots crawl delay".to_string(),
                            millis: backoff,
                        },
                        &download,
                    );
                }
                Err(error) => return self.error_outcome(&download.request, None, error).await,
            }
        }

        if download.request_started_at.is_none() {
            download.request_started_at = Some(now_millis());
        }

        self.stats.record_request();
        crate::trace::info(
            "request.start",
            vec![
                crate::trace::prop("spider", self.spider_name),
                crate::trace::prop("url", download.request.url.as_str()),
                crate::trace::prop("method", download.request.method.as_str()),
                crate::trace::prop(
                    "mode",
                    match download.request.mode {
                        RequestMode::Http => "http",
                        RequestMode::Browser => "browser",
                    },
                ),
            ],
        );
        let response = match download.request.mode {
            RequestMode::Http => self.http.fetch(&download.request).await,
            RequestMode::Browser => self.browser.fetch(&download.request).await,
        };

        let mut response = match response {
            Ok(response) => {
                self.stats.record_response();
                crate::trace::info(
                    "request.ok",
                    vec![
                        crate::trace::prop("spider", self.spider_name),
                        crate::trace::prop("url", download.request.url.as_str()),
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
                        crate::trace::prop("url", download.request.url.as_str()),
                        crate::trace::prop("error", &error),
                    ],
                );
                match run_middleware_download_error(
                    self.engine_middleware,
                    &self.step_execute.chain,
                    &mut download,
                    &error,
                )
                .await
                {
                    Ok(flow::Download::Continue) => {
                        return self
                            .run_errback(&download.task_id, &download.request, None, error)
                            .await;
                    }
                    Ok(next) => return map_download_to_task_outcome(next, &download),
                    Err(middleware_error) => {
                        return self
                            .error_outcome(&download.request, None, middleware_error)
                            .await;
                    }
                }
            }
        };

        match run_middleware_after_download(
            self.engine_middleware,
            &self.step_execute.chain,
            &mut download,
            &mut response,
        )
        .await
        {
            Ok(flow::Download::Continue) => {}
            Ok(next) => return map_download_to_task_outcome(next, &download),
            Err(error) => {
                return self
                    .error_outcome(&download.request, Some(response.clone()), error)
                    .await;
            }
        }

        self.signals
            .emit(crate::signals::Signal::response_received(
                self.spider_name,
                response.clone(),
            ))
            .await;

        let mut parse = context::Parse::new(download.request.clone(), response)
            .with_task_id(download.task_id.clone())
            .with_spider_name(self.spider_name);

        match run_middleware_before_parse(
            self.engine_middleware,
            &self.step_execute.chain,
            &mut parse,
        )
        .await
        {
            Ok(flow::Parse::Continue) => {}
            Ok(next) => return map_parse_to_task_outcome(next),
            Err(error) => {
                return self
                    .error_outcome(&parse.request, Some(parse.response.clone()), error)
                    .await;
            }
        }

        let output = self
            .spider
            .dispatch(&parse.response, self.compiled)
            .await
            .map(|output| CallbackOutput::from_parts(output.into_parts()));

        match output {
            Ok(output) => {
                if let Err(error) = run_middleware_after_parse(
                    self.engine_middleware,
                    &self.step_execute.chain,
                    &mut parse,
                )
                .await
                {
                    return self
                        .error_outcome(&parse.request, Some(parse.response.clone()), error)
                        .await;
                }

                self.process_spider_output(
                    output,
                    &parse.task_id,
                    &parse.request,
                    Some(parse.response.clone()),
                )
                .await
            }
            Err(error) => {
                crate::trace::error(
                    "spider.fail",
                    vec![
                        crate::trace::prop("spider", self.spider_name),
                        crate::trace::prop("url", parse.request.url.as_str()),
                        crate::trace::prop("error", &error),
                    ],
                );
                match run_middleware_parse_error(
                    self.engine_middleware,
                    &self.step_execute.chain,
                    &mut parse,
                    &error,
                )
                .await
                {
                    Ok(flow::Parse::Continue) => {
                        self.run_errback(
                            &parse.task_id,
                            &parse.request,
                            Some(parse.response.clone()),
                            error,
                        )
                        .await
                    }
                    Ok(next) => map_parse_to_task_outcome(next),
                    Err(middleware_error) => {
                        self.error_outcome(
                            &parse.request,
                            Some(parse.response.clone()),
                            middleware_error,
                        )
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

pub(super) async fn run_middleware_before_enqueue(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    context: &mut context::Enqueue,
) -> Result<flow::Enqueue, SpiderError> {
    let next = engine_middleware.before_enqueue(context).await?;
    if !matches!(next, flow::Enqueue::Continue) {
        return Ok(next);
    }
    step_middleware.before_enqueue(context).await
}

pub(super) async fn run_middleware_after_enqueue(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    request: Request,
    task_id: TaskId,
    spider_name: Option<&str>,
) -> Result<(), SpiderError> {
    let mut context = context::Enqueue::new(request).with_task_id(task_id);
    if let Some(spider_name) = spider_name {
        context = context.with_spider_name(spider_name);
    }

    engine_middleware.after_enqueue(&mut context).await?;
    step_middleware.after_enqueue(&mut context).await
}

pub(super) async fn run_middleware_before_download(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    context: &mut context::Download,
) -> Result<flow::Download, SpiderError> {
    let next = engine_middleware.before_download(context).await?;
    if !matches!(next, flow::Download::Continue) {
        return Ok(next);
    }
    step_middleware.before_download(context).await
}

pub(super) async fn run_middleware_after_download(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    context: &mut context::Download,
    response: &mut Response,
) -> Result<flow::Download, SpiderError> {
    let next = engine_middleware.after_download(context, response).await?;
    if !matches!(next, flow::Download::Continue) {
        return Ok(next);
    }
    step_middleware.after_download(context, response).await
}

pub(super) async fn run_middleware_download_error(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    context: &mut context::Download,
    error: &SpiderError,
) -> Result<flow::Download, SpiderError> {
    let next = engine_middleware.download_error(context, error).await?;
    if !matches!(next, flow::Download::Continue) {
        return Ok(next);
    }
    step_middleware.download_error(context, error).await
}

pub(super) async fn run_middleware_before_parse(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    context: &mut context::Parse,
) -> Result<flow::Parse, SpiderError> {
    let next = engine_middleware.before_parse(context).await?;
    if !matches!(next, flow::Parse::Continue) {
        return Ok(next);
    }
    step_middleware.before_parse(context).await
}

pub(super) async fn run_middleware_parse_error(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    context: &mut context::Parse,
    error: &SpiderError,
) -> Result<flow::Parse, SpiderError> {
    let next = engine_middleware.parse_error(context, error).await?;
    if !matches!(next, flow::Parse::Continue) {
        return Ok(next);
    }
    step_middleware.parse_error(context, error).await
}

pub(super) async fn run_middleware_after_parse(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    context: &mut context::Parse,
) -> Result<(), SpiderError> {
    engine_middleware.after_parse(context).await?;
    step_middleware.after_parse(context).await
}

pub(super) async fn run_middleware_before_item(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    context: &mut context::Item,
) -> Result<flow::Item, SpiderError> {
    let next = engine_middleware.before_item(context).await?;
    if !matches!(next, flow::Item::Continue) {
        return Ok(next);
    }
    step_middleware.before_item(context).await
}

pub(super) async fn run_middleware_after_item(
    engine_middleware: &Chain,
    step_middleware: &Chain,
    context: &mut context::Item,
) -> Result<(), SpiderError> {
    engine_middleware.after_item(context).await?;
    step_middleware.after_item(context).await
}

fn map_download_to_task_outcome(next: flow::Download, context: &context::Download) -> TaskOutcome {
    match next {
        flow::Download::Continue => unreachable!(),
        flow::Download::Drop { .. } => TaskOutcome::Drop,
        flow::Download::Delay { millis, .. } => {
            let task = Task::with_id_and_delay(
                context.request.clone().skip([DEDUP]),
                context.task_id.clone(),
                millis,
            );
            TaskOutcome::Delay(Box::new(task))
        }
        flow::Download::Retry { reason, backoff } => {
            let retries = context
                .request
                .meta
                .get("_retry_times")
                .and_then(crate::value::Value::as_f64)
                .unwrap_or(0.0)
                + 1.0;

            let mut request = context.request.clone().skip([DEDUP]);
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

fn map_parse_to_task_outcome(next: flow::Parse) -> TaskOutcome {
    match next {
        flow::Parse::Continue => unreachable!(),
        flow::Parse::Drop { .. } => TaskOutcome::Drop,
    }
}

fn request_attempt(request: &Request) -> u32 {
    request
        .meta
        .get("_retry_times")
        .and_then(crate::value::Value::as_f64)
        .unwrap_or(0.0)
        .max(0.0) as u32
        + 1
}

async fn dispatch_items_to_stores(
    stores: &[crate::store::SharedStore],
    items: &[crate::item::Item],
    spider_name: &str,
) -> Result<(), SpiderError> {
    if stores.is_empty() {
        return Err(SpiderError::engine(
            "step execute resolved no stores for item dispatch",
        ));
    }

    let futures = stores
        .iter()
        .cloned()
        .map(|store| async move { store.batch_write(items, spider_name).await });
    try_join_all(futures).await?;
    Ok(())
}

fn now_millis() -> u64 {
    u64::try_from(jiff::Timestamp::now().as_millisecond()).unwrap_or_default()
}

pub(super) struct TaskRun {
    lease: TaskLease,
    url: String,
    outcome: TaskOutcome,
}

pub(super) enum TaskOutcome {
    Success(TaskOutput),
    Delay(Box<Task>),
    Retry(Box<Task>),
    Drop,
    Error(SpiderError),
    LeaseLost(SpiderError),
}

#[derive(Debug)]
pub(super) struct TaskOutput {
    pub(super) items: Vec<crate::item::Item>,
    pub(super) follows: Vec<Request>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_outcome_keeps_task_identity_without_retry_metadata() {
        let context = context::Download::new(Request::new("https://example.com/delay"));
        let task_id = context.task_id.clone();

        let outcome = map_download_to_task_outcome(
            flow::Download::Delay {
                reason: "interval".to_string(),
                millis: 50,
            },
            &context,
        );

        let TaskOutcome::Delay(task) = outcome else {
            panic!("expected delay outcome");
        };

        assert_eq!(task.id, task_id);
        assert!(task.ready_at.is_some());
        assert!(!task.request.meta.contains_key("_retry_times"));
        assert!(!task.request.meta.contains_key("_retry_reason"));
        assert!(!task.request.meta.contains_key("_retry_backoff"));
        assert!(task.request.middleware_skips(DEDUP));
    }

    #[test]
    fn retry_outcome_increments_retry_metadata() {
        let context = context::Download::new(Request::new("https://example.com/retry"));
        let task_id = context.task_id.clone();

        let outcome = map_download_to_task_outcome(
            flow::Download::Retry {
                reason: "retry_by_status".to_string(),
                backoff: Some(100),
            },
            &context,
        );

        let TaskOutcome::Retry(task) = outcome else {
            panic!("expected retry outcome");
        };

        assert_eq!(task.id, task_id);
        assert_eq!(
            task.request.meta.get("_retry_times"),
            Some(&crate::value::Value::Number(1.0))
        );
        assert_eq!(
            task.request.meta.get("_retry_reason"),
            Some(&crate::value::Value::String("retry_by_status".to_string()))
        );
        assert_eq!(
            task.request.meta.get("_retry_backoff"),
            Some(&crate::value::Value::Number(100.0))
        );
        assert!(task.request.middleware_skips(DEDUP));
    }
}
