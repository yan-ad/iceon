//! Data-only importer for Zed marketplace icon themes. No extension code is extracted or run.
//!
//! The marketplace lists versions at `/extensions/{id}` and redirects downloads to
//! Zed's extension object store. Both response bodies and decompression are bounded.
//! Only normalized icon theme definitions, license text, and safe SVG assets survive import.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use flate2::read::MultiGzDecoder;
use fs2::FileExt;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

const API: &str = "https://api.zed.dev";
const METADATA_LIMIT: usize = 2 * 1024 * 1024;
const DOWNLOAD_LIMIT: usize = 32 * 1024 * 1024;
const EXPANDED_LIMIT: usize = 128 * 1024 * 1024;
const FILE_LIMIT: usize = 4 * 1024 * 1024;
const RECORD_LIMIT: usize = 24 * 1024 * 1024;
const ENTRY_LIMIT: usize = 65536;
const THEME_LIMIT: usize = 512;
const INSTALLED_LIMIT: usize = 256;
const DEPTH_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DirectoryIcons {
    #[serde(default)]
    pub collapsed: Option<String>,
    #[serde(default)]
    pub expanded: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ChevronIcons {
    #[serde(default)]
    pub collapsed: Option<String>,
    #[serde(default)]
    pub expanded: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIconDef {
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZedIconTheme {
    pub id: String,
    pub extension: String,
    pub name: String,
    pub appearance: String,
    #[serde(default)]
    pub directory_icons: Option<DirectoryIcons>,
    #[serde(default)]
    pub chevron_icons: Option<ChevronIcons>,
    #[serde(default)]
    pub named_directory_icons: BTreeMap<String, DirectoryIcons>,
    #[serde(default)]
    pub file_stems: BTreeMap<String, String>,
    #[serde(default)]
    pub file_suffixes: BTreeMap<String, String>,
    #[serde(default)]
    pub file_icons: BTreeMap<String, FileIconDef>,
}

impl ZedIconTheme {
    /// Resolve an icon name or path (e.g. `icons/chevron-right.svg`, `server`, `folder`)
    /// to the relative SVG path in the extension (e.g. `icons/latte/folder_server.svg`).
    pub fn resolve_icon(&self, icon_name_or_path: &str) -> Option<String> {
        let clean = icon_name_or_path.trim();
        let stem = clean
            .strip_prefix("icons/")
            .unwrap_or(clean)
            .strip_suffix(".svg")
            .unwrap_or(clean)
            .to_ascii_lowercase();

        let clean_path = |p: &str| -> String {
            let mut s = p.trim();
            if let Some(stripped) = s.strip_prefix("./") {
                s = stripped;
            }
            if let Some(stripped) = s.strip_prefix('/') {
                s = stripped;
            }
            s.to_string()
        };

        // 1. Chevrons
        match stem.as_str() {
            "chevron-right" | "chevron-left" => {
                if let Some(c) = &self.chevron_icons
                    && let Some(p) = &c.collapsed
                {
                    return Some(clean_path(p));
                }
            }
            "chevron-down" | "chevron-up" => {
                if let Some(c) = &self.chevron_icons
                    && let Some(p) = &c.expanded
                {
                    return Some(clean_path(p));
                }
            }
            "folder" => {
                if let Some(d) = &self.directory_icons
                    && let Some(p) = &d.collapsed
                {
                    return Some(clean_path(p));
                }
            }
            "folder-open" => {
                if let Some(d) = &self.directory_icons
                    && let Some(p) = &d.expanded
                {
                    return Some(clean_path(p));
                }
            }
            _ => {}
        }

        let find_key = |key: &str| -> Option<String> {
            if let Some(def) = self.file_icons.get(key) {
                return Some(clean_path(&def.path));
            }
            if let Some(icon_key) = self.file_stems.get(key)
                && let Some(def) = self.file_icons.get(icon_key)
            {
                return Some(clean_path(&def.path));
            }
            if let Some(icon_key) = self.file_suffixes.get(key)
                && let Some(def) = self.file_icons.get(icon_key)
            {
                return Some(clean_path(&def.path));
            }
            if let Some(dir) = self.named_directory_icons.get(key)
                && let Some(p) = &dir.collapsed
            {
                return Some(clean_path(p));
            }
            None
        };

        // 2. Direct match with stem
        if let Some(path) = find_key(&stem) {
            return Some(path);
        }

        // 3. Semantic aliases covering all Kubernetes sections, actions, tabs, and menus
        let aliases: &[&str] = match stem.as_str() {
            "layout-dashboard" => &[
                "dashboard",
                "project",
                "view",
                "window",
                "home",
                "layout",
                "browser",
            ],
            "server" => &["server", "host", "node", "compute", "cluster", "linux"],
            "box" | "container" => &[
                "docker",
                "container",
                "pod",
                "cube",
                "package",
                "helm",
                "kubernetes",
                "k8s",
            ],
            "layers" | "layers2" | "layers-2" => &[
                "kubernetes",
                "k8s",
                "layers",
                "stack",
                "deploy",
                "namespace",
                "group",
            ],
            "network" | "cable" | "route" | "signpost" => &[
                "network",
                "routing",
                "link",
                "connection",
                "port",
                "traffic",
            ],
            "database" | "hard-drive" | "hard-drive-download" => &[
                "database", "db", "sql", "storage", "disk", "drive", "volume",
            ],
            "copy" | "copy-check" | "clipboard-check" => &[
                "copy",
                "clone",
                "duplicate",
                "files",
                "document",
                "template",
            ],
            "list-checks" | "list-ordered" => {
                &["task", "job", "todo", "list", "test", "check", "ci"]
            }
            "calendar-clock" | "timer" => &[
                "clock", "calendar", "schedule", "cron", "time", "history", "timer", "watch",
            ],
            "file-text" | "file" | "file-check" => &[
                "yaml", "yml", "config", "settings", "text", "document", "file", "json", "toml",
            ],
            "key-round" | "crown" => &[
                "key",
                "lock",
                "cert",
                "certificate",
                "secret",
                "auth",
                "admin",
                "security",
            ],
            "chart-pie" => &[
                "chart",
                "analytics",
                "graph",
                "quota",
                "metrics",
                "dashboard",
            ],
            "sliders-horizontal" | "settings" | "settings-2" => {
                &["settings", "config", "toml", "tune", "controls", "sliders"]
            }
            "arrow-left-right" | "arrow-up-down" => {
                &["arrows", "scale", "sync", "transfer", "swap", "refresh"]
            }
            "shield" | "shield-check" | "shield-alert" => {
                &["shield", "security", "cert", "guard", "policy", "lock"]
            }
            "webhook" => &["webhook", "hook", "api", "http", "network", "link"],
            "badge-check" | "check" => &["check", "badge", "cert", "policy", "success", "verified"],
            "plug" | "blocks" | "puzzle" => &[
                "plugin",
                "extension",
                "service",
                "connect",
                "components",
                "puzzle",
                "crd",
            ],
            "map-pin" => &["location", "pin", "target", "endpoint", "network", "point"],
            "archive" => &["archive", "zip", "tar", "package", "storage", "bundle"],
            "bell" => &["bell", "notification", "event", "alert", "notice", "log"],
            "package" | "package-check" => {
                &["helm", "package", "cube", "box", "tar", "bundle", "docker"]
            }
            "users" | "user-cog" | "contact" => &[
                "users", "user", "team", "people", "account", "profile", "person", "admin",
            ],
            "link" | "link-2" => &["link", "url", "binding", "chain", "connection"],
            "square-terminal" | "terminal" => &[
                "terminal", "console", "bash", "shell", "prompt", "sh", "zsh",
            ],
            "logs" => &["log", "logs", "document", "text", "file"],
            "square-arrow-up-left-exit" => &["terminal", "console", "external", "link", "connect"],
            "trash" | "shredder" => &["trash", "delete", "remove", "bin"],
            "pencil" => &["edit", "pencil", "write"],
            "refresh-cw" => &["refresh", "reload", "sync"],
            "search" => &["search", "find"],
            "download" => &["download", "install", "get"],
            "git-pull-request" => &["git", "vcs", "branch"],
            "info" => &["info", "about"],
            "triangle-alert" | "circle-alert" | "ban" => &["warning", "alert", "error"],
            _ => &[],
        };

        for alias in aliases {
            if let Some(path) = find_key(alias) {
                return Some(path);
            }
        }

        None
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstalledIconExtension {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub download_count: Option<u64>,
    pub themes: Vec<ZedIconTheme>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IconRecord {
    extension: InstalledIconExtension,
    licenses: BTreeMap<String, String>,
}

fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id.as_bytes()[0].is_ascii_alphanumeric()
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err("Invalid Zed extension id".into());
    }
    Ok(())
}

fn theme_id(extension: &str, name: &str) -> String {
    format!(
        "{extension}/{}",
        percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC)
    )
}

fn root() -> PathBuf {
    crate::runtime::app_data_dir()
        .join("themes")
        .join("zed_icons")
}

fn checked_dir(path: &Path, create: bool) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Theme store requires an absolute path".into());
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err("Unsafe theme store path".into());
        }
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => (),
            Ok(_) => return Err("Theme store path is not a regular directory".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
                match fs::create_dir(&current) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                    Err(e) => return Err(e.to_string()),
                }
                let meta = fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
                if !meta.is_dir() || meta.file_type().is_symlink() {
                    return Err("Unsafe theme store directory".into());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn open_regular(path: &Path, create: bool) -> Result<File, String> {
    if let Ok(meta) = fs::symlink_metadata(path)
        && (!meta.is_file() || meta.file_type().is_symlink())
    {
        return Err("Theme store file is not a regular file".into());
    }
    let mut options = OpenOptions::new();
    options.read(true).write(create).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .mode(0o600);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Theme store file is not regular".into());
    }
    Ok(file)
}

fn store_lock(root: &Path) -> Result<File, String> {
    checked_dir(root, true)?;
    let lock = open_regular(&root.join(".lock"), true)?;
    lock.lock_exclusive().map_err(|e| e.to_string())?;
    Ok(lock)
}

fn read_limited(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Data exceeds size limit".into());
    }
    Ok(bytes)
}

fn read_record(root: &Path, id: &str) -> Result<IconRecord, String> {
    valid_id(id)?;
    let dir = root.join(id);
    checked_dir(&dir, false)?;
    let bytes = read_limited(
        open_regular(&dir.join("installed.json"), false)?,
        RECORD_LIMIT,
    )?;
    let record: IconRecord = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    validate_extension(&record.extension, id)?;
    Ok(record)
}

fn validate_extension(extension: &InstalledIconExtension, id: &str) -> Result<(), String> {
    if extension.id != id
        || semver::Version::parse(&extension.version).is_err()
        || extension.themes.is_empty()
        || extension.themes.len() > THEME_LIMIT
    {
        return Err("Invalid installed icon extension record".into());
    }
    if (!extension.name.is_empty() && extension.name.trim().is_empty())
        || extension.name.len() > 256
        || extension.description.len() > 4096
    {
        return Err("Invalid installed icon marketplace metadata".into());
    }
    let mut ids = BTreeSet::new();
    for theme in &extension.themes {
        validate_name(&theme.name)?;
        if theme.extension != id
            || theme.id != theme_id(id, &theme.name)
            || !matches!(theme.appearance.as_str(), "light" | "dark")
            || !ids.insert(&theme.id)
        {
            return Err("Invalid or duplicate installed icon theme id".into());
        }
    }
    Ok(())
}

/// List verified local icon records.
pub fn installed() -> Result<Vec<InstalledIconExtension>, String> {
    installed_at(&root())
}

fn installed_at(root: &Path) -> Result<Vec<InstalledIconExtension>, String> {
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
        Ok(_) => checked_dir(root, false)?,
    }
    let _lock = store_lock(root)?;
    let mut extensions = Vec::new();
    for (index, entry) in fs::read_dir(root).map_err(|e| e.to_string())?.enumerate() {
        if index > INSTALLED_LIMIT * 2 {
            return Err("Too many theme store entries".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Invalid theme store filename")?;
        if name.starts_with('.') {
            continue;
        }
        if extensions.len() >= INSTALLED_LIMIT {
            return Err("Too many installed extensions".into());
        }
        extensions.push(read_record(root, &name)?.extension);
    }
    extensions.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(extensions)
}

/// Find an icon theme by its full id (e.g. `catppuccin-icons/Catppuccin%20Mocha`).
pub fn find_theme(id: &str) -> Option<ZedIconTheme> {
    let (extension, _) = id.split_once('/')?;
    valid_id(extension).ok()?;
    let root = root();
    let _lock = store_lock(&root).ok()?;
    read_record(&root, extension)
        .ok()?
        .extension
        .themes
        .into_iter()
        .find(|t| t.id == id)
}

/// Safely read an SVG file belonging to an installed icon theme extension.
pub fn read_svg(extension_id: &str, relative_path: &str) -> Result<Vec<u8>, String> {
    valid_id(extension_id)?;
    let clean_path = relative_path
        .strip_prefix("./")
        .unwrap_or(relative_path)
        .strip_prefix('/')
        .unwrap_or(relative_path);

    if clean_path.is_empty()
        || clean_path.len() > 512
        || clean_path.contains(['\\', ':', '\0'])
        || clean_path.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
        })
    {
        return Err("Unsafe SVG path".into());
    }

    let ext_root = root().join(extension_id);
    let target = ext_root.join(clean_path);

    // Verify canonical path does not escape ext_root
    let canonical = target
        .canonicalize()
        .map_err(|e| format!("SVG file not found: {e}"))?;
    let canonical_root = ext_root
        .canonicalize()
        .map_err(|e| format!("Extension root not found: {e}"))?;

    if !canonical.starts_with(&canonical_root) {
        return Err("Path traversal forbidden".into());
    }

    let file = open_regular(&target, false)?;
    read_limited(file, FILE_LIMIT)
}

