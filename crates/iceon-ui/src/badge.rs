//! Lightweight status and metadata badge component for resource tables, inspector cards, and pod dock.

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeVariant {
    Success,
    Warning,
    Danger,
    Secondary,
    Outline,
}

#[derive(IntoElement)]
pub struct Badge {
    variant: BadgeVariant,
    small: bool,
    show_dot: bool,
    label: SharedString,
}

impl Badge {
    pub fn new(variant: BadgeVariant) -> Self {
        Self {
            variant,
            small: false,
            show_dot: matches!(
                variant,
                BadgeVariant::Success | BadgeVariant::Warning | BadgeVariant::Danger
            ),
            label: SharedString::default(),
        }
    }

    pub fn success() -> Self {
        Self::new(BadgeVariant::Success)
    }

    pub fn warning() -> Self {
        Self::new(BadgeVariant::Warning)
    }

    pub fn danger() -> Self {
        Self::new(BadgeVariant::Danger)
    }

    pub fn secondary() -> Self {
        Self::new(BadgeVariant::Secondary)
    }

    pub fn outline() -> Self {
        Self::new(BadgeVariant::Outline)
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    pub fn without_icon(mut self) -> Self {
        self.show_dot = false;
        self
    }

    pub fn child(mut self, text: impl Into<SharedString>) -> Self {
        self.label = text.into();
        self
    }

    pub fn for_resource_status(status: &str) -> Self {
        let lower = status.to_ascii_lowercase();
        let variant = if lower.contains("running")
            || lower.contains("ready")
            || lower.contains("active")
            || lower.contains("bound")
            || lower.contains("complete")
            || lower.contains("succeeded")
            || lower.contains("deployed")
            || lower.contains("available")
            || lower.contains("healthy")
            || lower.contains("abletoscale")
            || lower.contains("scalingactive")
            || lower == "normal"
            || lower == "true"
        {
            BadgeVariant::Success
        } else if lower.contains("pending")
            || lower.contains("terminating")
            || lower.contains("warning")
            || lower.contains("waiting")
            || lower.contains("progressing")
            || lower.contains("scaling")
            || lower.contains("suspended")
            || lower.contains("schedulingdisabled")
        {
            BadgeVariant::Warning
        } else if lower.contains("fail")
            || lower.contains("error")
            || lower.contains("crash")
            || lower.contains("notready")
            || lower.contains("lost")
            || lower.contains("blocked")
            || lower == "false"
        {
            BadgeVariant::Danger
        } else {
            BadgeVariant::Secondary
        };
        Self::new(variant).small().child(status.to_owned())
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_dark = cx.theme().mode.is_dark();
        let (bg, fg, border, dot): (Hsla, Hsla, Hsla, Hsla) = match self.variant {
            BadgeVariant::Success => {
                let c = cx.theme().success;
                (
                    c.alpha(if is_dark { 0.16 } else { 0.12 }),
                    c,
                    c.alpha(0.32),
                    c,
                )
            }
            BadgeVariant::Warning => {
                let c = cx.theme().warning;
                (
                    c.alpha(if is_dark { 0.16 } else { 0.12 }),
                    c,
                    c.alpha(0.32),
                    c,
                )
            }
            BadgeVariant::Danger => {
                let c = cx.theme().danger;
                (
                    c.alpha(if is_dark { 0.16 } else { 0.12 }),
                    c,
                    c.alpha(0.32),
                    c,
                )
            }
            BadgeVariant::Secondary => (
                cx.theme().secondary,
                cx.theme().muted_foreground,
                cx.theme().border,
                cx.theme().muted_foreground,
            ),
            BadgeVariant::Outline => (
                gpui_kit::transparent_black(),
                cx.theme().muted_foreground,
                cx.theme().border,
                cx.theme().muted_foreground,
            ),
        };

        h_flex()
            .items_center()
            .gap_1()
            .px(if self.small { px(6.0) } else { px(8.0) })
            .py(if self.small { px(1.5) } else { px(2.5) })
            .rounded(px(5.0))
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_xs()
            .text_color(fg)
            .when(self.show_dot, |el| {
                el.child(div().size(px(5.0)).rounded_full().bg(dot))
            })
            .child(self.label)
    }
}
