//! Read-only, ephemeral setup hints. This module never resolves authentication,
//! constructs a client, searches PATH, or runs a credential helper.

use super::{ConnectionEngine, ConnectionError, Result, config, map_catalog_error, validate_id};
use serde::Serialize;

/// Display-only names from the catalog-selected context (not `current-context`).
/// Unsafe names are omitted rather than escaped or partially echoed. No kubeconfig
/// paths, helper arguments/environment, user names, or credential values are retained.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSetupDiagnostic {
    pub context_name: Option<String>,
    pub executable_basename: Option<String>,
}

impl ConnectionEngine {
    /// Inspect local setup without opening/replacing a workspace lease or contacting
    /// a server. Uses the normal bounded kubeconfig preparation and transport/source
    /// validations; failures are typed categories, never raw filesystem/auth errors.
    pub async fn connection_setup_diagnostic(
        &self,
        cluster_id: String,
    ) -> Result<ConnectionSetupDiagnostic> {
        validate_id(&cluster_id)?;
        let permit = self
            .file_workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| ConnectionError::Busy)?;
        let catalog = self.catalog.clone();
        crate::runtime::spawn_blocking(move || {
            // Keep admission until the blocking read finishes, even if its caller
            // drops the future. Credentials exist only in this local preparation.
            let _permit = permit;
            let target = catalog
                .connection_target(&cluster_id)
                .map_err(map_catalog_error)?;
            let prepared = config::prepare(&target)?;
            let command = prepared
                .auth_infos
                .first()
                .and_then(|user| user.auth_info.as_ref())
                .and_then(|auth| auth.exec.as_ref())
                .and_then(|exec| exec.command.as_deref());
            Ok(ConnectionSetupDiagnostic {
                context_name: display_name(&target.context_name, true),
                executable_basename: command.and_then(executable_basename),
            })
        })
        .await
        .map_err(|_| ConnectionError::KubeconfigInvalid)?
    }
}

fn display_name(value: &str, context: bool) -> Option<String> {
    // An ASCII allowlist also excludes ANSI escapes, bidi/zero-width characters,
    // shell syntax, credential assignments, whitespace and path separators.
    if value.is_empty()
        || value.len() > 256
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value.bytes().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, b'.' | b'_' | b'-')
                || (context && matches!(c, b':' | b'@'))
        })
    {
        None
    } else {
        Some(value.to_owned())
    }
}

fn executable_basename(command: &str) -> Option<String> {
    if command.len() > 4096 || command.chars().any(|c| c.is_control() || !c.is_ascii()) {
        return None;
    }
    // Recognize both separator styles independent of the host OS. Do not use
    // Path::file_name: it can normalize a trailing separator into a plausible name.
    display_name(command.rsplit(['/', '\\']).next()?, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::tests::{CONFIG, fixture};
    use std::fs;

    #[test]
    fn names_are_bounded_display_only_and_never_paths_or_shell_text() {
        for command in [
            "kubelogin",
            "/opt/private/bin/kubelogin",
            "C:\\private\\kubelogin",
        ] {
            assert_eq!(executable_basename(command).as_deref(), Some("kubelogin"));
        }
        for command in [
            "",
            "/private/bin/",
            "C:\\private\\",
            ".",
            "..",
            "--token",
            "helper --token=secret",
            "token=secret",
            "helper\n",
            "helper\u{202e}",
            "helper\u{200b}",
            "/private\n/bin/helper",
        ] {
            assert_eq!(executable_basename(command), None, "{command:?}");
        }
        assert_eq!(executable_basename(&"a".repeat(257)), None);
        assert_eq!(
            executable_basename(&format!("/{}/helper", "a".repeat(4096))),
            None
        );
        for context in [
            "/private/config",
            "token=secret",
            "chosen\u{202e}",
            "chosen\n",
        ] {
            assert_eq!(display_name(context, true), None);
        }
        assert_eq!(
            display_name("dev-team:cluster@region", true).as_deref(),
            Some("dev-team:cluster@region")
        );
    }

    #[tokio::test]
    async fn selected_context_only_and_credentials_never_leave_preparation() {
        let (_root, _catalog, engine, id, target) = fixture();
        let config = format!("{CONFIG}\ncurrent-context: other\n");
        let config = config.replace(
            "contexts:\n",
            "contexts:\n- name: other\n  context:\n    cluster: target\n    user: other-user\n",
        );
        let config = config.replace(
            "users:\n",
            "users:\n- name: other-user\n  user:\n    exec:\n      command: other-helper\n",
        );
        fs::write(&target.source_path, config).unwrap();
        let before = fs::read(&target.source_path).unwrap();
        let diagnostic = engine.connection_setup_diagnostic(id).await.unwrap();
        assert_eq!(diagnostic.context_name.as_deref(), Some("chosen"));
        assert_eq!(diagnostic.executable_basename, None);
        let json = serde_json::to_value(&diagnostic).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"contextName": "chosen", "executableBasename": null})
        );
        assert!(!format!("{diagnostic:?}").contains("PRIVATE_FIXTURE_TOKEN"));
        assert_eq!(fs::read(&target.source_path).unwrap(), before);
        assert!(engine.session.lock().unwrap().is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn helper_is_not_executed_and_only_basename_is_exposed() {
        use std::os::unix::fs::PermissionsExt;
        let (root, _catalog, engine, id, target) = fixture();
        let helper = root.path().join("credential-helper");
        let marker = root.path().join("must-not-exist");
        fs::write(
            &helper,
            format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&target.source_path, format!("{CONFIG}    exec:\n      command: '{}'\n      args: ['PRIVATE_ARGUMENT']\n      env:\n      - name: PRIVATE_ENV\n        value: PRIVATE_VALUE\n", helper.display())).unwrap();
        let diagnostic = engine.connection_setup_diagnostic(id).await.unwrap();
        assert_eq!(
            serde_json::to_value(&diagnostic).unwrap(),
            serde_json::json!({
                "contextName": "chosen", "executableBasename": "credential-helper"
            })
        );
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn admission_input_and_existing_security_checks_are_preserved() {
        let (_root, _catalog, engine, id, target) = fixture();
        assert_eq!(
            engine.connection_setup_diagnostic("a".repeat(257)).await,
            Err(ConnectionError::InvalidInput)
        );
        let permits = engine
            .file_workers
            .clone()
            .try_acquire_many_owned(2)
            .unwrap();
        assert_eq!(
            engine.connection_setup_diagnostic(id.clone()).await,
            Err(ConnectionError::Busy)
        );
        drop(permits);
        fs::write(&target.source_path, CONFIG.replace("https://", "http://")).unwrap();
        assert_eq!(
            engine.connection_setup_diagnostic(id.clone()).await,
            Err(ConnectionError::InsecureTransport)
        );
        fs::write(
            &target.source_path,
            CONFIG.replace("- name: chosen", "- name: renamed"),
        )
        .unwrap();
        assert_eq!(
            engine.connection_setup_diagnostic(id).await,
            Err(ConnectionError::SourceChanged)
        );
    }
}
