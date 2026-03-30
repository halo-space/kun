# Git 提交规范

## 提交者信息

所有提交必须使用统一的提交者信息：

```
Author: halo-space
```

## 提交消息格式

遵循 Conventional Commits 规范：

```
<type>(<scope>): <subject>

<body>

<footer>
```

### Type 类型

- `feat`: 新功能
- `fix`: 修复 bug
- `refactor`: 重构（不改变功能）
- `perf`: 性能优化
- `style`: 代码格式调整
- `docs`: 文档更新
- `test`: 测试相关
- `chore`: 构建/工具链相关

### 示例

```
feat(spider): add AI selector support

- Integrate async-openai for intelligent content extraction
- Add Settings configuration for OpenAI API
- Update examples and documentation

Closes #123
```

## 禁止事项

- ❌ 不得使用 Co-Authored-By 标签
- ❌ 不得包含其他提交者的邮箱信息
- ❌ 不得使用个人邮箱提交
