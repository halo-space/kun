use crate::error::SpiderError;
use crate::redis::{Connection, Endpoint, ErrorContext};
use crate::scheduler::checkpoint::{Checkpoint, Counts};
use crate::scheduler::{Scheduler, Task, TaskId};
use jiff::Timestamp;
use std::cmp::Ordering;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct Redis {
    url: String,
    namespace: String,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl Redis {
    pub fn new(url: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            namespace: namespace.into(),
            connection: Arc::new(Mutex::new(None)),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        let keys = self.keys();

        let ready_tasks = self.load_ready_tasks(connection).await?;
        let delayed_ids = connection
            .send_command(
                &[
                    "ZRANGE".to_string(),
                    keys.delayed.clone(),
                    "0".to_string(),
                    "-1".to_string(),
                ],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?
            .into_strings("redis scheduler", ErrorContext::Scheduler)?;
        let delayed = self.load_tasks(connection, &delayed_ids).await?;

        let mut inflight_ids = connection
            .send_command(
                &["SMEMBERS".to_string(), keys.inflight.clone()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?
            .into_strings("redis scheduler", ErrorContext::Scheduler)?;
        inflight_ids.sort();
        let inflight = self.load_tasks(connection, &inflight_ids).await?;

        Ok(Checkpoint {
            ready: ready_tasks.into_iter().map(|(task, _)| task).collect(),
            delayed,
            inflight,
        })
    }

    pub async fn counts(&self) -> Result<Counts, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        let keys = self.keys();

        let ready = connection
            .send_command(
                &["SCARD".to_string(), keys.ready.clone()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?
            .into_integer("redis scheduler", ErrorContext::Scheduler)?;
        let delayed = connection
            .send_command(
                &["ZCARD".to_string(), keys.delayed.clone()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?
            .into_integer("redis scheduler", ErrorContext::Scheduler)?;
        let inflight = connection
            .send_command(
                &["SCARD".to_string(), keys.inflight.clone()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?
            .into_integer("redis scheduler", ErrorContext::Scheduler)?;

        Ok(Counts {
            ready: usize::try_from(ready).unwrap_or_default(),
            delayed: usize::try_from(delayed).unwrap_or_default(),
            inflight: usize::try_from(inflight).unwrap_or_default(),
        })
    }

    pub async fn close(&self) -> Result<(), SpiderError> {
        let connection = {
            let mut guard = self.connection.lock().await;
            guard.take()
        };

        if let Some(mut connection) = connection {
            connection
                .close("redis scheduler", ErrorContext::Scheduler)
                .await?;
        }

        Ok(())
    }

    fn validate(&self) -> Result<Endpoint, SpiderError> {
        let endpoint = Endpoint::parse(&self.url, "redis scheduler", ErrorContext::Scheduler)?;

        if self.namespace.trim().is_empty() {
            return Err(SpiderError::scheduler(
                "redis scheduler namespace cannot be empty",
            ));
        }

        Ok(endpoint)
    }

    async fn connection(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<Connection>>, SpiderError> {
        let endpoint = self.validate()?;
        let mut guard = self.connection.lock().await;

        if guard.is_none() {
            *guard = Some(
                Connection::connect(&endpoint, "redis scheduler", ErrorContext::Scheduler).await?,
            );
        }

        Ok(guard)
    }

    fn connection_mut<'a>(
        &self,
        guard: &'a mut tokio::sync::MutexGuard<'_, Option<Connection>>,
    ) -> Result<&'a mut Connection, SpiderError> {
        guard.as_mut().ok_or_else(|| {
            SpiderError::scheduler("redis scheduler connection is missing after initialization")
        })
    }

    fn keys(&self) -> Keys {
        Keys {
            tasks: format!("{}:tasks", self.namespace),
            ready: format!("{}:ready", self.namespace),
            ready_order: format!("{}:ready_order", self.namespace),
            delayed: format!("{}:delayed", self.namespace),
            inflight: format!("{}:inflight", self.namespace),
            sequence: format!("{}:ready_sequence", self.namespace),
        }
    }

    async fn enqueue_internal(
        &self,
        connection: &mut Connection,
        task: Task,
    ) -> Result<(), SpiderError> {
        let keys = self.keys();
        let task_json = serde_json::to_string(&task).map_err(|error| {
            SpiderError::scheduler(format!("failed to encode redis scheduler task: {error}"))
        })?;
        connection
            .send_command(
                &[
                    "HSET".to_string(),
                    keys.tasks.clone(),
                    task.id.as_str().to_string(),
                    task_json,
                ],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;

        if task.is_ready() {
            self.push_ready_task(connection, task.id.as_str()).await
        } else {
            connection
                .send_command(
                    &[
                        "ZADD".to_string(),
                        keys.delayed.clone(),
                        i64::try_from(task.ready_at_ms.unwrap_or_default())
                            .unwrap_or_default()
                            .to_string(),
                        task.id.as_str().to_string(),
                    ],
                    "redis scheduler",
                    ErrorContext::Scheduler,
                )
                .await?;
            Ok(())
        }
    }

    async fn push_ready_task(
        &self,
        connection: &mut Connection,
        task_id: &str,
    ) -> Result<(), SpiderError> {
        let keys = self.keys();
        let ready_order = self.next_ready_order(connection).await?;
        connection
            .send_command(
                &["SADD".to_string(), keys.ready.clone(), task_id.to_string()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        connection
            .send_command(
                &[
                    "HSET".to_string(),
                    keys.ready_order.clone(),
                    task_id.to_string(),
                    ready_order.to_string(),
                ],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        Ok(())
    }

    async fn next_ready_order(&self, connection: &mut Connection) -> Result<i64, SpiderError> {
        let keys = self.keys();
        connection
            .send_command(
                &["INCR".to_string(), keys.sequence.clone()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?
            .into_integer("redis scheduler", ErrorContext::Scheduler)
    }

    async fn promote_delayed(&self, connection: &mut Connection) -> Result<(), SpiderError> {
        let keys = self.keys();
        let delayed_ids = connection
            .send_command(
                &[
                    "ZRANGEBYSCORE".to_string(),
                    keys.delayed.clone(),
                    "-inf".to_string(),
                    now_ms().to_string(),
                ],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?
            .into_strings("redis scheduler", ErrorContext::Scheduler)?;

        for task_id in delayed_ids {
            connection
                .send_command(
                    &["ZREM".to_string(), keys.delayed.clone(), task_id.clone()],
                    "redis scheduler",
                    ErrorContext::Scheduler,
                )
                .await?;
            self.push_ready_task(connection, &task_id).await?;
        }

        Ok(())
    }

    async fn load_ready_tasks(
        &self,
        connection: &mut Connection,
    ) -> Result<Vec<(Task, i64)>, SpiderError> {
        let keys = self.keys();
        let ready_ids = connection
            .send_command(
                &["SMEMBERS".to_string(), keys.ready.clone()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?
            .into_strings("redis scheduler", ErrorContext::Scheduler)?;

        let mut tasks = Vec::with_capacity(ready_ids.len());
        for task_id in ready_ids {
            let task = self.load_task(connection, &task_id).await?;
            let ready_order = self.load_ready_order(connection, &task_id).await?;
            tasks.push((task, ready_order));
        }
        tasks.sort_by(ready_task_ordering);
        Ok(tasks)
    }

    async fn load_ready_order(
        &self,
        connection: &mut Connection,
        task_id: &str,
    ) -> Result<i64, SpiderError> {
        let keys = self.keys();
        let reply = connection
            .send_command(
                &[
                    "HGET".to_string(),
                    keys.ready_order.clone(),
                    task_id.to_string(),
                ],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        let Some(value) = reply.into_bulk("redis scheduler", ErrorContext::Scheduler)? else {
            return Ok(i64::MAX);
        };
        value.parse::<i64>().map_err(|error| {
            SpiderError::scheduler(format!(
                "redis scheduler returned invalid ready order: {error}"
            ))
        })
    }

    async fn load_tasks(
        &self,
        connection: &mut Connection,
        task_ids: &[String],
    ) -> Result<Vec<Task>, SpiderError> {
        let mut tasks = Vec::with_capacity(task_ids.len());
        for task_id in task_ids {
            tasks.push(self.load_task(connection, task_id).await?);
        }
        Ok(tasks)
    }

    async fn load_task(
        &self,
        connection: &mut Connection,
        task_id: &str,
    ) -> Result<Task, SpiderError> {
        let keys = self.keys();
        let reply = connection
            .send_command(
                &["HGET".to_string(), keys.tasks.clone(), task_id.to_string()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        let task_json = reply
            .into_bulk("redis scheduler", ErrorContext::Scheduler)?
            .ok_or_else(|| {
                SpiderError::scheduler(format!(
                    "redis scheduler task payload is missing for task id {task_id}"
                ))
            })?;
        serde_json::from_str(&task_json).map_err(|error| {
            SpiderError::scheduler(format!("failed to decode redis scheduler task: {error}"))
        })
    }

    async fn remove_completed_task(
        &self,
        connection: &mut Connection,
        task_id: &TaskId,
    ) -> Result<(), SpiderError> {
        let keys = self.keys();
        let task_id = task_id.as_str().to_string();
        connection
            .send_command(
                &["SREM".to_string(), keys.inflight.clone(), task_id.clone()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        connection
            .send_command(
                &["SREM".to_string(), keys.ready.clone(), task_id.clone()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        connection
            .send_command(
                &["ZREM".to_string(), keys.delayed.clone(), task_id.clone()],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        connection
            .send_command(
                &[
                    "HDEL".to_string(),
                    keys.ready_order.clone(),
                    task_id.clone(),
                ],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        connection
            .send_command(
                &["HDEL".to_string(), keys.tasks.clone(), task_id],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        Ok(())
    }
}

impl Scheduler for Redis {
    async fn enqueue(&mut self, task: Task) -> Result<(), SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.enqueue_internal(connection, task).await
    }

    async fn take_ready(&mut self) -> Result<Option<Task>, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.promote_delayed(connection).await?;

        let ready_tasks = self.load_ready_tasks(connection).await?;
        let Some((task, _)) = ready_tasks.into_iter().next() else {
            return Ok(None);
        };

        let keys = self.keys();
        connection
            .send_command(
                &[
                    "SREM".to_string(),
                    keys.ready.clone(),
                    task.id.as_str().to_string(),
                ],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        connection
            .send_command(
                &[
                    "HDEL".to_string(),
                    keys.ready_order.clone(),
                    task.id.as_str().to_string(),
                ],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        connection
            .send_command(
                &[
                    "SADD".to_string(),
                    keys.inflight.clone(),
                    task.id.as_str().to_string(),
                ],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;
        Ok(Some(task))
    }

    async fn complete(&mut self, task_id: &TaskId) -> Result<(), SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        self.remove_completed_task(connection, task_id).await
    }

    async fn requeue(&mut self, task_id: &TaskId) -> Result<(), SpiderError> {
        let mut guard = self.connection().await?;
        let connection = self.connection_mut(&mut guard)?;
        let task = match self.load_task(connection, task_id.as_str()).await {
            Ok(task) => task,
            Err(_) => return Ok(()),
        };
        let keys = self.keys();
        connection
            .send_command(
                &[
                    "SREM".to_string(),
                    keys.inflight.clone(),
                    task_id.as_str().to_string(),
                ],
                "redis scheduler",
                ErrorContext::Scheduler,
            )
            .await?;

        if task.is_ready() {
            self.push_ready_task(connection, task_id.as_str()).await
        } else {
            connection
                .send_command(
                    &[
                        "ZADD".to_string(),
                        keys.delayed.clone(),
                        i64::try_from(task.ready_at_ms.unwrap_or_default())
                            .unwrap_or_default()
                            .to_string(),
                        task_id.as_str().to_string(),
                    ],
                    "redis scheduler",
                    ErrorContext::Scheduler,
                )
                .await?;
            Ok(())
        }
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        Ok(self.counts().await?.has_pending())
    }
}

#[derive(Debug, Clone)]
struct Keys {
    tasks: String,
    ready: String,
    ready_order: String,
    delayed: String,
    inflight: String,
    sequence: String,
}

fn ready_task_ordering(left: &(Task, i64), right: &(Task, i64)) -> Ordering {
    right
        .0
        .priority
        .cmp(&left.0.priority)
        .then_with(|| left.0.depth.cmp(&right.0.depth))
        .then_with(|| left.1.cmp(&right.1))
        .then_with(|| left.0.id.as_str().cmp(right.0.id.as_str()))
}

fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redis::test_support::spawn_redis_server;
    use crate::request::Request;
    use jiff::SignedDuration;

    #[tokio::test]
    async fn redis_scheduler_supports_async_enqueue_and_take_ready() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let mut scheduler = Redis::new(format!("redis://{url}"), "news");
        let task = Task::new(Request::new("https://example.com"));

        scheduler.enqueue(task.clone()).await.unwrap();
        let taken = scheduler.take_ready().await.unwrap();

        assert_eq!(
            taken.as_ref().map(|task| task.id.as_str()),
            Some(task.id.as_str())
        );
        assert_eq!(taken.map(|task| task.request.url), Some(task.request.url));

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_prefers_higher_priority_then_lower_depth() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let mut scheduler = Redis::new(format!("redis://{url}"), "ordering");
        scheduler
            .enqueue(
                Task::new(Request::new("https://example.com/depth-2"))
                    .with_priority(0)
                    .with_depth(2),
            )
            .await
            .unwrap();
        scheduler
            .enqueue(
                Task::new(Request::new("https://example.com/high-priority"))
                    .with_priority(10)
                    .with_depth(8),
            )
            .await
            .unwrap();
        scheduler
            .enqueue(
                Task::new(Request::new("https://example.com/depth-0"))
                    .with_priority(0)
                    .with_depth(0),
            )
            .await
            .unwrap();

        let first = scheduler.take_ready().await.unwrap().unwrap();
        let second = scheduler.take_ready().await.unwrap().unwrap();
        let third = scheduler.take_ready().await.unwrap().unwrap();

        assert_eq!(first.request.url, "https://example.com/high-priority");
        assert_eq!(second.request.url, "https://example.com/depth-0");
        assert_eq!(third.request.url, "https://example.com/depth-2");

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_skips_delayed_task_until_ready() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let mut scheduler = Redis::new(format!("redis://{url}"), "delayed");
        scheduler
            .enqueue(Task::with_delay_ms(
                Request::new("https://example.com/delayed"),
                60,
            ))
            .await
            .unwrap();

        assert!(scheduler.take_ready().await.unwrap().is_none());
        tokio::time::sleep(std::time::Duration::try_from(SignedDuration::from_millis(80)).unwrap())
            .await;

        let taken = scheduler.take_ready().await.unwrap().unwrap();
        assert_eq!(taken.request.url, "https://example.com/delayed");

        scheduler.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_restores_tasks_from_existing_namespace() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let namespace = "restore";
        let mut first = Redis::new(format!("redis://{url}"), namespace);
        first
            .enqueue(Task::new(Request::new("https://example.com/restored")))
            .await
            .unwrap();
        first.close().await.unwrap();

        let mut second = Redis::new(format!("redis://{url}"), namespace);
        let taken = second.take_ready().await.unwrap().unwrap();

        assert_eq!(taken.request.url, "https://example.com/restored");

        second.close().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_scheduler_rejects_empty_namespace() {
        let mut scheduler = Redis::new("redis://127.0.0.1:6379", "   ");

        let error = scheduler
            .enqueue(Task::new(Request::new("https://example.com")))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            SpiderError::scheduler("redis scheduler namespace cannot be empty")
        );
    }
}
