//! Pure-Rust GPUI (`gpui-kit`) desktop UI shell for Iceon (`icekube`).

pub mod catalog_view;
pub mod cluster_view;
pub mod inspector_view;
pub mod navigation;
pub mod pod_dock_view;
pub mod preferences_view;
pub mod secrets_view;
mod shell_header;
mod shell_theme;
mod sidebar_view;
mod native_menu;

use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{EditorState, Input, InputState},
    scroll::ScrollableElement as _,
    v_flex, IconName,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use iceon_core::catalog::{CatalogEngine, CatalogSnapshot, ClusterSummary};
use iceon_core::connection::{
    ConnectionEngine, ConnectionSnapshot, ResourceKind, ResourcePage,
    custom_resources::CustomResourcePage,
    helm::HelmPage,
    pod_runtime::PodForward,
    rbac::RbacUsersResponse,
    rbac_management::{ManagementSnapshot, UserPlan},
};
use iceon_core::preferences::{Preferences, PreferencesEngine};
use iceon_core::secrets;
use cluster_view::OverviewSnapshot;
use inspector_view::InspectorState;
use navigation::{
    NavigationHistory, Route, active_group_id, destination_label,
};
use pod_dock_view::PodDockTab;
use preferences_view::PreferencesSection;
use secrets_view::SecretsWorkspaceState;

pub struct IceonApp {
    pub(crate) shell_focus: FocusHandle,
    pub(crate) catalog: Arc<CatalogEngine>,
    pub(crate) connection: Arc<ConnectionEngine>,
    pub(crate) preferences_engine: Arc<PreferencesEngine>,
    pub(crate) secrets: Arc<secrets::Engine>,

    pub(crate) catalog_snapshot: CatalogSnapshot,
    pub(crate) catalog_error: Option<String>,
    pub(crate) pending_remove_cluster: Option<ClusterSummary>,

