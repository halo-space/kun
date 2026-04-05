use crate::error::SpiderError;
use crate::item::Item;
use crate::store::Store;
use jiff::SignedDuration;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookMethod {
    Post,
    Put,
}

impl WebhookMethod {
    fn as_reqwest_method(self) -> reqwest::Method {
        match self {
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Webhook {
    url: String,
    method: WebhookMethod,
    headers: Vec<(String, String)>,
    retry_limit: usize,
    retry_backoff: SignedDuration,
    client: reqwest::Client,
}

impl Webhook {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: WebhookMethod::Post,
            headers: Vec::new(),
            retry_limit: 0,
            retry_backoff: SignedDuration::from_millis(250),
            client: reqwest::Client::new(),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn method(&self) -> WebhookMethod {
        self.method
    }

    pub fn with_method(mut self, method: WebhookMethod) -> Self {
        self.method = method;
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn with_retry_limit(mut self, retry_limit: usize) -> Self {
        self.retry_limit = retry_limit;
        self
    }

    pub fn with_retry_backoff(mut self, retry_backoff: SignedDuration) -> Self {
        self.retry_backoff = retry_backoff;
        self
    }

    fn build_headers(&self) -> Result<HeaderMap, SpiderError> {
        let mut headers = HeaderMap::new();

        for (name, value) in &self.headers {
            let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                SpiderError::engine(format!(
                    "invalid webhook store header name `{name}`: {error}"
                ))
            })?;
            let header_value = HeaderValue::from_str(value).map_err(|error| {
                SpiderError::engine(format!(
                    "invalid webhook store header value for `{name}`: {error}"
                ))
            })?;
            headers.insert(header_name, header_value);
        }

        Ok(headers)
    }

    fn validate(&self) -> Result<(), SpiderError> {
        reqwest::Url::parse(&self.url).map_err(|error| {
            SpiderError::engine(format!("invalid webhook store url `{}`: {error}", self.url))
        })?;
        self.build_headers()?;
        to_std_duration(self.retry_backoff).map_err(|error| {
            SpiderError::engine(format!("invalid webhook store retry_backoff: {error}"))
        })?;
        Ok(())
    }

    fn retry_delay(&self, retry_number: usize) -> Result<std::time::Duration, SpiderError> {
        let multiplier = i32::try_from(retry_number)
            .map_err(|_| SpiderError::engine("webhook store retry number overflowed i32"))?;
        let delay = self
            .retry_backoff
            .checked_mul(multiplier)
            .ok_or_else(|| SpiderError::engine("webhook store retry_backoff overflowed"))?;
        to_std_duration(delay).map_err(|error| {
            SpiderError::engine(format!("invalid webhook store retry_backoff: {error}"))
        })
    }

    fn should_retry_status(status: reqwest::StatusCode) -> bool {
        status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
    }
}

impl Store for Webhook {
    async fn open(&self, _spider_name: &str) -> Result<(), SpiderError> {
        self.validate()
    }

    async fn write(&self, item: &Item, _spider_name: &str) -> Result<(), SpiderError> {
        self.validate()?;
        let headers = self.build_headers()?;

        for attempt in 0..=self.retry_limit {
            let response = self
                .client
                .request(self.method.as_reqwest_method(), &self.url)
                .headers(headers.clone())
                .json(&item.to_json())
                .send()
                .await;

            match response {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) => {
                    let status = response.status();
                    if attempt < self.retry_limit && Self::should_retry_status(status) {
                        tokio::time::sleep(self.retry_delay(attempt + 1)?).await;
                        continue;
                    }
                    return Err(SpiderError::engine(format!(
                        "webhook store received non-success status {} from {}",
                        status, self.url
                    )));
                }
                Err(error) => {
                    if attempt < self.retry_limit {
                        tokio::time::sleep(self.retry_delay(attempt + 1)?).await;
                        continue;
                    }
                    return Err(SpiderError::engine(format!(
                        "webhook store request failed: {error}"
                    )));
                }
            }
        }

        Err(SpiderError::engine(
            "webhook store exhausted retries without a final result",
        ))
    }
}

fn to_std_duration(duration: SignedDuration) -> Result<std::time::Duration, String> {
    std::time::Duration::try_from(duration).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn webhook_store_posts_item_json_with_headers() {
        let (url, mut requests, server_handle) = spawn_test_server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
        ])
        .await;
        let store = Webhook::new(url)
            .with_method(WebhookMethod::Put)
            .with_header("x-api-key", "secret");
        let item = Item::new()
            .with_field("title", Value::String("period".to_string()))
            .with_field("front_page", Value::String("A01".to_string()));

