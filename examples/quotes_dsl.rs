//! DSL 配置模式示例
//!
//! 演示如何用 JSON DSL 规则文件驱动爬虫，零代码定义解析逻辑：
//! - 列表页通过 DSL 自动提取 text / author / tags
//! - 翻页链接和详情链接通过 DSL links 自动跟进
//! - 详情页同样通过 DSL 自动提取作者信息
//!
//! Spider 在构造时异步加载并编译规则，parse 内部根据 meta.next_step
//! 路由到对应的 DSL step 执行解析。解析产生的 Request 通过 meta
//! 透传 next_step，使后续回调自动匹配正确的 step。
//!
//! 运行方式：cargo run --example quotes_dsl

use spider::download::{BrowserDownloader, HttpDownloader};
use spider::engine::Engine;
use spider::error::SpiderError;
use spider::response::Response;
use spider::rules::{apply as apply_dsl, Compiled};
use spider::rules::compile::compile_rules;
use spider::scheduler::memory::MemoryScheduler;
use spider::spider::{Output, Spider};
use spider::value::Value;

struct QuotesDslSpider {
    compiled: Compiled,
}

impl QuotesDslSpider {
    async fn new(rules_path: &str) -> Result<Self, SpiderError> {
        let json = tokio::fs::read_to_string(rules_path)
            .await
            .map_err(|e| SpiderError::rules(format!("读取规则文件失败: {e}")))?;
        let compiled = compile_rules(Value::String(json))?;
        Ok(Self { compiled })
    }
}

impl Spider for QuotesDslSpider {
    fn name(&self) -> &str {
        "quotes_dsl"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        let step_id = response
            .meta
            .get("next_step")
            .and_then(|v| v.as_str())
            .unwrap_or("parse");

        let step = self
            .compiled
            .steps
            .iter()
            .find(|s| s.id == step_id)
            .ok_or_else(|| SpiderError::engine(format!("step 未找到: {step_id}")))?;

        let dsl_output = apply_dsl(response, step)?;

        Ok(Output {
            items: dsl_output.items,
            requests: dsl_output.requests,
        })
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let rules_path = "examples/rules/quotes.json";

    let spider = match QuotesDslSpider::new(rules_path).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("加载规则失败: {e}");
            return;
        }
    };

    let engine = Engine::new(
        MemoryScheduler::default(),
        HttpDownloader::default(),
        BrowserDownloader::default(),
    );

    let handle = engine.shutdown_handle();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("收到 Ctrl+C，停止引擎...");
        handle.stop();
    });

    println!("启动 Spider: {}", spider.name());
    println!("入口 URL: {:?}", spider.start_urls());
    println!("规则来源: {rules_path}");
    println!();

    let mut engine = engine;
    match engine.run(&spider).await {
        Ok(outputs) => {
            println!("=== 抓取完成 ===");
            println!("总共 {} 轮输出\n", outputs.len());

            let mut total_items = 0;
            let mut total_follows = 0;

            for (i, output) in outputs.iter().enumerate() {
                total_items += output.items.len();
                total_follows += output.requests.len();

                println!("--- 第 {} 轮 ---", i + 1);
                for item in &output.items {
                    if let Some(text) = item.get("text") {
                        println!("  text: {:?}", text);
                    }
                    if let Some(author) = item.get("author") {
                        println!("  author: {:?}", author);
                    }
                    if let Some(tags) = item.get("tags") {
                        println!("  tags: {:?}", tags);
                    }
                    if let Some(name) = item.get("name") {
                        println!("  name: {:?}", name);
                    }
                    if let Some(born_date) = item.get("born_date") {
                        println!("  born_date: {:?}", born_date);
                    }
                    println!();
                }
            }

            println!(
                "总计: {} 个 items, {} 个跟进请求",
                total_items, total_follows
            );
        }
        Err(e) => {
            eprintln!("抓取失败: {e}");
        }
    }
}
