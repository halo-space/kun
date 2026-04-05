use crate::error::SpiderError;
use crate::item::Item;
use crate::redis::{Connection, ErrorContext, connect, query, validate_url};
use crate::store::Store;
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
        validate_url(&self.url, "redis store", ErrorContext::Engine)?;

        if self.key.trim().is_empty() {
            return Err(SpiderError::engine("redis store key cannot be empty"));
        }

        Ok(())
    }

    async fn connection(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<Connection>>, SpiderError> {
        self.validate()?;
        let mut guard = self.connection.lock().await;

        if guard.is_none() {
            *guard = Some(connect(&self.url, "redis store", ErrorContext::Engine).await?);
        }

        Ok(guard)
    }
}

impl Store for Redis {
    async fn open(&self, _spider_name: &str) -> Result<(), SpiderError> {
        let _guard = self.connection().await?;
        Ok(())
    }

    async fn write(&self, item: &Item, _spider_name: &str) -> Result<(), SpiderError> {
        let payload = serde_json::to_string(&item.to_json()).map_err(|error| {
            SpiderError::engine(format!("failed to serialize item for redis store: {error}"))
        })?;

        let mut guard = self.connection().await?;
        let connection = guard.as_mut().ok_or_else(|| {
            SpiderError::engine("redis store connection is missing after initialization")
        })?;
        let mut command = redis::cmd("SADD");
        command.arg(&self.key).arg(payload);
        let _: i64 = query(
            connection,
            &mut command,
            "redis store",
            ErrorContext::Engine,
        )
        .await?;
        Ok(())
    }

    async fn batch_write(&self, items: &[Item], _spider_name: &str) -> Result<(), SpiderError> {
        if items.is_empty() {
            return Ok(());
        }

        let mut payloads = Vec::with_capacity(items.len());
        for item in items {
            payloads.push(serde_json::to_string(&item.to_json()).map_err(|error| {
                SpiderError::engine(format!("failed to serialize item for redis store: {error}"))
            })?);
        }

        let mut guard = self.connection().await?;
        let connection = guard.as_mut().ok_or_else(|| {
            SpiderError::engine("redis store connection is missing after initialization")
        })?;
        let mut command = redis::cmd("SADD");
        command.arg(&self.key);
        for payload in payloads {
            command.arg(payload);
        }
        let _: i64 = query(
            connection,
            &mut command,
            "redis store",
            ErrorContext::Engine,
        )
        .await?;
        Ok(())
    }

