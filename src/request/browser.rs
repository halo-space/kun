#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Driver {
    #[default]
    Playwright,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Engine {
    #[default]
    Chromium,
    GoogleChrome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub driver: Driver,
    pub engine: Engine,
    pub headless: bool,
    pub stealth: bool,
    pub fingerprint_profile: Option<String>,
    pub wait_for: Option<String>,
    pub viewport: Viewport,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            driver: Driver::default(),
            engine: Engine::default(),
            headless: true,
            stealth: false,
            fingerprint_profile: None,
            wait_for: None,
            viewport: Viewport::default(),
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
    }

    #[test]
    fn config_can_switch_browser_engine_and_profile() {
        let config = Config::default()
            .with_engine(Engine::GoogleChrome)
            .with_stealth(true)
            .with_fingerprint_profile("desktop_zh_cn");

        assert_eq!(config.engine, Engine::GoogleChrome);
        assert!(config.stealth);
        assert_eq!(config.fingerprint_profile.as_deref(), Some("desktop_zh_cn"));
    }
}
