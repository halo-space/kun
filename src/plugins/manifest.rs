use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub entry: String,
    #[serde(default)]
    pub r#override: bool,
}
