mod catalog;
#[cfg(all(feature = "dev-engine", not(debug_assertions)))]
compile_error!("dev-engine is restricted to debug builds");
mod catalog_picker;
mod connection;
mod context_menu;
#[cfg(all(feature = "dev-engine", debug_assertions))]
pub mod dev_engine;
#[cfg(target_os = "macos")]
mod menu;
mod native_navigation;
mod pin_menu;
mod preferences;
mod project;
mod route_history;
mod secrets;
mod tls;
mod updates;
mod window;

use catalog::{
    CatalogEngine, CatalogError, CatalogSnapshot, ImportRequest, PinRequest, RemoveRequest,
    ReorderPinsRequest, SectionPinRequest,
};
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

async fn catalog_operation(
    engine: Arc<CatalogEngine>,
    connections: Arc<connection::ConnectionEngine>,
    operation: impl FnOnce(&CatalogEngine) -> Result<CatalogSnapshot, CatalogError> + Send + 'static,
) -> Result<CatalogSnapshot, CatalogError> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut snapshot = operation(&engine)?;
        connections
            .project_catalog_statuses(&mut snapshot)
            .map_err(|_| CatalogError::StoreUnavailable)?;
        Ok(snapshot)
    })
    .await
    .map_err(|_| CatalogError::StoreUnavailable)?
}

#[tauri::command]
async fn cluster_rbac_management_snapshot(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::rbac_management::ManagementRequest,
) -> Result<connection::rbac_management::ManagementSnapshot, connection::ConnectionError> {
    engine.rbac_management_snapshot(request).await
}
#[tauri::command]
async fn cluster_rbac_user_plan(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::rbac_management::PlanRequest,
) -> Result<connection::rbac_management::UserPlan, connection::ConnectionError> {
    engine.rbac_user_plan(request).await
}
#[tauri::command]
async fn cluster_rbac_user_apply(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::rbac_management::ApplyRequest,
) -> Result<connection::rbac_management::ApplyResponse, connection::ConnectionError> {
    engine.rbac_user_apply(request).await
}
#[tauri::command]
fn cluster_rbac_management_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::rbac_management::CancelRequest,
) -> Result<(), connection::ConnectionError> {
    engine.cancel_rbac_management(request)
}
#[tauri::command]
async fn cluster_rbac_users(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::rbac::RbacUsersRequest,
) -> Result<connection::rbac::RbacUsersResponse, connection::ConnectionError> {
    engine.rbac_users(request).await
}
#[tauri::command]
fn cluster_rbac_users_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::rbac::RbacUsersRequest,
) -> Result<(), connection::ConnectionError> {
    engine.cancel_rbac_users(request)
}

#[tauri::command]
async fn route_history_get(
    engine: tauri::State<'_, Arc<preferences::PreferencesEngine>>,
) -> Result<Option<route_history::SavedRoute>, preferences::PreferencesError> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.route_history_get())
        .await
        .map_err(|_| preferences::PreferencesError::StoreUnavailable)?
}

#[tauri::command]
async fn route_history_save(
    engine: tauri::State<'_, Arc<preferences::PreferencesEngine>>,
    route: route_history::SavedRoute,
) -> Result<(), preferences::PreferencesError> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.route_history_save(route))
        .await
        .map_err(|_| preferences::PreferencesError::StoreUnavailable)?
}

#[tauri::command]
async fn route_history_clear(
    engine: tauri::State<'_, Arc<preferences::PreferencesEngine>>,
) -> Result<(), preferences::PreferencesError> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.route_history_clear())
        .await
        .map_err(|_| preferences::PreferencesError::StoreUnavailable)?
}
#[tauri::command]
async fn cluster_custom_resource_definitions(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::custom_resources::CustomDefinitionRequest,
) -> Result<connection::ResourcePage, connection::ConnectionError> {
    engine.custom_resource_definitions(request).await
}
#[tauri::command]
fn cluster_custom_resource_definitions_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::custom_resources::CustomDefinitionRequest,
) -> Result<(), connection::ConnectionError> {
    engine.cancel_custom_resource_definitions(request)
}
#[tauri::command]
async fn cluster_custom_resources(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::custom_resources::CustomResourceRequest,
) -> Result<connection::custom_resources::CustomResourcePage, connection::ConnectionError> {
    engine.custom_resources(request).await
}
#[tauri::command]
fn cluster_custom_resources_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::custom_resources::CustomResourceRequest,
) -> Result<(), connection::ConnectionError> {
    engine.cancel_custom_resources(request)
}

