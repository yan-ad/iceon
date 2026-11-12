//! Data-only importer for Zed marketplace themes. No extension code is extracted or run.
//!
//! The marketplace lists versions at `/extensions/{id}` and redirects downloads to
//! Zed's extension object store. Both response bodies and decompression are bounded.
//! Only normalized theme data and license text survive import. The store publishes a
//! single JSON record by atomic rename, so a failed update leaves the old record intact.

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
use serde_json::Value;

const API: &str = "https://api.zed.dev";
const METADATA_LIMIT: usize = 2 * 1024 * 1024;
const DOWNLOAD_LIMIT: usize = 16 * 1024 * 1024;
const EXPANDED_LIMIT: usize = 64 * 1024 * 1024;
const FILE_LIMIT: usize = 4 * 1024 * 1024;
const RECORD_LIMIT: usize = 24 * 1024 * 1024;
const ENTRY_LIMIT: usize = 2048;
const THEME_LIMIT: usize = 512;
const INSTALLED_LIMIT: usize = 256;
const DEPTH_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZedTheme {
    pub id: String,
    pub extension: String,
    pub name: String,
    pub appearance: String,
    pub style: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstalledExtension {
    pub id: String,
    pub version: String,
    pub themes: Vec<ZedTheme>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    extension: InstalledExtension,
    // Retained as data, not copied to executable or user-controlled filesystem paths.
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
    crate::runtime::app_data_dir().join("themes").join("zed")
}

// Check every ancestor, including the configured app-data directory. In particular,
// create_dir_all must never follow an existing symlink into another tree.
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
        return Err("Theme data exceeds size limit".into());
    }
    Ok(bytes)
}

fn read_record(root: &Path, id: &str) -> Result<Record, String> {
    valid_id(id)?;
    let dir = root.join(id);
    checked_dir(&dir, false)?;
    let bytes = read_limited(
        open_regular(&dir.join("installed.json"), false)?,
        RECORD_LIMIT,
    )?;
    let record: Record = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    validate_extension(&record.extension, id)?;
    Ok(record)
}

fn validate_extension(extension: &InstalledExtension, id: &str) -> Result<(), String> {
    if extension.id != id
        || semver::Version::parse(&extension.version).is_err()
        || extension.themes.is_empty()
        || extension.themes.len() > THEME_LIMIT
    {
        return Err("Invalid installed extension record".into());
    }
    let mut ids = BTreeSet::new();
    for theme in &extension.themes {
        validate_name(&theme.name)?;
        if theme.extension != id
            || theme.id != theme_id(id, &theme.name)
            || !matches!(theme.appearance.as_str(), "light" | "dark")
            || !ids.insert(&theme.id)
        {
            return Err("Invalid or duplicate installed theme id".into());
        }
    }
    Ok(())
}

/// List verified local records. Corrupt or unsafe records fail closed rather than
/// returning a misleading partial inventory. Ordering is deterministic.
pub fn installed() -> Result<Vec<InstalledExtension>, String> {
    installed_at(&root())
}

fn installed_at(root: &Path) -> Result<Vec<InstalledExtension>, String> {
    // Do not create an empty store just to list it.
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
            // .lock and in-progress RAII staging directories are never themes.
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

/// Find a namespaced, stable theme id from the local data-only store.
pub fn find_theme(id: &str) -> Option<ZedTheme> {
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

/// Remove only the normalized record, never recursively delete an untrusted tree.
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
    // Reject unexpected children before doing anything; never follow symlinks.
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name() != "installed.json"
            || !entry.file_type().map_err(|e| e.to_string())?.is_file()
        {
            return Err("Unexpected file in installed extension".into());
        }
    }
    let record = dir.join("installed.json");
    if record.try_exists().map_err(|e| e.to_string())? {
        let _file = open_regular(&record, false)?;
        fs::remove_file(record).map_err(|e| e.to_string())?;
    }
    fs::remove_dir(dir).map_err(|e| e.to_string())
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
    // Do not include reqwest errors/URLs: object-store query strings are signed.
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

/// Safe display-only metadata. No repository URLs or extension code are consumed.
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
        .append_pair("provides", "themes");
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
            || !entry.provides.iter().any(|kind| kind == "themes")
        {
            return Err("Invalid marketplace theme metadata".into());
        }
        extensions.push(item);
    }
    Ok(extensions)
}

