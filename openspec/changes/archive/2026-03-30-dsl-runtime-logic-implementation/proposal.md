# Proposal: DSL 运行时逻辑实现

## 概述

实现 dedup/retry/schedule 配置的运行时逻辑，使这些配置在爬虫执行时真正生效。

## 背景

在 `dsl-step-config-enhancement` change 中，我们已经完成：
- 配置结构定义
- JSON 解析
- 配置编译和传递
- Meta 字段运行时实现

但 dedup/retry/schedule 的运行时逻辑尚未实现，配置虽然可以解析但不会实际生效。

## 目标

实现三个配置的运行时逻辑：
1. **Dedup** - 从 parsed_fields 和 meta 提取 key，执行去重检查
2. **Retry** - 根据 http_status 判断是否重试，应用 backoff 延迟
3. **Schedule** - 应用 concurrency 和 interval 限制

## 非目标

- 不修改配置结构（已完成）
- 不修改解析逻辑（已完成）
