use crate::error::SpiderError;
use crate::plugins::manifest::PluginManifest;

pub fn load_plugin_manifest(_path: &str) -> Result<Vec<PluginManifest>, SpiderError> {
    Ok(Vec::new())
}
