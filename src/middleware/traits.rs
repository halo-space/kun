use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::response::Response;
use std::future::Future;
use std::pin::Pin;

#[allow(async_fn_in_trait)]
pub trait Middleware: Send + Sync {
    async fn before_enqueue(
        &self,
        _context: &mut context::Enqueue,
    ) -> Result<flow::Enqueue, SpiderError> {
        Ok(flow::Enqueue::Continue)
    }

    async fn after_enqueue(&self, _context: &mut context::Enqueue) -> Result<(), SpiderError> {
        Ok(())
    }

    async fn before_download(
        &self,
        _context: &mut context::Download,
    ) -> Result<flow::Download, SpiderError> {
        Ok(flow::Download::Continue)
    }

    async fn after_download(
        &self,
        _context: &mut context::Download,
        _response: &mut Response,
    ) -> Result<flow::Download, SpiderError> {
        Ok(flow::Download::Continue)
    }

    async fn download_error(
        &self,
        _context: &mut context::Download,
        _error: &SpiderError,
    ) -> Result<flow::Download, SpiderError> {
        Ok(flow::Download::Continue)
    }

    async fn before_parse(
        &self,
        _context: &mut context::Parse,
    ) -> Result<flow::Parse, SpiderError> {
        Ok(flow::Parse::Continue)
    }

    async fn parse_error(
        &self,
        _context: &mut context::Parse,
        _error: &SpiderError,
    ) -> Result<flow::Parse, SpiderError> {
        Ok(flow::Parse::Continue)
    }

    async fn after_parse(&self, _context: &mut context::Parse) -> Result<(), SpiderError> {
        Ok(())
    }

    async fn before_item(&self, _context: &mut context::Item) -> Result<flow::Item, SpiderError> {
        Ok(flow::Item::Continue)
    }

    async fn after_item(&self, _context: &mut context::Item) -> Result<(), SpiderError> {
        Ok(())
    }
}

impl<T> Middleware for Box<T>
where
    T: Middleware + ?Sized,
{
    async fn before_enqueue(
        &self,
        context: &mut context::Enqueue,
    ) -> Result<flow::Enqueue, SpiderError> {
        (**self).before_enqueue(context).await
    }

    async fn after_enqueue(&self, context: &mut context::Enqueue) -> Result<(), SpiderError> {
        (**self).after_enqueue(context).await
    }

    async fn before_download(
        &self,
        context: &mut context::Download,
    ) -> Result<flow::Download, SpiderError> {
        (**self).before_download(context).await
    }

    async fn after_download(
        &self,
        context: &mut context::Download,
        response: &mut Response,
    ) -> Result<flow::Download, SpiderError> {
        (**self).after_download(context, response).await
    }

    async fn download_error(
        &self,
        context: &mut context::Download,
        error: &SpiderError,
    ) -> Result<flow::Download, SpiderError> {
        (**self).download_error(context, error).await
    }

    async fn before_parse(&self, context: &mut context::Parse) -> Result<flow::Parse, SpiderError> {
        (**self).before_parse(context).await
    }

    async fn parse_error(
        &self,
        context: &mut context::Parse,
        error: &SpiderError,
    ) -> Result<flow::Parse, SpiderError> {
        (**self).parse_error(context, error).await
    }

    async fn after_parse(&self, context: &mut context::Parse) -> Result<(), SpiderError> {
        (**self).after_parse(context).await
    }

    async fn before_item(&self, context: &mut context::Item) -> Result<flow::Item, SpiderError> {
        (**self).before_item(context).await
    }

    async fn after_item(&self, context: &mut context::Item) -> Result<(), SpiderError> {
        (**self).after_item(context).await
    }
}