    pub(crate) history: NavigationHistory,
    pub(crate) cluster_select: Entity<shell_header::ClusterSelectState>,
    pub(crate) cluster_select_items: Vec<shell_header::ClusterChoice>,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) cluster_sidebar_collapsed: bool,
    pub(crate) expanded_cluster_groups: Vec<String>,

    pub(crate) search_modal_open: bool,
    pub(crate) preferences_open: bool,
    pub(crate) preferences_section: PreferencesSection,
    pub(crate) preferences_revision: u64,
    pub(crate) preferences_draft: Preferences,
    pub(crate) preferences_error: Option<String>,

    pub(crate) cluster_session: Option<ConnectionSnapshot>,
    pub(crate) active_cluster_section: String,
    pub(crate) namespaces: Vec<String>,
    pub(crate) selected_namespace: Option<String>,
    pub(crate) selected_status_filter: Option<String>,
    pub(crate) cluster_loading: bool,
    pub(crate) cluster_error: Option<String>,
    pub(crate) operation_counter: u64,

    pub(crate) overview_snapshot: OverviewSnapshot,
    pub(crate) resource_page: Option<ResourcePage>,
    pub(crate) active_pod_sync_id: Option<String>,
    pub(crate) creating_resource_kind: Option<ResourceKind>,
    pub(crate) create_modal_message: Option<String>,

    pub(crate) helm_page: Option<HelmPage>,
    pub(crate) helm_install_chart: Option<String>,
    pub(crate) helm_install_version: Option<String>,
    pub(crate) helm_install_message: Option<String>,

    pub(crate) custom_definitions_page: Option<ResourcePage>,
    pub(crate) custom_instances_page: Option<CustomResourcePage>,

    pub(crate) rbac_users_response: Option<RbacUsersResponse>,
    pub(crate) rbac_management_snapshot: Option<ManagementSnapshot>,
    pub(crate) rbac_active_plan: Option<UserPlan>,

    pub(crate) inspector: InspectorState,
    pub(crate) pod_dock_open: bool,
    pub(crate) pod_dock_tabs: Vec<PodDockTab>,
    pub(crate) active_pod_tab: Option<usize>,
    pub(crate) port_forwards: Vec<PodForward>,

    pub(crate) secrets_state: SecretsWorkspaceState,

    // GPUI input & editor entities
    pub(crate) global_search_input: Entity<InputState>,
    pub(crate) catalog_search_input: Entity<InputState>,
    pub(crate) resource_filter_input: Entity<InputState>,
    pub(crate) scale_replicas_input: Entity<InputState>,
    pub(crate) service_port_input: Entity<InputState>,
    pub(crate) pod_terminal_input: Entity<InputState>,
    pub(crate) helm_release_name_input: Entity<InputState>,
    pub(crate) helm_namespace_input: Entity<InputState>,
    pub(crate) rbac_new_user_input: Entity<InputState>,

    pub(crate) create_yaml_editor: Entity<EditorState>,
    pub(crate) inspector_yaml_editor: Entity<EditorState>,
    pub(crate) helm_values_editor: Entity<EditorState>,

    pub(crate) pref_timezone_input: Entity<InputState>,
    pub(crate) pref_proxy_input: Entity<InputState>,
    pub(crate) pref_helm_path_input: Entity<InputState>,
    pub(crate) pref_editor_font_family_input: Entity<InputState>,
    pub(crate) pref_editor_font_size_input: Entity<InputState>,
    pub(crate) pref_editor_tab_size_input: Entity<InputState>,
    pub(crate) pref_terminal_font_family_input: Entity<InputState>,
    pub(crate) pref_terminal_font_size_input: Entity<InputState>,
    pub(crate) pref_terminal_shell_input: Entity<InputState>,

    pub(crate) secret_name_input: Entity<InputState>,
    pub(crate) secret_address_input: Entity<InputState>,
    pub(crate) secret_namespace_input: Entity<InputState>,
    pub(crate) secret_auth_mount_input: Entity<InputState>,
    pub(crate) secret_kv_mount_input: Entity<InputState>,
    pub(crate) secret_base_path_input: Entity<InputState>,
    pub(crate) secret_username_input: Entity<InputState>,
    pub(crate) secret_password_input: Entity<InputState>,
}

impl IceonApp {
    pub fn new(
        catalog: Arc<CatalogEngine>,
        connection: Arc<ConnectionEngine>,
        preferences_engine: Arc<PreferencesEngine>,
        secrets: Arc<secrets::Engine>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let shell_focus = cx.focus_handle();
        window.focus(&shell_focus, cx);
        let (preferences_revision, preferences_draft) = match preferences_engine.snapshot() {
            Ok(snap) => (snap.revision, snap.preferences),
            Err(_) => (1, Preferences::default()),
        };

        let catalog_snapshot = catalog.list().unwrap_or(CatalogSnapshot {
            revision: 0,
            clusters: vec![],
            pinned_items: vec![],
            discovery_error: None,
        });

        let initial_route = if preferences_draft.app.restore_last_page {
            preferences_engine
                .route_history_get()
                .ok()
                .flatten()
                .map(Route::from_saved_route)
                .unwrap_or(Route::Catalog)
        } else {
            Route::Catalog
        };
        let (cluster_select, cluster_select_items) =
            shell_header::create_cluster_select(&catalog_snapshot, &initial_route, window, cx);
        let expanded_cluster_groups = match &initial_route {
            Route::Cluster { section, .. } => vec![active_group_id(section).to_owned()],
            _ => vec!["workloads".into(), "config".into()],
        };

        let global_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Jump to cluster or section..."));
        let catalog_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter clusters..."));
        let resource_filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter resources by name..."));
        let scale_replicas_input =
            cx.new(|cx| InputState::new(window, cx).default_value("1"));
        let service_port_input =
            cx.new(|cx| InputState::new(window, cx).default_value("80"));
        let pod_terminal_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Enter shell command..."));
        let helm_release_name_input =
            cx.new(|cx| InputState::new(window, cx).default_value("my-release"));
        let helm_namespace_input =
            cx.new(|cx| InputState::new(window, cx).default_value("default"));
        let rbac_new_user_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("new-service-account"));

