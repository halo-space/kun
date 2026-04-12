use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::middleware::traits::Middleware;
use crate::request::{Headers, SessionConfig};
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Cookies {
    default_session: Option<String>,
}

impl Cookies {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        Self {
            default_session: options
                .get("session")
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    }

    fn apply_default_session(&self, context: &mut context::Download) {
        if context.request.session.is_none()
            && let Some(session_id) = &self.default_session
        {
            context.request.session = Some(SessionConfig::new(session_id.clone()));
        }
    }

    fn normalize_cookie_headers(&self, context: &mut context::Download) {
        let cookie_headers = take_cookie_headers(&mut context.request.headers);
        if cookie_headers.is_empty() {
            return;
        }

        let cookies = &mut context.request.cookies;
        for header in cookie_headers {
            merge_cookie_header(cookies, &header);
        }
    }
}

impl Middleware for Cookies {
    async fn before_download(
        &self,
        context: &mut context::Download,
    ) -> Result<flow::Download, SpiderError> {
        self.apply_default_session(context);
        self.normalize_cookie_headers(context);
        Ok(flow::Download::Continue)
    }
}

fn take_cookie_headers(headers: &mut Headers) -> Vec<String> {
    let keys = headers
        .keys()
        .filter(|name| name.eq_ignore_ascii_case("cookie"))
        .cloned()
        .collect::<Vec<_>>();

    let mut values = Vec::new();
    for key in keys {
        if let Some(header_values) = headers.remove(&key) {
            values.extend(header_values);
        }
    }

    values
}

fn merge_cookie_header(cookies: &mut BTreeMap<String, String>, header: &str) {
    for segment in header.split(';') {
        let Some((name, value)) = segment.split_once('=') else {
            continue;
        };

        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || value.is_empty() {
            continue;
        }

        cookies
            .entry(name.to_string())
            .or_insert_with(|| value.to_string());
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
    fn cookies_middleware_assigns_default_session_and_normalizes_cookie_header() {
        let middleware = Cookies::new(
            &[("session".to_string(), Value::String("shared".to_string()))]
                .into_iter()
                .collect(),
        );
        let mut context = context::Download::new(
            Request::new("https://example.com")
                .with_header("Cookie", "sid=abc; theme=light")
                .with_cookie("theme", "dark"),
        );

        let flow = block_on(middleware.before_download(&mut context)).unwrap();

        assert_eq!(flow, flow::Download::Continue);
        assert_eq!(context.request.session, Some(SessionConfig::new("shared")));
        assert!(context.request.headers.is_empty());
        assert_eq!(
            context.request.cookies.get("sid").map(String::as_str),
            Some("abc")
        );
        assert_eq!(
            context.request.cookies.get("theme").map(String::as_str),
            Some("dark")
        );
    }

    #[test]
    fn cookies_middleware_keeps_explicit_session() {
        let middleware = Cookies::new(
            &[("session".to_string(), Value::String("shared".to_string()))]
                .into_iter()
                .collect(),
        );
        let mut context =
            context::Download::new(Request::new("https://example.com").with_session("custom"));

        block_on(middleware.before_download(&mut context)).unwrap();

        assert_eq!(context.request.session, Some(SessionConfig::new("custom")));
    }

    #[test]
    fn cookies_middleware_normalizes_browser_request_cookie_header_without_switching_mode() {
        let middleware = Cookies::default();
        let mut context = context::Download::new(
            Request::browser("https://example.com").with_header("Cookie", "sid=abc; theme=light"),
        );

        let flow = block_on(middleware.before_download(&mut context)).unwrap();

        assert_eq!(flow, flow::Download::Continue);
        assert_eq!(context.request.mode, crate::request::RequestMode::Browser);
        assert!(context.request.headers.is_empty());
        assert_eq!(
            context.request.cookies.get("sid").map(String::as_str),
            Some("abc")
        );
        assert_eq!(
            context.request.cookies.get("theme").map(String::as_str),
            Some("light")
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
