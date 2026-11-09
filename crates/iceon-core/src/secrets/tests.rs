use super::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use tempfile::TempDir;

#[derive(Default)]
struct FakeStore {
    values: Mutex<HashMap<String, Zeroizing<String>>>,
    fail: AtomicBool,
}

impl CredentialStore for FakeStore {
    fn kind(&self) -> &'static str {
        "fake"
    }

    fn get(&self, key: &str) -> Result<Zeroizing<String>> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::CredentialStoreUnavailable);
        }
        self.values
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or(Error::NotFound)
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::CredentialStoreUnavailable);
        }
        self.values
            .lock()
            .unwrap()
            .insert(key.into(), Zeroizing::new(value.into()));
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::CredentialStoreUnavailable);
        }
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

fn profile() -> Profile {
    Profile {
        id: "0123456789abcdef0123456789abcdef".into(),
        provider: Provider::Vault,
        name: "Synthetic Vault".into(),
        address: "https://vault.example/".into(),
        namespace: "team".into(),
        auth_mount: "userpass".into(),
        kv_mount: "secret".into(),
        base_path: "apps".into(),
        pinned: false,
    }
}

fn input() -> ProfileInput {
    let p = profile();
    ProfileInput {
        provider: p.provider,
        name: p.name,
        address: p.address,
        namespace: p.namespace,
        auth_mount: p.auth_mount,
        kv_mount: p.kv_mount,
        base_path: p.base_path,
    }
}

async fn fixture() -> (TempDir, Arc<Engine>, Arc<FakeStore>, Profile) {
    let dir = tempfile::tempdir().unwrap();
    let credentials = Arc::new(FakeStore::default());
    let engine = Engine::with_store(dir.path().to_path_buf(), credentials.clone());
    let p = engine.save(input()).await.unwrap();
    (dir, engine, credentials, p)
}

async fn open(engine: &Engine, p: &Profile, window: &str, owner: &str) -> SessionRequest {
    let view = engine.open(window, &p.id, owner.into()).await.unwrap();
    assert_eq!(view.status, "signedOut");
    assert_eq!(view.credential_store, "fake");
    assert!(view.expires_at.is_none());
    SessionRequest {
        profile_id: view.profile_id,
        session_id: view.session_id,
    }
}

fn auth(expires_at: Option<u64>) -> vault::Auth {
    vault::Auth {
        token: Zeroizing::new("synthetic-token-not-a-credential".into()),
        expires_at,
    }
}

fn connect(engine: &Engine, window: &str, request: &SessionRequest) {
    let work = engine.begin_auth(window, request).unwrap();
    let view = engine
        .finish_auth(window, request, &work, Ok(auth(None)))
        .unwrap();
    assert_eq!(view.status, "connected");
}

fn read(request: &SessionRequest, read_id: u64) -> ReadRequest {
    ReadRequest {
        profile_id: request.profile_id.clone(),
        session_id: request.session_id.clone(),
        read_id,
        path: "service/config".into(),
        mount: None,
    }
}

fn cancel(request: &SessionRequest, read_id: u64) -> CancelRequest {
    CancelRequest {
        profile_id: request.profile_id.clone(),
        session_id: request.session_id.clone(),
        read_id,
    }
}

fn disconnect(request: &SessionRequest, forget: bool) -> DisconnectRequest {
    DisconnectRequest {
        profile_id: request.profile_id.clone(),
        session_id: request.session_id.clone(),
        forget,
    }
}

#[test]
fn generated_ids_are_valid_unique_and_lowercase() {
    let ids: HashSet<_> = (0..128).map(|_| new_id().unwrap()).collect();
    assert_eq!(ids.len(), 128);
    assert!(ids.iter().all(|id| valid_id(id)));
    for id in [
        "",
        "a",
        "0123456789ABCDEF0123456789ABCDEF",
        "g123456789abcdef0123456789abcdef",
    ] {
        assert!(!valid_id(id));
    }
}

#[test]
fn profiles_accept_https_origins_and_bounded_metadata() {
    for address in [
        "https://vault.example",
        "https://vault.example/",
        "https://vault.example:8200/",
        "https://[::1]:8200/",
    ] {
        let mut p = profile();
        p.address = address.into();
        p.namespace.clear();
        p.base_path.clear();
        assert!(validate_profile(&p).is_ok(), "{address}");
    }
    let mut p = profile();
    p.name = "n".repeat(128);
    assert!(validate_profile(&p).is_ok());
    p.name.push('n');
    assert!(matches!(validate_profile(&p), Err(Error::InvalidInput)));
}