#[tauri::command]
async fn helm_install(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::helm::install::HelmInstallRequest,
) -> Result<connection::helm::install::HelmInstallResult, connection::ConnectionError> {
    connections.helm_install(request).await
}

#[tauri::command]
fn helm_install_cancel(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::helm::install::HelmInstallCancelRequest,
) -> Result<(), connection::ConnectionError> {
    connections.helm_install_cancel(request)
}

#[tauri::command]
async fn helm_table(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::helm::HelmRequest,
) -> Result<connection::helm::HelmPage, connection::ConnectionError> {
    connections.helm_table(request).await
}

#[tauri::command]
fn helm_cancel(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::helm::HelmCancelRequest,
) -> Result<(), connection::ConnectionError> {
    connections.helm_cancel(request)
}

#[tauri::command]
async fn helm_detail(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::helm::HelmDetailRequest,
) -> Result<connection::helm::HelmDetail, connection::ConnectionError> {
    engine.helm_detail(request).await
}

#[tauri::command]
fn helm_detail_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::helm::HelmCancelRequest,
) -> Result<(), connection::ConnectionError> {
    engine.helm_detail_cancel(request)
}

#[tauri::command]
async fn cluster_pod_detail(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pods::PodRequest,
) -> Result<connection::pods::PodDetails, connection::ConnectionError> {
    connections.pod_detail(request).await
}

#[tauri::command]
async fn cluster_pod_mutate(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pods::PodMutationRequest,
) -> Result<(), connection::ConnectionError> {
    connections.pod_mutate(request).await
}

#[tauri::command]
async fn cluster_pod_stream_open(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pod_runtime::PodStartRequest,
) -> Result<connection::pod_runtime::PodStreamSnapshot, connection::ConnectionError> {
    connections.pod_stream_open(request).await
}

#[tauri::command]
fn cluster_pod_stream_read(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pod_runtime::PodStreamRequest,
) -> Result<connection::pod_runtime::PodStreamSnapshot, connection::ConnectionError> {
    connections.pod_stream_read(request)
}

#[tauri::command]
fn cluster_pod_stream_close(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pod_runtime::PodStreamRequest,
) -> Result<(), connection::ConnectionError> {
    connections.pod_stream_close(request)
}

#[tauri::command]
fn cluster_pod_stream_write(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pod_runtime::PodInputRequest,
) -> Result<(), connection::ConnectionError> {
    connections.pod_stream_write(request)
}

#[tauri::command]
fn cluster_pod_stream_resize(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pod_runtime::PodResizeRequest,
) -> Result<(), connection::ConnectionError> {
    connections.pod_stream_resize(request)
}

#[tauri::command]
async fn cluster_pod_forward_start(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pod_runtime::PodForwardRequest,
) -> Result<connection::pod_runtime::PodForward, connection::ConnectionError> {
    connections.pod_forward_start(request).await
}

#[tauri::command]
fn cluster_pod_forward_list(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::SessionRequest,
) -> Result<Vec<connection::pod_runtime::PodForward>, connection::ConnectionError> {
    connections.pod_forward_list(request)
}

#[tauri::command]
fn cluster_pod_forward_stop(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pod_runtime::PodForwardStopRequest,
) -> Result<(), connection::ConnectionError> {
    connections.pod_forward_stop(request)
}

#[tauri::command]
async fn cluster_pod_sync_open(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::ResourceRequest,
) -> Result<connection::pod_sync::PodSyncSnapshot, connection::ConnectionError> {
    connections.pod_sync_open(request).await
}

#[tauri::command]
fn cluster_pod_sync_read(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pod_sync::PodSyncRequest,
) -> Result<connection::pod_sync::PodSyncSnapshot, connection::ConnectionError> {
    connections.pod_sync_read(request)
}

#[tauri::command]
fn cluster_pod_sync_close(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::pod_sync::PodSyncRequest,
) -> Result<(), connection::ConnectionError> {
    connections.pod_sync_close(request)
}

