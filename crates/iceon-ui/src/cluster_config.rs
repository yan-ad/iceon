//! Compact native Add Config dialog. The dialog entity, not the shell, owns
//! input and pending paste state. Credential material is never shown or logged.

use crate::{IceonApp, navigation::catalog_error_message};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::catalog::{CatalogError, ImportRequest, config};
use iceon_core::preferences::{PreferencesSnapshot, SavePreferencesRequest};
use std::path::PathBuf;

const PASTE_LABEL: &str = "Pasted kubeconfig — choose Save As";
const MAX_PASTE: usize = 1024 * 1024;

struct AddConfigDialog {
    owner: WeakEntity<IceonApp>,
    input: Entity<InputState>,
    yaml: Option<String>,
    busy: bool,
    closed: bool,
    error: Option<String>,
}

impl IceonApp {
    pub(crate) fn open_add_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owner = cx.entity().downgrade();
        let state = cx.new(|cx| AddConfigDialog::new(owner, window, cx));
        let weak = state.downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let weak = weak.clone();
            dialog
                .title("Add Config")
                .width(px(560.0))
                .child(state.clone())
                .on_close(move |_, _, cx| {
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |state, cx| {
                            state.closed = true;
                            state.yaml = None;
                            cx.notify();
                        });
                    });
                })
        });
    }
}

impl AddConfigDialog {
    fn new(owner: WeakEntity<IceonApp>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Kubeconfig path, folder, or paste YAML")
        });
        cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if input.read(cx).value().as_ref() != PASTE_LABEL {
                    this.yaml = None;
                }
                cx.notify();
            }
        })
        .detach();
        Self {
            owner,
            input,
            yaml: None,
            busy: false,
            closed: false,
            error: None,
        }
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Choose kubeconfig file or sync folder".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = prompt.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if this.closed {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first().and_then(|p| p.to_str()) {
                            this.yaml = None;
                            this.input
                                .update(cx, |input, cx| input.set_value(path, window, cx));
                            this.import(PathBuf::from(path), false, window, cx);
                        } else {
                            this.error = Some("The selected path cannot be imported.".into());
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => this.error = Some("The native file chooser is unavailable.".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.closed {
            return;
        }
        self.error = None;
        if let Some(yaml) = self.yaml.as_ref() {
            // Validate before even opening Save As; the worker validates again
            // immediately before writing. No credentials/auth plugins are used.
            if let Err(error) = config::validate_yaml(yaml) {
                self.error = Some(catalog_error_message(error).into());
                cx.notify();
                return;
            }
            self.busy = true;
            let home = home_directory();
            let prompt = cx.prompt_for_new_path(&home, Some("kubeconfig.yaml"));
            cx.spawn_in(window, async move |this, cx| {
                let result = prompt.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.busy = false;
                    if this.closed {
                        return;
                    }
                    match result {
                        Ok(Ok(Some(path))) => this.import(path, true, window, cx),
                        Ok(Ok(None)) => {}
                        _ => this.error = Some("The native Save As dialog is unavailable.".into()),
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            let value = self.input.read(cx).value().to_string();
            // Also recognize programmatically inserted/full-document input.
            if config::looks_like_yaml(&value) {
                if value.len() > MAX_PASTE {
                    self.error = Some("Kubeconfig exceeds the 1 MiB limit.".into());
                } else {
                    self.yaml = Some(value);
                    self.input
                        .update(cx, |input, cx| input.set_value(PASTE_LABEL, window, cx));
                }
                cx.notify();
                return;
            }
            match config::input_path(&value, Some(&home_directory())) {
                Ok(path) => self.import(path, false, window, cx),
                Err(_) => {
                    self.error = Some("Enter an absolute path or choose a file or folder.".into())
                }
            }
        }
        cx.notify();
    }

    fn import(&mut self, path: PathBuf, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let catalog = owner.read(cx).catalog.clone();
        let connection = owner.read(cx).connection.clone();
        let preferences = owner.read(cx).preferences_engine.clone();
        let yaml = if save { self.yaml.take() } else { None };
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = iceon_core::runtime::spawn_blocking(move || {
                if let Some(yaml) = yaml {
                    config::save_yaml_new(&path, &yaml)?;
                }
                let path_str = path.to_str().ok_or(CatalogError::InvalidInput)?.to_owned();
                let mut saved: Option<PreferencesSnapshot> = None;
                if path.is_dir() {
                    // Read latest persisted preferences, not a potentially stale
                    // settings-window draft. Optimistic revision protects races.
                    let mut snapshot = preferences.snapshot().map_err(|_| CatalogError::StoreUnavailable)?;
                    if !snapshot.preferences.kubernetes.kubeconfig_sync_paths.contains(&path_str) {
                        snapshot.preferences.kubernetes.kubeconfig_sync_paths.push(path_str);
                        snapshot = preferences.save(SavePreferencesRequest {
                            revision: snapshot.revision, preferences: snapshot.preferences,
                        }).map_err(|_| CatalogError::StoreUnavailable)?;
                    }
                    saved = Some(snapshot);
                } else {
                    catalog.import_kubeconfig(ImportRequest { path: path_str })?;
                }
                let prefs = preferences.snapshot().map_err(|_| CatalogError::StoreUnavailable)?;
                let mut snapshot = catalog.list_with_preferences(&prefs.preferences.kubernetes)?;
                let _ = connection.project_catalog_statuses(&mut snapshot);
                Ok::<_, CatalogError>((snapshot, saved))
            }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Ok((snapshot, saved))) => {
                        let _ = this.owner.update(cx, |app, cx| {
                            if let Some(saved) = saved {
                                app.preferences_revision = saved.revision;
                                app.preferences_draft = saved.preferences.clone();
                                if cx.has_global::<crate::preferences_view::GlobalPreferencesState>() {
                                    cx.update_global::<crate::preferences_view::GlobalPreferencesState, _>(|global, _| {
                                        global.revision = saved.revision;
                                        global.preferences = saved.preferences;
                                        global.error = None;
                                    });
                                }
                            }
                            app.catalog_snapshot = snapshot;
                            app.catalog_error = None;
                            app.ensure_cluster_route(window, cx);
                            app.refresh_catalog(cx);
                            cx.notify();
                        });
                        if !this.closed { window.close_dialog(cx); }
                        this.closed = true;
                    }
                    Ok(Err(error)) => {
                        this.error = Some(if save && error == CatalogError::InputUnavailable {
                            "Could not save. Choose a new filename in a writable folder; existing files are never replaced.".into()
                        } else { catalog_error_message(error).into() });
                    }
                    Err(_) => this.error = Some("The Add Config worker is unavailable.".into()),
                }
                // Never keep a misleading paste marker after its secret buffer
                // was moved to the worker. Re-paste to retry a failed save.
                if save && this.error.is_some() {
                    this.input.update(cx, |input, cx| input.set_value("", window, cx));
                }
                cx.notify();
            });
        }).detach();
    }
}

fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

impl Render for AddConfigDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        v_flex().gap_2()
            .child(h_flex().gap_2().items_center()
                .child(div().flex_1().min_w_0().child(Input::new(&self.input).small().disabled(self.busy)
                    .on_paste(move |item, window, cx| {
                        let Some(text) = item.text() else { return false; };
                        if !config::looks_like_yaml(&text) && text.len() <= MAX_PASTE { return false; }
                        let _ = weak.update(cx, |this, cx| {
                            if this.busy { return; }
                            if text.len() > MAX_PASTE {
                                this.error = Some("Kubeconfig exceeds the 1 MiB limit.".into());
                            } else {
                                this.yaml = Some(text);
                                this.error = None;
                                this.input.update(cx, |input, cx| input.set_value(PASTE_LABEL, window, cx));
                            }
                            cx.notify();
                        });
                        true
                    })))
                .child(Button::new("add-config-submit").small().primary()
                    .label(if self.yaml.is_some() { "Save As…" } else if self.input.read(cx).value().trim().is_empty() { "Choose…" } else { "Add" })
                    .disabled(self.busy).on_click(cx.listener(|this, _, window, cx| if this.yaml.is_none() && this.input.read(cx).value().trim().is_empty() { this.choose(window, cx); } else { this.submit(window, cx); }))))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(if self.busy { "Working…" } else { "Files are imported in place. Folders stay synced. Pasted YAML is saved only with Save As (new files only)." }))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child("Only import configurations you trust. Import reads catalog metadata; connecting may execute the sign-in helper defined by your kubeconfig."))
            .when_some(self.error.clone(), |view, error| view.child(div().text_sm().text_color(cx.theme().danger).child(error)))
    }
}
