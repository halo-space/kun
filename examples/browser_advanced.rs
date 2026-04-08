//! Browser advanced config example.
//!
//! Shows:
//! - built-in browser request path
//! - structured device profile
//! - optional external stealth script injection
//! - explicit keep_alive policy and bucket key
//! - stable session id plus optional lifecycle controls
//!
//! Run:
//!   cargo run --example browser_advanced

use halo_spider::request::{Request, browser};
use jiff::SignedDuration;

fn main() {
    let request = Request::browser("https://example.com/app")
        .with_session("news-browser")
        .with_browser(
            browser::Config::default()
                .with_engine(browser::Engine::Chromium)
                .with_stealth(true)
                .with_stealth_script("window.__thirdPartyStealth = true;")
                .with_device_profile(
                    browser::DeviceProfile::new()
                        .with_fingerprint(
                            browser::FingerprintProfile::new()
                                .with_user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
                                .with_locale("en-US")
                                .with_timezone("America/Los_Angeles")
                                .with_accept_language("en-US,en;q=0.9")
                                .with_languages(["en-US", "en"])
                                .with_platform("MacIntel")
                                .with_client_hints(
                                    browser::ClientHintsProfile::new()
                                        .with_architecture("arm")
                                        .with_bitness("64")
                                        .with_platform_version("14.4.0")
                                        .with_ua_full_version("136.0.0.0"),
                                )
                                .with_device_memory(8),
                        )
                        .with_screen(
                            browser::ScreenProfile::new()
                                .with_viewport(1440, 900)
                                .with_screen(1728, 1117)
                                .with_avail(1728, 1067)
                                .with_color_depth(24)
                                .with_pixel_depth(24)
                                .with_device_scale_factor(2),
                        ),
                )
                .with_keep_alive(browser::KeepAlive::Context)
                .with_keep_alive_scope(browser::KeepAliveScope::Origin)
                .with_keep_alive_key("account:primary")
                .with_keep_alive_max_idle(SignedDuration::from_secs(60))
                .with_keep_alive_max_uses(20)
                .with_keep_alive_on_error(browser::KeepAliveOnError::Reset)
                .with_wait_for_selector("#app"),
        );

    println!("{:#?}", request.browser);
}
