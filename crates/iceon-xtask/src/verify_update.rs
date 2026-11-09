use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use std::{
    fmt,
    fs::{self, File, Metadata},
    io::Read,
    path::{Component, Path, PathBuf},
};

const CONFIG_LIMIT: u64 = 256 * 1024;
const SIGNATURE_LIMIT: u64 = 16 * 1024;
const PAYLOAD_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Version,
    File(&'static str),
    Symlink,
    NotRegular,
    Empty,
    TooLarge,
    Changed,
    Config,
    PublicKey,
    Signature,
    Authentication,
    MissingVersion,
    DuplicateVersion,
    SignedVersion,
    VersionMismatch,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Version => "expected canonical stable X.Y.Z release version",
            Self::File(operation) => return write!(f, "update input file {operation} failed"),
            Self::Symlink => "update input symlink paths are not supported",
            Self::NotRegular => "update inputs must be regular files",
            Self::Empty => "update inputs must not be empty",
            Self::TooLarge => "update input exceeds its size limit",
            Self::Changed => "update input changed while being read",
            Self::Config => "invalid updater JSON config or missing updater pubkey",
            Self::PublicKey => "invalid base64 Tauri minisign public key",
            Self::Signature => "invalid base64 Tauri minisign signature",
            Self::Authentication => "update signature authentication failed",
            Self::MissingVersion => "authenticated signature has no version field",
            Self::DuplicateVersion => "authenticated signature has duplicate version fields",
            Self::SignedVersion => "authenticated signature version is not canonical stable X.Y.Z",
            Self::VersionMismatch => {
                "authenticated signature version does not match release version"
            }
        };
        f.write_str(message)
    }
}

impl std::error::Error for Error {}

#[derive(Deserialize)]
struct Config {
    plugins: Plugins,
}
#[derive(Deserialize)]
struct Plugins {
    updater: Updater,
}
#[derive(Deserialize)]
struct Updater {
    pubkey: String,
}

pub fn run(config: &Path, signature: &Path, payload: &Path, version: &str) -> Result<(), Error> {
    let version = super::stable(version).map_err(|_| Error::Version)?;
    let config: Config =
        serde_json::from_slice(&read_bounded(config, CONFIG_LIMIT)?).map_err(|_| Error::Config)?;
    let key_text =
        decode(config.plugins.updater.pubkey.as_bytes()).map_err(|_| Error::PublicKey)?;
    if key_text.lines().count() != 2 {
        return Err(Error::PublicKey);
    }
    let key = PublicKey::decode(&key_text).map_err(|_| Error::PublicKey)?;
    let signature_text =
        decode(&read_bounded(signature, SIGNATURE_LIMIT)?).map_err(|_| Error::Signature)?;
    if signature_text.lines().count() != 4 {
        return Err(Error::Signature);
    }
    let signature = Signature::decode(&signature_text).map_err(|_| Error::Signature)?;
    let data = read_bounded(payload, PAYLOAD_LIMIT)?;
    // Matches tauri-plugin-updater 2.14.0 verify_signature, including the global comment signature.
    key.verify(&data, &signature, true)
        .map_err(|_| Error::Authentication)?;
    signed_version(signature.trusted_comment(), &version)
}

fn decode(encoded: &[u8]) -> Result<String, ()> {
    let encoded = std::str::from_utf8(encoded).map_err(|_| ())?.trim();
    let bytes = STANDARD.decode(encoded).map_err(|_| ())?;
    String::from_utf8(bytes).map_err(|_| ())
}

fn signed_version(comment: &str, expected: &semver::Version) -> Result<(), Error> {
    // Tauri uses tab-separated fields with the exact prefix `version:`. Reject duplicates
    // and noncanonical versions rather than taking its first-match / v-prefix fallback.
    let mut versions = comment
        .split('\t')
        .filter_map(|s| s.strip_prefix("version:"));
    let text = versions.next().ok_or(Error::MissingVersion)?;
    if versions.next().is_some() {
        return Err(Error::DuplicateVersion);
    }
    let signed = super::stable(text).map_err(|_| Error::SignedVersion)?;
    if &signed != expected {
        return Err(Error::VersionMismatch);
    }
    Ok(())
}

fn checked_path(path: &Path) -> Result<PathBuf, Error> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| Error::File("working directory"))?
            .join(path)
    };
    let mut walked = PathBuf::new();
    for component in absolute.components() {
        walked.push(component);
        if matches!(component, Component::Normal(_) | Component::ParentDir) {
            let metadata = fs::symlink_metadata(&walked).map_err(|_| Error::File("metadata"))?;
            if metadata.file_type().is_symlink() {
                return Err(Error::Symlink);
            }
        }
    }
    Ok(absolute)
}

