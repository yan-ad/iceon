//! Registry managing native built-in extensions and their active lifecycle.

use std::collections::{BTreeMap, BTreeSet};

use crate::builtin::ALL_BUILTIN_EXTENSIONS;
use crate::model::{
    CrdDefinition, ExtensionCategory, ExtensionId, ExtensionManifest, SidebarContribution,
};

/// In-memory catalog and registry of all available native extensions.
#[derive(Debug, Clone, Default)]
pub struct ExtensionRegistry {
    extensions: BTreeMap<ExtensionId, &'static ExtensionManifest>,
}

impl ExtensionRegistry {
    /// Create a new registry populated with all built-in extensions.
    pub fn new() -> Self {
        let mut extensions = BTreeMap::new();
        for &manifest in ALL_BUILTIN_EXTENSIONS {
            extensions.insert(manifest.id, manifest);
        }
        Self { extensions }
    }

    /// Retrieve all registered extension manifests in deterministic order.
    pub fn all(&self) -> Vec<&'static ExtensionManifest> {
        self.extensions.values().copied().collect()
    }

    /// Lookup an extension manifest by its unique ID.
    pub fn by_id(&self, id: &str) -> Option<&'static ExtensionManifest> {
        self.extensions.get(id).copied()
    }

    /// Filter extensions matching a specific category.
    pub fn by_category(&self, category: ExtensionCategory) -> Vec<&'static ExtensionManifest> {
        self.extensions
            .values()
            .copied()
            .filter(|ext| ext.category == category)
            .collect()
    }

    /// Check if a given extension is enabled according to user preferences.
    pub fn is_enabled(&self, id: &str, enabled_set: &BTreeSet<String>) -> bool {
        if enabled_set.contains(id) {
            return true;
        }
        // Fall back to manifest default if not explicitly set
        self.by_id(id).is_some_and(|ext| ext.default_enabled)
    }

    /// Filter all active/enabled extensions for a given set of enabled IDs.
    pub fn enabled_extensions(
        &self,
        enabled_set: &BTreeSet<String>,
    ) -> Vec<&'static ExtensionManifest> {
        self.extensions
            .values()
            .copied()
            .filter(|ext| self.is_enabled(ext.id, enabled_set))
            .collect()
    }

    /// Get all CRDs contributed by currently enabled extensions.
    pub fn crds_for_enabled(&self, enabled_set: &BTreeSet<String>) -> Vec<&'static CrdDefinition> {
        self.enabled_extensions(enabled_set)
            .into_iter()
            .flat_map(|ext| ext.crds.iter())
            .collect()
    }

    /// Get all sidebar items contributed by currently enabled extensions.
    pub fn sidebar_items_for_enabled(
        &self,
        enabled_set: &BTreeSet<String>,
    ) -> Vec<&'static SidebarContribution> {
        self.enabled_extensions(enabled_set)
            .into_iter()
            .flat_map(|ext| ext.sidebar_items.iter())
            .collect()
    }

    /// Find a CRD definition by group and kind across all registered extensions.
    pub fn find_crd(&self, group: &str, kind: &str) -> Option<&'static CrdDefinition> {
        for ext in self.extensions.values() {
            for crd in ext.crds {
                if crd.group.eq_ignore_ascii_case(group) && crd.kind.eq_ignore_ascii_case(kind) {
                    return Some(crd);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_all_18_freelens_extensions() {
        let registry = ExtensionRegistry::new();
        assert_eq!(registry.all().len(), 18);

        let required_ids = [
            "keda",
            "fluxcd",
            "cnpg",
            "gateway-api",
            "eso",
            "kafka",
            "rabbitmq",
            "velero",
            "karpenter",
            "kamaji",
            "sveltos",
            "gpu",
            "vals",
            "resource-map",
            "ai",
            "for-claude",
            "agentbridge",
            "kubeswift",
        ];

        for id in required_ids {
            assert!(
                registry.by_id(id).is_some(),
                "missing required extension: {id}"
            );
        }
    }

    #[test]
    fn enabled_filtering_works_correctly() {
        let registry = ExtensionRegistry::new();
        let mut enabled = BTreeSet::new();

        assert!(registry.enabled_extensions(&enabled).is_empty());
        assert!(!registry.is_enabled("keda", &enabled));

        enabled.insert("keda".into());
        enabled.insert("cnpg".into());

        assert_eq!(registry.enabled_extensions(&enabled).len(), 2);
        assert!(registry.is_enabled("keda", &enabled));
        assert!(registry.is_enabled("cnpg", &enabled));
        assert!(!registry.is_enabled("fluxcd", &enabled));

        let crds = registry.crds_for_enabled(&enabled);
        assert!(crds.iter().any(|c| c.kind == "ScaledObject"));
        assert!(crds.iter().any(|c| c.kind == "Cluster"));

        let sidebar = registry.sidebar_items_for_enabled(&enabled);
        assert!(sidebar.iter().any(|s| s.title == "KEDA"));
        assert!(sidebar.iter().any(|s| s.title == "CloudNativePG"));
    }

    #[test]
    fn crd_lookups_are_case_insensitive() {
        let registry = ExtensionRegistry::new();
        let crd = registry.find_crd("keda.sh", "scaledobject");
        assert!(crd.is_some());
        let crd = crd.unwrap();
        assert_eq!(crd.group, "keda.sh");
        assert_eq!(crd.kind, "ScaledObject");
        assert_eq!(crd.plural, "scaledobjects");
        assert!(crd.namespaced);
    }
}