        let create_yaml_editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                .line_number(true)
        });
        let inspector_yaml_editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                .line_number(true)
        });
        let helm_values_editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                .line_number(true)
        });

        let pref_timezone_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(preferences_draft.app.timezone.clone())
        });
        let pref_proxy_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://proxy.example:8443")
                .default_value(preferences_draft.proxy.https_proxy.clone())
        });
        let pref_helm_path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/opt/homebrew/bin/helm")
                .default_value(preferences_draft.kubernetes.helm_binary_path.clone())
        });
        let pref_editor_font_family_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(preferences_draft.editor.font_family.clone())
        });
        let pref_editor_font_size_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(preferences_draft.editor.font_size.to_string())
        });
        let pref_editor_tab_size_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(preferences_draft.editor.tab_size.to_string())
        });
        let pref_terminal_font_family_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(preferences_draft.terminal.font_family.clone())
        });
        let pref_terminal_font_size_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(preferences_draft.terminal.font_size.to_string())
        });
        let pref_terminal_shell_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/bin/sh")
                .default_value(preferences_draft.terminal.shell_path.clone())
        });

        let secret_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Profile Name"));
        let secret_address_input =
            cx.new(|cx| InputState::new(window, cx).default_value("https://vault.example.com"));
        let secret_namespace_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Vault Namespace (optional)"));
        let secret_auth_mount_input =
            cx.new(|cx| InputState::new(window, cx).default_value("userpass"));
        let secret_kv_mount_input =
            cx.new(|cx| InputState::new(window, cx).default_value("secret"));
        let secret_base_path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Base path (optional)"));
        let secret_username_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Vault username"));
        let secret_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Vault password")
                .masked(true)
        });

        let mut app = Self {
            shell_focus,
            catalog,
            connection,
            preferences_engine,
            secrets,
            catalog_snapshot,
            catalog_error: None,
            pending_remove_cluster: None,
            history: NavigationHistory::new(initial_route.clone()),
            cluster_select,
            cluster_select_items,
            sidebar_collapsed: false,
            cluster_sidebar_collapsed: false,
            expanded_cluster_groups,
            search_modal_open: false,
            preferences_open: false,
            preferences_section: PreferencesSection::App,
            preferences_revision,
            preferences_draft: preferences_draft.clone(),
            preferences_error: None,
            cluster_session: None,
            active_cluster_section: "overview".into(),
            namespaces: vec![],
            selected_namespace: None,
            selected_status_filter: None,
            cluster_loading: false,
            cluster_error: None,
            operation_counter: 1,
            overview_snapshot: OverviewSnapshot::default(),
            resource_page: None,
            active_pod_sync_id: None,
            creating_resource_kind: None,
            create_modal_message: None,
            helm_page: None,
            helm_install_chart: None,
            helm_install_version: None,
            helm_install_message: None,
            custom_definitions_page: None,
            custom_instances_page: None,
            rbac_users_response: None,
            rbac_management_snapshot: None,
            rbac_active_plan: None,
            inspector: InspectorState::default(),
            pod_dock_open: false,
            pod_dock_tabs: vec![],
            active_pod_tab: None,
            port_forwards: vec![],
            secrets_state: SecretsWorkspaceState::default(),
            global_search_input,
            catalog_search_input,
            resource_filter_input,
            scale_replicas_input,
            service_port_input,
            pod_terminal_input,
            helm_release_name_input,
            helm_namespace_input,
            rbac_new_user_input,
            create_yaml_editor,
            inspector_yaml_editor,
            helm_values_editor,
            pref_timezone_input,
            pref_proxy_input,
            pref_helm_path_input,
            pref_editor_font_family_input,
            pref_editor_font_size_input,
            pref_editor_tab_size_input,
            pref_terminal_font_family_input,
            pref_terminal_font_size_input,
            pref_terminal_shell_input,
            secret_name_input,
            secret_address_input,
            secret_namespace_input,
            secret_auth_mount_input,
            secret_kv_mount_input,
            secret_base_path_input,
            secret_username_input,
            secret_password_input,
        };

        app.apply_theme(preferences_draft.app.theme, Some(window), cx);
        if let Route::Cluster {
            cluster_id,
            section,
        } = initial_route
        {
            app.open_cluster_workspace(cluster_id, section, window, cx);
        } else if matches!(app.history.current(), Route::Secrets) {
            app.load_secrets_profiles(window, cx);
        }
        app
    }

    pub(crate) fn next_operation_id(&mut self) -> u64 {
        self.operation_counter = self.operation_counter.saturating_add(1).max(1);
        self.operation_counter
    }

    pub(crate) fn navigate(
        &mut self,
        route: Route,
        push_history: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if push_history {
            self.history.push(route.clone());
        }
        if self.preferences_draft.app.restore_last_page {
            let _ = self
                .preferences_engine
                .route_history_save(route.to_saved_route());
        }
        match route {
            Route::Catalog => {
                self.refresh_catalog(cx);
            }
            Route::Secrets => {
                self.load_secrets_profiles(window, cx);
            }
            Route::Cluster {
                cluster_id,
                section,
            } => {
                let grp = active_group_id(&section);
                if !self.expanded_cluster_groups.iter().any(|id| id == grp) {
                    self.expanded_cluster_groups.push(grp.to_owned());
                }
                self.open_cluster_workspace(cluster_id, section, window, cx);
            }
        }
        cx.notify();
    }

    fn render_search_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self
            .global_search_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let clusters = self.catalog_snapshot.clusters.clone();
        let current_cid = match self.history.current() {
            Route::Cluster { cluster_id, .. } => Some(cluster_id.clone()),
            _ => clusters.first().map(|c| c.id.clone()),
        };

        v_flex()
            .w(px(520.0))
            .max_h(px(420.0))
            .p_3()
            .gap_2()
            .rounded(px(10.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .overflow_hidden()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.global_search_input).small().cleanable(true)),
                    )
                    .child(
                        Button::new("close-search-modal-btn")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.search_modal_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .gap_1()
                    .overflow_y_scrollbar()
                    .children(
                        clusters
                            .into_iter()
                            .filter(|c| {
                                query.is_empty() || c.name.to_lowercase().contains(&query)
                            })
                            .map(|c| {
                                let cid = c.id.clone();
                                Button::new(SharedString::from(format!("jump-cluster-{cid}")))
                                    .xsmall()
                                    .ghost()
                                    .label(format!("Cluster · {}", c.name))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.search_modal_open = false;
                                        this.navigate(
                                            Route::Cluster {
                                                cluster_id: cid.clone(),
                                                section: "overview".into(),
                                            },
                                            true,
                                            window,
                                        cx,
                                        );
                                    }))
                            }),
                    )
                    .when_some(current_cid, |el, cid| {
                        el.children(
                            iceon_core::catalog::SECTIONS
                                .iter()
                                .filter(|sec| {
                                    query.is_empty()
                                        || sec.contains(&query)
                                        || destination_label(sec).to_lowercase().contains(&query)
                                })
                                .take(25)
                                .map(|sec| {
                                    let target_cid = cid.clone();
                                    let target_sec = (*sec).to_string();
                                    Button::new(SharedString::from(format!(
                                        "jump-sec-{target_sec}"
                                    )))
                                    .xsmall()
                                    .ghost()
                                    .label(format!(
                                        "Section · {}",
                                        destination_label(&target_sec)
                                    ))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.search_modal_open = false;
                                        this.navigate(
                                            Route::Cluster {
                                                cluster_id: target_cid.clone(),
                                                section: target_sec.clone(),
                                            },
                                            true,
                                            window,
                                            cx,
                                        );
                                    }))
                                }),
                        )
                    }),
            )
    }
}