#[tauri::command]
async fn cluster_session_open(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::OpenRequest,
) -> Result<connection::ConnectionSnapshot, connection::ConnectionError> {
    connections.open(request).await
}

#[tauri::command]
async fn cluster_connect(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::SessionRequest,
) -> Result<connection::ConnectionSnapshot, connection::ConnectionError> {
    connections.connect(request).await
}

#[tauri::command]
async fn cluster_disconnect(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    menus: tauri::State<'_, Arc<context_menu::ContextMenus>>,
    request: connection::SessionRequest,
) -> Result<(), connection::ConnectionError> {
    let result = connections.disconnect(request);
    if result.is_ok() {
        menus.clear();
    }
    result
}

#[tauri::command]
fn cluster_connection_status(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::SessionRequest,
) -> Result<connection::ConnectionSnapshot, connection::ConnectionError> {
    connections.status(request)
}

#[tauri::command]
async fn cluster_resources(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::ResourceRequest,
) -> Result<connection::ResourcePage, connection::ConnectionError> {
    connections.resources(request).await
}

#[tauri::command]
async fn cluster_namespace_cache_get(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::SessionRequest,
) -> Result<Option<connection::namespace_cache::NamespaceSnapshot>, connection::ConnectionError> {
    connections.namespace_cache_get(request).await
}

#[tauri::command]
async fn cluster_namespace_cache_refresh(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::namespace_cache::NamespaceRefreshRequest,
) -> Result<connection::namespace_cache::NamespaceSnapshot, connection::ConnectionError> {
    connections.namespace_cache_refresh(request).await
}

#[tauri::command]
async fn cluster_permissions_review(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::permissions::ReviewRequest,
) -> Result<connection::permissions::ReviewResponse, connection::ConnectionError> {
    connections.review_permissions(request).await
}

#[tauri::command]
fn cluster_permissions_cancel(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::permissions::ReviewRequest,
) -> Result<(), connection::ConnectionError> {
    connections.cancel_permissions(request)
}

#[tauri::command]
fn cluster_permissions_refresh(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::SessionRequest,
) -> Result<(), connection::ConnectionError> {
    connections.refresh_permissions(request)
}

#[tauri::command]
fn cluster_resources_cancel(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::ResourceRequest,
) -> Result<(), connection::ConnectionError> {
    connections.cancel_resources(request)
}

#[tauri::command]
async fn cluster_resource_detail(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::details::DetailRequest,
) -> Result<connection::details::ResourceDetail, connection::ConnectionError> {
    engine.resource_detail(request).await
}
#[tauri::command]
fn cluster_resource_detail_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::details::DetailRequest,
) -> Result<(), connection::ConnectionError> {
    engine.cancel_resource_detail(request)
}

#[tauri::command]
async fn cluster_resource_collection(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::detail_collections::CollectionRequest,
) -> Result<connection::detail_collections::ResourceCollection, connection::ConnectionError> {
    engine.resource_collection(request).await
}

#[tauri::command]
fn cluster_resource_collection_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::detail_collections::CollectionRequest,
) -> Result<(), connection::ConnectionError> {
    engine.cancel_resource_collection(request)
}

#[tauri::command]
async fn cluster_workload_action(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::workload_actions::WorkflowRequest,
) -> Result<connection::workload_actions::WorkloadActionResult, connection::ConnectionError> {
    engine.resource_workload_action(request).await
}

#[tauri::command]
fn cluster_workload_action_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::workload_actions::Cancel,
) -> Result<(), connection::ConnectionError> {
    engine.cancel_workload_action(request)
}

#[tauri::command]
async fn cluster_resource_edit(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::resource_edit::EditRequest,
) -> Result<connection::resource_edit::EditResult, connection::ConnectionError> {
    engine.resource_edit(request).await
}

#[tauri::command]
fn cluster_resource_edit_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::resource_edit::EditCancelRequest,
) -> Result<(), connection::ConnectionError> {
    engine.cancel_resource_edit(request)
}

#[tauri::command]
async fn cluster_service_forward_start(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::service_forward::ServiceForwardRequest,
) -> Result<connection::service_forward::ServiceForwardResult, connection::ConnectionError> {
    engine.service_forward_start(request).await
}

