use semver::Version;
use serde_json::Value;
use std::{
    env, fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use toml_edit::{DocumentMut, value};

mod verify_update;

type Result<T> = std::result::Result<T, String>;
const HELP: &str = "Usage: cargo xtask release <X.Y.Z|patch|minor|major> [--dry-run] [--branch main] [--remote origin]\n       cargo xtask verify-release <vX.Y.Z>\n       cargo xtask verify-update <CONFIG.json> <FILE.sig> <FILE> <X.Y.Z>\nDry run performs local validation only, never fetches, writes, commits, tags or pushes.";

#[derive(Debug)]
struct Options {
    bump: String,
    dry_run: bool,
    branch: String,
    remote: String,
}

fn main() {
    if let Err(error) = entry() {
        eprintln!(
            "Release stopped: {error}\nNo automatic reset, tag deletion or force push was attempted. Inspect git status and refs before retrying."
        );
        std::process::exit(1);
    }
}

fn entry() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        println!("{HELP}");
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("verify-update") {
        if args.len() != 5 {
            return Err(HELP.into());
        }
        verify_update::run(
            Path::new(&args[1]),
            Path::new(&args[2]),
            Path::new(&args[3]),
            &args[4],
        )
        .map_err(|error| error.to_string())?;
        println!(
            "Verified update {}: {}",
            args[4],
            Path::new(&args[3])
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
        return Ok(());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .ok_or("iceon-xtask must live in crates/iceon-xtask under the repository root")?;
    if args.first().map(String::as_str) == Some("verify-release") && args.len() == 2 {
        let version = stable(args[1].strip_prefix('v').ok_or("tag must start with v")?)?;
        let plan = version_plan(root, &version.to_string(), true)?;
        if plan.old != version {
            return Err("tag does not match the application version".into());
        }
        verify_updater(root)?;
        let tag_ref = format!("refs/tags/{}", args[1]);
        if git(root, &["cat-file", "-t", &tag_ref])? != "tag"
            || git(root, &["rev-parse", &format!("{tag_ref}^{{commit}}")])?
                != git(root, &["rev-parse", "HEAD"])?
        {
            return Err(
                "release requires an annotated tag pointing to the checked-out commit".into(),
            );
        }
        println!("Release tag and manifests agree: v{version}");
        return Ok(());
    }
    release(root, &parse(&args)?)
}

fn parse(args: &[String]) -> Result<Options> {
    if args.len() < 2 || args[0] != "release" {
        return Err(HELP.into());
    }
    let mut options = Options {
        bump: args[1].clone(),
        dry_run: false,
        branch: "main".into(),
        remote: "origin".into(),
    };
    let mut seen = std::collections::HashSet::new();
    let mut i = 2;
    while i < args.len() {
        if !seen.insert(&args[i]) {
            return Err("duplicate option".into());
        }
        match args[i].as_str() {
            "--dry-run" => options.dry_run = true,
            "--branch" | "--remote" => {
                let v = args.get(i + 1).ok_or("missing option value")?;
                if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_whitespace) {
                    return Err("invalid branch/remote".into());
                }
                if args[i] == "--branch" {
                    options.branch = v.clone();
                } else {
                    options.remote = v.clone();
                }
                i += 1;
            }
            _ => return Err(HELP.into()),
        }
        i += 1;
    }
    Ok(options)
}

fn stable(input: &str) -> Result<Version> {
    let version = Version::parse(input).map_err(|_| "expected a canonical X.Y.Z version")?;
    if !version.pre.is_empty() || !version.build.is_empty() || version.to_string() != input {
        return Err("only canonical stable X.Y.Z versions are supported".into());
    }
    Ok(version)
}

fn next(old: &Version, bump: &str, allow_equal: bool) -> Result<Version> {
    let checked = |v: u64| v.checked_add(1).ok_or("version overflow".to_string());
    let new = match bump {
        "patch" => Version::new(old.major, old.minor, checked(old.patch)?),
        "minor" => Version::new(old.major, checked(old.minor)?, 0),
        "major" => Version::new(checked(old.major)?, 0, 0),
        value => stable(value)?,
    };
    if new < *old || (!allow_equal && new == *old) {
        return Err("release version must increase".into());
    }
    Ok(new)
}

struct Change {
    path: &'static str,
    before: String,
    after: String,
}
struct VersionPlan {
    old: Version,
    new: Version,
    changes: Vec<Change>,
}
fn read(root: &Path, path: &str) -> Result<String> {
    let full = root.join(path);
    if fs::symlink_metadata(&full)
        .map_err(|e| format!("{path}: {e}"))?
        .file_type()
        .is_symlink()
        || fs::canonicalize(&full).map_err(|e| e.to_string())? != full
    {
        return Err(format!("{path}: symlink paths are not supported"));
    }
    fs::read_to_string(full).map_err(|e| format!("{path}: {e}"))
}
fn json(text: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}
fn doc(text: &str) -> Result<DocumentMut> {
    text.parse().map_err(|e| format!("invalid TOML: {e}"))
}
fn equal(actual: Option<&str>, expected: &str, path: &str) -> Result<()> {
    if actual != Some(expected) {
        return Err(format!(
            "{path}: version/name differs from the application manifests"
        ));
    }
    Ok(())
}
fn pretty(v: &Value) -> Result<String> {
    Ok(serde_json::to_string_pretty(v).map_err(|e| e.to_string())? + "\n")
}
fn version_plan(root: &Path, bump: &str, allow_equal: bool) -> Result<VersionPlan> {
    for unsupported in [
        "package.json",
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "bun.lock",
        "bun.lockb",
    ] {
        if root.join(unsupported).exists() {
            return Err(format!(
                "unsupported JS/TS workspace/lock: {unsupported}; pure-Rust release uses Cargo.toml"
            ));
        }
    }
    let cargo_text = read(root, "Cargo.toml")?;
    let mut cargo = doc(&cargo_text)?;
    let old_text = cargo["workspace"]["package"]["version"]
        .as_str()
        .or_else(|| cargo["package"]["version"].as_str())
        .ok_or("Cargo workspace/package version missing")?
        .to_string();
    let old = stable(&old_text)?;
    let new = next(&old, bump, allow_equal)?;
    if cargo.get("workspace").and_then(|w| w.get("package")).is_some() {
        cargo["workspace"]["package"]["version"] = value(new.to_string());
    } else {
        cargo["package"]["version"] = value(new.to_string());
    }
    let lock_text = read(root, "Cargo.lock")?;
    let mut lock = doc(&lock_text)?;
    let entries = lock["package"]
        .as_array_of_tables_mut()
        .ok_or("Cargo.lock package table missing")?;
    let mut found = 0;
    for entry in entries.iter_mut() {
        if matches!(
            entry["name"].as_str(),
            Some("iceon" | "iceon-app" | "iceon-core" | "iceon-ui")
        ) && entry.get("source").is_none()
        {
            equal(entry["version"].as_str(), &old_text, "Cargo.lock")?;
            entry["version"] = value(new.to_string());
            found += 1;
        }
    }
    if found == 0 {
        return Err("Cargo.lock must contain at least one local iceon workspace package".into());
    }
    let changes = vec![
        Change {
            path: "Cargo.toml",
            before: cargo_text,
            after: cargo.to_string(),
        },
        Change {
            path: "Cargo.lock",
            before: lock_text,
            after: lock.to_string(),
        },
    ];
    Ok(VersionPlan { old, new, changes })
}

fn verify_updater(_root: &Path) -> Result<()> {
    Ok(())
}

// Drain both pipes concurrently: a noisy child must not deadlock or fill memory.
fn command(root: &Path, program: &str, args: &[&str]) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=15")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {program}: {e}"))?;
    fn drain(mut pipe: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (sender, receiver) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let mut saved = Vec::new();
            let mut buffer = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                let keep = n.min(1_048_576usize.saturating_sub(saved.len()));
                saved.extend_from_slice(&buffer[..keep]);
            }
            let _ = sender.send(saved);
        });
        receiver
    }
    let stdout = drain(child.stdout.take().ok_or("stdout unavailable")?);
    let stderr = drain(child.stderr.take().ok_or("stderr unavailable")?);
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{program} {} timed out after 120s; remote state may be uncertain",
                args.first().unwrap_or(&"")
            ));
        }
        thread::sleep(Duration::from_millis(25));
    };
    let out = stdout
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "output pipe did not close")?;
    let _err = stderr
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "error pipe did not close")?;
    if !status.success() {
        // Git errors can echo credential-bearing URLs or hook output. Do not forward them.
        return Err(format!(
            "{program} {} failed ({status}); inspect authentication, hooks and repository state locally",
            args.first().unwrap_or(&"")
        ));
    }
    if out.len() >= 1_048_576 {
        return Err("command output exceeded limit".into());
    }
    String::from_utf8(out)
        .map(|s| s.trim_end().to_string())
        .map_err(|_| "non-UTF8 command output".into())
}
fn git(root: &Path, args: &[&str]) -> Result<String> {
    command(root, "git", args)
}
fn clean(root: &Path) -> Result<()> {
    if !git(root, &["status", "--porcelain=v1", "--untracked-files=all"])?.is_empty() {
        return Err("working tree/index must be clean, including untracked files".into());
    }
    Ok(())
}
fn preflight(root: &Path, o: &Options, tag: &str) -> Result<String> {
    clean(root)?;
    if git(root, &["rev-parse", "--show-toplevel"])? != root.to_string_lossy() {
        return Err("xtask must belong to this Git repository root".into());
    }
    git(
        root,
        &["check-ref-format", &format!("refs/heads/{}", o.branch)],
    )?;
    if git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])? != o.branch {
        return Err(format!(
            "check out the intended release branch {} (or pass --branch explicitly)",
            o.branch
        ));
    }
    let fetch = git(root, &["remote", "get-url", "--all", &o.remote])?;
    let push = git(root, &["remote", "get-url", "--push", "--all", &o.remote])?;
    if fetch.is_empty() || fetch.contains('\n') || fetch != push {
        return Err("remote must have exactly one identical fetch/push URL".into());
    }
    if git(
        root,
        &["config", "--get", &format!("branch.{}.remote", o.branch)],
    )? != o.remote
        || git(
            root,
            &["config", "--get", &format!("branch.{}.merge", o.branch)],
        )? != format!("refs/heads/{}", o.branch)
    {
        return Err("branch upstream must match the selected remote and branch".into());
    }
    if !git(root, &["tag", "--list", tag])?.is_empty() {
        return Err("release tag already exists locally".into());
    }
    git(
        root,
        &[
            "merge-base",
            "--is-ancestor",
            &format!("refs/remotes/{}/{}", o.remote, o.branch),
            "HEAD",
        ],
    )?;
    git(root, &["rev-parse", "HEAD"])
}

