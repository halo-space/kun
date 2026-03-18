//! 插件系统完整示例
//!
//! 演示 plugins.toml 声明式加载、(kind, name) 唯一标识、override 冲突规则，
//! 以及如何将插件自动接入引擎。
//!
//! 流程：
//! 1. 插件作者实现中间件 + 工厂函数
//! 2. plugins.toml 声明插件（name, kind, entry, override）
//! 3. 引擎加载清单 → PluginRegistry 验证 → 工厂自动接入
//! 4. 最终用户只需在 Settings/MIDDLEWARES 中按名字启用
//!
//! 运行：cargo run --example plugins_demo
//! 按 Ctrl+C 优雅退出

use halo_spider::download::{BrowserDownloader, HttpDownloader};
use halo_spider::engine::Engine;
use halo_spider::engine::context::EngineContext;
use halo_spider::engine::types::Flow;
use halo_spider::error::SpiderError;
use halo_spider::future::BoxFuture;
use halo_spider::middleware::traits::Middleware;
use halo_spider::middleware::types::{MiddlewareConfig, MiddlewareType};
use halo_spider::plugins::{PluginManifest, PluginRegistry, load_plugin_manifest};
use halo_spider::response::Response;
use halo_spider::scheduler::memory::MemoryScheduler;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::value::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 插件作者实现的中间件
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 自定义签名中间件：为每个请求添加 X-Signature header。
/// 对应 plugins.toml 中的 (middleware, custom_signature)。
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
            tracing::info!(
                url = context.request.url.as_str(),
                signature = sig.as_str(),
                "[CustomSignature] 已签名"
            );
            Ok(Flow::Continue)
        })
    }
}

/// 统计中间件：记录请求/响应计数。
/// 对应 plugins.toml 中的 (middleware, stats)。
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
            tracing::info!(label = self.label.as_str(), count = n, "[Stats] 请求 #{n}");
            Ok(Flow::Continue)
        })
    }

    fn process_response<'a>(
        &'a self,
        _context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            let n = self.response_count.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::info!(label = self.label.as_str(), count = n, "[Stats] 响应 #{n}");
            Ok(Flow::Continue)
        })
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Spider
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

struct QuotesSpider;

