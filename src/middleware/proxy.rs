use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::traits::Middleware;
use crate::request::{ProxyConfig, RequestMode};
use crate::value::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
pub struct Proxy {
    fixed_proxy: Option<String>,
    pool: Vec<String>,
    next_index: AtomicUsize,
}

impl Proxy {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            fixed_proxy: options
                .get("url")
                .and_then(Value::as_str)
                .map(str::to_string),
            pool: options
                .get("pool")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            next_index: AtomicUsize::new(0),
        }
    }

    fn select_proxy(&self) -> Option<String> {
        if !self.pool.is_empty() {
            let index = self.next_index.fetch_add(1, Ordering::Relaxed);
            return Some(self.pool[index % self.pool.len()].clone());
        }

        self.fixed_proxy.clone()
    }
}

impl Middleware for Proxy {
    async fn before_download(
        &self,
        context: &mut context::Download,
    ) -> Result<flow::Download, SpiderError> {
        if context.request.mode != RequestMode::Http || context.request.proxy.is_some() {
            return Ok(flow::Download::Continue);
        }

        if let Some(proxy) = self.select_proxy() {
            context.request.proxy = Some(ProxyConfig::new(proxy));
        }

        Ok(flow::Download::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn proxy_middleware_applies_fixed_proxy_when_request_has_none() {
        let middleware = Proxy::new(
            &[(
                "url".to_string(),
                Value::String("http://127.0.0.1:8080".to_string()),
            )]
            .into_iter()
            .collect(),
        );
        let mut context = context::Download::new(Request::new("https://example.com"));

        let flow = block_on(middleware.before_download(&mut context)).unwrap();

        assert_eq!(flow, flow::Download::Continue);
        assert_eq!(
            context.request.proxy,
            Some(ProxyConfig::new("http://127.0.0.1:8080"))
        );
    }

    #[test]
    fn proxy_middleware_keeps_explicit_request_proxy() {
        let middleware = Proxy::new(
            &[(
                "url".to_string(),
                Value::String("http://127.0.0.1:8080".to_string()),
            )]
            .into_iter()
            .collect(),
        );
        let mut context = context::Download::new(
            Request::new("https://example.com").with_proxy("http://upstream"),
        );

        block_on(middleware.before_download(&mut context)).unwrap();

        assert_eq!(
            context.request.proxy,
            Some(ProxyConfig::new("http://upstream"))
        );
    }

    #[test]
    fn proxy_middleware_rotates_proxy_pool_round_robin() {
        let middleware = Proxy::new(
            &[(
                "pool".to_string(),
                Value::Array(vec![
                    Value::String("http://proxy-a".to_string()),
                    Value::String("http://proxy-b".to_string()),
                ]),
            )]
            .into_iter()
            .collect(),
        );
        let mut first = context::Download::new(Request::new("https://example.com/1"));
        let mut second = context::Download::new(Request::new("https://example.com/2"));
        let mut third = context::Download::new(Request::new("https://example.com/3"));

        block_on(middleware.before_download(&mut first)).unwrap();
        block_on(middleware.before_download(&mut second)).unwrap();
        block_on(middleware.before_download(&mut third)).unwrap();

        assert_eq!(
            first.request.proxy,
            Some(ProxyConfig::new("http://proxy-a"))
        );
        assert_eq!(
            second.request.proxy,
            Some(ProxyConfig::new("http://proxy-b"))
        );
        assert_eq!(
            third.request.proxy,
            Some(ProxyConfig::new("http://proxy-a"))
        );
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