        store.open("news").await.unwrap();
        store.write(&item, "news").await.unwrap();

        let request = requests.recv().await.unwrap();
        assert!(request.starts_with("PUT /items HTTP/1.1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-api-key: secret\r\n")
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/json\r\n")
        );
        assert!(request.ends_with("{\"front_page\":\"A01\",\"title\":\"period\"}"));

        server_handle.await.unwrap();
    }

    #[tokio::test]
    async fn webhook_store_rejects_non_success_status() {
        let (url, _requests, server_handle) = spawn_test_server(vec![
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 5\r\nConnection: close\r\n\r\nerror",
        ])
        .await;
        let store = Webhook::new(url);
        let item = Item::new().with_field("title", Value::String("period".to_string()));

        store.open("news").await.unwrap();
        let error = store.write(&item, "news").await.unwrap_err();

        assert!(matches!(error, SpiderError::Engine(message)
                if message.starts_with("webhook store received non-success status 500 Internal Server Error from http://127.0.0.1:")));

        server_handle.await.unwrap();
    }

    #[tokio::test]
    async fn webhook_store_retries_retryable_status_with_backoff() {
        let (url, mut requests, server_handle) = spawn_test_server(vec![
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 5\r\nConnection: close\r\n\r\nerror",
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
        ])
        .await;
        let store = Webhook::new(url)
            .with_retry_limit(1)
            .with_retry_backoff(SignedDuration::from_millis(1));
        let item = Item::new().with_field("title", Value::String("period".to_string()));

        store.open("news").await.unwrap();
        store.write(&item, "news").await.unwrap();

        let first = requests.recv().await.unwrap();
        let second = requests.recv().await.unwrap();

        assert!(first.starts_with("POST /items HTTP/1.1\r\n"));
        assert!(second.starts_with("POST /items HTTP/1.1\r\n"));

        server_handle.await.unwrap();
    }

    #[tokio::test]
    async fn webhook_store_does_not_retry_non_retryable_status() {
        let (url, mut requests, server_handle) = spawn_test_server(vec![
            "HTTP/1.1 400 Bad Request\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad",
        ])
        .await;
        let store = Webhook::new(url)
            .with_retry_limit(3)
            .with_retry_backoff(SignedDuration::from_millis(1));
        let item = Item::new().with_field("title", Value::String("period".to_string()));

        store.open("news").await.unwrap();
        let error = store.write(&item, "news").await.unwrap_err();

        assert!(matches!(error, SpiderError::Engine(message)
                if message.starts_with("webhook store received non-success status 400 Bad Request from http://127.0.0.1:")));
        assert!(requests.recv().await.is_some());
        assert!(requests.recv().await.is_none());

        server_handle.await.unwrap();
    }

    #[tokio::test]
    async fn webhook_store_rejects_invalid_header_name() {
        let store = Webhook::new("http://127.0.0.1:1").with_header("bad header", "value");
        let error = store.open("news").await.unwrap_err();

        assert!(matches!(error, SpiderError::Engine(message)
                if message.starts_with("invalid webhook store header name `bad header`:")));
    }

    async fn spawn_test_server(
        responses: Vec<&'static str>,
    ) -> (
        String,
        mpsc::UnboundedReceiver<String>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = mpsc::unbounded_channel();

        let server_handle = tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut stream).await;
                request_tx.send(request).ok();
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        });

        (format!("http://{address}/items"), request_rx, server_handle)
    }

    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut temp = [0_u8; 1024];
        let mut header_end = None;
        let mut content_length = 0_usize;

        loop {
            let bytes_read = stream.read(&mut temp).await.unwrap();
            if bytes_read == 0 {
                break;
            }
            buffer.extend_from_slice(&temp[..bytes_read]);

            if header_end.is_none() {
                if let Some(position) = find_header_end(&buffer) {
                    header_end = Some(position);
                    content_length = parse_content_length(&buffer[..position]);
                }
            }

            if let Some(position) = header_end {
                let body_len = buffer.len().saturating_sub(position);
                if body_len >= content_length {
                    break;
                }
            }
        }

        String::from_utf8(buffer).unwrap()
    }

    fn find_header_end(buffer: &[u8]) -> Option<usize> {
        buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
    }

    fn parse_content_length(headers: &[u8]) -> usize {
        let headers = String::from_utf8_lossy(headers);
        headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }
}
