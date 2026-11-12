//! Kubernetes settings and real, cancellable local Helm repository commands.
//! Included as a child of `preferences_view` so the shared persistence gateway
//! remains the single authority for saved preferences.
use super::{GlobalPreferencesState, current_prefs, mutate_preferences};
use gpui_kit::component::{
    Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{
        Input, InputEvent, InputGroup, InputGroupAddon, InputGroupAddonAlignment, InputGroupButton,
        InputState,
    },
    setting::{SettingField, SettingGroup, SettingItem},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::{
    connection::helm::HelmPreferences,
    preferences::{DownloadMirror, HelmRepository, KubernetesPreferences},
};

pub(super) struct KubernetesPreferencesView {
    path_inputs: std::collections::HashMap<&'static str, Entity<InputState>>,
    repository_name: Entity<InputState>,
    repository_url: Entity<InputState>,
    repository_search: Entity<InputState>,
    mirror_url: Entity<InputState>,
    repositories: Vec<HelmRepository>,
    loaded: bool,
    busy: bool,
    message: Option<String>,
    command: Option<tokio::task::AbortHandle>,
}

impl Drop for KubernetesPreferencesView {
    fn drop(&mut self) {
        // Dropping the Tokio future drops the core command's CancelOnDrop guard,
        // cancelling its supervisor/process even when the GPUI window is gone.
        if let Some(command) = self.command.take() {
            command.abort();
        }
    }
}

#[derive(Clone)]
enum RepositoryAction {
    List,
    Add(HelmRepository),
    Remove(String),
}

#[derive(Clone, Copy)]
enum PathTarget {
    Binaries,
    Kubectl,
    Helm,
    Kubeconfigs,
}

fn refresh_catalog(cx: &mut App) {
    if let Some(state) = cx.try_global::<GlobalPreferencesState>() {
        let main = state.main_app.clone();
        let _ = main.update(cx, |app, cx| app.refresh_catalog(cx));
    }
}

fn add_sync_paths(existing: &mut Vec<String>, paths: impl IntoIterator<Item = String>) {
    for path in paths {
        if !existing.contains(&path) {
            existing.push(path);
        }
    }
}

impl KubernetesPreferencesView {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prefs = current_prefs(cx);
        let repository_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter repositories"));
        cx.subscribe(
            &repository_search,
            |_, _, _: &gpui_kit::component::input::InputEvent, cx| {
                cx.refresh_windows();
            },
        )
        .detach();
        if let Some(main) = cx
            .try_global::<GlobalPreferencesState>()
            .and_then(|state| state.main_app.upgrade())
        {
            // Preparation completes on the main entity, not this settings window.
            cx.observe(&main, |_, _, cx| {
                cx.notify();
                cx.refresh_windows();
            })
            .detach();
        }
        Self {
            path_inputs: Default::default(),
            repository_name: cx
                .new(|cx| InputState::new(window, cx).placeholder("Repository name")),
            repository_url: cx
                .new(|cx| InputState::new(window, cx).placeholder("https://charts.example.com")),
            repository_search,
            mirror_url: cx.new(|cx| {
                InputState::new(window, cx).default_value(
                    if prefs.kubernetes.kubectl_mirror_url.is_empty() {
                        "https://dl.k8s.io".to_string()
                    } else {
                        prefs.kubernetes.kubectl_mirror_url
                    },
                )
            }),
            repositories: Vec::new(),
            loaded: false,
            busy: false,
            message: None,
            command: None,
        }
    }

    fn pick(&mut self, target: PathTarget, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: !matches!(target, PathTarget::Binaries),
            directories: matches!(target, PathTarget::Binaries | PathTarget::Kubeconfigs),
            multiple: matches!(target, PathTarget::Kubeconfigs),
            prompt: Some(
                match target {
                    PathTarget::Binaries => "Choose binary download directory",
                    PathTarget::Kubectl => "Choose kubectl executable",
                    PathTarget::Helm => "Choose Helm executable",
                    PathTarget::Kubeconfigs => "Choose kubeconfig files or directories",
                }
                .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        let paths: Option<Vec<String>> = paths
                            .into_iter()
                            .map(|p| p.to_str().map(str::to_owned))
                            .collect();
                        if let Some(paths) = paths {
                            mutate_preferences(cx, |prefs| match target {
                                PathTarget::Kubeconfigs => add_sync_paths(
                                    &mut prefs.kubernetes.kubeconfig_sync_paths,
                                    paths,
                                ),
                                _ => {
                                    if let Some(path) = paths.into_iter().next() {
                                        match target {
                                            PathTarget::Binaries => {
                                                prefs.kubernetes.binaries_directory = path
                                            }
                                            PathTarget::Kubectl => {
                                                prefs.kubernetes.kubectl_binary_path = path
                                            }
                                            PathTarget::Helm => {
                                                prefs.kubernetes.helm_binary_path = path
                                            }
                                            PathTarget::Kubeconfigs => unreachable!(),
                                        }
                                    }
                                }
                            });
                            if matches!(target, PathTarget::Kubeconfigs) {
                                refresh_catalog(cx);
                            }
                        } else {
                            this.message = Some(
                                "The selected path is not valid UTF-8 and cannot be saved.".into(),
                            );
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => {
                        this.message = Some(
                            "The native path picker failed. Check filesystem access and try again."
                                .into(),
                        )
                    }
                }
                cx.notify();
                cx.refresh_windows();
            });
        })
        .detach();
    }

    fn repository_command(
        &mut self,
        action: RepositoryAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let connection = cx
            .try_global::<GlobalPreferencesState>()
            .and_then(|state| state.main_app.upgrade())
            .map(|main| main.read(cx).connection.clone());
        let Some(connection) = connection else {
            self.message = Some(
                "The main workspace is unavailable. Reopen Preferences from the main window."
                    .into(),
            );
            cx.notify();
            cx.refresh_windows();
            return;
        };
        let preferences = HelmPreferences::from(&current_prefs(cx));
        self.busy = true;
        self.message = None;
        let changed = !matches!(action, RepositoryAction::List);
        let job = iceon_core::runtime::spawn(async move {
            match action {
                RepositoryAction::List => {}
                RepositoryAction::Add(repository) => {
                    connection
                        .helm_repository_add(repository, preferences.clone(), Default::default())
                        .await?
                }
                RepositoryAction::Remove(name) => {
                    connection
                        .helm_repository_remove(name, preferences.clone(), Default::default())
                        .await?
                }
            }
            connection
                .helm_repository_list(preferences, Default::default())
                .await
        });
        self.command = Some(job.abort_handle());
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.command = None;
                match result {
                    Ok(Ok(repositories)) => {
                        this.repositories = repositories;
                        this.loaded = true;
                        this.message = Some(if changed { "Helm repository configuration updated." } else { "Loaded repositories from Helm's local configuration." }.into());
                        if changed {
                            let repositories = this.repositories.clone();
                            mutate_preferences(cx, |prefs| prefs.kubernetes.helm_repositories = repositories);
                            if let Some(error) = cx
                                .try_global::<GlobalPreferencesState>()
                                .and_then(|state| state.error.clone())
                            {
                                this.message = Some(format!(
                                    "Helm configuration changed, but saving the repository snapshot failed: {error}. Reload before retrying; the Helm command has already completed."
                                ));
                            }
                        }
                    }
                    Ok(Err(error)) => {
                        this.loaded = false;
                        this.message = Some(format!("Helm repository operation failed: {} Verify the selected Helm executable, repository name, HTTPS URL, network/proxy, and local Helm configuration. No command output or credentials are displayed.", crate::navigation::connection_error_message(error)));
                    }
                    Err(_) => this.message = Some("The Helm repository worker stopped before completion. Reload to reconcile local Helm configuration.".into()),
                }
                cx.notify();
                cx.refresh_windows();
            });
        }).detach();
        cx.notify();
        cx.refresh_windows();
    }

    fn render_repositories(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let filter = self.repository_search.read(cx).value().to_lowercase();
        let rows = self
            .repositories
            .iter()
            .filter(|r| {
                r.name.to_lowercase().contains(&filter) || r.url.to_lowercase().contains(&filter)
            })
            .cloned()
            .collect::<Vec<_>>();
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&self.repository_search).small())
                    .child(
                        Button::new("helm-reload")
                            .small()
                            .icon(crate::action_icons::refresh())
                            .accessibility_label("Load or refresh Helm repositories")
                            .label(if self.busy {
                                "Working…"
                            } else {
                                "Load / Refresh"
                            })
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.repository_command(RepositoryAction::List, window, cx)
                            })),
                    ),
            )
            .child(if self.loaded {
                "Repositories read from Helm (not a sample list)."
            } else {
                "Load repositories to inspect Helm's actual local configuration."
            })
            .when(self.loaded && rows.is_empty(), |el| {
                el.child(if filter.is_empty() {
                    "No repositories configured in Helm."
                } else {
                    "No repositories match the filter."
                })
            })
            .children(rows.into_iter().enumerate().map(|(ix, repo)| {
                let name = repo.name.clone();
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(format!("{} — {}", repo.name, repo.url)),
                    )
                    .child(
                        Button::new(("helm-remove", ix))
                            .small()
                            .icon(crate::action_icons::delete())
                            .accessibility_label(format!("Remove Helm repository {}", repo.name))
                            .label("Remove")
                            .danger()
                            .disabled(self.busy || !self.loaded)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.repository_command(
                                    RepositoryAction::Remove(name.clone()),
                                    window,
                                    cx,
                                )
                            })),
                    )
            }))
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&self.repository_name).small())
                    .child(Input::new(&self.repository_url).small()),
            )
            .child(
                Button::new("helm-add")
                    .small()
                    .icon(IconName::Plus)
                    .accessibility_label("Add Helm repository")
                    .label("Add repository")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let repo = HelmRepository {
                            name: this.repository_name.read(cx).value().trim().into(),
                            url: this.repository_url.read(cx).value().trim().into(),
                        };
                        this.repository_command(RepositoryAction::Add(repo), window, cx);
                    })),
            )
            .when_some(self.message.clone(), |el, message| {
                el.child(div().text_sm().child(message))
            })
    }

    fn render_paths(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let paths = current_prefs(cx).kubernetes.kubeconfig_sync_paths;
        v_flex()
            .w_full()
            .gap_2()
            .child(
                Button::new("kubeconfig-add")
                    .small()
                    .icon(IconName::FolderOpen)
                    .accessibility_label("Add kubeconfig files or directories")
                    .label("Add files or directories…")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.pick(PathTarget::Kubeconfigs, window, cx)
                    })),
            )
            .when(paths.is_empty(), |el| {
                el.child("No additional kubeconfig sources. Default discovery is configured above.")
            })
            .children(paths.into_iter().enumerate().map(|(ix, path)| {
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(div().flex_1().child(path.clone()))
                    .child(
                        Button::new(("kubeconfig-remove", ix))
                            .small()
                            .icon(crate::action_icons::delete())
                            .accessibility_label("Remove kubeconfig source")
                            .label("Remove source")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                mutate_preferences(cx, |prefs| {
                                    prefs
                                        .kubernetes
                                        .kubeconfig_sync_paths
                                        .retain(|p| p != &path)
                                });
                                refresh_catalog(cx);
                            })),
                    )
            }))
    }
}