#[tauri::command]
fn cluster_service_forward_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::service_forward::ServiceForwardCancel,
) -> Result<(), connection::ConnectionError> {
    engine.service_forward_cancel(request)
}

#[tauri::command]
async fn cluster_node_action(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::node_actions::NodeActionRequest,
) -> Result<connection::node_actions::NodeActionResult, connection::ConnectionError> {
    engine.resource_node_action(request).await
}

#[tauri::command]
fn cluster_node_action_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::node_actions::Cancel,
) -> Result<(), connection::ConnectionError> {
    engine.cancel_node_action(request)
}

#[tauri::command]
async fn cluster_node_drain_review(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::node_drain::DrainReviewRequest,
) -> Result<connection::node_drain::DrainReview, connection::ConnectionError> {
    engine.node_drain_review(request).await
}

#[tauri::command]
async fn cluster_node_drain_execute(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::node_drain::DrainExecuteRequest,
) -> Result<connection::node_drain::DrainResult, connection::ConnectionError> {
    engine.node_drain_execute(request).await
}

#[tauri::command]
fn cluster_node_drain_cancel(
    engine: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::node_drain::DrainCancel,
) -> Result<(), connection::ConnectionError> {
    engine.node_drain_cancel(request)
}

#[tauri::command]
async fn cluster_resource_delete(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::delete::DeleteRequest,
) -> Result<connection::delete::DeleteResult, connection::ConnectionError> {
    connections.delete(request).await
}

#[tauri::command]
fn cluster_resource_delete_cancel(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::delete::DeleteCancelRequest,
) -> Result<(), connection::ConnectionError> {
    connections.cancel_delete(request)
}

#[tauri::command]
async fn cluster_resource_create(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::create::CreateRequest,
) -> Result<connection::create::CreateResult, connection::ConnectionError> {
    connections.create(request).await
}

#[tauri::command]
fn cluster_resource_create_cancel(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: connection::create::CreateCancelRequest,
) -> Result<(), connection::ConnectionError> {
    connections.cancel_create(request)
}

#[tauri::command]
async fn project_open_repository(app: tauri::AppHandle) -> Result<(), project::ProjectError> {
    tauri::async_runtime::spawn_blocking(move || {
        project::open_repository_with(|url| app.opener().open_url(url, None::<&str>))
    })
    .await
    .map_err(|_| project::ProjectError::OpenFailed)?
}

#[tauri::command]
async fn catalog_list(
    engine: tauri::State<'_, Arc<CatalogEngine>>,
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
) -> Result<CatalogSnapshot, CatalogError> {
    catalog_operation(
        engine.inner().clone(),
        connections.inner().clone(),
        CatalogEngine::list,
    )
    .await
}

#[tauri::command]
fn catalog_connection_states(
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
) -> Result<Vec<catalog::ClusterConnectionState>, connection::ConnectionError> {
    connections.catalog_connection_states()
}

#[tauri::command]
async fn catalog_import_kubeconfig(
    engine: tauri::State<'_, Arc<CatalogEngine>>,
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: ImportRequest,
) -> Result<CatalogSnapshot, CatalogError> {
    catalog_operation(
        engine.inner().clone(),
        connections.inner().clone(),
        move |engine| engine.import_kubeconfig(request),
    )
    .await
}

#[tauri::command]
async fn catalog_set_pin(
    engine: tauri::State<'_, Arc<CatalogEngine>>,
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: PinRequest,
) -> Result<CatalogSnapshot, CatalogError> {
    catalog_operation(
        engine.inner().clone(),
        connections.inner().clone(),
        move |engine| engine.set_pin(request),
    )
    .await
}

#[tauri::command]
async fn catalog_set_section_pin(
    engine: tauri::State<'_, Arc<CatalogEngine>>,
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: SectionPinRequest,
) -> Result<CatalogSnapshot, CatalogError> {
    catalog_operation(
        engine.inner().clone(),
        connections.inner().clone(),
        move |engine| engine.set_section_pin(request),
    )
    .await
}

