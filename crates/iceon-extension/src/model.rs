//! Data models and metadata types for Iceon native extensions.

use serde::{Deserialize, Serialize};

/// Unique identifier for a registered extension.
pub type ExtensionId = &'static str;

/// Category classification for grouping extensions in UI and settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionCategory {
    Autoscaling,
    GitOps,
    Database,
    StreamingAndMessaging,
    Networking,
    Security,
    StorageAndBackup,
    ClusterManagement,
    ComputeAndHardware,
    AiAndLlm,
    Observability,
    Workloads,
    General,
}

impl ExtensionCategory {
    /// Human-readable title for the category.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Autoscaling => "Autoscaling",
            Self::GitOps => "GitOps",
            Self::Database => "Database",
            Self::StreamingAndMessaging => "Streaming & Messaging",
            Self::Networking => "Networking",
            Self::Security => "Security",
            Self::StorageAndBackup => "Storage & Backup",
            Self::ClusterManagement => "Cluster Management",
            Self::ComputeAndHardware => "Compute & Hardware",
            Self::AiAndLlm => "AI & LLM",
            Self::Observability => "Observability",
            Self::Workloads => "Workloads",
            Self::General => "General",
        }
    }

    /// List all categories in display order.
    pub fn all() -> &'static [Self] {
        &[
            Self::Autoscaling,
            Self::GitOps,
            Self::Database,
            Self::StreamingAndMessaging,
            Self::Networking,
            Self::Security,
            Self::StorageAndBackup,
            Self::ClusterManagement,
            Self::ComputeAndHardware,
            Self::AiAndLlm,
            Self::Observability,
            Self::Workloads,
            Self::General,
        ]
    }
}

/// A Custom Resource Definition (CRD) managed or presented by an extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrdDefinition {
    /// API group, e.g. `keda.sh`.
    pub group: &'static str,
    /// API version, e.g. `v1alpha1`.
    pub version: &'static str,
    /// Resource Kind, e.g. `ScaledObject`.
    pub kind: &'static str,
    /// Plural name used in URL endpoints and kubectl, e.g. `scaledobjects`.
    pub plural: &'static str,
    /// Whether this resource is namespace-scoped or cluster-scoped.
    pub namespaced: bool,
    /// Human-friendly display title, e.g. "Scaled Objects".
    pub title: &'static str,
    /// Short summary of the resource's role.
    pub description: &'static str,
}

impl CrdDefinition {
    /// Full apiVersion string, e.g. `keda.sh/v1alpha1`.
    pub fn api_version(&self) -> String {
        format!("{}/{}", self.group, self.version)
    }
}

/// Sidebar navigation contribution made by an active extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SidebarContribution {
    /// Unique item identifier within the extension.
    pub id: &'static str,
    /// Title rendered in the sidebar tree.
    pub title: &'static str,
    /// Optional parent menu/group ID (e.g. grouped under "keda" or "fluxcd").
    pub parent_id: Option<&'static str>,
    /// Associated resource kind, if clicking navigates to a resource table.
    pub target_kind: Option<&'static str>,
    /// Associated resource API group.
    pub target_group: Option<&'static str>,
    /// Icon name or symbol.
    pub icon: Option<&'static str>,
}

/// Custom inspector facts or sections contributed for specific CRDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectorContribution {
    /// Target API group.
    pub group: &'static str,
    /// Target resource Kind.
    pub kind: &'static str,
    /// Section titles to render in the resource inspector.
    pub custom_sections: &'static [&'static str],
}

/// Complete manifest definition for a built-in native extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionManifest {
    /// Unique lowercase identifier, e.g. `keda`.
    pub id: ExtensionId,
    /// Display name, e.g. "KEDA".
    pub name: &'static str,
    /// Original Freelens package name, e.g. `@freelensapp/keda-extension`.
    pub package_name: &'static str,
    /// Version string, matching the upstream extension version.
    pub version: &'static str,
    /// Detailed description of the extension features.
    pub description: &'static str,
    /// Author / organization.
    pub author: &'static str,
    /// Repository or documentation URL.
    pub homepage: &'static str,
    /// Functional category.
    pub category: ExtensionCategory,
    /// Whether the extension is enabled by default.
    pub default_enabled: bool,
    /// CRDs provided and managed by this extension.
    pub crds: &'static [CrdDefinition],
    /// Navigation sidebar items contributed when active.
    pub sidebar_items: &'static [SidebarContribution],
    /// Custom inspector views contributed when active.
    pub inspectors: &'static [InspectorContribution],
}
