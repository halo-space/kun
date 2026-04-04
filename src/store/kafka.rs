use crate::error::SpiderError;
use crate::item::Item;
use crate::store::Store;
use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord, Producer};
use rdkafka::util::Timeout;
use std::fmt;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Kafka {
    brokers: String,
    topic: String,
    producer: Arc<Mutex<Option<KafkaClient>>>,
}

impl Kafka {
    pub fn new(brokers: impl Into<String>, topic: impl Into<String>) -> Self {
        Self {
            brokers: brokers.into(),
            topic: topic.into(),
            producer: Arc::new(Mutex::new(None)),
        }
    }

    pub fn brokers(&self) -> &str {
        &self.brokers
    }

    pub fn topic(&self) -> &str {
        &self.topic
    }

    fn validate(&self) -> Result<(), SpiderError> {
        if self.brokers.trim().is_empty() {
            return Err(SpiderError::engine("kafka store brokers cannot be empty"));
        }

        if self.topic.trim().is_empty() {
            return Err(SpiderError::engine("kafka store topic cannot be empty"));
        }

        Ok(())
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
        let payload = serialize_item(item)?;
        let producer = self.producer().await?;
        producer.send_payload(&self.topic, payload).await
    }

    async fn batch_write(&self, items: &[Item], _spider_name: &str) -> Result<(), SpiderError> {
        if items.is_empty() {
            return Ok(());
        }

        let producer = self.producer().await?;
        for item in items {
            let payload = serialize_item(item)?;
            producer.send_payload(&self.topic, payload).await?;
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

    async fn send_payload(&self, topic: &str, payload: String) -> Result<(), SpiderError> {
        match self {
            Self::Real(producer) => producer
                .send(
                    FutureRecord::<(), _>::to(topic).payload(&payload),
                    Timeout::Never,
                )
                .await
                .map(|_| ())
                .map_err(|(error, _)| {
                    SpiderError::engine(format!("failed to deliver kafka store message: {error}"))
                }),
            #[cfg(test)]
            Self::Test(producer) => producer.send_payload(topic, payload).await,
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
    messages: Arc<Mutex<Vec<(String, String)>>>,
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

    async fn send_payload(&self, topic: &str, payload: String) -> Result<(), SpiderError> {
        if let Some(message) = self.error.lock().await.clone() {
            return Err(SpiderError::engine(format!(
                "failed to deliver kafka store message: {message}"
            )));
        }

        self.messages
            .lock()
            .await
            .push((topic.to_string(), payload));
        Ok(())
    }

    async fn messages(&self) -> Vec<(String, String)> {
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
                serde_json::json!({
                    "front_page": "01",
                    "period_date": "2026-03-31",
                })
                .to_string(),
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
                    serde_json::json!({"issue_key": "2026-03-31-front-01"}).to_string(),
                ),
                (
                    "period_items".to_string(),
                    serde_json::json!({"issue_key": "2026-03-30-front-01"}).to_string(),
                ),
            ]
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
