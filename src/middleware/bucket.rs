use crate::error::SpiderError;
use crate::request::Request;
use crate::value::Value;
use std::collections::BTreeMap;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BucketToken {
    Spider,
    Origin,
    Domain,
    Url,
    Method,
    Meta(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketSpec {
    pub tokens: Vec<BucketToken>,
}

impl Default for BucketSpec {
    fn default() -> Self {
        Self::origin()
    }
}

impl BucketSpec {
    pub fn origin() -> Self {
        Self {
            tokens: vec![BucketToken::Origin],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BucketKey(String);

impl BucketKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

pub fn options_signature(options: &BTreeMap<String, Value>) -> Result<String, SpiderError> {
    serde_json::to_string(options).map_err(|error| {
        SpiderError::engine(format!("middleware options signature failed: {error}"))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketConfig {
    policy: String,
    spec: BucketSpec,
}

impl BucketConfig {
    pub fn from_options(
        options: &BTreeMap<String, Value>,
        default_policy: &str,
    ) -> Result<Self, SpiderError> {
        let policy = options
            .get("policy")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(default_policy)
            .to_string();

        Ok(Self {
            policy,
            spec: parse_bucket_spec(options.get("bucket"))?,
        })
    }

    pub fn resolve(&self, spider_name: Option<&str>, request: &Request) -> BucketKey {
        let mut parts = Vec::with_capacity(self.spec.tokens.len() + 1);
        parts.push(format!("policy={}", self.policy));

        for token in &self.spec.tokens {
            parts.push(resolve_token(spider_name, request, token));
        }

        BucketKey::new(parts.join("|"))
    }
}

fn parse_bucket_spec(value: Option<&Value>) -> Result<BucketSpec, SpiderError> {
    let Some(value) = value else {
        return Ok(BucketSpec::default());
    };

    match value {
        Value::String(token) => Ok(BucketSpec {
            tokens: vec![parse_bucket_token(token)?],
        }),
        Value::Array(values) => {
            let mut tokens = Vec::with_capacity(values.len());
            for value in values {
                let token = value.as_str().ok_or_else(|| {
                    SpiderError::engine("middleware bucket array values must be strings")
                })?;
                tokens.push(parse_bucket_token(token)?);
            }

            if tokens.is_empty() {
                return Err(SpiderError::engine(
                    "middleware bucket array must contain at least one token",
                ));
            }

            Ok(BucketSpec { tokens })
        }
        _ => Err(SpiderError::engine(
            "middleware bucket must be a string or array of strings",
        )),
    }
}

fn parse_bucket_token(value: &str) -> Result<BucketToken, SpiderError> {
    match value {
        "spider" => Ok(BucketToken::Spider),
        "origin" => Ok(BucketToken::Origin),
        "domain" => Ok(BucketToken::Domain),
        "url" => Ok(BucketToken::Url),
        "method" => Ok(BucketToken::Method),
        other if other.starts_with("meta.") => Ok(BucketToken::Meta(
            other.trim_start_matches("meta.").to_string(),
        )),
        other => Err(SpiderError::engine(format!(
            "unsupported middleware bucket token: {other}"
        ))),
    }
}

fn resolve_token(spider_name: Option<&str>, request: &Request, token: &BucketToken) -> String {
    match token {
        BucketToken::Spider => format!("spider={}", spider_name.unwrap_or_default()),
        BucketToken::Origin => format!("origin={}", request_origin(request.url.as_str())),
        BucketToken::Domain => format!("domain={}", request_domain(request.url.as_str())),
        BucketToken::Url => format!("url={}", request.url),
        BucketToken::Method => format!("method={}", request.method),
        BucketToken::Meta(key) => format!(
            "meta.{key}={}",
            request
                .meta
                .get(key)
                .map(value_to_key_string)
                .unwrap_or_default()
        ),
    }
}

fn value_to_key_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => serde_json::Value::from(value.clone()).to_string(),
    }
}

fn request_origin(url: &str) -> String {
    let Ok(parsed_url) = Url::parse(url) else {
        return url.to_string();
    };
    let Some(host) = parsed_url.host_str() else {
        return url.to_string();
    };

    let mut origin = format!("{}://{}", parsed_url.scheme(), host);
    if let Some(port) = parsed_url.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    origin
}

fn request_domain(url: &str) -> String {
    let Ok(parsed_url) = Url::parse(url) else {
        return url.to_string();
    };

    parsed_url.host_str().unwrap_or(url).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;

    #[test]
    fn key_defaults_to_origin() {
        let config = BucketConfig::from_options(&BTreeMap::new(), "concurrency").unwrap();
        let request = Request::new("https://example.com/path");

        assert_eq!(
            config.resolve(None, &request),
            BucketKey::new("policy=concurrency|origin=https://example.com")
        );
    }

    #[test]
    fn key_supports_composite_tokens() {
        let config = BucketConfig::from_options(
            &BTreeMap::from([(
                "bucket".to_string(),
                Value::Array(vec![
                    Value::String("origin".to_string()),
                    Value::String("meta.channel".to_string()),
                ]),
            )]),
            "rate_limit",
        )
        .unwrap();
        let request = Request::new("https://example.com/path")
            .with_meta("channel", Value::String("finance".to_string()));

        assert_eq!(
            config.resolve(None, &request),
            BucketKey::new("policy=rate_limit|origin=https://example.com|meta.channel=finance")
        );
    }
}
