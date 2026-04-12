## Overview

这次在现有 `device_profile.fingerprint` 模型内补一个低歧义字段：`mobile`。
目标不是把 browser 扩成完整设备模拟器，而是先把移动端最关键的几项默认画像统一起来。

## Design

1. 公开模型

- `FingerprintProfile` 新增 `mobile: Option<bool>`
- builder 提供 `with_mobile(bool)`
- 配置化 browser 编译入口也能把 `device_profile.fingerprint.mobile` 编译进来

2. 运行时解析

- 当 `mobile = true` 且调用方没有显式覆盖时：
  - user agent、platform 使用当前 engine 对应的移动端默认值
  - 默认 screen / viewport 切到移动端尺寸
  - Chromium `navigator.userAgentData.mobile` 与 touch hints 一并对齐

3. 边界

- 不新增 `tablet`、`vendor`、`hardware_concurrency` 这类公开字段
- 不承诺完整 mobile device emulation
- 如果调用方已经显式给了 user agent / platform / screen，则继续以显式配置为准

## Validation

- 增加 builder / serde / downloader 测试
- 覆盖移动端默认画像与 Chromium `userAgentData.mobile`
- 更新 README 与 capability 说明
