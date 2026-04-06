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
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintProfile {
    pub user_agent: String,
    pub locale: String,
    pub timezone: String,
    pub accept_language: String,
    pub languages: Vec<String>,
    pub platform: String,
    pub vendor: String,
    pub hardware_concurrency: u8,
    pub device_memory: u8,
    pub max_touch_points: u8,
}

impl Default for FingerprintProfile {
    fn default() -> Self {
        Self {
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".to_string(),
            locale: "en-US".to_string(),
            timezone: "America/New_York".to_string(),
            accept_language: "en-US,en;q=0.9".to_string(),
            languages: vec!["en-US".to_string(), "en".to_string()],
            platform: "Win32".to_string(),
            vendor: "Google Inc.".to_string(),
            hardware_concurrency: 8,
            device_memory: 8,
            max_touch_points: 0,
        }
    }
}

impl FingerprintProfile {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = locale.into();
        self
    }

    pub fn with_timezone(mut self, timezone: impl Into<String>) -> Self {
        self.timezone = timezone.into();
        self
    }

    pub fn with_accept_language(mut self, accept_language: impl Into<String>) -> Self {
        self.accept_language = accept_language.into();
        self
    }

    pub fn with_languages<I, S>(mut self, languages: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.languages = languages.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = platform.into();
        self
    }

    pub fn with_vendor(mut self, vendor: impl Into<String>) -> Self {
        self.vendor = vendor.into();
        self
    }

    pub fn with_hardware_concurrency(mut self, hardware_concurrency: u8) -> Self {
        self.hardware_concurrency = hardware_concurrency;
        self
    }

    pub fn with_device_memory(mut self, device_memory: u8) -> Self {
        self.device_memory = device_memory;
        self
    }

    pub fn with_max_touch_points(mut self, max_touch_points: u8) -> Self {
        self.max_touch_points = max_touch_points;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionReuse {
    #[default]
    Storage,
    Context,
    Page,
}

impl SessionReuse {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Storage => "storage",
            Self::Context => "context",
            Self::Page => "page",
        }
    }
}

impl Display for SessionReuse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for SessionReuse {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "storage" => Ok(Self::Storage),
            "context" => Ok(Self::Context),
            "page" => Ok(Self::Page),
            other => Err(format!("unsupported browser session reuse: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub driver: Driver,
    pub engine: Engine,
    pub headless: bool,
    pub stealth: bool,
    pub fingerprint_profile: Option<String>,
    #[serde(default)]
    pub custom_fingerprint_profile: Option<FingerprintProfile>,
    pub wait_for: Option<String>,
    pub viewport: Viewport,
    #[serde(default)]
    pub session_reuse: SessionReuse,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            driver: Driver::default(),
            engine: Engine::default(),
            headless: true,
            stealth: false,
            fingerprint_profile: None,
            custom_fingerprint_profile: None,
            wait_for: None,
            viewport: Viewport::default(),
            session_reuse: SessionReuse::default(),
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

    pub fn with_fingerprint_profile(mut self, profile: impl Into<String>) -> Self {
        self.fingerprint_profile = Some(profile.into());
        self.custom_fingerprint_profile = None;
        self
    }

    pub fn with_custom_fingerprint_profile(mut self, profile: FingerprintProfile) -> Self {
        self.custom_fingerprint_profile = Some(profile);
        self.fingerprint_profile = None;
        self
    }

    pub fn with_wait_for(mut self, selector: impl Into<String>) -> Self {
        self.wait_for = Some(selector.into());
        self
    }

    pub fn with_viewport(mut self, width: u32, height: u32) -> Self {
        self.viewport = Viewport { width, height };
        self
    }

    pub fn with_session_reuse(mut self, session_reuse: SessionReuse) -> Self {
        self.session_reuse = session_reuse;
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
        assert_eq!(config.fingerprint_profile, None);
        assert_eq!(config.custom_fingerprint_profile, None);
        assert_eq!(config.wait_for, None);
        assert_eq!(config.session_reuse, SessionReuse::Storage);
    }

    #[test]
    fn config_can_switch_browser_engine_builtin_profile_and_reuse_policy() {
        let config = Config::default()
            .with_engine(Engine::Firefox)
            .with_stealth(true)
            .with_fingerprint_profile("desktop_zh_cn")
            .with_wait_for("#app")
            .with_session_reuse(SessionReuse::Context);

        assert_eq!(config.engine, Engine::Firefox);
        assert!(config.stealth);
        assert_eq!(config.fingerprint_profile.as_deref(), Some("desktop_zh_cn"));
        assert_eq!(config.custom_fingerprint_profile, None);
        assert_eq!(config.wait_for.as_deref(), Some("#app"));
        assert_eq!(config.session_reuse, SessionReuse::Context);
    }

    #[test]
    fn config_can_switch_to_custom_fingerprint_profile() {
        let custom = FingerprintProfile::new()
            .with_locale("ja-JP")
            .with_timezone("Asia/Tokyo")
            .with_accept_language("ja-JP,ja;q=0.9")
            .with_languages(["ja-JP", "ja"]);
        let config = Config::default()
            .with_fingerprint_profile("desktop_en_us")
            .with_custom_fingerprint_profile(custom.clone());

        assert_eq!(config.fingerprint_profile, None);
        assert_eq!(config.custom_fingerprint_profile, Some(custom));
    }

    #[test]
    fn session_reuse_try_from_string_supports_explicit_policies() {
        assert_eq!(SessionReuse::try_from("storage"), Ok(SessionReuse::Storage));
        assert_eq!(SessionReuse::try_from("context"), Ok(SessionReuse::Context));
        assert_eq!(SessionReuse::try_from("page"), Ok(SessionReuse::Page));
        assert_eq!(
            SessionReuse::try_from("other"),
            Err("unsupported browser session reuse: other".to_string())
        );
    }
}