impl Render for IceonApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_menus(window, cx);
        let current_route = self.history.current().clone();
        let active_cluster_id = match &current_route {
            Route::Cluster { cluster_id, .. } => Some(cluster_id.clone()),
            _ => None,
        };

        self.menu_action_surface(cx)
            .flex()
            .flex_col()
            .size_full()
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(window, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_primary_sidebar(cx))
                    .when_some(
                        active_cluster_id.filter(|_| !self.sidebar_collapsed && !self.cluster_sidebar_collapsed),
                        |el, cid| el.child(self.render_cluster_sidebar(&cid, cx)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .bg(cx.theme().background)
                            .overflow_hidden()
                            .child(match current_route {
                                Route::Catalog => {
                                    self.render_catalog_view(window, cx).into_any_element()
                                }
                                Route::Secrets => {
                                    self.render_secrets_view(window, cx).into_any_element()
                                }
                                Route::Cluster { .. } => {
                                    self.render_cluster_workspace(window, cx).into_any_element()
                                }
                            }),
                    ),
            )
            .when(self.search_modal_open, |el| {
                el.child(
                    v_flex()
                        .occlude()
                        .absolute()
                        .top(px(44.0))
                        .left(px(180.0))
                        .child(self.render_search_modal(cx)),
                )
            })
            .when(self.preferences_open, |el| {
                el.child(
                    v_flex()
                        .occlude()
                        .absolute()
                        .top(px(56.0))
                        .left(px(140.0))
                        .child(self.render_preferences_modal(window, cx)),
                )
            })
    }
}

