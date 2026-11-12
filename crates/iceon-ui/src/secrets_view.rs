//! Secrets workspace view: HashiCorp Vault KV v2 profiles, OS keyring session login/resume,
//! mount discovery, directory browser, version metadata, and 30-second secret reveal.

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::ScrollableElement as _,
    switch::Switch,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::IceonApp;
use crate::empty_state::compact_empty_state;
use crate::navigation::secrets_error_message;
use iceon_core::secrets::{
    Entry, Field, LoginRequest, Metadata, Mount, Profile, ProfileInput, Provider, ReadRequest,
    Reveal, SessionRequest, SessionView,
};

const WINDOW_ID: &str = "main";
const OWNER_ID: &str = "gpui-workspace";

#[derive(Default, Clone)]
pub struct SecretsWorkspaceState {
    pub profiles: Vec<Profile>,
    profiles_loaded: bool,
    listing_loaded: bool,
    pub selected_profile_id: Option<String>,
    pub creating_profile: bool,
    pub session: Option<SessionView>,
    pub remember_credentials: bool,
    pub mounts: Vec<Mount>,
    pub active_mount: Option<String>,
    pub current_path: String,
    pub entries: Vec<Entry>,
    pub selected_secret_path: Option<String>,
    pub metadata: Option<Metadata>,
    pub reveal: Option<Reveal>,
    pub loading: bool,
    pub error: Option<String>,
    pub next_read_id: u64,
}

impl SecretsWorkspaceState {
    pub fn next_id(&mut self) -> u64 {
        self.next_read_id = self.next_read_id.saturating_add(1).max(1);
        self.next_read_id
    }
}

