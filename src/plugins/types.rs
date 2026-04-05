#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    Middleware,
    Store,
    Scheduler,
    Dedup,
    Robots,
    Http,
    Browser,
}

const KNOWN_PLUGIN_KIND_NAMES: [&str; 7] = [
    "middleware",
    "store",
    "scheduler",
    "dedup",
    "robots",
    "http",
    "browser",
];
const ENGINE_SUPPORTED_PLUGIN_KIND_NAMES: [&str; 1] = ["middleware"];
const ENGINE_DEFERRED_PLUGIN_KIND_NAMES: [&str; 6] =
    ["store", "scheduler", "dedup", "robots", "http", "browser"];

impl PluginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Middleware => "middleware",
            Self::Store => "store",
            Self::Scheduler => "scheduler",
            Self::Dedup => "dedup",
            Self::Robots => "robots",
            Self::Http => "http",
            Self::Browser => "browser",
        }
    }

    pub fn is_engine_supported(self) -> bool {
        matches!(self, Self::Middleware)
    }
}

impl TryFrom<&str> for PluginKind {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "middleware" => Ok(Self::Middleware),
            "store" => Ok(Self::Store),
            "scheduler" => Ok(Self::Scheduler),
            "dedup" => Ok(Self::Dedup),
            "robots" => Ok(Self::Robots),
            "http" => Ok(Self::Http),
            "browser" => Ok(Self::Browser),
            other => Err(format!(
                "unsupported plugin kind '{other}'; known kinds: {}",
                KNOWN_PLUGIN_KIND_NAMES.join(", ")
            )),
        }
    }
}

pub fn known_plugin_kind_names() -> &'static [&'static str] {
    &KNOWN_PLUGIN_KIND_NAMES
}

pub fn engine_supported_plugin_kind_names() -> &'static [&'static str] {
    &ENGINE_SUPPORTED_PLUGIN_KIND_NAMES
}

pub fn engine_deferred_plugin_kind_names() -> &'static [&'static str] {
    &ENGINE_DEFERRED_PLUGIN_KIND_NAMES
}