#[test]
fn profiles_reject_non_origin_addresses_and_userinfo() {
    for address in [
        "",
        "http://vault.example/",
        "file:///tmp/vault",
        "https://user@vault.example/",
        "https://user:password@vault.example/",
        "https://vault.example/api",
        "https://vault.example/?x=1",
        "https://vault.example/#fragment",
    ] {
        let mut p = profile();
        p.address = address.into();
        assert!(
            matches!(validate_profile(&p), Err(Error::InvalidInput)),
            "{address}"
        );
    }
}

#[test]
fn profiles_reject_traversal_and_ambiguous_paths_in_every_path_field() {
    for path in [
        "/a", "a/", "a//b", ".", "..", "a/../b", "a/./b", "a\\b", "%2e%2e", "a\0b", "a\nb",
    ] {
        for field in 0..4 {
            let mut p = profile();
            match field {
                0 => p.namespace = path.into(),
                1 => p.auth_mount = path.into(),
                2 => p.kv_mount = path.into(),
                _ => p.base_path = path.into(),
            }
            assert!(
                matches!(validate_profile(&p), Err(Error::InvalidInput)),
                "field {field}, path {path:?}"
            );
        }
    }
    assert!(valid_path(&"a".repeat(1024), false));
    assert!(!valid_path(&"a".repeat(1025), false));
    assert!(valid_path("", true));
    assert!(!valid_path("", false));
    assert!(!valid_path(
        &[
            "a".repeat(1024),
            "b".repeat(1024),
            "c".repeat(1024),
            "d".repeat(1024)
        ]
        .join("/"),
        false
    ));
}

#[test]
fn profiles_reject_invalid_ids_names_mounts_and_namespaces() {
    for field in 0..6 {
        let mut p = profile();
        match field {
            0 => p.id = "bad".into(),
            1 => p.name = " \t ".into(),
            2 => p.name = "bad\nname".into(),
            3 => p.auth_mount.clear(),
            4 => p.kv_mount.clear(),
            _ => p.namespace = "équipe".into(),
        }
        assert!(matches!(validate_profile(&p), Err(Error::InvalidInput)));
    }
}

#[test]
fn serialized_profiles_have_only_metadata_fields() {
    let value = serde_json::to_value(profile()).unwrap();
    let keys: HashSet<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        keys == HashSet::from([
            "id",
            "provider",
            "name",
            "address",
            "namespace",
            "authMount",
            "kvMount",
            "basePath",
            "pinned"
        ])
    );
    assert_eq!(value["provider"], "vault");
    let roundtrip: Profile = serde_json::from_value(value).unwrap();
    assert!(roundtrip == profile());
}

#[test]
fn ipc_rejects_unknown_fields_and_profile_credential_injection() {
    for field in ["password", "token", "username", "remember", "sessionId"] {
        let mut value = serde_json::to_value(profile()).unwrap();
        value[field] = serde_json::json!("injected");
        assert!(serde_json::from_value::<Profile>(value.clone()).is_err());
        value.as_object_mut().unwrap().remove("id");
        assert!(serde_json::from_value::<ProfileInput>(value).is_err());
    }
    let login = serde_json::json!({"profileId": profile().id, "sessionId": "synthetic", "username": "tester", "password": "synthetic", "remember": true});
    assert!(serde_json::from_value::<LoginRequest>(login.clone()).is_ok());
    for field in [
        "address",
        "authMount",
        "kvMount",
        "namespace",
        "token",
        "unknown",
    ] {
        let mut value = login.clone();
        value[field] = serde_json::json!("injected");
        assert!(serde_json::from_value::<LoginRequest>(value).is_err());
    }
}

#[tokio::test]
async fn save_remove_and_restart_persist_metadata_only() {
    let (dir, engine, credentials, p) = fixture().await;
    assert!(engine.profiles().await.unwrap() == vec![p.clone()]);
    let restarted = Engine::with_store(dir.path().to_path_buf(), credentials.clone());
    assert!(restarted.profiles().await.unwrap() == vec![p.clone()]);
    assert!(credentials.values.lock().unwrap().is_empty());
    engine.remove(p.id.clone()).await.unwrap();
    assert!(restarted.profiles().await.unwrap().is_empty());
    assert!(matches!(engine.remove(p.id).await, Err(Error::NotFound)));
}

