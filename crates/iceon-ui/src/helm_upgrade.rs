//! Freelens-style Helm release upgrade panel: pick a chart version, edit the
//! release's user-supplied values, then dry-run or upgrade. Values are read
//! only after the explicit "Upgrade" action, never during render.
use crate::{IceonApp, inspector_view::editor_surface::EditorSurface};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::connection::helm::{
    HelmPreferences, HelmView,
    install::HelmInstallRequest,
    release::{HelmReleaseValues, HelmReleaseValuesRequest},
};

/// Upgrade panel state, bound to the session that opened it.
pub(crate) struct HelmUpgradeState {
    pub cluster_id: String,
    pub session_id: String,
    pub namespace: String,
    pub release_name: String,
    pub read_id: u64,
    pub loaded: Option<HelmReleaseValues>,
    /// Index into `loaded.versions`; `None` keeps the current chart version.
    pub target: Option<usize>,
    pub baseline: String,
    pub busy: bool,
    pub message: Option<String>,
}

impl HelmUpgradeState {
    fn same_lease(&self, app: &IceonApp) -> bool {
        app.cluster_session.as_ref().is_some_and(|session| {
            session.cluster_id == self.cluster_id && session.session_id == self.session_id
        })
    }
}

/// Chart reference and version used for the upgrade, or `None` when the
/// current chart is not resolvable in any configured repository.
fn upgrade_target(values: &HelmReleaseValues, target: Option<usize>) -> Option<(String, String)> {
    match target {
        Some(index) => values
            .versions
            .get(index)
            .map(|v| (v.chart.clone(), v.version.clone())),
        None => values
            .versions
            .iter()
            .find(|v| v.version == values.metadata.version)
            .map(|v| (v.chart.clone(), v.version.clone())),
    }
}

