use crate::error::SpiderError;
use crate::item::Item;
use crate::request::Request;
use crate::response::Response;
use crate::rules::Config as RulesConfig;
use crate::rules::{apply as apply_dsl, Compiled, CompiledStep, StepImpl};

/// 将方法名转为回调字符串，写法接近函数引用。
///
/// ```rust,ignore
/// // 以下三种写法完全等价：
/// request.with_callback(cb!(parse_detail))      // 像传函数引用
/// request.with_callback(cb!(Self::parse_detail)) // 也支持 Self:: 前缀
/// request.with_callback("parse_detail")          // 直接写字符串
/// ```
///
/// 底层就是 `stringify!`，编译期零开销。
/// 好处：拼写错误会在 `call()` 路由时报错而非静默忽略，
/// 且代码搜索 `cb!(parse_detail)` 比搜字符串更精确。
#[macro_export]
macro_rules! cb {
    (Self::$method:ident) => { stringify!($method) };
    ($method:ident) => { stringify!($method) };
}

/// 自动生成 `call()` 的路由分发，免去手写 match。
///
/// ```rust,ignore
/// impl Spider for MySpider {
///     fn name(&self) -> &str { "my" }
///
///     async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
///         let req = response.follow(url)
///             .with_callback(cb!(Self::parse_detail));  // 像传函数引用
///         Ok(Output { items: vec![], requests: vec![req] })
///     }
///
///     spider_callbacks!(parse, parse_detail, parse_comment);
/// }
///
/// impl MySpider {
///     async fn parse_detail(&self, r: &Response) -> Result<Output, SpiderError> { ... }
///     async fn parse_comment(&self, r: &Response) -> Result<Output, SpiderError> { ... }
/// }
/// ```
#[macro_export]
macro_rules! spider_callbacks {
    ($($method:ident),+ $(,)?) => {
        async fn call(
            &self,
            name: &str,
            response: &$crate::response::Response,
        ) -> Result<$crate::spider::Output, $crate::error::SpiderError> {
            match name {
                $(stringify!($method) => self.$method(response).await,)+
                other => Err($crate::error::SpiderError::engine(
                    format!("unknown callback: {other}"),
                )),
            }
        }
    };
}

#[derive(Debug, Default)]
pub struct Output {
    pub items: Vec<Item>,
    pub requests: Vec<Request>,
}

impl Output {
    pub fn empty() -> Self {
        Self::default()
    }
}

#[allow(async_fn_in_trait)]
pub trait Spider: Send + Sync {
    fn name(&self) -> &str;

    fn start_urls(&self) -> Vec<String> {
        Vec::new()
    }

    /// 允许爬取的域名列表。空列表表示不限制。
    /// 引擎会在入队前过滤不属于这些域名的请求。
    fn allowed_domains(&self) -> Vec<String> {
        Vec::new()
    }

    fn rules(&self) -> Option<RulesConfig> {
        None
    }

    async fn parse(&self, _response: &Response) -> Result<Output, SpiderError> {
        Ok(Output::empty())
    }

    async fn call(&self, name: &str, response: &Response) -> Result<Output, SpiderError> {
        match name {
            "parse" => self.parse(response).await,
            other => Err(SpiderError::engine(format!("unknown callback: {other}"))),
        }
    }

    async fn dispatch(
        &self,
        response: &Response,
        compiled: Option<&Compiled>,
    ) -> Result<Output, SpiderError> {
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

        async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
            let title = response.css("h1.title::text").one().unwrap_or_default();
            Ok(Output {
                items: vec![Item::new().with_field("title", Value::String(title))],
                requests: Vec::new(),
            })
        }

        spider_callbacks!(parse, parse_detail);
    }

    impl TestSpider {
        async fn parse_detail(&self, response: &Response) -> Result<Output, SpiderError> {
            Ok(Output {
                items: vec![Item::new().with_field(
                    "detail",
                    Value::String(response.url.clone()),
                )],
                requests: Vec::new(),
            })
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