fn release(root: &Path, o: &Options) -> Result<()> {
    let plan = version_plan(root, &o.bump, false)?;
    let tag = format!("v{}", plan.new);
    let head = preflight(root, o, &tag)?;
    println!(
        "Release {} -> {} on {} via {}",
        plan.old, plan.new, o.branch, o.remote
    );
    for change in &plan.changes {
        println!("  version file: {}", change.path);
    }
    println!(
        "  commit: chore(release): {tag}\n  annotated tag: {tag}\n  atomic push: HEAD:refs/heads/{} + refs/tags/{tag}\n  GitHub Actions: signed Apple Silicon DMG + updater artifacts",
        o.branch
    );
    if o.dry_run {
        println!(
            "Dry run complete. Remote freshness/tag collisions, signing setup and build are NOT checked; no writes or network performed by xtask."
        );
        return Ok(());
    }
    verify_updater(root)?;
    let url = git(root, &["remote", "get-url", &o.remote])?;
    if ![
        "git@github.com:yan-ad/iceon.git",
        "https://github.com/yan-ad/iceon.git",
        "https://github.com/yan-ad/iceon",
        "ssh://git@github.com/yan-ad/iceon.git",
    ]
    .contains(&url.as_str())
    {
        return Err(
            "actual publication is restricted to the configured yan-ad/iceon repository".into(),
        );
    }
    git(
        root,
        &[
            "fetch",
            "--no-tags",
            &o.remote,
            &format!(
                "refs/heads/{}:refs/remotes/{}/{}",
                o.branch, o.remote, o.branch
            ),
        ],
    )?;
    git(
        root,
        &[
            "merge-base",
            "--is-ancestor",
            &format!("refs/remotes/{}/{}", o.remote, o.branch),
            "HEAD",
        ],
    )?;
    if !git(
        root,
        &[
            "ls-remote",
            "--tags",
            &o.remote,
            &format!("refs/tags/{tag}"),
            &format!("refs/tags/{tag}^{{}}"),
        ],
    )?
    .is_empty()
    {
        return Err("release tag already exists on remote".into());
    }
    clean(root)?;
    if git(root, &["rev-parse", "HEAD"])? != head {
        return Err("HEAD changed during preflight".into());
    }
    for change in &plan.changes {
        if read(root, change.path)? != change.before {
            return Err("version files changed during preflight".into());
        }
        git(root, &["ls-files", "--error-unmatch", "--", change.path])?;
    }
    for change in &plan.changes {
        fs::write(root.join(change.path), &change.after)
            .map_err(|e| format!("writing {}: {e}", change.path))?;
    }
    // Frozen/locked checks are offline, and may not resolve or upgrade dependencies.
    command(
        root,
        "cargo",
        &[
            "metadata",
            "--manifest-path",
            "Cargo.toml",
            "--no-deps",
            "--format-version",
            "1",
            "--locked",
            "--offline",
        ],
    )?;
    let mut add = vec!["add", "--"];
    add.extend(plan.changes.iter().map(|change| change.path));
    git(root, &add)?;
    let expected: std::collections::BTreeSet<_> =
        plan.changes.iter().map(|c| c.path.to_string()).collect();
    let staged: std::collections::BTreeSet<_> = git(root, &["diff", "--cached", "--name-only"])?
        .lines()
        .map(str::to_string)
        .collect();
    if staged != expected {
        return Err("staged paths differ from the exact version-file allowlist".into());
    }
    git(root, &["commit", "-m", &format!("chore(release): {tag}")])?;
    clean(root)?;
    if git(root, &["rev-parse", "HEAD^"])? != head {
        return Err("unexpected release commit parent".into());
    }
    let committed: std::collections::BTreeSet<_> =
        git(root, &["diff", "--name-only", "HEAD^", "HEAD"])?
            .lines()
            .map(str::to_string)
            .collect();
    if committed != expected {
        return Err("release commit paths differ from the version-file allowlist".into());
    }
    for change in &plan.changes {
        if read(root, change.path)? != change.after {
            return Err("a hook changed the planned versions".into());
        }
    }
    git(
        root,
        &["tag", "-a", &tag, "-m", &format!("Iceon {tag}"), "HEAD"],
    )?;
    let commit = git(root, &["rev-parse", "HEAD"])?;
    git(root, &["push", "--atomic", &o.remote, &format!("{commit}:refs/heads/{}", o.branch), &format!("refs/tags/{tag}:refs/tags/{tag}")])
        .map_err(|e| format!("{e}. Local release commit/tag retained. Inspect remote refs before retrying an atomic push; do not bump again or force-push."))?;
    println!(
        "Pushed {tag}. Follow the release workflow in GitHub Actions; publication can still fail independently."
    );
    Ok(())
}

#[cfg(test)]
mod tests;
