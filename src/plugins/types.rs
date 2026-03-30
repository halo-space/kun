#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    Middleware,
    Rules,
    Provider,
    Storage,
}

const KNOWN_PLUGIN_KIND_NAMES: [&str; 4] = ["middleware", "rules", "provider", "storage"];
const ENGINE_SUPPORTED_PLUGIN_KIND_NAMES: [&str; 1] = ["middleware"];
const ENGINE_RESERVED_PLUGIN_KIND_NAMES: [&str; 3] = ["rules", "provider", "storage"];

impl PluginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Middleware => "middleware",
            Self::Rules => "rules",
            Self::Provider => "provider",
            Self::Storage => "storage",
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
            "rules" => Ok(Self::Rules),
            "provider" => Ok(Self::Provider),
            "storage" => Ok(Self::Storage),
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

pub fn engine_reserved_plugin_kind_names() -> &'static [&'static str] {
    &ENGINE_RESERVED_PLUGIN_KIND_NAMES
}
