//! robots site policy example: local robots.txt + Site matcher overlay
//!
//! Shows:
//! - `robots::Site::pattern(...)` can provide broad defaults
//! - `robots::Site::host(...)` can tighten host-level policy
//! - `robots::Site::origin(...)` can override a single origin more specifically
//! - matched site policies merge delay and sitemap data
//!
//! Run:
//!   cargo run --example robots_site_policy

use halo_spider::request::Request;
use halo_spider::robots::{self, Decision, Robot};
use jiff::SignedDuration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let robots_body = "\
User-agent: *\n\
Disallow: /private\n\
Crawl-delay: 0.1\n\
Sitemap: https://example.com/default-sitemap.xml\n";

    let (base_url, server_handle) = spawn_robots_server(robots_body).await;

    let robot = robots::Memory::new()
        .with_site_policy(
            robots::Site::pattern("127.*"),
            robots::SitePolicy::new()
                .with_delay(SignedDuration::from_millis(500))
                .with_sitemap("https://example.com/network-sitemap.xml"),
        )
        .with_site_policy(
            robots::Site::host("127.0.0.1"),
            robots::SitePolicy::new()
                .with_access(robots::SiteAccess::DisallowAll)
                .with_sitemap("https://example.com/host-sitemap.xml"),
        )
        .with_site_policy(
            robots::Site::origin(base_url.clone()),
            robots::SitePolicy::new()
                .with_access(robots::SiteAccess::AllowAll)
                .with_unavailable_policy(robots::UnavailablePolicy::DisallowAll),
        );

    let private_request = Request::new(format!("{base_url}/private/page"));
    let public_request = Request::new(format!("{base_url}/news/2"));

    let first = robot.check(&private_request, "kun").await?;
    let second = robot.check(&public_request, "kun").await?;
    let sitemaps = robot.sitemaps(&private_request).await?;

    println!("origin: {base_url}");
    println!("private request decision: {}", describe_decision(&first));
    println!("second request decision: {}", describe_decision(&second));
    println!("merged sitemaps: {sitemaps:#?}");
    println!();
    println!("what this demonstrates:");
    println!("- origin matcher beats host matcher for access");
    println!("- delay uses the stricter value across robots + matched site policies");
    println!("- sitemap URLs are merged and deduplicated");

    server_handle.await??;

    Ok(())
}

fn describe_decision(decision: &Decision) -> String {
    match decision {
        Decision::Allow => "allow".to_string(),
        Decision::Disallow => "disallow".to_string(),
        Decision::Delay(delay) => format!("delay {} ms", delay.as_millis()),
    }
}

async fn spawn_robots_server(
    body: &str,
) -> (String, tokio::task::JoinHandle<Result<(), std::io::Error>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_string();

    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer).await?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await?;
        Ok(())
    });

    (format!("http://{}", address), handle)
}
