use crate::download::traits::Downloader;
use crate::engine::context::EngineContext;
use crate::engine::flow::Flow;
use crate::error::SpiderError;
use crate::middleware::{Chain, Stage};
use crate::request::{Request, RequestMode};
use crate::rules::Compiled;
use crate::scheduler::{Scheduler, Task, TaskId};
use crate::spider::{Output as SpiderOutput, Spider};
use std::sync::Arc;

pub(super) async fn apply_task_run<S: Scheduler>(
    run: TaskRun,
    scheduler: &mut S,
    allowed_domains: &[String],
    outputs: &mut Vec<SpiderOutput>,
    round: &mut usize,
    spider_name: &str,
) -> Result<(), SpiderError> {
    let task_id = run.task_id;
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
                if follow.dont_filter || super::is_domain_allowed(&follow.url, allowed_domains) {
                    scheduler.enqueue(Task::new(follow.clone())).await?;
                }
            }
            scheduler.complete(&task_id).await?;
            outputs.push(SpiderOutput {
                items: output.items,
                requests: output.follows,
            });
        }
        TaskOutcome::Retry(retry_task) => {
            scheduler.enqueue(*retry_task).await?;
            scheduler.complete(&task_id).await?;
        }
        TaskOutcome::Drop => {
            scheduler.complete(&task_id).await?;
        }
        TaskOutcome::Error(error) => {
            tracing::error!(spider = spider_name, url = url.as_str(), error = %error, "task failed");
            scheduler.requeue(&task_id).await?;
        }
    }
    Ok(())
}

pub(super) struct TaskRunReservation {
    task_id: TaskId,
    request: Request,
    global_permit_guard: tokio::sync::OwnedSemaphorePermit,
    domain_semaphore: Arc<tokio::sync::Semaphore>,
}

impl TaskRunReservation {
    pub(super) fn new(
        task_id: TaskId,
        request: Request,
        global_permit_guard: tokio::sync::OwnedSemaphorePermit,
        domain_semaphore: Arc<tokio::sync::Semaphore>,
    ) -> Self {
        Self {
            task_id,
            request,
            global_permit_guard,
            domain_semaphore,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct TaskExecutor<'a, H, B, P, Sp> {
    pub(super) http: &'a H,
    pub(super) browser: &'a B,
    pub(super) pipeline: &'a P,
    pub(super) engine_chain: &'a Chain,
    pub(super) step_chain: &'a Chain,
    pub(super) spider: &'a Sp,
    pub(super) compiled: Option<&'a Compiled>,
    pub(super) allowed_domains: &'a [String],
    pub(super) spider_name: &'a str,
}

impl<'a, H, B, P, Sp> TaskExecutor<'a, H, B, P, Sp>
where
    H: Downloader,
    B: Downloader,
    P: crate::pipeline::Pipeline,
    Sp: Spider,
{
    pub(super) async fn run_with_reservation(self, reservation: TaskRunReservation) -> TaskRun {
        let TaskRunReservation {
            task_id,
            request,
            global_permit_guard: _global_permit_guard,
            domain_semaphore,
        } = reservation;

        let url = request.url.clone();
        let _domain_permit_guard = match domain_semaphore.acquire().await {
            Ok(permit) => permit,
            Err(_) => {
                return TaskRun {
                    task_id,
                    url,
                    outcome: TaskOutcome::Error(SpiderError::engine("domain semaphore closed")),
                };
            }
        };

        let outcome = self.run(task_id.clone(), request).await;

        TaskRun {
            task_id,
            url,
            outcome,
        }
    }

    pub(super) async fn run(self, task_id: TaskId, request: Request) -> TaskOutcome {
        let mut context = EngineContext::new(request).with_task_id(task_id);

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

        let response = match context.request.mode {
            RequestMode::Http => self.http.fetch(&context.request).await,
            RequestMode::Browser => self.browser.fetch(&context.request).await,
        };

        let response = match response {
            Ok(response) => response,
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
                    Ok(Flow::Continue) => return TaskOutcome::Error(error),
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
            Ok(mut output) => {
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

                let mut kept_items = Vec::with_capacity(output.items.len());
                for mut item in output.items.drain(..) {
                    match self.pipeline.process(&mut item, self.spider_name).await {
                        Ok(true) => kept_items.push(item),
                        Ok(false) => {
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

                let mut follows = Vec::new();
                for request in output.requests {
                    if request.dont_filter
                        || super::is_domain_allowed(&request.url, self.allowed_domains)
                    {
                        follows.push(request);
                    } else {
                        tracing::debug!(
                            url = request.url.as_str(),
                            "request filtered out because domain is not in allowed_domains"
                        );
                    }
                }

                TaskOutcome::Success(TaskOutput {
                    items: kept_items,
                    follows,
                })
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
                    Ok(Flow::Continue) => TaskOutcome::Error(error),
                    Ok(flow) => map_flow_to_task_outcome(flow, &context),
                    Err(middleware_error) => TaskOutcome::Error(middleware_error),
                }
            }
        }
    }
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
                Some(ms) if ms > 0 => Task::with_id_and_delay(request, context.task_id.clone(), ms),
                _ => Task::with_id(request, context.task_id.clone()),
            };
            TaskOutcome::Retry(Box::new(task))
        }
    }
}

pub(super) struct TaskRun {
    task_id: TaskId,
    url: String,
    outcome: TaskOutcome,
}

pub(super) enum TaskOutcome {
    Success(TaskOutput),
    Retry(Box<Task>),
    Drop,
    Error(SpiderError),
}

pub(super) struct TaskOutput {
    pub(super) items: Vec<crate::item::Item>,
    pub(super) follows: Vec<Request>,
}
