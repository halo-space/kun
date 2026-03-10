use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::item::Item;
use crate::middleware::Map as MiddlewareMap;
use crate::request::Request;
use crate::response::Response;
use crate::runtime::Config as RuntimeConfig;
use crate::rules::{apply as apply_dsl, Compiled, CompiledStep, StepImpl};

#[derive(Debug, Default)]
pub struct Output {
    pub items: Vec<Item>,
    pub requests: Vec<Request>,
}

impl Output {
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            requests: Vec::new(),
        }
    }
}

pub trait Spider: Send + Sync {
    fn name(&self) -> &str;

    fn start_urls(&self) -> Vec<String> {
        Vec::new()
    }

    fn runtime(&self) -> RuntimeConfig {
        RuntimeConfig::default()
    }

    fn middlewares(&self) -> MiddlewareMap {
        MiddlewareMap::new()
    }

    fn parse<'a>(&'a self, _response: &'a Response) -> BoxFuture<'a, Result<Output, SpiderError>> {
        Box::pin(async { Ok(Output::empty()) })
    }

    fn call<'a>(
        &'a self,
        name: &'a str,
        response: &'a Response,
    ) -> BoxFuture<'a, Result<Output, SpiderError>> {
        match name {
            "parse" => self.parse(response),
            other => Box::pin(async move {
                Err(SpiderError::engine(format!("unknown callback: {other}")))
            }),
        }
    }

    fn dispatch<'a>(
        &'a self,
        response: &'a Response,
        compiled: Option<&'a Compiled>,
    ) -> BoxFuture<'a, Result<Output, SpiderError>> {
        Box::pin(async move {
            let Some(step) = resolve_step(response, compiled)? else {
                return self.parse(response).await;
            };

            match step.r#impl {
                StepImpl::Dsl => {
                    let output = apply_dsl(response, step)?;
                    Ok(Output {
                        items: output.items,
                        requests: output.requests,
                    })
                }
                StepImpl::Code => {
                    let callback = step.callback.as_deref().ok_or_else(|| {
                        SpiderError::engine(format!("code step {} missing callback", step.id))
                    })?;
                    self.call(callback, response).await
                }
            }
        })
    }
}

fn resolve_step<'a>(
    response: &Response,
    compiled: Option<&'a Compiled>,
) -> Result<Option<&'a CompiledStep>, SpiderError> {
    let Some(compiled) = compiled else {
        return Ok(None);
    };

    let step_id = response
        .meta
        .get("next_step")
        .and_then(|value| value.as_str())
        .unwrap_or("parse");

    compiled
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .map(Some)
        .ok_or_else(|| SpiderError::engine(format!("step not found: {step_id}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::compile::compile_rules;
    use crate::value::Value;

    struct TestSpider;

    impl Spider for TestSpider {
        fn name(&self) -> &str {
            "test"
        }

        fn parse<'a>(&'a self, response: &'a Response) -> BoxFuture<'a, Result<Output, SpiderError>> {
            let title = response.css("h1.title::text").one().unwrap_or_default();
            Box::pin(async move {
                Ok(Output {
                    items: vec![Item::new().with_field("title", Value::String(title))],
                    requests: Vec::new(),
                })
            })
        }

        fn call<'a>(
            &'a self,
            name: &'a str,
            response: &'a Response,
        ) -> BoxFuture<'a, Result<Output, SpiderError>> {
            match name {
                "parse" => self.parse(response),
                "parse_detail" => Box::pin(async move {
                    Ok(Output {
                        items: vec![Item::new().with_field(
                            "detail",
                            Value::String(response.url.clone()),
                        )],
                        requests: Vec::new(),
                    })
                }),
                other => Box::pin(async move {
                    Err(SpiderError::engine(format!("unknown callback: {other}")))
                }),
            }
        }
    }

    #[test]
    fn spider_dispatch_uses_dsl_step() {
        let spider = TestSpider;
        let compiled = compile_rules(Value::String(
            r#"{
                "steps":[
                    {
                        "id":"parse",
                        "impl":"dsl",
                        "parse":{
                            "fields":[
                                {
                                    "name":"title",
                                    "source":"html",
                                    "selector_type":"css",
                                    "selector":["h1.title"],
                                    "attribute":"text"
                                }
                            ]
                        }
                    }
                ]
            }"#
            .to_string(),
        ))
        .unwrap();

        let response = Response::new(
            "https://example.com",
            200,
            Default::default(),
            br#"<h1 class="title">Hello</h1>"#.to_vec(),
        );

        let output = block_on(spider.dispatch(&response, Some(&compiled))).unwrap();

        assert_eq!(
            output.items[0].get("title"),
            Some(&Value::String("Hello".to_string()))
        );
    }

    #[test]
    fn spider_dispatch_uses_code_step_callback() {
        let spider = TestSpider;
        let compiled = compile_rules(Value::String(
            r#"{
                "steps":[
                    {
                        "id":"detail",
                        "impl":"code",
                        "callback":"parse_detail"
                    }
                ]
            }"#
            .to_string(),
        ))
        .unwrap();

        let response = Response::from_request(
            Request::new("https://example.com/detail")
                .with_meta("next_step", Value::String("detail".to_string())),
            200,
            Default::default(),
            Vec::new(),
        );

        let output = block_on(spider.dispatch(&response, Some(&compiled))).unwrap();

        assert_eq!(
            output.items[0].get("detail"),
            Some(&Value::String("https://example.com/detail".to_string()))
        );
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::pin::Pin;
        use std::sync::Arc;
        use std::task::{Context, Poll, Waker};

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

    struct NoopWake;

    impl std::task::Wake for NoopWake {
        fn wake(self: std::sync::Arc<Self>) {}
    }
}
