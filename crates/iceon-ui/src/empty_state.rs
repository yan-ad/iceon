use gpui_kit::component::{
    Icon, IconName,
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle},
};
use gpui_kit::prelude::*;
use gpui_kit::*;

pub(crate) fn empty_state(
    icon: IconName,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> impl IntoElement {
    Empty::new()
        .w_full()
        .min_h(px(128.0))
        .items_center()
        .justify_center()
        .p_6()
        .border_1()
        .border_color(transparent_black())
        .header(
            EmptyHeader::new()
                .media(
                    EmptyMedia::new()
                        .with_variant(EmptyMediaVariant::Icon)
                        .child(Icon::new(icon)),
                )
                .title(EmptyTitle::new().child(title.into()))
                .description(EmptyDescription::new().child(description.into())),
        )
}

/// Full content-area empty state; never sized by the text alone.
pub(crate) fn page_empty_state(
    icon: IconName,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> impl IntoElement {
    div()
        .size_full()
        .min_h_0()
        .min_w_0()
        .flex()
        .items_center()
        .justify_center()
        .child(empty_state(icon, title, description))
}

/// Same visual language for embedded cards and inspector collections.
pub(crate) fn compact_empty_state(
    icon: IconName,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> impl IntoElement {
    Empty::new()
        .w_full()
        .min_h(px(96.))
        .p_3()
        .items_center()
        .justify_center()
        .header(
            EmptyHeader::new()
                .media(
                    EmptyMedia::new()
                        .with_variant(EmptyMediaVariant::Icon)
                        .child(Icon::new(icon)),
                )
                .title(EmptyTitle::new().text_sm().child(title.into()))
                .description(EmptyDescription::new().text_xs().child(description.into())),
        )
}

pub(crate) fn error_state(message: impl Into<SharedString>) -> impl IntoElement {
    empty_state(IconName::CircleAlert, "Unable to load this view", message)
}