    async fn close(&self, _spider_name: &str) -> Result<(), SpiderError> {
        let _connection = {
            let mut guard = self.connection.lock().await;
            guard.take()
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::redis::spawn_redis_server;
    use crate::value::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::net::TcpStream;

    #[tokio::test]
    async fn redis_store_authenticates_selects_database_and_adds_item_json_to_set() {
        let (url, commands_rx, server_handle) = spawn_redis_server().await;
        let store = Redis::new(format!("redis://spider:secret@{url}/2"), "period_items");
        let item = Item::new()
            .with_field("title", Value::String("period".to_string()))
            .with_field("front_page", Value::String("A01".to_string()));

        store.open("news").await.unwrap();
        store.write(&item, "news").await.unwrap();
        store.close("news").await.unwrap();

        let commands = commands_rx.await.unwrap();
        assert_eq!(
            commands,
            vec![
                vec![
                    "AUTH".to_string(),
                    "spider".to_string(),
                    "secret".to_string()
                ],
                vec!["SELECT".to_string(), "2".to_string()],
                vec![
                    "SADD".to_string(),
                    "period_items".to_string(),
                    "{\"front_page\":\"A01\",\"title\":\"period\"}".to_string()
                ]
            ]
        );

        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_store_batch_write_adds_multiple_item_json_values_to_set() {
        let (url, commands_rx, server_handle) = spawn_redis_server().await;
        let store = Redis::new(format!("redis://{url}"), "period_items");
        let first = Item::new().with_field("title", Value::String("first".to_string()));
        let second = Item::new().with_field("title", Value::String("second".to_string()));

        store.open("news").await.unwrap();
        store.batch_write(&[first, second], "news").await.unwrap();
        store.close("news").await.unwrap();

        let commands = commands_rx.await.unwrap();
        assert_eq!(
            commands,
            vec![vec![
                "SADD".to_string(),
                "period_items".to_string(),
                "{\"title\":\"first\"}".to_string(),
                "{\"title\":\"second\"}".to_string()
            ]]
        );

        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn redis_store_rejects_invalid_url_scheme() {
        let store = Redis::new("http://127.0.0.1:6379", "items");

        let error = store.open("news").await.unwrap_err();

        assert_eq!(
            error,
            SpiderError::engine("redis store url must use redis:// scheme: http://127.0.0.1:6379",)
        );
    }

    #[tokio::test]
    async fn redis_store_rejects_empty_key() {
        let store = Redis::new("redis://127.0.0.1:6379", "   ");

        let error = store.open("news").await.unwrap_err();

        assert_eq!(
            error,
            SpiderError::engine("redis store key cannot be empty")
        );
    }

    #[tokio::test]
    async fn redis_store_surfaces_error_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        let server_handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let command = read_until_sadd(&mut stream).await?;
            assert_eq!(command[0], "SADD");
            stream.write_all(b"-ERR target rejected\r\n").await?;
            stream.shutdown().await
        });

        let store = Redis::new(format!("redis://{address}"), "items");
        let item = Item::new().with_field("title", Value::String("period".to_string()));

        store.open("news").await.unwrap();
        let error = store.write(&item, "news").await.unwrap_err();

        assert_eq!(
            error,
            SpiderError::engine("redis store command failed: ERR target rejected")
        );

        server_handle.await.unwrap().unwrap();
    }

    async fn read_until_sadd(stream: &mut TcpStream) -> Result<Vec<String>, std::io::Error> {
        loop {
            let Some(command) = read_resp_command(stream).await? else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "stream closed before SADD command arrived",
                ));
            };

            if matches!(
                command.as_slice(),
                [name, action, ..] if name == "CLIENT" && action == "SETINFO"
            ) {
                stream.write_all(b"+OK\r\n").await?;
                continue;
            }

            return Ok(command);
        }
    }

    async fn read_resp_command(
        stream: &mut TcpStream,
    ) -> Result<Option<Vec<String>>, std::io::Error> {
        let mut prefix = [0_u8; 1];
        match stream.read_exact(&mut prefix).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }

        if prefix[0] != b'*' {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "resp command did not start with array marker",
            ));
        }

        let count = read_test_resp_line(stream)
            .await?
            .parse::<usize>()
            .map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid array length: {error}"),
                )
            })?;
        let mut command = Vec::with_capacity(count);

        for _ in 0..count {
            let mut bulk_prefix = [0_u8; 1];
            stream.read_exact(&mut bulk_prefix).await?;
            if bulk_prefix[0] != b'$' {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "resp bulk string did not start with $",
                ));
            }

            let length = read_test_resp_line(stream)
                .await?
                .parse::<usize>()
                .map_err(|error| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid bulk string length: {error}"),
                    )
                })?;

            let mut bytes = vec![0_u8; length + 2];
            stream.read_exact(&mut bytes).await?;
            command.push(
                String::from_utf8(bytes[..length].to_vec()).map_err(|error| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("command was not utf-8: {error}"),
                    )
                })?,
            );
        }

        Ok(Some(command))
    }

    async fn read_test_resp_line(stream: &mut TcpStream) -> Result<String, std::io::Error> {
        let mut bytes = Vec::new();

        loop {
            let mut byte = [0_u8; 1];
            stream.read_exact(&mut byte).await?;
            if byte[0] == b'\r' {
                let mut line_feed = [0_u8; 1];
                stream.read_exact(&mut line_feed).await?;
                if line_feed[0] != b'\n' {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "expected LF after CR",
                    ));
                }
                break;
            }
            bytes.push(byte[0]);
        }

        String::from_utf8(bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("line was not utf-8: {error}"),
            )
        })
    }
}
