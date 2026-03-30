# Design: DSL next_url_config and type Semantics

## 核心理念

**DSL 和代码共用同一套底层逻辑**

```
代码方式（程序员）          DSL 方式（运营）
      ↓                         ↓
  parse() 函数              JSON 配置
      ↓                         ↓
      └─────────┬───────────────┘
                ↓
          统一解析引擎
                ↓
        提取字段 + 生成请求
```

## 统一的执行流程

```rust
// 1. 代码方式
impl MySpider {
    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        // 手写提取逻辑
        let front_page = response.xml("//front_page").text().last()?;

        // 手写 URL 构造
        let url = format!("https://ep.shxwcb.com/2026/03/27/{}", front_page);

        // 手写 meta 传递
        let mut meta = response.meta().clone();
        meta.insert("front_page", front_page);

        // 返回 requests
        Ok(Output {
            items: vec![],  // type="node" 不保存
            requests: vec![response.follow_with_meta(&url, &meta)]
        })
    }
}

// 2. DSL 方式（等价）
{
  "type": "node",
  "parse": {
    "rule": [{"name": "front_page", "options": [{
      "selector_type": "XPATH",
      "selector": ["//front_page"],
      "attribute": "text"
    }]}]
  },
  "next_url_config": {
    "mode": "TEMPLATE",
    "template": "https://ep.shxwcb.com/2026/03/27/{front_page}"
  }
}
```

## 架构设计

### 1. 统一的 Output 结构

```rust
pub struct Output {
    pub items: Vec<Item>,      // type="end" 时有值
    pub requests: Vec<Request>, // type="node" 时有值
}
```

### 2. 统一的 dispatch 逻辑

```rust
async fn dispatch(&self, response: &Response, compiled: Option<&Compiled>)
    -> Result<Output, SpiderError>
{
    // 优先检查代码回调
    if let Some(request) = &response.request {
        if let Some(callback_target) = &request.callback {
            return self.call(&callback_target.name, response).await;
        }
    }

    // 否则使用 DSL
    if let Some(compiled) = compiled {
        return self.parse_with_dsl(response, compiled).await;
    }

    // 默认调用 parse
    self.parse(response).await
}
```

### 3. DSL 解析引擎

```rust
async fn parse_with_dsl(&self, response: &Response, compiled: &Compiled)
    -> Result<Output, SpiderError>
{
    let step = self.get_current_step(response, compiled)?;

    // 提取字段（与代码方式相同的逻辑）
    let parsed_fields = extract_fields(response, &step.parse)?;

    // 根据 type 决定输出
    match step.r#type {
        StepType::Node => {
            // 生成 next_urls（与代码方式相同的逻辑）
            let urls = build_next_urls(response, step, &parsed_fields)?;
            let requests = create_requests(response, urls, &parsed_fields)?;
            Ok(Output { items: vec![], requests })
        }
        StepType::End => {
            Ok(Output { items: vec![parsed_fields], requests: vec![] })
        }
    }
}
```

## 关键实现

### 1. next_url_config 模板替换

```rust
fn build_from_template(
    template: &str,
    parsed_fields: &HashMap<String, Value>,
    meta: &HashMap<String, Value>,
) -> String {
    let mut url = template.to_string();

    // 替换 {field}
    for (key, value) in parsed_fields {
        url = url.replace(&format!("{{{}}}", key), &value.to_string());
    }

    // 替换 {meta.xxx}
    for (key, value) in meta {
        url = url.replace(&format!("{{meta.{}}}", key), &value.to_string());
    }

    url
}
```

### 2. meta 自动透传

```rust
fn merge_meta(
    current_meta: &HashMap<String, Value>,
    parsed_fields: &HashMap<String, Value>,
) -> HashMap<String, Value> {
    let mut new_meta = current_meta.clone();

    // parsed_fields 自动进入 meta
    for (key, value) in parsed_fields {
        new_meta.insert(key.clone(), value.clone());
    }

    new_meta
}
```

### 3. 相对路径补全

```rust
fn resolve_url(base: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }

    // 使用 url crate 补全相对路径
    if let Ok(base_url) = url::Url::parse(base) {
        if let Ok(resolved) = base_url.join(url) {
            return resolved.to_string();
        }
    }

    url.to_string()
}
```

## 数据结构

```rust
// src/rules/config.rs

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub idx: usize,
    pub name: String,
    #[serde(rename = "type")]
    pub step_type: StepType,
    pub parse: ParseConfig,
    pub validate: Vec<ValidateField>,
    pub next_url_config: Option<NextUrlConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepType {
    Node,
    End,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NextUrlConfig {
    pub mode: NextUrlMode,
    pub from: Option<Vec<String>>,
    pub template: Option<String>,
    pub join_delimiter: Option<String>,
    #[serde(rename = "fn")]
    pub function: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NextUrlMode {
    Field,
    Template,
    Join,
    Function,
}
```

## 纯 DSL 模式

**默认就是 DSL，不需要 impl 字段**：

```jsonc
{
  "steps": [
    {
      "idx": 0,
      "type": "node",      // 业务语义
      "parse": {...}       // DSL 配置
    }
  ]
}
```

- `type="node"`：不保存 items，生成 next_urls
- `type="end"`：保存 items，不生成 next_urls

Spider 只需要定义一个 parse 函数作为入口：

```rust
impl Spider for MySpider {
    fn rules(&self) -> Option<RulesConfig> {
        Some(RulesConfig::local("config.json"))
    }

    spider_callbacks!(parse);
}

impl MySpider {
    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        // DSL 引擎自动处理，开发者无需关心
        Ok(Output::default())
    }
}
```
