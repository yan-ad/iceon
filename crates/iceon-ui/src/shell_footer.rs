//! Persistent shell chrome shared visually with console tabs.
use crate::IceonApp;
use gpui_kit::{
    component::{
        ActiveTheme as _, Disableable as _, IconName, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
    },
    prelude::*,
    *,
};

// One physical row: the top separator plus the session strip. Keep this
// relationship explicit so the strip cannot leave an empty band underneath it.
pub(crate) const FOOTER_HEIGHT: f32 = 34.;
const FOOTER_BORDER: f32 = 1.;

pub(crate) fn footer_frame(border: Hsla) -> Div {
    h_flex()
        .relative()
        .w_full()
        .h(px(FOOTER_HEIGHT))
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .border_t(px(FOOTER_BORDER))
        .border_color(border)
}
#[derive(Default)]
pub(crate) enum UpdateStatus {
    #[default]
    Unchecked,
    Current,
    Available(String),
    Failed,
}
impl UpdateStatus {
    fn label(&self, checking: bool) -> String {
        if checking {
            return "Checking…".into();
        }
        match self {
            Self::Unchecked => "Check for updates".into(),
            Self::Current => "Up to date".into(),
            Self::Available(version) => format!("Update available · {version}"),
            Self::Failed => "Check failed · Retry".into(),
        }
    }
}
impl IceonApp {
    pub(crate) fn render_shell_footer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sidebar_open = self.cluster_sidebar_open();
        footer_frame(crate::shell_theme::chrome_border(cx.theme()))
            .text_color(cx.theme().sidebar_foreground)
            .child(
                crate::shell_theme::chrome_background(
                    sidebar_open.then_some(self.navigation_widths.sidebar_width()),
                    cx,
                )
                // The absolute material layer must not paint over the top
                // separator; children paint after the parent's border.
                .top(px(FOOTER_BORDER)),
            )
            .child(
                h_flex()
                    .relative()
                    .h_full()
                    .when(sidebar_open, |controls| {
                        controls
                            .w(px(self.navigation_widths.sidebar_width()))
                            .border_r_1()
                            .border_color(cx.theme().sidebar_border)
                    })
                    .flex_shrink_0()
                    .px_2()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("footer-github")
                            .xsmall()
                            .ghost()
                            .icon(gpui_kit_assets::IconName::Github)
                            .accessibility_label("GitHub")
                            .tooltip("GitHub")
                            .on_click(|_, _, cx| cx.open_url(iceon_core::project::REPOSITORY_URL)),
                    )
                    .child(
                        Button::new("footer-settings")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Settings)
                            .accessibility_label("Settings")
                            .tooltip("Settings")
                            .on_click(
                                cx.listener(|app, _, window, cx| app.open_preferences(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("footer-theme")
                            .xsmall()
                            .ghost()
                            .icon(if cx.theme().mode.is_dark() {
                                IconName::Moon
                            } else {
                                IconName::Sun
                            })
                            .accessibility_label("Theme options")
                            .tooltip("Theme options")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.toggle_theme_picker(window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("footer-updates")
                            .xsmall()
                            .ghost()
                            .icon(gpui_kit_assets::IconName::Download)
                            .accessibility_label("Check for Updates")
                            .tooltip(format!(
                                "Iceon v{} · {}",
                                env!("CARGO_PKG_VERSION"),
                                self.update_status.label(self.update_check_running)
                            ))
                            .disabled(self.update_check_running)
                            .on_click(
                                cx.listener(|app, _, window, cx| app.check_for_updates(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .track_focus(&self.console_focus)
                    .key_context("IceonConsole")
                    .child(self.render_console_tabs(window, cx)),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::{FOOTER_BORDER, FOOTER_HEIGHT, UpdateStatus, footer_frame};
    use gpui_kit::{Styled as _, px, rgb};

    #[test]
    fn footer_reserves_exactly_one_session_row_and_separator() {
        let mut frame = footer_frame(rgb(0x112233).into());
        assert_eq!(frame.style().size.height, Some(px(FOOTER_HEIGHT).into()));
        assert_eq!(
            frame.style().border_widths.top,
            Some(px(FOOTER_BORDER).into())
        );
        assert_eq!(FOOTER_HEIGHT - FOOTER_BORDER, 33.);
        assert_eq!(frame.style().border_widths.bottom, None);
    }
    #[test]
    fn update_status_never_claims_current_before_success() {
        assert_eq!(UpdateStatus::default().label(false), "Check for updates");
        assert_eq!(UpdateStatus::Current.label(false), "Up to date");
        assert_eq!(UpdateStatus::Current.label(true), "Checking…");
        assert_ne!(UpdateStatus::Failed.label(false), "Up to date");
    }
}