#[tauri::command]
async fn catalog_reorder_pins(
    engine: tauri::State<'_, Arc<CatalogEngine>>,
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: ReorderPinsRequest,
) -> Result<CatalogSnapshot, CatalogError> {
    catalog_operation(
        engine.inner().clone(),
        connections.inner().clone(),
        move |engine| engine.reorder_pins(request),
    )
    .await
}

#[tauri::command]
async fn catalog_remove(
    engine: tauri::State<'_, Arc<CatalogEngine>>,
    connections: tauri::State<'_, Arc<connection::ConnectionEngine>>,
    request: RemoveRequest,
) -> Result<CatalogSnapshot, CatalogError> {
    catalog_operation(
        engine.inner().clone(),
        connections.inner().clone(),
        move |engine| engine.remove(request),
    )
    .await
}

#[tauri::command]
async fn preferences_get(
    engine: tauri::State<'_, Arc<preferences::PreferencesEngine>>,
) -> Result<preferences::PreferencesSnapshot, preferences::PreferencesError> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.snapshot())
        .await
        .map_err(|_| preferences::PreferencesError::StoreUnavailable)?
}

#[tauri::command]
async fn preferences_save(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Arc<preferences::PreferencesEngine>>,
    request: preferences::SavePreferencesRequest,
) -> Result<preferences::PreferencesSnapshot, preferences::PreferencesError> {
    let engine = engine.inner().clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || engine.save(request))
        .await
        .map_err(|_| preferences::PreferencesError::StoreUnavailable)??;
    native_navigation::sync_tray(&app)
        .map_err(|_| preferences::PreferencesError::StoreUnavailable)?;
    Ok(snapshot)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tls::initialize();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let catalog = Arc::new(CatalogEngine::new_with_home(
                app_data.clone(),
                app.path().home_dir().ok(),
            ));
            app.manage(catalog.clone());
            app.manage(Arc::new(connection::ConnectionEngine::new(catalog)));

            let secrets = secrets::Engine::new(app_data.join("secrets"));
            secrets.start();
            app.manage(secrets);
            let prefs = preferences::PreferencesEngine::new(app_data)
                .map_err(|e| format!("preferences init failed: {e:?}"))?;
            app.manage(prefs.clone());

            let proxy = prefs.snapshot().ok().and_then(|s| {
                if s.preferences.proxy.https_proxy.is_empty() {
                    None
                } else {
                    Some(s.preferences.proxy.https_proxy)
                }
            });
            app.manage(native_navigation::Navigation::default());
            app.manage(Arc::new(updates::WindowGate::default()));
            window::create_main_window(app)?;
            let updater =
                updates::UpdateEngine::start_with_proxy(app.handle().clone(), proxy.as_deref());
            app.manage(updater);
            native_navigation::install(app)?;
            context_menu::install(app);
            pin_menu::install(app);
            #[cfg(target_os = "macos")]
            menu::install(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            secrets::secrets_profiles,
            secrets::secrets_profile_save,
            secrets::secrets_profile_remove,
            secrets::secrets_session_open,
            secrets::secrets_login,
            secrets::secrets_resume,
            secrets::secrets_disconnect,
            secrets::secrets_list,
            secrets::secrets_reveal,
            secrets::secrets_cancel,
            secrets::secrets_close,
            context_menu::row_context_menu_show,
            context_menu::row_context_menu_cancel,
            pin_menu::catalog_pin_menu_show,
            pin_menu::catalog_pin_menu_cancel,
            catalog_list,
            cluster_namespace_cache_get,
            cluster_namespace_cache_refresh,
            catalog_picker::catalog_choose_kubeconfig,
            catalog_connection_states,
            catalog_import_kubeconfig,
            catalog_set_pin,
            catalog_set_section_pin,
            catalog_reorder_pins,
            catalog_remove,
            project_open_repository,
            project::project_open_homepage,
            native_navigation::native_navigation_ready,
            cluster_session_open,
            cluster_connect,
            cluster_disconnect,
            cluster_connection_status,
            cluster_resources,
            cluster_permissions_review,
            cluster_permissions_cancel,
            cluster_permissions_refresh,
            cluster_resources_cancel,
            cluster_resource_detail,
            cluster_resource_collection,
            cluster_resource_collection_cancel,
            cluster_workload_action,
            cluster_workload_action_cancel,
            cluster_resource_edit,
            cluster_resource_edit_cancel,
            cluster_service_forward_start,
            cluster_service_forward_cancel,
            cluster_node_action,
            cluster_node_action_cancel,
            cluster_node_drain_review,
            cluster_node_drain_execute,
            cluster_node_drain_cancel,
            cluster_custom_resources,
            cluster_custom_resources_cancel,
            cluster_custom_resource_definitions,
            cluster_rbac_users,
            cluster_rbac_users_cancel,
            cluster_rbac_management_snapshot,
            cluster_rbac_user_plan,
            cluster_rbac_user_apply,
            cluster_rbac_management_cancel,
            cluster_custom_resource_definitions_cancel,
            cluster_resource_detail_cancel,
            cluster_resource_create,
            cluster_resource_create_cancel,
            cluster_resource_delete,
            cluster_resource_delete_cancel,
            cluster_pod_detail,
            cluster_pod_mutate,
            cluster_pod_stream_open,
            cluster_pod_stream_read,
            cluster_pod_stream_close,
            cluster_pod_stream_write,
            cluster_pod_stream_resize,
            cluster_pod_forward_start,
            cluster_pod_forward_list,
            cluster_pod_forward_stop,
            cluster_pod_sync_open,
            cluster_pod_sync_read,
            cluster_pod_sync_close,
            helm_install,
            helm_install_cancel,
            helm_table,
            helm_cancel,
            helm_detail,
            helm_detail_cancel,
            preferences_get,
            preferences_save,
            route_history_get,
            route_history_save,
            route_history_clear,
            updates::update_state,
            updates::update_check,
            updates::update_window_ready,
            updates::update_window_prepared,
            updates::update_restart
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } = &event
            {
                api.prevent_exit();
                return;
            }
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Reopen { .. }) {
                let _ = native_navigation::focused_or_create(app);
            }
            if let tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Destroyed,
                ..
            } = &event
            {
                native_navigation::forget_window(app, label);
                app.state::<Arc<pin_menu::PinMenus>>().clear_window(label);
                app.state::<Arc<secrets::Engine>>().close_window(label);
            }
            if matches!(
                event,
                tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
            ) {
                app.state::<Arc<context_menu::ContextMenus>>().clear();
                app.state::<Arc<pin_menu::PinMenus>>().clear();
                let secrets = app.state::<Arc<secrets::Engine>>();
                secrets.shutdown();
                tauri::async_runtime::block_on(secrets.drain());
                let engine = app.state::<Arc<connection::ConnectionEngine>>();
                engine.shutdown();
                tauri::async_runtime::block_on(engine.drain());
                let updater = app.state::<Arc<updates::UpdateEngine>>();
                updater.shutdown();
                tauri::async_runtime::block_on(updater.drain());
            }
        });
}

