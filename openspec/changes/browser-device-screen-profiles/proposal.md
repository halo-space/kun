# 变更提案

## 为什么做

- 当前 browser 配置已经支持 `fingerprint_preset`、`fingerprint_profile`、`stealth` 与 `stealth_script`，但公开 API 同时存在 preset 与 profile 两套入口，语义重复，而且带着明显的历史包袱。
- 现有扁平 `fingerprint_profile` 也没有把屏幕相关字段和浏览器画像字段明确分层；`viewport` 仍然挂在 `Config` 顶层，调用方难以直观看出它和 `screen` 指纹之间的关系。
- 对 `halo-spider` 来说，这次更值得做的是把 browser 公开画像模型收口成一个统一入口：
  - `DeviceProfile`
    - `fingerprint`
    - `screen`
- 这样可以删除公开 preset 概念、删除旧内部扁平模型，并把 `viewport` 一并纳入 `screen` 子结构，让后续的 session 级稳定画像和 DSL 配置化表达都建立在同一套公开模型上。
- 这也为以后其它画像类能力预留了稳定分层方式：只要属于“浏览器呈现出来的身份”，都可以继续挂在 profile 体系下；而 `keep_alive`、`proxy`、`session`、`wait_for_selector` 这类运行策略继续留在 `Config` 外层。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/spider-api/spec.md`
- 会影响哪些模块 / 示例：
  - `src/request/browser.rs`
  - `src/download/browser.rs`
  - `README.md`
  - `docs/capabilities.md`
  - `examples/browser_advanced.rs`
- 预期带来哪些用户可见结果：
  - browser 配置对外只保留 `DeviceProfile`
  - `DeviceProfile` 统一承接 `fingerprint` 与 `screen` 两组画像
  - `DeviceProfile` 本身可选；`fingerprint` 与 `screen` 也都可选
  - 公开 `fingerprint_preset` 会被删除，不再保留第二套入口
  - 公开 `screen` 画像会用 `viewport / screen / avail` 三个子结构承接屏幕 / 布局语义
  - 旧的内部扁平 `FingerprintProfile` 不再继续保留为历史模型

## 非目标

- 这次不直接实现完整的第三方 stealth 套件接入，也不承诺品牌级完美伪装。
- 这次不扩成通用浏览器自动化配置面，不引入点击、脚本执行、滚动等页面动作 DSL。
- 这次不一次性覆盖所有高级探针字段；优先收口公开 `DeviceProfile` 这一层主结构。
- 这次不把 `keep_alive`、session 复用策略或 browser runtime 池化策略混入同一个 change。

## 风险

- 是否存在兼容性或迁移风险：
  - 存在。当前调用方可能已经依赖 `fingerprint_preset`、顶层 `viewport` 与旧 `fingerprint_profile` 字段；如果直接替换公开模型，需要明确迁移路径与示例更新策略。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - browser downloader 当前已经依赖 profile 生成 init script、headers 与签名校验；如果新的 `DeviceProfile` 分层与内部执行计划的编译边界拆分不当，容易让 request 配置、下载器行为与文档再次失配。