impl IceonApp {
    pub(crate) fn load_secrets_profiles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let engine = self.secrets.clone();
        self.secrets_state.loading = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = engine.profiles().await;
            let _ = this.update(cx, |this, cx| {
                this.secrets_state.loading = false;
                match result {
                    Ok(profiles) => {
                        this.secrets_state.profiles = profiles;
                        this.secrets_state.profiles_loaded = true;
                        this.secrets_state.error = None;
                    }
                    Err(err) => {
                        this.secrets_state.error = Some(secrets_error_message(err).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn save_secrets_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = ProfileInput {
            provider: Provider::Vault,
            name: self.secret_name_input.read(cx).value().trim().to_string(),
            address: self
                .secret_address_input
                .read(cx)
                .value()
                .trim()
                .to_string(),
            namespace: self
                .secret_namespace_input
                .read(cx)
                .value()
                .trim()
                .to_string(),
            auth_mount: self
                .secret_auth_mount_input
                .read(cx)
                .value()
                .trim()
                .to_string(),
            kv_mount: self
                .secret_kv_mount_input
                .read(cx)
                .value()
                .trim()
                .to_string(),
            base_path: self
                .secret_base_path_input
                .read(cx)
                .value()
                .trim()
                .to_string(),
        };
        let engine = self.secrets.clone();
        cx.spawn_in(window, async move |this, cx| {
            let saved = engine.save(input).await;
            let profiles = engine.profiles().await;
            let _ = this.update(cx, |this, cx| {
                match saved {
                    Ok(profile) => {
                        if let Ok(list) = profiles {
                            this.secrets_state.profiles = list;
                            this.secrets_state.profiles_loaded = true;
                        }
                        this.secrets_state.creating_profile = false;
                        this.secrets_state.selected_profile_id = Some(profile.id);
                        this.secrets_state.error = None;
                    }
                    Err(err) => {
                        this.secrets_state.error = Some(secrets_error_message(err).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn open_secrets_profile(
        &mut self,
        profile_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.secrets_state.selected_profile_id = Some(profile_id.clone());
        self.secrets_state.session = None;
        self.secrets_state.mounts.clear();
        self.secrets_state.entries.clear();
        self.secrets_state.listing_loaded = false;
        self.secrets_state.selected_secret_path = None;
        self.secrets_state.metadata = None;
        self.secrets_state.reveal = None;

        let base_path = self
            .secrets_state
            .profiles
            .iter()
            .find(|p| p.id == profile_id)
            .map(|p| p.base_path.clone())
            .unwrap_or_default();
        let kv_mount = self
            .secrets_state
            .profiles
            .iter()
            .find(|p| p.id == profile_id)
            .map(|p| p.kv_mount.clone());
        self.secrets_state.current_path = base_path;
        self.secrets_state.active_mount = kv_mount;

        let engine = self.secrets.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = engine.open(WINDOW_ID, &profile_id, OWNER_ID.into()).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(view) => {
                        this.secrets_state.session = Some(view);
                        this.secrets_state.error = None;
                    }
                    Err(err) => {
                        this.secrets_state.error = Some(secrets_error_message(err).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn login_secrets_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.secrets_state.session.clone() else {
            return;
        };
        let username = self
            .secret_username_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let password = self.secret_password_input.read(cx).value().to_string();
        let remember = self.secrets_state.remember_credentials;
        let engine = self.secrets.clone();
        self.secrets_state.loading = true;
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .login(
                    WINDOW_ID,
                    LoginRequest {
                        profile_id: session.profile_id.clone(),
                        session_id: session.session_id.clone(),
                        username,
                        password,
                        remember,
                    },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.secrets_state.loading = false;
                match res {
                    Ok(view) => {
                        this.secrets_state.session = Some(view);
                        this.secrets_state.error = None;
                    }
                    Err(err) => {
                        this.secrets_state.error = Some(secrets_error_message(err).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn resume_secrets_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.secrets_state.session.clone() else {
            return;
        };
        let engine = self.secrets.clone();
        self.secrets_state.loading = true;
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .resume(
                    WINDOW_ID,
                    SessionRequest {
                        profile_id: session.profile_id,
                        session_id: session.session_id,
                    },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.secrets_state.loading = false;
                match res {
                    Ok(view) => {
                        this.secrets_state.session = Some(view);
                        this.secrets_state.error = None;
                    }
                    Err(err) => {
                        this.secrets_state.error = Some(secrets_error_message(err).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn list_secrets_path(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.secrets_state.session.clone() else {
            return;
        };
        let read_id = self.secrets_state.next_id();
        self.secrets_state.current_path = path.clone();
        self.secrets_state.listing_loaded = false;
        self.secrets_state.loading = true;
        let mount = self.secrets_state.active_mount.clone();
        let engine = self.secrets.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = engine
                .list(
                    WINDOW_ID,
                    ReadRequest {
                        profile_id: session.profile_id,
                        session_id: session.session_id,
                        read_id,
                        path,
                        mount,
                    },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.secrets_state.loading = false;
                match res {
                    Ok(listing) => {
                        this.secrets_state.entries = listing.entries;
                        this.secrets_state.listing_loaded = true;
                        this.secrets_state.error = None;
                    }
                    Err(err) => {
                        this.secrets_state.error = Some(secrets_error_message(err).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn inspect_secret_leaf(
        &mut self,
        leaf_path: String,
        reveal_values: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.secrets_state.session.clone() else {
            return;
        };
        self.secrets_state.selected_secret_path = Some(leaf_path.clone());
        let mount = self.secrets_state.active_mount.clone();
        let meta_id = self.secrets_state.next_id();
        let reveal_id = self.secrets_state.next_id();
        let engine = self.secrets.clone();
        cx.spawn_in(window, async move |this, cx| {
            let meta = engine
                .metadata(
                    WINDOW_ID,
                    ReadRequest {
                        profile_id: session.profile_id.clone(),
                        session_id: session.session_id.clone(),
                        read_id: meta_id,
                        path: leaf_path.clone(),
                        mount: mount.clone(),
                    },
                )
                .await;
            let revealed = if reveal_values {
                Some(
                    engine
                        .reveal(
                            WINDOW_ID,
                            ReadRequest {
                                profile_id: session.profile_id,
                                session_id: session.session_id,
                                read_id: reveal_id,
                                path: leaf_path,
                                mount,
                            },
                        )
                        .await,
                )
            } else {
                None
            };
            let _ = this.update(cx, |this, cx| {
                if let Ok(m) = meta {
                    this.secrets_state.metadata = Some(m);
                }
                if let Some(Ok(rev)) = revealed {
                    this.secrets_state.reveal = Some(rev);
                } else if !reveal_values {
                    this.secrets_state.reveal = None;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn render_secrets_view(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.secrets_state.clone();

        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .w(px(280.0))
                    .h_full()
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .p_3()
                    .gap_3()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().font_semibold().child("Vault Profiles"))
                            .child(
                                Button::new("new-vault-profile")
                                    .xsmall()
                                    .primary()
                                    .icon(IconName::Plus)
                                    .label("New")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.secrets_state.creating_profile =
                                            !this.secrets_state.creating_profile;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(state.creating_profile, |el| {
                        el.child(
                            v_flex()
                                .p_2p5()
                                .gap_2()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().secondary)
                                .child(div().text_xs().font_medium().child("New Vault Profile"))
                                .child(Input::new(&self.secret_name_input).xsmall())
                                .child(Input::new(&self.secret_address_input).xsmall())
                                .child(Input::new(&self.secret_namespace_input).xsmall())
                                .child(Input::new(&self.secret_auth_mount_input).xsmall())
                                .child(Input::new(&self.secret_kv_mount_input).xsmall())
                                .child(Input::new(&self.secret_base_path_input).xsmall())
                                .child(
                                    Button::new("save-vault-profile-btn")
                                        .xsmall()
                                        .primary()
                                        .label("Save Profile")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.save_secrets_profile(window, cx);
                                        })),
                                ),
                        )
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .overflow_y_scrollbar()
                            .when(
                                state.profiles_loaded
                                    && !state.loading
                                    && state.error.is_none()
                                    && state.profiles.is_empty(),
                                |el| {
                                    el.child(compact_empty_state(
                                        IconName::Folder,
                                        "No Vault profiles",
                                        "Create a profile with New to connect to Vault.",
                                    ))
                                },
                            )
                            .children(state.profiles.iter().map(|profile| {
                                let pid = profile.id.clone();
                                let selected =
                                    state.selected_profile_id.as_deref() == Some(profile.id.as_str());
                                Button::new(SharedString::from(format!("vault-prof-{pid}")))
                                    .small()
                                    .w_full()
                                    .when(selected, |b| b.primary())
                                    .when(!selected, |b| b.ghost())
                                    .label(format!("{} ({})", profile.name, profile.kv_mount))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_secrets_profile(pid.clone(), window, cx);
                                    }))
                            })),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .p_4()
                    .gap_4()
                    .when_some(state.error.clone(), |el, msg| {
                        el.child(
                            div()
                                .p_2p5()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(cx.theme().danger)
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(msg),
                        )
                    })
                    .child(if let Some(session) = state.session.clone() {
                        v_flex()
                            .flex_1()
                            .gap_4()
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .p_3()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_semibold()
                                                    .child("Session Status:"),
                                            )
                                            .child(match session.status {
                                                "connected" => {
                                                    Tag::success().child("Connected")
                                                }
                                                "expired" => {
                                                    Tag::warning().child("Expired")
                                                }
                                                _ => Tag::secondary().child("Signed Out"),
                                            })
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(format!(
                                                        "Store: {}",
                                                        session.credential_store
                                                    )),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                Button::new("vault-resume-btn")
                                                    .xsmall()
                                                    .outline()
                                                    .label("Resume Saved Token")
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.resume_secrets_session(window, cx);
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Button::new("vault-list-root-btn")
                                                    .xsmall()
                                                    .primary()
                                                    .label("Browse Mount")
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            let path = this
                                                                .secrets_state
                                                                .current_path
                                                                .clone();
                                                            this.list_secrets_path(
                                                                path, window, cx,
                                                            );
                                                        },
                                                    )),
                                            ),
                                    ),
                            )
                            .when(session.status != "connected", |el| {
                                el.child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .p_3()
                                        .rounded(px(6.0))
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .child(
                                            div()
                                                .w(px(180.0))
                                                .child(
                                                    Input::new(&self.secret_username_input).small(),
                                                ),
                                        )
                                        .child(
                                            div().w(px(180.0)).child(
                                                Input::new(&self.secret_password_input)
                                                    .small()
                                                    .mask_toggle(),
                                            ),
                                        )
                                        .child(
                                            Switch::new("vault-remember-switch")
                                                .label("Remember in Keyring")
                                                .checked(state.remember_credentials)
                                                .on_click(cx.listener(
                                                    |this, checked: &bool, _, cx| {
                                                        this.secrets_state.remember_credentials =
                                                            *checked;
                                                        cx.notify();
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new("vault-login-btn")
                                                .small()
                                                .primary()
                                                .label("Sign In")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.login_secrets_session(window, cx);
                                                })),
                                        ),
                                )
                            })
                            .child(
                                h_flex()
                                    .flex_1()
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .w(px(280.0))
                                            .h_full()
                                            .border_1()
                                            .border_color(cx.theme().border)
                                            .rounded(px(6.0))
                                            .p_3()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_semibold()
                                                    .child(format!(
                                                        "Path: /{}",
                                                        state.current_path
                                                    )),
                                            )
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .gap_1()
                                                    .overflow_y_scrollbar()
                                                    .when(
                                                        state.listing_loaded
                                                            && !state.loading
                                                            && state.error.is_none()
                                                            && state.entries.is_empty(),
                                                        |el| {
                                                            el.child(compact_empty_state(
                                                                IconName::Folder,
                                                                "No secret entries",
                                                                "This path has no entries to browse.",
                                                            ))
                                                        },
                                                    )
                                                    .children(state.entries.iter().map(|entry| {
                                                        let full_path =
                                                            if state.current_path.is_empty() {
                                                                entry
                                                                    .name
                                                                    .trim_end_matches('/')
                                                                    .to_string()
                                                            } else {
                                                                format!(
                                                                    "{}/{}",
                                                                    state.current_path,
                                                                    entry.name.trim_end_matches('/')
                                                                )
                                                            };
                                                        let is_dir = entry.directory;
                                                        let label = entry.name.clone();
                                                        Button::new(SharedString::from(format!(
                                                            "secret-entry-{full_path}"
                                                        )))
                                                        .xsmall()
                                                        .ghost()
                                                        .icon(if is_dir {
                                                            IconName::Folder
                                                        } else {
                                                            IconName::File
                                                        })
                                                        .label(label)
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                if is_dir {
                                                                    this.list_secrets_path(
                                                                        full_path.clone(),
                                                                        window,
                                                                        cx,
                                                                    );
                                                                } else {
                                                                    this.inspect_secret_leaf(
                                                                        full_path.clone(),
                                                                        false,
                                                                        window,
                                                                        cx,
                                                                    );
                                                                }
                                                            },
                                                        ))
                                                    })),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .h_full()
                                            .border_1()
                                            .border_color(cx.theme().border)
                                            .rounded(px(6.0))
                                            .p_4()
                                            .gap_3()
                                            .overflow_y_scrollbar()
                                            .child(
                                                if let Some(secret_path) =
                                                    state.selected_secret_path.clone()
                                                {
                                                    let reveal_path = secret_path.clone();
                                                    v_flex()
                                                        .gap_3()
                                                        .child(
                                                            h_flex()
                                                                .items_center()
                                                                .justify_between()
                                                                .child(
                                                                    div()
                                                                        .text_sm()
                                                                        .font_semibold()
                                                                        .child(secret_path),
                                                                )
                                                                .child(
                                                                    Button::new(
                                                                        "reveal-secret-values-btn",
                                                                    )
                                                                    .xsmall()
                                                                    .primary()
                                                                    .icon(IconName::Eye)
                                                                    .label("Reveal (30s)")
                                                                    .on_click(cx.listener(
                                                                        move |this,
                                                                              _,
                                                                              window,
                                                                              cx| {
                                                                            this.inspect_secret_leaf(
                                                                                reveal_path.clone(),
                                                                                true,
                                                                                window,
                                                                                cx,
                                                                            );
                                                                        },
                                                                    )),
                                                                ),
                                                        )
                                                        .when_some(state.reveal, |el, rev| {
                                                            el.child(
                                                                v_flex()
                                                                    .gap_1p5()
                                                                    .children(
                                                                        rev.fields.into_iter().map(
                                                                            |f: Field| {
                                                                                h_flex()
                                                                                    .justify_between()
                                                                                    .p_2()
                                                                                    .rounded(px(4.0))
                                                                                    .bg(cx
                                                                                        .theme()
                                                                                        .secondary)
                                                                                    .text_xs()
                                                                                    .child(
                                                                                        div()
                                                                                            .font_medium()
                                                                                            .child(f.name),
                                                                                    )
                                                                                    .child(
                                                                                        div().child(
                                                                                            f.value,
                                                                                        ),
                                                                                    )
                                                                            },
                                                                        ),
                                                                    ),
                                                            )
                                                        })
                                                        .into_any_element()
                                                } else {
                                                    compact_empty_state(
                                                        IconName::File,
                                                        "No secret selected",
                                                        "Select a secret entry to view metadata or reveal values.",
                                                    )
                                                        .into_any_element()
                                                },
                                            ),
                                    ),
                            )
                            .into_any_element()
                    } else {
                        compact_empty_state(
                            IconName::Folder,
                            "No Vault profile selected",
                            "Select or create a Vault KV v2 profile on the left to start.",
                        )
                            .into_any_element()
                    }),
            )
    }
}
