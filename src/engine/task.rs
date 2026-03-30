use crate::download::traits::Downloader;
use crate::engine::context::EngineContext;
use crate::engine::types::Flow;
use crate::error::SpiderError;
use crate::middleware::{MiddlewareChain, MiddlewareType};
use crate::request::{Request, RequestMode};
use crate::rules::Compiled;
use crate::scheduler::traits::Scheduler;
use crate::scheduler::types::{ScheduledTask, TaskId};
use crate::spider::{Output as SpiderOutput, Spider};
use std::sync::Arc;

pub(super) async fn apply_task_execution_result<S: Scheduler>(
    result: TaskExecutionResult,
    scheduler: &mut S,
    allowed_domains: &[String],
    outputs: &mut Vec<SpiderOutput>,
    round: &mut usize,
    spider_name: &str,
) -> Result<(), SpiderError> {
    let task_id = result.task_id;
    let url = result.url;
    match result.decision {
        TaskDecision::Success(output) => {
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
                if follow.dont_filter || super::is_domain_allowed(&follow.url, allowed_domains) {
                    scheduler
                        .enqueue(ScheduledTask::new(follow.clone()))
                        .await?;
                }
            }
            scheduler.ack(&task_id).await?;
            outputs.push(SpiderOutput {
                items: output.items,
                requests: output.follows,
            });
        }
        TaskDecision::Retry(retry_task) => {
            scheduler.enqueue(*retry_task).await?;
            scheduler.ack(&task_id).await?;
        }
        TaskDecision::Drop => {
            scheduler.ack(&task_id).await?;
        }
        TaskDecision::Error(error) => {
            tracing::error!(spider = spider_name, url = url.as_str(), error = %error, "任务出错");
            scheduler.nack(&task_id).await?;
        }
    }
    Ok(())
}

pub(super) struct TaskExecutionLease {
    task_id: TaskId,
    request: Request,
    _global_permit_guard: tokio::sync::OwnedSemaphorePermit,
    domain_semaphore: Arc<tokio::sync::Semaphore>,
}

impl TaskExecutionLease {
    pub(super) fn new(
        task_id: TaskId,
        request: Request,
        global_permit_guard: tokio::sync::OwnedSemaphorePermit,
        domain_semaphore: Arc<tokio::sync::Semaphore>,
    ) -> Self {
        Self {
            task_id,
            request,
            _global_permit_guard: global_permit_guard,
            domain_semaphore,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct TaskExecutor<'a, H, B, P, Sp> {
    pub(super) http: &'a H,
    pub(super) browser: &'a B,
    pub(super) pipeline: &'a P,
    pub(super) engine_middleware: &'a MiddlewareChain,
    pub(super) step_middleware: &'a MiddlewareChain,
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
    pub(super) async fn execute_with_permits(
        self,
        lease: TaskExecutionLease,
    ) -> TaskExecutionResult {
        let TaskExecutionLease {
            task_id,
            request,
            _global_permit_guard,
            domain_semaphore,
        } = lease;

        let url = request.url.clone();
        let _domain_permit_guard = match domain_semaphore.acquire().await {
            Ok(permit) => permit,
            Err(_) => {
                return TaskExecutionResult {
                    task_id,
                    url,
                    decision: TaskDecision::Error(SpiderError::engine("domain semaphore closed")),
                };
            }
        };

        let decision = self.decide_execution(task_id.clone(), request).await;

        TaskExecutionResult {
            task_id,
            url,
            decision,
        }
    }

    pub(super) async fn decide_execution(self, task_id: TaskId, request: Request) -> TaskDecision {
        let mut context = EngineContext::new(request).with_task_id(task_id);

        match run_middleware_request(
            self.engine_middleware,
            self.step_middleware,
            MiddlewareType::Download,
            &mut context,
        )
        .await
        {
            Ok(Flow::Continue) => {}
            Ok(flow) => return map_flow_to_task_decision(flow, &context),
            Err(error) => return TaskDecision::Error(error),
        }

        let response = match context.request.mode {
            RequestMode::Http => self.http.fetch(&context.request).await,
            RequestMode::Browser => self.browser.fetch(&context.request).await,
        };

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(url = context.request.url.as_str(), error = %error, "下载失败");
                match run_middleware_exception(
                    self.engine_middleware,
                    self.step_middleware,
                    MiddlewareType::Download,
                    &mut context,
                    &error,
                )
                .await
                {
                    Ok(Flow::Continue) => return TaskDecision::Error(error),
                    Ok(flow) => return map_flow_to_task_decision(flow, &context),
                    Err(middleware_error) => return TaskDecision::Error(middleware_error),
                }
            }
        };

        context.response = Some(response.clone());

        match run_middleware_response(
            self.engine_middleware,
            self.step_middleware,
            MiddlewareType::Download,
            &mut context,
        )
        .await
        {
            Ok(Flow::Continue) => {}
            Ok(flow) => return map_flow_to_task_decision(flow, &context),
            Err(error) => return TaskDecision::Error(error),
        }

        match run_middleware_request(
            self.engine_middleware,
            self.step_middleware,
            MiddlewareType::Spider,
            &mut context,
        )
        .await
        {
            Ok(Flow::Continue) => {}
            Ok(flow) => return map_flow_to_task_decision(flow, &context),
            Err(error) => return TaskDecision::Error(error),
        }

        let output = self.spider.dispatch(&response, self.compiled).await;

        match output {
            Ok(mut output) => {
                match run_middleware_response(
                    self.engine_middleware,
                    self.step_middleware,
                    MiddlewareType::Spider,
                    &mut context,
                )
                .await
                {
                    Ok(Flow::Continue) => {}
                    Ok(flow) => return map_flow_to_task_decision(flow, &context),
                    Err(error) => return TaskDecision::Error(error),
                }

                let mut kept_items = Vec::with_capacity(output.items.len());
                for mut item in output.items.drain(..) {
                    match self.pipeline.process(&mut item, self.spider_name).await {
                        Ok(true) => kept_items.push(item),
                        Ok(false) => {
                            tracing::debug!(spider = self.spider_name, "pipeline 丢弃 item");
                        }
                        Err(error) => {
                            tracing::warn!(
                                spider = self.spider_name,
                                error = %error,
                                "pipeline 处理 item 出错"
                            );
                            return TaskDecision::Error(error);
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
                            "域名不在 allowed_domains 内，已过滤"
                        );
                    }
                }

                TaskDecision::Success(TaskSuccess {
                    items: kept_items,
                    follows,
                })
            }
            Err(error) => {
                tracing::error!(
                    spider = self.spider_name,
                    url = context.request.url.as_str(),
                    error = %error,
                    "解析回调执行失败"
                );
                match run_middleware_exception(
                    self.engine_middleware,
                    self.step_middleware,
                    MiddlewareType::Spider,
                    &mut context,
                    &error,
                )
                .await
                {
                    Ok(Flow::Continue) => TaskDecision::Error(error),
                    Ok(flow) => map_flow_to_task_decision(flow, &context),
                    Err(middleware_error) => TaskDecision::Error(middleware_error),
                }
            }
        }
    }
}

