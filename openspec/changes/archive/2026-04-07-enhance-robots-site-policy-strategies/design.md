# 技术设计

## 概览

- 这次不走兼容补丁路线，而是直接把 `robots::Memory` 的站点策略入口从“origin -> policy”改成“site matcher -> policy”。
- 目标不是重写整套 `robots.txt` parser，而是把 overlay 这层真正升级成更高阶站点策略能力：请求先按 URL 命中 site matcher，再把命中的站点策略和原始 `robots.txt` 语义合并成最终决策。

## 模块影响

- `src/robots.rs`
  - 重新设计公开的 site matcher / site policy API
  - 把当前 `BTreeMap<String, SitePolicy>` 这种 exact-origin 存储改成可表达 matcher 与注册顺序的结构
  - 明确 access / unavailable_policy / delay / sitemap 的合并规则
- `README.md`
  - 更新 robots 能力描述，去掉“只支持 exact origin”的旧表述
- `docs/capabilities.md`
  - 更新 site policy 使用方式和边界说明
- `TODO.md`
  - 移除这条已被新 change 吸收的 robots 旧缺口描述
- `openspec/specs/runtime-engine/spec.md`
  - 同步新的 site policy matcher 与 precedence / merge 语义

## 关键决策

- Runtime / middleware 影响：
  - 这次仍然只影响 `robots` owner，不新增 middleware，也不改 `Engine::with_robots(...)` 的装配边界
  - runtime 上的关键变化是：`check(...)` / `sitemaps(...)` 在读取 site policy overlay 时，不再假设只有唯一 exact-origin 命中
- 对外 API 影响：
  - `robots::Memory` 的 site policy 配置入口直接改成显式 site matcher 模型
  - 旧的 exact-origin-only 心智不再作为推荐或内部主结构保留
- Plugin 或 DSL 影响：
  - 无新增 plugin kind
  - 无新增 DSL 配置面；这次仍然属于代码爬虫与 runtime 的公开底层能力
- precedence / merge 规则：
  - `access` 与 `unavailable_policy` 采用“最具体 matcher 优先 + 稳定 tie-break”语义
  - `delay` 继续按更严格值生效
  - `sitemaps` 继续按去重 union 合并
  - 这样可以避免一条 broad policy 覆盖更具体站点的 allow/disallow，但仍保留 broad policy 提供的安全 delay 或额外 sitemap

## 验证方式

- 在 `src/robots.rs` 补单元测试，覆盖：
  - exact origin / host / wildcard 或 suffix matcher 命中
  - 多条 policy 同时命中时的 precedence
  - `delay` 取最大值
  - `sitemaps` union
  - `unavailable_policy` 的具体 matcher 覆盖
- 同步 `README.md`、`docs/capabilities.md`、`TODO.md` 与 `openspec/specs/runtime-engine/spec.md`
- 运行：
  - `cargo test --quiet`
