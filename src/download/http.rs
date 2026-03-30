use crate::download::traits::Downloader;
use crate::error::SpiderError;
use crate::request::{Headers, Request, RequestMode};
use crate::response::Response;
use reqwest::cookie::Jar;
use reqwest::{Client, Url};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub pool_max_idle_per_host: usize,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            pool_max_idle_per_host: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ClientKey {
    allow_redirects: bool,
    proxy: Option<String>,
    session: Option<String>,
}

#[derive(Clone)]
struct ResolvedClient {
    client: Client,
    jar: Option<Arc<Jar>>,
}

pub struct Http {
    config: HttpConfig,
    clients: Mutex<BTreeMap<ClientKey, Client>>,
    jars: Mutex<BTreeMap<String, Arc<Jar>>>,
}

impl Http {
    pub fn new() -> Self {
        Self::with_config(HttpConfig::default())
    }

    pub fn with_config(config: HttpConfig) -> Self {
        Self {
            config,
            clients: Mutex::new(BTreeMap::new()),
            jars: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn with_pool_size(mut self, pool_size: usize) -> Self {
        self.config.pool_max_idle_per_host = pool_size;
        self.clients
            .get_mut()
            .expect("client cache poisoned")
            .clear();
        self
    }

    fn resolve_client(&self, request: &Request) -> Result<ResolvedClient, SpiderError> {
        let allow_redirects = request
            .http
            .as_ref()
            .is_some_and(|config| config.allow_redirects);
        let proxy = request.proxy.as_ref().map(|proxy| proxy.url.clone());
        let session = request.session.as_ref().map(|session| session.id.clone());
        let jar = match session.as_deref() {
            Some(session_id) => Some(self.session_jar(session_id)?),
            None if has_request_cookies(request) || allow_redirects => {
                Some(Arc::new(Jar::default()))
            }
            None => None,
        };

        let key = ClientKey {
            allow_redirects,
            proxy,
            session,
        };

        if key.session.is_none() && jar.is_some() {
            return Ok(ResolvedClient {
                client: self.build_client(&key, jar.clone())?,
                jar,
            });
        }

        let mut clients = self
            .clients
            .lock()
            .map_err(|_| SpiderError::engine("http client cache poisoned"))?;

        if let Some(client) = clients.get(&key) {
            return Ok(ResolvedClient {
                client: client.clone(),
                jar,
            });
        }

        let client = self.build_client(&key, jar.clone())?;
        clients.insert(key, client.clone());

        Ok(ResolvedClient { client, jar })
    }

    fn session_jar(&self, session_id: &str) -> Result<Arc<Jar>, SpiderError> {
        let mut jars = self
            .jars
            .lock()
            .map_err(|_| SpiderError::engine("http session jar cache poisoned"))?;

        Ok(jars
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Jar::default()))
            .clone())
    }

    fn build_client(&self, key: &ClientKey, jar: Option<Arc<Jar>>) -> Result<Client, SpiderError> {
        let mut builder =
            Client::builder().pool_max_idle_per_host(self.config.pool_max_idle_per_host);

        if !key.allow_redirects {
            builder = builder.redirect(reqwest::redirect::Policy::none());
        }

        if let Some(proxy) = &key.proxy {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|error| SpiderError::request_build(error.to_string()))?,
            );
        }

        if let Some(jar) = jar {
            builder = builder.cookie_provider(jar);
        }

        builder
            .build()
            .map_err(|error| SpiderError::request_build(error.to_string()))
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Downloader for Http {
    async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
        if request.mode != RequestMode::Http {
            return Err(SpiderError::download(
                "http downloader received non-http request",
            ));
        }

        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|e| SpiderError::request_build(e.to_string()))?;
        let url = build_url(request)?;
        let resolved = self.resolve_client(request)?;
        prime_cookie_jar(resolved.jar.as_deref(), &url, request);

        let mut req_builder = resolved.client.request(method, url.clone());

        for (name, values) in &request.headers {
            if resolved.jar.is_some() && name.eq_ignore_ascii_case(reqwest::header::COOKIE.as_str())
            {
                continue;
            }

            for value in values {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }
        }

        if let Some(body) = &request.body {
            req_builder = req_builder.body(body.clone());
        }

        if let Some(timeout) = request.timeout {
            req_builder = req_builder.timeout(timeout);
        }

        let resp = req_builder
            .send()
            .await
            .map_err(|e| SpiderError::download(e.to_string()))?;

        let status = resp.status().as_u16();
        let protocol = version_str(resp.version());
        let ip_address = resp.remote_addr().map(|addr| addr.ip());
        let final_url = resp.url().to_string();
        let resp_headers = collect_headers(resp.headers());

        let body = resp
            .bytes()
            .await
            .map_err(|e| SpiderError::download(e.to_string()))?
            .to_vec();

        let mut response = Response::from_request(request.clone(), status, resp_headers, body);
        response.url = final_url;
        response.protocol = Some(protocol.to_string());
        response.ip_address = ip_address;
        Ok(response)
    }
}

