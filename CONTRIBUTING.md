# 贡献指南

## 开发流程

本项目使用 OPSX / OpenSpec 作为规范与变更工作流。

### 目录结构

```text
openspec/
  config.yaml           # OpenSpec 配置
  schemas/halo-spider/  # 项目 schema 定义
  specs/                # 主规范文档
  changes/              # 变更工作区

.claude/
  commands/opsx/        # Claude Code 命令

.codex/
  skills/openspec-*/    # Codex 技能
```

### 推荐流程

1. **提出变更**
   ```bash
   /opsx:propose "你的想法"
   ```

2. **完善文档**
   在 `openspec/changes/<change-name>/` 下补充：
   - `proposal.md` - 变更提案
   - `design.md` - 技术设计
   - `specs/` - 规范增量
   - `tasks.md` - 任务清单

3. **实现代码**
   ```bash
   /opsx:apply
   ```

4. **验证**
   ```bash
   cargo test
   cargo run --example <example-name>
   ```

5. **同步规范**
   实现完成后，将 delta specs 同步到 `openspec/specs/`

6. **归档**
   ```bash
   /opsx:archive
   ```

## 接手继续开发

本项目支持多人、跨电脑、跨 AI 工具接手。

### 接手前必读

- `openspec/HANDOFF.md` - 交接指南

### 接手顺序

1. 先看 `openspec/changes/<change-name>/tasks.md` - 了解当前进度
2. 再看 `proposal.md`、`design.md`、`specs/` - 理解背景和设计
3. 以 `tasks.md` 为准开始工作

### 记录频率

- 每完成一个任务：更新 `tasks.md` 中的复选框
- 每结束一次连续工作会话：补一条交接记录
- 每当需求或设计变化：更新 `proposal/design/specs`

## 规范入口

当前主规范位于：

- `openspec/specs/spider-api/spec.md` - Spider API 规范
- `openspec/specs/rules-dsl/spec.md` - Rules DSL 规范
- `openspec/specs/runtime-engine/spec.md` - 运行时引擎规范
- `openspec/specs/middleware-plugins/spec.md` - 中间件插件规范

## 测试

```bash
# 运行所有测试
cargo test

# 运行特定测试
cargo test test_name

# 运行示例
cargo run --example period_xml_spider
cargo run --example custom_middleware
```

## 代码风格

- 遵循 Rust 标准代码风格
- 使用 `cargo fmt` 格式化代码
- 使用 `cargo clippy` 检查代码质量

## 提交规范

使用语义化提交信息：

- `feat:` - 新功能
- `fix:` - 修复 bug
- `docs:` - 文档更新
- `refactor:` - 重构
- `test:` - 测试相关
- `chore:` - 构建/工具相关

## License

MIT
