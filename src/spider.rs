use crate::error::SpiderError;
use crate::item::Item;
use crate::request::{Metadata, Request};
use crate::response::Response;
use crate::rules::Config as RulesConfig;
use crate::rules::{Compiled, CompiledStep, apply as apply_dsl};

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
///     async fn parse(&self, response: &Response) -> Result<Request, SpiderError> {
///         let req = response.follow(url)
///             .with_callback(cb!(Self::parse_detail));
///         Ok(req)
///     }
///
///     spider_callbacks!(parse_detail, parse_comment);
/// }
///
/// impl MySpider {
///     async fn parse_detail(&self, r: &Response) -> Result<Item, SpiderError> { ... }
///     async fn parse_comment(&self, r: &Response) -> Result<Vec<Request>, SpiderError> { ... }
/// }
/// ```
#[macro_export]
macro_rules! spider_callbacks {
    ($($method:ident),+ $(,)?) => {
        async fn call(
            &self,
            name: &str,
            response: &$crate::response::Response,
        ) -> Result<impl $crate::spider::IntoSpiderResultParts, $crate::error::SpiderError> {
            match name {
                $(
                    stringify!($method) => Ok($crate::spider::into_spider_result_parts(
                        self.$method(response).await?,
                    )),
                )+
                "parse" => Ok($crate::spider::into_spider_result_parts(
                    self.parse(response).await?,
                )),
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
///     ) -> Result<halo_spider::request::Request, halo_spider::error::SpiderError> { ... }
/// }
/// ```
#[macro_export]
macro_rules! spider_errbacks {
    ($($method:ident),+ $(,)?) => {
        async fn handle_error(
            &self,
            name: &str,
            failure: &$crate::spider::Failure,
        ) -> Result<impl $crate::spider::IntoSpiderResultParts, $crate::error::SpiderError> {
            match name {
                $(
                    stringify!($method) => Ok($crate::spider::into_spider_result_parts(
                        self.$method(failure).await?,
                    )),
                )+
                other => Err($crate::error::SpiderError::engine(
                    format!("unknown errback: {other}"),
                )),
            }
        }
    };
}

#[doc(hidden)]
pub type SpiderResultParts = (Vec<Item>, Vec<Request>);

#[doc(hidden)]
pub trait IntoSpiderItems {
    fn into_items(self) -> Vec<Item>;
}

#[doc(hidden)]
pub trait IntoSpiderRequests {
    fn into_requests(self) -> Vec<Request>;
}

#[doc(hidden)]
pub trait IntoSpiderResultParts {
    fn into_parts(self) -> SpiderResultParts;
}

impl IntoSpiderItems for () {
    fn into_items(self) -> Vec<Item> {
        Vec::new()
    }
}

impl IntoSpiderItems for Item {
    fn into_items(self) -> Vec<Item> {
        vec![self]
    }
}

impl IntoSpiderItems for Vec<Item> {
    fn into_items(self) -> Vec<Item> {
        self
    }
}

impl IntoSpiderRequests for () {
    fn into_requests(self) -> Vec<Request> {
        Vec::new()
    }
}

impl IntoSpiderRequests for Request {
    fn into_requests(self) -> Vec<Request> {
        vec![self]
    }
}

impl IntoSpiderRequests for Vec<Request> {
    fn into_requests(self) -> Vec<Request> {
        self
    }
}

impl IntoSpiderResultParts for () {
    fn into_parts(self) -> SpiderResultParts {
        (Vec::new(), Vec::new())
    }
}

impl IntoSpiderResultParts for Request {
    fn into_parts(self) -> SpiderResultParts {
        (Vec::new(), vec![self])
    }
}

impl IntoSpiderResultParts for Vec<Request> {
    fn into_parts(self) -> SpiderResultParts {
        (Vec::new(), self)
    }
}

impl IntoSpiderResultParts for Item {
    fn into_parts(self) -> SpiderResultParts {
        (vec![self], Vec::new())
    }
}

impl IntoSpiderResultParts for Vec<Item> {
    fn into_parts(self) -> SpiderResultParts {
        (self, Vec::new())
    }
}

impl<I, R> IntoSpiderResultParts for (I, R)
where
    I: IntoSpiderItems,
    R: IntoSpiderRequests,
{
    fn into_parts(self) -> SpiderResultParts {
        (self.0.into_items(), self.1.into_requests())
    }
}

#[doc(hidden)]
pub fn into_spider_result_parts<T: IntoSpiderResultParts>(value: T) -> SpiderResultParts {
    value.into_parts()
}

#[derive(Debug, Default)]
pub(crate) struct CallbackOutput {
    pub(crate) items: Vec<Item>,
    pub(crate) requests: Vec<Request>,
}

impl CallbackOutput {
    pub(crate) fn from_parts((items, requests): SpiderResultParts) -> Self {
        Self { items, requests }
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

    pub fn cb_kwargs(&self) -> &Metadata {
        &self.request.cb_kwargs
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

    fn validator(&self) -> Option<crate::validator::StepValidator> {
        None
    }

    async fn parse(&self, _response: &Response) -> Result<impl IntoSpiderResultParts, SpiderError> {
        Ok(())
    }

    async fn call(
        &self,
        name: &str,
        response: &Response,
    ) -> Result<impl IntoSpiderResultParts, SpiderError> {
        match name {
            "parse" => Ok(into_spider_result_parts(self.parse(response).await?)),
            other => Err(SpiderError::engine(format!("unknown callback: {other}"))),
        }
    }

    async fn handle_error(
        &self,
        name: &str,
        _failure: &Failure,
    ) -> Result<impl IntoSpiderResultParts, SpiderError> {
        Result::<SpiderResultParts, SpiderError>::Err(SpiderError::engine(format!(
            "unknown errback: {name}"
        )))
    }

    async fn dispatch(
        &self,
        response: &Response,
        compiled: Option<&Compiled>,
    ) -> Result<impl IntoSpiderResultParts, SpiderError> {
        // Prefer an explicit request callback when one is present.
        if let Some(request) = &response.request
            && let Some(callback_target) = &request.callback
        {
            return Ok(into_spider_result_parts(
                self.call(&callback_target.name, response).await?,
            ));
        }

        let Some(step) = resolve_step(response, compiled)? else {
            return Ok(into_spider_result_parts(self.parse(response).await?));
        };

        if let Some(callback) = &step.callback {
            Ok(into_spider_result_parts(
                self.call(callback, response).await?,
            ))
        } else {
            let output = apply_dsl(response, step, compiled.unwrap()).await?;
            Ok((output.items, output.requests))
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

    compiled.step_from_meta(&response.meta).map(Some)
}
