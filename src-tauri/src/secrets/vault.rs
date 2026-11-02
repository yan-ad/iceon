use super::{Entry, Error, Field, Profile, Result};
use futures::StreamExt;
use reqwest::{Method, StatusCode, Url, header::HeaderValue};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroize, Zeroizing};

const RESPONSE_LIMIT: usize = 1024 * 1024;
const ENTRY_LIMIT: usize = 1000;
const TOKEN_LIMIT: usize = 16 * 1024;
const PATH_LIMIT: usize = 4096;
const NAME_LIMIT: usize = 1024;

pub(super) struct Vault {
    client: reqwest::Client,
}
// Intentionally not Debug: this value owns a bearer credential.
pub(super) struct Auth {
    pub(super) token: Zeroizing<String>,
    pub(super) expires_at: Option<u64>,
}

impl Vault {
    pub(super) fn new() -> Result<Self> {
        Self::build(true)
    }
    fn build(https_only: bool) -> Result<Self> {
        crate::tls::initialize();
        Ok(Self {
            client: reqwest::Client::builder()
                .https_only(https_only)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(15))
                .build()
                .map_err(|_| Error::Network)?,
        })
    }
    #[cfg(test)]
    fn loopback() -> Self {
        Self::build(false).unwrap()
    }

    pub(super) async fn login(
        &self,
        profile: &Profile,
        username: &str,
        password: &str,
        cancel: &CancellationToken,
    ) -> Result<Auth> {
        if username.is_empty()
            || username.len() > NAME_LIMIT
            || username.chars().any(char::is_control)
            || username == "."
            || username == ".."
            || password.is_empty()
            || password.len() > TOKEN_LIMIT
        {
            return Err(Error::InvalidInput);
        }
        let mut segments = vec!["auth"];
        segments.extend(scope(&profile.auth_mount, false)?);
        segments.extend(["login", username]);
        let url = endpoint(profile, &segments)?;
        #[derive(Serialize)]
        struct Login<'a> {
            password: &'a str,
        }
        let body = Zeroizing::new(
            serde_json::to_vec(&Login { password }).map_err(|_| Error::InvalidInput)?,
        );
        // reqwest owns its outgoing copy until dispatch completes.
        let request = self
            .request(profile, Method::POST, url, None)?
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_vec());
        let response: LoginResponse = self.send(request, true, cancel).await?;
        let mut auth = response.auth.ok_or(Error::InvalidResponse)?;
        if auth.mfa_requirement.is_some() {
            return Err(Error::UnsupportedAuth);
        }
        check_policies(&auth.policies)?;
        let token = Zeroizing::new(std::mem::take(&mut auth.client_token));
        token_header(&token).map_err(|_| Error::InvalidResponse)?;
        Ok(Auth {
            token,
            expires_at: expiration(auth.lease_duration)?,
        })
    }
    pub(super) async fn resume(
        &self,
        profile: &Profile,
        token: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<u64>> {
        let url = endpoint(profile, &["auth", "token", "lookup-self"])?;
        let response: LookupResponse = self
            .send(
                self.request(profile, Method::GET, url, Some(token))?,
                false,
                cancel,
            )
            .await?;
        check_policies(&response.data.policies)?;
        expiration(response.data.ttl)
    }
    pub(super) async fn list(
        &self,
        profile: &Profile,
        token: &str,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<(Vec<Entry>, bool)> {
        let url = kv_endpoint(profile, "metadata", path, true)?;
        let method = Method::from_bytes(b"LIST").map_err(|_| Error::InvalidInput)?;
        let response: ListResponse = self
            .send(
                self.request(profile, method, url, Some(token))?,
                false,
                cancel,
            )
            .await?;
        let partial = response.data.keys.len() > ENTRY_LIMIT;
        let mut entries = Vec::with_capacity(response.data.keys.len().min(ENTRY_LIMIT));
        for name in response.data.keys {
            let directory = name.ends_with('/');
            let child = if directory {
                &name[..name.len() - 1]
            } else {
                &name
            };
            if !valid_segment(child) {
                return Err(Error::InvalidResponse);
            }
            if entries.len() < ENTRY_LIMIT {
                entries.push(Entry {
                    name: child.to_owned(),
                    directory,
                });
            }
        }
        Ok((entries, partial))
    }
    pub(super) async fn reveal(
        &self,
        profile: &Profile,
        token: &str,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<(Vec<Field>, u64)> {
        let url = kv_endpoint(profile, "data", path, false)?;
        let response: ReadResponse = self
            .send(
                self.request(profile, Method::GET, url, Some(token))?,
                false,
                cancel,
            )
            .await?;
        let metadata = response.data.metadata;
        if metadata.destroyed || !metadata.deletion_time.is_empty() {
            return Err(Error::NotFound);
        }
        if metadata.version == 0 {
            return Err(Error::InvalidResponse);
        }
        if response.data.data.len() > 256 {
            return Err(Error::TooLarge);
        }
        let mut fields = Vec::with_capacity(response.data.data.len());
        for (name, value) in response.data.data {
            if name.is_empty() || name.len() > NAME_LIMIT || name.chars().any(char::is_control) {
                return Err(Error::InvalidResponse);
            }
            let value = match value {
                serde_json::Value::String(value) => value,
                other => serde_json::to_string(&other).map_err(|_| Error::InvalidResponse)?,
            };
            fields.push(Field { name, value });
        }
        Ok((fields, metadata.version))
    }
    fn request(
        &self,
        profile: &Profile,
        method: Method,
        url: Url,
        token: Option<&str>,
    ) -> Result<reqwest::RequestBuilder> {
        let mut request = self
            .client
            .request(method, url)
            .header(reqwest::header::ACCEPT, "application/json");
        if !profile.namespace.is_empty() {
            if profile.namespace.len() > PATH_LIMIT
                || profile.namespace.chars().any(char::is_control)
            {
                return Err(Error::InvalidInput);
            }
            request = request.header(
                "X-Vault-Namespace",
                HeaderValue::from_str(&profile.namespace).map_err(|_| Error::InvalidInput)?,
            );
        }
        if let Some(token) = token {
            request = request.header("X-Vault-Token", token_header(token)?);
        }
        Ok(request)
    }
    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        login: bool,
        cancel: &CancellationToken,
    ) -> Result<T> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(Error::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(15), async {
                let response = request.send().await.map_err(network_error)?;
                status(response.status(), login)?;
                if response.content_length().is_some_and(|length| length > RESPONSE_LIMIT as u64) { return Err(Error::TooLarge); }
                let mut stream = response.bytes_stream();
                let mut bytes = Zeroizing::new(Vec::new());
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(network_error)?;
                    if chunk.len() > RESPONSE_LIMIT - bytes.len() { return Err(Error::TooLarge); }
                    bytes.extend_from_slice(&chunk);
                }
                serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)
            }) => result.unwrap_or(Err(Error::Network)),
        }
    }
}

