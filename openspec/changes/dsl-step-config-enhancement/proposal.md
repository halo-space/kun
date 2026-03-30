# Proposal: DSL Step 配置增强

## 概述

增强 DSL Step 配置，支持 meta、dedup、schedule、retry 等核心功能，使 DSL 更接近设计文档（05-rules-dsl.md）的完整规范。

## 动机

当前 DSL 实现缺少多个关键配置项：
- meta 透传字段配置
- dedup 去重配置（key/ttl/scope）
- schedule 调度配置（concurrency/interval）
- retry 重试配置（count/http_status/backoff）

这些功能在设计文档中已定义，但尚未在代码中实现。

## 目标

**第一阶段（高优先级）**：
1. 添加 meta 字段到 StepConfig
2. 添加 dedup 配置支持
3. 添加 retry 配置支持

**第二阶段（中优先级）**：
1. 添加 schedule 配置
2. 添加 request 详细配置

## 非目标

- parse.auto（AI 自动提取）
- output sinks 配置
- proxy/cookies 高级配置

这些功能留待后续实现。
