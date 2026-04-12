# Browser 与 AI

[返回使用手册](../guide.md)

## Browser 能力边界

当前 `browser` 模式走 `playwright-rs` 这条实现线，对外仍然只是 `kun` 的一个浏览器下载能力，不额外暴露单独的 backend 概念。

当前已经接线的最小能力：

- `engine = chromium | firefox | webkit`
- `headless`
- `wait_for_selector`
- request method
- request body
- request cookies
- request timeout
- request headers
- request proxy
- request session
- optional `device_profile`
- optional `device_profile.fingerprint`
- optional `device_profile.screen`
- explicit `keep_alive = isolated | context | page`
- optional `keep_alive_scope = session | origin`
- optional `keep_alive_key`
- optional `keep_alive_max_idle`
- optional `keep_alive_max_uses`
- optional `keep_alive_on_error = keep | reset`
- `stealth = true` bootstrap
- optional external `stealth_script`
- browser response status / headers
- 页面渲染后的 HTML 抓取

### Session 与 KeepAlive

browser `session` 当前会把同一个 session id 映射到稳定的 Playwright user data dir，用于复用 cookies 和 local storage 这类浏览器态数据；是否继续保留 `keep_alive` 这层浏览器态，由 `keep_alive` 显式控制：

- `isolated`：只复用稳定 user data dir，每次请求仍新建并关闭 context/page
- `context`：同一 session 复用 live context，但每次请求新建 page
- `page`：同一 session 复用 live context 和同一张 live page

如果你希望把 `keep_alive` 再按更小范围隔离，还可以显式设置：

- `keep_alive_scope = session`：同一个 session 共用一份 `keep_alive`
- `keep_alive_scope = origin`：同一个 session 下，按 URL origin 分开维护 `keep_alive`

如果还需要进一步控制同一个 `keep_alive` bucket 的生命周期，可以继续加：

- `keep_alive_key`
- `keep_alive_max_idle`
- `keep_alive_max_uses`
- `keep_alive_on_error = keep | reset`

这几项都只影响 live `keep_alive` 的运行态复用，不改变稳定 user data dir 的基本语义。

### Device Profile 与 Stealth

当前已经支持的 browser 指纹能力边界：

- 公开画像入口统一收口到 `device_profile`
- `device_profile.fingerprint` 负责 `user_agent / locale / timezone / accept-language / languages / platform / mobile / client_hints / device_memory`
- `device_profile.screen` 负责 `viewport / screen / avail` 三组尺寸，以及 `color_depth / pixel_depth / device_scale_factor`
- `device_profile.fingerprint` 支持部分填写；下载器会按当前 `engine` 与稳定默认值补齐最终执行画像
- 如果显式声明 `device_profile.fingerprint.mobile = true`，下载器会切到移动端默认画像，并同步带上更贴近移动端的默认 viewport / touch hints
- `device_profile.fingerprint.client_hints` 当前负责 `architecture / bitness / model / platform_version / ua_full_version`，并用于 Chromium 路线的 `navigator.userAgentData`
- `client_hints` 当前只作用在浏览器 JS 侧画像，不会自动注入 HTTP `Sec-CH-UA*` 请求头
- `device_profile.screen` 也支持部分填写；缺失尺寸会按组合规则推导，明显冲突的尺寸组合会显式报错
- 同一个 `session` 一旦建立过 browser profile，后续同 session 请求会继续复用这份完整画像；如果又显式声明了冲突画像，会直接报错
- `stealth = true` 当前会注入 bootstrap，覆盖 `navigator.webdriver`、`navigator.language(s)`、`navigator.platform`、`navigator.vendor`、`hardwareConcurrency`、`deviceMemory`、`maxTouchPoints`、`plugins`、`mimeTypes`、`pdfViewerEnabled`、screen depth、notifications permissions 查询补丁，以及 Chromium 路线上的最小 `window.chrome` / `navigator.userAgentData`
- `stealth_script` 可以把外部 stealth JS 叠加到内置 bootstrap 后面；如果只想注入外部脚本，也可以不打开 `stealth = true`
- 默认画像和 stealth 现在会跟随 `engine` 切到 Chromium / Firefox / WebKit 对应的浏览器族，不内置更细的品牌级画像库

