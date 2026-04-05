use crate::error::SpiderError;
use crate::redis::{Connection, ErrorContext, connect, query, validate_url};
use crate::scheduler::checkpoint::{Checkpoint, Persist};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct Redis {
    url: String,
    key: String,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl Redis {
    pub fn new(url: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            key: key.into(),
            connection: Arc::new(Mutex::new(None)),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    fn validate(&self) -> Result<(), SpiderError> {
        validate_url(
            &self.url,
            "redis scheduler checkpoint",
            ErrorContext::Scheduler,
        )?;

        if self.key.trim().is_empty() {
            return Err(SpiderError::scheduler(
                "redis scheduler checkpoint key cannot be empty",
            ));
        }

        Ok(())
    }

    async fn connection(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<Connection>>, SpiderError> {
        self.validate()?;
        let mut guard = self.connection.lock().await;

        if guard.is_none() {
            *guard = Some(
                connect(
                    &self.url,
                    "redis scheduler checkpoint",
                    ErrorContext::Scheduler,
                )
                .await?,
            );
        }

        Ok(guard)
    }
}

impl Persist for Redis {
    async fn load(&self) -> Result<Checkpoint, SpiderError> {
        let mut guard = self.connection().await?;
        let connection = guard.as_mut().ok_or_else(|| {
            SpiderError::scheduler(
                "redis scheduler checkpoint connection is missing after initialization",
            )
        })?;
        let mut command = redis::cmd("GET");
        command.arg(&self.key);
        let payload: Option<String> = query(
            connection,
            &mut command,
            "redis scheduler checkpoint",
            ErrorContext::Scheduler,
        )
        .await?;

        let Some(payload) = payload else {
            return Ok(Checkpoint::default());
        };

        serde_json::from_str(&payload).map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to decode redis scheduler checkpoint: {error}"
            ))
        })
    }

    async fn save(&self, checkpoint: &Checkpoint) -> Result<(), SpiderError> {
        let payload = serde_json::to_string(checkpoint).map_err(|error| {
            SpiderError::scheduler(format!(
                "failed to encode redis scheduler checkpoint: {error}"
            ))
        })?;

        let mut guard = self.connection().await?;
        let connection = guard.as_mut().ok_or_else(|| {
            SpiderError::scheduler(
                "redis scheduler checkpoint connection is missing after initialization",
            )
        })?;
        let mut command = redis::cmd("SET");
        command.arg(&self.key).arg(payload);
        let _: String = query(
            connection,
            &mut command,
            "redis scheduler checkpoint",
            ErrorContext::Scheduler,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::scheduler::Task;
    use crate::test_support::redis::spawn_redis_server;

    #[tokio::test]
    async fn redis_round_trips_checkpoint() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let persist = Redis::new(format!("redis://{url}"), "scheduler_checkpoint");
        let checkpoint = Checkpoint {
            ready: vec![Task::new(Request::new("https://example.com/ready")).with_priority(5)],
            delayed: Vec::new(),
            inflight: vec![Task::new(Request::new("https://example.com/inflight"))],
        };

        persist.save(&checkpoint).await.unwrap();
        let restored = persist.load().await.unwrap();

        assert_eq!(restored.ready.len(), 1);
        assert_eq!(restored.ready[0].priority, 5);
        assert_eq!(restored.inflight.len(), 1);

        drop(persist);
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_returns_default_checkpoint_when_key_is_missing() {
        let (url, _commands_rx, server_handle) = spawn_redis_server().await;
        let persist = Redis::new(format!("redis://{url}"), "missing_checkpoint");

        let checkpoint = persist.load().await.unwrap();

        assert!(checkpoint.ready.is_empty());
        assert!(checkpoint.delayed.is_empty());
        assert!(checkpoint.inflight.is_empty());

        drop(persist);
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_rejects_empty_key() {
        let persist = Redis::new("redis://127.0.0.1:6379", "   ");

        let error = persist.load().await.unwrap_err();

        assert_eq!(
            error,
            SpiderError::scheduler("redis scheduler checkpoint key cannot be empty")
        );
    }
}
