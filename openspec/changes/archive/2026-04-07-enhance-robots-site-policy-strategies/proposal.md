# 变更提案

## 为什么做

- 当前 `robots::Memory` 的站点策略 overlay 仍然围绕“exact origin -> SitePolicy”组织，这对单站点还够用，但一旦 crawler 需要同时覆盖多个子域名、整组站点或一类 host pattern，调用方就只能重复写很多条 origin 配置，或者直接退回自定义 `Robot`。
- 这让内置 `robots` 组件在“更高阶站点策略”这条核心能力线上还不够顺手，也和 README / `docs/capabilities.md` 里已经明确标出的剩余缺口一致。
- 这次变更要直接把这条 API 改成面向 site matcher 的模型，而不是继续围绕 exact-origin 做兼容补丁。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/runtime-engine/spec.md`
- 会影响哪些模块 / 示例：
  - `src/robots.rs`
  - `README.md`
  - `docs/capabilities.md`
  - `TODO.md`
- 预期带来哪些用户可见结果：
  - `robots` 站点策略不再只支持 exact origin
  - 调用方可以直接声明更高阶的 site matcher，而不是为多个子域名重复注册 policy
  - `robots` 站点策略的 precedence / merge 语义会被明确写进实现和规范

## 非目标

- 不在这次变更里重写 `robots.txt` parser 主体；`Allow` / `Disallow`、`Crawl-delay`、`Request-rate`、wildcard path 规则继续沿用现有实现
- 不在这次变更里引入新的 DSL 配置面或 plugin kind
- 不把 site policy 扩成 path-level 手工覆盖规则；这次只解决 site / host / pattern 级别的站点策略边界

## 风险

- 是否存在兼容性或迁移风险：
  - 有。这次会直接调整 `robots::Memory` 的 site policy API，不再把旧的 exact-origin-only 入口当成主语义保留
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - 有。site matcher precedence / merge 规则如果不清楚，容易让 `access`、`delay`、`sitemap` 与 `unavailable_policy` 的最终行为变得难以预测，因此这次必须把运行时语义和测试一起补齐
