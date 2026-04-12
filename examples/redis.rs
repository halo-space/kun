#![allow(refining_impl_trait)]

//! Redis store example: period.xml -> Spider item -> store::Redis
//!
//! Shows:
//! - Spider only produces items
//! - final item is assembled directly in `parse()`
//! - built-in `store::Redis` stays on the same `parse -> item -> pipeline -> store` chain
//! - this example uses a local fake Redis server so it can run without an external dependency
//!
//! Run:
//!   cargo run --example redis

use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::pipeline::Pipeline;
use halo_spider::response::Response;
use halo_spider::settings::Config;
use halo_spider::spider::Spider;
use halo_spider::store::Redis;
use halo_spider::value::Value;
use jiff::SignedDuration;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

struct PeriodIssueSpider;

impl Spider for PeriodIssueSpider {
    fn name(&self) -> &str {
        "period_redis"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://ep.shxwcb.com/2026/03/period.xml".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Item, SpiderError> {
        let (period_date, front_page) = latest_issue(response)?;
        let edition_url = build_edition_url(&period_date, &front_page)?;
        let issue_key = format!("{period_date}-front-{front_page}");

        let item = Item::new()
            .with_field("period_date", Value::String(period_date))
            .with_field("front_page", Value::String(front_page))
            .with_field("edition_url", Value::String(edition_url))
            .with_field("source", Value::String("period.xml".to_string()))
            .with_field("issue_key", Value::String(issue_key));

        Ok(item)
    }
}

#[derive(Clone)]
struct StopAfterFirst {
    handle: ShutdownHandle,
    stopped: Arc<AtomicBool>,
}

impl StopAfterFirst {
    fn new(handle: ShutdownHandle) -> Self {
        Self {
            handle,
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Pipeline for StopAfterFirst {
    async fn process(&self, _item: &mut Item, _spider_name: &str) -> Result<bool, SpiderError> {
        if !self.stopped.swap(true, Ordering::Relaxed) {
            self.handle.stop();
        }
        Ok(true)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    halo_spider::trace::init_console();

    let (redis_url, commands_rx, server_handle) = spawn_fake_redis_server().await;
    let redis = Redis::new(redis_url, "period_items");
    let settings = Config::default().with_idle_timeout(SignedDuration::from_millis(200));

    let engine = Engine::new().with_config(settings);
    let handle = engine.shutdown_handle();

    let mut engine = engine
        .with_pipeline(StopAfterFirst::new(handle))
        .with_store(redis.clone());

    engine.run(&PeriodIssueSpider).await?;
    let total_items = engine.stats().item_count;
    let commands = commands_rx.await?;

    println!("engine returned {total_items} item(s)");
    println!("redis commands:");
    for command in commands {
        println!("  {command:?}");
    }

    server_handle.await??;

    Ok(())
}

async fn spawn_fake_redis_server() -> (
    String,
    oneshot::Receiver<Vec<Vec<String>>>,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (commands_tx, commands_rx) = oneshot::channel();

    let server_handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut commands = Vec::new();

        while let Some(command) = read_resp_command(&mut stream).await? {
            let reply = match command.first().map(String::as_str) {
                Some("SADD") => b":1\r\n".as_slice(),
                _ => b"-ERR unsupported example command\r\n".as_slice(),
            };
            commands.push(command);
            stream.write_all(reply).await?;
        }

        commands_tx.send(commands).ok();
        Ok(())
    });

    (format!("redis://{address}"), commands_rx, server_handle)
}

async fn read_resp_command(stream: &mut TcpStream) -> Result<Option<Vec<String>>, std::io::Error> {
    let mut prefix = [0_u8; 1];
    match stream.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }

    if prefix[0] != b'*' {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "resp command did not start with array marker",
        ));
    }

    let count = read_resp_line(stream)
        .await?
        .parse::<usize>()
        .map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid array length: {error}"),
            )
        })?;
    let mut command = Vec::with_capacity(count);

    for _ in 0..count {
        let mut bulk_prefix = [0_u8; 1];
        stream.read_exact(&mut bulk_prefix).await?;
        if bulk_prefix[0] != b'$' {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "resp bulk string did not start with $",
            ));
        }

        let length = read_resp_line(stream)
            .await?
            .parse::<usize>()
            .map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid bulk string length: {error}"),
                )
            })?;

        let mut bytes = vec![0_u8; length + 2];
        stream.read_exact(&mut bytes).await?;
        command.push(
            String::from_utf8(bytes[..length].to_vec()).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("command was not utf-8: {error}"),
                )
            })?,
        );
    }

    Ok(Some(command))
}

async fn read_resp_line(stream: &mut TcpStream) -> Result<String, std::io::Error> {
    let mut bytes = Vec::new();

    loop {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await?;
        if byte[0] == b'\r' {
            let mut line_feed = [0_u8; 1];
            stream.read_exact(&mut line_feed).await?;
            if line_feed[0] != b'\n' {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "expected LF after CR",
                ));
            }
            break;
        }
        bytes.push(byte[0]);
    }

    String::from_utf8(bytes).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("line was not utf-8: {error}"),
        )
    })
}

fn latest_issue(response: &Response) -> Result<(String, String), SpiderError> {
    let period_date = response
        .xml("//period[last()]/period_date")
        .text()
        .one()
        .ok_or_else(|| SpiderError::parse("period_date not found"))?;
    let front_page = response
        .xml("//period[last()]/front_page")
        .text()
        .one()
        .ok_or_else(|| SpiderError::parse("front_page not found"))?;

    Ok((period_date, front_page))
}

fn build_edition_url(period_date: &str, front_page: &str) -> Result<String, SpiderError> {
    let mut parts = period_date.split('-');
    let year = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date is missing year"))?;
    let month = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date is missing month"))?;
    let day = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date is missing day"))?;

    Ok(format!(
        "https://ep.shxwcb.com/{year}/{month}/{day}/{front_page}?f={year}/{month}/period.xml"
    ))
}
