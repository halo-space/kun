# 规范增量

## ADDED Requirements

### Requirement: XML 多级解析示例

系统必须提供示例展示如何使用 XML 解析器配合回调链实现多级数据提取。

#### Scenario: 从 XML 索引提取日期并构造下级 URL

- **WHEN** 用户需要从 XML 文件中提取动态参数（如日期）来构造后续请求 URL
- **THEN** 可以使用 `response.xml("//xpath").text().one()` 提取值，并通过 `response.follow(url).with_callback()` 传递到下一个回调

#### Scenario: 三级回调链传递参数

- **WHEN** 用户需要在多个回调之间传递上下文数据
- **THEN** 可以使用 `response.follow_with_meta(url, &meta)` 在请求间传递参数

## MODIFIED Requirements

无

## REMOVED Requirements

无