impl Spider for QuotesSpider {
    fn name(&self) -> &str {
        "quotes_plugin_demo"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        let texts = response.css("div.quote span.text::text").all();
        let authors = response.css("div.quote small.author::text").all();

        let items: Vec<_> = texts
            .into_iter()
            .zip(authors)
            .map(|(text, author)| {
                halo_spider::item::Item::new()
                    .with_field("text", Value::String(text))
                    .with_field("author", Value::String(author))
            })
            .collect();

        tracing::info!(count = items.len(), "解析出 {} 条名言", items.len());

        Ok(Output {
            items,
            requests: vec![],
        })
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 演示 (kind, name) 冲突和 override 规则
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn demo_conflict_rules() {
    println!("── 演示 (kind, name) 唯一标识规则 ──\n");

    let mut registry = PluginRegistry::new();

    // 1) 注册 (middleware, proxy)
    registry
        .register(PluginManifest {
            name: "proxy".to_string(),
            kind: "middleware".to_string(),
            entry: "builtin::ProxyMiddleware".to_string(),
            r#override: false,
        })
        .unwrap();
    println!("  [OK] 注册 (middleware, proxy)");

    // 2) 不同 kind 同名 — 允许：(rules, proxy)
    registry
        .register(PluginManifest {
            name: "proxy".to_string(),
            kind: "rules".to_string(),
            entry: "custom::ProxyRulesPlugin".to_string(),
            r#override: false,
        })
        .unwrap();
    println!("  [OK] 注册 (rules, proxy) — 不同 kind 同名，允许共存");

    // 3) 同 kind 同 name 无 override — 冲突
    let err = registry
        .register(PluginManifest {
            name: "proxy".to_string(),
            kind: "middleware".to_string(),
            entry: "another::ProxyMiddleware".to_string(),
            r#override: false,
        })
        .unwrap_err();
    println!("  [ERR] 再次注册 (middleware, proxy) override=false → {err}");

    // 4) 同 kind 同 name 有 override — 覆盖成功
    registry
        .register(PluginManifest {
            name: "proxy".to_string(),
            kind: "middleware".to_string(),
            entry: "another::ProxyMiddleware".to_string(),
            r#override: true,
        })
        .unwrap();
    let updated = registry.get("middleware", "proxy").unwrap();
    println!(
        "  [OK] 注册 (middleware, proxy) override=true → 已替换为 '{}'",
        updated.entry
    );

    println!(
        "\n  注册表共 {} 个插件：middleware={}, rules={}\n",
        registry.manifests.len(),
        registry.by_kind("middleware").len(),
        registry.by_kind("rules").len(),
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 主入口
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    // ── Part 1: 演示冲突规则 ──
    demo_conflict_rules();

    // ── Part 2: 从 plugins.toml 加载并运行 ──
    println!("── 从 plugins.toml 加载插件并运行引擎 ──\n");

    // Step 1: 加载 plugins.toml 清单
    let manifest_path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/plugins.toml");
    let manifests = load_plugin_manifest(manifest_path).expect("无法加载 plugins.toml");

    println!("  从 plugins.toml 读取到 {} 个插件声明：", manifests.len());
    for m in &manifests {
        println!(
            "    - ({}, {}) entry={} override={}",
            m.kind, m.name, m.entry, m.r#override
        );
    }
    println!();

    // Step 2: 注册到 PluginRegistry（验证 (kind, name) 唯一性）
    let mut registry = PluginRegistry::new();
    registry
        .register_all(manifests)
        .expect("插件注册冲突");

    // Step 3: 最终用户在 Settings 中按名字启用中间件插件
    let settings = Settings::default()
        .download_delay(Duration::from_millis(300))
        .idle_timeout(Duration::from_secs(3))
        .with_middleware(
            "custom_signature",
            MiddlewareConfig {
                enabled: true,
                r#type: MiddlewareType::Download,
                order: 50,
                options: BTreeMap::from([(
                    "secret".to_string(),
                    Value::String("my-app-secret-key".to_string()),
                )]),
            },
        )
        .with_middleware(
            "stats",
            MiddlewareConfig {
                enabled: true,
                r#type: MiddlewareType::Download,
                order: 10,
                options: BTreeMap::from([(
                    "label".to_string(),
                    Value::String("global".to_string()),
                )]),
            },
        );

    // Step 4: 构建引擎，注册工厂，加载插件
    let engine = Engine::new(
        MemoryScheduler::default(),
        HttpDownloader::default(),
        BrowserDownloader::default(),
    )
    .with_settings(settings)
    // 插件作者注册工厂函数（名称必须与 plugins.toml 中的 name 对应）
    .register_middleware("custom_signature", |options| {
        Ok(Box::new(CustomSignatureMiddleware::new(options)))
    })
    .register_middleware("stats", |options| {
        Ok(Box::new(StatsMiddleware::new(options)))
    })
    // 验证：所有 plugins.toml 中声明的 middleware 插件都有工厂
    .load_plugins(&registry)
    .expect("插件加载失败");

    println!("  插件加载完成，引擎就绪\n");
    println!("  中间件执行顺序：stats(10) → custom_signature(50)");
    println!("  按 Ctrl+C 停止\n");

    // Step 5: 运行
    let mut engine = engine;
    let handle = engine.shutdown_handle();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("收到 Ctrl+C，停止引擎...");
        handle.stop();
    });

    match engine.run(&QuotesSpider).await {
        Ok(outputs) => {
            let total: usize = outputs.iter().map(|o| o.items.len()).sum();
            println!("\n=== 完成 ===");
            println!("共 {} 轮，{} 个 items", outputs.len(), total);
        }
        Err(e) => eprintln!("出错: {e}"),
    }
}
