//! Data-only Zed theme installation and cached native settings controls.
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::zed_themes::{self, InstalledExtension, ZedTheme};

#[derive(Default)]
pub(crate) struct ThemeCatalog {
    pub(crate) extensions: Vec<InstalledExtension>,
    pub(crate) error: Option<String>,
    pub(crate) loading: bool,
}
impl Global for ThemeCatalog {}

/// Read the local store once at startup. Never called from render or theme lookup.
pub(crate) fn init(cx: &mut App) {
    if cx.try_global::<ThemeCatalog>().is_some() {
        return;
    }
    let mut catalog = ThemeCatalog::default();
    match zed_themes::installed() {
        Ok(extensions) => catalog.extensions = extensions,
        Err(error) => catalog.error = Some(error),
    }
    cx.set_global(catalog);
}

pub(crate) fn find(id: &str, cx: &App) -> Option<ZedTheme> {
    cx.try_global::<ThemeCatalog>()?
        .extensions
        .iter()
        .flat_map(|extension| &extension.themes)
        .find(|theme| theme.id == id)
        .cloned()
}

/// Accept an exact marketplace URL or a bare extension id, never an arbitrary URL.
fn extension_id(input: &str) -> Result<String, String> {
    let input = input.trim();
    let id = input
        .strip_prefix("https://zed.dev/extensions/")
        .map(|id| id.strip_suffix('/').unwrap_or(id))
        .unwrap_or(input);
    if id.is_empty()
        || id.len() > 128
        || !id.as_bytes()[0].is_ascii_alphanumeric()
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err("Enter a Zed extension ID or https://zed.dev/extensions/{id}.".into());
    }
    Ok(id.to_owned())
}

pub(crate) struct ThemesSettings {
    extension_input: Entity<InputState>,
}

impl ThemesSettings {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        init(cx);
        Self {
            extension_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Extension ID or https://zed.dev/extensions/{id}")
            }),
        }
    }

    fn change(&mut self, input: &str, remove: bool, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<ThemeCatalog>().loading {
            return;
        }
        let id = match extension_id(input) {
            Ok(id) => id,
            Err(error) => {
                cx.global_mut::<ThemeCatalog>().error = Some(error);
                cx.notify();
                cx.refresh_windows();
                return;
            }
        };
        let catalog = cx.global_mut::<ThemeCatalog>();
        catalog.loading = true;
        catalog.error = None;
        let changed_id = id.clone();
        // Reqwest runs on Tokio, not the GPUI foreground executor. Store reads and
        // removals also stay off the UI thread. Detached completion reconciles the
        // global cache even if this settings entity has already been dismissed.
        let handle = iceon_core::runtime::spawn(async move {
            let operation = if remove {
                iceon_core::runtime::spawn_blocking(move || zed_themes::uninstall(&id))
                    .await
                    .map_err(|_| "Theme removal worker stopped".to_owned())
                    .and_then(|result| result)
            } else {
                zed_themes::install(&id).await.map(|_| ())
            };
            let inventory = iceon_core::runtime::spawn_blocking(zed_themes::installed)
                .await
                .map_err(|_| "Theme catalog worker stopped".to_owned())
                .and_then(|result| result);
            (operation, inventory)
        });
        let app = cx.to_async();
        cx.spawn_in(window, async move |_, _| {
            let result = handle.await;
            app.update(|cx| {
                let catalog = cx.global_mut::<ThemeCatalog>();
                catalog.loading = false;
                let removed = match result {
                    Ok((operation, inventory)) => {
                        let removed = remove && operation.is_ok();
                        catalog.error = operation.err();
                        match inventory {
                            Ok(extensions) => catalog.extensions = extensions,
                            Err(error) => {
                                // Never continue advertising stale variants after a failed scan.
                                catalog.extensions.clear();
                                catalog.error.get_or_insert(error);
                            }
                        }
                        removed
                    }
                    Err(_) => {
                        catalog.error = Some(
                            "Theme worker stopped. Reopen the app to reload local themes.".into(),
                        );
                        false
                    }
                };
                if removed
                    && crate::theme_picker::selected_zed(cx).is_some_and(|id| {
                        id.split_once('/')
                            .is_some_and(|(extension, _)| extension == changed_id)
                    })
                {
                    crate::preferences_view::set_zed_theme(cx, None);
                }
                crate::shell_theme::apply(gpui_kit::component::theme::ThemeMode::Dark, None, cx);
                cx.refresh_windows();
            });
        })
        .detach();
        cx.notify();
        cx.refresh_windows();
    }
}

