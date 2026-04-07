# 规范增量

## MODIFIED Requirements

### Requirement: Callers can overlay explicit site policy on robots memory

系统 MUST 让 `robots::Memory` 的站点策略入口围绕显式 site matcher 建模，而不是继续固定为 exact-origin-only 配置。

#### Scenario: Site policy targets can match more than one exact origin

- **WHEN** 调用方保留内置 `robots::Memory`
- **AND** 它通过 `robots::Memory::with_site_policy(...)` 配置显式 site matcher
- **THEN** 这条站点策略可以匹配 exact origin、host，或更高阶的 host pattern
- **AND** 调用方不需要为了多个相关站点重复退回手写 `Robot`

#### Scenario: More specific site matchers win access and unavailable overrides

- **WHEN** 同一个请求同时命中多条 site policy
- **THEN** `access` 与 `unavailable_policy` 由更具体的 matcher 决定
- **AND** 当 matcher specificity 相同，系统使用稳定、可测试的 tie-break 规则

#### Scenario: Delay stays strict across robots and matched site policies

- **WHEN** 某个请求同时命中 `robots.txt` delay 和一条或多条 site policy delay
- **THEN** 最终 delay 取更严格的那个值
- **AND** 不会因为更具体 matcher 覆盖 access 就丢掉更严格的 delay

#### Scenario: Sitemaps are merged across robots and matched site policies

- **WHEN** 某个请求命中的 site policy 补充了额外 sitemap
- **THEN** 这些 sitemap 会并入当前站点最终可见的 sitemap 集合
- **AND** 与 `robots.txt` 自身声明的 sitemap 保持去重后的合并语义