#[tokio::test]
async fn save_trims_names_and_rejects_invalid_profiles_without_writing() {
    let (_dir, engine, _, p) = fixture().await;
    let mut valid = input();
    valid.name = "  Another  ".into();
    assert_eq!(engine.save(valid).await.unwrap().name, "Another");
    let mut invalid = input();
    invalid.address = "http://vault.example/".into();
    assert!(matches!(
        engine.save(invalid).await,
        Err(Error::InvalidInput)
    ));
    assert_eq!(engine.profiles().await.unwrap().len(), 2);
    assert!(matches!(
        engine.profile("bad").await,
        Err(Error::InvalidInput)
    ));
    assert!(matches!(
        engine.profile(&"f".repeat(32)).await,
        Err(Error::NotFound)
    ));
    assert!(engine.profile(&p.id).await.is_ok());
}

#[tokio::test]
async fn credential_keys_bind_all_profile_fields_and_appdata_directory() {
    let (dir, engine, credentials, p) = fixture().await;
    let original = engine.key(&p);
    assert_eq!(original.len(), 64);
    assert!(
        original
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    let same = Engine::with_store(dir.path().to_path_buf(), credentials.clone());
    assert_eq!(same.key(&p), original);
    let other_dir = tempfile::tempdir().unwrap();
    let other = Engine::with_store(other_dir.path().to_path_buf(), credentials);
    assert_ne!(other.key(&p), original);
    for field in 0..7 {
        let mut changed = p.clone();
        match field {
            0 => changed.id = "f".repeat(32),
            1 => changed.address = "https://other.example/".into(),
            2 => changed.namespace = "other".into(),
            3 => changed.auth_mount = "other".into(),
            4 => changed.kv_mount = "other".into(),
            5 => changed.base_path = "other".into(),
            _ => changed.name = "Other".into(),
        }
        assert_ne!(engine.key(&changed), original, "field {field}");
    }
}

#[tokio::test]
async fn open_is_idempotent_only_for_same_window_owner_and_profile() {
    let (_dir, engine, _, p) = fixture().await;
    let first = open(&engine, &p, "a", "owner").await;
    let work = engine.begin_auth("a", &first).unwrap();
    let same = engine.open("a", &p.id, "owner".into()).await.unwrap();
    assert_eq!(same.session_id, first.session_id);
    assert!(!work.cancel.is_cancelled());
    let replacement = open(&engine, &p, "a", "next-owner").await;
    assert_ne!(replacement.session_id, first.session_id);
    assert!(work.cancel.is_cancelled());
    assert!(matches!(
        engine.finish_auth("a", &first, &work, Ok(auth(None))),
        Err(Error::StaleSession)
    ));
    connect(&engine, "a", &replacement);
    let pending = engine.begin_read("a", &read(&replacement, 1)).unwrap();
    let other = engine.save(input()).await.unwrap();
    let replaced = open(&engine, &other, "a", "next-owner").await;
    assert_ne!(replaced.session_id, replacement.session_id);
    assert!(!pending.cancel.is_cancelled());
    assert!(
        engine
            .finish_read("a", &read(&replacement, 1), &pending, Ok(()))
            .is_ok()
    );
}

#[tokio::test]
async fn windows_cannot_auth_read_cancel_close_or_disconnect_other_sessions() {
    let (_dir, engine, _, p) = fixture().await;
    let a = open(&engine, &p, "a", "owner").await;
    let b = open(&engine, &p, "b", "owner").await;
    assert_ne!(a.session_id, b.session_id);
    connect(&engine, "a", &a);
    let pending = engine.begin_read("a", &read(&a, 1)).unwrap();
    assert!(matches!(
        engine.begin_auth("b", &a),
        Err(Error::StaleSession)
    ));
    assert!(matches!(
        engine.begin_read("b", &read(&a, 2)),
        Err(Error::StaleSession)
    ));
    assert!(matches!(
        engine.cancel("b", cancel(&a, 1)),
        Err(Error::StaleSession)
    ));
    assert!(matches!(engine.close("b", &a), Err(Error::StaleSession)));
    assert!(matches!(
        engine.disconnect("b", disconnect(&a, true)).await,
        Err(Error::StaleSession)
    ));
    assert!(!pending.cancel.is_cancelled());
    assert!(
        engine
            .finish_read("a", &read(&a, 1), &pending, Ok(()))
            .is_ok()
    );
    assert!(engine.begin_auth("b", &b).is_ok());
}

#[tokio::test]
async fn open_validates_owner_and_limits_distinct_windows() {
    let (_dir, engine, _, p) = fixture().await;
    for owner in [
        "".into(),
        "a".repeat(65),
        "bad owner".into(),
        "bad_owner".into(),
        "é".into(),
    ] {
        assert!(matches!(
            engine.open("w", &p.id, owner).await,
            Err(Error::InvalidInput)
        ));
    }
    for index in 0..32 {
        open(&engine, &p, &format!("w{index}"), "owner").await;
    }
    assert!(matches!(
        engine.open("ninth", &p.id, "owner".into()).await,
        Err(Error::Busy)
    ));
    assert!(engine.open("w0", &p.id, "replacement".into()).await.is_ok());
    engine.close_window("w1");
    assert!(engine.open("ninth", &p.id, "owner".into()).await.is_ok());
}

#[tokio::test]
async fn auth_epochs_cancel_reads_and_reject_replies_after_disconnect() {
    let (_dir, engine, _, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    connect(&engine, "w", &request);
    let reading = engine.begin_read("w", &read(&request, 1)).unwrap();
    let first = engine.begin_auth("w", &request).unwrap();
    assert!(reading.cancel.is_cancelled());
    assert!(matches!(engine.begin_auth("w", &request), Err(Error::Busy)));
    assert!(matches!(
        engine.begin_read("w", &read(&request, 2)),
        Err(Error::Busy)
    ));
    engine
        .disconnect("w", disconnect(&request, false))
        .await
        .unwrap();
    assert!(first.cancel.is_cancelled());
    let next = engine.begin_auth("w", &request).unwrap();
    assert!(next.epoch > first.epoch);
    assert!(matches!(
        engine.check_auth("w", &request, &first),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        engine.finish_auth("w", &request, &first, Ok(auth(None))),
        Err(Error::Cancelled)
    ));
    assert!(
        engine
            .finish_auth("w", &request, &next, Ok(auth(None)))
            .is_ok()
    );
}

#[tokio::test]
async fn auth_errors_and_expired_tokens_leave_no_in_memory_token() {
    let (_dir, engine, _, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    for error in [
        Error::AuthFailed,
        Error::PermissionDenied,
        Error::CredentialStoreUnavailable,
        Error::Network,
    ] {
        let work = engine.begin_auth("w", &request).unwrap();
        assert!(
            matches!(engine.finish_auth("w", &request, &work, Err(error)), Err(e) if e == error)
        );
        let sessions = engine.sessions.lock().unwrap();
        let s = sessions.values().find(|s| s.window == "w").unwrap();
        assert!(!s.authenticating);
        assert!(s.token.is_none());
        assert!(s.expires.is_none());
    }
    let work = engine.begin_auth("w", &request).unwrap();
    assert!(matches!(
        engine.finish_auth("w", &request, &work, Ok(auth(Some(0)))),
        Err(Error::SessionExpired)
    ));
    assert!(
        engine
            .sessions
            .lock()
            .unwrap()
            .values()
            .find(|s| s.window == "w")
            .unwrap()
            .token
            .is_none()
    );
}

#[tokio::test]
async fn remembered_token_lookup_failures_do_not_contact_vault_or_connect() {
    let (_dir, engine, credentials, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    assert!(matches!(
        engine.resume("w", request.clone()).await,
        Err(Error::NotFound)
    ));
    credentials.fail.store(true, Ordering::SeqCst);
    assert!(matches!(
        engine.resume("w", request.clone()).await,
        Err(Error::CredentialStoreUnavailable)
    ));
    let sessions = engine.sessions.lock().unwrap();
    let s = sessions.values().find(|s| s.window == "w").unwrap();
    assert!(s.token.is_none());
    assert!(!s.authenticating);
}

#[tokio::test]
async fn read_replacement_and_exact_cancellation_enforce_monotonic_ids() {
    let (_dir, engine, _, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    connect(&engine, "w", &request);
    let first_request = read(&request, 4);
    let first = engine.begin_read("w", &first_request).unwrap();
    let second_request = read(&request, 5);
    let second = engine.begin_read("w", &second_request).unwrap();
    assert!(first.cancel.is_cancelled());
    assert!(matches!(
        engine.finish_read("w", &first_request, &first, Ok(())),
        Err(Error::Cancelled)
    ));
    engine.cancel("w", cancel(&request, 4)).unwrap();
    engine.cancel("w", cancel(&request, 6)).unwrap();
    assert!(!second.cancel.is_cancelled());
    assert!(matches!(
        engine.begin_read("w", &first_request),
        Err(Error::Cancelled)
    ));
    assert!(!second.cancel.is_cancelled());
    engine.cancel("w", cancel(&request, 5)).unwrap();
    engine.cancel("w", cancel(&request, 5)).unwrap();
    assert!(second.cancel.is_cancelled());
    assert!(matches!(
        engine.finish_read("w", &second_request, &second, Ok(())),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        engine.begin_read("w", &second_request),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        engine.begin_read("w", &read(&request, 6)),
        Err(Error::Cancelled)
    ));
    let third_request = read(&request, 7);
    let third = engine.begin_read("w", &third_request).unwrap();
    assert!(
        engine
            .finish_read("w", &third_request, &third, Ok(()))
            .is_ok()
    );
    assert!(
        engine
            .sessions
            .lock()
            .unwrap()
            .values()
            .find(|s| s.window == "w")
            .unwrap()
            .read
            .is_none()
    );
}

#[tokio::test]
async fn invalid_reads_are_rejected_without_replacing_active_work() {
    let (_dir, engine, _, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    assert!(matches!(
        engine.begin_read("w", &read(&request, 1)),
        Err(Error::AuthFailed)
    ));
    connect(&engine, "w", &request);
    let active = engine.begin_read("w", &read(&request, 1)).unwrap();
    for id in [0, 9_007_199_254_740_992, u64::MAX] {
        assert!(matches!(
            engine.begin_read("w", &read(&request, id)),
            Err(Error::InvalidInput)
        ));
    }
    for path in ["../escape", "a//b", "%2e%2e", "a\\b"] {
        let mut invalid = read(&request, 2);
        invalid.path = path.into();
        assert!(matches!(
            engine.begin_read("w", &invalid),
            Err(Error::InvalidInput)
        ));
    }
    let mut empty = read(&request, 2);
    empty.path.clear();
    assert!(matches!(
        engine.reveal("w", empty).await,
        Err(Error::InvalidInput)
    ));
    assert!(!active.cancel.is_cancelled());
}

#[tokio::test]
async fn expired_read_start_clears_token_expiry_and_active_work() {
    let (_dir, engine, _, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    connect(&engine, "w", &request);
    let pending = engine.begin_read("w", &read(&request, 1)).unwrap();
    engine
        .sessions
        .lock()
        .unwrap()
        .values_mut()
        .find(|s| s.window == "w")
        .unwrap()
        .expires = Some(0);
    assert!(matches!(
        engine.begin_read("w", &read(&request, 2)),
        Err(Error::SessionExpired)
    ));
    assert!(pending.cancel.is_cancelled());
    let sessions = engine.sessions.lock().unwrap();
    let s = sessions.values().find(|s| s.window == "w").unwrap();
    assert!(s.token.is_none());
    assert!(s.expires.is_none());
    assert!(s.read.is_none());
    assert!(s.expired);
    assert_eq!(s.view("fake").status, "expired");
}

#[tokio::test]
async fn expired_read_completion_clears_memory_for_local_and_remote_expiry() {
    let (_dir, engine, _, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    for (id, result) in [(1, Ok(())), (2, Err(Error::SessionExpired))] {
        connect(&engine, "w", &request);
        let r = read(&request, id);
        let work = engine.begin_read("w", &r).unwrap();
        if id == 1 {
            engine
                .sessions
                .lock()
                .unwrap()
                .values_mut()
                .find(|s| s.window == "w")
                .unwrap()
                .expires = Some(0);
        }
        assert!(matches!(
            engine.finish_read("w", &r, &work, result),
            Err(Error::SessionExpired)
        ));
        let sessions = engine.sessions.lock().unwrap();
        let s = sessions.values().find(|s| s.window == "w").unwrap();
        assert!(s.token.is_none());
        assert!(s.expired);
        assert!(s.read.is_none());
    }
}

#[tokio::test]
async fn non_expiration_read_errors_release_slot_without_clearing_auth() {
    let (_dir, engine, _, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    connect(&engine, "w", &request);
    let r = read(&request, 1);
    let work = engine.begin_read("w", &r).unwrap();
    assert!(matches!(
        engine.finish_read("w", &r, &work, Err(Error::Network)),
        Err(Error::Network)
    ));
    let sessions = engine.sessions.lock().unwrap();
    let s = sessions.values().find(|s| s.window == "w").unwrap();
    assert!(s.token.is_some());
    assert!(s.read.is_none());
}

#[tokio::test]
async fn close_and_close_window_cancel_work_and_reject_late_replies() {
    let (_dir, engine, _, p) = fixture().await;
    let a = open(&engine, &p, "a", "owner").await;
    let b = open(&engine, &p, "b", "owner").await;
    let authenticating = engine.begin_auth("a", &a).unwrap();
    connect(&engine, "b", &b);
    let r = read(&b, 1);
    let reading = engine.begin_read("b", &r).unwrap();
    engine.close("a", &a).unwrap();
    engine.close("a", &a).unwrap();
    engine.close_window("b");
    engine.close_window("missing");
    assert!(authenticating.cancel.is_cancelled());
    assert!(reading.cancel.is_cancelled());
    assert!(matches!(
        engine.finish_auth("a", &a, &authenticating, Ok(auth(None))),
        Err(Error::StaleSession)
    ));
    assert!(matches!(
        engine.finish_read("b", &r, &reading, Ok(())),
        Err(Error::StaleSession)
    ));
    assert!(matches!(
        engine.begin_read("b", &read(&b, 2)),
        Err(Error::StaleSession)
    ));
}

#[tokio::test]
async fn disconnect_preserves_or_forgets_only_the_bound_remembered_token() {
    let (_dir, engine, credentials, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    let key = engine.key(&p);
    credentials.set(&key, "synthetic").unwrap();
    credentials
        .set("unrelated-profile", "synthetic-other")
        .unwrap();
    connect(&engine, "w", &request);
    let pending = engine.begin_read("w", &read(&request, 1)).unwrap();
    engine
        .disconnect("w", disconnect(&request, false))
        .await
        .unwrap();
    assert!(pending.cancel.is_cancelled());
    assert!(credentials.get(&key).is_ok());
    assert!(matches!(
        engine.begin_read("w", &read(&request, 2)),
        Err(Error::AuthFailed)
    ));
    engine
        .disconnect("w", disconnect(&request, true))
        .await
        .unwrap();
    engine
        .disconnect("w", disconnect(&request, true))
        .await
        .unwrap();
    assert!(matches!(credentials.get(&key), Err(Error::NotFound)));
    assert!(credentials.get("unrelated-profile").is_ok());
    assert!(engine.profile(&p.id).await.is_ok());
}

#[tokio::test]
async fn forget_failure_still_clears_memory_and_cancels_work() {
    let (_dir, engine, credentials, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    let key = engine.key(&p);
    credentials.set(&key, "synthetic").unwrap();
    connect(&engine, "w", &request);
    let pending = engine.begin_read("w", &read(&request, 1)).unwrap();
    credentials.fail.store(true, Ordering::SeqCst);
    assert!(matches!(
        engine.disconnect("w", disconnect(&request, true)).await,
        Err(Error::CredentialStoreUnavailable)
    ));
    assert!(pending.cancel.is_cancelled());
    let sessions = engine.sessions.lock().unwrap();
    let s = sessions.values().find(|s| s.window == "w").unwrap();
    assert!(s.token.is_none());
    assert!(s.expires.is_none());
    assert!(!s.expired);
    drop(sessions);
    credentials.fail.store(false, Ordering::SeqCst);
    assert!(credentials.get(&key).is_ok());
}

#[tokio::test]
async fn remove_store_failure_preserves_profile_but_invalidates_all_its_sessions() {
    let (_dir, engine, credentials, p) = fixture().await;
    let a = open(&engine, &p, "a", "owner").await;
    let b = open(&engine, &p, "b", "owner").await;
    let work = engine.begin_auth("a", &a).unwrap();
    connect(&engine, "b", &b);
    let reading = engine.begin_read("b", &read(&b, 1)).unwrap();
    let other_profile = engine.save(input()).await.unwrap();
    let other = open(&engine, &other_profile, "other", "owner").await;
    credentials.fail.store(true, Ordering::SeqCst);
    assert!(matches!(
        engine.remove(p.id.clone()).await,
        Err(Error::CredentialStoreUnavailable)
    ));
    assert!(work.cancel.is_cancelled());
    assert!(reading.cancel.is_cancelled());
    assert!(engine.profile(&p.id).await.is_ok());
    assert!(matches!(
        engine.begin_auth("a", &a),
        Err(Error::StaleSession)
    ));
    assert!(engine.begin_auth("other", &other).is_ok());
    credentials.fail.store(false, Ordering::SeqCst);
    engine.remove(p.id.clone()).await.unwrap();
    assert!(matches!(engine.profile(&p.id).await, Err(Error::NotFound)));
    assert!(engine.profile(&other_profile.id).await.is_ok());
}

#[test]
fn synchronous_native_setup_starts_and_drains_expiry_task_without_tokio_context() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let (_dir, engine, _, profile) = crate::runtime::block_on(fixture());
    let request = crate::runtime::block_on(open(&engine, &profile, "w", "owner"));
    let authenticating = engine.begin_auth("w", &request).unwrap();
    engine
        .sessions
        .lock()
        .unwrap()
        .values_mut()
        .find(|s| s.window == "w")
        .unwrap()
        .touched = 0;

    assert!(tokio::runtime::Handle::try_current().is_err());
    engine.start();
    let expired = crate::runtime::block_on(async {
        tokio::time::timeout(Duration::from_secs(3), authenticating.cancel.cancelled()).await
    });
    engine.shutdown();
    let drained = crate::runtime::block_on(async {
        tokio::time::timeout(Duration::from_secs(2), engine.drain()).await
    });
    assert!(
        expired.is_ok(),
        "startup task must actually poll on shared Tokio runtime"
    );
    assert!(
        drained.is_ok(),
        "shutdown must drain the tracked startup task"
    );
    assert!(engine.sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn shutdown_cancels_auth_reads_and_background_task_then_drains() {
    let (_dir, engine, _, p) = fixture().await;
    let a = open(&engine, &p, "a", "owner").await;
    let b = open(&engine, &p, "b", "owner").await;
    let authenticating = engine.begin_auth("a", &a).unwrap();
    connect(&engine, "b", &b);
    let r = read(&b, 1);
    let reading = engine.begin_read("b", &r).unwrap();
    engine.start();
    tokio::task::yield_now().await;
    engine.shutdown();
    tokio::time::timeout(Duration::from_secs(2), engine.drain())
        .await
        .unwrap();
    assert!(engine.stop.is_cancelled());
    assert!(engine.sessions.lock().unwrap().is_empty());
    assert!(authenticating.cancel.is_cancelled());
    assert!(reading.cancel.is_cancelled());
    assert!(matches!(
        engine.finish_auth("a", &a, &authenticating, Ok(auth(None))),
        Err(Error::StaleSession)
    ));
    assert!(matches!(
        engine.finish_read("b", &r, &reading, Ok(())),
        Err(Error::StaleSession)
    ));
    assert!(matches!(
        engine.open("a", &p.id, "owner".into()).await,
        Err(Error::Cancelled)
    ));
    engine.shutdown();
    engine.drain().await;
}

#[tokio::test]
async fn background_sweep_expires_idle_sessions_and_cancels_auth() {
    let (_dir, engine, _, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    let work = engine.begin_auth("w", &request).unwrap();
    engine
        .sessions
        .lock()
        .unwrap()
        .values_mut()
        .find(|s| s.window == "w")
        .unwrap()
        .touched = 0;
    engine.start();
    tokio::time::timeout(Duration::from_secs(3), work.cancel.cancelled())
        .await
        .unwrap();
    {
        let sessions = engine.sessions.lock().unwrap();
        let s = sessions.values().find(|s| s.window == "w").unwrap();
        assert!(s.expired);
        assert!(!s.authenticating);
        assert!(s.token.is_none());
    }
    engine.shutdown();
    engine.drain().await;
}

// AuthWork has no unique completion latch: replaying a finished work item must not
// be able to replace the session's token after its first completion.
#[tokio::test]
async fn finished_auth_work_cannot_be_completed_twice() {
    let (_dir, engine, _, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    let work = engine.begin_auth("w", &request).unwrap();
    assert!(
        engine
            .finish_auth("w", &request, &work, Ok(auth(None)))
            .is_ok()
    );
    assert!(matches!(
        engine.finish_auth("w", &request, &work, Ok(auth(None))),
        Err(Error::Cancelled)
    ));
}

fn edit_profile(p: &Profile) -> ProfileUpdate {
    ProfileUpdate {
        id: p.id.clone(),
        provider: p.provider,
        name: "Edited".into(),
        address: p.address.clone(),
        namespace: p.namespace.clone(),
        auth_mount: p.auth_mount.clone(),
        kv_mount: p.kv_mount.clone(),
        base_path: p.base_path.clone(),
    }
}

#[tokio::test]
async fn pin_preserves_credential_identity_and_active_sessions_across_restart() {
    let (directory, engine, credentials, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    connect(&engine, "w", &request);
    let work = engine.begin_read("w", &read(&request, 1)).unwrap();
    let key = engine.key(&p);
    credentials
        .values
        .lock()
        .unwrap()
        .insert(key.clone(), Zeroizing::new("remembered".into()));
    credentials.fail.store(true, Ordering::SeqCst);
    let pinned = engine.pin(p.id.clone(), true).await.unwrap();
    assert!(pinned.pinned);
    assert_eq!(engine.key(&pinned), key);
    assert!(!work.cancel.is_cancelled());
    assert!(credentials.values.lock().unwrap().contains_key(&key));
    let restart = Engine::with_store(directory.path().to_path_buf(), credentials);
    assert!(restart.profiles().await.unwrap()[0].pinned);
    assert!(
        engine
            .finish_read("w", &read(&request, 1), &work, Ok(()))
            .is_ok()
    );
}

#[tokio::test]
async fn edit_fails_closed_on_credentials_and_only_cancels_target_profile() {
    let (directory, engine, credentials, p) = fixture().await;
    let other = engine.save(input()).await.unwrap();
    let a = open(&engine, &p, "w", "owner").await;
    let b = open(&engine, &other, "w", "owner").await;
    connect(&engine, "w", &a);
    connect(&engine, "w", &b);
    let a_work = engine.begin_read("w", &read(&a, 1)).unwrap();
    let b_work = engine.begin_read("w", &read(&b, 1)).unwrap();
    let pinned = engine.pin(p.id.clone(), true).await.unwrap();
    credentials.fail.store(true, Ordering::SeqCst);
    assert!(matches!(
        engine.update(edit_profile(&p)).await,
        Err(Error::CredentialStoreUnavailable)
    ));
    assert!(engine.profile(&p.id).await.unwrap() == pinned);
    assert!(!a_work.cancel.is_cancelled());
    credentials.fail.store(false, Ordering::SeqCst);
    let updated = engine.update(edit_profile(&p)).await.unwrap();
    assert_eq!(updated.id, p.id);
    assert!(updated.pinned);
    assert_eq!(updated.name, "Edited");
    assert!(a_work.cancel.is_cancelled());
    assert!(!b_work.cancel.is_cancelled());
    assert!(matches!(
        engine.begin_auth("w", &a),
        Err(Error::StaleSession)
    ));
    assert!(
        engine
            .finish_read("w", &read(&b, 1), &b_work, Ok(()))
            .is_ok()
    );
    let restart = Engine::with_store(directory.path().to_path_buf(), credentials);
    assert!(restart.profile(&p.id).await.unwrap() == updated);
    engine.close_window("w");
    assert!(engine.sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_mounts_require_discovery_and_stale_discovery_cannot_populate_cache() {
    let (_directory, engine, _, p) = fixture().await;
    let request = open(&engine, &p, "w", "owner").await;
    connect(&engine, "w", &request);
    let mut selected = read(&request, 1);
    selected.mount = Some("another".into());
    assert!(matches!(
        engine.begin_read("w", &selected),
        Err(Error::InvalidInput)
    ));
    let mut discovery = read(&request, 1);
    discovery.path.clear();
    let work = engine.begin_read("w", &discovery).unwrap();
    engine.cancel("w", cancel(&request, 1)).unwrap();
    let mounts = vec![Mount {
        path: "another".into(),
        kind: "kv".into(),
        version: Some(2),
        supported: true,
    }];
    assert_eq!(
        engine.finish_read_with_mounts("w", &discovery, &work, Ok(()), Some(&mounts)),
        Err(Error::Cancelled)
    );
    selected.read_id = 2;
    assert!(matches!(
        engine.begin_read("w", &selected),
        Err(Error::InvalidInput)
    ));
    discovery.read_id = 2;
    let work = engine.begin_read("w", &discovery).unwrap();
    engine
        .finish_read_with_mounts("w", &discovery, &work, Ok(()), Some(&mounts))
        .unwrap();
    selected.read_id = 3;
    let work = engine.begin_read("w", &selected).unwrap();
    assert_eq!(work.profile.kv_mount, "another");
    engine.finish_read("w", &selected, &work, Ok(())).unwrap();
    engine
        .disconnect("w", disconnect(&request, false))
        .await
        .unwrap();
    connect(&engine, "w", &request);
    selected.read_id = 4;
    assert!(matches!(
        engine.begin_read("w", &selected),
        Err(Error::InvalidInput)
    ));
}