impl Render for ThemesSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalog = cx.global::<ThemeCatalog>();
        let loading = catalog.loading;
        let error = catalog.error.clone();
        let extensions = catalog.extensions.clone();
        let active = crate::theme_picker::selected_zed(cx).map(str::to_owned);
        v_flex().w_full().gap_4()
            .child(div().text_lg().child("Themes"))
            .child(crate::theme_picker::appearance(cx))
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child("Install theme data from Zed extensions. Downloads happen only when you choose Install; extension code is never run."))
            .child(h_flex().flex_wrap().gap_2()
                .child(Button::new("theme-marketplace").ghost().label("Browse marketplace")
                    .on_click(|_, _, cx| cx.open_url("https://zed.dev/extensions?filter=themes")))
                .child(Button::new("theme-catppuccin").label("Install Catppuccin").disabled(loading)
                    .on_click(cx.listener(|this, _, window, cx| this.change("catppuccin", false, window, cx))))
                .child(Button::new("theme-github").label("Install GitHub").disabled(loading)
                    .on_click(cx.listener(|this, _, window, cx| this.change("github-theme", false, window, cx)))))
            .child(h_flex().w_full().gap_2()
                .child(Input::new(&self.extension_input).small())
                .child(Button::new("theme-install").label("Install").disabled(loading)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let input = this.extension_input.read(cx).value().to_string();
                        this.change(&input, false, window, cx);
                    }))))
            .when(loading, |el| el.child("Updating installed themes…"))
            .when_some(error, |el, error| el.child(div().text_sm().child(error)))
            .when(extensions.is_empty() && !loading, |el| el.child("No theme extensions installed. System, Dark and Light are always available."))
            .children(extensions.into_iter().enumerate().map(|(ix, extension)| {
                let id = extension.id.clone();
                v_flex().w_full().gap_2().p_3().border_1().border_color(cx.theme().border).rounded_md()
                    .child(h_flex().w_full().justify_between().gap_2()
                        .child(format!("{} · {}", extension.id, extension.version))
                        .child(Button::new(("theme-uninstall", ix)).small().ghost().label("Uninstall").disabled(loading)
                            .on_click(cx.listener(move |this, _, window, cx| this.change(&id, true, window, cx)))))
                    .child(h_flex().flex_wrap().gap_2().children(extension.themes.into_iter().enumerate().map(|(tx, theme)| {
                        let selected = active.as_deref() == Some(theme.id.as_str());
                        Button::new(SharedString::from(format!("installed-theme-{ix}-{tx}")))
                            .small().ghost().selected(selected)
                            .label(format!("{} · {}", theme.name, theme.appearance))
                            .on_click(move |_, _, cx| crate::preferences_view::set_zed_theme(cx, Some(theme.id.clone())))
                    })))
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::extension_id;
    #[test]
    fn accepts_ids_and_exact_marketplace_urls() {
        for input in [
            "catppuccin",
            " catppuccin ",
            "https://zed.dev/extensions/catppuccin",
            "https://zed.dev/extensions/catppuccin/",
        ] {
            assert_eq!(extension_id(input).unwrap(), "catppuccin");
        }
        assert_eq!(extension_id("github-theme").unwrap(), "github-theme");
        assert_eq!(extension_id("theme_123").unwrap(), "theme_123");
    }
    #[test]
    fn rejects_other_urls_paths_and_invalid_ids() {
        for input in [
            "",
            "../theme",
            "UPPER",
            "-theme",
            "a.b",
            "http://zed.dev/extensions/a",
            "https://evil.test/extensions/a",
            "https://zed.dev.evil.test/extensions/a",
            "https://zed.dev/extensions/a/b",
            "https://zed.dev/extensions/a?x=1",
            "https://zed.dev/extensions/a#theme",
            "https://zed.dev/extensions/%61",
            "https://zed.dev/extensions/a//",
        ] {
            assert!(extension_id(input).is_err(), "{input}");
        }
        assert!(extension_id(&"a".repeat(129)).is_err());
    }
}
