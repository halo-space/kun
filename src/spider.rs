use crate::error::SpiderError;
use crate::item::Item;
use crate::request::{Metadata, Request};
use crate::response::Response;
use crate::rules::Config as RulesConfig;
use crate::rules::{Compiled, CompiledStep, apply as apply_dsl};
use crate::value::Value;

/// Turn a method name into a callback string with a function-reference-like
/// syntax.
///
/// ```rust,ignore
/// // These three forms are equivalent:
/// request.with_callback(cb!(parse_detail))
/// request.with_callback(cb!(Self::parse_detail))
/// request.with_callback("parse_detail")
/// ```
///
/// Internally this is just `stringify!`, so there is no runtime cost.
/// It also makes callback references easier to search and less error-prone
/// than raw strings.
#[macro_export]
macro_rules! cb {
    (Self::$method:ident) => {
        stringify!($method)
    };
    ($method:ident) => {
        stringify!($method)
    };
}

/// Generate the `call()` callback dispatcher automatically instead of writing
/// a manual `match`.
///
/// ```rust,ignore
/// impl Spider for MySpider {
///     fn name(&self) -> &str { "my" }
///
///     async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
///         let req = response.follow(url)
///             .with_callback(cb!(Self::parse_detail));
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

/// Generate the `handle_error()` errback dispatcher automatically instead of
/// writing a manual `match`.
///
/// ```rust,ignore
/// impl Spider for MySpider {
///     fn name(&self) -> &str { "my" }
///
///     spider_errbacks!(handle_detail_error);
/// }
///
/// impl MySpider {
///     async fn handle_detail_error(
///         &self,
///         failure: &halo_spider::spider::Failure,
///     ) -> Result<halo_spider::spider::Output, halo_spider::error::SpiderError> { ... }
/// }
/// ```
#[macro_export]
macro_rules! spider_errbacks {
    ($($method:ident),+ $(,)?) => {
        async fn handle_error(
            &self,
            name: &str,
            failure: &$crate::spider::Failure,
        ) -> Result<$crate::spider::Output, $crate::error::SpiderError> {
            match name {
                $(stringify!($method) => self.$method(failure).await,)+
                other => Err($crate::error::SpiderError::engine(
                    format!("unknown errback: {other}"),
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

#[derive(Debug, Clone)]
pub struct Failure {
    pub request: Request,
    pub response: Option<Response>,
    pub error: SpiderError,
}

impl Failure {
    pub fn new(request: Request, response: Option<Response>, error: SpiderError) -> Self {
        Self {
            request,
            response,
            error,
        }
    }

    pub fn kwargs(&self) -> &Metadata {
        &self.request.kwargs
    }

    pub fn kwarg(&self, key: &str) -> Option<&Value> {
        self.request.kwargs.get(key)
    }
}

#[allow(async_fn_in_trait)]
pub trait Spider: Send + Sync {
    fn name(&self) -> &str;

    fn start_urls(&self) -> Vec<String> {
        Vec::new()
    }

    /// Build start URLs dynamically when needed.
    /// By default this returns `start_urls()`.
    fn build_start_urls(&self) -> Vec<String> {
        self.start_urls()
    }

    /// Build full start requests when the spider needs request-level
    /// capabilities such as cookies, proxy, session, or browser mode.
    ///
    /// By default this maps `build_start_urls()` into `Request::new(...)`.
    fn build_start_requests(&self) -> Vec<Request> {
        self.build_start_urls()
            .into_iter()
            .map(Request::new)
            .collect()
    }

    /// Allowed crawl domains. An empty list means no domain filter.
    /// The engine filters requests before enqueueing them.
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

    async fn handle_error(&self, name: &str, _failure: &Failure) -> Result<Output, SpiderError> {
        Err(SpiderError::engine(format!("unknown errback: {name}")))
    }

    async fn dispatch(
        &self,
        response: &Response,
        compiled: Option<&Compiled>,
    ) -> Result<Output, SpiderError> {
        // Prefer an explicit request callback when one is present.
        if let Some(request) = &response.request
            && let Some(callback_target) = &request.callback
        {
            return self.call(&callback_target.name, response).await;
        }

        let Some(step) = resolve_step(response, compiled)? else {
            return self.parse(response).await;
        };

        if let Some(callback) = &step.callback {
            self.call(callback, response).await
        } else {
            let output = apply_dsl(response, step, compiled.unwrap()).await?;
            Ok(Output {
                items: output.items,
                requests: output.requests,
            })
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
