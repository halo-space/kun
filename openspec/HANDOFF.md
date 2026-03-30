# 接手继续开发指南

本项目要求把“上下文”放进仓库，而不是留在某个 AI 工具的聊天记录里。

不管下一个人使用 Claude、Codex、Cursor，还是换了一台电脑，只要先读这份文档，再读对应的 OpenSpec 文件，就应该能继续当前变更。

## 接手目标

接手时要先回答这 3 个问题：

1. 现在在做哪个变更？
2. 这个变更已经完成到哪里？
3. 我应该从哪一个未完成任务继续？

## 标准接手顺序

### 1. 找当前进行中的变更

先看 `openspec/changes/` 下正在进行的变更。

当前仓库如果只有一个进行中的变更，可以直接从该变更开始；如果后续有多个变更，先确认你要接手的是哪一个。

### 2. 看当前做到哪

先打开：

- `openspec/changes/<change-name>/tasks.md`

这里是当前进度的第一入口。  
优先根据任务勾选状态判断：

- 哪些任务已经完成
- 哪些任务还没完成
- 下一步最应该继续哪一项

### 3. 看为什么这样做

再依次阅读：

- `openspec/changes/<change-name>/proposal.md`
- `openspec/changes/<change-name>/design.md`
- `openspec/changes/<change-name>/specs/...`

它们分别回答：

- 为什么做这次变更
- 设计是怎么定的
- 需求和行为边界是什么

### 4. 看最近实际做了什么

当前阶段先看这些信息：

- 最近的提交
- 当前分支状态
- `tasks.md` 的勾选情况

如果后续仓库引入 `openspec/worklog/`，再把“看最新 session 记录”加入固定接手步骤。

### 5. 再让 AI 接着做

不要只说“继续上一个任务”。

正确做法是明确告诉 AI：

- 当前变更名称
- 要先读哪些 OpenSpec 文件
- 先汇报当前进度
- 再从下一条未完成任务开始

## 推荐接手话术

下面这些提示词都可以直接复制，再替换 `<change-name>`。

### Claude

```text
请接手 `openspec/changes/<change-name>/` 这个变更。
先阅读 proposal.md、design.md、tasks.md 和 specs 目录，再结合当前代码库判断这次变更已完成到哪里。
先输出：
1. 已完成项
2. 未完成项
3. 下一步最应该做的任务

如果发现文档和代码现状不一致，请先说明差异，不要直接乱改。
确认完当前状态后，再继续实现下一条未完成任务。
```

### Codex

```text
请基于 `openspec/changes/<change-name>/` 接手当前工作。
先读取 proposal.md、design.md、tasks.md、specs/...，并检查仓库当前代码状态。
先告诉我：
1. 当前变更的目标
2. tasks.md 已完成和未完成的项目
3. 应该从哪一步继续
4. 有哪些风险或文档/代码偏差

然后从下一条未完成任务开始实施，并在完成后更新 tasks.md。
```

### Cursor

```text
请作为接手开发者处理 `openspec/changes/<change-name>/`。
先阅读 proposal、design、tasks 和 specs，并结合当前仓库代码判断进度。
先总结：
1. 当前变更在做什么
2. 已完成到哪里
3. 下一步从哪开始

如果发现设计和实现不一致，请先指出。
确认后继续处理下一条未完成任务。
```

## 什么时候更新什么

这里是本项目的默认记录频率规则。

### 每完成一个任务

只做这件事：

- 更新 `openspec/changes/<change-name>/tasks.md`

也就是把完成的任务勾掉。  
不需要因为完成了一个 task，就单独写一篇总结。

### 每结束一次连续工作会话

写一条会话级交接记录 / worklog 记录。

记录内容至少应包括：

- 这轮做了什么
- 改了哪些文件
- 跑了哪些验证
- 当前停在哪
- 下一个人应该从哪里开始

说明：

- “工作会话”指一轮连续的实现或整理工作
- 一次会话里可以完成多个 task
- 会话结束时再统一写一条记录，不要每个小动作都写

### 每当需求、设计、范围变化

再去更新这些文件：

- `proposal.md`
- `design.md`
- `specs/...`

也就是说：

- `tasks.md` 是任务级进度
- 交接记录 / worklog 是会话级总结
- proposal/design/specs 是方案级变更

## 常见错误与正确做法

### 错误 1

```text
继续上一个任务
```

### 正确 1

```text
请接手 `openspec/changes/<change-name>/`，先阅读 proposal、design、tasks、specs，再汇报当前进度并继续下一条未完成任务。
```

### 错误 2

```text
每完成一个 task 就写一篇总结
```

### 正确 2

```text
task 完成后只更新 tasks.md；这一轮工作结束时再写一条会话级记录。
```

### 错误 3

```text
代码改完就结束
```

### 正确 3

```text
代码改完后，还要补 tasks 勾选、验证结果和交接说明。
```

## 最小接手检查清单

开始实现前，至少确认以下几点：

- 我知道当前要接手的 `<change-name>`
- 我已经看过 `tasks.md`
- 我已经看过 `proposal.md` 和 `design.md`
- 我知道下一条未完成任务是什么
- 我知道当前代码与文档是否一致

如果以上任一项还不清楚，不要直接开始改代码。

## 当前最小可用接手路径

在 worklog 机制还没正式落地之前，默认按这条路径接手：

1. `openspec/changes/<change-name>/tasks.md`
2. `openspec/changes/<change-name>/proposal.md`
3. `openspec/changes/<change-name>/design.md`
4. `openspec/changes/<change-name>/specs/...`
5. `git log --oneline -n 10`
6. 当前分支与工作区状态

只要这几项是清楚的，下一个人就可以继续当前任务。