impl IceonApp {
    pub(crate) fn open_helm_upgrade(
        &mut self,
        release_name: String,
        namespace: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.cluster_session.clone() else {
            return;
        };
        let read_id = self.next_operation_id();
        self.helm_install_chart = None;
        self.helm_upgrade = Some(HelmUpgradeState {
            cluster_id: session.cluster_id.clone(),
            session_id: session.session_id.clone(),
            namespace: namespace.clone(),
            release_name: release_name.clone(),
            read_id,
            loaded: None,
            target: None,
            baseline: String::new(),
            busy: true,
            message: Some("Loading release values…".into()),
        });
        self.helm_values_editor
            .update(cx, |editor, cx| editor.set_value("", window, cx));
        cx.notify();
        let request = HelmReleaseValuesRequest {
            cluster_id: session.cluster_id,
            session_id: session.session_id,
            read_id,
            namespace,
            release_name,
        };
        let preferences = HelmPreferences::from(&self.preferences_draft);
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = connection
                .helm_release_values_with_preferences(request, preferences)
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let lease_ok = this
                    .helm_upgrade
                    .as_ref()
                    .is_some_and(|state| state.read_id == read_id && state.same_lease(this));
                if !lease_ok {
                    return;
                }
                let values_yaml = result.as_ref().ok().map(|v| v.values_yaml.clone());
                if let Some(yaml) = &values_yaml {
                    this.helm_values_editor
                        .update(cx, |editor, cx| editor.set_value(yaml.clone(), window, cx));
                }
                let Some(state) = this.helm_upgrade.as_mut() else {
                    return;
                };
                state.busy = false;
                match result {
                    Ok(loaded) => {
                        state.message = if loaded.preconditions.is_none() {
                            Some("Release Secret identity is unavailable; upgrade is disabled.".into())
                        } else if upgrade_target(&loaded, None).is_none() && loaded.versions.is_empty() {
                            Some(format!(
                                "Chart \"{}\" was not found in any configured Helm repository. Add or refresh its repository to upgrade.",
                                loaded.metadata.chart
                            ))
                        } else {
                            None
                        };
                        state.baseline = values_yaml.unwrap_or_default();
                        state.loaded = Some(loaded);
                    }
                    Err(err) => state.message = Some(format!("Helm values error: {err:?}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn submit_helm_upgrade(&mut self, dry_run: bool, window: &mut Window, cx: &mut Context<Self>) {
        let operation = self.next_operation_id();
        let Some(state) = self.helm_upgrade.as_ref() else {
            return;
        };
        if state.busy || !state.same_lease(self) {
            return;
        }
        let Some(loaded) = state.loaded.clone() else {
            return;
        };
        let (Some(expected), Some((chart, version))) = (
            loaded.preconditions.clone(),
            upgrade_target(&loaded, state.target),
        ) else {
            return;
        };
        let request = HelmInstallRequest {
            cluster_id: state.cluster_id.clone(),
            session_id: state.session_id.clone(),
            read_id: operation,
            namespace: state.namespace.clone(),
            release_name: state.release_name.clone(),
            chart,
            version: Some(version.clone()),
            values_yaml: self.helm_values_editor.read(cx).value().to_string(),
            dry_run,
        };
        let preferences = HelmPreferences::from(&self.preferences_draft);
        let connection = self.connection.clone();
        if let Some(state) = self.helm_upgrade.as_mut() {
            state.busy = true;
            state.read_id = operation;
            state.message = Some(if dry_run {
                "Running server dry-run…".into()
            } else {
                "Upgrading release…".into()
            });
        }
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = connection
                .helm_upgrade_with_preferences(request, expected, preferences, false)
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let current = this
                    .helm_upgrade
                    .as_ref()
                    .is_some_and(|state| state.read_id == operation && state.same_lease(this));
                if !current {
                    return;
                }
                match result {
                    Ok(_) if dry_run => {
                        if let Some(state) = this.helm_upgrade.as_mut() {
                            state.busy = false;
                            state.message =
                                Some(format!("Dry-run validated upgrade to version {version}."));
                        }
                    }
                    Ok(upgraded) => {
                        this.helm_upgrade = None;
                        this.helm_install_message = Some(format!(
                            "Upgraded release {} to version {version}.",
                            upgraded.release_name
                        ));
                        this.load_helm_section(HelmView::Releases, window, cx);
                    }
                    Err(err) => {
                        if let Some(state) = this.helm_upgrade.as_mut() {
                            state.busy = false;
                            state.message = Some(match err {
                                iceon_core::connection::ConnectionError::Conflict => {
                                    "Upgrade failed or the release changed since it was loaded. Reload and try again.".into()
                                }
                                other => format!("Helm upgrade error: {other:?}"),
                            });
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_helm_upgrade(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.helm_upgrade.as_ref() else {
            return;
        };
        let Some(loaded) = state.loaded.as_ref() else {
            return;
        };
        let Some((chart, version)) = upgrade_target(loaded, state.target) else {
            return;
        };
        let description = format!(
            "Upgrade release \"{}\" in namespace \"{}\" from {} {} to {chart} {version} with the edited values?",
            state.release_name, state.namespace, loaded.metadata.chart, loaded.metadata.version
        );
        self.confirm_destructive_action(
            "Upgrade Helm release",
            description,
            "Upgrade",
            window,
            cx,
            |this, window, cx| this.submit_helm_upgrade(false, window, cx),
        );
    }

    pub(crate) fn render_helm_upgrade_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.helm_upgrade.as_ref()?;
        let loaded = state.loaded.as_ref();
        let target = loaded.and_then(|v| upgrade_target(v, state.target));
        let dirty = self.helm_values_editor.read(cx).value().as_ref() != state.baseline;
        let can_submit =
            !state.busy && target.is_some() && loaded.is_some_and(|v| v.preconditions.is_some());
        let version_label = match (loaded, state.target) {
            (Some(v), Some(index)) => v
                .versions
                .get(index)
                .map(|t| format!("{} · {}", t.chart, t.version))
                .unwrap_or_default(),
            (Some(v), None) => format!("Current · {}", v.metadata.version),
            (None, _) => "Version".into(),
        };
        let owner = cx.entity().downgrade();
        let versions = loaded.map(|v| v.versions.clone()).unwrap_or_default();
        let current_version = loaded.map(|v| v.metadata.version.clone());
        let selected = state.target;

        Some(
            v_flex()
                .w_full()
                .p_3()
                .gap_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary)
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .truncate()
                                        .child(format!(
                                            "Upgrade Helm Release · {}",
                                            state.release_name
                                        )),
                                )
                                .child(Tag::secondary().small().child(state.namespace.clone()))
                                .when_some(loaded, |el, v| {
                                    el.child(Tag::secondary().small().child(format!(
                                        "{}-{} · rev {}",
                                        v.metadata.chart, v.metadata.version, v.metadata.revision
                                    )))
                                })
                                .when(dirty, |el| el.child(Tag::warning().small().child("Modified"))),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_shrink_0()
                                .child(
                                    Button::new("helm-upgrade-version-btn")
                                        .xsmall()
                                        .outline()
                                        .label(version_label)
                                        .disabled(state.busy || versions.is_empty())
                                        .accessibility_label("Select chart version for upgrade")
                                        .dropdown_menu(move |mut menu, _, _| {
                                            {
                                                let owner = owner.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(format!(
                                                        "Current · {}",
                                                        current_version.clone().unwrap_or_default()
                                                    ))
                                                    .checked(selected.is_none())
                                                    .on_click(move |_, _, cx| {
                                                        let _ = owner.update(cx, |app, cx| {
                                                            if let Some(s) = app.helm_upgrade.as_mut() {
                                                                s.target = None;
                                                            }
                                                            cx.notify();
                                                        });
                                                    }),
                                                );
                                            }
                                            for (index, version) in versions.iter().enumerate() {
                                                let owner = owner.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(format!(
                                                        "{} · {}{}",
                                                        version.chart,
                                                        version.version,
                                                        if version.app_version.is_empty() {
                                                            String::new()
                                                        } else {
                                                            format!(" (app {})", version.app_version)
                                                        }
                                                    ))
                                                    .checked(selected == Some(index))
                                                    .on_click(move |_, _, cx| {
                                                        let _ = owner.update(cx, |app, cx| {
                                                            if let Some(s) = app.helm_upgrade.as_mut() {
                                                                s.target = Some(index);
                                                            }
                                                            cx.notify();
                                                        });
                                                    }),
                                                );
                                            }
                                            menu
                                        }),
                                )
                                .child(
                                    Button::new("helm-upgrade-reset-btn")
                                        .xsmall()
                                        .ghost()
                                        .icon(IconName::Undo)
                                        .tooltip("Reset values")
                                        .accessibility_label("Reset values to the current release")
                                        .disabled(state.busy || !dirty)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            if let Some(baseline) =
                                                this.helm_upgrade.as_ref().map(|s| s.baseline.clone())
                                            {
                                                this.helm_values_editor.update(cx, |editor, cx| {
                                                    editor.set_value(baseline, window, cx)
                                                });
                                            }
                                        })),
                                )
                                .child(
                                    Button::new("helm-upgrade-dry-run-btn")
                                        .xsmall()
                                        .outline()
                                        .label("Dry-Run")
                                        .disabled(!can_submit)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.submit_helm_upgrade(true, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("helm-upgrade-submit-btn")
                                        .xsmall()
                                        .primary()
                                        .label("Upgrade")
                                        .disabled(!can_submit)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.confirm_helm_upgrade(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("helm-upgrade-close-btn")
                                        .xsmall()
                                        .ghost()
                                        .icon(IconName::Close)
                                        .accessibility_label("Close upgrade panel")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.helm_upgrade = None;
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
                .when_some(state.message.clone(), |el, msg| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(msg),
                    )
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("User-supplied values (helm get values). Edits replace them on upgrade."),
                )
                .child(
                    div()
                        .h(px(360.0))
                        .w_full()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(px(6.0))
                        .overflow_hidden()
                        .child(EditorSurface::new(
                            &self.helm_values_editor,
                            &self.preferences_draft.editor,
                        )),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{HelmReleaseValues, upgrade_target};
    use iceon_core::connection::helm::release::{HelmChartVersion, HelmReleaseMetadata};

    fn values(versions: &[(&str, &str)]) -> HelmReleaseValues {
        HelmReleaseValues {
            cluster_id: "c".into(),
            session_id: "s".into(),
            read_id: 1,
            namespace: "dev".into(),
            release_name: "demo".into(),
            metadata: HelmReleaseMetadata {
                chart: "nginx".into(),
                version: "1.0.0".into(),
                app_version: String::new(),
                revision: 2,
                status: "deployed".into(),
            },
            values_yaml: "{}\n".into(),
            preconditions: None,
            versions: versions
                .iter()
                .map(|(chart, version)| HelmChartVersion {
                    chart: (*chart).into(),
                    version: (*version).into(),
                    app_version: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn target_defaults_to_current_version_when_resolvable() {
        let v = values(&[("repo/nginx", "2.0.0"), ("repo/nginx", "1.0.0")]);
        assert_eq!(
            upgrade_target(&v, None),
            Some(("repo/nginx".into(), "1.0.0".into()))
        );
        assert_eq!(
            upgrade_target(&v, Some(0)),
            Some(("repo/nginx".into(), "2.0.0".into()))
        );
        assert_eq!(upgrade_target(&v, Some(9)), None);
        assert_eq!(
            upgrade_target(&values(&[("repo/nginx", "2.0.0")]), None),
            None
        );
        assert_eq!(upgrade_target(&values(&[]), None), None);
    }
}
