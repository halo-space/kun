# Proposal: DSL next_url_config and type Semantics

## Problem

当前 kun 的 DSL 实现存在以下限制：

1. **无法动态构造 URL**：只能提取现有链接，无法通过模板或字段拼接生成新 URL
2. **缺少 type 语义**：无法区分中间层（node）和最终层（end），所有 step 都会产出 items
3. **meta 传递不完整**：parsed_fields 无法自动传递到下一个 step
4. **单一 parse 函数未实现**：当前需要为每个 step 写不同的回调函数

## Solution

实现与当前共享底层能力一致的 DSL `next_url_config` 与 `type` 语义：

### 1. next_url_config 支持

支持 4 种模式：
- **FIELD**：从字段直接取值作为 URL
- **TEMPLATE**：模板替换 `{field}` 和 `{meta.xxx}`
- **JOIN**：多字段拼接
- **FUNCTION**：自定义函数生成

### 2. type="node" vs "end" 语义

- `type="node"`：中间层，不保存 items，只生成 next_urls
- `type="end"`：最终层，保存 items，不生成 next_urls

### 3. meta 自动透传

parsed_fields 自动进入 meta，供下一个 step 使用

### 4. 单一 parse 函数

所有 step 都调用同一个 `parse` 函数，DSL 引擎根据 step.idx 应用不同规则

## Benefits

1. **纯 DSL 实现多级爬取**：无需编写代码回调
2. **配置即代码**：通过 JSON 配置完成复杂爬取逻辑
3. **对齐底层能力**：避免 DSL 继续发明独立语义
4. **简化开发**：开发者只需定义一个 parse 函数

## Example

```jsonc
{
  "steps": [
    {
      "idx": 0,
      "name": "parse_xml",
      "type": "node",
      "parse": {
        "mode": "RULE_ONLY",
        "rule": [{"name": "front_page", "options": [{...}]}]
      },
      "validate": [{"name": "front_page", "type": "text", "rule": {"required": true}}],
      "next_url_config": {
        "mode": "TEMPLATE",
        "template": "https://ep.shxwcb.com/2026/03/27/{front_page}?f=2026/03/period.xml"
      }
    },
    {
      "idx": 1,
      "name": "parse_list",
      "type": "node",
      "parse": {...},
      "validate": [{"name": "detail_url", "type": "text", "rule": {"required": true}}],
      "next_url_config": {
        "mode": "FIELD",
        "from": ["detail_url"]
      }
    },
    {
      "idx": 2,
      "name": "parse_detail",
      "type": "end",
      "parse": {...},
      "validate": [
        {"name": "title", "type": "text", "rule": {"required": true}},
        {"name": "content", "type": "text", "rule": {"required": false}}
      ]
    }
  ]
}
```

## Risks

- 需要重构现有 DSL 引擎的 dispatch 逻辑
- 需要在 DSL 语义调整时继续保持 callback 路由和现有 step 配置的清晰边界
