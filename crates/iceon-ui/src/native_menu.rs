//! One menu model for the macOS menu bar and the component AppMenuBar on Linux.
//! Linux uses an in-window menu: no DBus/global-menu integration is assumed.

use gpui_kit::component::input::{Copy, Cut, Delete, Paste, Redo, SelectAll, Undo};
use gpui_kit::component::{GlobalState, WindowExt as _, menu::AppMenuBar, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, App, Context, Div, Entity, Global, KeyBinding, Menu, MenuItem, OsAction, OwnedMenu,
    OwnedMenuItem, Window, div,
};

use crate::{IceonApp, navigation::Route};
use iceon_core::project::REPOSITORY_URL;

#[path = "native_app.rs"]
mod native_app;

gpui_kit::actions!(
    iceon,
    [
        About,
        Preferences,
        Extensions,
        Hide,
        HideOthers,
        ShowAll,
        Quit,
        ShowMainWindow,
        AddCluster,
        CloseWindow,
        Catalog,
        ToggleSidebar,
        CommandPalette,
        Back,
        Forward,
        // Menu-only placeholders: never register listeners or key bindings.
        UnavailableBack,
        UnavailableForward,
        Reload,
        DeveloperTools,
        ActualSize,
        ZoomIn,
        ZoomOut,
        FullScreen,
        Welcome,
        Documentation,
        Licenses,
        Support,
    ]
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Capabilities {
    back: bool,
    forward: bool,
    // Undo, redo, cut, copy, paste, delete, select all, in menu order.
    edit: [bool; 7],
}

#[derive(Default)]
struct MenuState {
    capabilities: Option<Capabilities>,
    bar: Option<Entity<AppMenuBar>>,
}
impl Global for MenuState {}

/// Call after gpui_kit::init. Shutdown belongs in the application's on_quit hook.
pub(crate) fn init(cx: &mut App) {
    if let Err(error) = native_app::init() {
        eprintln!("Iceon native application integration: {error}");
    }
    cx.set_global(MenuState::default());
    cx.bind_keys(bindings(cfg!(target_os = "macos")));
    cx.on_action(|_: &Quit, cx| cx.quit());
    publish_menus(Capabilities::default(), cx);
    // GPUI's macOS backend already implements applicationDockMenu: on its
    // application delegate. Use that supported hook, never replace/swizzle it.
    #[cfg(target_os = "macos")]
    cx.set_dock_menu(quick_access_menu());
}

/// Shared, non-destructive commands for the status item and Dock icon menu.
/// The regular menu bar remains independent (including Services/Edit handling).
pub(crate) fn quick_access_menu() -> Vec<MenuItem> {
    vec![
        MenuItem::action("Show Iceon", ShowMainWindow),
        MenuItem::action("Catalog", Catalog),
        MenuItem::separator(),
        MenuItem::action("Preferences…", Preferences),
        MenuItem::separator(),
        MenuItem::action("Quit Iceon", Quit),
    ]
}

/// Available on all platforms; only render this in the Linux/Windows header.
pub(crate) fn create_menu_bar(cx: &mut App) -> Entity<AppMenuBar> {
    if let Some(bar) = cx.global::<MenuState>().bar.as_ref() {
        return bar.clone();
    }

    let bar = AppMenuBar::new(cx);
    cx.global_mut::<MenuState>().bar = Some(bar.clone());
    bar
}

fn bindings(macos: bool) -> Vec<KeyBinding> {
    let mut keys = vec![KeyBinding::new(
        if macos { "cmd-q" } else { "ctrl-q" },
        Quit,
        None,
    )];
    macro_rules! bind {
        ($suffix:literal, $action:expr) => {
            keys.push(KeyBinding::new(
                if macos {
                    concat!("cmd-", $suffix)
                } else {
                    concat!("ctrl-", $suffix)
                },
                $action,
                Some("Iceon"),
            ));
        };
    }
    bind!(",", Preferences);
    bind!("o", AddCluster);
    bind!("w", CloseWindow);
    bind!("b", ToggleSidebar);
    bind!("k", CommandPalette);
    bind!("[", Back);
    bind!("]", Forward);
    bind!("r", Reload);
    bind!("0", ActualSize);
    bind!("=", ZoomIn);
    bind!("-", ZoomOut);
    if macos {
        keys.push(KeyBinding::new("cmd-h", Hide, Some("Iceon")));
        keys.push(KeyBinding::new("cmd-alt-h", HideOthers, Some("Iceon")));
        keys.push(KeyBinding::new("ctrl-cmd-f", FullScreen, Some("Iceon")));
    } else {
        keys.push(KeyBinding::new("f11", FullScreen, Some("Iceon")));
    }
    // Input/editor initialization supplies its own editing bindings and listeners.
    keys
}

/// Register once for the shell entity and its exact window, after opening it.
/// Native menu validation sees app listeners even when the focused view is not
/// on the shell's dispatch path. Weak ownership must not keep a closed shell alive.
pub(crate) fn register_window(app: &Entity<IceonApp>, window: &Window, cx: &mut App) {
    macro_rules! register {
        ($action:ty, $handler:expr) => {
            register_window_action::<$action>(app, window, cx, $handler);
        };
    }
    register!(ShowMainWindow, |_, _: &ShowMainWindow, window, cx| {
        cx.activate(true);
        window.activate_window();
    });
    register!(About, |_, _: &About, window, cx| {
        window.open_dialog(cx, |dialog, _, _| {
            dialog.title("About Iceon").child(
                v_flex()
                    .gap_2()
                    .child(format!("Iceon {}", env!("CARGO_PKG_VERSION")))
                    .child("Pure-Rust Kubernetes IDE built on GPUI")
                    .child(format!("License: {}", env!("CARGO_PKG_LICENSE")))
                    .child(REPOSITORY_URL),
            )
        });
    });
    register!(Preferences, |this, _: &Preferences, window, cx| {
        cx.activate(true);
        window.activate_window();
        this.open_preferences(window, cx);
    });
    register!(Extensions, |_, _: &Extensions, window, cx| {
        window.open_dialog(cx, |dialog, _, _| {
            dialog.title("Extensions").child(
                v_flex()
                    .gap_2()
                    .child("Iceon runs a native 100% Rust engine without external JS/Electron extensions.")
                    .child("Built-in capabilities include Kubernetes 41-kind inventory, live Pod watch, logs/exec/attach, port-forwarding, Helm v3/v4, CRD discovery, and RBAC permission planner."),
            )
        });
    });
    register!(AddCluster, |this, _: &AddCluster, window, cx| {
        this.prompt_import_kubeconfig(window, cx);
    });
    register!(CloseWindow, |_, _: &CloseWindow, window, _| window
        .remove_window());
    register!(Catalog, |this, _: &Catalog, window, cx| {
        cx.activate(true);
        window.activate_window();
        this.navigate(Route::Catalog, true, window, cx);
    });
    register!(ToggleSidebar, |this, _: &ToggleSidebar, _, cx| {
        this.sidebar_collapsed = !this.sidebar_collapsed;
        cx.notify();
    });
    register!(Welcome, |this, _: &Welcome, window, cx| {
        this.navigate(Route::Catalog, true, window, cx);
    });
    register!(CommandPalette, |this, _: &CommandPalette, _, cx| {
        this.search_modal_open = !this.search_modal_open;
        cx.notify();
    });
    register!(Reload, |this, _: &Reload, window, cx| {
        this.navigate(this.history.current().clone(), false, window, cx);
    });
    register!(DeveloperTools, |this, _: &DeveloperTools, window, cx| {
        let cluster_count = this.catalog_snapshot.clusters.len();
        let revision = this.catalog_snapshot.revision;
        let session_info = match &this.cluster_session {
            Some(s) => format!(
                "{} · status={} · version={}",
                s.cluster_id,
                s.status,
                s.api_version.as_deref().unwrap_or("n/a")
            ),
            None => "No active cluster session".into(),
        };
        let active_section = this.active_cluster_section.clone();
        let loaded_rows = this
            .resource_page
            .as_ref()
            .map(|p| p.items.len())
            .unwrap_or(0);
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title("Runtime Diagnostics").child(
                v_flex()
                    .gap_1p5()
                    .child(format!(
                        "Catalog Revision: {revision} ({cluster_count} clusters)"
                    ))
                    .child(format!("Active Session: {session_info}"))
                    .child(format!(
                        "Active Section: {active_section} ({loaded_rows} rows loaded)"
                    )),
            )
        });
    });
    register!(ActualSize, |this, _: &ActualSize, window, cx| {
        this.adjust_font_zoom(0, window, cx);
    });
    register!(ZoomIn, |this, _: &ZoomIn, window, cx| {
        this.adjust_font_zoom(1, window, cx);
    });
    register!(ZoomOut, |this, _: &ZoomOut, window, cx| {
        this.adjust_font_zoom(-1, window, cx);
    });
    register!(FullScreen, |_, _: &FullScreen, window, _| window
        .toggle_fullscreen());
    register!(Documentation, |_, _: &Documentation, _, cx| {
        cx.open_url(REPOSITORY_URL);
    });
    register!(Licenses, |_, _: &Licenses, window, cx| {
        window.open_dialog(cx, |dialog, _, _| {
            dialog.title("Open Source Licenses").child(
                v_flex()
                    .gap_2()
                    .child(format!(
                        "Iceon {} is licensed under {}.",
                        env!("CARGO_PKG_VERSION"),
                        env!("CARGO_PKG_LICENSE")
                    ))
                    .child(REPOSITORY_URL),
            )
        });
    });
    register!(Support, |_, _: &Support, _, cx| cx.open_url(REPOSITORY_URL));
    register!(Back, |this, _: &Back, window, cx| {
        if let Some(route) = this.history.go_back() {
            this.navigate(route, false, window, cx);
        }
    });
    register!(Forward, |this, _: &Forward, window, cx| {
        if let Some(route) = this.history.go_forward() {
            this.navigate(route, false, window, cx);
        }
    });
    #[cfg(target_os = "macos")]
    {
        register!(Hide, |_, _: &Hide, _, cx| cx.hide());
        register!(HideOthers, |_, _: &HideOthers, _, cx| cx.hide_other_apps());
        register!(ShowAll, |_, _: &ShowAll, _, cx| cx.unhide_other_apps());
    }
}

