//! Bounded, secret-safe helpers for the native Add Config workflow.
//! YAML never enters catalog storage; only an explicitly selected file is written.

use super::{CatalogError, MAX_INPUT, parse_metadata, valid_source_path};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// Classify pasted documents before a single-line input can flatten their newlines.
pub fn looks_like_yaml(text: &str) -> bool {
    let text = text.trim_start_matches('\u{feff}').trim();
    text.contains('\n')
        || text.starts_with('{')
        || text.starts_with("apiVersion:")
        || text.starts_with("kind:")
        || text.starts_with("---")
}

/// Expand only the unambiguous home-directory shorthand, not shell expressions.
pub fn input_path(text: &str, home: Option<&Path>) -> Result<PathBuf, CatalogError> {
    let text = text.trim();
    let path = if let Some(suffix) = text.strip_prefix("~/") {
        home.ok_or(CatalogError::InvalidInput)?.join(suffix)
    } else {
        PathBuf::from(text)
    };
    if !path.to_str().is_some_and(valid_source_path) {
        return Err(CatalogError::InvalidInput);
    }
    Ok(path)
}

/// Parse both the catalog metadata and the actual kube client schema, without
/// resolving credentials, executing auth plugins, or reporting YAML contents.
pub fn validate_yaml(yaml: &str) -> Result<(), CatalogError> {
    if yaml.len() > MAX_INPUT {
        return Err(CatalogError::LimitExceeded);
    }
    if parse_metadata(yaml.as_bytes())?.is_empty() {
        return Err(CatalogError::InvalidKubeconfig);
    }
    kube::config::Kubeconfig::from_yaml(yaml).map_err(|_| CatalogError::InvalidKubeconfig)?;
    Ok(())
}

/// Atomically publish a validated file in its destination directory. Existing
/// destinations (including symlinks) are NEVER replaced, even after a native
/// overwrite confirmation. The user must choose a new name instead.
pub fn save_yaml_new(path: &Path, yaml: &str) -> Result<(), CatalogError> {
    validate_yaml(yaml)?;
    if !path.to_str().is_some_and(valid_source_path) {
        return Err(CatalogError::InvalidInput);
    }
    let parent = path.parent().ok_or(CatalogError::InvalidInput)?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| CatalogError::InputUnavailable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| CatalogError::InputUnavailable)?;
    }
    file.write_all(yaml.as_bytes())
        .map_err(|_| CatalogError::InputUnavailable)?;
    file.as_file()
        .sync_all()
        .map_err(|_| CatalogError::InputUnavailable)?;
    file.persist_noclobber(path)
        .map_err(|_| CatalogError::InputUnavailable)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const YAML: &str = "apiVersion: v1\nkind: Config\nclusters:\n- name: local\n  cluster:\n    server: https://localhost:6443\ncontexts:\n- name: local\n  context:\n    cluster: local\nusers: []\n";

    #[test]
    fn detects_documents_not_paths() {
        assert!(looks_like_yaml(YAML));
        assert!(looks_like_yaml("{\"kind\": \"Config\"}"));
        assert!(!looks_like_yaml("~/.kube/config"));
        assert!(!looks_like_yaml("/home/user/config.yaml"));
        assert_eq!(
            input_path("~/config", Some(Path::new("/home/user"))).unwrap(),
            Path::new("/home/user/config")
        );
        assert!(input_path("relative", None).is_err());
        assert!(input_path("/tmp/a\0b", None).is_err());
    }

    #[test]
    fn validates_and_caps_documents() {
        assert_eq!(validate_yaml(YAML), Ok(()));
        assert!(validate_yaml("token: private-value").is_err());
        assert!(validate_yaml(&YAML.replace("cluster: local", "cluster: missing")).is_err());
        assert!(validate_yaml("apiVersion: v1\nkind: Config\nclusters: []\ncontexts: []").is_err());
        assert_eq!(
            validate_yaml(&"x".repeat(MAX_INPUT + 1)),
            Err(CatalogError::LimitExceeded)
        );
    }

    #[test]
    fn private_atomic_save_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        save_yaml_new(&path, YAML).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), YAML);
        assert!(save_yaml_new(&path, YAML).is_err());
        assert!(save_yaml_new(&dir.path().join("invalid"), "secret: x").is_err());
        assert!(!dir.path().join("invalid").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
