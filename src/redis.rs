use crate::error::SpiderError;
use redis::{Cmd, FromRedisValue, aio::MultiplexedConnection};
use url::Url;

pub(crate) type Connection = MultiplexedConnection;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ErrorContext {
    Engine,
    Scheduler,
}

impl ErrorContext {
    fn error(self, message: impl Into<String>) -> SpiderError {
        match self {
            Self::Engine => SpiderError::engine(message),
            Self::Scheduler => SpiderError::scheduler(message.into()),
        }
    }
}

pub(crate) fn validate_url(
    url: &str,
    label: &str,
    context: ErrorContext,
) -> Result<(), SpiderError> {
    let parsed_url = Url::parse(url)
        .map_err(|error| context.error(format!("invalid {label} url `{url}`: {error}")))?;

    if parsed_url.scheme() != "redis" {
        return Err(context.error(format!("{label} url must use redis:// scheme: {url}")));
    }

    if parsed_url.host_str().is_none() {
        return Err(context.error(format!("{label} url must include a host: {url}")));
    }

    Ok(())
}

pub(crate) async fn connect(
    url: &str,
    label: &str,
    context: ErrorContext,
) -> Result<Connection, SpiderError> {
    validate_url(url, label, context)?;

    let client = redis::Client::open(url)
        .map_err(|error| context.error(format!("invalid {label} url `{url}`: {error}")))?;

    client
        .get_multiplexed_async_connection()
        .await
        .map_err(|error| context.error(format!("failed to connect to {label} at {url}: {error}")))
}

pub(crate) async fn query<T>(
    connection: &mut Connection,
    command: &mut Cmd,
    label: &str,
    context: ErrorContext,
) -> Result<T, SpiderError>
where
    T: FromRedisValue,
{
    command.query_async(connection).await.map_err(|error| {
        context.error(format!(
            "{label} command failed: {}",
            redis_error_text(&error)
        ))
    })
}

fn redis_error_text(error: &redis::RedisError) -> String {
    match (error.code(), error.detail()) {
        (Some(code), Some(detail)) => format!("{code} {detail}"),
        _ => error.to_string(),
    }
}
