# 规范增量

## ADDED Requirements

### Requirement: AI 选择器支持

库必须在 DSL 中支持 `ai` 选择器类型，允许使用 OpenAI API 进行智能内容提取。

#### Scenario: AI 选择器提取文本内容

- **WHEN** 字段规则声明 `selector_type: "ai"` 且 `selector` 包含提示词
- **THEN** 系统调用 OpenAI API，将提示词和源内容发送，返回提取结果

#### Scenario: AI 选择器需要 API key 配置

- **WHEN** 使用 AI 选择器但未配置 API key
- **THEN** 系统返回配置错误，提示用户设置 `OPENAI_API_KEY` 环境变量或通过 Settings 配置

#### Scenario: AI 选择器支持自定义模型

- **WHEN** Settings 中配置了 `openai_model`
- **THEN** AI 选择器使用指定的模型而非默认的 `gpt-4o-mini`

### Requirement: AI 选择器作为可选 feature

库必须将 AI 选择器实现为可选的 Cargo feature，避免强制依赖 OpenAI。

#### Scenario: 未启用 ai-selector feature 时编译失败

- **WHEN** DSL 中使用 `selector_type: "ai"` 但未启用 `ai-selector` feature
- **THEN** 编译时或运行时返回明确错误，提示启用 feature

## MODIFIED Requirements

### Requirement: 支持的选择器类型可枚举

库必须在规则 schema 中显式保留来源类型与选择器类型。

#### Scenario: 支持的选择器类型包含 ai

- **WHEN** 任意字段规则或链接规则被编译
- **THEN** 选择器类型从显式值中解析，包括 `css`、`xpath`、`json`、`xml`、`regex` 或 `ai`
