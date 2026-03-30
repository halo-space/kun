# Tasks: DSL next_url_config and type Semantics

## Phase 1: 数据结构定义

### Task 1.1: 扩展 Step 结构
- **文件**: `src/rules/config.rs`
- **描述**: 添加 `step_type` 和 `next_url_config` 字段
- **验收**:
  - Step 结构包含 StepType enum
  - 支持 JSON 反序列化 `"type": "node"/"end"`
  - NextUrlConfig 结构完整

### Task 1.2: 定义 NextUrlConfig
- **文件**: `src/rules/config.rs`
- **描述**: 定义 NextUrlConfig 和 NextUrlMode
- **验收**:
  - 支持 4 种 mode: FIELD/TEMPLATE/JOIN/FUNCTION
  - 字段约束正确（mode=FIELD 需要 from 等）

## Phase 2: URL 构造逻辑

### Task 2.1: 创建 url_builder 模块
- **文件**: `src/rules/url_builder.rs` (新建)
- **描述**: 实现 build_next_urls 主函数
- **验收**:
  - 根据 mode 分发到不同构造函数
  - 返回 Vec<String>

### Task 2.2: 实现 FIELD 模式
- **文件**: `src/rules/url_builder.rs`
- **描述**: 从字段取值作为 URL
- **验收**:
  - 支持 string 和 array 类型
  - 单字段约束检查

### Task 2.3: 实现 TEMPLATE 模式
- **文件**: `src/rules/url_builder.rs`
- **描述**: 模板替换 {field} 和 {meta.xxx}
- **验收**:
  - 正确替换 {field}
  - 正确替换 {meta.xxx}
  - 支持 string/number/bool 类型

### Task 2.4: 实现 JOIN 模式
- **文件**: `src/rules/url_builder.rs`
- **描述**: 多字段拼接
- **验收**:
  - 支持 "" 和 "/" 分隔符
  - 至少 2 个字段约束

### Task 2.5: 实现 URL 规范化
- **文件**: `src/rules/url_builder.rs`
- **描述**: trim + 相对路径补全 + 去重
- **验收**:
  - 使用 url crate 补全相对路径
  - 过滤非 http/https
  - 稳定去重

## Phase 3: dispatch 逻辑重构

### Task 3.1: 修改 dispatch 函数
- **文件**: `src/spider.rs`
- **描述**: 统一 dispatch 逻辑
- **验收**:
  - 优先检查代码回调
  - 其次使用 DSL
  - 最后调用默认 parse

### Task 3.2: 实现 parse_with_dsl
- **文件**: `src/spider.rs`
- **描述**: DSL 解析主逻辑
- **验收**:
  - 提取字段
  - 根据 step_type 分支
  - type=node 生成 requests
  - type=end 返回 items

### Task 3.3: 实现 meta 自动透传
- **文件**: `src/spider.rs`
- **描述**: create_requests_with_meta 函数
- **验收**:
  - 合并 current_meta + parsed_fields
  - 所有 requests 携带新 meta
  - 统一调用 parse 回调

### Task 3.4: 实现 get_current_step
- **文件**: `src/spider.rs`
- **描述**: 从 meta 获取 step_idx
- **验收**:
  - 默认 step_idx=0
  - 返回对应 Step 引用

## Phase 4: 集成测试

### Task 4.1: 创建 period_xml_dsl_full.json
- **文件**: `examples/period_xml_dsl_full.json`
- **描述**: 完整 DSL 配置（无代码回调）
- **验收**:
  - 3 个 step 全部用 DSL
  - step[0] 用 TEMPLATE 模式
  - step[1] 用 FIELD 模式
  - step[2] type=end

### Task 4.2: 创建 period_xml_dsl_full.rs
- **文件**: `examples/period_xml_dsl_full.rs`
- **描述**: 纯 DSL 示例（只有 parse 函数）
- **验收**:
  - 只定义 parse 回调
  - 加载 DSL 配置
  - 运行成功爬取 3 级

### Task 4.3: 单元测试
- **文件**: `src/rules/url_builder.rs`
- **描述**: URL 构造函数测试
- **验收**:
  - test_field_mode
  - test_template_mode
  - test_join_mode
  - test_normalize_urls

## Phase 5: 文档和示例

### Task 5.1: 更新 README
- **文件**: `README.md`
- **描述**: 添加 DSL 使用说明
- **验收**:
  - 说明 type="node"/"end" 语义
  - 说明 next_url_config 用法
  - 链接到示例

### Task 5.2: 添加 DSL 配置示例
- **文件**: `examples/README.md`
- **描述**: DSL 配置最佳实践
- **验收**:
  - FIELD/TEMPLATE/JOIN 示例
  - meta 传递示例
  - 完整三级爬取示例
