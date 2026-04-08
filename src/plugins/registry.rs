use crate::error::SpiderError;
use crate::plugins::manifest::PluginManifest;
use std::collections::BTreeMap;

type PluginKey = String;

#[derive(Default)]
pub struct PluginRegistry {
    pub manifests: BTreeMap<PluginKey, PluginManifest>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, manifest: PluginManifest) -> Result<(), SpiderError> {
        let key = manifest.name.clone();

        if let Some(existing) = self.manifests.get(&key)
            && !manifest.r#override
        {
            return Err(SpiderError::plugin(format!(
                "plugin conflict: middleware plugin '{}' already registered as '{}'; set override = true to replace",
                key, existing.entry
            )));
        }

        self.manifests.insert(key, manifest);
        Ok(())
    }

    pub fn register_all(&mut self, manifests: Vec<PluginManifest>) -> Result<(), SpiderError> {
        for manifest in manifests {
            self.register(manifest)?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&PluginManifest> {
        self.manifests.get(name)
    }

    pub fn all(&self) -> impl Iterator<Item = &PluginManifest> {
        self.manifests.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manifest(name: &str, override_flag: bool) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            entry: format!("middleware.{name}:Plugin"),
            r#override: override_flag,
        }
    }

    #[test]
    fn register_and_get() {
        let mut registry = PluginRegistry::new();
        registry.register(make_manifest("proxy", false)).unwrap();

        let plugin = registry.get("proxy").unwrap();
        assert_eq!(plugin.entry, "middleware.proxy:Plugin");
    }

    #[test]
    fn same_name_conflict_fails_without_override() {
        let mut registry = PluginRegistry::new();
        registry.register(make_manifest("proxy", false)).unwrap();

        let err = registry
            .register(make_manifest("proxy", false))
            .unwrap_err();
        assert!(err.to_string().contains("plugin conflict"));
    }

    #[test]
    fn same_name_succeeds_with_override() {
        let mut registry = PluginRegistry::new();
        registry.register(make_manifest("proxy", false)).unwrap();
        registry.register(make_manifest("proxy", true)).unwrap();

        assert_eq!(registry.manifests.len(), 1);
    }

    #[test]
    fn all_returns_registered_plugins() {
        let mut registry = PluginRegistry::new();
        registry.register(make_manifest("proxy", false)).unwrap();
        registry.register(make_manifest("cookies", false)).unwrap();

        let names = registry
            .all()
            .map(|manifest| manifest.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["cookies", "proxy"]);
    }
}