pub(crate) trait MiddlewareObject: Send + Sync {
    fn before_enqueue<'a>(
        &'a self,
        context: &'a mut context::Enqueue,
    ) -> MiddlewareFuture<'a, Result<flow::Enqueue, SpiderError>>;

    fn after_enqueue<'a>(
        &'a self,
        context: &'a mut context::Enqueue,
    ) -> MiddlewareFuture<'a, Result<(), SpiderError>>;

    fn before_download<'a>(
        &'a self,
        context: &'a mut context::Download,
    ) -> MiddlewareFuture<'a, Result<flow::Download, SpiderError>>;

    fn after_download<'a>(
        &'a self,
        context: &'a mut context::Download,
        response: &'a mut Response,
    ) -> MiddlewareFuture<'a, Result<flow::Download, SpiderError>>;

    fn download_error<'a>(
        &'a self,
        context: &'a mut context::Download,
        error: &'a SpiderError,
    ) -> MiddlewareFuture<'a, Result<flow::Download, SpiderError>>;

    fn before_parse<'a>(
        &'a self,
        context: &'a mut context::Parse,
    ) -> MiddlewareFuture<'a, Result<flow::Parse, SpiderError>>;

    fn parse_error<'a>(
        &'a self,
        context: &'a mut context::Parse,
        error: &'a SpiderError,
    ) -> MiddlewareFuture<'a, Result<flow::Parse, SpiderError>>;

    fn after_parse<'a>(
        &'a self,
        context: &'a mut context::Parse,
    ) -> MiddlewareFuture<'a, Result<(), SpiderError>>;

    fn before_item<'a>(
        &'a self,
        context: &'a mut context::Item,
    ) -> MiddlewareFuture<'a, Result<flow::Item, SpiderError>>;

    fn after_item<'a>(
        &'a self,
        context: &'a mut context::Item,
    ) -> MiddlewareFuture<'a, Result<(), SpiderError>>;
}

impl<T> MiddlewareObject for T
where
    T: Middleware + 'static,
{
    fn before_enqueue<'a>(
        &'a self,
        context: &'a mut context::Enqueue,
    ) -> MiddlewareFuture<'a, Result<flow::Enqueue, SpiderError>> {
        Box::pin(<T as Middleware>::before_enqueue(self, context))
    }

    fn after_enqueue<'a>(
        &'a self,
        context: &'a mut context::Enqueue,
    ) -> MiddlewareFuture<'a, Result<(), SpiderError>> {
        Box::pin(<T as Middleware>::after_enqueue(self, context))
    }

    fn before_download<'a>(
        &'a self,
        context: &'a mut context::Download,
    ) -> MiddlewareFuture<'a, Result<flow::Download, SpiderError>> {
        Box::pin(<T as Middleware>::before_download(self, context))
    }

    fn after_download<'a>(
        &'a self,
        context: &'a mut context::Download,
        response: &'a mut Response,
    ) -> MiddlewareFuture<'a, Result<flow::Download, SpiderError>> {
        Box::pin(<T as Middleware>::after_download(self, context, response))
    }

    fn download_error<'a>(
        &'a self,
        context: &'a mut context::Download,
        error: &'a SpiderError,
    ) -> MiddlewareFuture<'a, Result<flow::Download, SpiderError>> {
        Box::pin(<T as Middleware>::download_error(self, context, error))
    }

    fn before_parse<'a>(
        &'a self,
        context: &'a mut context::Parse,
    ) -> MiddlewareFuture<'a, Result<flow::Parse, SpiderError>> {
        Box::pin(<T as Middleware>::before_parse(self, context))
    }

    fn parse_error<'a>(
        &'a self,
        context: &'a mut context::Parse,
        error: &'a SpiderError,
    ) -> MiddlewareFuture<'a, Result<flow::Parse, SpiderError>> {
        Box::pin(<T as Middleware>::parse_error(self, context, error))
    }

    fn after_parse<'a>(
        &'a self,
        context: &'a mut context::Parse,
    ) -> MiddlewareFuture<'a, Result<(), SpiderError>> {
        Box::pin(<T as Middleware>::after_parse(self, context))
    }

    fn before_item<'a>(
        &'a self,
        context: &'a mut context::Item,
    ) -> MiddlewareFuture<'a, Result<flow::Item, SpiderError>> {
        Box::pin(<T as Middleware>::before_item(self, context))
    }

    fn after_item<'a>(
        &'a self,
        context: &'a mut context::Item,
    ) -> MiddlewareFuture<'a, Result<(), SpiderError>> {
        Box::pin(<T as Middleware>::after_item(self, context))
    }
}

type MiddlewareFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

pub(crate) type BoxMiddleware = Box<dyn MiddlewareObject>;

pub(crate) fn box_middleware(middleware: impl Middleware + 'static) -> BoxMiddleware {
    Box::new(middleware)
}
