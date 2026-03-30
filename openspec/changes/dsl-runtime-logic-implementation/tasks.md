# Tasks: DSL 运行时逻辑实现

## Dedup 运行时实现

### Task 1: 实现 dedup 值提取
- [x] DSL step 生成 Request 时透传 parsed_fields，供共享 dedup 逻辑使用
- [x] 支持从 `request.meta` 读取 `meta.xxx` 风格 key
- [x] 支持多 key 拼接（用 `|` 连接）

### Task 2: 实现 scope 逻辑
- [x] TASK scope: 直接使用拼接值
- [x] STEP scope: 添加 step.id 前缀
- [x] CUSTOM scope: 添加 namespace 前缀

### Task 3: 集成去重检查
- [x] DSL 顶层 `dedup` 配置编译进共享 runtime/middleware
- [x] 统一由框架 dedup middleware 过滤重复 request
- [x] 添加测试覆盖 Request 级去重行为

## Retry 运行时实现

### Task 4: 捕获请求失败
- [x] 在引擎请求执行层捕获失败
- [x] 统一读取响应 `http_status`

### Task 5: 实现重试逻辑
- [x] DSL 顶层 `retry` 配置编译进共享 runtime/middleware
- [x] 根据 `http_status` 判断是否重试
- [x] 应用 backoff 延迟
- [x] 限制重试次数

## Schedule 运行时实现

### Task 6: 实现并发控制
- [x] 添加 step 级别的并发限制
- [x] 通过共享并发 gate middleware 控制并发数

### Task 7: 实现间隔控制
- [x] 记录上次请求时间
- [x] 应用 interval 延迟
