use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use std::{
    fmt,
    fs::{self, File, Metadata},
    io::Read,
    path::{Component, Path, PathBuf},
};

const SIGNATURE_LIMIT: u64 = 16 * 1024;
const PAYLOAD_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Version,
    Environment,
    PrivateKey,
    Filename,
    Clock,
    Signing,
    Output,
    File(&'static str),
    Symlink,
    NotRegular,
    Empty,
    TooLarge,
    Changed,
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
            Self::Environment => "update signing environment is missing or invalid",
            Self::PrivateKey => "update signing key could not be opened",
            Self::Filename => "update filename must be UTF-8 without control characters",
            Self::Clock => "update signing clock is invalid",
            Self::Signing => "update signing failed or key does not match pinned public key",
            Self::Output => {
                "update signature output could not be created (existing output is never overwritten)"
            }
            Self::Version => "expected canonical stable X.Y.Z release version",
            Self::File(operation) => return write!(f, "update input file {operation} failed"),
            Self::Symlink => "update input symlink paths are not supported",
            Self::NotRegular => "update inputs must be regular files",
            Self::Empty => "update inputs must not be empty",
            Self::TooLarge => "update input exceeds its size limit",
            Self::Changed => "update input changed while being read",
            Self::PublicKey => "invalid base64 updater minisign public key",
            Self::Signature => "invalid base64 updater minisign signature",
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

// Pinned from debe548:src-tauri/tauri.conf.json. Never supplied by a release artifact.
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDNCRUY2REJDNUExQjA1MEQKUldRTkJSdGF2RzN2TzJ5cE80b0IrOEFsNHpManZxN1JkMXpUdXNlR0dSMHdJNU95MFVvQmJHT3UK";

pub fn verify_update(signature: &Path, payload: &Path, version: &str) -> Result<(), Error> {
    verify_with_key(signature, payload, version, PUBLIC_KEY)
}

fn verify_with_key(
    signature: &Path,
    payload: &Path,
    version: &str,
    public_key: &str,
) -> Result<(), Error> {
    super::stable(version).map_err(|_| Error::Version)?;
    let data = read_bounded(payload, PAYLOAD_LIMIT)?;
    verify_bytes(
        &read_bounded(signature, SIGNATURE_LIMIT)?,
        &data,
        version,
        public_key,
    )
}

fn verify_bytes(encoded: &[u8], data: &[u8], version: &str, public_key: &str) -> Result<(), Error> {
    let version = super::stable(version).map_err(|_| Error::Version)?;
    let key_text = decode(public_key.as_bytes()).map_err(|_| Error::PublicKey)?;
    if key_text.lines().count() != 2 {
        return Err(Error::PublicKey);
    }
    let key = PublicKey::decode(&key_text).map_err(|_| Error::PublicKey)?;
    let signature_text = decode(encoded).map_err(|_| Error::Signature)?;
    if signature_text.lines().count() != 4 {
        return Err(Error::Signature);
    }
    let signature = Signature::decode(&signature_text).map_err(|_| Error::Signature)?;
    // Independent implementation verifies both payload and global trusted-comment signatures.
    key.verify(data, &signature, true)
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

/// The private-key environment value is the base64-encoded complete minisign key document.
/// Password presence is mandatory, but an explicitly empty password is supported.
pub fn sign_update(payload: &Path, version: &str) -> Result<(), Error> {
    super::stable(version).map_err(|_| Error::Version)?;
    let encoded = std::env::var("ICEON_UPDATE_PRIVATE_KEY").map_err(|_| Error::Environment)?;
    let password =
        std::env::var("ICEON_UPDATE_PRIVATE_KEY_PASSWORD").map_err(|_| Error::Environment)?;
    sign_with_key(payload, version, &encoded, password, PUBLIC_KEY)
}

fn open_secret(encoded: &str, password: String) -> Result<minisign::SecretKey, Error> {
    let text = decode(encoded.as_bytes()).map_err(|_| Error::PrivateKey)?;
    if text.lines().count() != 2 {
        return Err(Error::PrivateKey);
    }
    let boxed = minisign::SecretKeyBox::from_string(&text).map_err(|_| Error::PrivateKey)?;
    // Some, even for an empty password: None would prompt on stdin.
    minisign::SecretKey::from_box(boxed, Some(password)).map_err(|_| Error::PrivateKey)
}

fn sign_with_key(
    payload: &Path,
    version: &str,
    encoded: &str,
    password: String,
    public_key: &str,
) -> Result<(), Error> {
    use std::{
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };
    super::stable(version).map_err(|_| Error::Version)?;
    let name = payload
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Error::Filename)?;
    if name.chars().any(char::is_control) {
        return Err(Error::Filename);
    }
    let data = read_bounded(payload, PAYLOAD_LIMIT)?;
    let sk = open_secret(encoded, password)?;
    let key_text = decode(public_key.as_bytes()).map_err(|_| Error::PublicKey)?;
    let pk = minisign::PublicKey::from_box(
        minisign::PublicKeyBox::from_string(&key_text).map_err(|_| Error::PublicKey)?,
    )
    .map_err(|_| Error::PublicKey)?;
    // Compare the complete derived public key, not only its 64-bit key identifier.
    if minisign::PublicKey::from_secret_key(&sk)
        .map_err(|_| Error::Signing)?
        .to_bytes()
        != pk.to_bytes()
    {
        return Err(Error::Signing);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Clock)?
        .as_secs();
    let comment = format!("timestamp:{timestamp}\tfile:{name}\tversion:{version}");
    let signature = minisign::sign(Some(&pk), &sk, data.as_slice(), Some(&comment), None)
        .map_err(|_| Error::Signing)?;
    let encoded = STANDARD.encode(signature.to_string().as_bytes());
    verify_bytes(encoded.as_bytes(), &data, version, public_key)?;
    let mut output = payload.as_os_str().to_os_string();
    output.push(".sig");
    let output = PathBuf::from(output);
    // Stage privately, verify the final payload bytes and on-disk signature independently,
    // then publish atomically without replacing an existing file (including symlinks).
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = checked_path(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|_| Error::Output)?;
    staged
        .write_all(encoded.as_bytes())
        .map_err(|_| Error::Output)?;
    staged.as_file().sync_all().map_err(|_| Error::Output)?;
    verify_with_key(staged.path(), payload, version, public_key)?;
    staged
        .persist_noclobber(output)
        .map_err(|_| Error::Output)?;
    Ok(())
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

    #[test]
    fn independent_historic_fixtures() {
        assert_eq!(verify_bytes(SIG.as_bytes(), PAYLOAD, "1.2.3", KEY), Ok(()));
        assert_eq!(
            verify_bytes(SIG.as_bytes(), PAYLOAD, "1.2.4", KEY),
            Err(Error::VersionMismatch)
        );
        assert_eq!(
            verify_bytes(SIG.as_bytes(), PAYLOAD, "1.2.3", OTHER_KEY),
            Err(Error::Authentication)
        );
        assert_eq!(
            verify_bytes(SIG.as_bytes(), PAYLOAD, "1.2.3", PUBLIC_KEY),
            Err(Error::Authentication)
        );
        assert_eq!(
            verify_bytes(LEGACY.as_bytes(), PAYLOAD, "1.2.3", KEY),
            Err(Error::MissingVersion)
        );
        assert_eq!(
            verify_bytes(SIG.as_bytes(), b"tampered", "1.2.3", KEY),
            Err(Error::Authentication)
        );
        let changed = decode(SIG.as_bytes())
            .unwrap()
            .replace("version:1.2.3", "version:1.2.4");
        assert_eq!(
            verify_bytes(STANDARD.encode(changed).as_bytes(), PAYLOAD, "1.2.4", KEY),
            Err(Error::Authentication)
        );
        // The fourth line is the global signature: dropping or changing it is fatal.
        let mut lines: Vec<_> = decode(SIG.as_bytes())
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        lines[3].replace_range(..1, "A");
        assert_eq!(
            verify_bytes(
                STANDARD.encode(lines.join("\n")).as_bytes(),
                PAYLOAD,
                "1.2.3",
                KEY
            ),
            Err(Error::Authentication)
        );
        lines.pop();
        assert_eq!(
            verify_bytes(
                STANDARD.encode(lines.join("\n")).as_bytes(),
                PAYLOAD,
                "1.2.3",
                KEY
            ),
            Err(Error::Signature)
        );
    }

    #[test]
    fn canonical_unique_version() {
        let expected = semver::Version::new(1, 2, 3);
        for version in [
            "v1.2.3",
            "01.2.3",
            "1.2",
            "1.2.3-beta",
            "1.2.3+build",
            " 1.2.3",
            "",
        ] {
            assert_eq!(
                verify_bytes(SIG.as_bytes(), PAYLOAD, version, KEY),
                Err(Error::Version)
            );
            assert_eq!(
                signed_version(&format!("version:{version}"), &expected),
                Err(Error::SignedVersion)
            );
        }
        for comment in [
            "version:1.2.3\tversion:1.2.3",
            "version:1.2.3\tversion:1.2.4",
        ] {
            assert_eq!(
                signed_version(comment, &expected),
                Err(Error::DuplicateVersion)
            );
        }
        assert_eq!(
            signed_version("file:version:1.2.3", &expected),
            Err(Error::MissingVersion)
        );
    }

    #[test]
    fn real_encrypted_key_roundtrip_no_overwrite_and_mismatch() {
        // Explicit empty password exercises exactly the CI noninteractive key-opening path.
        let pair = minisign::KeyPair::generate_encrypted_keypair(Some(String::new())).unwrap();
        let private = STANDARD.encode(pair.sk.to_box(None).unwrap().into_string());
        let public = STANDARD.encode(pair.pk.to_box().unwrap().into_string());
        assert!(matches!(
            open_secret(&private, "incorrect password".into()),
            Err(Error::PrivateKey)
        ));
        let temp = tempfile::tempdir().unwrap();
        let payload = temp.path().canonicalize().unwrap().join("fixture.bin");
        let signature = payload.with_file_name("fixture.bin.sig");
        fs::write(&payload, PAYLOAD).unwrap();
        assert_eq!(
            sign_with_key(&payload, "1.2.3", &private, String::new(), &public),
            Ok(())
        );
        assert_eq!(
            verify_with_key(&signature, &payload, "1.2.3", &public),
            Ok(())
        );
        let original = fs::read(&signature).unwrap();
        let document = decode(&original).unwrap();
        let sig = Signature::decode(&document).unwrap();
        let fields: Vec<_> = sig.trusted_comment().split('\t').collect();
        assert_eq!(fields.len(), 3);
        assert!(
            fields[0]
                .strip_prefix("timestamp:")
                .unwrap()
                .parse::<u64>()
                .is_ok()
        );
        assert_eq!(fields[1], "file:fixture.bin");
        assert_eq!(fields[2], "version:1.2.3");
        assert_eq!(
            sign_with_key(&payload, "1.2.3", &private, String::new(), &public),
            Err(Error::Output)
        );
        assert_eq!(fs::read(&signature).unwrap(), original);
        fs::remove_file(&signature).unwrap();
        assert_eq!(
            sign_with_key(&payload, "1.2.3", &private, String::new(), PUBLIC_KEY),
            Err(Error::Signing)
        );
        assert!(!signature.exists());
        // Authenticate noncanonical/duplicate fields cryptographically, not just parser unit tests.
        let sk = open_secret(&private, String::new()).unwrap();
        for (comment, error) in [
            ("version:1.2.3\tversion:1.2.3", Error::DuplicateVersion),
            ("version:v1.2.3", Error::SignedVersion),
        ] {
            let sig = minisign::sign(Some(&pair.pk), &sk, PAYLOAD, Some(comment), None).unwrap();
            assert_eq!(
                verify_bytes(
                    STANDARD.encode(sig.to_string()).as_bytes(),
                    PAYLOAD,
                    "1.2.3",
                    &public
                ),
                Err(error)
            );
        }
    }

    #[test]
    fn malformed_and_bounded_inputs() {
        for value in ["!", "bm90IG1pbmlzaWdu", "////", ""] {
            assert_eq!(
                verify_bytes(value.as_bytes(), PAYLOAD, "1.2.3", KEY),
                Err(Error::Signature)
            );
            assert!(matches!(
                open_secret(value, String::new()),
                Err(Error::PrivateKey)
            ));
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let data = root.join("data");
        fs::write(&data, PAYLOAD).unwrap();
        assert_eq!(
            read_bounded(&data, PAYLOAD.len() as u64),
            Ok(PAYLOAD.to_vec())
        );
        assert_eq!(read_bounded(&data, 1), Err(Error::TooLarge));
        assert_eq!(read_bounded(&root, 1), Err(Error::NotRegular));
        fs::write(&data, []).unwrap();
        assert_eq!(read_bounded(&data, 1), Err(Error::Empty));
        File::options()
            .write(true)
            .open(&data)
            .unwrap()
            .set_len(PAYLOAD_LIMIT + 1)
            .unwrap();
        assert_eq!(read_bounded(&data, PAYLOAD_LIMIT), Err(Error::TooLarge));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_comment_injection() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let data = root.join("data");
        fs::write(&data, PAYLOAD).unwrap();
        let link = root.join("link");
        symlink(&data, &link).unwrap();
        assert_eq!(read_bounded(&link, PAYLOAD_LIMIT), Err(Error::Symlink));
        fs::remove_file(&link).unwrap();
        symlink(&root, &link).unwrap();
        assert_eq!(
            read_bounded(&link.join("data"), PAYLOAD_LIMIT),
            Err(Error::Symlink)
        );
        assert_eq!(
            sign_with_key(
                &root.join("data\tversion:9.9.9"),
                "1.2.3",
                "",
                String::new(),
                PUBLIC_KEY
            ),
            Err(Error::Filename)
        );
    }
}
