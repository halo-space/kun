# DSL 实现问题记录

这份问题记录保留为早期 DSL 探索笔记，但其中多条内容已经被后续实现或设计调整覆盖。

已被覆盖的点：

- DSL 动态构造 URL 不再只依赖 links 文本提取；`next_url_config` 现在已经支持 `FIELD`、`TEMPLATE`、`JOIN`、`FUNCTION`。
- `meta_patch` 已被移除；当前参数透传语义以 `step.meta` 与 `links[].meta` 为准。
- `links` 仍然负责链接提取；“提取文本后构造 URL” 应通过 `next_url_config` 或代码回调完成，而不是重新引入旧字段。

目前仍然成立、但暂未继续推进的点：

- DSL 还没有统一的日期/时间表达式能力。
- DSL 配置面整体后置，优先跟随代码爬虫与共享底层能力一起收敛。

当前 DSL 真实状态请以 README、`openspec/specs/rules-dsl/spec.md` 与 `src/rules/` 下的实现为准。
