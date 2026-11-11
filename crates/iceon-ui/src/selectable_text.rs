use gpui_kit::component::text::TextView;
use gpui_kit::*;
use std::hash::{Hash, Hasher};

fn literal_markdown(text: &str) -> String {
    let mut literal = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            literal.push('\\');
        }
        literal.push(ch);
    }
    literal
}

/// Literal native selectable text. Double click selects a word; triple click
/// selects a line via gpui-kit's window selection layer. Controls remain native.
#[track_caller]
pub(crate) fn selectable_text(text: impl Into<SharedString>) -> TextView {
    let text = text.into();
    let location = std::panic::Location::caller();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (
        location.file(),
        location.line(),
        location.column(),
        text.as_ref(),
    )
        .hash(&mut hash);
    TextView::markdown(
        SharedString::from(format!("selectable-text-{:016x}", hash.finish())),
        literal_markdown(&text),
    )
    .selectable(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn literal_text_does_not_create_markdown_links_images_or_formatting() {
        assert_eq!(
            super::literal_markdown("[pod](url) *a* <img> #x"),
            "\\[pod\\]\\(url\\) \\*a\\* \\<img\\> \\#x"
        );
        assert_eq!(super::literal_markdown("hello\n世界"), "hello\n世界");
    }
}
