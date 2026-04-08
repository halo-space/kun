# 规范增量

## ADDED Requirements

### Requirement: Browser Keep Alive Lifecycle Is Explicit At Runtime

系统 MUST 让 browser downloader 的 keep_alive 生命周期与错误恢复行为显式、可测试，而不是继续依赖隐含缓存实现。

#### Scenario: Keep alive idle timeout expires unused browser entries

- **WHEN** 某个 keep_alive entry 在 `keep_alive_max_idle` 窗口内没有再次被取用
- **THEN** downloader 在后续访问或归还该 bucket 时通过懒清理丢弃旧 entry
- **AND** 新请求会创建新的 keep_alive entry

#### Scenario: Keep alive lifecycle does not require a background sweeper

- **WHEN** runtime 管理 keep_alive idle 生命周期
- **THEN** 系统通过请求路径上的懒清理完成过期处理
- **AND** 不要求额外后台 sweep 任务才能维持正确语义

#### Scenario: Keep alive max uses rebuilds entries deterministically

- **WHEN** 某个 keep_alive entry 达到 `keep_alive_max_uses`
- **THEN** downloader 在下一次访问该 bucket 时重建 entry
- **AND** 不再继续复用旧的 context 或 page

#### Scenario: Keep alive error policy controls entry retention

- **WHEN** browser request 在持有 keep_alive 的执行过程中发生 browser 级错误
- **THEN** 若 `keep_alive_on_error = reset`，downloader 会主动丢弃当前 entry
- **AND** 若 `keep_alive_on_error = keep`，downloader 会按声明继续保留该 entry

#### Scenario: Keep alive controls do not replace session persistence

- **WHEN** browser request 使用 `keep_alive_key`、`keep_alive_max_idle`、`keep_alive_max_uses` 或 `keep_alive_on_error`
- **THEN** 这些控制项只影响 keep_alive entry 的运行态复用
- **AND** 不改变稳定 session user data dir 的基本语义