fn same_metadata(before: &Metadata, after: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return false;
        }
    }
    before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && before.created().ok() == after.created().ok()
        && after.is_file()
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, Error> {
    let path = checked_path(path)?;
    let before = fs::symlink_metadata(&path).map_err(|_| Error::File("metadata"))?;
    if !before.is_file() {
        return Err(Error::NotRegular);
    }
    if before.len() == 0 {
        return Err(Error::Empty);
    }
    if before.len() > limit {
        return Err(Error::TooLarge);
    }
    let identity = same_file::Handle::from_path(&path).map_err(|_| Error::File("identity"))?;
    let file = File::open(&path).map_err(|_| Error::File("open"))?;
    let opened = file.metadata().map_err(|_| Error::File("metadata"))?;
    let handle = same_file::Handle::from_file(file.try_clone().map_err(|_| Error::File("clone"))?)
        .map_err(|_| Error::File("identity"))?;
    if identity != handle || !same_metadata(&before, &opened) {
        return Err(Error::Changed);
    }
    let mut bytes = Vec::new();
    (&file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::File("read"))?;
    if bytes.len() as u64 > limit {
        return Err(Error::TooLarge);
    }
    checked_path(&path)?;
    let after = file.metadata().map_err(|_| Error::File("metadata"))?;
    let final_metadata = fs::symlink_metadata(&path).map_err(|_| Error::File("metadata"))?;
    let final_identity =
        same_file::Handle::from_path(&path).map_err(|_| Error::File("identity"))?;
    if final_identity != handle
        || !same_metadata(&before, &after)
        || !same_metadata(&before, &final_metadata)
        || bytes.len() as u64 != before.len()
    {
        return Err(Error::Changed);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tauri CLI 2.12.4 signer fixtures. Independent disposable test key, destroyed after signing.
    const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDI4Q0E5RkIxODE0OTdEQjAKUldTd2ZVbUJzWi9LS0xPWFl2eUxvbWRIVW9NaUltelM1RlNhNjdnS3ZETEZLMVA3OENOMVowYm4K";
    const OTHER_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEE3ODYxMUE4MENBOUZGMUEKUldRYS82a01xQkdHcDRNN1FFV05UQ3RlZk5rOHBqRmZENHVpR1JxVC9tWUlYajhlVk9UT2dma3gK";
    const SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTd2ZVbUJzWi9LS1BBWFcycDlraXVvcThJSWFrMDNzNSszZW15WENXWnRsSDk2a25iR2FGS3JkQ1B4cWZWbTEzTEdtMi9jUktiU0puVHJwaEg4Q3RtYXVHQmlvaDVnd3c0PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMTY4ODA0CWZpbGU6Zml4dHVyZS5iaW4JdmVyc2lvbjoxLjIuMwpIT0kzcVhKR05ka211WmdCSnVsWE92dXVwQTdOd2V2UklCMS96YXVORlg1MTI4bkNCWVdkM2JMemRzdG1FZ3FqV29WcTlGSElVMkk1Y3ZqUjcybUlBZz09Cg==";
    const LEGACY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTd2ZVbUJzWi9LS0U4Um43aUNrUGxKdW9pUEVSZlhIMVcvVWNDV3FScmFFM1lVc0E4NWhURG83TmhGczdOQStIWVNUbjY3bkVVMmQydFpHMTNMSDJuOUd6QytrcDU5aUFzPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMTY4ODA0CWZpbGU6Zml4dHVyZS5iaW4KN1RjaGx1OVVGeVcxcy9seTQrRTdicFlSdWhkeERVN2NTYkRYZVVFRUdITHpmTUdWQkkxUmpkcmRnemRhWjFsRDBvN1lLbHh4dEJ4NUF1Ly81RkpBQlE9PQo=";
    const PAYLOAD: &[u8] = b"Iceon updater verification fixture\n";

    struct Fixture {
        _temp: tempfile::TempDir,
        config: PathBuf,
        sig: PathBuf,
        data: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let config = root.join("config.json");
            let sig = root.join("fixture.sig");
            let data = root.join("fixture.bin");
            fs::write(
                &config,
                serde_json::json!({"plugins":{"updater":{"pubkey":KEY}}}).to_string(),
            )
            .unwrap();
            fs::write(&sig, SIG).unwrap();
            fs::write(&data, PAYLOAD).unwrap();
            Self {
                _temp: temp,
                config,
                sig,
                data,
            }
        }
        fn run(&self, version: &str) -> Result<(), Error> {
            run(&self.config, &self.sig, &self.data, version)
        }
    }

    #[test]
    fn authentic_fixture_and_wrong_version() {
        let f = Fixture::new();
        assert_eq!(f.run("1.2.3"), Ok(()));
        assert_eq!(f.run("1.2.4"), Err(Error::VersionMismatch));
        for version in [
            "v1.2.3",
            "01.2.3",
            "1.2",
            "1.2.3-beta",
            "1.2.3+build",
            " 1.2.3",
            "",
        ] {
            assert_eq!(f.run(version), Err(Error::Version));
        }
    }

    #[test]
    fn tampering_and_wrong_key_fail_authentication() {
        let f = Fixture::new();
        fs::write(&f.data, b"tampered payload").unwrap();
        assert_eq!(f.run("1.2.3"), Err(Error::Authentication));
        fs::write(&f.data, PAYLOAD).unwrap();
        let comment = decode(SIG.as_bytes())
            .unwrap()
            .replace("version:1.2.3", "version:1.2.4");
        fs::write(&f.sig, STANDARD.encode(comment)).unwrap();
        assert_eq!(f.run("1.2.4"), Err(Error::Authentication));
        fs::write(&f.sig, SIG).unwrap();
        fs::write(
            &f.config,
            serde_json::json!({"plugins":{"updater":{"pubkey":OTHER_KEY}}}).to_string(),
        )
        .unwrap();
        assert_eq!(f.run("1.2.3"), Err(Error::Authentication));
    }

    #[test]
    fn missing_authenticated_version_fails() {
        let f = Fixture::new();
        fs::write(&f.sig, LEGACY).unwrap();
        assert_eq!(f.run("1.2.3"), Err(Error::MissingVersion));
    }

    #[test]
    fn exact_unique_canonical_comment_fields() {
        let expected = semver::Version::new(1, 2, 3);
        for comment in [
            "version:1.2.3\tversion:1.2.3",
            "version:1.2.3\tversion:1.2.4",
        ] {
            assert_eq!(
                signed_version(comment, &expected),
                Err(Error::DuplicateVersion)
            );
        }
        for comment in [
            "version:v1.2.3",
            "version:1.2.3+build",
            "version:1.2.3-beta",
            "version: 1.2.3",
            "version:",
        ] {
            assert_eq!(
                signed_version(comment, &expected),
                Err(Error::SignedVersion)
            );
        }
        assert_eq!(
            signed_version("file:version:1.2.3", &expected),
            Err(Error::MissingVersion)
        );
    }

    #[test]
    fn malformed_and_empty_inputs() {
        let f = Fixture::new();
        for config in [
            "{}",
            "not JSON",
            r#"{"plugins":{"updater":{"pubkey":"invalid"}}}"#,
            r#"{"plugins":{"updater":{"pubkey":""}}}"#,
        ] {
            fs::write(&f.config, config).unwrap();
            assert!(matches!(
                f.run("1.2.3"),
                Err(Error::Config | Error::PublicKey)
            ));
        }
        let f = Fixture::new();
        for sig in ["!", "bm90IG1pbmlzaWdu", "////"] {
            fs::write(&f.sig, sig).unwrap();
            assert_eq!(f.run("1.2.3"), Err(Error::Signature));
        }
        for path in [&f.config, &f.sig, &f.data] {
            fs::write(path, []).unwrap();
            assert_eq!(read_bounded(path, PAYLOAD_LIMIT), Err(Error::Empty));
        }
    }

    #[test]
    fn bounded_regular_inputs() {
        let f = Fixture::new();
        assert_eq!(
            read_bounded(&f.data, PAYLOAD.len() as u64),
            Ok(PAYLOAD.to_vec())
        );
        assert_eq!(
            read_bounded(&f.data, PAYLOAD.len() as u64 - 1),
            Err(Error::TooLarge)
        );
        assert_eq!(
            read_bounded(f.data.parent().unwrap(), 100),
            Err(Error::NotRegular)
        );
        for (path, limit) in [
            (&f.config, CONFIG_LIMIT),
            (&f.sig, SIGNATURE_LIMIT),
            (&f.data, PAYLOAD_LIMIT),
        ] {
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_len(limit + 1)
                .unwrap();
            assert_eq!(read_bounded(path, limit), Err(Error::TooLarge));
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_in_files_and_ancestors() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let link = f.data.with_file_name("link");
        symlink(&f.data, &link).unwrap();
        assert_eq!(read_bounded(&link, PAYLOAD_LIMIT), Err(Error::Symlink));
        fs::remove_file(&link).unwrap();
        symlink(f.data.parent().unwrap(), &link).unwrap();
        assert_eq!(
            read_bounded(&link.join("fixture.bin"), PAYLOAD_LIMIT),
            Err(Error::Symlink)
        );
    }
}
