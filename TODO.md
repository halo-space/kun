# TODO

这里只保留当前仍然成立、并且会影响后续实现取舍的能力缺口。
更细的任务拆分以 `openspec/changes/track-kun-core-capability-gaps/tasks.md` 为准。

## 1. Parser 缺口

- HTML XPath 仍然暂缓。当前 XPath 基于 XML 解析器，在不规范 HTML 上不稳定，HTML 场景继续建议优先使用 CSS。
- `ocr` 相关解析能力仍未实现，当前不纳入落地范围。
- parse 后处理已经补了一组更完整的 query transform：`fallback(...)`、`fallback_many(...)`、`field(...)`、`index(...)`、`flatten()`、`compact()`、`trim()`、`first_non_empty()`、`skip(...)`、`take(...)`、`last()`、`dedup()`、`join(...)`、`split(...)`、`replace(...)`、`normalize_whitespace()`、`resolve_url(...)`、`parse_number()`、`parse_bool()`、`parse_json()`、`parse_datetime()`、`parse_datetime_with_format(...)`。
- query 级最小断言也已补：`require_non_empty()`、`require_one()`。
- 结构化 map/filter、多选择器策略收口与 parser 后处理语义还没有统一抽象；当前已经补到多 query 兜底、结构投影、数组拉平、结果切片/去重、string transform、URL resolve、embedded JSON parse、number/bool/datetime conversion 与 query-level assertions。

## 2. Validation 缺口

- 共享 `Validation` 目前已经支持 `field`、`value_type`、`rule.required`、`rule.regex`、`rule.min/max`、`rule.enum_values`、文本/列表/对象的显式大小约束，以及最小字段路径解析：`meta.title`、`authors[0].name`、`tags[]`、`articles[].title`。
- 共享 `Validation` 也已经支持 `ValidationTransform` 链式转换后再校验：`Trim`、`NormalizeWhitespace`、`ParseNumber`、`ParseBool`、`ParseDatetime`。
- 共享 `Validation` 也已经支持 `with_object_validations([...])`、`with_each_validations([...])` 和 `validate_fields_report()` / `validate_item_report()` 这类 collect-all 报告能力。
- 共享 `Validation` 也已经支持 `with_all_of([...])`、`with_any_of([...])`、`with_one_of([...])`、`with_mutually_exclusive([...])` 这类跨字段组合约束。
- 共享 `Validation` 也已经支持最小条件约束：`with_when_exists(...)`、`with_when_missing(...)`、`with_when_equals(...)`、`with_when_not_equals(...)`，以及 `with_required_when_*` 这类条件必填语义。
- 当前 validation 语义已经明确为“显式配置才执行”；字段缺失时只有 `required` 才报错，其它规则默认跳过，组合约束里的可选字段也不会被当成自动通过。
- 和 engine/runtime 更紧的失败策略映射，以及更复杂的条件编排/派生约束还没有完全统一语义。

## 3. Request / Browser / Middleware 缺口

- 代码爬虫侧的 request 能力已经比较完整，但 DSL 到共享 request 模型的映射还没有完全收敛。
- browser 路线当前已经接住统一 `Request` 上的 `method`、`body`、cookies、最小 `session` 复用、`wait_for`、内置 `fingerprint_profile` 与最小 `stealth` bootstrap；更完整的 stealth 套件、自定义 profile 与更高阶指纹伪装仍未实现。
- browser `Response` 现在已经接住真实的导航 `status` 与响应头；`protocol` 继续表示 browser 执行语义，`ip_address` / `certificate` 仍受 Playwright 当前接口限制而保持为空。
- `Response.text` 现在已经统一从 `Response.body` 解码，并支持 BOM / `Content-Type charset` / 文档声明 / apparent encoding 猜测；更细的编码策略仍可能继续优化。
- browser `session` 当前已经通过稳定的 Playwright user data dir 落了最小复用能力，并对相同 session id 做了最小串行协调；但还没有更细粒度的 browser context / page 复用策略。
- browser 仍定位为“渲染型下载器”，不会朝通用自动化框架方向扩展；当前未实现多标签页编排、复杂交互流、截图/PDF 等更重能力。
- 统一 request cookies 目前只建模扁平的 key/value；domain、path、expires、same-site 这类更细粒度 cookie 属性还没有进入公开 `Request` API。
- proxy / cookies 已接到真实 HTTP 下载链路，但更细的 DSL 配置面和高级策略还没有统一。