pub(super) async fn run_middleware_request(
    engine_middleware: &MiddlewareChain,
    step_middleware: &MiddlewareChain,
    kind: MiddlewareType,
    context: &mut EngineContext,
) -> Result<Flow, SpiderError> {
    let flow = engine_middleware.process_request(kind, context).await?;
    if !matches!(flow, Flow::Continue) {
        return Ok(flow);
    }
    step_middleware.process_request(kind, context).await
}

pub(super) async fn run_middleware_response(
    engine_middleware: &MiddlewareChain,
    step_middleware: &MiddlewareChain,
    kind: MiddlewareType,
    context: &mut EngineContext,
) -> Result<Flow, SpiderError> {
    let flow = engine_middleware.process_response(kind, context).await?;
    if !matches!(flow, Flow::Continue) {
        return Ok(flow);
    }
    step_middleware.process_response(kind, context).await
}

async fn run_middleware_exception(
    engine_middleware: &MiddlewareChain,
    step_middleware: &MiddlewareChain,
    kind: MiddlewareType,
    context: &mut EngineContext,
    error: &SpiderError,
) -> Result<Flow, SpiderError> {
    let flow = engine_middleware
        .process_exception(kind, context, error)
        .await?;
    if !matches!(flow, Flow::Continue) {
        return Ok(flow);
    }
    step_middleware
        .process_exception(kind, context, error)
        .await
}

fn map_flow_to_task_decision(flow: Flow, context: &EngineContext) -> TaskDecision {
    match flow {
        Flow::Continue => unreachable!(),
        Flow::Drop(_) => TaskDecision::Drop,
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
                    ScheduledTask::with_task_id_and_delay(request, context.task_id.clone(), ms)
                }
                _ => ScheduledTask::with_task_id(request, context.task_id.clone()),
            };
            TaskDecision::Retry(Box::new(task))
        }
    }
}

pub(super) struct TaskExecutionResult {
    task_id: TaskId,
    url: String,
    decision: TaskDecision,
}

pub(super) enum TaskDecision {
    Success(TaskSuccess),
    Retry(Box<ScheduledTask>),
    Drop,
    Error(SpiderError),
}

pub(super) struct TaskSuccess {
    pub(super) items: Vec<crate::item::Item>,
    pub(super) follows: Vec<Request>,
}
