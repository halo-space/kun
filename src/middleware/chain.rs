use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::config::{Config, Stage};
use crate::middleware::traits::{BoxMiddleware, Middleware, box_middleware};
use crate::response::Response;

pub struct Entry {
    pub key: String,
    pub config: Config,
    pub(crate) middleware: BoxMiddleware,
}

#[derive(Default)]
pub struct Chain {
    pub entries: Vec<Entry>,
}

impl Chain {
    pub fn push(
        &mut self,
        key: impl Into<String>,
        config: Config,
        middleware: impl Middleware + 'static,
    ) {
        self.push_boxed(key, config, box_middleware(middleware));
    }

    pub(crate) fn push_boxed(
        &mut self,
        key: impl Into<String>,
        config: Config,
        middleware: BoxMiddleware,
    ) {
        self.entries.push(Entry {
            key: key.into(),
            config,
            middleware,
        });
        self.entries.sort_by_key(|entry| entry.config.order);
    }

    pub fn upsert(
        &mut self,
        key: impl Into<String>,
        config: Config,
        middleware: impl Middleware + 'static,
    ) {
        self.upsert_boxed(key, config, box_middleware(middleware));
    }

    pub(crate) fn upsert_boxed(
        &mut self,
        key: impl Into<String>,
        config: Config,
        middleware: BoxMiddleware,
    ) {
        let key = key.into();
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.key == key) {
            entry.config = config;
            entry.middleware = middleware;
        } else {
            self.entries.push(Entry {
                key,
                config,
                middleware,
            });
        }
        self.entries.sort_by_key(|entry| entry.config.order);
    }

    pub async fn before_enqueue(
        &self,
        context: &mut context::Enqueue,
    ) -> Result<flow::Enqueue, SpiderError> {
        let entries = ordered_entries(&self.entries, Stage::Enqueue, &context.request);
        for entry in entries {
            if context.request.middleware_skips(entry.key.as_str()) {
                continue;
            }

            let next = entry.middleware.before_enqueue(context).await?;
            if !matches!(next, flow::Enqueue::Continue) {
                return Ok(next);
            }
        }

        Ok(flow::Enqueue::Continue)
    }

    pub async fn after_enqueue(&self, context: &mut context::Enqueue) -> Result<(), SpiderError> {
        let entries = ordered_entries(&self.entries, Stage::Enqueue, &context.request);
        for entry in entries {
            if context.request.middleware_skips(entry.key.as_str()) {
                continue;
            }

            entry.middleware.after_enqueue(context).await?;
        }

        Ok(())
    }

    pub async fn before_download(
        &self,
        context: &mut context::Download,
    ) -> Result<flow::Download, SpiderError> {
        let entries = ordered_entries(&self.entries, Stage::Download, &context.request);
        for entry in entries {
            if context.request.middleware_skips(entry.key.as_str()) {
                continue;
            }

            let next = entry.middleware.before_download(context).await?;
            if !matches!(next, flow::Download::Continue) {
                return Ok(next);
            }
        }

        Ok(flow::Download::Continue)
    }

    pub async fn after_download(
        &self,
        context: &mut context::Download,
        response: &mut Response,
    ) -> Result<flow::Download, SpiderError> {
        let entries = ordered_entries(&self.entries, Stage::Download, &context.request);
        for entry in entries {
            if context.request.middleware_skips(entry.key.as_str()) {
                continue;
            }

            let next = entry.middleware.after_download(context, response).await?;
            if !matches!(next, flow::Download::Continue) {
                return Ok(next);
            }
        }

        Ok(flow::Download::Continue)
    }

    pub async fn download_error(
        &self,
        context: &mut context::Download,
        error: &SpiderError,
    ) -> Result<flow::Download, SpiderError> {
        let entries = ordered_entries(&self.entries, Stage::Download, &context.request);
        for entry in entries {
            if context.request.middleware_skips(entry.key.as_str()) {
                continue;
            }

            let next = entry.middleware.download_error(context, error).await?;
            if !matches!(next, flow::Download::Continue) {
                return Ok(next);
            }
        }

        Ok(flow::Download::Continue)
    }

    pub async fn before_parse(
        &self,
        context: &mut context::Parse,
    ) -> Result<flow::Parse, SpiderError> {
        let entries = ordered_entries(&self.entries, Stage::Spider, &context.request);
        for entry in entries {
            if context.request.middleware_skips(entry.key.as_str()) {
                continue;
            }

            let next = entry.middleware.before_parse(context).await?;
            if !matches!(next, flow::Parse::Continue) {
                return Ok(next);
            }
        }

        Ok(flow::Parse::Continue)
    }

    pub async fn parse_error(
        &self,
        context: &mut context::Parse,
        error: &SpiderError,
    ) -> Result<flow::Parse, SpiderError> {
        let entries = ordered_entries(&self.entries, Stage::Spider, &context.request);
        for entry in entries {
            if context.request.middleware_skips(entry.key.as_str()) {
                continue;
            }

            let next = entry.middleware.parse_error(context, error).await?;
            if !matches!(next, flow::Parse::Continue) {
                return Ok(next);
            }
        }

        Ok(flow::Parse::Continue)
    }

    pub async fn after_parse(&self, context: &mut context::Parse) -> Result<(), SpiderError> {
        let entries = ordered_entries(&self.entries, Stage::Spider, &context.request);
        for entry in entries {
            if context.request.middleware_skips(entry.key.as_str()) {
                continue;
            }

            entry.middleware.after_parse(context).await?;
        }

        Ok(())
    }

    pub async fn before_item(
        &self,
        context: &mut context::Item,
    ) -> Result<flow::Item, SpiderError> {
        let entries = ordered_entries(&self.entries, Stage::Item, &context.request);
        for entry in entries {
            if context.request.middleware_skips(entry.key.as_str()) {
                continue;
            }

            let next = entry.middleware.before_item(context).await?;
            if !matches!(next, flow::Item::Continue) {
                return Ok(next);
            }
        }

        Ok(flow::Item::Continue)
    }

    pub async fn after_item(&self, context: &mut context::Item) -> Result<(), SpiderError> {
        let entries = ordered_entries(&self.entries, Stage::Item, &context.request);
        for entry in entries {
            if context.request.middleware_skips(entry.key.as_str()) {
                continue;
            }

            entry.middleware.after_item(context).await?;
        }

        Ok(())
    }
}