## 4. Scheduler / Checkpoint 缺口

- 当前 `scheduler::Memory` 与 `scheduler::Redis` 已经把任务状态明确为 `ready / delayed / inflight` 三组状态，并支持 `priority / depth` 排序；其中 `scheduler::Memory` 也支持 `scheduler::checkpoint::Checkpoint` 导出/恢复。
- 当前已经有内置的本地文件 `scheduler::checkpoint::File`、Redis `scheduler::checkpoint::Redis`、`scheduler::checkpoint::Memory`，以及直接基于 Redis 的 `scheduler::Redis`；用户如果要接别的 scheduler / checkpoint 后端，可以直接实现对应 trait。更强的分布式协调与事务语义仍待继续补齐。

## 5. Pipeline 与 Store 能力

- `pipeline` 现在只负责 item 处理与过滤，不再直接承载最终输出实现。
- `store` 负责最终持久化或投递；当前内置 `store::Memory`、`store::File`、`store::Sqlite`、`store::Postgres`、`store::Webhook`、`store::Redis` 与 `store::Kafka`。
- `Engine::new()` 默认使用 `store::File::default()`，并写入 `output/<spider_name>.jsonl`。
- SQLite / Postgres 当前都已经支持自动建表、完整 `item_json` 落盘和显式字段列映射；其中 Postgres 当前要求目标数据库先存在。
- API 推送已经有最小 `store::Webhook` 实现。
- 消息输出当前已经有内置 `store::Redis` 与 `store::Kafka`；Redis 走最小 `SADD` set 写入，Kafka 走最小“完整 item JSON 消息”写入。
- 更多文件格式和更高阶消息语义仍应继续扩展在 `store` 这一层。

## 6. Runtime 观测与抓取策略缺口

- 当前已经有最小 `Engine::stats()` 运行时快照，包含 `request_count`、`response_count`、`error_count`、`retry_count`、`item_count` 与 `pipeline_drop_count`。
- 当前 stats 还是 engine 实例内的内存累计计数，还没有 Prometheus / OpenTelemetry exporter 或持久化/外发能力。
- 当前已经有最小 `robots.txt` 抓取策略：默认关闭、按 origin 内存缓存、支持 `User-agent` / `Allow` / `Disallow` 前缀语义；但 `Crawl-delay`、`Sitemap`、更完整 wildcard 规则和持久化 cache 还没补。
- HTTP cache / conditional request 仍未实现，`ETag` / `Last-Modified` 语义还没有进入下载运行时；当前已明确列为 `P3`，放到第三期再做。

## 7. Plugin 边界

- `Engine::load_plugins()` 当前只支持 `middleware` kind 的自动装载。
- `rules`、`provider`、`storage` 目前只是已命名的 manifest kind，不代表底层 runtime 已经具备对应装配能力。
- 如果后面要扩展新的 plugin kind，必须先有清晰的底层 owner：例如稳定的 pipeline/storage runtime、provider 抽象或其它可验证的共享链路。

## 8. DSL 对齐缺口

- DSL 配置面仍然整体后置，优先跟随代码爬虫与共享底层能力收敛。
- `step.meta`、`links[].meta`、dedup / schedule / retry、以及 step validate 已经落到底层能力。
- `next_url_config` 已支持 `FIELD`、`TEMPLATE`、`JOIN`、`FUNCTION`，其中最小函数集为 `concat`、`replace`、`coalesce`。
- 日期/时间表达式、更多函数、以及更完整的 request / parse 配置还没有统一设计。

## 9. API 命名与导出

- 评估 `item::Item` 这个公开路径是否需要继续收口成更顺手的对外导出，同时保持与 `request::Request`、`response::Response`、`settings::Settings` 的公开 API 一致性。

## 10. Engine 结构缺口

- `engine` 当前主循环和任务执行逻辑已经拆分出独立子模块；如果后面继续增长，再评估是否把调度主循环、runtime 组装等职责继续细分。
