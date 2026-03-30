# 技术设计

## 概览

创建 JSON DSL 配置文件实现三级爬取，对比代码模式展示 DSL 的能力。

**DSL 配置结构**：
- 3 个 steps：parse_xml、parse_list、parse_detail
- 使用 xpath 解析 XML
- 使用 links 配置实现回调链
- 使用 meta_patch 传递参数

## 模块影响

- 不修改任何 `src/` 模块
- 新增 `examples/period_xml_dsl.json` - DSL 配置
- 新增 `examples/period_xml_dsl.rs` - 加载 DSL 的代码
- 可能更新 `openspec/specs/dsl-configuration.md`

## 关键决策

- Runtime / middleware 影响：无
- 对外 API 影响：无
- Plugin 或 DSL 影响：验证现有 DSL 语法是否足够

## 验证方式

运行 `cargo run --example period_xml_dsl` 验证 DSL 配置是否正确执行三级爬取