当前如果构建没有启用 `browser` feature，browser request 会直接返回显式错误，不会返回 stub response。

启用方式：

```toml
halo-spider = { version = "0.0.5", features = ["browser"] }
```

首次使用前需要安装 Playwright 浏览器：

```bash
npx playwright@1.58.2 install chromium firefox webkit
```

最小使用示例：

```rust
use halo_spider::request::{browser, Request};
use jiff::SignedDuration;

let request = Request::browser("https://example.com/app")
    .with_timeout(SignedDuration::from_secs(15))
    .with_session("news-browser")
    .with_browser(
        browser::Config::default()
            .with_engine(browser::Engine::Chromium)
            .with_wait_for_selector("#app")
            .with_stealth(true)
            .with_stealth_script("window.__thirdPartyStealth = true;")
            .with_device_profile(
                browser::DeviceProfile::new()
                    .with_fingerprint(
                        browser::FingerprintProfile::new()
                            .with_locale("ja-JP")
                            .with_timezone("Asia/Tokyo")
                            .with_accept_language("ja-JP,ja;q=0.9")
                            .with_languages(["ja-JP", "ja", "en-US", "en"])
                            .with_client_hints(
                                browser::ClientHintsProfile::new()
                                    .with_architecture("arm")
                                    .with_bitness("64")
                                    .with_platform_version("14.0.0")
                                    .with_ua_full_version("136.0.0.0"),
                            ),
                    )
                    .with_screen(
                        browser::ScreenProfile::new()
                            .with_viewport(1440, 900)
                            .with_screen(1728, 1117)
                            .with_avail(1728, 1067),
                    ),
            )
            .with_keep_alive(browser::KeepAlive::Context)
            .with_keep_alive_scope(browser::KeepAliveScope::Origin)
            .with_keep_alive_key("account:primary")
            .with_keep_alive_max_idle(SignedDuration::from_secs(60))
            .with_keep_alive_max_uses(20)
            .with_keep_alive_on_error(browser::KeepAliveOnError::Reset),
    );
```

当前边界：

- `HTML` 与 `XML` 现在都支持 `XPath`
- HTML 响应会先被解析并规范化成稳定 DOM，再执行 `one()`、`all()`、`text()`、`html()` 与 `attr()` 这组统一提取语义
- 当前 browser 定位仍然是“浏览器渲染型下载器”，不是通用自动化框架
- 当前不继续暴露点击、滚动、脚本执行这类页面动作配置

对应示例：

- `examples/browser_advanced.rs`

## AI 选择器

使用 OpenAI API 进行智能内容提取：

```toml
[dependencies]
halo-spider = { version = "0.0.5", features = ["ai-selector"] }
```

```rust
// 设置 API key（优先从环境变量读取）
let config = Config::default()
    .with_openai_api_key(std::env::var("OPENAI_API_KEY").ok().unwrap())
    .with_openai_model("gpt-4o-mini");

// 使用自定义 API endpoint（兼容 OpenAI 的服务）
let config = Config::default()
    .with_openai_api_key("your-api-key")
    .with_openai_base_url("https://your-api-endpoint.com/v1")
    .with_openai_model("your-model-name");

// 在 parse 中使用，支持重试和超时配置
async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
    let mut query = response.ai("Extract the main article title and summary")
        .with_max_retries(3)
        .with_timeout(jiff::SignedDuration::from_secs(30));
    query.execute().await.map_err(|e| SpiderError::parse(e))?;

    if let Some(result) = query.one() {
        println!("AI extracted: {}", result);
    }
    Ok(Output::empty())
}
```

当前特性：

- 自动重试机制
- 可配置超时时间
- 完整错误返回

注意：

- AI 调用会产生 API 费用
- 建议只在复杂内容提取场景使用
- 现阶段它是能力补充，不是爬虫主线的默认路径

对应示例：

- `examples/ai_extraction.rs`
