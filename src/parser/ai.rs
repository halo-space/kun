use crate::parser::query::{Kind, ValueQuery};
use jiff::SignedDuration;

#[derive(Debug, Clone, PartialEq)]
pub struct AiQuery {
    pub input: String,
    pub prompt: String,
    pub source: Option<String>,
    pub value: ValueQuery,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: String,
    pub max_retries: u32,
    pub timeout: SignedDuration,
}

impl Default for AiQuery {
    fn default() -> Self {
        Self {
            input: String::new(),
            prompt: String::new(),
            source: None,
            value: ValueQuery::default(),
            api_key: None,
            base_url: None,
            model: "gpt-4o-mini".to_string(),
            max_retries: 3,
            timeout: SignedDuration::from_secs(30),
        }
    }
}

impl AiQuery {
    pub fn new(
        input: impl Into<String>,
        prompt: impl Into<String>,
        source: Option<String>,
    ) -> Self {
        let source_value = source.unwrap_or_else(|| "html".to_string());
        Self {
            input: input.into(),
            prompt: prompt.into(),
            source: Some(source_value.clone()),
            value: ValueQuery::new(Kind::Ai, source_value),
            ..Default::default()
        }
    }

    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    pub fn with_timeout(mut self, timeout: SignedDuration) -> Self {
        self.timeout = non_negative_duration(timeout);
        self
    }

    pub fn with_config(
        mut self,
        api_key: Option<String>,
        base_url: Option<String>,
        model: String,
    ) -> Self {
        self.api_key = api_key;
        self.base_url = base_url;
        self.model = model;
        self
    }

    pub async fn execute(&mut self) -> Result<(), String> {
        #[cfg(feature = "ai-selector")]
        {
            use crate::value::Value;
            let result = self.execute_with_retry().await?;
            self.value = self.value.clone().with_values(vec![Value::String(result)]);
            Ok(())
        }
        #[cfg(not(feature = "ai-selector"))]
        {
            Err("ai-selector feature not enabled".to_string())
        }
    }

    #[cfg(feature = "ai-selector")]
    async fn execute_with_retry(&self) -> Result<String, String> {
        use tokio::time::{sleep, timeout};

        let timeout_duration = std::time::Duration::try_from(self.timeout)
            .map_err(|e| format!("invalid AI timeout: {e}"))?;
        let mut attempt = 0;
        loop {
            let result = timeout(
                timeout_duration,
                openai(
                    &self.api_key,
                    &self.base_url,
                    &self.model,
                    &self.prompt,
                    &self.input,
                ),
            )
            .await;

            match result {
                Ok(Ok(text)) => return Ok(text),
                Ok(Err(e)) if is_retryable(&e) && attempt < self.max_retries => {
                    attempt += 1;
                    tracing::warn!(
                        "AI request failed (attempt {}/{}): {}",
                        attempt,
                        self.max_retries + 1,
                        e
                    );
                    sleep(
                        std::time::Duration::try_from(SignedDuration::from_secs(
                            2i64.pow(attempt - 1),
                        ))
                        .map_err(|e| format!("invalid AI backoff: {e}"))?,
                    )
                    .await;
                }
                Ok(Err(e)) => return Err(e),
                Err(_) if attempt < self.max_retries => {
                    attempt += 1;
                    tracing::warn!(
                        "AI request timeout (attempt {}/{})",
                        attempt,
                        self.max_retries + 1
                    );
                    sleep(
                        std::time::Duration::try_from(SignedDuration::from_secs(
                            2i64.pow(attempt - 1),
                        ))
                        .map_err(|e| format!("invalid AI backoff: {e}"))?,
                    )
                    .await;
                }
                Err(_) => return Err("Request timeout".to_string()),
            }
        }
    }

    pub fn one(&self) -> Option<String> {
        self.value.one()
    }

    pub fn all(&self) -> Vec<String> {
        self.value.all()
    }

    pub fn value(&self) -> Option<crate::value::Value> {
        self.value.value()
    }
}

fn non_negative_duration(duration: SignedDuration) -> SignedDuration {
    if duration.is_negative() {
        SignedDuration::ZERO
    } else {
        duration
    }
}

#[cfg(feature = "ai-selector")]
async fn openai(
    api_key: &Option<String>,
    base_url: &Option<String>,
    model: &str,
    prompt: &str,
    content: &str,
) -> Result<String, String> {
    use async_openai::{Client, config::OpenAIConfig, types::responses::CreateResponseArgs};

    let api_key = api_key.as_ref().ok_or("OPENAI_API_KEY not configured")?;

    let mut config = OpenAIConfig::new().with_api_key(api_key);
    if let Some(url) = base_url {
        config = config.with_api_base(url);
    }
    let client = Client::with_config(config);

    let request = CreateResponseArgs::default()
        .model(model)
        .input(format!("Content:\n{}\n\nInstruction: {}", content, prompt))
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    let response = client
        .responses()
        .create(request)
        .await
        .map_err(|e| format!("OpenAI API error: {}", e))?;

    response
        .output_text()
        .ok_or_else(|| "No content in response".to_string())
}

#[cfg(feature = "ai-selector")]
fn is_retryable(error: &str) -> bool {
    error.contains("timeout")
        || error.contains("connection")
        || error.contains("network")
        || error.contains("500")
        || error.contains("502")
        || error.contains("503")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_query_uses_html_source_by_default() {
        let query = AiQuery::new("<h1>Title</h1>", "extract title", None);

        assert_eq!(query.source.as_deref(), Some("html"));
        assert!(query.value.trim);
    }

    #[test]
    fn ai_query_has_default_retry_and_timeout() {
        let query = AiQuery::new("content", "prompt", None);

        assert_eq!(query.max_retries, 3);
        assert_eq!(query.timeout, SignedDuration::from_secs(30));
    }

    #[test]
    fn ai_query_can_configure_retry_and_timeout() {
        let query = AiQuery::new("content", "prompt", None)
            .with_max_retries(5)
            .with_timeout(SignedDuration::from_secs(60));

        assert_eq!(query.max_retries, 5);
        assert_eq!(query.timeout, SignedDuration::from_secs(60));
    }

    #[cfg(feature = "ai-selector")]
    #[test]
    fn is_retryable_identifies_network_errors() {
        assert!(is_retryable("connection timeout"));
        assert!(is_retryable("network error"));
        assert!(is_retryable("500 internal server error"));
        assert!(is_retryable("502 bad gateway"));
        assert!(is_retryable("503 service unavailable"));

        assert!(!is_retryable("401 unauthorized"));
        assert!(!is_retryable("invalid api key"));
    }
}