fn register_window_action<A: Action>(
    app: &Entity<IceonApp>,
    window: &Window,
    cx: &mut App,
    handler: impl Fn(&mut IceonApp, &A, &mut Window, &mut Context<IceonApp>) + 'static,
) {
    let app = app.downgrade();
    let window = window.window_handle();
    cx.on_action(move |action: &A, cx| {
        let _ = window.update(cx, |_, window, cx| {
            let _ = app.update(cx, |this, cx| handler(this, action, window, cx));
        });
    });
}

impl IceonApp {
    pub(crate) fn adjust_font_zoom(
        &mut self,
        delta: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if delta == 0 {
            self.preferences_draft.editor.font_size = 12;
            self.preferences_draft.terminal.font_size = 12;
        } else {
            let next_editor =
                (self.preferences_draft.editor.font_size as i32 + delta).clamp(10, 50) as u8;
            let next_term =
                (self.preferences_draft.terminal.font_size as i32 + delta).clamp(10, 50) as u8;
            self.preferences_draft.editor.font_size = next_editor;
            self.preferences_draft.terminal.font_size = next_term;
        }
        let editor_str = self.preferences_draft.editor.font_size.to_string();
        let term_str = self.preferences_draft.terminal.font_size.to_string();
        self.pref_editor_font_size_input.update(cx, |input, cx| {
            input.set_value(editor_str, window, cx);
        });
        self.pref_terminal_font_size_input.update(cx, |input, cx| {
            input.set_value(term_str, window, cx);
        });
        cx.notify();
    }

