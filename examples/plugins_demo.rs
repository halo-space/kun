//! Plugin system example focused on the current middleware path.
//!
//! This example shows:
//! - declarative loading from `plugins.toml`
//! - `(kind, name)` identity and override rules in `PluginRegistry`
//! - how middleware plugins are connected into the engine
//!
//! Flow:
//! 1. A plugin author implements middleware plus a factory function
//! 2. `plugins.toml` declares the plugin (`name`, `kind`, `entry`, `override`)
//! 3. The engine loads manifests, `PluginRegistry` validates identity rules,
//!    and registered factories are matched by name
//! 4. End users enable middleware by key in `Settings`
//!
//! Note: `Engine::load_plugins()` currently supports only `kind = "middleware"`.
//! Other known kinds map to future engine component owners such as `store`,
//! `scheduler`, `dedup`, `robots`, `http`, and `browser`, but they are not
//! auto-loaded into the runtime yet.
//!
//! Run with: `cargo run --example plugins_demo`

use halo_spider::engine::Engine;
use halo_spider::engine::context::EngineContext;
use halo_spider::engine::flow::Flow;
use halo_spider::error::SpiderError;
use halo_spider::future::BoxFuture;
use halo_spider::item::Item;
use halo_spider::middleware::traits::Middleware;
use halo_spider::middleware::{Config, Stage};
use halo_spider::plugins::{PluginManifest, PluginRegistry, load_plugin_manifest};
use halo_spider::response::Response;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::value::Value;
use halo_spider::{cb, spider_callbacks};
use jiff::SignedDuration;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

// Plugin middleware implementations used by the example.

/// Custom signature middleware that adds an `X-Signature` header to requests.
/// Corresponds to `(middleware, custom_signature)` in `plugins.toml`.
struct CustomSignatureMiddleware {
    secret: String,
}

impl CustomSignatureMiddleware {
    fn new(options: &BTreeMap<String, Value>) -> Self {
        let secret = options
            .get("secret")
            .and_then(Value::as_str)
            .unwrap_or("default-secret")
            .to_string();
        Self { secret }
    }
}

impl Middleware for CustomSignatureMiddleware {
    fn process_request<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            let sig = format!("sha256:{:x}", {
                let mut hash: u64 = 0;
                for byte in self.secret.as_bytes() {
                    hash = hash.wrapping_mul(31).wrapping_add(*byte as u64);
                }
                for byte in context.request.url.as_bytes() {
                    hash = hash.wrapping_mul(31).wrapping_add(*byte as u64);
                }
                hash
            });
            context
                .request
                .headers
                .entry("X-Signature".to_string())
                .or_insert_with(|| vec![sig.clone()]);
            halo_spider::trace::info(
                "signature.sign",
                vec![
                    halo_spider::trace::prop("url", context.request.url.as_str()),
                    halo_spider::trace::prop("signature", sig.as_str()),
                ],
            );
            Ok(Flow::Continue)
        })
    }
}

/// Stats middleware that counts requests and responses.
/// Corresponds to `(middleware, stats)` in `plugins.toml`.
struct StatsMiddleware {
    request_count: AtomicUsize,
    response_count: AtomicUsize,
    label: String,
}

impl StatsMiddleware {
    fn new(options: &BTreeMap<String, Value>) -> Self {
        let label = options
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_string();
        Self {
            request_count: AtomicUsize::new(0),
            response_count: AtomicUsize::new(0),
            label,
        }
    }
}

impl Middleware for StatsMiddleware {
    fn process_request<'a>(
        &'a self,
        _context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            let n = self.request_count.fetch_add(1, Ordering::Relaxed) + 1;
            halo_spider::trace::info(
                "stats.request",
                vec![
                    halo_spider::trace::prop("label", self.label.as_str()),
                    halo_spider::trace::prop("requests", n),
                ],
            );
            Ok(Flow::Continue)
        })
    }

    fn process_response<'a>(
        &'a self,
        _context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            let n = self.response_count.fetch_add(1, Ordering::Relaxed) + 1;
            halo_spider::trace::info(
                "stats.response",
                vec![
                    halo_spider::trace::prop("label", self.label.as_str()),
                    halo_spider::trace::prop("responses", n),
                ],
            );
            Ok(Flow::Continue)
        })
    }
}

// Spider used by the example.

struct PeriodSpider;

impl Spider for PeriodSpider {
    fn name(&self) -> &str {
        "period_plugin_demo"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://ep.shxwcb.com/2026/03/period.xml".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        Ok(Output {
            items: vec![],
            requests: vec![
                response
                    .follow_with_meta(&latest_edition_url(response)?, &latest_meta(response)?)
                    .with_callback(cb!(Self::parse_edition)),
            ],
        })
    }

    spider_callbacks!(parse, parse_edition);
}

impl PeriodSpider {
    async fn parse_edition(&self, response: &Response) -> Result<Output, SpiderError> {
        let period_date = response
            .meta
            .get("period_date")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let front_page = response
            .meta
            .get("front_page")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let title = response.css("h2.S10_bb").text().one().unwrap_or_default();

        Ok(Output {
            items: vec![
                Item::new()
                    .with_field("period_date", Value::String(period_date.to_string()))
                    .with_field("front_page", Value::String(front_page.to_string()))
                    .with_field("edition_title", Value::String(title))
                    .with_field("url", Value::String(response.url.clone())),
            ],
            requests: vec![],
        })
    }
}