#[cfg(test)]
mod command_state_tests {
    #[test]
    fn catalog_commands_extract_the_registered_shared_connection_engine() {
        let source = include_str!("lib.rs");
        for command in [
            "catalog_list",
            "catalog_import_kubeconfig",
            "catalog_set_pin",
            "catalog_set_section_pin",
            "catalog_reorder_pins",
            "catalog_remove",
            "catalog_connection_states",
        ] {
            let signature = source
                .split(&format!("fn {command}("))
                .nth(1)
                .expect("catalog command exists")
                .split(") ->")
                .next()
                .unwrap();
            assert!(
                signature.contains("tauri::State<'_, Arc<connection::ConnectionEngine>>"),
                "{command} must observe the registered shared engine"
            );
        }
    }

    #[test]
    fn generic_details_extract_the_registered_shared_engine_type() {
        let source = include_str!("lib.rs");
        for command in ["cluster_resource_detail", "cluster_resource_detail_cancel"] {
            let signature = source
                .split(&format!("fn {command}("))
                .nth(1)
                .expect("detail command exists")
                .split(") ->")
                .next()
                .unwrap();
            assert!(
                signature.contains("tauri::State<'_, Arc<connection::ConnectionEngine>>"),
                "{command} must use the same Arc state registered by setup"
            );
        }
        assert!(
            source.contains("app.manage(Arc::new(connection::ConnectionEngine::new(catalog)))")
        );
    }
}