fn path_items(
    view: &Entity<KubernetesPreferencesView>,
    title: &'static str,
    target: PathTarget,
    get: fn(&KubernetesPreferences) -> &String,
    set: fn(&mut KubernetesPreferences, String),
) -> Vec<SettingItem> {
    let view = view.clone();
    vec![
        SettingItem::new(
            title,
            SettingField::render(move |_, window, cx| {
                let saved = get(&current_prefs(cx).kubernetes).clone();
                let input = view.update(cx, |this, cx| {
                    this.path_inputs
                        .entry(title)
                        .or_insert_with(|| {
                            let input = cx
                                .new(|cx| InputState::new(window, cx).default_value(saved.clone()));
                            cx.subscribe(&input, move |_, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    let value = input.read(cx).value().trim().to_string();
                                    mutate_preferences(cx, |preferences| {
                                        set(&mut preferences.kubernetes, value)
                                    });
                                }
                            })
                            .detach();
                            input
                        })
                        .clone()
                });
                if input.read(cx).value().as_ref() != saved {
                    input.update(cx, |input, cx| input.set_value(saved, window, cx));
                }
                let weak = view.downgrade();
                InputGroup::new(SharedString::from(format!("path-group-{title}")))
                    .small()
                    .w_full()
                    .aria_label(title)
                    .input(Input::new(&input).small().aria_label(title))
                    .addon(
                        InputGroupAddon::new("browse-addon")
                            .align(InputGroupAddonAlignment::InlineEnd)
                            .child(
                                InputGroupButton::new(SharedString::from(format!("pick-{title}")))
                                    .icon(IconName::FolderOpen)
                                    .label("Browse…")
                                    .accessibility_label(format!("Choose {title}"))
                                    .tooltip(format!("Choose {title}"))
                                    .on_click(move |_, window, cx| {
                                        let _ = weak
                                            .update(cx, |this, cx| this.pick(target, window, cx));
                                    }),
                            ),
                    )
            }),
        )
        .keywords([title]),
    ]
}

