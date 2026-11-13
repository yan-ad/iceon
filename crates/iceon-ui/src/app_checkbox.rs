//! Quiet, Zed-style checkbox presentation over GPUI's accessible base control.

use std::rc::Rc;

use gpui_kit::base::{Checkbox as BaseCheckbox, CheckboxState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
use gpui_kit::prelude::*;
use gpui_kit::*;

const INDICATOR_SIZE: f32 = 16.;
const INDICATOR_RADIUS: f32 = 2.;
type ChangeHandler = Rc<dyn Fn(CheckboxState, &ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct Checkbox {
    id: ElementId,
    state: CheckboxState,
    disabled: bool,
    label: Option<SharedString>,
    accessibility_label: Option<SharedString>,
    on_change: Option<ChangeHandler>,
}

impl Checkbox {
    pub(crate) fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            state: CheckboxState::Unchecked,
            disabled: false,
            label: None,
            accessibility_label: None,
            on_change: None,
        }
    }

    pub(crate) fn state(mut self, state: CheckboxState) -> Self {
        self.state = state;
        self
    }

    pub(crate) fn checked(self, checked: bool) -> Self {
        self.state(if checked {
            CheckboxState::Checked
        } else {
            CheckboxState::Unchecked
        })
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(crate) fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub(crate) fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    pub(crate) fn on_change(
        mut self,
        handler: impl Fn(CheckboxState, &ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    pub(crate) fn on_click(self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change(move |state, _, window, cx| {
            handler(&(state == CheckboxState::Checked), window, cx);
        })
    }
}

fn mark(state: CheckboxState) -> Option<IconName> {
    match state {
        CheckboxState::Unchecked => None,
        CheckboxState::Checked => Some(IconName::Check),
        CheckboxState::Indeterminate => Some(IconName::Minus),
    }
}

fn indicator_style(
    background: Hsla,
    border: Hsla,
    foreground: Hsla,
    disabled: bool,
) -> StyleRefinement {
    StyleRefinement::default()
        .size(px(INDICATOR_SIZE))
        .rounded(px(INDICATOR_RADIUS))
        .border_1()
        .border_color(border)
        .bg(background)
        .text_color(foreground)
        .opacity(if disabled { 0.45 } else { 1. })
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let theme = cx.theme();
        let active = self.state != CheckboxState::Unchecked;
        // Checked boxes stay neutral, rather than using the brand/button accent.
        let background = if active {
            theme.background
        } else {
            theme.button_secondary
        };
        let border = theme.border;
        let hover_border = theme.muted_foreground;
        let ring = theme.ring;
        let mark_color = theme.muted_foreground;
        let label_color = if self.disabled {
            theme.muted_foreground
        } else {
            theme.foreground
        };
        let indicator = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .map(|mut indicator| {
                indicator.style().refine(&indicator_style(
                    background,
                    border,
                    mark_color,
                    self.disabled,
                ));
                indicator
            })
            .when(!self.disabled, |indicator| {
                indicator.group_hover("iceon-checkbox", |style| style.border_color(hover_border))
            })
            .when_some(mark(self.state), |indicator, icon| {
                indicator.child(Icon::new(icon).size(px(12.)).text_color(mark_color))
            });
        BaseCheckbox::new(self.id)
            .state(self.state)
            .disabled(self.disabled)
            .track_focus(&focus)
            .group("iceon-checkbox")
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(INDICATOR_RADIUS))
            .text_sm()
            .text_color(label_color)
            .when(!self.disabled, |control| {
                control
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        window.focus(&focus, cx);
                        window.prevent_default();
                    })
            })
            .focus_visible(|style| {
                style.shadow(vec![BoxShadow {
                    inset: false,
                    color: ring,
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                }])
            })
            .when_some(
                self.accessibility_label.or_else(|| self.label.clone()),
                |control, label| control.accessibility_label(label),
            )
            .child(indicator)
            .when_some(self.label, |control, label| control.child(label))
            .when_some(self.on_change, |control, handler| {
                control.on_change(move |state, event, window, cx| {
                    window.prevent_default();
                    handler(state, event, window, cx);
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{Checkbox, INDICATOR_RADIUS, INDICATOR_SIZE, indicator_style, mark};
    use gpui_kit::base::CheckboxState;
    use gpui_kit::{Hsla, Styled as _, px, rgb};

    #[test]
    fn checked_and_mixed_states_use_distinct_marks_and_unchecked_has_none() {
        assert!(mark(CheckboxState::Unchecked).is_none());
        assert!(matches!(
            mark(CheckboxState::Checked),
            Some(gpui_kit::component::IconName::Check)
        ));
        assert!(matches!(
            mark(CheckboxState::Indeterminate),
            Some(gpui_kit::component::IconName::Minus)
        ));
    }

    #[test]
    fn geometry_and_theme_colors_are_preserved_in_light_and_dark_palettes() {
        for (background, border, foreground) in [
            (0x181818, 0x484848, 0x84938a),
            (0xf8f8f8, 0xa8a8a8, 0x405448),
        ] {
            let bg = Hsla::from(rgb(background));
            let edge = Hsla::from(rgb(border));
            let ink = Hsla::from(rgb(foreground));
            for disabled in [false, true] {
                let mut style = indicator_style(bg, edge, ink, disabled);
                assert_eq!(
                    style.background.as_ref().unwrap().color().unwrap(),
                    bg.into()
                );
                assert_eq!(style.border_color, Some(edge));
                assert_eq!(style.text_style().color, Some(ink));
                assert_eq!(style.size.width, Some(px(INDICATOR_SIZE).into()));
                assert_eq!(style.size.height, Some(px(INDICATOR_SIZE).into()));
                assert_eq!(
                    style.corner_radii.top_left,
                    Some(px(INDICATOR_RADIUS).into())
                );
                assert_eq!(style.opacity, Some(if disabled { 0.45 } else { 1. }));
            }
        }
    }

    #[test]
    fn bool_builder_clears_mixed_state_and_labels_do_not_change_the_value() {
        let control = Checkbox::new("mixed")
            .state(CheckboxState::Indeterminate)
            .checked(false)
            .label("Select")
            .accessibility_label("Select resource");
        assert_eq!(control.state, CheckboxState::Unchecked);
        assert_eq!(control.label.as_deref(), Some("Select"));
        assert_eq!(
            control.accessibility_label.as_deref(),
            Some("Select resource")
        );
        assert_eq!(
            Checkbox::new("checked").checked(true).state,
            CheckboxState::Checked
        );
    }
}
