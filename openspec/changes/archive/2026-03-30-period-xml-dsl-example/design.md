# 技术设计

这份设计稿保留为早期 DSL 示例探索记录，当前已不再作为维护中的示例方案。

当前状态：

- `examples/` 目录已经回到“只保留已落地底层能力示例”的策略。
- `meta_patch` 已经被移除，参数透传以 `step.meta` 和 `links[].meta` 为准。
- `period.xml` 相关现行示例以代码爬虫版本为准，见 `examples/period_xml_spider.rs`。

如果未来重新补 DSL 示例，应直接基于当前 README、`openspec/specs/rules-dsl/spec.md` 与真实实现重新设计，而不是继续沿用这份旧草稿。
