## Why

Browser `session` 现在已经会复用稳定的 Playwright user data dir，但 browser profile 还是偏“当前请求声明什么，就按这次请求算什么”。
这会让同一个 session 的浏览器身份画像仍然依赖调用方每次重复声明，和 session 本身“稳定 browser 身份”的语义不完全一致。

## What Changes

- 给 browser session 增加“首次解析出的完整 browser profile 固定复用”语义
- 同一个 session 后续请求如果不再重复声明 profile，也会继续沿用已建立的 profile
- 如果后续请求显式声明了冲突的 engine 或 profile，直接返回显式错误
- README 和 capability 文档同步说明这条语义边界

## Impact

- browser session 的身份画像会比现在更稳定
- 不改变 `keep_alive` 的现有层级、scope 和生命周期控制
- 非 session 请求继续保持当前按请求解析的语义
