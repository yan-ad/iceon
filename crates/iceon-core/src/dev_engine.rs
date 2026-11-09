//! Opt-in debug transport. Domain work stays in the same Rust engines as Tauri.
use crate::{catalog::CatalogEngine, connection::ConnectionEngine, preferences::PreferencesEngine};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{RwLock, Semaphore};

const BODY_LIMIT: usize = 2 * 1024 * 1024;
const RESPONSE_LIMIT: usize = 16 * 1024 * 1024;
const OWNER_TTL: Duration = Duration::from_secs(30);
type Result<T> = std::result::Result<T, Value>;

struct Owner {
    client: String,
    cluster: String,
    session: String,
    seen: Mutex<Instant>,
}
struct Engine {
    stopping: AtomicBool,
    token: String,
    host: String,
    frontend_port: u16,
    catalog: Arc<CatalogEngine>,
    connections: Arc<ConnectionEngine>,
    preferences: Arc<PreferencesEngine>,
    owner: RwLock<Option<Owner>>,
    permits: Semaphore,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    command: String,
    args: serde_json::Map<String, Value>,
}
fn error(code: &str) -> Value {
    Value::String(code.into())
}
fn encoded<T: Serialize, E: Serialize>(result: std::result::Result<T, E>) -> Result<Value> {
    match result {
        Ok(value) => serde_json::to_value(value).map_err(|_| error("invalidResponse")),
        Err(code) => Err(serde_json::to_value(code).unwrap_or_else(|_| error("storeUnavailable"))),
    }
}
fn response(status: StatusCode, result: Result<Value>) -> Response {
    let value = match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    let mut response = match serde_json::to_vec(&value) {
        Ok(bytes) if bytes.len() <= RESPONSE_LIMIT => {
            (status, [("content-type", "application/json")], bytes).into_response()
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"ok":false,"error":"limitExceeded"})),
        )
            .into_response(),
    };
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
}
fn valid_client(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
async fn guard(State(state): State<Arc<Engine>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let exact = |name: &str, expected: &str| {
        let mut values = headers.get_all(name).iter();
        values
            .next()
            .is_some_and(|v| v.as_bytes() == expected.as_bytes())
            && values.next().is_none()
    };
    let authorized = exact("host", &state.host)
        && exact("authorization", &format!("Bearer {}", state.token))
        && (exact(
            "origin",
            &format!("http://127.0.0.1:{}", state.frontend_port),
        ) || exact(
            "origin",
            &format!("http://localhost:{}", state.frontend_port),
        ))
        && exact("x-iceon-dev", "1")
        && headers
            .get("x-iceon-client")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| valid_client(s) && exact("x-iceon-client", s));
    if !authorized {
        return response(StatusCode::FORBIDDEN, Err(error("unauthorized")));
    }
    if request.method() != Method::POST {
        return response(StatusCode::METHOD_NOT_ALLOWED, Err(error("invalidRequest")));
    }
    if !matches!(request.uri().path(), "/invoke" | "/heartbeat") || request.uri().query().is_some()
    {
        return response(StatusCode::NOT_FOUND, Err(error("notFound")));
    }
    let mut content_types = headers.get_all("content-type").iter();
    let json_content_type = content_types.next().is_some_and(|value| {
        value.to_str().is_ok_and(|value| {
            let mut parts = value.split(';').map(str::trim);
            parts
                .next()
                .is_some_and(|mime| mime.eq_ignore_ascii_case("application/json"))
                && parts.all(|parameter| parameter.eq_ignore_ascii_case("charset=utf-8"))
        })
    }) && content_types.next().is_none();
    if !json_content_type || headers.contains_key("content-encoding") {
        return response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Err(error("invalidRequest")),
        );
    }

    let Ok(_permit) = state.permits.try_acquire() else {
        return response(StatusCode::TOO_MANY_REQUESTS, Err(error("busy")));
    };
    let (parts, body) = request.into_parts();
    let bytes = match tokio::time::timeout(Duration::from_secs(5), to_bytes(body, BODY_LIMIT)).await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return response(StatusCode::PAYLOAD_TOO_LARGE, Err(error("limitExceeded"))),
        Err(_) => return response(StatusCode::REQUEST_TIMEOUT, Err(error("timeout"))),
    };
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}
fn router(state: Arc<Engine>) -> Router {
    Router::new()
        .route("/invoke", post(invoke))
        .route("/heartbeat", post(heartbeat))
        .fallback(|| async { response(StatusCode::NOT_FOUND, Err(error("notFound"))) })
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}
async fn heartbeat(State(state): State<Arc<Engine>>, request: Request) -> Response {
    let client = request.headers()["x-iceon-client"]
        .to_str()
        .unwrap_or_default()
        .to_owned();
    let bytes = to_bytes(request.into_body(), BODY_LIMIT)
        .await
        .unwrap_or_default();
    if serde_json::from_slice::<Value>(&bytes).ok() != Some(json!({})) {
        return response(StatusCode::BAD_REQUEST, Err(error("invalidInput")));
    }
    let owner = state.owner.read().await;
    let result = match owner.as_ref() {
        Some(owner) if owner.client == client => {
            *owner.seen.lock().unwrap() = Instant::now();
            Ok(Value::Null)
        }
        Some(_) => Err(error("busy")),
        None => Ok(Value::Null),
    };
    response(StatusCode::OK, result)
}
async fn invoke(State(state): State<Arc<Engine>>, request: Request) -> Response {
    let client = request.headers()["x-iceon-client"]
        .to_str()
        .unwrap_or_default()
        .to_owned();
    let bytes = to_bytes(request.into_body(), BODY_LIMIT)
        .await
        .unwrap_or_default();
    let Ok(invocation) = serde_json::from_slice::<Invocation>(&bytes) else {
        return response(StatusCode::BAD_REQUEST, Err(error("invalidInput")));
    };
    response(StatusCode::OK, dispatch(state, &client, invocation).await)
}
fn keys(args: &serde_json::Map<String, Value>, expected: &[&str]) -> Result<()> {
    if args.len() != expected.len() || !expected.iter().all(|key| args.contains_key(*key)) {
        return Err(error("invalidInput"));
    }
    Ok(())
}
fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(|_| error("invalidInput"))
}
async fn blocking<T: Serialize + Send + 'static, E: Serialize + Send + 'static>(
    f: impl FnOnce() -> std::result::Result<T, E> + Send + 'static,
) -> Result<Value> {
    encoded(
        tokio::task::spawn_blocking(f)
            .await
            .map_err(|_| error("storeUnavailable"))?,
    )
}
async fn dispatch(state: Arc<Engine>, client: &str, invocation: Invocation) -> Result<Value> {
    if state.stopping.load(Ordering::SeqCst) {
        return Err(error("staleSession"));
    }
    let Invocation { command, args } = invocation;
    macro_rules! request {
        () => {{
            keys(&args, &["request"])?;
            parse(&args["request"])?
        }};
    }
    if command == "cluster_session_open" {
        let request = request!();
        let mut owner = state.owner.write().await;
        if state.stopping.load(Ordering::SeqCst) {
            return Err(error("staleSession"));
        }
        if owner.as_ref().is_some_and(|o| o.client != client) {
            return Err(error("busy"));
        }
        // Claim and replacement are ordered against expiry and old disposal.
        state.connections.shutdown();
        *owner = None;
        let snapshot = state
            .connections
            .open(request)
            .await
            .map_err(|e| serde_json::to_value(e).unwrap())?;
        *owner = Some(Owner {
            client: client.into(),
            cluster: snapshot.cluster_id.clone(),
            session: snapshot.session_id.clone(),
            seen: Mutex::new(Instant::now()),
        });
        return encoded(Ok::<_, Value>(snapshot));
    }
    if command == "cluster_disconnect" {
        let request: crate::connection::SessionRequest = request!();
        let mut owner = state.owner.write().await;
        let Some(current) = owner.as_ref() else {
            return Err(error("staleSession"));
        };
        if current.client != client {
            return Err(error("busy"));
        }
        if current.cluster != request.cluster_id || current.session != request.session_id {
            return Err(error("staleSession"));
        }
        let result = encoded(state.connections.disconnect(request));
        if result.is_ok() {
            *owner = None;
        }
        return result;
    }
    if command.starts_with("cluster_") || command.starts_with("helm_") {
        let owner = state.owner.read().await;
        match owner.as_ref() {
            Some(o) if o.client == client => {}
            Some(_) => return Err(error("busy")),
            None => return Err(error("staleSession")),
        }
        // Engine request/session IDs reject work racing with a replacement. Do not
        // retain this guard while awaiting IO: disconnect must cancel that IO.
    }
    match command.as_str() {
        "catalog_list" => {
            keys(&args, &[])?;
            let catalog = state.catalog.clone();
            let connections = state.connections.clone();
            blocking(move || {
                let mut snapshot = catalog.list()?;
                connections
                    .project_catalog_statuses(&mut snapshot)
                    .map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?;
                Ok::<_, crate::catalog::CatalogError>(snapshot)
            })
            .await
        }
        "catalog_connection_states" => {
            keys(&args, &[])?;
            encoded(state.connections.catalog_connection_states())
        }
        "preferences_get" => {
            keys(&args, &[])?;
            let engine = state.preferences.clone();
            blocking(move || engine.snapshot()).await
        }
        "preferences_save" => {
            let request = request!();
            let engine = state.preferences.clone();
            blocking(move || engine.save(request)).await
        }
        "route_history_get" => {
            keys(&args, &[])?;
            let engine = state.preferences.clone();
            blocking(move || engine.route_history_get()).await
        }
        "route_history_save" => {
            keys(&args, &["route"])?;
            let route = parse(&args["route"])?;
            let engine = state.preferences.clone();
            blocking(move || engine.route_history_save(route)).await
        }
        "route_history_clear" => {
            keys(&args, &[])?;
            let engine = state.preferences.clone();
            blocking(move || engine.route_history_clear()).await
        }
        // The cache is lease-bound; the retired renderer write endpoint is not exposed.
        "namespace_cache_get" | "namespace_cache_put" => Err(error("unsupportedCommand")),
        "cluster_namespace_cache_get" => {
            let request = request!();
            encoded(state.connections.namespace_cache_get(request).await)
        }
        "cluster_namespace_cache_refresh" => {
            let request = request!();
            encoded(state.connections.namespace_cache_refresh(request).await)
        }
        "cluster_rbac_management_snapshot" => {
            let request = request!();
            encoded(state.connections.rbac_management_snapshot(request).await)
        }
        "cluster_rbac_user_plan" => {
            let request = request!();
            encoded(state.connections.rbac_user_plan(request).await)
        }
        "cluster_rbac_user_apply" => {
            let request = request!();
            encoded(state.connections.rbac_user_apply(request).await)
        }
        "cluster_rbac_management_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_rbac_management(request))
        }
        "cluster_rbac_users" => {
            let request = request!();
            encoded(state.connections.rbac_users(request).await)
        }
        "cluster_rbac_users_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_rbac_users(request))
        }
        "cluster_custom_resource_definitions" => {
            let request = request!();
            encoded(state.connections.custom_resource_definitions(request).await)
        }
        "cluster_custom_resource_definitions_cancel" => {
            let request = request!();
            encoded(
                state
                    .connections
                    .cancel_custom_resource_definitions(request),
            )
        }
        "cluster_custom_resources" => {
            let request = request!();
            encoded(state.connections.custom_resources(request).await)
        }
        "cluster_custom_resources_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_custom_resources(request))
        }
        "helm_install" => {
            let request = request!();
            encoded(state.connections.helm_install(request).await)
        }
        "helm_install_cancel" => {
            let request = request!();
            encoded(state.connections.helm_install_cancel(request))
        }
        "helm_table" => {
            let request = request!();
            encoded(state.connections.helm_table(request).await)
        }
        "helm_cancel" => {
            let request = request!();
            encoded(state.connections.helm_cancel(request))
        }
        "helm_detail" => {
            let request = request!();
            encoded(state.connections.helm_detail(request).await)
        }
        "helm_detail_cancel" => {
            let request = request!();
            encoded(state.connections.helm_detail_cancel(request))
        }
        "cluster_pod_detail" => {
            let request = request!();
            encoded(state.connections.pod_detail(request).await)
        }
        "cluster_pod_mutate" => {
            let request = request!();
            encoded(state.connections.pod_mutate(request).await)
        }
        "cluster_pod_stream_open" => {
            let request = request!();
            encoded(state.connections.pod_stream_open(request).await)
        }
        "cluster_pod_stream_read" => {
            let request = request!();
            encoded(state.connections.pod_stream_read(request))
        }
        "cluster_pod_stream_close" => {
            let request = request!();
            encoded(state.connections.pod_stream_close(request))
        }
        "cluster_pod_stream_write" => {
            let request = request!();
            encoded(state.connections.pod_stream_write(request))
        }
        "cluster_pod_stream_resize" => {
            let request = request!();
            encoded(state.connections.pod_stream_resize(request))
        }
        "cluster_pod_forward_start" => {
            let request = request!();
            encoded(state.connections.pod_forward_start(request).await)
        }
        "cluster_pod_forward_list" => {
            let request = request!();
            encoded(state.connections.pod_forward_list(request))
        }
        "cluster_pod_forward_stop" => {
            let request = request!();
            encoded(state.connections.pod_forward_stop(request))
        }
        "cluster_pod_sync_open" => {
            let request = request!();
            encoded(state.connections.pod_sync_open(request).await)
        }
        "cluster_pod_sync_read" => {
            let request = request!();
            encoded(state.connections.pod_sync_read(request))
        }
        "cluster_pod_sync_close" => {
            let request = request!();
            encoded(state.connections.pod_sync_close(request))
        }
        "cluster_connect" => {
            let request = request!();
            encoded(state.connections.connect(request).await)
        }
        "cluster_connection_status" => {
            let request = request!();
            encoded(state.connections.status(request))
        }
        "cluster_resources" => {
            let request = request!();
            encoded(state.connections.resources(request).await)
        }
        "cluster_permissions_review" => {
            let request = request!();
            encoded(state.connections.review_permissions(request).await)
        }
        "cluster_permissions_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_permissions(request))
        }
        "cluster_permissions_refresh" => {
            let request = request!();
            encoded(state.connections.refresh_permissions(request))
        }
        "cluster_resources_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_resources(request))
        }
        "cluster_resource_detail" => {
            let request = request!();
            encoded(state.connections.resource_detail(request).await)
        }
        "cluster_resource_detail_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_resource_detail(request))
        }
        "cluster_resource_collection" => {
            let request = request!();
            encoded(state.connections.resource_collection(request).await)
        }
        "cluster_resource_collection_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_resource_collection(request))
        }
        "cluster_workload_action" => {
            let request = request!();
            encoded(state.connections.resource_workload_action(request).await)
        }
        "cluster_workload_action_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_workload_action(request))
        }
        "cluster_resource_edit" => {
            let request = request!();
            encoded(state.connections.resource_edit(request).await)
        }
        "cluster_resource_edit_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_resource_edit(request))
        }
        "cluster_service_forward_start" => {
            let request = request!();
            encoded(state.connections.service_forward_start(request).await)
        }
        "cluster_service_forward_cancel" => {
            let request = request!();
            encoded(state.connections.service_forward_cancel(request))
        }
        "cluster_node_action" => {
            let request = request!();
            encoded(state.connections.resource_node_action(request).await)
        }
        "cluster_node_action_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_node_action(request))
        }
        "cluster_node_drain_review" => {
            let request = request!();
            encoded(state.connections.node_drain_review(request).await)
        }
        "cluster_node_drain_execute" => {
            let request = request!();
            encoded(state.connections.node_drain_execute(request).await)
        }
        "cluster_node_drain_cancel" => {
            let request = request!();
            encoded(state.connections.node_drain_cancel(request))
        }
        "cluster_resource_delete" => {
            let request = request!();
            encoded(state.connections.delete(request).await)
        }
        "cluster_resource_delete_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_delete(request))
        }
        "cluster_resource_create" => {
            let request = request!();
            encoded(state.connections.create(request).await)
        }
        "cluster_resource_create_cancel" => {
            let request = request!();
            encoded(state.connections.cancel_create(request))
        }
        "catalog_import_kubeconfig" => {
            let request = request!();
            let catalog = state.catalog.clone();
            let connections = state.connections.clone();
            blocking(move || {
                let mut snapshot = catalog.import_kubeconfig(request)?;
                connections
                    .project_catalog_statuses(&mut snapshot)
                    .map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?;
                Ok::<_, crate::catalog::CatalogError>(snapshot)
            })
            .await
        }
        "catalog_set_pin" => {
            let request = request!();
            let catalog = state.catalog.clone();
            let connections = state.connections.clone();
            blocking(move || {
                let mut snapshot = catalog.set_pin(request)?;
                connections
                    .project_catalog_statuses(&mut snapshot)
                    .map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?;
                Ok::<_, crate::catalog::CatalogError>(snapshot)
            })
            .await
        }
        "catalog_set_section_pin" => {
            let request = request!();
            let catalog = state.catalog.clone();
            let connections = state.connections.clone();
            blocking(move || {
                let mut snapshot = catalog.set_section_pin(request)?;
                connections
                    .project_catalog_statuses(&mut snapshot)
                    .map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?;
                Ok::<_, crate::catalog::CatalogError>(snapshot)
            })
            .await
        }
        "catalog_reorder_pins" => {
            let request = request!();
            let catalog = state.catalog.clone();
            let connections = state.connections.clone();
            blocking(move || {
                let mut snapshot = catalog.reorder_pins(request)?;
                connections
                    .project_catalog_statuses(&mut snapshot)
                    .map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?;
                Ok::<_, crate::catalog::CatalogError>(snapshot)
            })
            .await
        }
        "catalog_remove" => {
            let request = request!();
            let catalog = state.catalog.clone();
            let connections = state.connections.clone();
            blocking(move || {
                let mut snapshot = catalog.remove(request)?;
                connections
                    .project_catalog_statuses(&mut snapshot)
                    .map_err(|_| crate::catalog::CatalogError::StoreUnavailable)?;
                Ok::<_, crate::catalog::CatalogError>(snapshot)
            })
            .await
        }
        _ => Err(error("unknownCommand")),
    }
}

