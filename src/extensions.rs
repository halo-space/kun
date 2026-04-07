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
                crate::trace::info(
                    "summary",
                    vec![
                        crate::trace::prop("spider", closed.spider_name.as_str()),
                        crate::trace::prop("requests", closed.stats.request_count),
                        crate::trace::prop("responses", closed.stats.response_count),
                        crate::trace::prop("errors", closed.stats.error_count),
                        crate::trace::prop("retries", closed.stats.retry_count),
                        crate::trace::prop("items", closed.stats.item_count),
                        crate::trace::prop("dropped", closed.stats.pipeline_drop_count),
                    ],
                );
            }
        })
    }
}
