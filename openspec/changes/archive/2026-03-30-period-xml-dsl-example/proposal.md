# 变更提案

## 为什么做

- 已有代码示例展示了三级爬取，但缺少 DSL 配置示例
- 用户需要了解如何用 JSON DSL 实现同样的多级爬取场景
- 验证 DSL 是否能完整支持 XML 解析、动态 URL 构造、回调链等功能

当前说明：

- 这份 proposal 保留为早期 DSL 示例探索记录。
- 当前 `examples/` 已不再保留 DSL 示例文件，优先只展示已落地底层能力。
- 如果未来重新引入 DSL 示例，应基于当前 README、`openspec/specs/rules-dsl/spec.md` 与真实实现重新规划。

## 范围

- 会影响哪些 capability specs：
  - `dsl-configuration.md`：展示 DSL 配置三级爬取的完整示例
  - `parsing-and-extraction.md`：补充 XML 解析的 DSL 配置

- 会影响哪些模块 / 示例：
  - 当时计划新增 `examples/period_xml_dsl.json` DSL 配置文件
  - 当时计划新增 `examples/period_xml_dsl.rs` 加载 DSL 的示例代码
  - 不修改核心模块

- 预期带来哪些用户可见结果：
  - 用户可以参考 DSL 配置实现复杂爬取
  - 对比代码模式和 DSL 模式的差异

## 非目标

- 不添加新的 DSL 语法
- 不修改 DSL 解析逻辑
- 不改变现有 API

## 风险

- 是否存在兼容性或迁移风险：无
- 是否存在 runtime、middleware 或 plugin 相关风险：无
