use crate::download::traits::Downloader;
use crate::error::SpiderError;
use crate::request::{Headers, Request, RequestMode};
use crate::response::Response;
use reqwest::Client;

pub struct HttpDownloader {
    client: Client,
    no_redirect_client: Client,
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self {
            client: Client::new(),
            no_redirect_client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("failed to build no-redirect reqwest client"),
        }
    }
}

impl Downloader for HttpDownloader {
    async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
        if request.mode != RequestMode::Http {
            return Err(SpiderError::download(
                "http downloader received non-http request",
            ));
        }

        let no_redirect = request
            .http
            .as_ref()
            .is_some_and(|c| !c.allow_redirects);
        let client = if no_redirect {
            &self.no_redirect_client
        } else {
            &self.client
        };

        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|e| SpiderError::request_build(e.to_string()))?;

        let mut req_builder = client.request(method, &request.url);

        for (name, values) in &request.headers {
            for value in values {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }
        }

        if let Some(body) = &request.body {
            req_builder = req_builder.body(body.clone());
        }

        if let Some(http_config) = &request.http {
            for (k, v) in &http_config.query {
                req_builder = req_builder.query(&[(k.as_str(), v.as_str())]);
            }
            for (k, v) in &http_config.cookies {
                req_builder = req_builder.header(
                    reqwest::header::COOKIE,
                    format!("{k}={v}"),
                );
            }
        }

        let resp = req_builder
            .send()
            .await
            .map_err(|e| SpiderError::download(e.to_string()))?;

        let status = resp.status().as_u16();
        let protocol = version_str(resp.version());
        let ip_address = resp.remote_addr().map(|addr| addr.ip());
        let resp_headers = collect_headers(resp.headers());

        let body = resp
            .bytes()
            .await
            .map_err(|e| SpiderError::download(e.to_string()))?
            .to_vec();

        let mut response =
            Response::from_request(request.clone(), status, resp_headers, body);
        response.protocol = Some(protocol.to_string());
        response.ip_address = ip_address;
        Ok(response)
    }
}

fn version_str(version: reqwest::Version) -> &'static str {
    match version {
        reqwest::Version::HTTP_09 => "HTTP/0.9",
        reqwest::Version::HTTP_10 => "HTTP/1.0",
        reqwest::Version::HTTP_11 => "HTTP/1.1",
        reqwest::Version::HTTP_2 => "HTTP/2",
        reqwest::Version::HTTP_3 => "HTTP/3",
        _ => "HTTP/unknown",
    }
}

fn collect_headers(headers: &reqwest::header::HeaderMap) -> Headers {
    let mut result = Headers::new();
    for (name, value) in headers {
        if let Ok(v) = value.to_str() {
            result
                .entry(name.to_string())
                .or_default()
                .push(v.to_string());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_downloader_rejects_browser_request() {
        let downloader = HttpDownloader::default();
        let request = Request::browser("https://example.com");

        let result = futures_block_on(downloader.fetch(&request));

        assert!(matches!(result, Err(SpiderError::Download(_))));
    }

    #[test]
    fn version_str_maps_all_variants() {
        assert_eq!(version_str(reqwest::Version::HTTP_09), "HTTP/0.9");
        assert_eq!(version_str(reqwest::Version::HTTP_10), "HTTP/1.0");
        assert_eq!(version_str(reqwest::Version::HTTP_11), "HTTP/1.1");
        assert_eq!(version_str(reqwest::Version::HTTP_2), "HTTP/2");
        assert_eq!(version_str(reqwest::Version::HTTP_3), "HTTP/3");
    }

    fn futures_block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::pin::Pin;
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct NoopWake;
        impl Wake for NoopWake {
            fn wake(self: Arc<Self>) {}
        }

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
}
