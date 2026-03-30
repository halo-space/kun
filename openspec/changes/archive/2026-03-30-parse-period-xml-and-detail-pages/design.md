# 技术设计

## 概览

直接使用 kun 现有能力实现三级爬取，不修改任何核心代码：

**流程**：
1. `parse()` - 请求 period.xml，用 XPath 提取最新日期
2. `parse_list()` - 根据日期构造列表页 URL，提取详情页链接
3. `parse_detail()` - 解析详情页内容

**使用的现有能力**：
- `response.xml("//xpath").text().one()` - XML 解析
- `response.follow(url).with_callback(cb!(parse_list))` - 回调链
- `response.follow_with_meta(url, &meta)` - 传递日期参数

## 模块影响

- 不修改任何 `src/` 模块
- 新增 `examples/period_xml_spider.rs`
- 可能更新 `openspec/specs/` 中的示例说明

## 关键决策

- Runtime / middleware 影响：无
- 对外 API 影响：无
- Plugin 或 DSL 影响：无

## 验证方式

1. 实现 `examples/period_xml_spider.rs`
2. 运行验证三级爬取是否正常
3. 记录实现过程中发现的问题到 TODO
4. 评估代码优雅度和性能
