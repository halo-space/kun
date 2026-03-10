#[derive(Debug, Clone, Default)]
pub struct PluginManifest {
    pub name: String,
    pub kind: String,
    pub entry: String,
    pub override_builtin: bool,
}
