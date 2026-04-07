//! Browser advanced config example.
//!
//! Shows:
//! - built-in browser request path
//! - structured fingerprint profile
//! - optional external stealth script injection
//! - explicit keep_alive policy
//! - stable session id plus optional origin-scoped live reuse boundaries
//!
//! Run:
//!   cargo run --example browser_advanced

use halo_spider::request::{Request, browser};

fn main() {
    let request = Request::browser("https://example.com/app")
        .with_session("news-browser")
        .with_browser(
            browser::Config::default()
                .with_engine(browser::Engine::Chromium)
                .with_stealth(true)
                .with_stealth_script("window.__thirdPartyStealth = true;")
                .with_fingerprint_profile(
                    browser::FingerprintProfile::new()
                        .with_user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
                        .with_locale("en-US")
                        .with_timezone("America/Los_Angeles")
                        .with_accept_language("en-US,en;q=0.9")
                        .with_languages(["en-US", "en"])
                        .with_platform("MacIntel")
                        .with_vendor("Google Inc.")
                        .with_hardware_concurrency(10)
                        .with_device_memory(8)
                        .with_max_touch_points(0),
                )
                .with_keep_alive(browser::KeepAlive::Context)
                .with_keep_alive_scope(browser::KeepAliveScope::Origin)
                .with_wait_for_selector("#app"),
        );

    println!("{:#?}", request.browser);
}
