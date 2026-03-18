use crate::error::SpiderError;
use crate::plugins::manifest::PluginManifest;
use std::collections::BTreeMap;

type PluginKey = (String, String);

#[derive(Default)]
pub struct PluginRegistry {
    pub manifests: BTreeMap<PluginKey, PluginManifest>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, manifest: PluginManifest) -> Result<(), SpiderError> {
        let key = (manifest.kind.clone(), manifest.name.clone());

        if let Some(existing) = self.manifests.get(&key) {
            if !manifest.r#override {
                return Err(SpiderError::plugin(format!(
                    "plugin conflict: ({}, {}) already registered as '{}'; set override = true to replace",
                    key.0, key.1, existing.entry
                )));
            }
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

    pub fn get(&self, kind: &str, name: &str) -> Option<&PluginManifest> {
        self.manifests.get(&(kind.to_string(), name.to_string()))
    }

    pub fn all(&self) -> impl Iterator<Item = &PluginManifest> {
        self.manifests.values()
    }

    pub fn by_kind(&self, kind: &str) -> Vec<&PluginManifest> {
        self.manifests
            .iter()
            .filter(|((k, _), _)| k == kind)
            .map(|(_, m)| m)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manifest(kind: &str, name: &str, override_flag: bool) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            kind: kind.to_string(),
            entry: format!("{kind}.{name}:Plugin"),
            r#override: override_flag,
        }
    }

    #[test]
    fn register_and_get() {
        let mut registry = PluginRegistry::new();
        registry.register(make_manifest("middleware", "proxy", false)).unwrap();

        let plugin = registry.get("middleware", "proxy").unwrap();
        assert_eq!(plugin.entry, "middleware.proxy:Plugin");
    }

    #[test]
    fn same_kind_same_name_conflict_fails_without_override() {
        let mut registry = PluginRegistry::new();
        registry.register(make_manifest("middleware", "proxy", false)).unwrap();

        let err = registry.register(make_manifest("middleware", "proxy", false)).unwrap_err();
        assert!(err.to_string().contains("plugin conflict"));
    }

    #[test]
    fn same_kind_same_name_succeeds_with_override() {
        let mut registry = PluginRegistry::new();
        registry.register(make_manifest("middleware", "proxy", false)).unwrap();
        registry.register(make_manifest("middleware", "proxy", true)).unwrap();

        assert_eq!(registry.manifests.len(), 1);
    }

    #[test]
    fn different_kind_same_name_allowed() {
        let mut registry = PluginRegistry::new();
        registry.register(make_manifest("middleware", "redis", false)).unwrap();
        registry.register(make_manifest("rules", "redis", false)).unwrap();

        assert_eq!(registry.manifests.len(), 2);
    }

    #[test]
    fn by_kind_filters_correctly() {
        let mut registry = PluginRegistry::new();
        registry.register(make_manifest("middleware", "proxy", false)).unwrap();
        registry.register(make_manifest("middleware", "cookies", false)).unwrap();
        registry.register(make_manifest("rules", "local", false)).unwrap();

        assert_eq!(registry.by_kind("middleware").len(), 2);
        assert_eq!(registry.by_kind("rules").len(), 1);
    }
}
