# 规范增量

## ADDED Requirements

### Requirement: 连接池大小可配置

库必须允许用户配置 HTTP 客户端的连接池大小。

#### Scenario: Settings 配置连接池大小

- **WHEN** 用户通过 `Settings::connection_pool_size()` 配置连接池
- **THEN** Engine 创建的 HTTP 客户端使用指定的连接池大小

#### Scenario: 默认连接池大小

- **WHEN** 用户未显式配置连接池大小
- **THEN** 系统使用默认值（如 100）

### Requirement: 按域名并发控制实现

库必须实现按域名的并发限制，确保对同一域名的并发请求不超过配置值。

#### Scenario: 域名级并发限制生效

- **WHEN** 对同一域名的并发请求数达到 `concurrent_requests_per_domain` 上限
- **THEN** 新的请求等待，直到该域名有可用的并发槽位

#### Scenario: 不同域名独立计数

- **WHEN** 同时请求多个不同域名
- **THEN** 每个域名的并发限制独立计算，互不影响

## MODIFIED Requirements

### Requirement: Engine 应用并发与域名控制

库必须遵守 `Settings` 中的全局并发与按域名并发控制。

#### Scenario: 全局并发与域名并发同时生效

- **WHEN** 配置了全局并发上限和域名并发上限
- **THEN** 两个限制同时生效，取更严格的限制
