//! OS side effects for App preferences. No system paths are touched by tests.
use crate::preferences::AppPreferences;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const LABEL: &str = "com.iceon.app.login";
const OWNER: &str = "<!-- Managed by Iceon: com.iceon.app.login -->";

#[derive(Debug, thiserror::Error)]
pub enum IntegrationError {
    #[error("{0} is not supported on this platform/build")]
    Unsupported(&'static str),
    #[error("tray icon: {0}")]
    Tray(String),
    #[error("login registration contains an invalid path")]
    InvalidPath,
    #[error("refusing to replace a login registration not owned by Iceon")]
    NotOwned,
    #[error("OS preferences: {0}")]
    Io(#[from] std::io::Error),
    #[error("{operation}; rollback also failed: {rollback}")]
    Rollback {
        operation: Box<IntegrationError>,
        rollback: Box<IntegrationError>,
    },
}

/// Implementations must restore their previous state if applying a change fails.
/// Tray ownership lives on the UI thread; an unsupported tray must return an error,
/// not silently persist `true`.
pub trait AppIntegration {
    fn apply(
        &mut self,
        previous: &AppPreferences,
        next: &AppPreferences,
    ) -> Result<(), IntegrationError>;
}

/// LaunchAgents are loaded by launchd at the next graphical login. Deliberately
/// do not bootstrap/RunAtLoad this registration into the current session, which
/// would launch a duplicate application while changing the checkbox.
pub struct LoginAgent {
    directory: PathBuf,
    executable: PathBuf,
}
impl LoginAgent {
    /// Explicit paths also allow isolated tests; the executable must be absolute.
    pub fn new(directory: PathBuf, executable: PathBuf) -> Result<Self, IntegrationError> {
        if !directory.is_absolute() || !executable.is_absolute() {
            return Err(IntegrationError::InvalidPath);
        }
        xml_path(&executable)?;
        Ok(Self {
            directory,
            executable,
        })
    }
    #[cfg(target_os = "macos")]
    pub fn current_user() -> Result<Self, IntegrationError> {
        let home = dirs::home_dir().ok_or(IntegrationError::InvalidPath)?;
        Self::new(home.join("Library/LaunchAgents"), std::env::current_exe()?)
    }
    pub fn path(&self) -> PathBuf {
        self.directory.join(format!("{LABEL}.plist"))
    }
    fn contents(&self) -> Result<String, IntegrationError> {
        Ok(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{OWNER}\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{LABEL}</string><key>ProgramArguments</key><array><string>{}</string></array><key>RunAtLoad</key><true/></dict></plist>\n",
            xml_path(&self.executable)?
        ))
    }
    /// Reject symlink ancestors and foreign files; never remove another app's file.
    fn existing(&self) -> Result<Option<Vec<u8>>, IntegrationError> {
        for ancestor in self.directory.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                    return Err(IntegrationError::NotOwned);
                }
                Ok(_) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        match fs::symlink_metadata(self.path()) {
            Ok(m) => {
                if !m.is_file() || m.file_type().is_symlink() || m.len() > 65536 {
                    return Err(IntegrationError::NotOwned);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    // SAFETY: geteuid takes no arguments and has no memory effects.
                    if m.uid() != unsafe { libc::geteuid() } {
                        return Err(IntegrationError::NotOwned);
                    }
                }
                let bytes = fs::read(self.path())?;
                if !bytes.windows(OWNER.len()).any(|w| w == OWNER.as_bytes()) {
                    return Err(IntegrationError::NotOwned);
                }
                Ok(Some(bytes))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    pub fn set_enabled(&self, enabled: bool) -> Result<(), IntegrationError> {
        let previous = self.existing()?;
        if !enabled {
            if previous.is_some() {
                fs::remove_file(self.path())?;
            }
            return Ok(());
        }
        let contents = self.contents()?;
        if previous.as_deref() == Some(contents.as_bytes()) {
            return Ok(());
        }
        fs::create_dir_all(&self.directory)?;
        // Same-directory NamedTempFile + rename gives readers a complete plist,
        // with owner-only permissions; no predictable temporary file names.
        let mut tmp = tempfile::NamedTempFile::new_in(&self.directory)?;
        tmp.write_all(contents.as_bytes())?;
        tmp.as_file().sync_all()?;
        self.existing()?;
        tmp.persist(self.path()).map_err(|e| e.error)?;
        Ok(())
    }
}
fn xml_path(path: &Path) -> Result<String, IntegrationError> {
    let value = path.to_str().ok_or(IntegrationError::InvalidPath)?;
    if value.chars().any(|c| c.is_control()) {
        return Err(IntegrationError::InvalidPath);
    }
    Ok(value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}

/// Default backend with the existing dependency set. GPUI Kit 0.9 / GPUI Pre
/// 0.3.11 expose no tray API. Reject tray enable until a real adapter is supplied.
#[derive(Default)]
pub struct NativeAppIntegration;
impl AppIntegration for NativeAppIntegration {
    fn apply(
        &mut self,
        previous: &AppPreferences,
        next: &AppPreferences,
    ) -> Result<(), IntegrationError> {
        if next.show_tray_icon {
            return Err(IntegrationError::Unsupported("tray icon"));
        }
        #[cfg(target_os = "macos")]
        {
            let _ = previous;
            LoginAgent::current_user()?.set_enabled(next.open_at_login)?;
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = previous;
            if next.open_at_login {
                return Err(IntegrationError::Unsupported("open at login"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_escapes_paths_and_removes_only_owned_files() {
        let dir = tempfile::tempdir().unwrap();
        let agent = LoginAgent::new(
            dir.path().canonicalize().unwrap().join("LaunchAgents"),
            PathBuf::from("/Applications/Ice & <test> \"'.app/Contents/MacOS/iceon"),
        )
        .unwrap();
        agent.set_enabled(true).unwrap();
        let contents = fs::read_to_string(agent.path()).unwrap();
        assert!(contents.contains("Ice &amp; &lt;test&gt; &quot;&apos;.app"));
        agent.set_enabled(true).unwrap();
        agent.set_enabled(false).unwrap();
        assert!(!agent.path().exists());
        fs::write(agent.path(), "foreign").unwrap();
        assert!(matches!(
            agent.set_enabled(false),
            Err(IntegrationError::NotOwned)
        ));
        assert!(matches!(
            agent.set_enabled(true),
            Err(IntegrationError::NotOwned)
        ));
        assert_eq!(fs::read_to_string(agent.path()).unwrap(), "foreign");
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_and_has_private_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let agent =
            LoginAgent::new(dir.path().canonicalize().unwrap(), "/bin/true".into()).unwrap();
        agent.set_enabled(true).unwrap();
        assert_eq!(
            fs::metadata(agent.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        agent.set_enabled(false).unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, "untouched").unwrap();
        symlink(&victim, agent.path()).unwrap();
        assert!(agent.set_enabled(true).is_err());
        assert_eq!(fs::read_to_string(victim).unwrap(), "untouched");
    }
}
