use jiff::SignedDuration;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Driver {
    #[default]
    Playwright,
}

impl Driver {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Playwright => "playwright",
        }
    }
}

impl Display for Driver {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for Driver {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "playwright" => Ok(Self::Playwright),
            other => Err(format!("unsupported browser driver: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    #[default]
    Chromium,
    Firefox,
    Webkit,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chromium => "chromium",
            Self::Firefox => "firefox",
            Self::Webkit => "webkit",
        }
    }
}

impl Display for Engine {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for Engine {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "chromium" => Ok(Self::Chromium),
            "firefox" => Ok(Self::Firefox),
            "webkit" => Ok(Self::Webkit),
            other => Err(format!("unsupported browser engine: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FingerprintProfile {
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub accept_language: Option<String>,
    #[serde(default)]
    pub languages: Option<Vec<String>>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub device_memory: Option<u8>,
}

impl FingerprintProfile {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    pub fn with_timezone(mut self, timezone: impl Into<String>) -> Self {
        self.timezone = Some(timezone.into());
        self
    }

    pub fn with_accept_language(mut self, accept_language: impl Into<String>) -> Self {
        self.accept_language = Some(accept_language.into());
        self
    }

    pub fn with_languages<I, S>(mut self, languages: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.languages = Some(languages.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }

    pub fn with_device_memory(mut self, device_memory: u8) -> Self {
        self.device_memory = Some(device_memory);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScreenProfile {
    #[serde(default)]
    pub viewport: Option<Size>,
    #[serde(default)]
    pub screen: Option<Size>,
    #[serde(default)]
    pub avail: Option<Size>,
    #[serde(default)]
    pub color_depth: Option<u8>,
    #[serde(default)]
    pub pixel_depth: Option<u8>,
    #[serde(default)]
    pub device_scale_factor: Option<u32>,
}

impl ScreenProfile {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_viewport(mut self, width: u32, height: u32) -> Self {
        self.viewport = Some(Size::new(width, height));
        self
    }

    pub fn with_viewport_size(mut self, viewport: Size) -> Self {
        self.viewport = Some(viewport);
        self
    }

    pub fn with_screen(mut self, width: u32, height: u32) -> Self {
        self.screen = Some(Size::new(width, height));
        self
    }

    pub fn with_screen_size(mut self, screen: Size) -> Self {
        self.screen = Some(screen);
        self
    }

    pub fn with_avail(mut self, width: u32, height: u32) -> Self {
        self.avail = Some(Size::new(width, height));
        self
    }

    pub fn with_avail_size(mut self, avail: Size) -> Self {
        self.avail = Some(avail);
        self
    }

    pub fn with_color_depth(mut self, color_depth: u8) -> Self {
        self.color_depth = Some(color_depth);
        self
    }

    pub fn with_pixel_depth(mut self, pixel_depth: u8) -> Self {
        self.pixel_depth = Some(pixel_depth);
        self
    }

    pub fn with_device_scale_factor(mut self, device_scale_factor: u32) -> Self {
        self.device_scale_factor = Some(device_scale_factor);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DeviceProfile {
    #[serde(default)]
    pub fingerprint: Option<FingerprintProfile>,
    #[serde(default)]
    pub screen: Option<ScreenProfile>,
}

impl DeviceProfile {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_fingerprint(mut self, fingerprint: FingerprintProfile) -> Self {
        self.fingerprint = Some(fingerprint);
        self
    }

    pub fn with_screen(mut self, screen: ScreenProfile) -> Self {
        self.screen = Some(screen);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepAlive {
    #[default]
    Isolated,
    Context,
    Page,
}

impl KeepAlive {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Isolated => "isolated",
            Self::Context => "context",
            Self::Page => "page",
        }
    }
}

impl Display for KeepAlive {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for KeepAlive {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "isolated" => Ok(Self::Isolated),
            "context" => Ok(Self::Context),
            "page" => Ok(Self::Page),
            other => Err(format!("unsupported browser keep_alive: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepAliveScope {
    #[default]
    Session,
    Origin,
}

impl KeepAliveScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Origin => "origin",
        }
    }
}

impl Display for KeepAliveScope {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for KeepAliveScope {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "session" => Ok(Self::Session),
            "origin" => Ok(Self::Origin),
            other => Err(format!("unsupported browser keep_alive_scope: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepAliveOnError {
    #[default]
    Keep,
    Reset,
}

impl KeepAliveOnError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Reset => "reset",
        }
    }
}

impl Display for KeepAliveOnError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for KeepAliveOnError {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "keep" => Ok(Self::Keep),
            "reset" => Ok(Self::Reset),
            other => Err(format!("unsupported browser keep_alive_on_error: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub driver: Driver,
    pub engine: Engine,
    pub headless: bool,
    pub stealth: bool,
    #[serde(default)]
    pub stealth_scripts: Vec<String>,
    #[serde(default)]
    pub device_profile: Option<DeviceProfile>,
    pub wait_for_selector: Option<String>,
    #[serde(default)]
    pub keep_alive: KeepAlive,
    #[serde(default)]
    pub keep_alive_scope: KeepAliveScope,
    #[serde(default)]
    pub keep_alive_key: Option<String>,
    #[serde(default, with = "super::option_signed_duration_millis")]
    pub keep_alive_max_idle: Option<SignedDuration>,
    #[serde(default)]
    pub keep_alive_max_uses: Option<u64>,
    #[serde(default)]
    pub keep_alive_on_error: KeepAliveOnError,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            driver: Driver::default(),
            engine: Engine::default(),
            headless: true,
            stealth: false,
            stealth_scripts: Vec::new(),
            device_profile: None,
            wait_for_selector: None,
            keep_alive: KeepAlive::default(),
            keep_alive_scope: KeepAliveScope::default(),
            keep_alive_key: None,
            keep_alive_max_idle: None,
            keep_alive_max_uses: None,
            keep_alive_on_error: KeepAliveOnError::default(),
        }
    }
}

impl Config {
    pub fn with_driver(mut self, driver: Driver) -> Self {
        self.driver = driver;
        self
    }

    pub fn with_engine(mut self, engine: Engine) -> Self {
        self.engine = engine;
        self
    }

    pub fn with_headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    pub fn with_stealth(mut self, stealth: bool) -> Self {
        self.stealth = stealth;
        self
    }

    pub fn with_stealth_script(mut self, script: impl Into<String>) -> Self {
        self.stealth_scripts.push(script.into());
        self
    }

    pub fn with_stealth_scripts<I, S>(mut self, scripts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.stealth_scripts = scripts.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_device_profile(mut self, device_profile: DeviceProfile) -> Self {
        self.device_profile = Some(device_profile);
        self
    }

    pub fn with_wait_for_selector(mut self, selector: impl Into<String>) -> Self {
        self.wait_for_selector = Some(selector.into());
        self
    }

    pub fn with_keep_alive(mut self, keep_alive: KeepAlive) -> Self {
        self.keep_alive = keep_alive;
        self
    }

    pub fn with_keep_alive_scope(mut self, keep_alive_scope: KeepAliveScope) -> Self {
        self.keep_alive_scope = keep_alive_scope;
        self
    }

    pub fn with_keep_alive_key(mut self, keep_alive_key: impl Into<String>) -> Self {
        self.keep_alive_key = Some(keep_alive_key.into());
        self
    }

    pub fn with_keep_alive_max_idle(mut self, keep_alive_max_idle: SignedDuration) -> Self {
        self.keep_alive_max_idle = Some(keep_alive_max_idle);
        self
    }

    pub fn with_keep_alive_max_uses(mut self, keep_alive_max_uses: u64) -> Self {
        self.keep_alive_max_uses = Some(keep_alive_max_uses);
        self
    }

    pub fn with_keep_alive_on_error(mut self, keep_alive_on_error: KeepAliveOnError) -> Self {
        self.keep_alive_on_error = keep_alive_on_error;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_reserves_browser_core_fields() {
        let config = Config::default();

        assert_eq!(config.driver, Driver::Playwright);
        assert_eq!(config.engine, Engine::Chromium);
        assert!(config.headless);
        assert!(!config.stealth);
        assert!(config.stealth_scripts.is_empty());
        assert_eq!(config.device_profile, None);
        assert_eq!(config.wait_for_selector, None);
        assert_eq!(config.keep_alive, KeepAlive::Isolated);
        assert_eq!(config.keep_alive_scope, KeepAliveScope::Session);
        assert_eq!(config.keep_alive_key, None);
        assert_eq!(config.keep_alive_max_idle, None);
        assert_eq!(config.keep_alive_max_uses, None);
        assert_eq!(config.keep_alive_on_error, KeepAliveOnError::Keep);
    }

    #[test]
    fn config_can_switch_browser_engine_device_profile_and_reuse_policy() {
        let config = Config::default()
            .with_engine(Engine::Firefox)
            .with_stealth(true)
            .with_stealth_script("window.__thirdPartyStealth = true;")
            .with_device_profile(
                DeviceProfile::new()
                    .with_fingerprint(
                        FingerprintProfile::new()
                            .with_user_agent("custom-agent")
                            .with_locale("ja-JP")
                            .with_timezone("Asia/Tokyo")
                            .with_accept_language("ja-JP,ja;q=0.9")
                            .with_languages(["ja-JP", "ja"])
                            .with_platform("MacIntel")
                            .with_device_memory(16),
                    )
                    .with_screen(
                        ScreenProfile::new()
                            .with_viewport(1440, 900)
                            .with_screen(1728, 1117)
                            .with_avail(1728, 1067)
                            .with_color_depth(24)
                            .with_pixel_depth(24)
                            .with_device_scale_factor(2),
                    ),
            )
            .with_wait_for_selector("#app")
            .with_keep_alive(KeepAlive::Context)
            .with_keep_alive_scope(KeepAliveScope::Origin)
            .with_keep_alive_key("account:primary")
            .with_keep_alive_max_idle(SignedDuration::from_secs(30))
            .with_keep_alive_max_uses(20)
            .with_keep_alive_on_error(KeepAliveOnError::Reset);

        assert_eq!(config.engine, Engine::Firefox);
        assert!(config.stealth);
        assert_eq!(
            config.stealth_scripts,
            vec!["window.__thirdPartyStealth = true;".to_string()]
        );
        assert_eq!(
            config
                .device_profile
                .as_ref()
                .and_then(|profile| profile.fingerprint.as_ref())
                .and_then(|fingerprint| fingerprint.locale.as_deref()),
            Some("ja-JP")
        );
        assert_eq!(
            config
                .device_profile
                .as_ref()
                .and_then(|profile| profile.screen.as_ref())
                .and_then(|screen| screen.viewport.as_ref())
                .cloned(),
            Some(Size::new(1440, 900))
        );
        assert_eq!(config.wait_for_selector.as_deref(), Some("#app"));
        assert_eq!(config.keep_alive, KeepAlive::Context);
        assert_eq!(config.keep_alive_scope, KeepAliveScope::Origin);
        assert_eq!(config.keep_alive_key.as_deref(), Some("account:primary"));
        assert_eq!(
            config.keep_alive_max_idle,
            Some(SignedDuration::from_secs(30))
        );
        assert_eq!(config.keep_alive_max_uses, Some(20));
        assert_eq!(config.keep_alive_on_error, KeepAliveOnError::Reset);
    }

    #[test]
    fn config_can_replace_external_stealth_scripts() {
        let config = Config::default()
            .with_stealth_script("window.__stealthA = true;")
            .with_stealth_scripts(["window.__stealthB = true;", "window.__stealthC = true;"]);

        assert_eq!(
            config.stealth_scripts,
            vec![
                "window.__stealthB = true;".to_string(),
                "window.__stealthC = true;".to_string(),
            ]
        );
    }

    #[test]
    fn keep_alive_try_from_string_supports_explicit_policies() {
        assert_eq!(KeepAlive::try_from("isolated"), Ok(KeepAlive::Isolated));
        assert_eq!(KeepAlive::try_from("context"), Ok(KeepAlive::Context));
        assert_eq!(KeepAlive::try_from("page"), Ok(KeepAlive::Page));
        assert_eq!(
            KeepAlive::try_from("other"),
            Err("unsupported browser keep_alive: other".to_string())
        );
    }

    #[test]
    fn keep_alive_scope_try_from_string_supports_explicit_policies() {
        assert_eq!(
            KeepAliveScope::try_from("session"),
            Ok(KeepAliveScope::Session)
        );
        assert_eq!(
            KeepAliveScope::try_from("origin"),
            Ok(KeepAliveScope::Origin)
        );
        assert_eq!(
            KeepAliveScope::try_from("other"),
            Err("unsupported browser keep_alive_scope: other".to_string())
        );
    }

    #[test]
    fn keep_alive_on_error_try_from_string_supports_explicit_policies() {
        assert_eq!(
            KeepAliveOnError::try_from("keep"),
            Ok(KeepAliveOnError::Keep)
        );
        assert_eq!(
            KeepAliveOnError::try_from("reset"),
            Ok(KeepAliveOnError::Reset)
        );
        assert_eq!(
            KeepAliveOnError::try_from("other"),
            Err("unsupported browser keep_alive_on_error: other".to_string())
        );
    }
}