fn build_url(request: &Request) -> Result<Url, SpiderError> {
    let mut url =
        Url::parse(&request.url).map_err(|error| SpiderError::request_build(error.to_string()))?;

    if let Some(http_config) = &request.http {
        if !http_config.query.is_empty() {
            let mut query_pairs = url.query_pairs_mut();
            for (key, value) in &http_config.query {
                query_pairs.append_pair(key, value);
            }
        }
    }

    Ok(url)
}

fn has_request_cookies(request: &Request) -> bool {
    request
        .http
        .as_ref()
        .is_some_and(|http| !http.cookies.is_empty())
}

fn prime_cookie_jar(jar: Option<&Jar>, url: &Url, request: &Request) {
    let Some(jar) = jar else {
        return;
    };

    let Some(http_config) = &request.http else {
        return;
    };

    for (key, value) in &http_config.cookies {
        jar.add_cookie_str(&format!("{key}={value}"), url);
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
    use crate::request::http::Config as RequestHttpConfig;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn http_downloader_rejects_browser_request() {
        let downloader = Http::default();
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

    #[tokio::test]
    async fn http_downloader_applies_timeout() {
        let server = spawn_server(1, |_, _, _| TestResponse {
            status_line: "HTTP/1.1 200 OK".to_string(),
            headers: Vec::new(),
            body: b"slow".to_vec(),
            delay: Some(Duration::from_millis(200)),
        });
        let downloader = Http::default();
        let request = Request::new(server.url("/timeout")).with_timeout(Duration::from_millis(50));

        let result = downloader.fetch(&request).await;

        assert!(matches!(result, Err(SpiderError::Download(_))));
    }

    #[tokio::test]
    async fn http_downloader_persists_session_cookies_with_reqwest_jar() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = seen.clone();
        let server = spawn_server(2, move |_, index, request| {
            seen_clone.lock().unwrap().push(request);
            if index == 0 {
                TestResponse {
                    status_line: "HTTP/1.1 200 OK".to_string(),
                    headers: vec![("Set-Cookie".to_string(), "sid=abc; Path=/".to_string())],
                    body: b"first".to_vec(),
                    delay: None,
                }
            } else {
                TestResponse::ok("second")
            }
        });
        let downloader = Http::default();
        let url = server.url("/session");

        let first = Request::new(url.clone()).with_session("shared");
        let second = Request::new(url).with_session("shared");

        downloader.fetch(&first).await.unwrap();
        downloader.fetch(&second).await.unwrap();

        let requests = seen.lock().unwrap();
        assert_eq!(
            requests[1]
                .headers
                .get("cookie")
                .and_then(|values| values.first())
                .map(String::as_str),
            Some("sid=abc")
        );
    }

    #[tokio::test]
    async fn http_downloader_routes_http_requests_via_proxy() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = seen.clone();
        let proxy = spawn_server(1, move |_, _, request| {
            seen_clone.lock().unwrap().push(request);
            TestResponse::ok("proxied")
        });
        let downloader = Http::default();
        let request = Request::new("http://does-not-resolve.test/articles")
            .with_proxy(format!("http://{}", proxy.addr));

        let response = downloader.fetch(&request).await.unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.text, "proxied");
        assert_eq!(
            seen.lock().unwrap()[0].request_line,
            "GET http://does-not-resolve.test/articles HTTP/1.1"
        );
    }

    #[tokio::test]
    async fn http_downloader_follows_redirects_when_enabled() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = seen.clone();
        let server = spawn_server(2, move |addr, index, request| {
            seen_clone.lock().unwrap().push(request);
            if index == 0 {
                TestResponse {
                    status_line: "HTTP/1.1 302 Found".to_string(),
                    headers: vec![("Location".to_string(), format!("http://{addr}/final"))],
                    body: Vec::new(),
                    delay: None,
                }
            } else {
                TestResponse::ok("done")
            }
        });
        let final_url = server.url("/final");
        let downloader = Http::default();
        let request = Request::new(server.url("/redirect"))
            .with_http(RequestHttpConfig::default().with_redirects(true));

        let response = downloader.fetch(&request).await.unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.url, final_url);
        assert_eq!(response.text, "done");
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    fn futures_block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::pin::Pin;
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

    #[derive(Debug, Clone)]
    struct TestRequest {
        request_line: String,
        headers: BTreeMap<String, Vec<String>>,
    }

    struct TestResponse {
        status_line: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        delay: Option<Duration>,
    }

    impl TestResponse {
        fn ok(body: &str) -> Self {
            Self {
                status_line: "HTTP/1.1 200 OK".to_string(),
                headers: Vec::new(),
                body: body.as_bytes().to_vec(),
                delay: None,
            }
        }
    }

    struct TestServer {
        addr: SocketAddr,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn url(&self, path: &str) -> String {
            format!("http://{}{}", self.addr, path)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                handle.join().unwrap();
            }
        }
    }

    fn spawn_server<F>(expected_requests: usize, handler: F) -> TestServer
    where
        F: FnMut(SocketAddr, usize, TestRequest) -> TestResponse + Send + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || serve(listener, addr, expected_requests, handler));

        TestServer {
            addr,
            handle: Some(handle),
        }
    }

    fn serve<F>(listener: TcpListener, addr: SocketAddr, expected_requests: usize, mut handler: F)
    where
        F: FnMut(SocketAddr, usize, TestRequest) -> TestResponse,
    {
        for index in 0..expected_requests {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let response = handler(addr, index, request);
            if let Some(delay) = response.delay {
                thread::sleep(delay);
            }
            write_response(&mut stream, response);
        }
    }

    fn read_request(stream: &mut TcpStream) -> TestRequest {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        let mut buffer = [0; 4096];
        let mut raw = Vec::new();
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&buffer[..read]);
            if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }

        let text = String::from_utf8(raw).unwrap();
        let mut lines = text.split("\r\n");
        let request_line = lines.next().unwrap_or_default().to_string();
        let mut headers = BTreeMap::new();

        for line in lines.take_while(|line| !line.is_empty()) {
            if let Some((name, value)) = line.split_once(':') {
                headers
                    .entry(name.trim().to_ascii_lowercase())
                    .or_insert_with(Vec::new)
                    .push(value.trim().to_string());
            }
        }

        TestRequest {
            request_line,
            headers,
        }
    }

    fn write_response(stream: &mut TcpStream, response: TestResponse) {
        let mut headers = response.headers;
        headers.push((
            "Content-Length".to_string(),
            response.body.len().to_string(),
        ));
        headers.push(("Connection".to_string(), "close".to_string()));

        let mut raw = format!("{}\r\n", response.status_line);
        for (name, value) in headers {
            raw.push_str(&format!("{name}: {value}\r\n"));
        }
        raw.push_str("\r\n");

        stream.write_all(raw.as_bytes()).unwrap();
        stream.write_all(&response.body).unwrap();
    }
}
