# Specs: DSL next_url_config and type Semantics

## 1. 数据结构定义

### 1.1 Step 结构（纯 DSL）

**文件**: `src/rules/config.rs`

```rust
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
    Node,  // 中间层，不保存 items
    End,   // 最终层，保存 items
}
```

### 1.2 NextUrlConfig 结构

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct NextUrlConfig {
    pub mode: NextUrlMode,

    #[serde(default)]
    pub from: Option<Vec<String>>,

    #[serde(default)]
    pub template: Option<String>,

    #[serde(default)]
    pub join_delimiter: Option<String>,

    #[serde(rename = "fn", default)]
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

## 2. URL 构造逻辑

### 2.1 build_next_urls 函数

**文件**: `src/rules/url_builder.rs` (新建)

```rust
use std::collections::HashMap;
use serde_json::Value;
use crate::rules::config::{NextUrlConfig, NextUrlMode};
use crate::response::Response;
use crate::error::SpiderError;

pub fn build_next_urls(
    response: &Response,
    config: &NextUrlConfig,
    parsed_fields: &HashMap<String, Value>,
) -> Result<Vec<String>, SpiderError> {
    let urls = match &config.mode {
        NextUrlMode::Field => build_from_field(config, parsed_fields)?,
        NextUrlMode::Template => build_from_template(config, parsed_fields, response)?,
        NextUrlMode::Join => build_from_join(config, parsed_fields)?,
        NextUrlMode::Function => build_from_function(config, parsed_fields, response)?,
    };

    normalize_urls(response, urls)
}
```

### 2.2 FIELD 模式

```rust
fn build_from_field(
    config: &NextUrlConfig,
    parsed_fields: &HashMap<String, Value>,
) -> Result<Vec<String>, SpiderError> {
    let from = config.from.as_ref()
        .ok_or_else(|| SpiderError::parse("FIELD mode requires 'from'"))?;

    if from.len() != 1 {
        return Err(SpiderError::parse("FIELD mode requires exactly one field"));
    }

    let field_name = &from[0];
    let value = parsed_fields.get(field_name)
        .ok_or_else(|| SpiderError::parse(&format!("Field '{}' not found", field_name)))?;

    match value {
        Value::String(s) => Ok(vec![s.clone()]),
        Value::Array(arr) => {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
                .into()
        }
        _ => Err(SpiderError::parse("Field value must be string or array")),
    }
}
```

### 2.3 TEMPLATE 模式

```rust
fn build_from_template(
    config: &NextUrlConfig,
    parsed_fields: &HashMap<String, Value>,
    response: &Response,
) -> Result<Vec<String>, SpiderError> {
    let template = config.template.as_ref()
        .ok_or_else(|| SpiderError::parse("TEMPLATE mode requires 'template'"))?;

    let mut url = template.clone();

    // 替换 {field}
    for (key, value) in parsed_fields {
        let placeholder = format!("{{{}}}", key);
        if url.contains(&placeholder) {
            let value_str = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            url = url.replace(&placeholder, &value_str);
        }
    }

    // 替换 {meta.xxx}
    if let Some(meta) = response.meta() {
        for (key, value) in meta {
            let placeholder = format!("{{meta.{}}}", key);
            if url.contains(&placeholder) {
                url = url.replace(&placeholder, &value.to_string());
            }
        }
    }

    Ok(vec![url])
}
```

### 2.4 JOIN 模式

```rust
fn build_from_join(
    config: &NextUrlConfig,
    parsed_fields: &HashMap<String, Value>,
) -> Result<Vec<String>, SpiderError> {
    let from = config.from.as_ref()
        .ok_or_else(|| SpiderError::parse("JOIN mode requires 'from'"))?;

    if from.len() < 2 {
        return Err(SpiderError::parse("JOIN mode requires at least 2 fields"));
    }

    let delimiter = config.join_delimiter.as_deref().unwrap_or("");

    let parts: Vec<String> = from.iter()
        .filter_map(|field_name| {
            parsed_fields.get(field_name)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    if parts.len() != from.len() {
        return Err(SpiderError::parse("Some fields not found for JOIN"));
    }

    Ok(vec![parts.join(delimiter)])
}
```

### 2.5 URL 规范化

```rust
fn normalize_urls(
    response: &Response,
    urls: Vec<String>,
) -> Result<Vec<String>, SpiderError> {
    let mut normalized = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for url in urls {
        let url = url.trim();
        if url.is_empty() {
            continue;
        }

        // 相对路径补全
        let resolved = resolve_url(response.url(), url);

        // 只保留 http/https
        if !resolved.starts_with("http://") && !resolved.starts_with("https://") {
            continue;
        }

        // 去重
        if seen.insert(resolved.clone()) {
            normalized.push(resolved);
        }
    }

    Ok(normalized)
}

fn resolve_url(base: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }

    if let Ok(base_url) = url::Url::parse(base) {
        if let Ok(resolved) = base_url.join(url) {
            return resolved.to_string();
        }
    }

    url.to_string()
}
```

## 3. dispatch 逻辑修改

### 3.1 统一 dispatch 函数

**文件**: `src/spider.rs`

```rust
async fn dispatch(&self, response: &Response, compiled: Option<&Compiled>)
    -> Result<Output, SpiderError>
{
    // 1. 优先检查代码回调
    if let Some(request) = &response.request {
        if let Some(callback_target) = &request.callback {
            return self.call(&callback_target.name, response).await;
        }
    }

    // 2. 使用 DSL
    if let Some(compiled) = compiled {
        return self.parse_with_dsl(response, compiled).await;
    }

    // 3. 默认调用 parse
    self.parse(response).await
}
```

### 3.2 parse_with_dsl 实现

```rust
async fn parse_with_dsl(&self, response: &Response, compiled: &Compiled)
    -> Result<Output, SpiderError>
{
    let step = self.get_current_step(response, compiled)?;

    // 提取字段
    let parsed_fields = self.extract_fields(response, &step.parse).await?;

    // 校验字段
    self.validate_fields(&parsed_fields, &step.validate)?;

    // 根据 type 决定输出
    match step.step_type {
        StepType::Node => {
            // 生成 next_urls
            let urls = build_next_urls(response,
                step.next_url_config.as_ref()
                    .ok_or_else(|| SpiderError::parse("type=node requires next_url_config"))?,
                &parsed_fields
            )?;

            // 创建 requests，自动透传 meta
            let requests = self.create_requests_with_meta(response, urls, &parsed_fields)?;

            Ok(Output { items: vec![], requests })
        }
        StepType::End => {
            Ok(Output {
                items: vec![parsed_fields],
                requests: vec![]
            })
        }
    }
}
```

## 4. meta 自动透传

### 4.1 create_requests_with_meta

**文件**: `src/spider.rs`

```rust
fn create_requests_with_meta(
    &self,
    response: &Response,
    urls: Vec<String>,
    parsed_fields: &HashMap<String, Value>,
) -> Result<Vec<Request>, SpiderError> {
    let mut requests = Vec::new();

    // 合并 meta
    let mut new_meta = response.meta().cloned().unwrap_or_default();
    for (key, value) in parsed_fields {
        new_meta.insert(key.clone(), value.clone());
    }

    for url in urls {
        let req = response.follow_with_meta(&url, &new_meta)
            .with_callback(cb!(Self::parse));
        requests.push(req);
    }

    Ok(requests)
}
```

## 5. 获取当前 step

### 5.1 get_current_step

```rust
fn get_current_step<'a>(
    &self,
    response: &Response,
    compiled: &'a Compiled,
) -> Result<&'a Step, SpiderError> {
    // 从 meta 中获取 step_idx
    let step_idx = response.meta()
        .and_then(|m| m.get("step_idx"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    compiled.steps.get(step_idx)
        .ok_or_else(|| SpiderError::parse(&format!("Step {} not found", step_idx)))
}
```

## 6. 测试规范

### 6.1 单元测试

**文件**: `src/rules/url_builder.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_mode() {
        let mut fields = HashMap::new();
        fields.insert("url".to_string(), Value::String("http://example.com".to_string()));

        let config = NextUrlConfig {
            mode: NextUrlMode::Field,
            from: Some(vec!["url".to_string()]),
            template: None,
            join_delimiter: None,
            function: None,
        };

        let urls = build_from_field(&config, &fields).unwrap();
        assert_eq!(urls, vec!["http://example.com"]);
    }

    #[test]
    fn test_template_mode() {
        let mut fields = HashMap::new();
        fields.insert("id".to_string(), Value::String("123".to_string()));

        let config = NextUrlConfig {
            mode: NextUrlMode::Template,
            from: None,
            template: Some("https://example.com/p/{id}.html".to_string()),
            join_delimiter: None,
            function: None,
        };

        // 需要 mock response
        // let urls = build_from_template(&config, &fields, &response).unwrap();
        // assert_eq!(urls, vec!["https://example.com/p/123.html"]);
    }
}
```

