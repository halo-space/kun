# TODO

## 1. HTML XPath 支持

当前 XPath 选择器使用 XML 解析器（sxd_document），对 HTML 的容错性较差。

**问题**：
- HTML 中的 XPath 选择器无法正确匹配元素
- XML 解析器要求严格的格式，不适合解析不规范的 HTML

**解决方案**：
- 添加专门的 HTML XPath 解析库（如 lxml 的 Rust 替代品）
- 或者在 HTML 场景下将 XPath 转换为 CSS 选择器

**临时方案**：
- 在 HTML 解析中使用 CSS 选择器替代 XPath

---

## 2. DSL 功能完善（参考 05-rules-dsl.md）

### 2.1 Step 级别配置

**meta（透传字段）**：
- [x] `meta`: 透传字段配置（BTreeMap<String, Value>）

**dedup（去重配置）**：
- [x] `enabled`: 是否启用去重（已支持解析）
- [x] `key`: 去重键字段列表（支持 `meta.xxx`）（已支持解析）
- [x] `ttl`: 去重 TTL（秒）（已支持解析）
- [x] `scope`: 去重范围（TASK / STEP / CUSTOM）（已支持解析）
- [x] `namespace`: 自定义命名空间（scope=CUSTOM 时使用）（已支持解析）
- [x] 运行时逻辑实现

**schedule（调度配置）**：
- [x] `concurrency`: 并发数（已支持解析）
- [x] `interval`: 请求间隔（ms）（已支持解析）
- [x] 运行时逻辑实现

**retry（重试配置）**：
- [x] `count`: 重试次数（已支持解析）
- [x] `http_status`: 触发重试的 HTTP 状态码列表（已支持解析）
- [x] `backoff`: 退避时间列表（ms）（已支持解析）
- [x] 运行时逻辑实现

**request（请求配置）**：
- [ ] `method`: HTTP 方法
- [ ] `timeout`: 超时时间（ms）
- [ ] `enabled_headers`: 是否启用自定义 headers
- [ ] `headers`: 自定义请求头
- [ ] `payload`: 请求体
- [ ] `proxy`: 代理配置（mode/url/pool_key/rotate_on/max_failures）
- [ ] `cookies`: Cookie 配置（mode/scope/persist/ttl_sec/isolate_per_item）

**allow_url_pattern**：
- [ ] URL 正则过滤列表

**download**：
- [ ] 下载配置（可选）

### 2.2 Parse 配置增强

**parse.mode**：
- [ ] `AUTO_THEN_RULE`: AI 自动提取 + 规则兜底
- [ ] `RULE_ONLY`: 仅规则提取（当前实现）
- [ ] `AUTO_ONLY`: 仅 AI 自动提取

**parse.auto**（AI/OCR 自动提取）：
- [ ] `type`: AI / OCR / OCR_AI
- [ ] `source`: 输入来源
- [ ] `model`: 模型名称
- [ ] `output_format`: 输出格式
- [ ] `prompt`: 提示词
- [ ] `timeout`: 超时时间
- [ ] `max_tokens`: 最大 token 数

**parse.rule[].options**（多选择器兜底）：
- [ ] 支持每个字段配置多个选择器选项
- [ ] 按顺序尝试直到成功

### 2.3 Validate 配置

- [ ] `type`: 字段类型（text/number/bool/list/object）
- [ ] `rule.required`: 是否必填
- [ ] `rule.regex`: 正则校验
- [ ] `rule.min/max`: 数值范围
- [ ] `rule.enum`: 枚举值


### 2.4 Output 配置（顶层）

**output.sinks**：
- [ ] `type`: MYSQL / FILE / MQ
- [ ] `mode`: UPSERT / INSERT
- [ ] `table`: 表名
- [ ] `unique_keys`: 唯一键
- [ ] `mapping`: 字段映射
- [ ] `path_template`: 文件路径模板
- [ ] `topic/key`: MQ 配置

**output.policy**：
- [ ] `on_sink_error`: FAIL_ITEM / SKIP_SINK

### 2.5 Next URL Config 增强

- [x] `mode`: FIELD / TEMPLATE（已实现）
- [ ] `mode`: JOIN / FUNCTION
- [x] `from`: 字段列表（已实现）
- [ ] `join_delimiter`: 拼接分隔符
- [x] `template`: URL 模板（已实现）
- [ ] `fn`: 自定义函数名
- [ ] `args`: 函数参数


---

## 3. 优先级建议

**高优先级**（核心功能）：
1. dedup 配置支持（step 级别）
2. parse.rule[].options 多选择器兜底
3. validate 完整实现
4. retry 配置
5. meta 透传字段配置

**中优先级**（增强功能）：
1. schedule 配置（concurrency/interval）
2. request 详细配置（method/timeout/headers/payload）
3. allow_url_pattern URL 过滤
4. next_url_config 的 JOIN/FUNCTION 模式

**低优先级**（高级功能）：
1. parse.auto（AI 自动提取）
2. output sinks 配置
3. proxy/cookies 高级配置
4. download 配置
