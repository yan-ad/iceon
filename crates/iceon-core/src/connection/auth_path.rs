use super::{ConnectionError, Result};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_DIRECTORIES: usize = 128;
const FILE_LIMIT: usize = 8 * 1024;
const READ_LIMIT: usize = 32 * 1024;

#[path = "auth_fish.rs"]
mod fish;

pub(super) fn bare(command: &Path) -> Option<&str> {
    let name = command.to_str()?;
    (!name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != "..").then_some(name)
}

pub(super) fn resolve(command: &Path, env: &[(String, String)]) -> Option<PathBuf> {
    let name = bare(command)?;
    let path = &env.iter().find(|(key, _)| key == "PATH")?.1;
    if path.len() > MAX_PATH_BYTES {
        return None;
    }
    for directory in std::env::split_paths(path).take(MAX_DIRECTORIES) {
        if !directory.is_absolute() {
            continue;
        }
        let Ok(candidate) = std::fs::canonicalize(directory.join(name)) else {
            continue;
        };
        let Ok(metadata) = candidate.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        return Some(candidate);
    }
    None
}

// This is a literal SDK-source recognizer, not a shell or PATH interpreter.
fn sdk_source(line: &str, home: &Path) -> Option<PathBuf> {
    let line = line.trim();
    let source = if let Some(rest) = line.strip_prefix("if ") {
        rest.split_once("; then ")?.1.strip_suffix("; fi")?.trim()
    } else {
        line
    };
    let rest = source
        .strip_prefix("source ")
        .or_else(|| source.strip_prefix(". "))?
        .trim();
    let literal = if rest.starts_with(['\'', '"']) {
        let quote = rest.chars().next()?;
        rest.strip_prefix(quote)?.strip_suffix(quote)?
    } else {
        if rest.chars().any(char::is_whitespace) {
            return None;
        }
        rest
    };
    let expanded = if let Some(rest) = literal
        .strip_prefix("$HOME/")
        .or_else(|| literal.strip_prefix("${HOME}/"))
        .or_else(|| literal.strip_prefix("~/"))
    {
        home.join(rest)
    } else {
        PathBuf::from(literal)
    };
    let text = expanded.to_str()?;
    if !expanded.is_absolute()
        || text.contains(['$', '`', ';', '\n', '\r', '\\', '|', '&', '<', '>', '\0'])
        || expanded
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return None;
    }
    if !text.ends_with("/google-cloud-sdk/path.zsh.inc")
        && !text.ends_with("/google-cloud-sdk/path.bash.inc")
    {
        return None;
    }
    Some(expanded.parent()?.join("bin"))
}

