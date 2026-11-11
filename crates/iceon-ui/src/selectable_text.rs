use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::*;

/// Render non-interactive UI copy through gpui-kit's native selection surface.
/// Buttons, inputs, editors, and terminals keep their specialized interaction.
pub(crate) fn selectable_text(text: impl Into<SharedString>) -> TextView {
    let text = text.into();
    TextView::markdown(
        ElementId::Name(SharedString::from(format!(
            "selectable-text-{}",
            text.len()
        ))),
        text,
    )
    .selectable(true)
}

#[allow(dead_code)]
pub(crate) fn selectable_text_style(
    text: impl Into<SharedString>,
    style: TextViewStyle,
) -> TextView {
    selectable_text(text).style(style)
}
