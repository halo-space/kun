use crate::error::SpiderError;
use crate::item::Item;
use crate::store::Store;
use rdkafka::config::ClientConfig;
use rdkafka::message::{Header, OwnedHeaders};
use rdkafka::producer::{FutureProducer, FutureRecord, Producer};
use rdkafka::util::Timeout;
use std::fmt;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Kafka {
    brokers: String,
    topic: String,
    key: Option<KafkaTextSource>,
    headers: Vec<KafkaHeaderConfig>,
    producer: Arc<Mutex<Option<KafkaClient>>>,
}

impl Kafka {
    pub fn new(brokers: impl Into<String>, topic: impl Into<String>) -> Self {
        Self {
            brokers: brokers.into(),
            topic: topic.into(),
            key: None,
            headers: Vec::new(),
            producer: Arc::new(Mutex::new(None)),
        }
    }

    pub fn brokers(&self) -> &str {
        &self.brokers
    }

    pub fn topic(&self) -> &str {
        &self.topic
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(KafkaTextSource::Static(key.into()));
        self
    }

    pub fn with_key_field(mut self, field: impl Into<String>) -> Self {
        self.key = Some(KafkaTextSource::ItemField(field.into()));
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push(KafkaHeaderConfig {
            name: name.into(),
            value: KafkaTextSource::Static(value.into()),
        });
        self
    }

    pub fn with_header_field(mut self, name: impl Into<String>, field: impl Into<String>) -> Self {
        self.headers.push(KafkaHeaderConfig {
            name: name.into(),
            value: KafkaTextSource::ItemField(field.into()),
        });
        self
    }

    fn validate(&self) -> Result<(), SpiderError> {
        if self.brokers.trim().is_empty() {
            return Err(SpiderError::engine("kafka store brokers cannot be empty"));
        }

        if self.topic.trim().is_empty() {
            return Err(SpiderError::engine("kafka store topic cannot be empty"));
        }

        for header in &self.headers {
            if header.name.trim().is_empty() {
                return Err(SpiderError::engine(
                    "kafka store header name cannot be empty",
                ));
            }
        }

        Ok(())
    }

    fn build_message(&self, item: &Item) -> Result<KafkaMessage, SpiderError> {
        let payload = serialize_item(item)?;
        let key = self
            .key
            .as_ref()
            .map(|source| source.resolve(item, "kafka store key"))
            .transpose()?;
        let headers = self
            .headers
            .iter()
            .map(|header| {
                Ok((
                    header.name.clone(),
                    header
                        .value
                        .resolve(item, &format!("kafka store header `{}`", header.name))?,
                ))
            })
            .collect::<Result<Vec<_>, SpiderError>>()?;

        Ok(KafkaMessage {
            payload,
            key,
            headers,
        })
    }

    async fn producer(&self) -> Result<KafkaClient, SpiderError> {
        let mut guard = self.producer.lock().await;

        if let Some(producer) = guard.as_ref() {
            return Ok(producer.clone());
        }

        self.validate()?;

        let producer = KafkaClient::connect(&self.brokers)?;
        *guard = Some(producer.clone());
        Ok(producer)
    }

    #[cfg(test)]
    fn with_test_producer(
        brokers: impl Into<String>,
        topic: impl Into<String>,
        producer: TestProducer,
    ) -> Self {
        Self {
            brokers: brokers.into(),
            topic: topic.into(),
            key: None,
            headers: Vec::new(),
            producer: Arc::new(Mutex::new(Some(KafkaClient::Test(producer)))),
        }
    }
}

impl fmt::Debug for Kafka {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Kafka")
            .field("brokers", &self.brokers)
            .field("topic", &self.topic)
            .finish_non_exhaustive()
    }
}

impl Store for Kafka {
    async fn open(&self, _spider_name: &str) -> Result<(), SpiderError> {
        let _producer = self.producer().await?;
        Ok(())
    }

    async fn write(&self, item: &Item, _spider_name: &str) -> Result<(), SpiderError> {
        let message = self.build_message(item)?;
        let producer = self.producer().await?;
        producer.send_message(&self.topic, message).await
    }

    async fn batch_write(&self, items: &[Item], _spider_name: &str) -> Result<(), SpiderError> {
        if items.is_empty() {
            return Ok(());
        }

        let producer = self.producer().await?;
        for item in items {
            let message = self.build_message(item)?;
            producer.send_message(&self.topic, message).await?;
        }

        Ok(())
    }

    async fn close(&self, _spider_name: &str) -> Result<(), SpiderError> {
        let producer = {
            let mut guard = self.producer.lock().await;
            guard.take()
        };

        if let Some(producer) = producer {
            producer.close().await?;
        }

        Ok(())
    }
}

