use super::*;
use futures::future::BoxFuture;
use std::sync::{
    Condvar,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

const TOKEN: &str = "synthetic-provider-token";
const PASSWORD: &str = "synthetic-login-password";

struct FakeProvider {
    login_error: Mutex<Option<Error>>,
    resume_result: Mutex<Result<Option<u64>>>,
    hold_login: AtomicBool,
    entered: Notify,
    release: Notify,
    resumes: AtomicUsize,
}
impl Default for FakeProvider {
    fn default() -> Self {
        Self {
            login_error: Mutex::new(None),
            resume_result: Mutex::new(Ok(Some(now() + 60_000))),
            hold_login: AtomicBool::new(false),
            entered: Notify::new(),
            release: Notify::new(),
            resumes: AtomicUsize::new(0),
        }
    }
}
impl SecretProvider for FakeProvider {
    fn login<'a>(
        &'a self,
        profile: &'a Profile,
        username: &'a str,
        password: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<vault::Auth>> {
        Box::pin(async move {
            assert!(profile.address == "https://synthetic.invalid/");
            assert!(username == "synthetic-user");
            assert!(password == PASSWORD);
            self.entered.notify_one();
            if self.hold_login.load(Ordering::SeqCst) {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(Error::Cancelled),
                    _ = self.release.notified() => {},
                    _ = tokio::time::sleep(Duration::from_secs(3)) => return Err(Error::Network),
                }
            }
            if let Some(error) = *self.login_error.lock().unwrap() {
                return Err(error);
            }
            Ok(vault::Auth {
                token: Zeroizing::new(TOKEN.into()),
                expires_at: Some(now() + 60_000),
            })
        })
    }
    fn resume<'a>(
        &'a self,
        _: &'a Profile,
        token: &'a str,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<u64>>> {
        Box::pin(async move {
            assert!(token == TOKEN);
            self.resumes.fetch_add(1, Ordering::SeqCst);
            *self.resume_result.lock().unwrap()
        })
    }
    fn list<'a>(
        &'a self,
        _: &'a Profile,
        token: &'a str,
        path: &'a str,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(Vec<Entry>, bool)>> {
        Box::pin(async move {
            assert!(token == TOKEN);
            assert!(path.is_empty());
            Ok((
                vec![Entry {
                    name: "config".into(),
                    directory: false,
                }],
                true,
            ))
        })
    }
    fn reveal<'a>(
        &'a self,
        _: &'a Profile,
        token: &'a str,
        path: &'a str,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(Vec<Field>, u64)>> {
        Box::pin(async move {
            assert!(token == TOKEN);
            assert!(path == "config");
            Ok((
                vec![Field {
                    name: "value".into(),
                    value: "synthetic-field".into(),
                }],
                7,
            ))
        })
    }
}

#[derive(Default)]
struct FakeCredentialStore {
    values: Mutex<HashMap<String, Zeroizing<String>>>,
    sets: AtomicUsize,
    deletes: AtomicUsize,
    fail_set: AtomicBool,
    hold_set: Mutex<bool>,
    released: Condvar,
    entered: Notify,
}
impl FakeCredentialStore {
    fn release(&self) {
        *self.hold_set.lock().unwrap() = false;
        self.released.notify_all();
    }
}
impl CredentialStore for FakeCredentialStore {
    fn kind(&self) -> &'static str {
        "synthetic"
    }
    fn get(&self, key: &str) -> Result<Zeroizing<String>> {
        self.values
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or(Error::NotFound)
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.sets.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        let held = self.hold_set.lock().unwrap();
        let (held, timeout) = self
            .released
            .wait_timeout_while(held, Duration::from_secs(3), |held| *held)
            .unwrap();
        if *held || timeout.timed_out() {
            return Err(Error::CredentialStoreUnavailable);
        }
        if self.fail_set.load(Ordering::SeqCst) {
            return Err(Error::CredentialStoreUnavailable);
        }
        self.values
            .lock()
            .unwrap()
            .insert(key.into(), Zeroizing::new(value.into()));
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

struct Fixture {
    _directory: tempfile::TempDir,
    engine: Arc<Engine>,
    credentials: Arc<FakeCredentialStore>,
    provider: Arc<FakeProvider>,
    request: SessionRequest,
}
impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let credentials = Arc::new(FakeCredentialStore::default());
        let provider = Arc::new(FakeProvider::default());
        let engine = Engine::with_provider(
            directory.path().to_path_buf(),
            credentials.clone(),
            provider.clone(),
        );
        let profile = engine
            .save(ProfileInput {
                provider: Provider::Vault,
                name: "Synthetic".into(),
                address: "https://synthetic.invalid/".into(),
                namespace: "team".into(),
                auth_mount: "userpass".into(),
                kv_mount: "secret".into(),
                base_path: "apps".into(),
            })
            .await
            .unwrap();
        let view = engine
            .open("main", &profile.id, "owner".into())
            .await
            .unwrap();
        Self {
            _directory: directory,
            engine,
            credentials,
            provider,
            request: SessionRequest {
                profile_id: view.profile_id,
                session_id: view.session_id,
            },
        }
    }
    fn login(&self, remember: bool) -> LoginRequest {
        LoginRequest {
            profile_id: self.request.profile_id.clone(),
            session_id: self.request.session_id.clone(),
            username: "synthetic-user".into(),
            password: PASSWORD.into(),
            remember,
        }
    }
    fn disconnect(&self, forget: bool) -> DisconnectRequest {
        DisconnectRequest {
            profile_id: self.request.profile_id.clone(),
            session_id: self.request.session_id.clone(),
            forget,
        }
    }
    fn signed_out(&self) {
        let sessions = self.engine.sessions.lock().unwrap();
        if let Some(session) = sessions.get("main") {
            assert!(session.token.is_none());
            assert!(!session.authenticating);
        }
    }
}
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("synthetic operation timed out")
}

