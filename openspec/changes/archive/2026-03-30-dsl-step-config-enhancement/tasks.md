# Tasks: DSL Step 配置增强

当前状态说明：

- 这份任务清单已经明显落后于当前代码现状。
- `meta / dedup / schedule / retry` 中的大部分实现，已经被后续 change 和当前代码吸收。
- 如果继续推进 DSL，对应工作应优先并入 `track-kun-core-capability-gaps` 或新的 DSL 主线 change，而不是继续逐条执行这份旧任务。

## 阶段 1：Schema 扩展

### Task 1.1: 添加配置结构体
- [ ] 在 `src/rules/schema.rs` 添加 `DedupConfig`
- [ ] 在 `src/rules/schema.rs` 添加 `ScheduleConfig`
- [ ] 在 `src/rules/schema.rs` 添加 `RetryConfig`
- [ ] 实现 Default trait

### Task 1.2: 扩展 StepConfig
- [ ] 添加 `meta: Option<BTreeMap<String, Value>>`
- [ ] 添加 `dedup: Option<DedupConfig>`
- [ ] 添加 `schedule: Option<ScheduleConfig>`
- [ ] 添加 `retry: Option<RetryConfig>`

## 阶段 2：解析支持

### Task 2.1: 实现 JSON 解析
- [ ] 在 `src/rules/load.rs` 添加 dedup 解析逻辑
- [ ] 在 `src/rules/load.rs` 添加 schedule 解析逻辑
- [ ] 在 `src/rules/load.rs` 添加 retry 解析逻辑
- [ ] 在 `src/rules/load.rs` 添加 meta 解析逻辑

## 阶段 3：编译支持

### Task 3.1: 编译配置
- [ ] 在 `src/rules/compile.rs` 处理 dedup 配置
- [ ] 在 `src/rules/compile.rs` 处理 schedule 配置
- [ ] 在 `src/rules/compile.rs` 处理 retry 配置
- [ ] 在 `src/rules/compile.rs` 处理 meta 配置

## 阶段 4：运行时应用

### Task 4.1: Dedup 实现
- [ ] 在 `src/rules/run.rs` 实现 dedup 检查逻辑
- [ ] 支持从 parsed_fields 和 meta 提取 key
- [ ] 实现 scope 逻辑（TASK/STEP/CUSTOM）
- [ ] 集成到现有 dedup middleware

### Task 4.2: Retry 实现
- [ ] 在请求失败时应用 retry 配置
- [ ] 根据 http_status 判断是否重试
- [ ] 实现 backoff 延迟

### Task 4.3: Schedule 实现
- [ ] 应用 concurrency 限制
- [ ] 应用 interval 间隔控制

### Task 4.4: Meta 实现
- [ ] 在生成 request 时合并 step.meta
- [ ] 确保 meta 正确透传

## 阶段 5：测试

### Task 5.1: 单元测试
- [ ] 测试 dedup 配置解析
- [ ] 测试 schedule 配置解析
- [ ] 测试 retry 配置解析
- [ ] 测试 meta 配置解析

### Task 5.2: 集成测试
- [ ] 创建包含 dedup 的 DSL 示例
- [ ] 创建包含 retry 的 DSL 示例
- [ ] 创建包含 schedule 的 DSL 示例
- [ ] 创建包含 meta 的 DSL 示例
- [ ] 验证功能正常工作

## 阶段 6：文档

### Task 6.1: 更新文档
- [ ] 更新 README 说明新增配置
- [ ] 添加 DSL 配置示例
- [ ] 更新 TODO.md 标记已完成项