pub(super) fn groups(view: &Entity<KubernetesPreferencesView>, _cx: &App) -> Vec<SettingGroup> {
    let mut binaries = vec![
        SettingItem::new(
            "Automatically download kubectl",
            SettingField::switch(
                |cx| current_prefs(cx).kubernetes.kubectl_auto_download,
                |checked, cx| {
                    mutate_preferences(cx, |p| p.kubernetes.kubectl_auto_download = checked)
                },
            )
            .default_value(false),
        )
        .description("Allow the Prepare action to download a cluster-version-matched kubectl when no configured binary exists. The binary is checksum-verified, not executed."),
        SettingItem::new(
            "kubectl download mirror",
            SettingField::dropdown(
                vec![
                    ("default".into(), "Default (dl.k8s.io)".into()),
                    ("custom".into(), "Custom HTTPS mirror".into()),
                ],
                |cx| match current_prefs(cx).kubernetes.kubectl_download_mirror {
                    DownloadMirror::Default => "default".into(),
                    DownloadMirror::Custom => "custom".into(),
                },
                |value: SharedString, cx| {
                    mutate_preferences(cx, |p| {
                        p.kubernetes.kubectl_download_mirror = if value.as_ref() == "custom" {
                            DownloadMirror::Custom
                        } else {
                            DownloadMirror::Default
                        };
                        // Custom mode must always have a valid URL under core validation.
                        if value.as_ref() == "custom" && p.kubernetes.kubectl_mirror_url.is_empty()
                        {
                            p.kubernetes.kubectl_mirror_url = "https://dl.k8s.io".into();
                        }
                    })
                },
            )
            .default_value(SharedString::from("default")),
        ),
    ];
    let mirror_view = view.clone();
    binaries.push(
        SettingItem::render(move |_, _, cx| {
            let custom =
                current_prefs(cx).kubernetes.kubectl_download_mirror == DownloadMirror::Custom;
            let weak = mirror_view.downgrade();
            let input = mirror_view.read(cx).mirror_url.clone();
            v_flex().w_full().gap_2().when(custom, |el| {
                el.child("Custom mirror HTTPS base URL")
                    .child(Input::new(&input).small())
                    .child(
                        Button::new("mirror-apply")
                            .small()
                            .icon(IconName::Check)
                            .accessibility_label("Apply mirror URL")
                            .label("Apply mirror URL")
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    let value = this.mirror_url.read(cx).value().trim().to_string();
                                    mutate_preferences(cx, |p| {
                                        p.kubernetes.kubectl_mirror_url = value
                                    });
                                    cx.notify();
                                    cx.refresh_windows();
                                });
                            }),
                    )
            })
        })
        .keywords(["custom mirror URL"]),
    );
    binaries.extend(path_items(
        view,
        "Binaries directory",
        PathTarget::Binaries,
        |p| &p.binaries_directory,
        |p, v| p.binaries_directory = v,
    ));
    binaries.extend(path_items(
        view,
        "kubectl binary path",
        PathTarget::Kubectl,
        |p| &p.kubectl_binary_path,
        |p, v| p.kubectl_binary_path = v,
    ));
    binaries.push(
        SettingItem::render(|_, _, cx| {
            let main = cx
                .try_global::<GlobalPreferencesState>()
                .map(|state| state.main_app.clone());
            let status = main
                .as_ref()
                .and_then(|main| main.upgrade())
                .and_then(|main| main.read(cx).preferences_error.clone());
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    Button::new("prepare-kubectl")
                        .small()
                        .icon(gpui_kit_assets::IconName::Download)
                        .accessibility_label("Prepare kubectl for connected cluster")
                        .label("Prepare kubectl for connected cluster")
                        .on_click(move |_, window, cx| {
                            if let Some(main) = &main {
                                let _ = main.update(cx, |app, cx| app.prepare_kubectl(window, cx));
                            }
                        }),
                )
                .when_some(status, |el, status| el.child(div().text_sm().child(status)))
        })
        .keywords(["prepare kubectl connected cluster clipboard"]),
    );
    let mut helm = path_items(
        view,
        "Helm binary path",
        PathTarget::Helm,
        |p| &p.helm_binary_path,
        |p, v| p.helm_binary_path = v,
    );
    helm.push(SettingItem::new(
        "Helm server-side apply",
        SettingField::switch(
            |cx| current_prefs(cx).kubernetes.helm_server_side,
            |checked, cx| mutate_preferences(cx, |p| p.kubernetes.helm_server_side = checked),
        )
        .default_value(false),
    ).description("Use server-side apply for installs and upgrades (Helm 4 required). Dry runs always use server validation."));
    let repo_view = view.clone();
    let paths_view = view.clone();
    vec![SettingGroup::new().title("kubectl & binary management").items(binaries),
        SettingGroup::new().title("Helm").items(helm),
        SettingGroup::new().title("Helm repositories").description("Add and remove run the real Helm CLI and affect its local repository configuration. Operations do not require a cluster connection.").items([SettingItem::render(move |_, window, cx| repo_view.update(cx, |v,cx| v.render_repositories(window,cx))).keywords(["helm repositories add remove refresh search"])]),
        SettingGroup::new().title("Kubeconfig synchronization").description("Sources are synchronized immediately; only cluster/context metadata is imported, never credentials.").items([
            SettingItem::new("Discover default kubeconfig", SettingField::switch(|cx| current_prefs(cx).kubernetes.discover_default_kubeconfig, |checked,cx| { mutate_preferences(cx, |p| p.kubernetes.discover_default_kubeconfig = checked); refresh_catalog(cx); }).default_value(true)),
            SettingItem::render(move |_,window,cx| paths_view.update(cx, |v,cx| v.render_paths(window,cx))).keywords(["kubeconfig files directories sources remove"]),
        ])]
}

#[cfg(test)]
mod tests {
    use super::add_sync_paths;
    #[test]
    fn adding_sources_preserves_order_and_deduplicates() {
        let mut paths = vec!["/one".into()];
        add_sync_paths(&mut paths, ["/one".into(), "/two".into(), "/two".into()]);
        assert_eq!(paths, ["/one", "/two"]);
    }

    #[test]
    fn cancelled_picker_does_not_change_sources() {
        let mut paths = vec!["/one".into()];
        add_sync_paths(&mut paths, []);
        assert_eq!(paths, ["/one"]);
    }

    #[test]
    fn source_paths_remain_case_sensitive() {
        let mut paths = vec!["/Config".into()];
        add_sync_paths(&mut paths, ["/config".into()]);
        assert_eq!(paths, ["/Config", "/config"]);
    }
}
