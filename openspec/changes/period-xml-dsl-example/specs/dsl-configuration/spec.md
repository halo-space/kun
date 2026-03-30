# 规范增量

## ADDED Requirements

### Requirement: DSL 三级爬取示例

系统必须提供 DSL 配置示例展示如何实现多级爬取场景。

#### Scenario: XML 解析配置

- **WHEN** 用户需要用 DSL 解析 XML 并提取数据
- **THEN** 可以配置 source="xml", selector_type="xpath"

#### Scenario: 回调链配置

- **WHEN** 用户需要在 DSL 中实现多级回调
- **THEN** 可以使用 links 的 to.next_step 配置下一个 step

## MODIFIED Requirements

无

## REMOVED Requirements

无
