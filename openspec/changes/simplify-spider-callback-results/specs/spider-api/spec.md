# 规范增量

## ADDED Requirements

### Requirement: Spider Callbacks Return Real Products Directly

系统 MUST 让代码爬虫 callback 直接返回真实产物，而不是继续要求调用方显式组装 `Output { items, requests }`。

#### Scenario: Parse returns one next request directly

- **WHEN** spider 的 `parse()` 只需要产出一条下一跳 request
- **THEN** 调用方可以直接返回 `Request`
- **AND** 不需要再写 `Output { items: vec![], requests: vec![req] }`

#### Scenario: List callbacks return multiple requests directly

- **WHEN** 某个列表页 callback 需要继续调度多条详情页 request
- **THEN** 调用方可以直接返回 `Vec<Request>`
- **AND** `spider_callbacks!` 会继续把这些 request 分发给 engine

#### Scenario: Detail callbacks return final items directly

- **WHEN** 某个详情页 callback 只需要产出最终 item
- **THEN** 调用方可以直接返回 `Item` 或 `Vec<Item>`
- **AND** 不需要再手工把它塞进公开 `Output.items`

#### Scenario: Empty callbacks can return unit directly

- **WHEN** 某个 callback 当前不需要产出 request 或 item
- **THEN** 它可以直接返回 `()`
- **AND** spider 不需要再依赖 `Output::empty()`

#### Scenario: Mixed callback results can use a plain tuple

- **WHEN** 某个 callback 需要在同一跳同时产出 item 和下一跳 request
- **THEN** 调用方可以直接返回 Rust tuple，例如 `(Item, Vec<Request>)` 或 `(Vec<Item>, Request)`
- **AND** 不需要重新引入公开 `Output { items, requests }`

### Requirement: Spider Errbacks Follow The Same Direct Result Model

系统 MUST 让 request `errback` 与普通 callback 使用同一套直接返回模型，而不是继续特殊要求 `Result<Output, SpiderError>`。

#### Scenario: Errback can reschedule one request directly

- **WHEN** 某条 request 的 errback 只需要重新安排一条 request
- **THEN** errback 可以直接返回 `Request`
- **AND** engine 会继续按统一 callback 结果模型接管它

#### Scenario: Errback can return items directly

- **WHEN** errback 选择基于失败上下文直接产出补偿 item
- **THEN** 它可以直接返回 `Item` 或 `Vec<Item>`
- **AND** 这些 item 继续进入现有 item pipeline 与 store 链路

## MODIFIED Requirements

### Requirement: Request Remains The Primary Spider-Facing Object

系统 MUST 让 `Request` 继续作为代码爬虫里的第一等请求对象，而不是把 callback、meta、runtime 或输出语义分散到 `Response`、公开 `Output` 包装或 engine 隐式全局上。

#### Scenario: Callback returns requests without an output wrapper

- **WHEN** spider callback 构造一条新的 request
- **THEN** 它直接返回 `Request` 或 `Vec<Request>`
- **AND** callback、errback、meta、cb_kwargs、priority 与 middleware override 仍然都挂在这些 request 本身上

#### Scenario: Absolute URL requests do not need Response to be constructed

- **WHEN** spider 已经拿到一条完整绝对 URL
- **THEN** 它可以直接从 `Request::new(url)` 或同类 builder 起手构造下一条 request
- **AND** 不要求调用方为了构造 request 额外依赖 `Response`

## REMOVED Requirements

### Requirement: Public Spider Output Container

**Reason**: 公开 `Output { items, requests }` 会把“下一跳 request”与“最终 item”强行绑成一个统一包装，增加代码爬虫 callback 的样板和心智负担。

**Migration**: 代码爬虫 callback 与 errback 需要迁移为直接返回 `Request`、`Vec<Request>`、`Item`、`Vec<Item>` 或 `()`；不再使用 `Output::empty()` 或手工组装 `Output { items, requests }`。
