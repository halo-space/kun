# 任务清单

## 1. 实现示例代码

- [x] 1.1 创建 `examples/period_xml_spider.rs` 文件
- [x] 1.2 实现 `PeriodSpider` 结构体和 `Spider` trait
- [x] 1.3 实现 `parse()` 方法：解析 period.xml 提取最新日期
- [x] 1.4 实现 `parse_list()` 方法：解析列表页提取详情页链接
- [x] 1.5 实现 `parse_detail()` 方法：解析详情页内容
- [x] 1.6 添加 `spider_callbacks!` 宏注册回调

## 2. 验证功能

- [x] 2.1 运行 `cargo run --example period_xml_spider` 验证三级爬取
- [x] 2.2 检查日志输出确认流程正确
- [x] 2.3 记录发现的问题到 TODO 文件

## 3. 文档更新

- [x] 3.1 如需要，更新 `openspec/specs/parsing-and-extraction.md` 添加 XML 示例
- [x] 3.2 如需要，在 README.md 中添加示例引用

## 验证命令

```bash
cargo run --example period_xml_spider
```
