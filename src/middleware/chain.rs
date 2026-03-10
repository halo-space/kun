use crate::middleware::traits::Middleware;
use crate::middleware::types::{MiddlewareConfig, MiddlewareType};
use crate::error::SpiderError;
use crate::future::BoxFuture;

pub struct MiddlewareEntry {
    pub key: String,
    pub config: MiddlewareConfig,
    pub middleware: Box<dyn Middleware>,
}

#[derive(Default)]
pub struct MiddlewareChain {
    pub entries: Vec<MiddlewareEntry>,
}

impl MiddlewareChain {
    pub fn push(
        &mut self,
        key: impl Into<String>,
        config: MiddlewareConfig,
        middleware: Box<dyn Middleware>,
    ) {
        self.entries.push(MiddlewareEntry {
            key: key.into(),
            config,
            middleware,
        });
        self.entries.sort_by_key(|entry| entry.config.order);
    }

    pub fn process_request<'a>(
        &'a self,
        kind: MiddlewareType,
    ) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            for entry in self.entries.iter().filter(|entry| matches_type(entry, kind)) {
                entry.middleware.process_request().await?;
            }
            Ok(())
        })
    }

    pub fn process_response<'a>(
        &'a self,
        kind: MiddlewareType,
    ) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            for entry in self.entries.iter().filter(|entry| matches_type(entry, kind)) {
                entry.middleware.process_response().await?;
            }
            Ok(())
        })
    }

    pub fn process_exception<'a>(
        &'a self,
        kind: MiddlewareType,
        error: &'a SpiderError,
    ) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            for entry in self.entries.iter().filter(|entry| matches_type(entry, kind)) {
                entry.middleware.process_exception(error).await?;
            }
            Ok(())
        })
    }
}

fn matches_type(entry: &MiddlewareEntry, kind: MiddlewareType) -> bool {
    entry.config.enabled && entry.config.r#type == kind
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn middleware_chain_runs_enabled_entries_in_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut chain = MiddlewareChain::default();

        chain.push(
            "second",
            config(true, 200, MiddlewareType::Download),
            Box::new(Record::new("second", log.clone())),
        );
        chain.push(
            "first",
            config(true, 100, MiddlewareType::Download),
            Box::new(Record::new("first", log.clone())),
        );
        chain.push(
            "disabled",
            config(false, 50, MiddlewareType::Download),
            Box::new(Record::new("disabled", log.clone())),
        );

        block_on(chain.process_request(MiddlewareType::Download)).unwrap();

        assert_eq!(*log.lock().unwrap(), vec!["first:req", "second:req"]);
    }

    #[test]
    fn middleware_chain_filters_by_type() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut chain = MiddlewareChain::default();

        chain.push(
            "download",
            config(true, 100, MiddlewareType::Download),
            Box::new(Record::new("download", log.clone())),
        );
        chain.push(
            "spider",
            config(true, 100, MiddlewareType::Spider),
            Box::new(Record::new("spider", log.clone())),
        );

        block_on(chain.process_response(MiddlewareType::Spider)).unwrap();

        assert_eq!(*log.lock().unwrap(), vec!["spider:res"]);
    }

    fn config(enabled: bool, order: i32, r#type: MiddlewareType) -> MiddlewareConfig {
        MiddlewareConfig {
            enabled,
            r#type,
            order,
            options: BTreeMap::new(),
        }
    }

    struct Record {
        name: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Record {
        fn new(name: &'static str, log: Arc<Mutex<Vec<&'static str>>>) -> Self {
            Self { name, log }
        }
    }

    impl Middleware for Record {
        fn process_request<'a>(&'a self) -> BoxFuture<'a, Result<(), SpiderError>> {
            Box::pin(async move {
                self.log.lock().unwrap().push(match self.name {
                    "first" => "first:req",
                    "second" => "second:req",
                    "disabled" => "disabled:req",
                    "download" => "download:req",
                    "spider" => "spider:req",
                    _ => "unknown:req",
                });
                Ok(())
            })
        }

        fn process_response<'a>(&'a self) -> BoxFuture<'a, Result<(), SpiderError>> {
            Box::pin(async move {
                self.log.lock().unwrap().push(match self.name {
                    "first" => "first:res",
                    "second" => "second:res",
                    "disabled" => "disabled:res",
                    "download" => "download:res",
                    "spider" => "spider:res",
                    _ => "unknown:res",
                });
                Ok(())
            })
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
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

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }
}