#[tokio::test]
async fn login_remembers_only_token_and_redacts_serialized_view() {
    let f = Fixture::new().await;
    let view = f.engine.login("main", f.login(true)).await.unwrap();
    assert_eq!(view.status, "connected");
    assert_eq!(f.credentials.sets.load(Ordering::SeqCst), 1);
    let values = f.credentials.values.lock().unwrap();
    assert_eq!(values.len(), 1);
    assert!(values.values().all(|value| value.as_str() == TOKEN));
    assert!(
        values
            .keys()
            .all(|key| key.len() == 64 && !key.contains("synthetic"))
    );
    let json = serde_json::to_string(&view).unwrap();
    assert!(!json.contains(TOKEN) && !json.contains(PASSWORD) && !json.contains("synthetic-user"));
}

#[tokio::test]
async fn login_without_remember_does_not_write_credentials() {
    let f = Fixture::new().await;
    assert_eq!(
        f.engine.login("main", f.login(false)).await.unwrap().status,
        "connected"
    );
    assert_eq!(f.credentials.sets.load(Ordering::SeqCst), 0);
    assert!(f.credentials.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_remember_never_publishes_connected_session() {
    let f = Fixture::new().await;
    f.credentials.fail_set.store(true, Ordering::SeqCst);
    let error = f.engine.login("main", f.login(true)).await.err().unwrap();
    assert_eq!(error, Error::CredentialStoreUnavailable);
    assert_eq!(
        serde_json::to_string(&error).unwrap(),
        "\"credentialStoreUnavailable\""
    );
    f.signed_out();
    assert!(f.credentials.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_login_never_writes_credentials_and_can_retry() {
    let f = Fixture::new().await;
    *f.provider.login_error.lock().unwrap() = Some(Error::AuthFailed);
    assert!(matches!(
        f.engine.login("main", f.login(true)).await,
        Err(Error::AuthFailed)
    ));
    f.signed_out();
    assert_eq!(f.credentials.sets.load(Ordering::SeqCst), 0);
    *f.provider.login_error.lock().unwrap() = None;
    assert!(f.engine.login("main", f.login(false)).await.is_ok());
}

#[tokio::test]
async fn resume_revalidates_saved_token_and_uses_remote_expiry() {
    let f = Fixture::new().await;
    f.engine.login("main", f.login(true)).await.unwrap();
    f.engine
        .disconnect("main", f.disconnect(false))
        .await
        .unwrap();
    let expiry = now() + 20_000;
    *f.provider.resume_result.lock().unwrap() = Ok(Some(expiry));
    let view = f.engine.resume("main", f.request.clone()).await.unwrap();
    assert_eq!(view.status, "connected");
    assert_eq!(view.expires_at, Some(expiry));
    assert_eq!(f.provider.resumes.load(Ordering::SeqCst), 1);
    assert_eq!(f.credentials.sets.load(Ordering::SeqCst), 1);
    assert!(!serde_json::to_string(&view).unwrap().contains(TOKEN));
}

#[tokio::test]
async fn resume_without_saved_token_never_calls_provider() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.engine.resume("main", f.request.clone()).await,
        Err(Error::NotFound)
    ));
    assert_eq!(f.provider.resumes.load(Ordering::SeqCst), 0);
    f.signed_out();
}

#[tokio::test]
async fn expired_or_rejected_remote_resume_never_connects() {
    for response in [
        Ok(Some(now().saturating_sub(1))),
        Err(Error::SessionExpired),
        Err(Error::PermissionDenied),
    ] {
        let f = Fixture::new().await;
        f.engine.login("main", f.login(true)).await.unwrap();
        f.engine
            .disconnect("main", f.disconnect(false))
            .await
            .unwrap();
        *f.provider.resume_result.lock().unwrap() = response;
        assert!(f.engine.resume("main", f.request.clone()).await.is_err());
        f.signed_out();
    }
}

#[tokio::test]
async fn connected_reads_use_injected_transport() {
    let f = Fixture::new().await;
    f.engine.login("main", f.login(false)).await.unwrap();
    let request = |read_id, path: &str| ReadRequest {
        profile_id: f.request.profile_id.clone(),
        session_id: f.request.session_id.clone(),
        read_id,
        path: path.into(),
    };
    let listing = f.engine.list("main", request(1, "")).await.unwrap();
    assert!(listing.partial);
    assert_eq!(listing.entries[0].name, "config");
    let reveal = f.engine.reveal("main", request(2, "config")).await.unwrap();
    assert_eq!(reveal.version, 7);
    assert_eq!(reveal.fields[0].value, "synthetic-field");
}

#[tokio::test]
async fn delayed_login_cannot_revive_disconnected_closed_or_shutdown_session() {
    for action in 0..4 {
        let f = Fixture::new().await;
        f.provider.hold_login.store(true, Ordering::SeqCst);
        let engine = f.engine.clone();
        let login = f.login(true);
        let task = tokio::spawn(async move { engine.login("main", login).await });
        bounded(f.provider.entered.notified()).await;
        match action {
            0 => f
                .engine
                .disconnect("main", f.disconnect(false))
                .await
                .unwrap(),
            1 => f.engine.close("main", &f.request).unwrap(),
            2 => f.engine.close_window("main"),
            _ => f.engine.shutdown(),
        }
        f.provider.release.notify_one();
        assert!(matches!(
            bounded(task).await.unwrap(),
            Err(Error::Cancelled | Error::StaleSession)
        ));
        f.signed_out();
        assert!(f.credentials.values.lock().unwrap().is_empty());
        assert_eq!(f.credentials.sets.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn cancelled_held_remember_rolls_back_token_before_completing() {
    for action in 0..4 {
        let f = Fixture::new().await;
        *f.credentials.hold_set.lock().unwrap() = true;
        let engine = f.engine.clone();
        let login = f.login(true);
        let task = tokio::spawn(async move { engine.login("main", login).await });
        bounded(f.credentials.entered.notified()).await;
        f.signed_out_auth_in_flight();
        match action {
            0 => f
                .engine
                .disconnect("main", f.disconnect(false))
                .await
                .unwrap(),
            1 => f.engine.close("main", &f.request).unwrap(),
            2 => f.engine.close_window("main"),
            _ => f.engine.shutdown(),
        }
        f.credentials.release();
        assert!(matches!(
            bounded(task).await.unwrap(),
            Err(Error::Cancelled | Error::StaleSession)
        ));
        f.signed_out();
        assert!(f.credentials.values.lock().unwrap().is_empty());
        assert_eq!(f.credentials.deletes.load(Ordering::SeqCst), 1);
    }
}
impl Fixture {
    fn signed_out_auth_in_flight(&self) {
        let sessions = self.engine.sessions.lock().unwrap();
        let session = sessions.get("main").unwrap();
        assert!(session.token.is_none());
        assert!(session.authenticating);
    }
}

#[tokio::test]
async fn forget_removes_remembered_token_and_blocks_resume() {
    let f = Fixture::new().await;
    f.engine.login("main", f.login(true)).await.unwrap();
    f.engine
        .disconnect("main", f.disconnect(true))
        .await
        .unwrap();
    assert!(f.credentials.values.lock().unwrap().is_empty());
    assert!(matches!(
        f.engine.resume("main", f.request.clone()).await,
        Err(Error::NotFound)
    ));
    assert_eq!(f.provider.resumes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn remembered_token_survives_engine_recreation_but_requires_remote_validation() {
    let f = Fixture::new().await;
    f.engine.login("main", f.login(true)).await.unwrap();
    f.engine.shutdown();
    let engine = Engine::with_provider(
        f._directory.path().to_path_buf(),
        f.credentials.clone(),
        f.provider.clone(),
    );
    let opened = engine
        .open("main", &f.request.profile_id, "new-owner".into())
        .await
        .unwrap();
    assert_eq!(opened.status, "signedOut");
    assert!(opened.expires_at.is_none());
    let request = SessionRequest {
        profile_id: opened.profile_id,
        session_id: opened.session_id,
    };
    *f.provider.resume_result.lock().unwrap() = Err(Error::AuthFailed);
    assert!(matches!(
        engine.resume("main", request.clone()).await,
        Err(Error::AuthFailed)
    ));
    *f.provider.resume_result.lock().unwrap() = Ok(None);
    let view = engine.resume("main", request).await.unwrap();
    assert_eq!(view.status, "connected");
    assert!(view.expires_at.is_none());
    assert_eq!(f.provider.resumes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn forget_serializes_with_held_remember_and_leaves_no_saved_token() {
    let f = Fixture::new().await;
    *f.credentials.hold_set.lock().unwrap() = true;
    let engine = f.engine.clone();
    let input = f.login(true);
    let login = tokio::spawn(async move { engine.login("main", input).await });
    bounded(f.credentials.entered.notified()).await;
    let engine = f.engine.clone();
    let input = f.disconnect(true);
    let forget = tokio::spawn(async move { engine.disconnect("main", input).await });
    tokio::task::yield_now().await;
    assert!(!forget.is_finished());
    f.credentials.release();
    let _ = bounded(login).await.unwrap();
    bounded(forget).await.unwrap().unwrap();
    f.signed_out();
    assert!(f.credentials.values.lock().unwrap().is_empty());
    assert!(matches!(
        f.engine.resume("main", f.request.clone()).await,
        Err(Error::NotFound)
    ));
}
