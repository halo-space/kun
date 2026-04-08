## Why

当前 `device_profile.fingerprint` 已经能表达浏览器身份画像，但默认仍偏桌面端。
如果调用方需要移动端浏览器画像，现有模型缺少一个显式入口，导致 `userAgentData.mobile`、默认 viewport、touch hints 这类移动端特征很难统一收口。

## What Changes

- 为 `device_profile.fingerprint` 增加显式 `mobile` 字段
- 当 `mobile = true` 时，按当前 `engine` 切到移动端默认 user agent / platform
- 同步调整默认 screen / viewport 与 touch hints
- README、capabilities 和 DSL 编译入口同步支持这条语义

## Impact

- browser 画像可以更明确地区分桌面端和移动端
- 不引入新的顶层 browser API，继续收口在 `device_profile.fingerprint`
- 仍然不承诺“品牌级完整伪装”
