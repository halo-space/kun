use crate::error::SpiderError;
use crate::rules::compile::compile_rules;
use crate::rules::schema::{Compiled, Config};
use crate::rules::source::Source;

pub async fn load(config: &Config) -> Result<Compiled, SpiderError> {
    let source: Box<dyn Source> = match config.r#type.as_str() {
        "local" => Box::new(crate::rules::local::Source),
        "inline" => Box::new(crate::rules::inline::Source),
        other => {
            return Err(SpiderError::rules(format!(
                "unsupported rules source type: {other}"
            )))
        }
    };

    let value = source.load(config).await?;
    compile_rules(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use std::collections::BTreeMap;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::pin::Pin;
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct NoopWake;
        impl Wake for NoopWake {
            fn wake(self: Arc<Self>) {}
        }

        let waker = Waker::from(Arc::new(NoopWake));
        let mut future = Pin::from(Box::new(future));
        let mut context = Context::from_waker(&waker);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    #[test]
    fn load_inline_rules_compiles_successfully() {
        let config = Config {
            r#type: "inline".to_string(),
            options: [(
                "value".to_string(),
                Value::String(
                    r#"{"steps":[{"id":"parse","impl":"dsl","parse":{"fields":[{"name":"title","source":"html","selector_type":"css","selector":["h1"],"attribute":"text"}]}}]}"#.to_string(),
                ),
            )]
            .into_iter()
            .collect(),
        };

        let compiled = block_on(load(&config)).unwrap();

        assert_eq!(compiled.steps.len(), 1);
        assert_eq!(compiled.steps[0].id, "parse");
    }

    #[test]
    fn load_unsupported_type_returns_error() {
        let config = Config {
            r#type: "redis".to_string(),
            options: BTreeMap::new(),
        };

        let err = block_on(load(&config)).unwrap_err();
        assert!(err.to_string().contains("unsupported rules source type"));
    }
}
