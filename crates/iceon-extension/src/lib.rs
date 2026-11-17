//! Extension foundation crate for Iceon.
//!
//! Provides native implementations of Freelens extensions as built-in,
//! modular features that can be toggled via preferences.

pub mod builtin;
pub mod model;
pub mod registry;

pub use builtin::ALL_BUILTIN_EXTENSIONS;
pub use model::{
    CrdDefinition, ExtensionCategory, ExtensionId, ExtensionManifest, InspectorContribution,
    SidebarContribution,
};
pub use registry::ExtensionRegistry;
