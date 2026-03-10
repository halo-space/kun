use crate::plugins::manifest::PluginManifest;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct PluginRegistry {
    pub manifests: BTreeMap<(String, String), PluginManifest>,
}