fn token_header(token: &str) -> Result<HeaderValue> {
    if token.is_empty()
        || token.len() > TOKEN_LIMIT
        || !token.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(Error::InvalidInput);
    }
    let mut value = HeaderValue::from_str(token).map_err(|_| Error::InvalidInput)?;
    value.set_sensitive(true);
    Ok(value)
}
fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= NAME_LIMIT
        && value != "."
        && value != ".."
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | '%'))
}
fn scope(value: &str, empty: bool) -> Result<Vec<&str>> {
    if value.is_empty() && empty {
        return Ok(Vec::new());
    }
    if value.len() > PATH_LIMIT || !value.split('/').all(valid_segment) {
        return Err(Error::InvalidInput);
    }
    Ok(value.split('/').collect())
}
fn endpoint(profile: &Profile, segments: &[&str]) -> Result<Url> {
    let mut url = Url::parse(&profile.address).map_err(|_| Error::InvalidInput)?;
    let secure = url.scheme() == "https";
    #[cfg(test)]
    let secure =
        secure || (url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "[::1]")));
    if !secure
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(Error::InvalidInput);
    }
    url.path_segments_mut()
        .map_err(|_| Error::InvalidInput)?
        .clear()
        .push("v1")
        .extend(segments);
    Ok(url)
}
fn kv_endpoint(profile: &Profile, kind: &str, path: &str, empty: bool) -> Result<Url> {
    let mut segments = scope(&profile.kv_mount, false)?;
    segments.push(kind);
    segments.extend(scope(&profile.base_path, true)?);
    segments.extend(scope(path, empty)?);
    endpoint(profile, &segments)
}
fn check_policies(policies: &[String]) -> Result<()> {
    if policies.is_empty() {
        return Err(Error::InvalidResponse);
    }
    if policies.iter().any(|policy| policy == "root") {
        return Err(Error::UnsupportedAuth);
    }
    Ok(())
}
fn expiration(ttl: u64) -> Result<Option<u64>> {
    // Indefinite credentials are not supported for userpass login or resume.
    if ttl == 0 {
        return Err(Error::UnsupportedAuth);
    }
    if ttl > u32::MAX as u64 {
        return Err(Error::InvalidResponse);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::InvalidResponse)?
        .as_millis();
    let now = u64::try_from(now).map_err(|_| Error::InvalidResponse)?;
    Ok(Some(
        now.checked_add(ttl * 1000).ok_or(Error::InvalidResponse)?,
    ))
}
fn status(status: StatusCode, login: bool) -> Result<()> {
    match status.as_u16() {
        200..=299 => Ok(()),
        401 | 403 if login => Err(Error::AuthFailed),
        401 => Err(Error::SessionExpired),
        403 => Err(Error::PermissionDenied),
        404 => Err(Error::NotFound),
        429 => Err(Error::Busy),
        500..=599 => Err(Error::Network),
        _ => Err(Error::InvalidResponse),
    }
}
fn network_error(error: reqwest::Error) -> Error {
    use std::error::Error as _;
    let mut cause = error.source();
    while let Some(current) = cause {
        if current.is::<rustls::Error>() {
            return Error::Tls;
        }
        cause = current.source();
    }
    Error::Network
}
#[derive(Deserialize)]
struct LoginResponse {
    auth: Option<LoginAuth>,
}
#[derive(Deserialize)]
struct LoginAuth {
    #[serde(default)]
    client_token: String,
    #[serde(default)]
    lease_duration: u64,
    #[serde(default)]
    policies: Vec<String>,
    #[serde(default)]
    mfa_requirement: Option<serde::de::IgnoredAny>,
}
impl Drop for LoginAuth {
    fn drop(&mut self) {
        self.client_token.zeroize();
    }
}
#[derive(Deserialize)]
struct LookupResponse {
    data: LookupData,
}
#[derive(Deserialize)]
struct LookupData {
    ttl: u64,
    policies: Vec<String>,
}
#[derive(Deserialize)]
struct ListResponse {
    data: ListData,
}
#[derive(Deserialize)]
struct ListData {
    keys: Vec<String>,
}
#[derive(Deserialize)]
struct ReadResponse {
    data: ReadData,
}
#[derive(Deserialize)]
struct ReadData {
    data: BTreeMap<String, serde_json::Value>,
    metadata: Metadata,
}
#[derive(Deserialize)]
struct Metadata {
    version: u64,
    destroyed: bool,
    deletion_time: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn server(
        status: u16,
        headers: &str,
        body: String,
    ) -> (Url, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!(
            "http://{}/v1/test",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let headers = headers.to_owned();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let length = stream.read(&mut chunk).await.unwrap();
                if length == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..length]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 {status} Test\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            String::from_utf8(request).unwrap()
        });
        (url, task)
    }

    #[test]
    fn validation_and_sensitive_headers() {
        for path in ["..", ".", "%2e", "a//b", "/root", "a/", "a\\b", "a\nb"] {
            assert!(scope(path, false).is_err());
        }
        assert!(scope("", true).unwrap().is_empty());
        assert!(scope("team/sub dir", false).is_ok());
        assert!(token_header("fixture-token").unwrap().is_sensitive());
        for token in ["", "bad\r\nheader", "bad token"] {
            assert!(token_header(token).is_err());
        }
        assert!(matches!(expiration(0), Err(Error::UnsupportedAuth)));
        assert!(expiration(1).unwrap().is_some());
        assert!(expiration(u64::MAX).is_err());
        assert!(matches!(
            check_policies(&["root".into()]),
            Err(Error::UnsupportedAuth)
        ));
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_pending_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, pending) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
                .await
                .unwrap();
            ready.send(()).unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let vault = Vault::loopback();
        let cancel = CancellationToken::new();
        let request = vault.client.get(format!("http://{address}/v1/test"));
        let cancellation = cancel.clone();
        let stopper = tokio::spawn(async move {
            pending.await.unwrap();
            cancellation.cancel();
        });
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            vault.send::<ListResponse>(request, false, &cancel),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(Error::Cancelled)));
        stopper.await.unwrap();
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn chunked_bodies_are_bounded_without_content_length() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            let body = "x".repeat(RESPONSE_LIMIT + 1);
            let _ = stream
                .write_all(format!("{:x}\r\n{body}\r\n0\r\n\r\n", body.len()).as_bytes())
                .await;
        });
        let vault = Vault::loopback();
        let result = vault
            .send::<ListResponse>(
                vault.client.get(format!("http://{address}/v1/test")),
                false,
                &CancellationToken::new(),
            )
            .await;
        assert!(matches!(result, Err(Error::TooLarge)));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn bounded_read_and_wire_contract() {
        let vault = Vault::loopback();
        let (url, task) = server(
            200,
            "",
            "{\"data\":{\"keys\":[\"folder/\",\"key\"]}}".into(),
        )
        .await;
        let request = vault
            .client
            .request(Method::from_bytes(b"LIST").unwrap(), url)
            .header("X-Vault-Token", token_header("fixture-token").unwrap())
            .header("X-Vault-Namespace", "team");
        let data: ListResponse = vault
            .send(request, false, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(data.data.keys.len(), 2);
        let request = task.await.unwrap().to_lowercase();
        assert!(request.starts_with("list /v1/test http/1.1"));
        assert!(request.contains("x-vault-token: fixture-token"));
        assert!(request.contains("x-vault-namespace: team"));
        let (url, task) = server(200, "", "x".repeat(RESPONSE_LIMIT + 1)).await;
        assert!(matches!(
            vault
                .send::<ListResponse>(vault.client.get(url), false, &CancellationToken::new())
                .await,
            Err(Error::TooLarge)
        ));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn redirects_errors_and_cancellation_are_sanitized() {
        let vault = Vault::loopback();
        for (code, login) in [
            (302, false),
            (401, false),
            (403, false),
            (401, true),
            (404, false),
            (429, false),
            (500, false),
        ] {
            let (url, task) = server(
                code,
                "Location: http://127.0.0.1:1/credential-leak\r\n",
                "sensitive-server-error".into(),
            )
            .await;
            let result = vault
                .send::<ListResponse>(vault.client.get(url), login, &CancellationToken::new())
                .await;
            assert!(matches!(
                (code, login, result),
                (302, _, Err(Error::InvalidResponse))
                    | (401, false, Err(Error::SessionExpired))
                    | (403, false, Err(Error::PermissionDenied))
                    | (401, true, Err(Error::AuthFailed))
                    | (404, _, Err(Error::NotFound))
                    | (429, _, Err(Error::Busy))
                    | (500, _, Err(Error::Network))
            ));
            task.await.unwrap();
        }
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            vault
                .send::<ListResponse>(vault.client.get("http://127.0.0.1:1"), false, &cancel)
                .await,
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn mfa_response_needs_no_token() {
        let response: LoginResponse =
            serde_json::from_str(r#"{"auth":{"mfa_requirement":{"mfa_request_id":"fixture"}}}"#)
                .unwrap();
        assert!(response.auth.unwrap().mfa_requirement.is_some());
        for child in ["../", "a/b", "//", "%2f", "bad\n"] {
            assert!(!valid_segment(child.strip_suffix('/').unwrap_or(child)));
        }
        let data: ReadResponse = serde_json::from_str(r#"{"data":{"data":{"text":"raw","number":42},"metadata":{"version":3,"destroyed":false,"deletion_time":""}}}"#).unwrap();
        assert_eq!(data.data.metadata.version, 3);
        assert_eq!(data.data.data["text"], "raw");
    }
}