/// Browse the theme-only marketplace using its supported `filter` search.
/// The endpoint returns a full list (page parameters are ignored); callers cache
/// this bounded result and paginate locally. Run on Tokio, not a UI executor.
pub async fn marketplace(query: &str) -> Result<Vec<MarketplaceExtension>, String> {
    let url = marketplace_url(query)?;
    // Browse has a separate no-redirect policy: cloud.zed.dev is NOT added to
    // the install/download allowlist and metadata cannot supply network targets.
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

/// Install the highest semantic version advertised by Zed. Network work happens
/// before acquiring the store lock; parsing/publication runs off the async executor.
pub async fn install(id: &str) -> Result<InstalledExtension, String> {
    valid_id(id)?;
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
        let record = parse_archive(&archive, &id, &version)?;
        publish(&root, &record)?;
        Ok(record.extension)
    })
    .await
    .map_err(|_| "Theme installation worker failed".to_string())?
}

fn publish(root: &Path, record: &Record) -> Result<(), String> {
    valid_id(&record.extension.id)?;
    validate_extension(&record.extension, &record.extension.id)?;
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    if bytes.len() > RECORD_LIMIT {
        return Err("Installed theme record exceeds size limit".into());
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
    let dir = root.join(&record.extension.id);
    checked_dir(&dir, true)?;
    let destination = dir.join("installed.json");
    if fs::symlink_metadata(&destination).is_ok() {
        let _existing = open_regular(&destination, false)?;
    }
    // Staging and final destination share a filesystem; only the record is renamed.
    let mut stage = tempfile::Builder::new()
        .prefix(".stage-")
        .tempfile_in(root)
        .map_err(|e| e.to_string())?;
    stage.write_all(&bytes).map_err(|e| e.to_string())?;
    stage.as_file().sync_all().map_err(|e| e.to_string())?;
    stage
        .persist(&destination)
        .map_err(|e| e.error.to_string())?;
    #[cfg(unix)]
    File::open(&dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
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
        return Err("Invalid theme name".into());
    }
    Ok(())
}

// Bound nesting before invoking JSON5's recursive parser, including comments and
// quoted strings. serde_json also retains its own recursion limit.
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
                return Err("Theme JSON nesting exceeds limit".into());
            }
        } else if matches!(c, '}' | ']') {
            depth = depth.saturating_sub(1);
        }
    }
    Ok(())
}

fn parse_family(bytes: &[u8], extension: &str) -> Result<Vec<ZedTheme>, String> {
    #[derive(Deserialize)]
    struct Family {
        themes: Vec<Theme>,
    }
    #[derive(Deserialize)]
    struct Theme {
        name: String,
        appearance: String,
        style: BTreeMap<String, Value>,
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "Theme is not UTF-8")?;
    check_depth(text)?;
    let family: Family = match serde_json::from_str(text) {
        Ok(family) => family,
        Err(_) => json5::from_str(text).map_err(|_| "Invalid theme JSON/JSONC")?,
    };
    if family.themes.is_empty() || family.themes.len() > THEME_LIMIT {
        return Err("Invalid theme count".into());
    }
    family
        .themes
        .into_iter()
        .map(|theme| {
            validate_name(&theme.name)?;
            if !matches!(theme.appearance.as_str(), "dark" | "light") {
                return Err("Invalid theme appearance".into());
            }
            Ok(ZedTheme {
                id: theme_id(extension, &theme.name),
                extension: extension.into(),
                name: theme.name,
                appearance: theme.appearance,
                style: theme.style,
            })
        })
        .collect()
}

