# Design: DSL 运行时逻辑实现

## 架构概览

dedup/retry/schedule 是框架级别的能力，应该对 DSL 和代码回调都生效。

## 核心问题

配置在 CompiledStep 中，但需要在 engine 执行请求时生效。需要一个机制让 engine 访问当前请求对应的 step 配置。

## 架构方案

### 方案 1：在 Request meta 中携带配置（推荐）

在生成 Request 时，将 step 的 dedup/retry/schedule 配置序列化到 meta 中：
- `meta["__step_dedup"]`
- `meta["__step_retry"]`
- `meta["__step_schedule"]`

优点：
- 最小侵入性
- Request 自包含所有需要的信息
- middleware 可以直接从 context.request.meta 读取

缺点：
- meta 中混入框架内部字段