fn read_hint(path: &Path, budget: &mut usize) -> Option<String> {
    use std::io::Read;
    if *budget == 0 {
        return None;
    }
    // In particular, never open a FIFO/device or follow a startup-file symlink.
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > FILE_LIMIT as u64 || metadata.len() > *budget as u64
    {
        return None;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    let opened = file.metadata().ok()?;
    if !opened.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
            return None;
        }
    }
    let mut bytes = Vec::new();
    let allowed = FILE_LIMIT.min(*budget);
    file.take((allowed + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    *budget = budget.saturating_sub(bytes.len());
    if bytes.len() > allowed || bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

pub(super) fn fallback(
    command: &mut PathBuf,
    env: &mut Vec<(String, String)>,
    home: &Path,
    system: &Path,
    config_home: Option<&Path>,
    stopped: impl Fn() -> bool,
) -> Result<()> {
    let Some(name) = bare(command) else {
        return Ok(());
    };
    if stopped() {
        return Ok(());
    }
    let mut directories = Vec::new();
    let Some((_, current)) = env.iter().find(|(key, _)| key == "PATH") else {
        return Ok(());
    };
    if current.len() > MAX_PATH_BYTES {
        return Err(ConnectionError::LimitExceeded);
    }
    for directory in std::env::split_paths(current) {
        add(&mut directories, directory)?;
    }
    let mut budget = READ_LIMIT;
    // macOS /etc itself may be the system-provided /private/etc symlink. Entries are not followed.
    if let Some(contents) = read_hint(&system.join("paths"), &mut budget) {
        for line in contents.lines() {
            add(&mut directories, PathBuf::from(line.trim()))?;
        }
    }
    let snippets = system.join("paths.d");
    if std::fs::symlink_metadata(&snippets).is_ok_and(|metadata| metadata.is_dir())
        && let Ok(entries) = std::fs::read_dir(&snippets)
    {
        let mut paths = Vec::new();
        for entry in entries.take(33) {
            if stopped() {
                return Ok(());
            }
            if let Ok(entry) = entry {
                paths.push(entry.path());
            }
        }
        if paths.len() <= 32 {
            paths.sort();
            for path in paths {
                if stopped() {
                    return Ok(());
                }
                if let Some(contents) = read_hint(&path, &mut budget) {
                    for line in contents.lines() {
                        add(&mut directories, PathBuf::from(line.trim()))?;
                    }
                }
            }
        }
    }
    if matches!(name, "gke-gcloud-auth-plugin" | "gcloud") && home.is_absolute() {
        fish::discover(home, config_home, &mut directories, &mut budget, &stopped)?;
        for filename in [
            ".zshrc",
            ".zprofile",
            ".bashrc",
            ".bash_profile",
            ".profile",
        ] {
            if stopped() {
                return Ok(());
            }
            if let Some(contents) = read_hint(&home.join(filename), &mut budget) {
                for line in contents.lines() {
                    if let Some(directory) = sdk_source(line, home) {
                        add(&mut directories, directory)?;
                    }
                }
            }
        }
    }
    if stopped() {
        return Ok(());
    }
    let path = std::env::join_paths(directories)
        .map_err(|_| ConnectionError::KubeconfigInvalid)?
        .into_string()
        .map_err(|_| ConnectionError::KubeconfigInvalid)?;
    if path.len() > MAX_PATH_BYTES {
        return Err(ConnectionError::LimitExceeded);
    }
    let mut proposed = env.clone();
    proposed
        .iter_mut()
        .find(|(key, _)| key == "PATH")
        .unwrap()
        .1 = path;
    if let Some(resolved) = resolve(command, &proposed) {
        *command = resolved;
        *env = proposed;
    }
    Ok(())
}

/// Finder does not inherit the login shell's PATH. Extend only this helper's
/// environment; never execute shell startup files or change the app environment.
pub(super) fn configure(
    env: &mut Vec<(String, String)>,
    inherited: Option<&OsStr>,
    home: Option<&Path>,
) -> Result<()> {
    if env.iter().any(|(name, _)| name == "PATH") {
        return Ok(());
    }
    let mut directories = Vec::new();
    if let Some(path) = inherited {
        if path.len() > MAX_PATH_BYTES {
            return Err(ConnectionError::LimitExceeded);
        }
        for directory in std::env::split_paths(path) {
            add(&mut directories, directory)?;
        }
    }
    for directory in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/opt/homebrew/share/google-cloud-sdk/bin",
        "/opt/homebrew/Caskroom/google-cloud-sdk/latest/google-cloud-sdk/bin",
        "/usr/local/share/google-cloud-sdk/bin",
        "/usr/local/Caskroom/google-cloud-sdk/latest/google-cloud-sdk/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        add(&mut directories, PathBuf::from(directory))?;
    }
    if let Some(home) = home.filter(|path| path.is_absolute()) {
        for directory in ["google-cloud-sdk/bin", ".local/share/google-cloud-sdk/bin"] {
            add(&mut directories, home.join(directory))?;
        }
    }
    let path = std::env::join_paths(directories).map_err(|_| ConnectionError::KubeconfigInvalid)?;
    let path = path
        .into_string()
        .map_err(|_| ConnectionError::KubeconfigInvalid)?;
    if path.len() > MAX_PATH_BYTES {
        return Err(ConnectionError::LimitExceeded);
    }
    env.push(("PATH".into(), path));
    Ok(())
}

fn add(directories: &mut Vec<PathBuf>, directory: PathBuf) -> Result<()> {
    // Empty/relative entries could resolve a helper from an arbitrary working directory.
    if !directory.is_absolute() || directories.contains(&directory) {
        return Ok(());
    }
    if directories.len() >= MAX_DIRECTORIES {
        return Err(ConnectionError::LimitExceeded);
    }
    directories.push(directory);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_literal_sdk_sources_including_google_installer_guards() {
        let home = Path::new("/Users/fixture");
        for line in [
            "source /Users/fixture/Kerjaa/google-cloud-sdk/path.zsh.inc",
            ". \"${HOME}/SDK ü space/google-cloud-sdk/path.bash.inc\"",
            "if [ -f '/Users/fixture/Kerjaa/google-cloud-sdk/path.zsh.inc' ]; then . '/Users/fixture/Kerjaa/google-cloud-sdk/path.zsh.inc'; fi",
            "source \"$HOME/SDK ü space/google-cloud-sdk/path.zsh.inc\"",
            "source ~/google-cloud-sdk/path.bash.inc",
        ] {
            assert!(sdk_source(line, home).is_some(), "{line}");
        }
        for line in [
            "source \"$(/must/notexecute)/google-cloud-sdk/path.zsh.inc\"",
            "source /must/`notexecute`/google-cloud-sdk/path.zsh.inc",
            "source /secret/google-cloud-sdk/path.zsh.inc; /must/notexecute",
            "export PATH=/secret/google-cloud-sdk/bin:$PATH",
            "source relative/google-cloud-sdk/path.zsh.inc",
            "source /secret/other/path.zsh.inc",
            "source /secret/../google-cloud-sdk/path.zsh.inc",
            "# source /secret/google-cloud-sdk/path.zsh.inc",
        ] {
            assert!(sdk_source(line, home).is_none(), "{line}");
        }
    }

    #[cfg(unix)]
    fn executable(path: &Path) {
        use std::{fs, os::unix::fs::PermissionsExt};
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resolves_first_regular_executable_and_canonicalizes_symlinks() {
        use std::{
            fs,
            os::unix::fs::{PermissionsExt, symlink},
        };
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        executable(&first.join("fixture-helper"));
        executable(&second.join("fixture-helper"));
        let env = vec![(
            "PATH".into(),
            std::env::join_paths([&first, &second])
                .unwrap()
                .into_string()
                .unwrap(),
        )];
        assert_eq!(
            resolve(Path::new("fixture-helper"), &env),
            Some(fs::canonicalize(first.join("fixture-helper")).unwrap())
        );
        fs::set_permissions(
            first.join("fixture-helper"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert_eq!(
            resolve(Path::new("fixture-helper"), &env),
            Some(fs::canonicalize(second.join("fixture-helper")).unwrap())
        );
        fs::remove_file(first.join("fixture-helper")).unwrap();
        symlink(second.join("fixture-helper"), first.join("fixture-helper")).unwrap();
        assert_eq!(
            resolve(Path::new("fixture-helper"), &env),
            Some(fs::canonicalize(second.join("fixture-helper")).unwrap())
        );
        assert!(resolve(&root.path().join("absent"), &env).is_none());
        assert!(resolve(Path::new("relative/fixture-helper"), &env).is_none());
        fs::remove_file(first.join("fixture-helper")).unwrap();
        fs::create_dir(first.join("fixture-helper")).unwrap();
        assert_eq!(
            resolve(Path::new("fixture-helper"), &env),
            Some(fs::canonicalize(second.join("fixture-helper")).unwrap())
        );
    }

    #[cfg(unix)]
    #[test]
    fn fallback_is_fixture_scoped_provider_gated_bounded_and_atomic() {
        use std::fs;
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let system = root.path().join("etc");
        fs::create_dir_all(&home).unwrap();
        let sdk = home.join("Kerjaa ü space/google-cloud-sdk");
        executable(&sdk.join("bin/gke-gcloud-auth-plugin"));
        fs::write(
            home.join(".zshrc"),
            format!(
                "if [ -f '{0}/path.zsh.inc' ]; then . '{0}/path.zsh.inc'; fi",
                sdk.display()
            ),
        )
        .unwrap();
        let initial = vec![(
            "PATH".into(),
            root.path().join("empty").to_str().unwrap().into(),
        )];
        let mut env = initial.clone();
        let mut command = PathBuf::from("gke-gcloud-auth-plugin");
        fallback(&mut command, &mut env, &home, &system, None, || false).unwrap();
        assert_eq!(
            command,
            fs::canonicalize(sdk.join("bin/gke-gcloud-auth-plugin")).unwrap()
        );
        assert!(path(&env).contains(&sdk.join("bin")));
        for name in ["synthetic-helper", "/absent/gke-gcloud-auth-plugin"] {
            let mut command = PathBuf::from(name);
            let mut env = initial.clone();
            fallback(&mut command, &mut env, &home, &system, None, || false).unwrap();
            assert_eq!(command, PathBuf::from(name));
            assert_eq!(env, initial);
        }
        fs::write(home.join(".zshrc"), "x".repeat(FILE_LIMIT + 1)).unwrap();
        let mut command = PathBuf::from("gke-gcloud-auth-plugin");
        let mut env = initial.clone();
        fallback(&mut command, &mut env, &home, &system, None, || false).unwrap();
        assert_eq!(env, initial);
        assert_eq!(command, PathBuf::from("gke-gcloud-auth-plugin"));
        fs::create_dir_all(system.join("paths.d")).unwrap();
        fs::write(
            system.join("paths"),
            format!("relative\n{}\n", sdk.join("bin").display()),
        )
        .unwrap();
        fallback(&mut command, &mut env, &home, &system, None, || true).unwrap();
        assert_eq!(env, initial);
        fallback(&mut command, &mut env, &home, &system, None, || false).unwrap();
        assert!(command.is_absolute());
    }

    #[cfg(unix)]
    #[test]
    fn finder_discovers_custom_sdk_from_fish_without_running_startup_commands() {
        use std::fs;
        for (filename, contents) in [
            (
                "config.fish",
                "if [ -f '~/Kerjaa/google-cloud-sdk/path.fish.inc' ]; . '~/Kerjaa/google-cloud-sdk/path.fish.inc'; end",
            ),
            (
                "config.fish",
                "set -gx PATH \"$HOME/Kerjaa ü space/google-cloud-sdk/bin\" $PATH",
            ),
            (
                "config.fish",
                "set -gx PATH \"$HOME//Kerjaa ü space/google-cloud-sdk/bin\" $PATH",
            ),
            (
                "config.fish",
                "fish_add_path ~//Kerjaa/google-cloud-sdk/bin",
            ),
            (
                "conf.d/gcloud.fish",
                "source \"$HOME/Kerjaa ü space/google-cloud-sdk/path.fish.inc\"",
            ),
            (
                "config.fish",
                "fish_add_path --prepend \"$HOME/Kerjaa ü space/google-cloud-sdk/bin\"",
            ),
            (
                "config.fish",
                "set -U fish_user_paths \"$HOME/Kerjaa ü space/google-cloud-sdk/bin\" $fish_user_paths",
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let home = root.path().join("home");
            let config = home.join(".config/fish").join(filename);
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            let sdk = if contents.contains("~//") || contents.contains("~/Kerjaa/") {
                "Kerjaa"
            } else {
                "Kerjaa ü space"
            };
            let helper = home
                .join(sdk)
                .join("google-cloud-sdk/bin/gke-gcloud-auth-plugin");
            executable(&helper);
            let marker = root.path().join("must-not-exist");
            fs::write(
                &config,
                format!("touch '{}'\n{contents}\n", marker.display()),
            )
            .unwrap();
            let initial = vec![(
                "PATH".into(),
                root.path().join("empty").to_str().unwrap().into(),
            )];
            let mut env = initial.clone();
            let mut command = PathBuf::from("gke-gcloud-auth-plugin");
            fallback(
                &mut command,
                &mut env,
                &home,
                &root.path().join("etc"),
                None,
                || false,
            )
            .unwrap();
            assert_eq!(
                command,
                fs::canonicalize(&helper).unwrap(),
                "{filename}: {contents}"
            );
            assert!(!marker.exists());
            assert!(path(&env).contains(&helper.parent().unwrap().to_owned()));
            assert_eq!(
                fs::read_to_string(config).unwrap(),
                format!("touch '{}'\n{contents}\n", marker.display())
            );
            for name in ["synthetic-helper", "/absent/gke-gcloud-auth-plugin"] {
                let mut command = PathBuf::from(name);
                let mut env = initial.clone();
                fallback(
                    &mut command,
                    &mut env,
                    &home,
                    &root.path().join("etc"),
                    None,
                    || false,
                )
                .unwrap();
                assert_eq!(command, PathBuf::from(name));
                assert_eq!(env, initial);
            }
            fs::remove_file(helper).unwrap();
            let mut command = PathBuf::from("gke-gcloud-auth-plugin");
            let mut env = initial.clone();
            fallback(
                &mut command,
                &mut env,
                &home,
                &root.path().join("etc"),
                None,
                || false,
            )
            .unwrap();
            assert_eq!(command, PathBuf::from("gke-gcloud-auth-plugin"));
            assert_eq!(env, initial);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_path_keeps_relative_entry_precedence_over_absolute_entries() {
        use super::super::{V1, helper_request, run};
        use kube::config::{Cluster, ExecConfig, ExecInteractiveMode};
        use std::{fs, time::Duration};
        let cwd = std::env::current_dir().unwrap();
        let root = tempfile::tempdir_in(&cwd).unwrap();
        let relative = root.path().strip_prefix(&cwd).unwrap().join("relative");
        let absolute = root.path().join("absolute");
        for (directory, output) in [
            (cwd.join(&relative), "relative"),
            (absolute.clone(), "absolute"),
        ] {
            let helper = directory.join("iceon-fixture-auth-helper");
            executable(&helper);
            fs::write(helper, format!("#!/bin/sh\nprintf {output}\n")).unwrap();
        }
        let path = std::env::join_paths([&relative, &absolute])
            .unwrap()
            .into_string()
            .unwrap();
        let exec = ExecConfig {
            command: Some("iceon-fixture-auth-helper".into()),
            api_version: Some(V1.into()),
            interactive_mode: Some(ExecInteractiveMode::Never),
            env: Some(vec![
                [
                    ("name".into(), "PATH".into()),
                    ("value".into(), path.clone()),
                ]
                .into(),
            ]),
            ..Default::default()
        };
        let request = helper_request(&exec, &Cluster::default(), root.path()).unwrap();
        assert!(request.fallback_home.is_none());
        assert_eq!(request.env[0].1, path);
        assert_eq!(
            run(request, Duration::from_secs(2)).await.unwrap(),
            b"relative"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fish_discovered_sdk_runs_fixture_sibling_with_literal_arguments() {
        use super::super::{HelperRequest, run};
        use std::{fs, time::Duration};
        let root = tempfile::tempdir().unwrap();
        let sdk = root.path().join("Kerjaa/google-cloud-sdk");
        let helper = sdk.join("bin/gke-gcloud-auth-plugin");
        let sibling = sdk.join("bin/iceon-fixture-cli");
        executable(&helper);
        executable(&sibling);
        fs::write(&helper, "#!/bin/sh\nexec iceon-fixture-cli \"$@\"\n").unwrap();
        fs::write(
            &sibling,
            "#!/bin/sh\n[ \"$1\" = 'literal ; ü' ] || exit 9\nprintf fixture-only\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join(".config/fish")).unwrap();
        fs::write(
            root.path().join(".config/fish/config.fish"),
            "if [ -f '~/Kerjaa/google-cloud-sdk/path.fish.inc' ]; . '~/Kerjaa/google-cloud-sdk/path.fish.inc'; end",
        )
        .unwrap();
        let mut env = vec![(
            "PATH".into(),
            root.path().join("empty").to_str().unwrap().into(),
        )];
        let mut command = PathBuf::from("gke-gcloud-auth-plugin");
        fallback(
            &mut command,
            &mut env,
            root.path(),
            &root.path().join("etc"),
            None,
            || false,
        )
        .unwrap();
        assert!(command.is_absolute());
        let output = run(
            HelperRequest {
                command,
                args: vec!["literal ; ü".into()],
                env,
                info: "{}".into(),
                fallback_home: None,
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(output, b"fixture-only");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_path_never_scans_hints_or_rewrites_an_absolute_command() {
        use super::super::{V1, helper_request, run};
        use kube::config::{Cluster, ExecConfig, ExecInteractiveMode};
        use std::time::Duration;
        let root = tempfile::tempdir().unwrap();
        for command in [
            "iceon-fixture-definitely-absent".to_owned(),
            root.path().join("absent").to_str().unwrap().to_owned(),
        ] {
            let exec = ExecConfig {
                command: Some(command.clone()),
                api_version: Some(V1.into()),
                interactive_mode: Some(ExecInteractiveMode::Never),
                env: Some(vec![
                    [
                        ("name".into(), "PATH".into()),
                        ("value".into(), root.path().to_str().unwrap().into()),
                    ]
                    .into(),
                ]),
                ..Default::default()
            };
            let request = helper_request(&exec, &Cluster::default(), root.path()).unwrap();
            assert_eq!(request.command, PathBuf::from(command));
            assert!(request.fallback_home.is_none());
            assert_eq!(request.env[0].1, root.path().to_str().unwrap());
            assert_eq!(
                run(request, Duration::from_secs(2)).await,
                Err(ConnectionError::AuthHelperMissing)
            );
        }
    }

    #[test]
    fn bounded_reader_rejects_oversize_symlinks_and_total_overflow() {
        use std::fs;
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("hint");
        fs::write(&file, "fixture").unwrap();
        let mut budget = 6;
        assert!(read_hint(&file, &mut budget).is_none());
        assert_eq!(budget, 6);
        let mut budget = READ_LIMIT;
        fs::write(&file, "x".repeat(FILE_LIMIT + 1)).unwrap();
        assert!(read_hint(&file, &mut budget).is_none());
        assert_eq!(budget, READ_LIMIT);
        fs::write(&file, "fixture").unwrap();
        #[cfg(unix)]
        {
            let link = root.path().join("link");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert!(read_hint(&link, &mut budget).is_none());
        }
        assert_eq!(read_hint(&file, &mut budget).as_deref(), Some("fixture"));
        assert_eq!(budget, READ_LIMIT - 7);
    }

    #[cfg(unix)]
    #[test]
    fn hint_reader_never_opens_a_fifo_or_directory() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("startup-hint");
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // Synthetic FIFO only; opening it normally would block without a writer.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let mut budget = READ_LIMIT;
        assert!(read_hint(&fifo, &mut budget).is_none());
        assert!(read_hint(root.path(), &mut budget).is_none());
        assert_eq!(budget, READ_LIMIT);
    }

    fn path(env: &[(String, String)]) -> Vec<PathBuf> {
        std::env::split_paths(&env.iter().find(|(name, _)| name == "PATH").unwrap().1).collect()
    }

    #[test]
    fn finder_path_gets_bounded_standard_locations_in_stable_order() {
        let mut env = vec![("SYNTHETIC_AUTH".into(), "unchanged".into())];
        configure(
            &mut env,
            Some(OsStr::new("/usr/bin:/bin")),
            Some(Path::new("/Users/fixture")),
        )
        .unwrap();
        let dirs = path(&env);
        assert_eq!(&dirs[..2], [Path::new("/usr/bin"), Path::new("/bin")]);
        for directory in [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/opt/homebrew/share/google-cloud-sdk/bin",
            "/usr/local/share/google-cloud-sdk/bin",
            "/Users/fixture/google-cloud-sdk/bin",
            "/Users/fixture/.local/share/google-cloud-sdk/bin",
        ] {
            assert!(dirs.contains(&PathBuf::from(directory)), "{directory}");
        }
        assert_eq!(env[0], ("SYNTHETIC_AUTH".into(), "unchanged".into()));
        let before = env.clone();
        configure(&mut env, None, None).unwrap();
        assert_eq!(env, before);
    }

    #[test]
    fn explicit_kubeconfig_path_is_not_rewritten_even_when_empty() {
        for value in ["", "/explicit/only", "relative::/explicit"] {
            let mut env = vec![("PATH".into(), value.into())];
            let before = env.clone();
            configure(
                &mut env,
                Some(OsStr::new("/usr/bin")),
                Some(Path::new("/fixture")),
            )
            .unwrap();
            assert_eq!(env, before);
        }
    }

    #[test]
    fn inherited_path_has_no_cwd_search_duplicates_or_shell_expansion() {
        let mut env = vec![];
        configure(
            &mut env,
            Some(OsStr::new(
                ":.:relative:~/bin:$HOME/bin:/custom/bin:/custom/bin:/usr/bin:",
            )),
            Some(Path::new("relative-home")),
        )
        .unwrap();
        let dirs = path(&env);
        assert_eq!(dirs[0], Path::new("/custom/bin"));
        assert!(dirs.iter().all(|directory| directory.is_absolute()));
        assert_eq!(
            dirs.iter()
                .filter(|directory| **directory == Path::new("/custom/bin"))
                .count(),
            1
        );
        assert!(!env[0].1.contains("$HOME"));
        assert!(!env[0].1.contains('~'));
    }

    #[test]
    fn oversized_environments_fail_without_mutation() {
        for inherited in [
            "x".repeat(MAX_PATH_BYTES + 1),
            (0..MAX_DIRECTORIES + 1)
                .map(|i| format!("/fixture/{i}"))
                .collect::<Vec<_>>()
                .join(":"),
        ] {
            let mut env = vec![];
            assert_eq!(
                configure(&mut env, Some(OsStr::new(&inherited)), None),
                Err(ConnectionError::LimitExceeded)
            );
            assert!(env.is_empty());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn finder_environment_runs_only_fixture_sdk_helper_and_its_sibling_cli() {
        use super::super::{V1, helper_request, run};
        use kube::config::{Cluster, ExecConfig, ExecInteractiveMode};
        use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

        let home = tempfile::tempdir().unwrap();
        let sdk = home.path().join("google-cloud-sdk/bin");
        fs::create_dir_all(&sdk).unwrap();
        // Distinct fixture names ensure the test cannot execute an installed cloud CLI.
        for (name, body) in [
            (
                "iceon-fixture-auth-helper",
                "exec iceon-fixture-cloud-cli \"$@\"",
            ),
            (
                "iceon-fixture-cloud-cli",
                "[ \"$1\" = '--fixture' ] || exit 7\nprintf '%s' '{\"apiVersion\":\"client.authentication.k8s.io/v1\",\"kind\":\"ExecCredential\",\"status\":{\"token\":\"synthetic-output\"}}'",
            ),
        ] {
            let executable = sdk.join(name);
            fs::write(&executable, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(executable, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut env = vec![];
        configure(
            &mut env,
            Some(OsStr::new("/usr/bin:/bin")),
            Some(home.path()),
        )
        .unwrap();
        let exec = ExecConfig {
            command: Some("iceon-fixture-auth-helper".into()),
            api_version: Some(V1.into()),
            interactive_mode: Some(ExecInteractiveMode::Never),
            args: Some(vec!["--fixture".into()]),
            env: Some(
                env.into_iter()
                    .map(|(name, value)| [("name".into(), name), ("value".into(), value)].into())
                    .collect(),
            ),
            ..Default::default()
        };
        let request = helper_request(&exec, &Cluster::default(), home.path()).unwrap();
        let output = run(request, Duration::from_secs(2)).await.unwrap();
        let response: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(response["status"]["token"], "synthetic-output");
    }
}