/// Remove an installed icon theme extension safely.
pub fn uninstall(id: &str) -> Result<(), String> {
    uninstall_at(&root(), id)
}

fn uninstall_at(root: &Path, id: &str) -> Result<(), String> {
    valid_id(id)?;
    let _lock = store_lock(root)?;
    let dir = root.join(id);
    match fs::symlink_metadata(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
        Ok(_) => checked_dir(&dir, false)?,
    }

    // Atomically move to a trash prefix before removing
    let trash_dir = tempfile::Builder::new()
        .prefix(&format!(".trash-{id}-"))
        .tempdir_in(root)
        .map_err(|e| e.to_string())?
        .keep();
    let _ = fs::remove_dir(&trash_dir);
    fs::rename(&dir, &trash_dir).map_err(|e| e.to_string())?;

    let _ = fs::remove_dir_all(trash_dir);
    Ok(())
}

fn allowed_url(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some("api.zed.dev" | "zed-extensions.nyc3.digitaloceanspaces.com")
        )
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !allowed_url(attempt.url()) {
                attempt.error("Disallowed Zed marketplace redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "Unable to create Zed marketplace client".into())
}

async fn download(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let parsed = url::Url::parse(url).map_err(|_| "Invalid marketplace URL")?;
    if !allowed_url(&parsed) {
        return Err("Disallowed marketplace URL".into());
    }
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|_| "Marketplace request failed")?;
    if !response.status().is_success() {
        return Err(format!(
            "Marketplace returned HTTP {}",
            response.status().as_u16()
        ));
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("Marketplace response exceeds size limit".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Marketplace body download failed")?;
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err("Marketplace response exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct MarketplaceExtension {
    pub id: String,
    pub name: String,
    pub description: String,
    pub download_count: u64,
}

fn marketplace_url(query: &str) -> Result<url::Url, String> {
    let query = query.trim();
    if query.len() > 256 || query.chars().any(char::is_control) {
        return Err("Search must be at most 256 bytes without control characters".into());
    }
    let mut url = url::Url::parse("https://cloud.zed.dev/extensions").unwrap();
    url.query_pairs_mut()
        .append_pair("max_schema_version", "1")
        .append_pair("provides", "icon-themes");
    if !query.is_empty() {
        url.query_pairs_mut().append_pair("filter", query);
    }
    Ok(url)
}

fn parse_marketplace(bytes: &[u8]) -> Result<Vec<MarketplaceExtension>, String> {
    #[derive(Deserialize)]
    struct Listing {
        data: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        #[serde(flatten)]
        metadata: MarketplaceExtension,
        provides: Vec<String>,
    }
    if bytes.len() > METADATA_LIMIT {
        return Err("Marketplace response exceeds size limit".into());
    }
    let listing: Listing =
        serde_json::from_slice(bytes).map_err(|_| "Invalid marketplace response".to_owned())?;
    if listing.data.len() > 4096 {
        return Err("Too many marketplace extensions".into());
    }
    let mut ids = BTreeSet::new();
    let mut extensions = Vec::new();
    for entry in listing.data {
        let item = entry.metadata;
        valid_id(&item.id)?;
        if item.name.trim().is_empty()
            || item.name.len() > 256
            || item.description.len() > 4096
            || !ids.insert(item.id.clone())
            || !entry
                .provides
                .iter()
                .any(|kind| kind == "icon-themes" || kind == "icon_themes")
        {
            continue;
        }
        extensions.push(item);
    }
    Ok(extensions)
}

/// Browse the icon-theme marketplace using query search.
pub async fn marketplace(query: &str) -> Result<Vec<MarketplaceExtension>, String> {
    let url = marketplace_url(query)?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Unable to create marketplace browser".to_owned())?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| "Marketplace search failed".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "Marketplace returned HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > METADATA_LIMIT as u64)
    {
        return Err("Marketplace response exceeds size limit".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Marketplace body download failed".to_owned())?;
        if chunk.len() > METADATA_LIMIT.saturating_sub(bytes.len()) {
            return Err("Marketplace response exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_marketplace(&bytes)
}

fn latest_version(bytes: &[u8], id: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Listing {
        data: Vec<Version>,
    }
    #[derive(Deserialize)]
    struct Version {
        id: String,
        version: String,
    }
    let listing: Listing =
        serde_json::from_slice(bytes).map_err(|_| "Invalid marketplace version listing")?;
    if listing.data.len() > 4096 {
        return Err("Too many marketplace versions".into());
    }
    let mut versions = Vec::new();
    for version in listing.data {
        if version.id != id {
            return Err("Marketplace extension identity mismatch".into());
        }
        let parsed =
            semver::Version::parse(&version.version).map_err(|_| "Invalid marketplace semver")?;
        versions.push((parsed, version.version));
    }
    versions.sort();
    versions
        .pop()
        .map(|(_, version)| version)
        .ok_or_else(|| "No marketplace versions found".into())
}

/// Install the highest semantic version of an icon theme from Zed.
pub async fn install(id: &str) -> Result<InstalledIconExtension, String> {
    valid_id(id)?;
    let metadata = marketplace(id)
        .await?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "Icon theme extension was not found in the marketplace".to_owned())?;
    install_with_metadata(&metadata).await
}

/// Install an exact marketplace listing while preserving its display metadata.
pub async fn install_with_metadata(
    item: &MarketplaceExtension,
) -> Result<InstalledIconExtension, String> {
    let id = item.id.as_str();
    valid_id(id)?;
    if item.name.trim().is_empty() || item.name.len() > 256 || item.description.len() > 4096 {
        return Err("Invalid marketplace icon theme metadata".into());
    }
    let item = item.clone();
    let client = client()?;
    let metadata = download(&client, &format!("{API}/extensions/{id}"), METADATA_LIMIT).await?;
    let version = latest_version(&metadata, id)?;
    let archive = download(
        &client,
        &format!("{API}/extensions/{id}/{version}/download"),
        DOWNLOAD_LIMIT,
    )
    .await?;
    let id = id.to_owned();
    let root = root();
    tokio::task::spawn_blocking(move || {
        let (mut record, files) = parse_archive(&archive, &id, &version)?;
        record.extension.name = item.name;
        record.extension.description = item.description;
        record.extension.download_count = Some(item.download_count);
        publish(&root, &record, &files)?;
        Ok(record.extension)
    })
    .await
    .map_err(|_| "Icon theme installation worker failed".to_string())?
}

fn publish(
    root: &Path,
    record: &IconRecord,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    valid_id(&record.extension.id)?;
    validate_extension(&record.extension, &record.extension.id)?;
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    if bytes.len() > RECORD_LIMIT {
        return Err("Installed icon theme record exceeds size limit".into());
    }
    let _lock = store_lock(root)?;

    let mut count = 0;
    for (index, entry) in fs::read_dir(root).map_err(|e| e.to_string())?.enumerate() {
        if index > INSTALLED_LIMIT * 2 {
            return Err("Too many theme store entries".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_name().to_string_lossy().starts_with('.') {
            count += 1;
        }
    }
    if count >= INSTALLED_LIMIT
        && !root
            .join(&record.extension.id)
            .try_exists()
            .map_err(|e| e.to_string())?
    {
        return Err("Too many installed extensions".into());
    }

    // Prepare staging directory
    let stage_dir = tempfile::Builder::new()
        .prefix(&format!(".stage-{}-", record.extension.id))
        .tempdir_in(root)
        .map_err(|e| e.to_string())?
        .keep();

    // Write all SVG files into stage_dir
    for (rel_path, data) in files {
        let dest = stage_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            checked_dir(parent, true)?;
        }
        let mut file = open_regular(&dest, true)?;
        file.write_all(data).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }

    // Write installed.json
    let record_file = stage_dir.join("installed.json");
    let mut file = open_regular(&record_file, true)?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;

    // Atomic rename into target
    let final_dir = root.join(&record.extension.id);
    if final_dir.exists() {
        let trash_dir = tempfile::Builder::new()
            .prefix(&format!(".old-{}-", record.extension.id))
            .tempdir_in(root)
            .map_err(|e| e.to_string())?
            .keep();
        let _ = fs::remove_dir(&trash_dir);
        fs::rename(&final_dir, &trash_dir).map_err(|e| e.to_string())?;
        let _ = fs::remove_dir_all(trash_dir);
    }

    fs::rename(&stage_dir, &final_dir).map_err(|e| e.to_string())?;
    Ok(())
}

fn archive_path(raw: &[u8]) -> Result<String, String> {
    let raw = std::str::from_utf8(raw).map_err(|_| "Non-UTF8 archive path")?;
    if raw.len() > 512 || raw.contains(['\\', ':', '\0']) || raw.starts_with('/') {
        return Err("Unsafe archive path".into());
    }
    let raw = raw.strip_prefix("./").unwrap_or(raw).trim_end_matches('/');
    if raw.is_empty() {
        return Ok(String::new());
    }
    if raw.split('/').any(|part| {
        part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
    }) {
        return Err("Unsafe archive path component".into());
    }
    Ok(raw.to_owned())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err("Invalid icon theme name".into());
    }
    Ok(())
}

fn check_depth(text: &str) -> Result<(), String> {
    let mut chars = text.chars().peekable();
    let mut depth = 0usize;
    let mut quote = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == '\\' {
                chars.next();
            } else if c == q {
                quote = None;
            }
            continue;
        }
        if matches!(c, '"' | '\'') {
            quote = Some(c);
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' || c == '\r' {
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            continue;
        }
        if matches!(c, '{' | '[') {
            depth += 1;
            if depth > DEPTH_LIMIT {
                return Err("Icon theme JSON nesting exceeds limit".into());
            }
        } else if matches!(c, '}' | ']') {
            depth = depth.saturating_sub(1);
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct RawIconThemeFamily {
    #[serde(default)]
    themes: Vec<RawIconThemeDef>,
}

#[derive(Deserialize)]
struct RawIconThemeDef {
    name: String,
    appearance: String,
    #[serde(default)]
    directory_icons: Option<DirectoryIcons>,
    #[serde(default)]
    chevron_icons: Option<ChevronIcons>,
    #[serde(default)]
    named_directory_icons: BTreeMap<String, DirectoryIcons>,
    #[serde(default)]
    file_stems: BTreeMap<String, String>,
    #[serde(default)]
    file_suffixes: BTreeMap<String, String>,
    #[serde(default)]
    file_icons: BTreeMap<String, FileIconDef>,
}

fn parse_icon_family(bytes: &[u8], extension: &str) -> Result<Vec<ZedIconTheme>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Icon theme is not UTF-8")?;
    check_depth(text)?;
    let family: RawIconThemeFamily = match serde_json::from_str(text) {
        Ok(family) => family,
        Err(_) => json5::from_str(text).map_err(|_| "Invalid icon theme JSON/JSONC")?,
    };
    if family.themes.is_empty() || family.themes.len() > THEME_LIMIT {
        return Err("Invalid icon theme count".into());
    }
    family
        .themes
        .into_iter()
        .map(|theme| {
            validate_name(&theme.name)?;
            if !matches!(theme.appearance.as_str(), "dark" | "light") {
                return Err("Invalid icon theme appearance".into());
            }
            Ok(ZedIconTheme {
                id: theme_id(extension, &theme.name),
                extension: extension.into(),
                name: theme.name,
                appearance: theme.appearance,
                directory_icons: theme.directory_icons,
                chevron_icons: theme.chevron_icons,
                named_directory_icons: theme.named_directory_icons,
                file_stems: theme.file_stems,
                file_suffixes: theme.file_suffixes,
                file_icons: theme.file_icons,
            })
        })
        .collect()
}

fn parse_archive(
    bytes: &[u8],
    id: &str,
    version: &str,
) -> Result<(IconRecord, BTreeMap<String, Vec<u8>>), String> {
    valid_id(id)?;
    semver::Version::parse(version).map_err(|_| "Invalid extension version")?;
    if bytes.len() > DOWNLOAD_LIMIT {
        return Err("Compressed archive exceeds limit".into());
    }
    let expanded = read_limited(MultiGzDecoder::new(bytes), EXPANDED_LIMIT)?;
    let mut archive = tar::Archive::new(expanded.as_slice());
    let mut paths = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut themes = Vec::new();
    let mut licenses = BTreeMap::new();
    let mut svg_files = BTreeMap::new();
    let mut retained = 0usize;

    for (index, entry) in archive
        .entries()
        .map_err(|e| e.to_string())?
        .raw(true)
        .enumerate()
    {
        if index >= ENTRY_LIMIT {
            return Err("Archive entry limit exceeded".into());
        }
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = archive_path(&entry.path_bytes())?;
        if !paths.insert(path.clone()) {
            return Err("Duplicate archive path".into());
        }
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() || path.is_empty() {
            return Err("Unsupported archive entry (links are forbidden)".into());
        }

        let is_json = path.ends_with(".json") || path.ends_with(".jsonc");
        let is_theme_json = is_json
            && (path.starts_with("icon_themes/")
                || path.starts_with("themes/")
                || !path.contains('/'));
        let is_svg = path.ends_with(".svg");
        let name = path.rsplit('/').next().unwrap_or("").to_ascii_uppercase();
        let license = ["LICENSE", "LICENCE", "COPYING", "NOTICE"].iter().any(|p| {
            name == *p || name.starts_with(&format!("{p}.")) || name.starts_with(&format!("{p}-"))
        });

        if !is_theme_json && !is_svg && !license {
            continue;
        }

        let size = entry.size();
        if size > FILE_LIMIT as u64 {
            return Err("Archive file exceeds limit".into());
        }
        retained += size as usize;
        if retained > EXPANDED_LIMIT {
            return Err("Retained icon theme data exceeds limit".into());
        }
        let data = read_limited(&mut entry, FILE_LIMIT)?;

        if is_theme_json {
            // Attempt to parse as an icon theme family
            if let Ok(parsed) = parse_icon_family(&data, id) {
                for theme in parsed {
                    if ids.insert(theme.id.clone()) {
                        themes.push(theme);
                    }
                }
            }
        } else if is_svg {
            // Validate SVG content header
            if let Ok(text) = std::str::from_utf8(&data)
                && text.contains("<svg")
            {
                svg_files.insert(path, data);
            }
        } else if license && let Ok(text) = std::str::from_utf8(&data) {
            licenses.insert(path, text.to_string());
        }
    }

    if themes.is_empty() {
        return Err("No valid icon theme definitions found in archive".into());
    }

    let record = IconRecord {
        extension: InstalledIconExtension {
            id: id.to_string(),
            version: version.to_string(),
            name: id.to_string(),
            description: String::new(),
            download_count: None,
            themes,
        },
        licenses,
    };

    Ok((record, svg_files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_id_rules() {
        assert!(valid_id("catppuccin-icons").is_ok());
        assert!(valid_id("material-icon-theme").is_ok());
        assert!(valid_id("vscode-icons").is_ok());
        assert!(valid_id("").is_err());
        assert!(valid_id("-bad").is_err());
        assert!(valid_id("UPPERCASE").is_err());
        assert!(valid_id("has spaces").is_err());
        assert!(valid_id("bad/slash").is_err());
    }

    #[test]
    fn theme_resolution_works() {
        let mut file_icons = BTreeMap::new();
        file_icons.insert(
            "docker".into(),
            FileIconDef {
                path: "./icons/docker.svg".into(),
            },
        );
        file_icons.insert(
            "settings".into(),
            FileIconDef {
                path: "./icons/settings.svg".into(),
            },
        );

        let mut named_directory_icons = BTreeMap::new();
        named_directory_icons.insert(
            "server".into(),
            DirectoryIcons {
                collapsed: Some("./icons/folder_server.svg".into()),
                expanded: Some("./icons/folder_server_open.svg".into()),
            },
        );

        let theme = ZedIconTheme {
            id: "test-icons/Dark".into(),
            extension: "test-icons".into(),
            name: "Dark".into(),
            appearance: "dark".into(),
            directory_icons: Some(DirectoryIcons {
                collapsed: Some("./icons/folder.svg".into()),
                expanded: Some("./icons/folder_open.svg".into()),
            }),
            chevron_icons: Some(ChevronIcons {
                collapsed: Some("./icons/chevron_right.svg".into()),
                expanded: Some("./icons/chevron_down.svg".into()),
            }),
            named_directory_icons,
            file_stems: BTreeMap::new(),
            file_suffixes: BTreeMap::new(),
            file_icons,
        };

        assert_eq!(
            theme.resolve_icon("chevron-right"),
            Some("icons/chevron_right.svg".into())
        );
        assert_eq!(
            theme.resolve_icon("icons/chevron-down.svg"),
            Some("icons/chevron_down.svg".into())
        );
        assert_eq!(
            theme.resolve_icon("icons/folder.svg"),
            Some("icons/folder.svg".into())
        );
        assert_eq!(
            theme.resolve_icon("icons/server.svg"),
            Some("icons/folder_server.svg".into())
        );
        assert_eq!(
            theme.resolve_icon("icons/container.svg"),
            Some("icons/docker.svg".into())
        );
        assert_eq!(
            theme.resolve_icon("icons/settings.svg"),
            Some("icons/settings.svg".into())
        );
        assert_eq!(theme.resolve_icon("icons/unknown_icon.svg"), None);
    }

    #[test]
    fn parse_archive_and_store_lifecycle() {
        use flate2::Compression;
        use flate2::write::GzEncoder;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = tar::Builder::new(&mut encoder);

            let theme_json = r#"{
                "name": "My Icons",
                "themes": [
                    {
                        "name": "My Dark",
                        "appearance": "dark",
                        "directory_icons": {
                            "collapsed": "./icons/folder.svg"
                        },
                        "file_icons": {
                            "yaml": { "path": "./icons/yaml.svg" }
                        }
                    }
                ]
            }"#;

            let mut header = tar::Header::new_gnu();
            header.set_size(theme_json.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(
                &mut header,
                "icon_themes/my-icons.json",
                theme_json.as_bytes(),
            )
            .unwrap();

            let svg_data = "<svg viewBox='0 0 16 16'><circle cx='8' cy='8' r='8'/></svg>";
            let mut header = tar::Header::new_gnu();
            header.set_size(svg_data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "icons/folder.svg", svg_data.as_bytes())
                .unwrap();

            tar.finish().unwrap();
        }
        let gz_bytes = encoder.finish().unwrap();

        let (record, svgs) = parse_archive(&gz_bytes, "test-icons", "1.0.0").unwrap();
        assert_eq!(record.extension.id, "test-icons");
        assert_eq!(record.extension.themes.len(), 1);
        assert_eq!(record.extension.themes[0].name, "My Dark");
        assert!(svgs.contains_key("icons/folder.svg"));

        let temp_dir = tempfile::tempdir().unwrap();
        let store_root = temp_dir.path().canonicalize().unwrap().join("zed_icons");
        checked_dir(&store_root, true).unwrap();

        publish(&store_root, &record, &svgs).unwrap();

        let list = installed_at(&store_root).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "test-icons");

        // Verify read file
        let read = fs::read(store_root.join("test-icons").join("icons/folder.svg")).unwrap();
        assert!(read.starts_with(b"<svg"));

        // Uninstall
        uninstall_at(&store_root, "test-icons").unwrap();
        let list_after = installed_at(&store_root).unwrap();
        assert!(list_after.is_empty());
    }

    #[test]
    fn read_svg_rejects_unsafe_paths() {
        assert!(read_svg("test-icons", "../escape.svg").is_err());
        assert!(read_svg("test-icons", "/etc/passwd").is_err());
        assert!(read_svg("test-icons", "icons/../bad.svg").is_err());
        assert!(read_svg("test-icons", "").is_err());
    }

    #[test]
    fn resolve_icon_maps_all_sidebar_and_action_aliases() {
        let mut theme = ZedIconTheme {
            id: "test-theme".into(),
            extension: "test-ext".into(),
            name: "Test Icons".into(),
            appearance: "dark".into(),
            directory_icons: None,
            chevron_icons: None,
            named_directory_icons: BTreeMap::new(),
            file_stems: BTreeMap::new(),
            file_suffixes: BTreeMap::new(),
            file_icons: BTreeMap::new(),
        };

        theme.file_icons.insert(
            "docker".into(),
            FileIconDef {
                path: "icons/docker.svg".into(),
            },
        );
        theme.file_icons.insert(
            "kubernetes".into(),
            FileIconDef {
                path: "icons/k8s.svg".into(),
            },
        );
        theme.file_icons.insert(
            "server".into(),
            FileIconDef {
                path: "icons/server.svg".into(),
            },
        );
        theme.file_icons.insert(
            "yaml".into(),
            FileIconDef {
                path: "icons/yaml.svg".into(),
            },
        );
        theme.file_icons.insert(
            "lock".into(),
            FileIconDef {
                path: "icons/lock.svg".into(),
            },
        );
        theme.file_icons.insert(
            "task".into(),
            FileIconDef {
                path: "icons/task.svg".into(),
            },
        );
        theme.file_icons.insert(
            "clock".into(),
            FileIconDef {
                path: "icons/clock.svg".into(),
            },
        );
        theme.file_icons.insert(
            "users".into(),
            FileIconDef {
                path: "icons/users.svg".into(),
            },
        );
        theme.file_icons.insert(
            "helm".into(),
            FileIconDef {
                path: "icons/helm.svg".into(),
            },
        );
        theme.file_icons.insert(
            "database".into(),
            FileIconDef {
                path: "icons/database.svg".into(),
            },
        );
        theme.file_icons.insert(
            "network".into(),
            FileIconDef {
                path: "icons/network.svg".into(),
            },
        );
        theme.file_icons.insert(
            "dashboard".into(),
            FileIconDef {
                path: "icons/dashboard.svg".into(),
            },
        );
        theme.file_icons.insert(
            "chart".into(),
            FileIconDef {
                path: "icons/chart.svg".into(),
            },
        );
        theme.file_icons.insert(
            "shield".into(),
            FileIconDef {
                path: "icons/shield.svg".into(),
            },
        );
        theme.file_icons.insert(
            "plugin".into(),
            FileIconDef {
                path: "icons/plugin.svg".into(),
            },
        );
        theme.file_icons.insert(
            "terminal".into(),
            FileIconDef {
                path: "icons/terminal.svg".into(),
            },
        );
        theme.file_icons.insert(
            "log".into(),
            FileIconDef {
                path: "icons/log.svg".into(),
            },
        );
        theme.file_icons.insert(
            "trash".into(),
            FileIconDef {
                path: "icons/trash.svg".into(),
            },
        );
        theme.file_icons.insert(
            "arrows".into(),
            FileIconDef {
                path: "icons/arrows.svg".into(),
            },
        );
        theme.file_icons.insert(
            "archive".into(),
            FileIconDef {
                path: "icons/archive.svg".into(),
            },
        );
        theme.file_icons.insert(
            "bell".into(),
            FileIconDef {
                path: "icons/bell.svg".into(),
            },
        );

        // Sidebar items
        assert_eq!(
            theme.resolve_icon("icons/layout-dashboard.svg").unwrap(),
            "icons/dashboard.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/server.svg").unwrap(),
            "icons/server.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/box.svg").unwrap(),
            "icons/docker.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/layers.svg").unwrap(),
            "icons/k8s.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/layers-2.svg").unwrap(),
            "icons/k8s.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/file-text.svg").unwrap(),
            "icons/yaml.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/key-round.svg").unwrap(),
            "icons/lock.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/list-checks.svg").unwrap(),
            "icons/task.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/calendar-clock.svg").unwrap(),
            "icons/clock.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/users.svg").unwrap(),
            "icons/users.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/package.svg").unwrap(),
            "icons/helm.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/database.svg").unwrap(),
            "icons/database.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/network.svg").unwrap(),
            "icons/network.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/chart-pie.svg").unwrap(),
            "icons/chart.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/shield-check.svg").unwrap(),
            "icons/shield.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/puzzle.svg").unwrap(),
            "icons/plugin.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/archive.svg").unwrap(),
            "icons/archive.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/bell.svg").unwrap(),
            "icons/bell.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/arrow-up-down.svg").unwrap(),
            "icons/arrows.svg"
        );

        // Action icons
        assert_eq!(
            theme.resolve_icon("icons/square-terminal.svg").unwrap(),
            "icons/terminal.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/logs.svg").unwrap(),
            "icons/log.svg"
        );
        assert_eq!(
            theme.resolve_icon("icons/trash.svg").unwrap(),
            "icons/trash.svg"
        );
        assert_eq!(
            theme
                .resolve_icon("icons/square-arrow-up-left-exit.svg")
                .unwrap(),
            "icons/terminal.svg"
        );
    }
}