async fn expire(state: &Engine) {
    let mut owner = state.owner.write().await;
    if owner
        .as_ref()
        .is_some_and(|o| o.seen.lock().unwrap().elapsed() >= OWNER_TTL)
    {
        state.connections.shutdown();
        *owner = None;
    }
}
fn absolute_env(name: &str) -> std::result::Result<PathBuf, &'static str> {
    let path = PathBuf::from(std::env::var_os(name).ok_or("invalidConfiguration")?);
    if !path.is_absolute() {
        return Err("invalidConfiguration");
    }
    Ok(path)
}
fn configured_frontend_port(
    value: Option<&std::ffi::OsStr>,
) -> std::result::Result<u16, &'static str> {
    let Some(value) = value else {
        return Ok(1420);
    };
    let value = value.to_str().ok_or("invalidConfiguration")?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalidConfiguration");
    }
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or("invalidConfiguration")
}
/// Starts only a loopback ephemeral listener; stdout contains one readiness record.
pub async fn run() -> std::result::Result<(), &'static str> {
    let frontend_port = configured_frontend_port(std::env::var_os("ICEON_DEV_PORT").as_deref())?;
    let token = std::env::var("ICEON_DEV_ENGINE_TOKEN").map_err(|_| "invalidConfiguration")?;
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalidConfiguration");
    }
    let directory = absolute_env("ICEON_DEV_DATA_DIR")?;
    let home = if std::env::var_os("ICEON_DEV_DISCOVERY_HOME").is_some() {
        Some(absolute_env("ICEON_DEV_DISCOVERY_HOME")?)
    } else {
        None
    };
    crate::tls::initialize();
    let (catalog, preferences) = tokio::task::spawn_blocking(move || {
        let catalog = Arc::new(CatalogEngine::new_with_home(directory.clone(), home));
        catalog.list().map_err(|_| "storeUnavailable")?;
        let preferences = PreferencesEngine::new(directory).map_err(|_| "storeUnavailable")?;
        Ok::<_, &'static str>((catalog, preferences))
    })
    .await
    .map_err(|_| "storeUnavailable")??;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "listenFailed")?;
    let port = listener.local_addr().map_err(|_| "listenFailed")?.port();
    let state = Arc::new(Engine {
        stopping: AtomicBool::new(false),
        token,
        host: format!("127.0.0.1:{port}"),
        frontend_port,
        connections: Arc::new(ConnectionEngine::new(catalog.clone())),
        catalog,
        preferences,
        owner: RwLock::new(None),
        permits: Semaphore::new(32),
    });
    let cancel = tokio_util::sync::CancellationToken::new();
    let expiry_state = state.clone();
    let expiry_cancel = cancel.clone();
    let expiry = tokio::spawn(async move {
        loop {
            tokio::select! { _ = expiry_cancel.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(1)) => expire(&expiry_state).await }
        }
    });
    let stop_state = state.clone();
    let stop_cancel = cancel.clone();
    let stop = async move {
        shutdown_signal().await;
        stop_state.stopping.store(true, Ordering::SeqCst);
        stop_cancel.cancel();
        let mut owner = stop_state.owner.write().await;
        stop_state.connections.shutdown();
        *owner = None;
    };
    println!("{}", json!({"port":port}));
    let result = axum::serve(listener, router(state.clone()))
        .with_graceful_shutdown(stop)
        .await;
    cancel.cancel();
    let _ = expiry.await;
    state.connections.shutdown();
    state.connections.drain().await;
    result.map_err(|_| "serveFailed")
}
async fn shutdown_signal() {
    use tokio::io::AsyncReadExt;
    let eof = async {
        let mut stdin = tokio::io::stdin();
        let mut buffer = [0; 256];
        loop {
            match stdin.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                _ => {}
            }
        }
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate => {}, _ = eof => {} }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    const CLIENT: &str = "a00a0000-0000-4000-8000-000000000000";
    const OTHER: &str = "b00b0000-0000-4000-8000-000000000000";
    fn fixture() -> (tempfile::TempDir, Arc<Engine>) {
        let root = tempfile::tempdir().unwrap();
        let catalog = Arc::new(CatalogEngine::new_with_home(root.path().into(), None));
        let state = Arc::new(Engine {
            stopping: AtomicBool::new(false),
            token: "a".repeat(64),
            host: "127.0.0.1:12345".into(),
            frontend_port: 1420,
            connections: Arc::new(ConnectionEngine::new(catalog.clone())),
            preferences: PreferencesEngine::new(root.path().into()).unwrap(),
            catalog,
            owner: RwLock::new(None),
            permits: Semaphore::new(32),
        });
        (root, state)
    }
    fn request(body: impl Into<Body>) -> Request {
        Request::builder()
            .method("POST")
            .uri("/invoke")
            .header("host", "127.0.0.1:12345")
            .header("authorization", format!("Bearer {}", "a".repeat(64)))
            .header("origin", "http://127.0.0.1:1420")
            .header("x-iceon-dev", "1")
            .header("x-iceon-client", CLIENT)
            .header("content-type", "application/json")
            .body(body.into())
            .unwrap()
    }
    #[test]
    fn frontend_port_is_optional_and_strictly_nonzero_u16() {
        use std::ffi::OsStr;
        assert_eq!(configured_frontend_port(None), Ok(1420));
        for (value, port) in [("1", 1), ("1430", 1430), ("65535", 65535)] {
            assert_eq!(configured_frontend_port(Some(OsStr::new(value))), Ok(port));
        }
        for value in [
            "",
            "0",
            "000",
            "65536",
            "99999999999999999999",
            "-1",
            "+1430",
            " 1430",
            "1430 ",
            "1430\n",
            "1.5",
            "0x596",
            "localhost:1430",
            "１４３０",
        ] {
            assert_eq!(
                configured_frontend_port(Some(OsStr::new(value))),
                Err("invalidConfiguration"),
                "{value:?}"
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(
                configured_frontend_port(Some(OsStr::from_bytes(b"\xff"))),
                Err("invalidConfiguration")
            );
        }
    }
    #[tokio::test]
    async fn origins_are_exact_loopback_hosts_on_the_configured_port_only() {
        let (_root, mut state) = fixture();
        Arc::get_mut(&mut state).unwrap().frontend_port = 1430;
        for (origin, expected) in [
            ("http://127.0.0.1:1430", StatusCode::OK),
            ("http://localhost:1430", StatusCode::OK),
            ("http://127.0.0.1:1420", StatusCode::FORBIDDEN),
            ("http://localhost:1420", StatusCode::FORBIDDEN),
            ("http://localhost:1431", StatusCode::FORBIDDEN),
            ("https://localhost:1430", StatusCode::FORBIDDEN),
            ("http://example.test:1430", StatusCode::FORBIDDEN),
            ("http://127.0.0.2:1430", StatusCode::FORBIDDEN),
            ("http://[::1]:1430", StatusCode::FORBIDDEN),
            ("http://localhost.evil.test:1430", StatusCode::FORBIDDEN),
            ("http://localhost:1430/", StatusCode::FORBIDDEN),
            ("http://localhost:01430", StatusCode::FORBIDDEN),
            ("null", StatusCode::FORBIDDEN),
        ] {
            let mut request = request(r#"{"command":"route_history_get","args":{}}"#);
            request
                .headers_mut()
                .insert("origin", origin.parse().unwrap());
            assert_eq!(
                router(state.clone())
                    .oneshot(request)
                    .await
                    .unwrap()
                    .status(),
                expected,
                "{origin}"
            );
        }
        let mut duplicate = request("{}");
        duplicate
            .headers_mut()
            .insert("origin", "http://localhost:1430".parse().unwrap());
        duplicate
            .headers_mut()
            .append("origin", "http://127.0.0.1:1430".parse().unwrap());
        assert_eq!(
            router(state).oneshot(duplicate).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    async fn call(state: Arc<Engine>, client: &str, command: &str, args: Value) -> Value {
        let mut request = request(json!({"command":command,"args":args}).to_string());
        request
            .headers_mut()
            .insert("x-iceon-client", client.parse().unwrap());
        let response = router(state).oneshot(request).await.unwrap();
        serde_json::from_slice(
            &to_bytes(response.into_body(), RESPONSE_LIMIT)
                .await
                .unwrap(),
        )
        .unwrap()
    }
    #[tokio::test]
    async fn auth_precedes_body_and_all_headers_are_required() {
        let (_root, state) = fixture();
        for header in [
            "host",
            "authorization",
            "origin",
            "x-iceon-dev",
            "x-iceon-client",
        ] {
            let mut request = request("not json");
            request.headers_mut().remove(header);
            let response = router(state.clone()).oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{header}");
            assert!(
                response
                    .headers()
                    .keys()
                    .all(|name| !name.as_str().starts_with("access-control-"))
            );
        }
        for (header, value) in [
            ("host", "localhost:12345"),
            ("origin", "https://localhost:1420"),
            ("origin", "http://localhost:1421"),
            ("x-iceon-dev", "true"),
            ("x-iceon-client", "tab"),
            ("authorization", "Bearer wrong"),
        ] {
            let mut request = request("not json");
            request.headers_mut().insert(
                axum::http::HeaderName::from_bytes(header.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
            assert_eq!(
                router(state.clone())
                    .oneshot(request)
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        let mut duplicate = request("{}");
        duplicate
            .headers_mut()
            .append("origin", "http://127.0.0.1:1420".parse().unwrap());
        assert_eq!(
            router(state).oneshot(duplicate).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn transport_limits_methods_and_safe_errors() {
        let (_root, state) = fixture();
        for content_type in [
            None,
            Some("text/plain"),
            Some("application/json; charset=latin1"),
        ] {
            let mut request = request("secret malformed input");
            request.headers_mut().remove("content-type");
            if let Some(value) = content_type {
                request
                    .headers_mut()
                    .insert("content-type", value.parse().unwrap());
            }
            let response = router(state.clone()).oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert_eq!(
                serde_json::from_slice::<Value>(
                    &to_bytes(response.into_body(), RESPONSE_LIMIT)
                        .await
                        .unwrap()
                )
                .unwrap(),
                json!({"error": "invalidRequest", "ok": false})
            );
        }
        let mut duplicate = request("{}");
        duplicate
            .headers_mut()
            .append("content-type", "application/json".parse().unwrap());
        assert_eq!(
            router(state.clone())
                .oneshot(duplicate)
                .await
                .unwrap()
                .status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        let mut encoded = request("{}");
        encoded
            .headers_mut()
            .insert("content-encoding", "gzip".parse().unwrap());
        assert_eq!(
            router(state.clone())
                .oneshot(encoded)
                .await
                .unwrap()
                .status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        let response = router(state.clone())
            .oneshot(request(" ".repeat(BODY_LIMIT + 1)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        for method in [Method::GET, Method::OPTIONS, Method::PUT] {
            let mut request = request("{}");
            *request.method_mut() = method;
            assert_eq!(
                router(state.clone())
                    .oneshot(request)
                    .await
                    .unwrap()
                    .status(),
                StatusCode::METHOD_NOT_ALLOWED
            );
        }
        let mut missing = request("{}");
        *missing.uri_mut() = "/other".parse().unwrap();
        assert_eq!(
            router(state.clone())
                .oneshot(missing)
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(state.clone(), CLIENT, "project_open_repository", json!({})).await,
            json!({"ok":false,"error":"unknownCommand"})
        );
        assert_eq!(
            call(
                state.clone(),
                CLIENT,
                "catalog_list",
                json!({"oops":"secret"})
            )
            .await,
            json!({"ok":false,"error":"invalidInput"})
        );
        assert_eq!(
            call(
                state.clone(),
                CLIENT,
                "catalog_import_kubeconfig",
                json!({"request":{"path":"/fixture","oops":1}})
            )
            .await["error"],
            "invalidInput"
        );
        assert_eq!(
            call(
                state.clone(),
                CLIENT,
                "catalog_import_kubeconfig",
                json!({"request":{"path":"relative"}})
            )
            .await["error"],
            "invalidInput"
        );
        let permits = state.permits.acquire_many(32).await.unwrap();
        assert_eq!(
            router(state.clone())
                .oneshot(request("{}"))
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        drop(permits);
    }
    #[tokio::test]
    async fn body_timeout_is_bounded() {
        let (_root, state) = fixture();
        tokio::time::pause();
        let body = Body::from_stream(futures::stream::pending::<
            std::result::Result<axum::body::Bytes, std::io::Error>,
        >());
        assert_eq!(
            router(state).oneshot(request(body)).await.unwrap().status(),
            StatusCode::REQUEST_TIMEOUT
        );
    }
    #[tokio::test]
    async fn stopping_rejects_work_and_response_overflow_is_safe() {
        let (_root, state) = fixture();
        state.stopping.store(true, Ordering::SeqCst);
        assert_eq!(
            call(state, CLIENT, "catalog_list", json!({})).await,
            json!({"ok":false,"error":"staleSession"})
        );
        let response = response(
            StatusCode::OK,
            Ok(Value::String("x".repeat(RESPONSE_LIMIT))),
        );
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(
            serde_json::from_slice::<Value>(
                &to_bytes(response.into_body(), RESPONSE_LIMIT)
                    .await
                    .unwrap()
            )
            .unwrap(),
            json!({"ok":false,"error":"limitExceeded"})
        );
    }
    #[tokio::test]
    async fn catalog_cluster_pins_persist_and_retired_commands_are_unavailable() {
        let (root, state) = fixture();
        let config = root.path().join("fixture.yaml");
        std::fs::write(&config, "apiVersion: v1\nkind: Config\nclusters:\n- name: fixture\ncontexts:\n- name: fixture\n  context:\n    cluster: fixture\n").unwrap();
        let created = call(
            state.clone(),
            CLIENT,
            "catalog_import_kubeconfig",
            json!({"request":{"path":config}}),
        )
        .await;
        assert_eq!(created["ok"], true);
        let id = created["value"]["clusters"][0]["id"].clone();
        assert!(created["value"].get("webLinks").is_none());
        assert_eq!(
            call(
                state.clone(),
                CLIENT,
                "catalog_set_pin",
                json!({"request":{"id":id,"pinned":true}})
            )
            .await["ok"],
            true
        );
        assert_eq!(
            call(
                state.clone(),
                CLIENT,
                "catalog_open_web_link",
                json!({"request":{"id":id}})
            )
            .await["error"],
            "unknownCommand"
        );
        assert_eq!(
            call(
                state.clone(),
                CLIENT,
                "catalog_add_web_link",
                json!({"request":{"name":"Docs","url":"https://example.test"}})
            )
            .await["error"],
            "unknownCommand"
        );
        let reopened = Arc::new(CatalogEngine::new_with_home(root.path().into(), None));
        let snapshot = reopened.list().unwrap();
        assert!(snapshot.clusters[0].pinned);
        assert_eq!(snapshot.pinned_items[0].id, id.as_str().unwrap());
        assert_eq!(
            call(state.clone(), CLIENT, "catalog_list", json!({})).await["value"]["clusters"][0]["pinned"],
            true
        );
        assert_eq!(
            call(
                state.clone(),
                CLIENT,
                "catalog_remove",
                json!({"request":{"id":id}})
            )
            .await["ok"],
            true
        );
        let snapshot = reopened.list().unwrap();
        assert!(snapshot.clusters.is_empty());
        assert!(snapshot.pinned_items.is_empty());
    }
    #[tokio::test]
    async fn ownership_heartbeat_expiry_and_stale_disconnect() {
        let (root, state) = fixture();
        let config = root.path().join("fixture.yaml");
        std::fs::write(&config, "apiVersion: v1\nkind: Config\nclusters:\n- name: fixture\n  cluster:\n    server: https://127.0.0.1:9\ncontexts:\n- name: fixture\n  context:\n    cluster: fixture\ncurrent-context: fixture\n").unwrap();
        let imported = call(
            state.clone(),
            CLIENT,
            "catalog_import_kubeconfig",
            json!({"request":{"path":config}}),
        )
        .await;
        assert_eq!(imported["ok"], true, "{imported}");
        let id = imported["value"]["clusters"][0]["id"].clone();
        let opened = call(
            state.clone(),
            CLIENT,
            "cluster_session_open",
            json!({"request":{"clusterId":id}}),
        )
        .await;
        assert_eq!(opened["ok"], true, "{opened}");
        let session = opened["value"]["sessionId"].clone();
        assert_eq!(
            call(
                state.clone(),
                OTHER,
                "cluster_session_open",
                json!({"request":{"clusterId":id}})
            )
            .await["error"],
            "busy"
        );
        assert_eq!(
            call(
                state.clone(),
                OTHER,
                "cluster_connection_status",
                json!({"request":{"clusterId":id,"sessionId":session}})
            )
            .await["error"],
            "busy"
        );
        {
            let owner = state.owner.read().await;
            *owner.as_ref().unwrap().seen.lock().unwrap() =
                Instant::now() - Duration::from_secs(31);
        }
        let mut heartbeat = request("{}");
        *heartbeat.uri_mut() = "/heartbeat".parse().unwrap();
        assert_eq!(
            router(state.clone())
                .oneshot(heartbeat)
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        expire(&state).await;
        assert!(state.owner.read().await.is_some());
        {
            let owner = state.owner.read().await;
            *owner.as_ref().unwrap().seen.lock().unwrap() =
                Instant::now() - Duration::from_secs(31);
        }
        expire(&state).await;
        assert!(state.owner.read().await.is_none());
        let replaced = call(
            state.clone(),
            OTHER,
            "cluster_session_open",
            json!({"request":{"clusterId":id}}),
        )
        .await;
        assert_eq!(replaced["ok"], true);
        assert_eq!(
            call(
                state.clone(),
                CLIENT,
                "cluster_disconnect",
                json!({"request":{"clusterId":id,"sessionId":session}})
            )
            .await["error"],
            "busy"
        );
        assert_eq!(state.owner.read().await.as_ref().unwrap().client, OTHER);
        assert_eq!(
            call(
                state.clone(),
                OTHER,
                "cluster_disconnect",
                json!({"request":{"clusterId":id,"sessionId":session}})
            )
            .await["error"],
            "staleSession"
        );
        assert_eq!(
            call(
                state.clone(),
                OTHER,
                "cluster_disconnect",
                json!({"request":{"clusterId":id,"sessionId":replaced["value"]["sessionId"]}})
            )
            .await["ok"],
            true
        );
        assert!(state.owner.read().await.is_none());
        state.connections.shutdown();
        state.connections.drain().await;
    }

    #[tokio::test]
    async fn synthetic_import_open_disconnect_never_authenticates() {
        let (root, state) = fixture();
        let config = root.path().join("synthetic.yaml");
        let marker = root.path().join("auth-must-not-run");
        let secret = "synthetic-token-must-not-reach-the-renderer";
        // An executable auth fixture would leave evidence if metadata import or open ran it.
        std::fs::write(&config, format!("apiVersion: v1\nkind: Config\nclusters:\n- name: synthetic\n  cluster:\n    server: https://127.0.0.1:9\ncontexts:\n- name: synthetic\n  context:\n    cluster: synthetic\n    user: synthetic\nusers:\n- name: synthetic\n  user:\n    token: {secret}\n    exec:\n      apiVersion: client.authentication.k8s.io/v1\n      command: /usr/bin/touch\n      args: ['{}']\n      interactiveMode: Never\ncurrent-context: synthetic\n", marker.display())).unwrap();
        let imported = call(
            state.clone(),
            CLIENT,
            "catalog_import_kubeconfig",
            json!({"request":{"path":config}}),
        )
        .await;
        assert_eq!(imported["ok"], true, "{imported}");
        let id = imported["value"]["clusters"][0]["id"].clone();
        let (first, second) = tokio::join!(
            call(
                state.clone(),
                CLIENT,
                "cluster_session_open",
                json!({"request":{"clusterId":id}})
            ),
            call(
                state.clone(),
                OTHER,
                "cluster_session_open",
                json!({"request":{"clusterId":id}})
            )
        );
        assert_ne!(
            first["ok"], second["ok"],
            "exactly one owner may claim the session"
        );
        let (opened, client, loser) = if first["ok"] == true {
            (first, CLIENT, second)
        } else {
            (second, OTHER, first)
        };
        assert_eq!(loser["error"], "busy");
        assert_eq!(opened["ok"], true, "{opened}");
        assert_eq!(opened["value"]["status"], "disconnected");
        assert!(opened["value"]["apiVersion"].is_null());
        let session = json!({"request":{"clusterId":id,"sessionId":opened["value"]["sessionId"]}});
        let status = call(
            state.clone(),
            client,
            "cluster_connection_status",
            session.clone(),
        )
        .await;
        assert_eq!(status["value"]["status"], "disconnected");
        for value in [&imported, &opened, &status] {
            assert!(!value.to_string().contains(secret));
            assert!(!value.to_string().contains("/usr/bin/touch"));
        }
        assert!(!marker.exists());
        assert!(!state.connections.has_active_runtime());
        assert_eq!(
            call(state.clone(), client, "cluster_disconnect", session.clone()).await["ok"],
            true
        );
        assert_eq!(
            call(state.clone(), client, "cluster_connection_status", session).await["error"],
            "staleSession"
        );
        assert!(state.owner.read().await.is_none());
        assert!(!marker.exists());
        state.connections.shutdown();
        state.connections.drain().await;
    }

    #[test]
    fn domain_dispatch_includes_core_commands() {
        let source = include_str!("dev_engine.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        for name in [
            "catalog_list",
            "catalog_import_kubeconfig",
            "catalog_set_pin",
            "catalog_set_section_pin",
            "catalog_reorder_pins",
            "catalog_remove",
            "preferences_get",
            "preferences_save",
            "route_history_get",
            "route_history_save",
            "route_history_clear",
            "cluster_session_open",
            "cluster_connect",
            "cluster_disconnect",
            "cluster_resources",
            "cluster_resource_detail",
            "cluster_pod_detail",
            "helm_table",
        ] {
            assert!(
                source.contains(&format!("\"{name}\"")),
                "missing bridge command: {name}"
            );
        }
    }
}
