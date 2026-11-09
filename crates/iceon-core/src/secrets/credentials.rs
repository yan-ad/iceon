use super::{Error, Result};
use std::sync::Mutex;
use zeroize::Zeroizing;
const SERVICE: &str = "ad.yan.iceon";
const TOKEN_LIMIT: usize = 16 * 1024;
// Keyring backends can race even when callers use distinct Entry instances.
static ACCESS: Mutex<()> = Mutex::new(());
pub(super) trait CredentialStore: Send + Sync {
    fn kind(&self) -> &'static str;
    fn get(&self, key: &str) -> Result<Zeroizing<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}
pub(super) struct NativeStore;
impl CredentialStore for NativeStore {
    fn kind(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            "macosKeychain"
        } else if cfg!(target_os = "windows") {
            "windowsCredentialManager"
        } else if cfg!(target_os = "linux") {
            "linuxSecretService"
        } else {
            "unavailable"
        }
    }
    fn get(&self, key: &str) -> Result<Zeroizing<String>> {
        with_entry(key, |entry| {
            let token = Zeroizing::new(entry.get_password().map_err(map_error)?);
            if !valid_token(&token) {
                return Err(Error::CredentialStoreUnavailable);
            }
            Ok(token)
        })
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        if !valid_token(value) {
            return Err(Error::InvalidInput);
        }
        with_entry(key, |entry| entry.set_password(value).map_err(map_error))
    }
    fn delete(&self, key: &str) -> Result<()> {
        with_entry(key, |entry| delete_result(entry.delete_credential()))
    }
}
fn validate_key(key: &str) -> Result<()> {
    if key.len() != 64
        || !key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
fn valid_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= TOKEN_LIMIT
}
fn with_entry<T>(key: &str, action: impl FnOnce(&keyring::Entry) -> Result<T>) -> Result<T> {
    validate_key(key)?;
    let _guard = ACCESS
        .lock()
        .map_err(|_| Error::CredentialStoreUnavailable)?;
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        // Target-specific Cargo features select native Keychain/Credential Manager/Secret Service.
        let entry = keyring::Entry::new(SERVICE, key).map_err(map_error)?;
        action(&entry)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (SERVICE, action);
        Err(Error::CredentialStoreUnavailable)
    }
}

fn map_error(error: keyring::Error) -> Error {
    match error {
        keyring::Error::NoEntry => Error::NotFound,
        _ => Error::CredentialStoreUnavailable,
    }
}

fn delete_result(result: keyring::Result<()>) -> Result<()> {
    match result {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(map_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pure validation/mapping only: no Entry construction or credential-store operations.
    #[test]
    fn opaque_keys_only() {
        assert!(validate_key(&"0123456789abcdef".repeat(4)).is_ok());
        for key in [
            "".into(),
            "a".repeat(63),
            "a".repeat(65),
            "A".repeat(64),
            "g".repeat(64),
            "/".repeat(64),
            "\0".repeat(64),
            "é".repeat(32),
        ] {
            assert!(matches!(validate_key(&key), Err(Error::InvalidInput)));
        }
    }

    #[test]
    fn token_byte_limit() {
        assert!(!valid_token(""));
        assert!(valid_token(&"t".repeat(TOKEN_LIMIT)));
        assert!(!valid_token(&"t".repeat(TOKEN_LIMIT + 1)));
        assert!(!valid_token(&"é".repeat(TOKEN_LIMIT)));
    }

    #[test]
    fn errors_are_redacted_and_deletion_is_idempotent() {
        assert!(matches!(
            map_error(keyring::Error::NoEntry),
            Error::NotFound
        ));
        for error in [
            keyring::Error::Invalid("token".into(), "private".into()),
            keyring::Error::BadEncoding(b"private".to_vec()),
        ] {
            assert!(matches!(
                map_error(error),
                Error::CredentialStoreUnavailable
            ));
        }
        assert!(delete_result(Ok(())).is_ok());
        assert!(delete_result(Err(keyring::Error::NoEntry)).is_ok());
        assert!(matches!(
            delete_result(Err(keyring::Error::BadEncoding(vec![0xff]))),
            Err(Error::CredentialStoreUnavailable)
        ));
    }

    #[test]
    fn platform_kind_without_store_access() {
        let expected = if cfg!(target_os = "macos") {
            "macosKeychain"
        } else if cfg!(target_os = "windows") {
            "windowsCredentialManager"
        } else if cfg!(target_os = "linux") {
            "linuxSecretService"
        } else {
            "unavailable"
        };
        assert_eq!(NativeStore.kind(), expected);
        fn send_sync<T: Send + Sync>() {}
        send_sync::<NativeStore>();
    }
}