// Demonstrate how `(kind, name)` identity works inside `PluginRegistry`.

fn demo_plugin_key_conflicts() {
    println!("-- Demonstrating (kind, name) identity rules --\n");

    let mut registry = PluginRegistry::new();

    // 1) 注册 (middleware, proxy)
    registry
        .register(PluginManifest {
            name: "proxy".to_string(),
            kind: "middleware".to_string(),
            entry: "builtin::Proxy".to_string(),
            r#override: false,
        })
        .unwrap();
    println!("  [OK] registered (middleware, proxy)");

    // 2) Same name under a different kind is allowed: `(store, proxy)`.
    //    This demonstrates registry namespacing only. It does not mean the
    //    engine can auto-load `store` plugins today.
    registry
        .register(PluginManifest {
            name: "proxy".to_string(),
            kind: "store".to_string(),
            entry: "custom::ProxyStorePlugin".to_string(),
            r#override: false,
        })
        .unwrap();
    println!("  [OK] registered (store, proxy) -- same name, different kind");

    // 3) Same kind plus same name without override causes a conflict.
    let err = registry
        .register(PluginManifest {
            name: "proxy".to_string(),
            kind: "middleware".to_string(),
            entry: "another::Proxy".to_string(),
            r#override: false,
        })
        .unwrap_err();
    println!("  [ERR] duplicate (middleware, proxy) with override=false -> {err}");

    // 4) Same kind plus same name with override replaces the previous entry.
    registry
        .register(PluginManifest {
            name: "proxy".to_string(),
            kind: "middleware".to_string(),
            entry: "another::Proxy".to_string(),
            r#override: true,
        })
        .unwrap();
    let updated = registry.get("middleware", "proxy").unwrap();
    println!(
        "  [OK] registered (middleware, proxy) with override=true -> replaced by '{}'",
        updated.entry
    );

    println!(
        "\n  registry now holds {} manifests: middleware={}, store={}\n",
        registry.manifests.len(),
        registry.by_kind("middleware").len(),
        registry.by_kind("store").len(),
    );
}

// Main entry.

#[tokio::main]
async fn main() {
    halo_spider::trace::init_console();

    // Part 1: demonstrate registry identity and override rules.
    demo_plugin_key_conflicts();

    // Part 2: load from plugins.toml and run the engine.
    println!("-- Loading plugins.toml and running the engine --\n");

    // Step 1: load plugin manifests from plugins.toml
    let manifest_path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/plugins.toml");
    let manifests = load_plugin_manifest(manifest_path).expect("failed to load plugins.toml");

    println!(
        "  loaded {} plugin manifests from plugins.toml:",
        manifests.len()
    );
    for m in &manifests {
        println!(
            "    - ({}, {}) entry={} override={}",
            m.kind, m.name, m.entry, m.r#override
        );
    }
    println!();

    // Step 2: register manifests into PluginRegistry
    let mut registry = PluginRegistry::new();
    registry
        .register_all(manifests)
        .expect("plugin manifest conflict");

    // Step 3: enable middleware by key in Settings
    let settings = Settings::default()
        .with_download_delay(SignedDuration::from_millis(300))
        .with_idle_timeout(SignedDuration::from_secs(3))
        .with_middleware(
            "custom_signature",
            Config {
                enabled: true,
                stage: Stage::Download,
                order: 50,
                options: BTreeMap::from([(
                    "secret".to_string(),
                    Value::String("my-app-secret-key".to_string()),
                )]),
            },
        )
        .with_middleware(
            "stats",
            Config {
                enabled: true,
                stage: Stage::Download,
                order: 10,
                options: BTreeMap::from([(
                    "label".to_string(),
                    Value::String("global".to_string()),
                )]),
            },
        );

    // Step 4: build the engine, register factories, then load plugins
    let engine = Engine::new()
        .with_settings(settings)
        // Factory keys must match the manifest `name` values.
        .register_middleware("custom_signature", |options| {
            Ok(Box::new(CustomSignatureMiddleware::new(options)))
        })
        .register_middleware("stats", |options| {
            Ok(Box::new(StatsMiddleware::new(options)))
        })
        // Verify that every declared middleware manifest has a factory.
        .load_plugins(&registry)
        .expect("failed to load plugins");

    println!("  plugins loaded, engine is ready\n");
    println!("  middleware order: stats(10) -> custom_signature(50)");
    println!("  press Ctrl+C to stop\n");

    // Step 5: run
    let mut engine = engine;
    let handle = engine.shutdown_handle();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        println!("received Ctrl+C, stopping engine...");
        handle.stop();
    });

    let spider = PeriodSpider;

    match engine.run(&spider).await {
        Ok(outputs) => {
            let total: usize = outputs.iter().map(|o| o.items.len()).sum();
            println!("\n=== Done ===");
            println!("{} run(s), {} item(s) total", outputs.len(), total);
        }
        Err(e) => eprintln!("error: {e}"),
    }
}

fn latest_period(response: &Response) -> Result<(String, String), SpiderError> {
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

fn latest_meta(response: &Response) -> Result<BTreeMap<String, Value>, SpiderError> {
    let (period_date, front_page) = latest_period(response)?;
    Ok(BTreeMap::from([
        ("period_date".to_string(), Value::String(period_date)),
        ("front_page".to_string(), Value::String(front_page)),
    ]))
}

fn latest_edition_url(response: &Response) -> Result<String, SpiderError> {
    let (period_date, front_page) = latest_period(response)?;
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
