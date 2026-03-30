# 任务清单

## 1. 创建 DSL 配置文件

- [x] 1.1 创建 `examples/period_xml_dsl.json` 文件
- [x] 1.2 配置 parse_xml step：解析 XML 提取 front_page
- [x] 1.3 配置 parse_list step：解析列表页提取详情页链接
- [x] 1.4 配置 parse_detail step：解析详情页内容

## 2. 创建加载 DSL 的示例代码

- [x] 2.1 创建 `examples/period_xml_dsl.rs` 文件
- [x] 2.2 实现 Spider 加载 DSL 配置

## 3. 验证功能

- [x] 3.1 运行 `cargo run --example period_xml_dsl` 验证
- [x] 3.2 对比代码模式和 DSL 模式的效果

## 验证命令

```bash
cargo run --example period_xml_dsl
```