fn serialize_item(item: &Item) -> Result<String, SpiderError> {
    serde_json::to_string(&item.to_json()).map_err(|error| {
        SpiderError::engine(format!("failed to serialize item for kafka store: {error}"))
    })
}

#[derive(Debug, Clone)]
enum KafkaTextSource {
    Static(String),
    ItemField(String),
}

impl KafkaTextSource {
    fn resolve(&self, item: &Item, label: &str) -> Result<String, SpiderError> {
        match self {
            Self::Static(value) => Ok(value.clone()),
            Self::ItemField(field) => {
                let value = item.get(field).ok_or_else(|| {
                    SpiderError::engine(format!("{label} field `{field}` is missing"))
                })?;
                value_to_text(value).ok_or_else(|| {
                    SpiderError::engine(format!(
                        "{label} field `{field}` must be string, number, or bool"
                    ))
                })
            }
        }
    }
}

#[derive(Debug, Clone)]
struct KafkaHeaderConfig {
    name: String,
    value: KafkaTextSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KafkaMessage {
    payload: String,
    key: Option<String>,
    headers: Vec<(String, String)>,
}

fn value_to_text(value: &crate::value::Value) -> Option<String> {
    match value {
        crate::value::Value::String(value) => Some(value.clone()),
        crate::value::Value::Bool(value) => Some(value.to_string()),
        crate::value::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn build_kafka_headers(headers: &[(String, String)]) -> OwnedHeaders {
    let mut owned = OwnedHeaders::new_with_capacity(headers.len());

    for (name, value) in headers {
        owned = owned.insert(Header {
            key: name.as_str(),
            value: Some(value.as_str()),
        });
    }

    owned
}

#[derive(Clone)]
enum KafkaClient {
    Real(FutureProducer),
    #[cfg(test)]
    Test(TestProducer),
}

impl KafkaClient {
    fn connect(brokers: &str) -> Result<Self, SpiderError> {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("delivery.timeout.ms", "10000")
            .create()
            .map_err(|error| {
                SpiderError::engine(format!("failed to create kafka store producer: {error}"))
            })?;

        Ok(Self::Real(producer))
    }

    async fn send_message(&self, topic: &str, message: KafkaMessage) -> Result<(), SpiderError> {
        match self {
            Self::Real(producer) => {
                let mut record = FutureRecord::to(topic).payload(message.payload.as_str());
                if let Some(key) = message.key.as_deref() {
                    record = record.key(key);
                }
                if !message.headers.is_empty() {
                    record = record.headers(build_kafka_headers(&message.headers));
                }

                producer
                    .send(record, Timeout::Never)
                    .await
                    .map(|_| ())
                    .map_err(|(error, _)| {
                        SpiderError::engine(format!(
                            "failed to deliver kafka store message: {error}"
                        ))
                    })
            }
            #[cfg(test)]
            Self::Test(producer) => producer.send_message(topic, message).await,
        }
    }

    async fn close(self) -> Result<(), SpiderError> {
        match self {
            Self::Real(producer) => tokio::task::spawn_blocking(move || {
                producer.flush(Timeout::Never).map_err(|error| {
                    SpiderError::engine(format!("failed to flush kafka store producer: {error}"))
                })
            })
            .await
            .map_err(|error| {
                SpiderError::engine(format!(
                    "failed to join kafka store producer flush: {error}"
                ))
            })?,
            #[cfg(test)]
            Self::Test(producer) => producer.close().await,
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
struct TestProducer {
    messages: Arc<Mutex<Vec<(String, KafkaMessage)>>>,
    error: Arc<Mutex<Option<String>>>,
}

#[cfg(test)]
impl TestProducer {
    fn failing(message: impl Into<String>) -> Self {
        Self {
            messages: Arc::new(Mutex::new(Vec::new())),
            error: Arc::new(Mutex::new(Some(message.into()))),
        }
    }

    async fn send_message(&self, topic: &str, message: KafkaMessage) -> Result<(), SpiderError> {
        if let Some(message) = self.error.lock().await.clone() {
            return Err(SpiderError::engine(format!(
                "failed to deliver kafka store message: {message}"
            )));
        }

        self.messages
            .lock()
            .await
            .push((topic.to_string(), message));
        Ok(())
    }

    async fn messages(&self) -> Vec<(String, KafkaMessage)> {
        self.messages.lock().await.clone()
    }

    async fn close(&self) -> Result<(), SpiderError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    #[tokio::test]
    async fn kafka_store_rejects_empty_brokers() {
        let store = Kafka::new("   ", "period_items");

        let error = store.open("news").await.unwrap_err();

        assert_eq!(
            error,
            SpiderError::engine("kafka store brokers cannot be empty")
        );
    }

    #[tokio::test]
    async fn kafka_store_rejects_empty_topic() {
        let store = Kafka::new("127.0.0.1:9092", "   ");

        let error = store.open("news").await.unwrap_err();

        assert_eq!(
            error,
            SpiderError::engine("kafka store topic cannot be empty")
        );
    }

    #[tokio::test]
    async fn kafka_store_sends_item_json_message() {
        let producer = TestProducer::default();
        let store = Kafka::with_test_producer("127.0.0.1:9092", "period_items", producer.clone());
        let item = Item::new()
            .with_field("period_date", Value::String("2026-03-31".to_string()))
            .with_field("front_page", Value::String("01".to_string()));

        store.open("news").await.unwrap();
        store.write(&item, "news").await.unwrap();
        store.close("news").await.unwrap();

        assert_eq!(
            producer.messages().await,
            vec![(
                "period_items".to_string(),
                KafkaMessage {
                    payload: serde_json::json!({
                        "front_page": "01",
                        "period_date": "2026-03-31",
                    })
                    .to_string(),
                    key: None,
                    headers: Vec::new(),
                },
            )]
        );
    }

    #[tokio::test]
    async fn kafka_store_batch_write_sends_multiple_item_json_messages() {
        let producer = TestProducer::default();
        let store = Kafka::with_test_producer("127.0.0.1:9092", "period_items", producer.clone());
        let first =
            Item::new().with_field("issue_key", Value::String("2026-03-31-front-01".into()));
        let second =
            Item::new().with_field("issue_key", Value::String("2026-03-30-front-01".into()));

        store.open("news").await.unwrap();
        store.batch_write(&[first, second], "news").await.unwrap();
        store.close("news").await.unwrap();

        assert_eq!(
            producer.messages().await,
            vec![
                (
                    "period_items".to_string(),
                    KafkaMessage {
                        payload: serde_json::json!({"issue_key": "2026-03-31-front-01"})
                            .to_string(),
                        key: None,
                        headers: Vec::new(),
                    },
                ),
                (
                    "period_items".to_string(),
                    KafkaMessage {
                        payload: serde_json::json!({"issue_key": "2026-03-30-front-01"})
                            .to_string(),
                        key: None,
                        headers: Vec::new(),
                    },
                ),
            ]
        );
    }

    #[tokio::test]
    async fn kafka_store_can_send_key_and_headers_from_static_and_item_fields() {
        let producer = TestProducer::default();
        let store = Kafka::with_test_producer("127.0.0.1:9092", "period_items", producer.clone())
            .with_key_field("issue_key")
            .with_header("x-spider", "period_kafka")
            .with_header_field("x-date", "period_date");
        let item = Item::new()
            .with_field(
                "issue_key",
                Value::String("2026-03-31-front-01".to_string()),
            )
            .with_field("period_date", Value::String("2026-03-31".to_string()));

        store.open("news").await.unwrap();
        store.write(&item, "news").await.unwrap();
        store.close("news").await.unwrap();

        assert_eq!(
            producer.messages().await,
            vec![(
                "period_items".to_string(),
                KafkaMessage {
                    payload: serde_json::json!({
                        "issue_key": "2026-03-31-front-01",
                        "period_date": "2026-03-31",
                    })
                    .to_string(),
                    key: Some("2026-03-31-front-01".to_string()),
                    headers: vec![
                        ("x-spider".to_string(), "period_kafka".to_string()),
                        ("x-date".to_string(), "2026-03-31".to_string()),
                    ],
                },
            )]
        );
    }

    #[tokio::test]
    async fn kafka_store_rejects_missing_key_field() {
        let producer = TestProducer::default();
        let store = Kafka::with_test_producer("127.0.0.1:9092", "period_items", producer)
            .with_key_field("issue_key");
        let item = Item::new().with_field("title", Value::String("period".to_string()));

        store.open("news").await.unwrap();
        let error = store.write(&item, "news").await.unwrap_err();

        assert_eq!(
            error,
            SpiderError::engine("kafka store key field `issue_key` is missing")
        );
    }

    #[tokio::test]
    async fn kafka_store_surfaces_delivery_error() {
        let producer = TestProducer::failing("Message production error: queue full");
        let store = Kafka::with_test_producer("127.0.0.1:9092", "period_items", producer);
        let item = Item::new().with_field("title", Value::String("period".to_string()));

        store.open("news").await.unwrap();
        let error = store.write(&item, "news").await.unwrap_err();

        assert_eq!(
            error,
            SpiderError::engine(
                "failed to deliver kafka store message: Message production error: queue full"
            )
        );
    }
}
