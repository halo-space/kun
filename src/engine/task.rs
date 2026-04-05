use crate::download::traits::Downloader;
use crate::engine::context::EngineContext;
use crate::engine::flow::Flow;
use crate::error::SpiderError;
use crate::middleware::{Chain, Stage};
use crate::request::{Request, RequestMode};
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
    if !request.dont_filter && !super::is_domain_allowed(&request.url, allowed_domains) {
        tracing::debug!(
            url = request.url.as_str(),
            "request filtered out because domain is not in allowed_domains"
        );
        return Ok(false);
    }

    if !request.dont_filter && !dedup.check_and_insert(&request).await? {
        if let Some(stats) = stats {
            stats.record_dedup_reject();
        }
        tracing::debug!(
            url = request.url.as_str(),
            "request dropped by dedup component before scheduler"
        );
        return Ok(false);
    }

    scheduler.enqueue(Task::new(request)).await?;
    Ok(true)
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
            tracing::info!(
                spider = spider_name,
                round = *round,
                items = output.items.len(),
                follows = output.follows.len(),
                "completed parse round {}",
                round,
            );
            for follow in &output.follows {
                enqueue_request(
                    scheduler,
                    dedup,
                    follow.clone(),
                    allowed_domains,
                    Some(stats),
                )
                .await?;
            }
            resolve_scheduler_transition(
                scheduler.complete(&lease),
                &lease,
                "complete",
                url.as_str(),
                spider_name,
                stats,
            )
            .await?;
            outputs.push(SpiderOutput {
                items: output.items,
                requests: output.follows,
            });
        }
        TaskOutcome::Retry(retry_task) => {
            stats.record_retry();
            scheduler.enqueue(*retry_task).await?;
            resolve_scheduler_transition(
                scheduler.complete(&lease),
                &lease,
                "complete",
                url.as_str(),
                spider_name,
                stats,
            )
            .await?;
        }
        TaskOutcome::Drop => {
            resolve_scheduler_transition(
                scheduler.complete(&lease),
                &lease,
                "complete",
                url.as_str(),
                spider_name,
                stats,
            )
            .await?;
        }
        TaskOutcome::Error(error) => {
            stats.record_error();
            tracing::error!(spider = spider_name, url = url.as_str(), error = %error, "task failed");
            resolve_scheduler_transition(
                scheduler.requeue(&lease),
                &lease,
                "requeue",
                url.as_str(),
                spider_name,
                stats,
            )
            .await?;
        }
        TaskOutcome::LeaseLost(error) => {
            stats.record_error();
            tracing::warn!(
                spider = spider_name,
                task_id = lease.task_id().as_str(),
                worker_id = lease.worker_id(),
                url = url.as_str(),
                error = %error,
                "task lease was lost before completion"
            );
        }
    }
    Ok(())
}

async fn resolve_scheduler_transition(
    transition: impl std::future::Future<Output = Result<(), SpiderError>>,
    lease: &TaskLease,
    action: &'static str,
    url: &str,
    spider_name: &str,
    stats: &crate::stats::Tracker,
) -> Result<(), SpiderError> {
    match transition.await {
        Ok(()) => Ok(()),
        Err(error) if error.is_scheduler_lease_resolution_error() => {
            stats.record_error();
            tracing::warn!(
                spider = spider_name,
                task_id = lease.task_id().as_str(),
                worker_id = lease.worker_id(),
                action,
                url,
                error = %error,
                "task lease could not be resolved after task execution"
            );
            Ok(())
        }
        Err(error) => Err(error),
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
    async fn process_spider_output(self, mut output: SpiderOutput) -> TaskOutcome {
        let mut kept_items = Vec::with_capacity(output.items.len());
        for mut item in output.items.drain(..) {
            match self.pipeline.process(&mut item, self.spider_name).await {
                Ok(true) => {
                    kept_items.push(item);
                }
                Ok(false) => {
                    self.stats.record_pipeline_drop();
                    tracing::debug!(spider = self.spider_name, "pipeline dropped item");
                }
                Err(error) => {
                    tracing::warn!(
                        spider = self.spider_name,
                        error = %error,
                        "pipeline failed while processing item"
                    );
                    return TaskOutcome::Error(error);
                }
            }
        }

        if !kept_items.is_empty() {
            match self.store.batch_write(&kept_items, self.spider_name).await {
                Ok(()) => {
                    for _ in &kept_items {
                        self.stats.record_item();
                    }
                }
                Err(error) => {
                    self.stats.record_store_error();
                    tracing::warn!(
                        spider = self.spider_name,
                        error = %error,
                        "store failed while batch writing items"
                    );
                    return TaskOutcome::Error(error);
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
            return TaskOutcome::Error(error);
        };

        tracing::info!(
            spider = self.spider_name,
            url = context.request.url.as_str(),
            errback = errback.name.as_str(),
            "dispatching request errback"
        );

        let failure = Failure::new(context.request.clone(), context.response.clone(), error);

        match self.spider.handle_error(&errback.name, &failure).await {
            Ok(output) => self.process_spider_output(output).await,
            Err(errback_error) => TaskOutcome::Error(errback_error),
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
        let task_future = async move {
            let _global_permit_guard = global_permit_guard;
            let _domain_permit_guard = match domain_semaphore.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    return TaskOutcome::Error(SpiderError::engine("domain semaphore closed"));
                }
            };

            self.run(task_id, request).await
        };

        let outcome = run_task_with_heartbeat(scheduler, &lease, task_future).await;

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
            Err(error) => return TaskOutcome::Error(error),
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
                    tracing::info!(
                        spider = self.spider_name,
                        url = context.request.url.as_str(),
                        "request blocked by robots.txt"
                    );
                    return TaskOutcome::Drop;
                }
                Ok(crate::robots::Decision::Delay(delay)) => {
                    self.stats.record_robots_delay();
                    let backoff = u64::try_from(delay.as_millis()).unwrap_or_default().max(1);
                    tracing::debug!(
                        spider = self.spider_name,
                        url = context.request.url.as_str(),
                        backoff,
                        "request delayed by robots crawl-delay"
                    );
                    return map_flow_to_task_outcome(
                        Flow::Retry {
                            reason: "robots crawl delay".to_string(),
                            backoff: Some(backoff),
                        },
                        &context,
                    );
                }
                Err(error) => return TaskOutcome::Error(error),
            }
        }

        self.stats.record_request();
        let response = match context.request.mode {
            RequestMode::Http => self.http.fetch(&context.request).await,
            RequestMode::Browser => self.browser.fetch(&context.request).await,
        };

        let response = match response {
            Ok(response) => {
                self.stats.record_response();
                response
            }
            Err(error) => {
                tracing::warn!(url = context.request.url.as_str(), error = %error, "download failed");
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
                    Err(middleware_error) => return TaskOutcome::Error(middleware_error),
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
            Err(error) => return TaskOutcome::Error(error),
        }

        let response = context.response.clone().unwrap_or(response);

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
            Err(error) => return TaskOutcome::Error(error),
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
                    Err(error) => return TaskOutcome::Error(error),
                }

                self.process_spider_output(output).await
            }
            Err(error) => {
                tracing::error!(
                    spider = self.spider_name,
                    url = context.request.url.as_str(),
                    error = %error,
                    "spider callback failed"
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
                    Err(middleware_error) => TaskOutcome::Error(middleware_error),
                }
            }
        }
    }
}

async fn run_task_with_heartbeat<S, F>(
    scheduler: &S,
    lease: &TaskLease,
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