fn matches_stage(entry: &Entry, kind: Stage) -> bool {
    entry.config.enabled && entry.config.stage == kind
}

fn ordered_entries<'a>(
    entries: &'a [Entry],
    kind: Stage,
    request: &crate::request::Request,
) -> Vec<&'a Entry> {
    let mut out = entries
        .iter()
        .filter(|entry| matches_stage(entry, kind))
        .collect::<Vec<_>>();
    out.sort_by_key(|entry| {
        request
            .middleware_order(entry.key.as_str())
            .unwrap_or(entry.config.order)
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn chain_runs_enabled_entries_in_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut chain = Chain::default();

        chain.push(
            "second",
            config(true, 200, Stage::Download),
            Box::new(Record::new("second", log.clone())),
        );
        chain.push(
            "first",
            config(true, 100, Stage::Download),
            Box::new(Record::new("first", log.clone())),
        );
        chain.push(
            "disabled",
            config(false, 50, Stage::Download),
            Box::new(Record::new("disabled", log.clone())),
        );

        let mut context = context::Download::new(Request::new("https://example.com"));
        let next = block_on(chain.before_download(&mut context)).unwrap();

        assert_eq!(next, flow::Download::Continue);
        assert_eq!(
            *log.lock().unwrap(),
            vec!["first:download", "second:download"]
        );
    }

    #[test]
    fn chain_filters_by_stage() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut chain = Chain::default();

        chain.push(
            "download",
            config(true, 100, Stage::Download),
            Box::new(Record::new("download", log.clone())),
        );
        chain.push(
            "spider",
            config(true, 100, Stage::Spider),
            Box::new(Record::new("spider", log.clone())),
        );

        let request = Request::new("https://example.com");
        let response = Response::from_request(request.clone(), 200, Default::default(), Vec::new());
        let mut context = context::Parse::new(request, response);
        let next = block_on(chain.before_parse(&mut context)).unwrap();

        assert_eq!(next, flow::Parse::Continue);
        assert_eq!(*log.lock().unwrap(), vec!["spider:parse"]);
    }

    #[test]
    fn chain_skips_exact_request_middleware_keys() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut chain = Chain::default();

        chain.push(
            "download",
            config(true, 100, Stage::Download),
            Box::new(Record::new("download", log.clone())),
        );

        let mut context =
            context::Download::new(Request::new("https://example.com").skip(["download"]));
        let next = block_on(chain.before_download(&mut context)).unwrap();

        assert_eq!(next, flow::Download::Continue);
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn chain_uses_request_override_order_when_present() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut chain = Chain::default();

        chain.push(
            "second",
            config(true, 200, Stage::Download),
            Box::new(Record::new("second", log.clone())),
        );
        chain.push(
            "first",
            config(true, 100, Stage::Download),
            Box::new(Record::new("first", log.clone())),
        );

        let mut context = context::Download::new(
            Request::new("https://example.com").with_middleware_options_ordered(
                "second",
                BTreeMap::new(),
                50,
            ),
        );
        let next = block_on(chain.before_download(&mut context)).unwrap();

        assert_eq!(next, flow::Download::Continue);
        assert_eq!(
            *log.lock().unwrap(),
            vec!["second:download", "first:download"]
        );
    }

    fn config(enabled: bool, order: i32, stage: Stage) -> Config {
        Config {
            enabled,
            stage,
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
        async fn before_download(
            &self,
            _context: &mut context::Download,
        ) -> Result<flow::Download, SpiderError> {
            self.log.lock().unwrap().push(match self.name {
                "first" => "first:download",
                "second" => "second:download",
                "disabled" => "disabled:download",
                "download" => "download:download",
                "spider" => "spider:download",
                _ => "unknown:download",
            });
            Ok(flow::Download::Continue)
        }

        async fn before_parse(
            &self,
            _context: &mut context::Parse,
        ) -> Result<flow::Parse, SpiderError> {
            self.log.lock().unwrap().push(match self.name {
                "first" => "first:parse",
                "second" => "second:parse",
                "disabled" => "disabled:parse",
                "download" => "download:parse",
                "spider" => "spider:parse",
                _ => "unknown:parse",
            });
            Ok(flow::Parse::Continue)
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
