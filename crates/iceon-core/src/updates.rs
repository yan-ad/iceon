//! Automatic and manual update checking, background downloading, and restart lifecycle.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

pub const FEED_URL: &str = "https://github.com/yan-ad/iceon/releases/latest/download/latest.json";
pub const RELEASES_URL: &str = "https://github.com/yan-ad/iceon/releases/latest";
const FEED_LIMIT: usize = 256 * 1024;
const MAX_UPDATE_SIZE: u64 = 500 * 1024 * 1024; // 500MB

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    pub current: String,
    pub latest: String,
    pub available: bool,
    pub download_url: Option<String>,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadedUpdate {
    pub version: String,
    pub path: PathBuf,
    pub downloaded_at: u64,
}

#[derive(Deserialize)]
struct Feed {
    version: String,
    #[serde(default)]
    platforms: HashMap<String, PlatformFeed>,
}

#[derive(Deserialize)]
struct PlatformFeed {
    url: String,
    #[serde(default)]
    signature: Option<String>,
}

#[allow(unreachable_code)]
pub fn current_platform_keys() -> &'static [&'static str] {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return &["darwin-aarch64", "darwin-aarch64-app", "darwin-arm64"];
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return &["darwin-x86_64", "darwin-x86_64-app"];
    #[cfg(all(target_os = "windows", target_arch = "x86"))]
    return &["windows-i686", "windows-i686-nsis"];
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return &["windows-x86_64", "windows-x86_64-nsis"];
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return &["linux-x86_64", "linux-amd64"];
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return &["linux-aarch64", "linux-arm64"];
    &["unknown"]
}

#[allow(unreachable_code)]
pub fn fallback_asset_name(version: &str) -> String {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return format!("Iceon_{version}_aarch64.app.tar.gz");
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return format!("Iceon_{version}_x86_64.app.tar.gz");
    #[cfg(all(target_os = "windows", target_arch = "x86"))]
    return format!("Iceon_{version}_x86-setup.exe");
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return format!("Iceon_{version}_x64-setup.exe");
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return format!("Iceon_{version}_amd64.tar.gz");
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return format!("Iceon_{version}_arm64.tar.gz");
    format!("Iceon_{version}.tar.gz")
}

pub fn updates_dir() -> PathBuf {
    crate::runtime::app_data_dir().join("updates")
}

pub fn update_marker_path() -> PathBuf {
    updates_dir().join("ready_update.json")
}

pub fn read_downloaded_update() -> Option<DownloadedUpdate> {
    let marker = update_marker_path();
    let bytes = fs::read(marker).ok()?;
    let update: DownloadedUpdate = serde_json::from_slice(&bytes).ok()?;
    if update.path.exists() {
        Some(update)
    } else {
        None
    }
}

pub fn save_downloaded_update(update: &DownloadedUpdate) -> Result<(), String> {
    let marker = update_marker_path();
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_vec_pretty(update).map_err(|e| e.to_string())?;
    fs::write(marker, data).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn is_newer_version(candidate: &str, current: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(current),
    ) {
        (Ok(cand), Ok(curr)) => cand > curr,
        _ => false,
    }
}

pub fn has_ready_update(current_version: &str) -> Option<DownloadedUpdate> {
    if let Some(downloaded) = read_downloaded_update() {
        if is_newer_version(&downloaded.version, current_version) {
            return Some(downloaded);
        } else {
            clear_downloaded_update();
        }
    }
    None
}

pub fn clear_downloaded_update() {
    let _ = fs::remove_file(update_marker_path());
}

fn parse(bytes: &[u8], current: &str) -> Result<UpdateCheck, String> {
    let feed: Feed =
        serde_json::from_slice(bytes).map_err(|_| "The update feed is invalid.".to_owned())?;
    let latest = semver::Version::parse(&feed.version)
        .map_err(|_| "The release version is invalid.".to_owned())?;
    let current = semver::Version::parse(current)
        .map_err(|_| "The installed version is invalid.".to_owned())?;
    if !latest.pre.is_empty() {
        return Err("The stable update feed contains a prerelease.".into());
    }
    let available = latest > current;
    let mut download_url = None;
    let mut signature = None;

    for key in current_platform_keys() {
        if let Some(platform) = feed.platforms.get(*key) {
            download_url = Some(platform.url.clone());
            signature = platform.signature.clone();
            break;
        }
    }

    if download_url.is_none() && available {
        let asset = fallback_asset_name(&latest.to_string());
        download_url = Some(format!(
            "https://github.com/yan-ad/iceon/releases/download/{latest}/{asset}"
        ));
    }

    Ok(UpdateCheck {
        available,
        latest: latest.to_string(),
        current: current.to_string(),
        download_url,
        signature,
    })
}

