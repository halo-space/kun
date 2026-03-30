# DSL 实现问题记录

## 发现的问题

### 1. DSL 无法动态构造 URL ❌

**场景**：需要从 XML 提取 `front_page`，然后拼接当前日期构造列表页 URL
```
https://ep.shxwcb.com/2026/03/27/{front_page}?f=2026/03/period.xml
```

**当前 DSL 限制**：
- links 只能提取链接，不能进行字符串拼接
- 无法访问当前日期
- 无法使用模板语法构造 URL

**影响**：无法用纯 DSL 实现这个场景

**可能的解决方案**：
1. 添加 URL 模板语法：`"url_template": "https://ep.shxwcb.com/2026/03/27/{field}?f=..."`
2. 添加变量支持：`"variables": {"date": "{{now.day}}"}`
3. 混合模式：DSL + 少量代码回调

### 2. DSL 缺少日期/时间函数 ❌

**场景**：需要获取当前日期（27）来构造 URL

**当前 DSL 限制**：
- 没有内置函数或表达式系统
- 无法访问系统时间

**建议**：
- 添加内置变量：`{{now.year}}`, `{{now.month}}`, `{{now.day}}`
- 或添加表达式支持

### 3. meta_patch 无法传递提取的值 ❌

**场景**：需要将提取的 front_page 值传递到下一个 step

**当前 DSL 限制**：
- meta_patch 只支持静态值
- 无法引用当前提取的字段值
- 没有模板语法支持

**期望语法**：
```json
"meta_patch": {
  "front_page": "{{value}}"  // 引用当前提取的值
}
```

**建议**：
- 添加变量引用语法
- 或自动将提取的值传递到 meta

### 4. links 提取的是文本而不是 URL ❌

**场景**：从 XML 提取 `<front_page>9454__01.html</front_page>`

**当前行为**：
- links 提取到 "9454__01.html" 文本
- 但这不是一个有效的 URL
- 需要构造完整 URL：`https://ep.shxwcb.com/2026/03/27/9454__01.html`

**根本问题**：
- links 设计用于提取 href 属性
- 不适合提取文本后构造 URL

**建议**：
- 添加 url_template 支持
- 或允许 links 使用 text 属性并自动 follow

## 临时方案

使用混合模式：
- parse_xml 用 DSL 提取 front_page
- parse_list 用代码回调构造 URL
- parse_detail 用 DSL 解析
