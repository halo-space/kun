use crate::future::BoxFuture;
use crate::signals::Signal;

pub trait Extension: Send + Sync {
    fn on_signal<'a>(&'a self, signal: &'a Signal) -> BoxFuture<'a, ()>;
}

impl<T> crate::signals::Listener for T
where
    T: Extension,
{
    fn on_signal<'a>(&'a self, signal: &'a Signal) -> BoxFuture<'a, ()> {
        Extension::on_signal(self, signal)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Summary;

impl Extension for Summary {
    fn on_signal<'a>(&'a self, signal: &'a Signal) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Signal::SpiderClosed(closed) = signal {
                tracing::info!(
                    spider = closed.spider_name.as_str(),
                    request_count = closed.stats.request_count,
                    response_count = closed.stats.response_count,
                    error_count = closed.stats.error_count,
                    retry_count = closed.stats.retry_count,
                    item_count = closed.stats.item_count,
                    pipeline_drop_count = closed.stats.pipeline_drop_count,
                    "extension summary"
                );
            }
        })
    }
}
