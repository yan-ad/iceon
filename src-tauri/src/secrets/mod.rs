mod commands;
mod credentials;
mod provider;
mod storage;
mod vault;
pub use commands::*;

use credentials::{CredentialStore, NativeStore};
use provider::{SecretProvider, VaultProvider};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    InvalidInput,
    NotFound,
    StoreUnavailable,
    CredentialStoreUnavailable,
    AuthFailed,
    PermissionDenied,
    SessionExpired,
    StaleSession,
    Cancelled,
    Busy,
    Network,
    Tls,
    InvalidResponse,
    TooLarge,
    UnsupportedAuth,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Vault,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub provider: Provider,
    pub name: String,
    pub address: String,
    pub namespace: String,
    pub auth_mount: String,
    pub kv_mount: String,
    pub base_path: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileInput {
    pub provider: Provider,
    pub name: String,
    pub address: String,
    pub namespace: String,
    pub auth_mount: String,
    pub kv_mount: String,
    pub base_path: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionRequest {
    pub profile_id: String,
    pub session_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginRequest {
    pub profile_id: String,
    pub session_id: String,
    pub username: String,
    pub password: String,
    pub remember: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisconnectRequest {
    pub profile_id: String,
    pub session_id: String,
    pub forget: bool,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadRequest {
    pub profile_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub path: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelRequest {
    pub profile_id: String,
    pub session_id: String,
    pub read_id: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub profile_id: String,
    pub session_id: String,
    pub status: &'static str,
    pub expires_at: Option<u64>,
    pub credential_store: &'static str,
}
#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub directory: bool,
}
#[derive(Serialize)]
pub struct Field {
    pub name: String,
    pub value: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub profile_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub path: String,
    pub entries: Vec<Entry>,
    pub partial: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reveal {
    pub profile_id: String,
    pub session_id: String,
    pub read_id: u64,
    pub path: String,
    pub fields: Vec<Field>,
    pub version: u64,
    pub expires_at: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn new_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| Error::StoreUnavailable)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn valid_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_path(value: &str, empty: bool) -> bool {
    (empty && value.is_empty())
        || (!value.is_empty()
            && value.len() <= 4096
            && value.split('/').all(|part| {
                !part.is_empty()
                    && part.len() <= 1024
                    && part != "."
                    && part != ".."
                    && !part
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '\\' | '%'))
            }))
}
fn validate_profile(p: &Profile) -> Result<()> {
    let url = url::Url::parse(&p.address).map_err(|_| Error::InvalidInput)?;
    if !valid_id(&p.id)
        || p.name.trim().is_empty()
        || p.name.len() > 128
        || p.name.chars().any(char::is_control)
        || p.address.len() > 2048
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || !valid_path(&p.namespace, true)
        || !p.namespace.is_ascii()
        || !valid_path(&p.auth_mount, false)
        || !valid_path(&p.kv_mount, false)
        || !valid_path(&p.base_path, true)
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

struct Session {
    window: String,
    owner: String,
    profile: Profile,
    id: String,
    token: Option<Zeroizing<String>>,
    expires: Option<u64>,
    expired: bool,
    epoch: u64,
    auth: CancellationToken,
    authenticating: bool,
    read: Option<(u64, CancellationToken)>,
    last_read: u64,
    touched: u64,
}
impl Session {
    fn clear(&mut self) {
        self.auth.cancel();
        self.auth = CancellationToken::new();
        self.authenticating = false;
        self.epoch += 1;
        self.token = None;
        self.expires = None;
        if let Some((_, token)) = self.read.take() {
            token.cancel();
        }
    }
    fn view(&self, store: &'static str) -> SessionView {
        SessionView {
            profile_id: self.profile.id.clone(),
            session_id: self.id.clone(),
            status: if self.expired {
                "expired"
            } else if self.token.is_some() {
                "connected"
            } else {
                "signedOut"
            },
            expires_at: self.expires,
            credential_store: store,
        }
    }
}
struct AuthWork {
    profile: Profile,
    epoch: u64,
    cancel: CancellationToken,
}
struct ReadWork {
    profile: Profile,
    token: Zeroizing<String>,
    epoch: u64,
    cancel: CancellationToken,
}

pub struct Engine {
    store: Arc<storage::Store>,
    credentials: Arc<dyn CredentialStore>,
    provider: Arc<dyn SecretProvider>,
    binding: String,
    sessions: Mutex<HashMap<String, Session>>,
    credential_gate: tokio::sync::Mutex<()>,
    stop: CancellationToken,
    tasks: TaskTracker,
}
impl Engine {
    pub fn new(directory: PathBuf) -> Arc<Self> {
        Self::with_store(directory, Arc::new(NativeStore))
    }
    fn with_store(directory: PathBuf, credentials: Arc<dyn CredentialStore>) -> Arc<Self> {
        Self::with_provider(directory, credentials, Arc::new(VaultProvider))
    }
    fn with_provider(
        directory: PathBuf,
        credentials: Arc<dyn CredentialStore>,
        provider: Arc<dyn SecretProvider>,
    ) -> Arc<Self> {
        let binding = format!(
            "{:x}",
            Sha256::digest(directory.as_os_str().as_encoded_bytes())
        );
        Arc::new(Self {
            store: Arc::new(storage::Store::new(directory)),
            credentials,
            provider,
            binding,
            sessions: Mutex::new(HashMap::new()),
            credential_gate: tokio::sync::Mutex::new(()),
            stop: CancellationToken::new(),
            tasks: TaskTracker::new(),
        })
    }
    pub fn start(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let stop = self.stop.clone();
        // Native setup runs outside an entered Tokio runtime.
        self.tasks.spawn_on(
            async move {
                loop {
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    }
                    let Some(engine) = weak.upgrade() else { break };
                    if let Ok(mut sessions) = engine.sessions.lock() {
                        for session in sessions.values_mut() {
                            if !session.expired
                                && (session.expires.is_some_and(|expiry| expiry <= now())
                                    || now().saturating_sub(session.touched) > 30 * 60 * 1000)
                            {
                                session.clear();
                                session.expired = true;
                            }
                        }
                    }
                }
            },
            tauri::async_runtime::handle().inner(),
        );
    }
    pub fn shutdown(&self) {
        self.stop.cancel();
        if let Ok(mut sessions) = self.sessions.lock() {
            for session in sessions.values_mut() {
                session.clear();
            }
            sessions.clear();
        }
        self.tasks.close();
    }
    pub async fn drain(&self) {
        self.tasks.wait().await;
    }
    fn key(&self, p: &Profile) -> String {
        let identity = serde_json::to_vec(p).unwrap_or_default();
        let mut hash = Sha256::new();
        hash.update(b"iceon-secrets-v1\0");
        hash.update(self.binding.as_bytes());
        hash.update(b"\0");
        hash.update(identity);
        format!("{:x}", hash.finalize())
    }
    pub async fn profiles(&self) -> Result<Vec<Profile>> {
        let store = self.store.clone();
        blocking(move || store.list()).await
    }
    async fn profile(&self, id: &str) -> Result<Profile> {
        if !valid_id(id) {
            return Err(Error::InvalidInput);
        }
        self.profiles()
            .await?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or(Error::NotFound)
    }
    pub async fn save(&self, input: ProfileInput) -> Result<Profile> {
        let p = Profile {
            id: new_id()?,
            provider: input.provider,
            name: input.name.trim().into(),
            address: input.address,
            namespace: input.namespace,
            auth_mount: input.auth_mount,
            kv_mount: input.kv_mount,
            base_path: input.base_path,
        };
        validate_profile(&p)?;
        let store = self.store.clone();
        let saved = p.clone();
        blocking(move || store.insert(&saved)).await?;
        Ok(p)
    }
    pub async fn remove(&self, id: String) -> Result<()> {
        let _gate = self.credential_gate.lock().await;
        let p = self.profile(&id).await?;
        {
            let mut sessions = self.sessions.lock().map_err(|_| Error::StoreUnavailable)?;
            sessions.retain(|_, s| {
                if s.profile.id == id {
                    s.clear();
                    false
                } else {
                    true
                }
            });
        }
        let key = self.key(&p);
        let credentials = self.credentials.clone();
        blocking(move || credentials.delete(&key)).await?;
        let store = self.store.clone();
        blocking(move || store.remove(&id)).await
    }
    pub async fn open(&self, window: &str, id: &str, owner: String) -> Result<SessionView> {
        if owner.is_empty()
            || owner.len() > 64
            || !owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(Error::InvalidInput);
        }
        let _gate = self.credential_gate.lock().await;
        let profile = self.profile(id).await?;
        if self.stop.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let mut sessions = self.sessions.lock().map_err(|_| Error::StoreUnavailable)?;
        if let Some(s) = sessions.get(window)
            && s.owner == owner
            && s.profile.id == id
        {
            return Ok(s.view(self.credentials.kind()));
        }
        if !sessions.contains_key(window) && sessions.len() >= 8 {
            return Err(Error::Busy);
        }
        if let Some(mut previous) = sessions.remove(window) {
            previous.clear();
        }
        let s = Session {
            window: window.into(),
            owner,
            profile,
            id: new_id()?,
            token: None,
            expires: None,
            expired: false,
            epoch: 0,
            auth: CancellationToken::new(),
            authenticating: false,
            read: None,
            last_read: 0,
            touched: now(),
        };
        let view = s.view(self.credentials.kind());
        sessions.insert(window.into(), s);
        Ok(view)
    }
    fn session<'a>(
        sessions: &'a mut HashMap<String, Session>,
        window: &str,
        request: &SessionRequest,
    ) -> Result<&'a mut Session> {
        let s = sessions.get_mut(window).ok_or(Error::StaleSession)?;
        if s.window != window || s.id != request.session_id || s.profile.id != request.profile_id {
            return Err(Error::StaleSession);
        }
        Ok(s)
    }
    fn begin_auth(&self, window: &str, request: &SessionRequest) -> Result<AuthWork> {
        let mut sessions = self.sessions.lock().map_err(|_| Error::StoreUnavailable)?;
        let s = Self::session(&mut sessions, window, request)?;
        if s.authenticating {
            return Err(Error::Busy);
        }
        s.clear();
        s.expired = false;
        s.authenticating = true;
        s.touched = now();
        Ok(AuthWork {
            profile: s.profile.clone(),
            epoch: s.epoch,
            cancel: s.auth.clone(),
        })
    }
    fn check_auth(&self, window: &str, request: &SessionRequest, work: &AuthWork) -> Result<()> {
        let mut sessions = self.sessions.lock().map_err(|_| Error::StoreUnavailable)?;
        let s = Self::session(&mut sessions, window, request)?;
        if !s.authenticating || s.epoch != work.epoch || work.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
    fn finish_auth(
        &self,
        window: &str,
        request: &SessionRequest,
        work: &AuthWork,
        auth: Result<vault::Auth>,
    ) -> Result<SessionView> {
        let mut sessions = self.sessions.lock().map_err(|_| Error::StoreUnavailable)?;
        let s = Self::session(&mut sessions, window, request)?;
        if !s.authenticating || s.epoch != work.epoch || work.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        s.authenticating = false;
        let auth = auth?;
        if auth.expires_at.is_some_and(|e| e <= now()) {
            return Err(Error::SessionExpired);
        }
        s.token = Some(auth.token);
        s.expires = auth.expires_at;
        s.touched = now();
        Ok(s.view(self.credentials.kind()))
    }
    pub async fn login(&self, window: &str, mut input: LoginRequest) -> Result<SessionView> {
        let password = Zeroizing::new(std::mem::take(&mut input.password));
        let request = SessionRequest {
            profile_id: input.profile_id,
            session_id: input.session_id,
        };
        let work = self.begin_auth(window, &request)?;
        let result = self
            .provider
            .login(&work.profile, &input.username, &password, &work.cancel)
            .await;
        let _gate = self.credential_gate.lock().await;
        let mut saved_key = None;
        let result = async {
            let auth = result?;
            self.check_auth(window, &request, &work)?;
            if input.remember {
                let key = self.key(&work.profile);
                let credentials = self.credentials.clone();
                let token = auth.token.clone();
                let save_key = key.clone();
                blocking(move || credentials.set(&save_key, &token)).await?;
                saved_key = Some(key);
            }
            Ok(auth)
        }
        .await;
        let result = self.finish_auth(window, &request, &work, result);
        if result.is_err()
            && let Some(key) = saved_key
        {
            let credentials = self.credentials.clone();
            blocking(move || credentials.delete(&key)).await?;
        }
        result
    }

    pub async fn resume(&self, window: &str, request: SessionRequest) -> Result<SessionView> {
        let work = self.begin_auth(window, &request)?;
        let result = async {
            let token = {
                let _gate = self.credential_gate.lock().await;
                self.check_auth(window, &request, &work)?;
                let credentials = self.credentials.clone();
                let key = self.key(&work.profile);
                blocking(move || credentials.get(&key)).await?
            };
            let expires_at = self
                .provider
                .resume(&work.profile, &token, &work.cancel)
                .await?;
            Ok(vault::Auth { token, expires_at })
        }
        .await;
        self.finish_auth(window, &request, &work, result)
    }
    pub async fn disconnect(&self, window: &str, input: DisconnectRequest) -> Result<()> {
        let request = SessionRequest {
            profile_id: input.profile_id,
            session_id: input.session_id,
        };
        let _gate = if input.forget {
            Some(self.credential_gate.lock().await)
        } else {
            None
        };
        let profile = {
            let mut sessions = self.sessions.lock().map_err(|_| Error::StoreUnavailable)?;
            let s = Self::session(&mut sessions, window, &request)?;
            s.clear();
            s.expired = false;
            s.touched = now();
            s.profile.clone()
        };
        if input.forget {
            let credentials = self.credentials.clone();
            let key = self.key(&profile);
            blocking(move || credentials.delete(&key)).await?;
        }
        Ok(())
    }
    pub fn close(&self, window: &str, request: &SessionRequest) -> Result<()> {
        let mut sessions = self.sessions.lock().map_err(|_| Error::StoreUnavailable)?;
        if !sessions.contains_key(window) {
            return Ok(());
        }
        Self::session(&mut sessions, window, request)?.clear();
        sessions.remove(window);
        Ok(())
    }
    pub fn close_window(&self, window: &str) {
        if let Ok(mut sessions) = self.sessions.lock()
            && let Some(mut s) = sessions.remove(window)
        {
            s.clear();
        }
    }
    fn begin_read(&self, window: &str, r: &ReadRequest) -> Result<ReadWork> {
        if r.read_id == 0 || r.read_id > 9_007_199_254_740_991 || !valid_path(&r.path, true) {
            return Err(Error::InvalidInput);
        }
        let request = SessionRequest {
            profile_id: r.profile_id.clone(),
            session_id: r.session_id.clone(),
        };
        let mut sessions = self.sessions.lock().map_err(|_| Error::StoreUnavailable)?;
        let s = Self::session(&mut sessions, window, &request)?;
        if s.expires.is_some_and(|e| e <= now()) {
            s.clear();
            s.expired = true;
        }
        if s.expired {
            return Err(Error::SessionExpired);
        }
        if s.authenticating {
            return Err(Error::Busy);
        }
        let token = s.token.as_ref().ok_or(Error::AuthFailed)?.clone();
        if r.read_id <= s.last_read {
            return Err(Error::Cancelled);
        }
        if let Some((_, old)) = s.read.take() {
            old.cancel();
        }
        let cancel = CancellationToken::new();
        s.read = Some((r.read_id, cancel.clone()));
        s.last_read = r.read_id;
        s.touched = now();
        Ok(ReadWork {
            profile: s.profile.clone(),
            token,
            epoch: s.epoch,
            cancel,
        })
    }
    fn finish_read(
        &self,
        window: &str,
        r: &ReadRequest,
        work: &ReadWork,
        result: Result<()>,
    ) -> Result<()> {
        let request = SessionRequest {
            profile_id: r.profile_id.clone(),
            session_id: r.session_id.clone(),
        };
        let mut sessions = self.sessions.lock().map_err(|_| Error::StoreUnavailable)?;
        let s = Self::session(&mut sessions, window, &request)?;
        if s.epoch != work.epoch
            || work.cancel.is_cancelled()
            || s.read.as_ref().map(|r| r.0) != Some(r.read_id)
        {
            return Err(Error::Cancelled);
        }
        s.read = None;
        if s.expires.is_some_and(|e| e <= now()) || result == Err(Error::SessionExpired) {
            s.clear();
            s.expired = true;
            return Err(Error::SessionExpired);
        }
        result
    }
    pub async fn list(&self, window: &str, r: ReadRequest) -> Result<Listing> {
        let work = self.begin_read(window, &r)?;
        let result = async {
            self.provider
                .list(&work.profile, &work.token, &r.path, &work.cancel)
                .await
        }
        .await;
        self.finish_read(
            window,
            &r,
            &work,
            result.as_ref().map(|_| ()).map_err(|e| *e),
        )?;
        let (entries, partial) = result?;
        Ok(Listing {
            profile_id: r.profile_id,
            session_id: r.session_id,
            read_id: r.read_id,
            path: r.path,
            entries,
            partial,
        })
    }
    pub async fn reveal(&self, window: &str, r: ReadRequest) -> Result<Reveal> {
        if r.path.is_empty() {
            return Err(Error::InvalidInput);
        }
        let work = self.begin_read(window, &r)?;
        let result = async {
            self.provider
                .reveal(&work.profile, &work.token, &r.path, &work.cancel)
                .await
        }
        .await;
        self.finish_read(
            window,
            &r,
            &work,
            result.as_ref().map(|_| ()).map_err(|e| *e),
        )?;
        let (fields, version) = result?;
        Ok(Reveal {
            profile_id: r.profile_id,
            session_id: r.session_id,
            read_id: r.read_id,
            path: r.path,
            fields,
            version,
            expires_at: now() + 30_000,
        })
    }
    pub fn cancel(&self, window: &str, r: CancelRequest) -> Result<()> {
        if r.read_id == 0 || r.read_id > 9_007_199_254_740_991 {
            return Err(Error::InvalidInput);
        }
        let request = SessionRequest {
            profile_id: r.profile_id,
            session_id: r.session_id,
        };
        let mut sessions = self.sessions.lock().map_err(|_| Error::StoreUnavailable)?;
        let s = Self::session(&mut sessions, window, &request)?;
        s.last_read = s.last_read.max(r.read_id);
        if s.read.as_ref().is_some_and(|(id, _)| *id == r.read_id)
            && let Some((_, cancel)) = s.read.take()
        {
            cancel.cancel();
        }
        Ok(())
    }
}
async fn blocking<T: Send + 'static>(
    action: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(action)
        .await
        .map_err(|_| Error::StoreUnavailable)?
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod provider_tests;
