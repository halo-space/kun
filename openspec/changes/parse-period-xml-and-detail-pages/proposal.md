# 变更提案

## 为什么做

- 验证 kun 现有架构能否优雅地实现"XML 索引 → 动态 URL 构造 → 列表页 → 详情页"的三级爬取流程
- 这是一个真实场景：从 period.xml 解析最新日期（如 03-27），拼接列表页 URL，再提取详情页链接
- 需要确认性能和代码可读性是否满足要求

## 范围

- 会影响哪些 capability specs：
  - `parsing-and-extraction.md`：验证 XML XPath 解析在实际场景中的表现
  - `request-and-response.md`：验证动态 URL 构造和 meta 传递机制
  - `callback-chaining.md`：验证三级回调链的实现方式

- 会影响哪些模块 / 示例：
  - 新增 `examples/period_xml_spider.rs` 展示完整流程
  - 不修改任何核心模块代码

- 预期带来哪些用户可见结果：
  - 一个可运行的示例，证明 kun 能处理复杂的多级爬取
  - 验证代码是否简洁、性能是否合理

## 非目标

- 不添加新的 XML 解析功能（使用现有的 XPath）
- 不修改 Spider trait 或 Engine 核心逻辑
- 不处理登录、验证码等复杂场景
- 不添加新的 middleware 或 pipeline

## 风险

- 是否存在兼容性或迁移风险：
  - 无，纯新增示例

- 是否存在 runtime、middleware 或 plugin 相关风险：
  - 无，使用现有机制
