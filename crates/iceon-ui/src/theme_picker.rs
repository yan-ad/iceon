//! Shared theme selection: label-only menus and palette previews.
use gpui_kit::component::{
    ActiveTheme as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::preferences::AppTheme;

fn families() -> Vec<&'static str> {
    let mut families = vec!["System", "Dark", "Light"];
    for theme in AppTheme::ALL {
        if theme.family() != "Default" && !families.contains(&theme.family()) {
            families.push(theme.family());
        }
    }
    families
}

fn variants(family: &str) -> Vec<AppTheme> {
    AppTheme::ALL
        .iter()
        .copied()
        .filter(|theme| {
            if theme.family() == "Default" {
                theme.label() == family
            } else {
                theme.family() == family
            }
        })
        .collect()
}

fn selected(cx: &App) -> AppTheme {
    cx.try_global::<crate::preferences_view::GlobalPreferencesState>()
        .map(|state| state.preferences.app.theme)
        .unwrap_or(AppTheme::System)
}

fn choice(theme: AppTheme, current: AppTheme, full_label: bool) -> PopupMenuItem {
    let label = if full_label && theme.family() != "Default" {
        format!("{} {}", theme.family(), theme.label())
    } else {
        theme.label().to_owned()
    };
    PopupMenuItem::new(label)
        .checked(theme == current)
        .on_click(move |_, _, cx| {
            crate::preferences_view::set_theme(cx, theme);
        })
}

pub(crate) fn menu(mut menu: PopupMenu, cx: &App) -> PopupMenu {
    let current = selected(cx);
    let mut family = "Default";
    for &theme in AppTheme::ALL {
        if theme.family() != family {
            family = theme.family();
            menu = menu.separator().item(PopupMenuItem::label(family));
        }
        menu = menu.item(choice(theme, current, false));
    }
    menu
}

/// Wrap cards so the settings page remains usable at its minimum window width.
pub(crate) fn appearance(cx: &App) -> AnyElement {
    let current = selected(cx);
    h_flex()
        .w_full()
        .flex_wrap()
        .gap_3()
        .children(families().into_iter().map(|family| {
            let variants = variants(family);
            let active = variants.contains(&current);
            let shown = if active { current } else { variants[0] };
            let [background, sidebar, surface, accent] = crate::shell_theme::preview_colors(shown);
            let description = if shown.family() == "Default" {
                shown.label().to_owned()
            } else {
                format!("{} {}", shown.family(), shown.label())
            };
            let preview = h_flex()
                .w_full()
                .h(px(82.))
                .rounded(px(10.))
                .overflow_hidden()
                .bg(rgb(background))
                .border_2()
                .border_color(if active {
                    cx.theme().ring
                } else {
                    cx.theme().border
                })
                .child(
                    v_flex()
                        .w(px(28.))
                        .h_full()
                        .p_2()
                        .gap_2()
                        .bg(rgb(sidebar))
                        .children(
                            (0..3).map(|_| div().w_full().h(px(3.)).rounded_full().bg(rgb(accent))),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .p_2()
                        .gap_2()
                        .child(div().w(px(36.)).h(px(4.)).rounded_full().bg(rgb(accent)))
                        .child(h_flex().gap_1().children((0..3).map(|i| {
                            div().flex_1().h(px(22.)).rounded(px(4.)).bg(rgb(if i == 0 {
                                accent
                            } else {
                                surface
                            }))
                        })))
                        .child(div().w_full().h(px(9.)).rounded_full().bg(rgb(surface))),
                );
            let button = Button::new(SharedString::from(format!("theme-preview-{family}")))
                .ghost()
                .w_full()
                .h_auto()
                .p_0()
                .child(preview)
                .accessibility_label(format!("Select {description}"));
            let control = if variants.len() == 1 {
                button
                    .on_click(move |_, _, cx| crate::preferences_view::set_theme(cx, shown))
                    .into_any_element()
            } else {
                button
                    .dropdown_menu(move |mut menu, _, cx| {
                        let current = selected(cx);
                        for &variant in &variants {
                            menu = menu.item(choice(variant, current, false));
                        }
                        menu
                    })
                    .into_any_element()
            };
            v_flex().w(px(142.)).gap_1().child(control).child(
                div()
                    .text_xs()
                    .text_center()
                    .text_color(if active {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(description),
            )
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{families, variants};
    use iceon_core::preferences::AppTheme;

    #[test]
    fn every_theme_has_exactly_one_nonempty_preview_family() {
        let all = families()
            .iter()
            .flat_map(|family| {
                let choices = variants(family);
                assert!(!choices.is_empty());
                choices
            })
            .collect::<Vec<_>>();
        assert_eq!(all.len(), AppTheme::ALL.len());
        for theme in AppTheme::ALL {
            assert_eq!(
                all.iter().filter(|candidate| *candidate == theme).count(),
                1
            );
        }
    }
}