fn parse_archive(bytes: &[u8], id: &str, version: &str) -> Result<Record, String> {
    valid_id(id)?;
    semver::Version::parse(version).map_err(|_| "Invalid extension version")?;
    if bytes.len() > DOWNLOAD_LIMIT {
        return Err("Compressed archive exceeds limit".into());
    }
    // Read through EOF to validate the gzip checksum, even when tar has early EOF.
    let expanded = read_limited(MultiGzDecoder::new(bytes), EXPANDED_LIMIT)?;
    let mut archive = tar::Archive::new(expanded.as_slice());
    let mut paths = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut themes = Vec::new();
    let mut licenses = BTreeMap::new();
    let mut retained = 0usize;
    // Raw iteration rejects PAX/GNU extension headers rather than letting them
    // override a path, size or link target that was already validated.
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
        let theme =
            path.starts_with("themes/") && (path.ends_with(".json") || path.ends_with(".jsonc"));
        let name = path.rsplit('/').next().unwrap_or("").to_ascii_uppercase();
        let license = ["LICENSE", "LICENCE", "COPYING", "NOTICE"].iter().any(|p| {
            name == *p || name.starts_with(&format!("{p}.")) || name.starts_with(&format!("{p}-"))
        });
        if !theme && !license {
            continue;
        }
        let size = entry.size();
        if size > FILE_LIMIT as u64 {
            return Err("Archive file exceeds limit".into());
        }
        retained += size as usize;
        if retained > DOWNLOAD_LIMIT {
            return Err("Retained theme data exceeds limit".into());
        }
        let data = read_limited(&mut entry, FILE_LIMIT)?;
        if theme {
            for theme in parse_family(&data, id)? {
                if !ids.insert(theme.id.clone()) {
                    return Err("Duplicate theme id".into());
                }
                themes.push(theme);
                if themes.len() > THEME_LIMIT {
                    return Err("Theme count exceeds limit".into());
                }
            }
        } else {
            licenses.insert(
                path,
                String::from_utf8(data).map_err(|_| "License is not UTF-8")?,
            );
        }
    }
    themes.sort_by(|a, b| a.id.cmp(&b.id));
    let extension = InstalledExtension {
        id: id.into(),
        version: version.into(),
        themes,
    };
    validate_extension(&extension, id)?;
    Ok(Record {
        extension,
        licenses,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};

    const FAMILY: &str = r##"{"themes":[{"name":"Ocean Dark","appearance":"dark","style":{"background":"#123456","syntax":{"keyword":{"color":"#abcdef"}}}}]}"##;

    fn archive(files: &[(&str, &[u8], tar::EntryType)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, data, kind) in files {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, *data).unwrap();
        }
        gzip(&builder.into_inner().unwrap())
    }
    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }
    fn sample() -> Record {
        parse_archive(
            &archive(&[(
                "./themes/ocean.json",
                FAMILY.as_bytes(),
                tar::EntryType::Regular,
            )]),
            "ocean",
            "1.0.0",
        )
        .unwrap()
    }

    #[test]
    fn marketplace_search_is_encoded_and_bounded() {
        let url = marketplace_url(" github & dark ").unwrap();
        assert_eq!(url.host_str(), Some("cloud.zed.dev"));
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "filter").unwrap().1,
            "github & dark"
        );
        assert!(marketplace_url(&"x".repeat(257)).is_err());
        assert!(marketplace_url("a\nb").is_err());
        assert!(!allowed_url(&url)); // browse cannot broaden install hosts
    }

    #[test]
    fn marketplace_metadata_is_validated() {
        let fixture = br#"{"data":[{"id":"github-theme","name":"GitHub","description":"Themes","download_count":42,"provides":["themes"],"repository":"https://ignored.test"}]}"#;
        let entries = parse_marketplace(fixture).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].download_count, 42);
        for (from, to) in [
            ("github-theme", "../unsafe"),
            ("[\"themes\"]", "[\"languages\"]"),
        ] {
            let invalid = String::from_utf8(fixture.to_vec())
                .unwrap()
                .replace(from, to);
            assert!(parse_marketplace(invalid.as_bytes()).is_err());
        }
        assert!(parse_marketplace(&vec![b' '; METADATA_LIMIT + 1]).is_err());
        assert!(parse_marketplace(br#"{"data":null}"#).is_err());
    }

    #[test]
    fn parses_jsonc_and_preserves_style() {
        let jsonc = FAMILY
            .replace("\"themes\":", "// comment\n\"themes\":")
            .replace("\"#123456\",", "\"#123456\", /* comment */");
        let themes = parse_family(jsonc.as_bytes(), "ocean").unwrap();
        assert_eq!(themes[0].id, "ocean/Ocean%20Dark");
        assert_eq!(themes[0].style["syntax"]["keyword"]["color"], "#abcdef");
        assert!(parse_family(b"{themes: []}", "ocean").is_err());
        assert!(parse_family(FAMILY.replace("dark", "unknown").as_bytes(), "ocean").is_err());
    }

    #[test]
    fn semver_order_and_identity() {
        let listing = br#"{"data":[{"id":"ocean","version":"1.9.0"},{"id":"ocean","version":"1.10.0"},{"id":"ocean","version":"1.10.0-rc.1"}]}"#;
        assert_eq!(latest_version(listing, "ocean").unwrap(), "1.10.0");
        assert!(latest_version(listing, "other").is_err());
        assert!(latest_version(br#"{"data":[]}"#, "ocean").is_err());
    }

    #[test]
    fn archive_retains_only_data_and_licenses() {
        let data = archive(&[
            (
                "themes/ocean.json",
                FAMILY.as_bytes(),
                tar::EntryType::Regular,
            ),
            ("extension.wasm", b"not executable", tar::EntryType::Regular),
            ("LICENSE", b"MIT", tar::EntryType::Regular),
        ]);
        let record = parse_archive(&data, "ocean", "1.0.0").unwrap();
        assert_eq!(record.extension.themes.len(), 1);
        assert_eq!(record.licenses["LICENSE"], "MIT");
    }

    #[test]
    fn rejects_unsafe_paths_and_links() {
        for path in [
            "../escape",
            "/absolute",
            "themes/../../escape",
            "C:/escape",
            "themes\\evil",
            "themes//x",
            "././themes/x",
            "themes/./x",
        ] {
            assert!(archive_path(path.as_bytes()).is_err(), "{path}");
        }
        for kind in [
            tar::EntryType::Symlink,
            tar::EntryType::Link,
            tar::EntryType::Fifo,
            tar::EntryType::GNULongName,
            tar::EntryType::XHeader,
        ] {
            assert!(parse_archive(&archive(&[("evil", b"", kind)]), "ocean", "1.0.0").is_err());
        }
        // Construct raw malicious headers because tar::Builder rightly refuses ../.
        let mut header = tar::Header::new_gnu();
        header.as_mut_bytes()[..9].copy_from_slice(b"../escape");
        header.set_size(0);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        let mut bytes = header.as_bytes().to_vec();
        bytes.extend_from_slice(&[0; 1024]);
        assert!(parse_archive(&gzip(&bytes), "ocean", "1.0.0").is_err());
    }

    #[test]
    fn rejects_truncated_and_corrupt_gzip() {
        let bytes = archive(&[(
            "themes/ocean.json",
            FAMILY.as_bytes(),
            tar::EntryType::Regular,
        )]);
        assert!(parse_archive(&bytes[..bytes.len() - 4], "ocean", "1.0.0").is_err());
        let mut corrupt = bytes.clone();
        let offset = corrupt.len() - 8;
        corrupt[offset] ^= 1;
        assert!(parse_archive(&corrupt, "ocean", "1.0.0").is_err());
        let mut trailing = bytes;
        trailing.extend_from_slice(b"not another gzip member");
        assert!(parse_archive(&trailing, "ocean", "1.0.0").is_err());
    }

    #[test]
    fn rejects_duplicate_paths_and_theme_ids() {
        let data = archive(&[
            ("themes/a.json", FAMILY.as_bytes(), tar::EntryType::Regular),
            ("themes/b.json", FAMILY.as_bytes(), tar::EntryType::Regular),
        ]);
        assert!(
            parse_archive(&data, "ocean", "1.0.0")
                .unwrap_err()
                .contains("Duplicate theme")
        );
        let data = archive(&[
            ("themes/a.json", FAMILY.as_bytes(), tar::EntryType::Regular),
            (
                "./themes/a.json",
                FAMILY.as_bytes(),
                tar::EntryType::Regular,
            ),
        ]);
        assert!(parse_archive(&data, "ocean", "1.0.0").is_err());
    }

    #[test]
    fn enforces_size_entry_and_depth_limits() {
        assert!(read_limited(&b"12345"[..], 4).is_err());
        assert!(parse_archive(&vec![0; DOWNLOAD_LIMIT + 1], "ocean", "1.0.0").is_err());
        let large = vec![b' '; FILE_LIMIT + 1];
        assert!(
            parse_archive(
                &archive(&[("themes/a.json", &large, tar::EntryType::Regular)]),
                "ocean",
                "1.0.0"
            )
            .is_err()
        );
        let mut files = Vec::new();
        let paths: Vec<_> = (0..=ENTRY_LIMIT).map(|i| format!("ignored-{i}")).collect();
        for path in &paths {
            files.push((path.as_str(), b"".as_slice(), tar::EntryType::Regular));
        }
        assert!(parse_archive(&archive(&files), "ocean", "1.0.0").is_err());
        assert!(
            parse_family(
                format!(
                    "{}{}",
                    "[".repeat(DEPTH_LIMIT + 1),
                    "]".repeat(DEPTH_LIMIT + 1)
                )
                .as_bytes(),
                "ocean"
            )
            .is_err()
        );
        assert!(check_depth(r#"{/* [[[[ */ "a": "[[[["}"#).is_ok());
        let bomb = gzip(&vec![0; EXPANDED_LIMIT + 1]);
        assert!(parse_archive(&bomb, "ocean", "1.0.0").is_err());
    }

    #[test]
    fn store_roundtrip_atomic_update_and_uninstall() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("zed");
        assert!(installed_at(&root).unwrap().is_empty());
        let mut record = sample();
        publish(&root, &record).unwrap();
        assert_eq!(installed_at(&root).unwrap(), vec![record.extension.clone()]);
        record.extension.version = "2.0.0".into();
        publish(&root, &record).unwrap();
        assert_eq!(installed_at(&root).unwrap()[0].version, "2.0.0");
        record.extension.themes[0].id = "wrong".into();
        assert!(publish(&root, &record).is_err());
        assert_eq!(installed_at(&root).unwrap()[0].version, "2.0.0");
        uninstall_at(&root, "ocean").unwrap();
        uninstall_at(&root, "ocean").unwrap();
        assert!(installed_at(&root).unwrap().is_empty());
        for id in ["../ocean", "", "/tmp", "ocean/x", "UPPER", "a.b"] {
            assert!(valid_id(id).is_err());
        }
    }

    #[test]
    fn redirect_policy_is_exact_https_allowlist() {
        for url in [
            "http://api.zed.dev/extensions/a",
            "https://api.zed.dev.evil.test/a",
            "https://user@api.zed.dev/a",
            "https://api.zed.dev:444/a",
            "https://127.0.0.1/a",
            "https://evil.test/a",
        ] {
            assert!(!allowed_url(&url::Url::parse(url).unwrap()), "{url}");
        }
        assert!(allowed_url(&url::Url::parse("https://zed-extensions.nyc3.digitaloceanspaces.com/extensions/a/1.0.0/archive.tar.gz?signature=x").unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_filesystem_symlinks_without_touching_targets() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("zed");
        let outside = temp.path().canonicalize().unwrap().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, &root).unwrap();
        assert!(publish(&root, &sample()).is_err());
        fs::remove_file(&root).unwrap();
        fs::create_dir(&root).unwrap();
        symlink(&outside, root.join("ocean")).unwrap();
        assert!(publish(&root, &sample()).is_err());
        assert!(uninstall_at(&root, "ocean").is_err());
        fs::remove_file(root.join("ocean")).unwrap();
        fs::create_dir(root.join("ocean")).unwrap();
        let target = outside.join("target");
        fs::write(&target, b"untouched").unwrap();
        symlink(&target, root.join("ocean/installed.json")).unwrap();
        assert!(publish(&root, &sample()).is_err());
        assert!(installed_at(&root).is_err());
        assert!(uninstall_at(&root, "ocean").is_err());
        assert_eq!(fs::read(target).unwrap(), b"untouched");
    }
}