pub fn run() {
    iceon_core::tls::initialize();
    // GPUI polls foreground futures on this thread, outside Tokio's executor.
    // Keep its timers and I/O bound to the shared runtime for the entire app loop.
    let runtime = iceon_core::runtime::handle();
    let _runtime_guard = runtime.enter();
    let data_dir = iceon_core::runtime::app_data_dir();
    let home_dir = iceon_core::runtime::home_dir();

    let catalog = Arc::new(CatalogEngine::new_with_home(data_dir.clone(), home_dir));
    let connection = Arc::new(ConnectionEngine::new(catalog.clone()));
    let preferences_engine =
        PreferencesEngine::new(data_dir.clone()).expect("failed to initialize preferences store");
    let secrets_engine = secrets::Engine::new(data_dir);
    secrets_engine.start();

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            native_menu::init(cx);
            let shutdown_connection = connection.clone();
            let shutdown_secrets = secrets_engine.clone();
            cx.on_app_quit(move |_| {
                shutdown_connection.shutdown();
                shutdown_secrets.shutdown();
                let connection = shutdown_connection.clone();
                let secrets = shutdown_secrets.clone();
                async move {
                    let _ = tokio::time::timeout(std::time::Duration::from_millis(750), async {
                        tokio::join!(connection.drain(), secrets.drain());
                    }).await;
                }
            }).detach();
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            }).detach();
            let catalog = catalog.clone();
            let connection = connection.clone();
            let preferences_engine = preferences_engine.clone();
            let secrets_engine = secrets_engine.clone();
            let options = WindowOptions {
                window_background: shell_theme::background_appearance(),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                let app = cx.new(|cx| {
                    IceonApp::new(
                        catalog,
                        connection,
                        preferences_engine,
                        secrets_engine,
                        window,
                        cx,
                    )
                });
                native_menu::register_window(&app, window, cx);
                app
            })
            .expect("failed to open Iceon GPUI window");
            cx.activate(true);
        });
}
