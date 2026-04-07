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
                        crate::trace::prop("request_count", closed.stats.request_count),
                        crate::trace::prop("response_count", closed.stats.response_count),
                        crate::trace::prop("error_count", closed.stats.error_count),
                        crate::trace::prop("retry_count", closed.stats.retry_count),
                        crate::trace::prop("item_count", closed.stats.item_count),
                        crate::trace::prop("pipeline_drop_count", closed.stats.pipeline_drop_count),
                    ],
                );
            }
        })
    }
}
