## Overview

这次不引入新的公开 API，直接在现有 browser `session` 语义里补一层稳定 profile 固定逻辑。

## Design

1. 请求级 profile 解析

- 继续复用当前 `device_profile` 解析逻辑
- 如果当前请求启用了 `stealth = true`，但没有显式 `device_profile`，则生成一份当前 engine 对应的内置 browser profile

2. session 级 profile 固定

- 对带 `session` 的 browser 请求，在 session user data dir 下保存一份 `device-profile.json`
- 第一次解析出 profile 时写入
- 后续同 session 请求优先读取这份固定 profile
- 如果当前请求显式解析出的 profile 与已固定 profile 不一致，直接报错

3. 边界

- 只有 session 请求会固定 profile
- 非 session 请求继续按当前请求独立解析
- 这层逻辑不放宽 `keep_alive` 的稳定签名要求；live context/page 复用仍保持现有约束

## Validation

- 覆盖首次固定、后续复用、冲突报错三类测试
- README 和 `docs/capabilities.md` 更新 session 画像复用说明