    /// Use this Div as the ancestor of the ENTIRE shell (including inputs).
    /// Do not mount it as an empty sibling: actions bubble along the focus path.
    pub(crate) fn menu_action_surface(&self, cx: &mut Context<Self>) -> Div {
        let surface = div()
            .track_focus(&self.shell_focus)
            .key_context("Iceon")
            .on_action(cx.listener(|_, _: &About, window, cx| {
                window.open_dialog(cx, |dialog, _, _| {
                    dialog.title("About Iceon").child(
                        v_flex()
                            .gap_2()
                            .child(format!("Iceon {}", env!("CARGO_PKG_VERSION")))
                            .child("Pure-Rust Kubernetes IDE built on GPUI")
                            .child(format!("License: {}", env!("CARGO_PKG_LICENSE")))
                            .child(REPOSITORY_URL),
                    )
                });
            }))
            .on_action(cx.listener(|this, _: &Preferences, window, cx| {
                this.open_preferences(window, cx);
            }))
            .on_action(cx.listener(|_, _: &Extensions, window, cx| {
                window.open_dialog(cx, |dialog, _, _| {
                    dialog.title("Extensions").child(
                        v_flex()
                            .gap_2()
                            .child("Iceon runs a native 100% Rust engine without external JS/Electron extensions.")
                            .child("Built-in capabilities include Kubernetes 41-kind inventory, live Pod watch, logs/exec/attach, port-forwarding, Helm v3/v4, CRD discovery, and RBAC permission planner."),
                    )
                });
            }))
            .on_action(cx.listener(|this, _: &AddCluster, window, cx| {
                this.prompt_import_kubeconfig(window, cx);
            }))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|this, _: &Catalog, window, cx| {
                this.navigate(Route::Catalog, true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_collapsed = !this.sidebar_collapsed;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Welcome, window, cx| {
                this.navigate(Route::Catalog, true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CommandPalette, _, cx| {
                this.search_modal_open = !this.search_modal_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Reload, window, cx| {
                this.navigate(this.history.current().clone(), false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &DeveloperTools, window, cx| {
                let cluster_count = this.catalog_snapshot.clusters.len();
                let revision = this.catalog_snapshot.revision;
                let session_info = match &this.cluster_session {
                    Some(s) => format!(
                        "{} · status={} · version={}",
                        s.cluster_id,
                        s.status,
                        s.api_version.as_deref().unwrap_or("n/a")
                    ),
                    None => "No active cluster session".into(),
                };
                let active_section = this.active_cluster_section.clone();
                let loaded_rows = this
                    .resource_page
                    .as_ref()
                    .map(|p| p.items.len())
                    .unwrap_or(0);
                window.open_dialog(cx, move |dialog, _, _| {
                    dialog.title("Runtime Diagnostics").child(
                        v_flex()
                            .gap_1p5()
                            .child(format!(
                                "Catalog Revision: {revision} ({cluster_count} clusters)"
                            ))
                            .child(format!("Active Session: {session_info}"))
                            .child(format!(
                                "Active Section: {active_section} ({loaded_rows} rows loaded)"
                            )),
                    )
                });
            }))
            .on_action(cx.listener(|this, _: &ActualSize, window, cx| {
                this.adjust_font_zoom(0, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| {
                this.adjust_font_zoom(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| {
                this.adjust_font_zoom(-1, window, cx);
            }))
            .on_action(cx.listener(|_, _: &FullScreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|_, _: &Documentation, _, cx| cx.open_url(REPOSITORY_URL)))
            .on_action(cx.listener(|_, _: &Licenses, window, cx| {
                window.open_dialog(cx, |dialog, _, _| {
                    dialog.title("Open Source Licenses").child(
                        v_flex()
                            .gap_2()
                            .child(format!(
                                "Iceon {} is licensed under {}.",
                                env!("CARGO_PKG_VERSION"),
                                env!("CARGO_PKG_LICENSE")
                            ))
                            .child(REPOSITORY_URL),
                    )
                });
            }))
            .on_action(cx.listener(|_, _: &Support, _, cx| cx.open_url(REPOSITORY_URL)));
        // Local fallbacks stop bubbling, so the app handler never runs twice.
        let surface = if self.history.can_go_back() {
            surface.on_action(cx.listener(|this, _: &Back, window, cx| {
                if let Some(route) = this.history.go_back() {
                    this.navigate(route, false, window, cx);
                }
            }))
        } else {
            surface
        };
        let surface = if self.history.can_go_forward() {
            surface.on_action(cx.listener(|this, _: &Forward, window, cx| {
                if let Some(route) = this.history.go_forward() {
                    this.navigate(route, false, window, cx);
                }
            }))
        } else {
            surface
        };
        #[cfg(target_os = "macos")]
        let surface = surface
            .on_action(cx.listener(|_, _: &Hide, _, cx| cx.hide()))
            .on_action(cx.listener(|_, _: &HideOthers, _, cx| cx.hide_other_apps()))
            .on_action(cx.listener(|_, _: &ShowAll, _, cx| cx.unhide_other_apps()));
        surface
    }

    /// Synchronize menu capabilities only when navigation history state changes on macOS,
    /// avoiding per-frame `on_next_frame` dispatch tree walks during list scrolling.
    pub(crate) fn sync_menus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let back = self.history.can_go_back();
        let forward = self.history.can_go_forward();
        if cfg!(target_os = "macos") {
            let current = cx.global::<MenuState>().capabilities;
            if current.is_some_and(|c| c.back == back && c.forward == forward) {
                return;
            }
            publish_menus(
                Capabilities {
                    back,
                    forward,
                    edit: [true; 7],
                },
                cx,
            );
            return;
        }
        window.on_next_frame(move |window, cx| {
            let capabilities = Capabilities {
                back,
                forward,
                edit: [
                    window.is_action_available(&Undo, cx),
                    window.is_action_available(&Redo, cx),
                    window.is_action_available(&Cut, cx),
                    window.is_action_available(&Copy, cx),
                    window.is_action_available(&Paste, cx),
                    window.is_action_available(&Delete, cx),
                    window.is_action_available(&SelectAll, cx),
                ],
            };
            if cx.global::<MenuState>().capabilities != Some(capabilities) {
                publish_menus(capabilities, cx);
            }
        });
    }
}

fn publish_menus(capabilities: Capabilities, cx: &mut App) {
    let menus = menu_model(cfg!(target_os = "macos"), capabilities);
    let owned = menus.iter().map(owned_menu).collect();
    GlobalState::global_mut(cx).set_app_menus(owned);
    cx.set_menus(menus);
    let state = cx.global_mut::<MenuState>();
    state.capabilities = Some(capabilities);
    let bar = state.bar.clone();
    if let Some(bar) = bar {
        bar.update(cx, |bar, cx| bar.reload(cx));
    }
}

fn owned_menu(menu: &Menu) -> OwnedMenu {
    OwnedMenu {
        name: menu.name.clone(),
        disabled: menu.disabled,
        items: menu
            .items
            .iter()
            .map(|item| match item {
                MenuItem::Separator => OwnedMenuItem::Separator,
                MenuItem::Submenu(menu) => OwnedMenuItem::Submenu(owned_menu(menu)),
                MenuItem::SystemMenu(menu) => OwnedMenuItem::SystemMenu(gpui_kit::OwnedOsMenu {
                    name: menu.name.clone(),
                    menu_type: menu.menu_type,
                }),
                MenuItem::Action {
                    name,
                    action,
                    os_action,
                    checked,
                    disabled,
                } => OwnedMenuItem::Action {
                    name: name.to_string(),
                    action: action.boxed_clone(),
                    os_action: *os_action,
                    checked: *checked,
                    disabled: *disabled,
                },
            })
            .collect(),
    }
}

fn menu_model(macos: bool, caps: Capabilities) -> Vec<Menu> {
    let mut app = vec![
        MenuItem::action("About Iceon", About),
        MenuItem::separator(),
        MenuItem::action("Preferences…", Preferences),
        MenuItem::action("Extensions…", Extensions),
    ];
    if macos {
        app.extend([
            MenuItem::separator(),
            MenuItem::os_submenu("Services", gpui_kit::SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Iceon", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
        ]);
    }
    app.extend([MenuItem::separator(), MenuItem::action("Quit Iceon", Quit)]);
    let edit_disabled = |i: usize| !macos && !caps.edit[i];
    vec![
        Menu::new("Iceon").items(app),
        Menu::new("File").items([
            MenuItem::action("Add Cluster…", AddCluster),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo).disabled(edit_disabled(0)),
            MenuItem::os_action("Redo", Redo, OsAction::Redo).disabled(edit_disabled(1)),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut).disabled(edit_disabled(2)),
            MenuItem::os_action("Copy", Copy, OsAction::Copy).disabled(edit_disabled(3)),
            MenuItem::os_action("Paste", Paste, OsAction::Paste).disabled(edit_disabled(4)),
            MenuItem::action("Delete", Delete).disabled(edit_disabled(5)),
            MenuItem::separator(),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll)
                .disabled(edit_disabled(6)),
        ]),
        Menu::new("View").items([
            MenuItem::action("Catalog", Catalog),
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Command Palette…", CommandPalette),
            MenuItem::separator(),
            if caps.back {
                MenuItem::action("Back", Back)
            } else {
                MenuItem::action("Back", UnavailableBack).disabled(true)
            },
            if caps.forward {
                MenuItem::action("Forward", Forward)
            } else {
                MenuItem::action("Forward", UnavailableForward).disabled(true)
            },
            MenuItem::action("Reload", Reload),
            MenuItem::separator(),
            MenuItem::action("Developer Tools", DeveloperTools),
            MenuItem::separator(),
            MenuItem::action("Actual Size", ActualSize),
            MenuItem::action("Zoom In", ZoomIn),
            MenuItem::action("Zoom Out", ZoomOut),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", FullScreen),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Welcome", Welcome),
            MenuItem::action("Documentation", Documentation),
            MenuItem::action("Licenses", Licenses),
            MenuItem::separator(),
            MenuItem::action("Support", Support),
        ]),
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        Back, Capabilities, Forward, Quit, ShowMainWindow, UnavailableBack, UnavailableForward,
        bindings, menu_model, owned_menu, quick_access_menu,
    };
    use gpui_kit::{Action, KeyBinding, MenuItem};

    #[test]
    fn quick_access_menu_is_small_and_safe() {
        let items = quick_access_menu();
        assert!(matches!(&items[2], MenuItem::Separator));
        assert!(items.iter().any(|item| matches!(item,
            MenuItem::Action { name, action, .. }
            if name.as_ref() == "Show Iceon" && action.name() == ShowMainWindow.name())));
        assert!(items.iter().any(|item| matches!(item,
            MenuItem::Action { name, action, .. }
            if name.as_ref() == "Quit Iceon" && action.name() == Quit.name())));
    }

    #[test]
    fn platform_layout_and_system_services() {
        for macos in [true, false] {
            let menus = menu_model(macos, Capabilities::default());
            assert_eq!(
                menus.iter().map(|m| m.name.as_ref()).collect::<Vec<_>>(),
                ["Iceon", "File", "Edit", "View", "Help"]
            );
            assert_eq!(
                menus[0]
                    .items
                    .iter()
                    .filter(|i| matches!(i, MenuItem::SystemMenu(_)))
                    .count(),
                usize::from(macos)
            );
            for menu in &menus {
                let owned = owned_menu(menu);
                assert_eq!(owned.items.len(), menu.items.len());
                assert_eq!(owned.name, menu.name);
            }
        }
    }

    #[test]
    fn interactive_menu_commands_are_enabled_and_history_defaults_disabled() {
        for macos in [true, false] {
            let menus = menu_model(macos, Capabilities::default());
            for enabled_item in [
                "Extensions…",
                "Toggle Sidebar",
                "Developer Tools",
                "Actual Size",
                "Zoom In",
                "Zoom Out",
                "Documentation",
                "Licenses",
            ] {
                let item = menus
                    .iter()
                    .flat_map(|m| &m.items)
                    .find(|i| {
                        matches!(i, MenuItem::Action { name, .. } if name.as_ref() == enabled_item)
                    })
                    .expect("menu entry");
                assert!(!item.is_disabled(), "{enabled_item} should be enabled");
            }
            for disabled_item in ["Back", "Forward"] {
                let item = menus
                    .iter()
                    .flat_map(|m| &m.items)
                    .find(|i| {
                        matches!(i, MenuItem::Action { name, .. } if name.as_ref() == disabled_item)
                    })
                    .expect("menu entry");
                assert!(item.is_disabled(), "{disabled_item}");
            }
        }
    }

    #[test]
    fn history_and_linux_edit_capabilities_are_projected() {
        let menus = menu_model(
            false,
            Capabilities {
                back: true,
                forward: true,
                edit: [true; 7],
            },
        );
        for item in &menus[2].items {
            assert!(!item.is_disabled());
        }
        for item in &menus[3].items {
            if matches!(item, MenuItem::Action { name, .. } if name.as_ref() == "Back" || name.as_ref() == "Forward")
            {
                assert!(!item.is_disabled());
            }
        }
        let disabled = menu_model(false, Capabilities::default());
        assert!(
            disabled[2]
                .items
                .iter()
                .filter(|i| matches!(i, MenuItem::Action { .. }))
                .all(MenuItem::is_disabled)
        );
    }

    #[test]
    fn unavailable_history_uses_unhandled_actions_not_global_commands() {
        for macos in [true, false] {
            for back in [true, false] {
                for forward in [true, false] {
                    let menus = menu_model(
                        macos,
                        Capabilities {
                            back,
                            forward,
                            ..Capabilities::default()
                        },
                    );
                    for (label, available, enabled_name, disabled_name) in [
                        ("Back", back, Back.name(), UnavailableBack.name()),
                        (
                            "Forward",
                            forward,
                            Forward.name(),
                            UnavailableForward.name(),
                        ),
                    ] {
                        let item = menus[3].items.iter().find(|item| {
                            matches!(item, MenuItem::Action { name, .. } if name.as_ref() == label)
                        }).unwrap();
                        let MenuItem::Action { action, .. } = item else {
                            unreachable!()
                        };
                        assert_eq!(
                            action.name(),
                            if available {
                                enabled_name
                            } else {
                                disabled_name
                            }
                        );
                        assert_eq!(item.is_disabled(), !available);
                        assert!(
                            bindings(macos)
                                .iter()
                                .all(|binding| binding.action().name() != disabled_name)
                        );
                        let owned = owned_menu(&menus[3]);
                        assert!(owned.items.iter().any(|item| matches!(item,
                            gpui_kit::OwnedMenuItem::Action { name, action, disabled, .. }
                            if name == label && action.name() == if available { enabled_name } else { disabled_name }
                                && *disabled == !available
                        )));
                    }
                }
            }
        }
    }

    #[test]
    fn platform_binding_definitions_are_valid() {
        for (macos, shortcut) in [(true, "cmd-q"), (false, "ctrl-q")] {
            let expected = KeyBinding::new(shortcut, Quit, None);
            let keys = bindings(macos);
            assert!(keys.iter().any(|binding| {
                binding.action().name() == expected.action().name()
                    && binding.keystrokes() == expected.keystrokes()
            }));
        }
    }
}
