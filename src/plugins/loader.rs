use crate::error::SpiderError;
use crate::plugins::manifest::PluginManifest;
use serde::Deserialize;

#[derive(Deserialize)]
struct ManifestFile {
    #[serde(default)]
    plugins: Vec<PluginManifest>,
}

pub fn load_plugin_manifest(path: &str) -> Result<Vec<PluginManifest>, SpiderError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| SpiderError::plugin(format!("failed to read plugins.toml: {e}")))?;

    let manifest: ManifestFile = toml::from_str(&content)
        .map_err(|e| SpiderError::plugin(format!("failed to parse plugins.toml: {e}")))?;

    Ok(manifest.plugins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_plugins_toml() {
        let toml_content = r#"
[[plugins]]
name = "custom_signature"
kind = "middleware"
entry = "myproject.plugins.custom_signature:Plugin"
override = false

[[plugins]]
name = "local"
kind = "rules"
entry = "myproject.plugins.local_rules:Plugin"
"#;

        let tmp = std::env::temp_dir().join("test_plugins.toml");
        std::fs::write(&tmp, toml_content).unwrap();

        let manifests = load_plugin_manifest(tmp.to_str().unwrap()).unwrap();

        assert_eq!(manifests.len(), 2);
        assert_eq!(manifests[0].name, "custom_signature");
        assert_eq!(manifests[0].kind, "middleware");
        assert!(!manifests[0].r#override);
        assert_eq!(manifests[1].name, "local");
        assert_eq!(manifests[1].kind, "rules");

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn missing_file_returns_error() {
        let result = load_plugin_manifest("/nonexistent/plugins.toml");
        assert!(result.is_err());
    }
}