pub async fn check() -> Result<UpdateCheck, String> {
    check_with_url(FEED_URL).await
}

pub async fn check_with_url(feed_url: &str) -> Result<UpdateCheck, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("Iceon/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= 5
                || url.scheme() != "https"
                || !matches!(
                    url.host_str(),
                    Some(
                        "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "Could not initialize update checking.".to_owned())?;
    let mut response = client
        .get(feed_url)
        .send()
        .await
        .map_err(|_| "Could not reach GitHub. Check your connection and try again.".to_owned())?
        .error_for_status()
        .map_err(|_| "The update feed is unavailable. Please try again later.".to_owned())?;
    if response
        .content_length()
        .is_some_and(|size| size > FEED_LIMIT as u64)
    {
        return Err("The update feed exceeds the size limit.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the update feed.".to_owned())?
    {
        if bytes.len() + chunk.len() > FEED_LIMIT {
            return Err("The update feed exceeds the size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    parse(&bytes, env!("CARGO_PKG_VERSION"))
}

pub async fn download_release(url: &str, destination: &Path) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent(concat!("Iceon/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= 5
                || url.scheme() != "https"
                || !matches!(
                    url.host_str(),
                    Some(
                        "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "Could not initialize update download.".to_owned())?;

    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Could not download update: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Update download failed: {e}"))?;

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create updates directory: {e}"))?;
    }

    let temp_dest = destination.with_extension("part");
    let mut file = fs::File::create(&temp_dest)
        .map_err(|e| format!("Could not create temporary update file: {e}"))?;

    let mut total_bytes: u64 = 0;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Error while downloading update stream: {e}"))?
    {
        total_bytes += chunk.len() as u64;
        if total_bytes > MAX_UPDATE_SIZE {
            let _ = fs::remove_file(&temp_dest);
            return Err("Downloaded update exceeds maximum allowed size.".to_string());
        }
        use std::io::Write;
        file.write_all(&chunk)
            .map_err(|e| format!("Failed writing update file: {e}"))?;
    }

    use std::io::Write;
    file.flush()
        .map_err(|e| format!("Failed flushing update file: {e}"))?;
    drop(file);

    fs::rename(&temp_dest, destination)
        .map_err(|e| format!("Failed finalizing update file: {e}"))?;

    Ok(total_bytes)
}

pub fn extract_tar_gz(archive_path: &Path, extract_to: &Path) -> Result<(), String> {
    let file =
        fs::File::open(archive_path).map_err(|e| format!("Failed to open update archive: {e}"))?;
    let tar = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(tar);
    archive
        .unpack(extract_to)
        .map_err(|e| format!("Failed to unpack update archive: {e}"))?;
    Ok(())
}

pub async fn check_and_download(current_version: &str) -> Result<Option<DownloadedUpdate>, String> {
    if let Some(existing) = read_downloaded_update() {
        if let (Ok(existing_v), Ok(curr_v)) = (
            semver::Version::parse(&existing.version),
            semver::Version::parse(current_version),
        ) {
            if existing_v > curr_v && existing.path.exists() {
                return Ok(Some(existing));
            } else if existing_v <= curr_v {
                clear_downloaded_update();
            }
        }
    }

    let check_res = check().await?;
    if !check_res.available {
        return Ok(None);
    }

    let Some(download_url) = check_res.download_url else {
        return Err("No download URL found for this platform.".into());
    };

    let filename = download_url.rsplit('/').next().unwrap_or("update_payload");
    let version_dir = updates_dir().join(format!("v{}", check_res.latest));
    let payload_path = version_dir.join(filename);

    download_release(&download_url, &payload_path).await?;

    let mut target_path = payload_path.clone();

    if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
        let extract_dir = version_dir.join("extracted");
        let _ = fs::create_dir_all(&extract_dir);
        if let Ok(()) = extract_tar_gz(&payload_path, &extract_dir) {
            #[cfg(target_os = "macos")]
            {
                let app_path = extract_dir.join("Iceon.app");
                if app_path.exists() {
                    target_path = app_path;
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let bin_path = extract_dir.join("iceon-app");
                if bin_path.exists() {
                    target_path = bin_path;
                }
            }
        }
    }

    let downloaded = DownloadedUpdate {
        version: check_res.latest,
        path: target_path,
        downloaded_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    save_downloaded_update(&downloaded)?;
    Ok(Some(downloaded))
}

pub fn restart_with_update(target_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if target_path.extension().is_some_and(|ext| ext == "app")
            || target_path.join("Contents").exists()
        {
            std::process::Command::new("/usr/bin/open")
                .arg("-n")
                .arg(target_path)
                .spawn()
                .map_err(|e| format!("Failed to launch updated app: {e}"))?;
            return Ok(());
        }
    }

    #[cfg(target_os = "windows")]
    {
        if target_path.extension().is_some_and(|ext| ext == "exe") {
            std::process::Command::new(target_path)
                .spawn()
                .map_err(|e| format!("Failed to launch updated installer: {e}"))?;
            return Ok(());
        }
    }

    if target_path.is_file() {
        std::process::Command::new(target_path)
            .spawn()
            .map_err(|e| format!("Failed to spawn updated binary: {e}"))?;
        return Ok(());
    }

    if let Ok(current_exe) = std::env::current_exe() {
        std::process::Command::new(current_exe)
            .spawn()
            .map_err(|e| format!("Failed to relaunch current executable: {e}"))?;
        return Ok(());
    }

    Err("Could not determine executable to launch for update.".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_versions_not_strings() {
        assert!(
            parse(br#"{"version":"0.1.10"}"#, "0.1.9")
                .unwrap()
                .available
        );
        assert!(!parse(br#"{"version":"0.1.3"}"#, "0.1.3").unwrap().available);
        assert!(!parse(br#"{"version":"0.1.2"}"#, "0.1.3").unwrap().available);
    }
    #[test]
    fn rejects_invalid_and_prerelease_feeds() {
        for feed in [
            br#"{}"#.as_slice(),
            br#"{"version":"bad"}"#,
            br#"{"version":"0.2.0-beta.1"}"#,
        ] {
            assert!(parse(feed, "0.1.3").is_err());
        }
    }

    #[test]
    fn parses_platform_download_url() {
        let feed = br#"{
            "version": "0.1.7",
            "platforms": {
                "darwin-aarch64": {
                    "url": "https://github.com/yan-ad/iceon/releases/download/0.1.7/Iceon_0.1.7_aarch64.app.tar.gz",
                    "signature": "sig123"
                },
                "windows-i686": {
                    "url": "https://github.com/yan-ad/iceon/releases/download/0.1.7/Iceon_0.1.7_x86-setup.exe",
                    "signature": "sigwin"
                }
            }
        }"#;
        let check = parse(feed, "0.1.6").unwrap();
        assert!(check.available);
        assert_eq!(check.latest, "0.1.7");
        assert!(check.download_url.is_some());
    }

    #[test]
    fn downloaded_update_serialization() {
        let temp = tempfile::tempdir().unwrap();
        let target_file = temp.path().join("dummy-update.bin");
        std::fs::write(&target_file, b"content").unwrap();

        let update = DownloadedUpdate {
            version: "0.1.7".into(),
            path: target_file.clone(),
            downloaded_at: 1234567,
        };
        let encoded = serde_json::to_vec(&update).unwrap();
        let decoded: DownloadedUpdate = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.version, "0.1.7");
        assert_eq!(decoded.path, target_file);
        assert_eq!(decoded.downloaded_at, 1234567);
    }

    #[test]
    fn version_comparison_logic() {
        assert!(is_newer_version("0.1.7", "0.1.6"));
        assert!(!is_newer_version("0.1.6", "0.1.6"));
        assert!(!is_newer_version("0.1.5", "0.1.6"));
        assert!(!is_newer_version("bad", "0.1.6"));
    }
}
