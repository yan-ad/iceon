//! Native editor chrome driven by the editor's real buffer and display map.
//!
//! The kit exposes only a boolean line-number policy and no minimap renderer.
//! Keep its absolute gutter for `On`; other policies use a viewport gutter.
//! Neither navigation nor preferences replace the editor entity/undo history.
use gpui_kit::component::{
    ActiveTheme as _,
    input::{DisplayPoint, Editor, EditorState, RopeExt as _},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use iceon_core::preferences::{EditorPreferences, LineNumbers, MinimapSide};

const MAP_WIDTH: f32 = 64.0;
const INTERVAL: usize = 10;

/// Zero-based buffer lines in, one-based labels out. The active line always
/// shows its absolute number (also in relative and interval modes).
fn line_label(mode: LineNumbers, line: usize, cursor: usize) -> Option<usize> {
    match mode {
        LineNumbers::Off => None,
        LineNumbers::On => Some(line + 1),
        LineNumbers::Relative if line != cursor => Some(line.abs_diff(cursor)),
        LineNumbers::Relative => Some(line + 1),
        LineNumbers::Interval
            if line == cursor || line == 0 || (line + 1).is_multiple_of(INTERVAL) =>
        {
            Some(line + 1)
        }
        LineNumbers::Interval => None,
    }
}

/// Limit thumbnail rows to three pixels without stretching short documents.
fn map_step(rows: usize, height: f32) -> f32 {
    (height.max(1.0) / rows.max(1) as f32).min(3.0)
}

fn map_row(y: f32, rows: usize, height: f32) -> usize {
    ((y.max(0.0) / map_step(rows, height)).floor() as usize).min(rows.saturating_sub(1))
}

/// Content-derived ink spans, with a bounded column budget. Tabs retain their
/// indentation and Unicode characters consume columns rather than UTF-8 bytes.
fn ink_spans(text: &str, tab_size: usize) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut col = 0;
    let mut start = None;
    for ch in text.chars() {
        if col >= 96 || ch == '\n' || ch == '\r' {
            break;
        }
        if ch.is_whitespace() {
            if let Some(start) = start.take() {
                spans.push((start, col));
            }
        } else {
            start.get_or_insert(col);
        }
        col += if ch == '\t' {
            tab_size.max(1) - col % tab_size.max(1)
        } else {
            1
        };
    }
    if let Some(start) = start {
        spans.push((start, col.min(96)));
    }
    spans
}

/// Call from the shell preference synchronizer instead of mapping all modes to
/// `!= Off`. The surface also applies this on mount, including hidden editors.
pub(crate) fn apply_line_number_mode(
    state: &Entity<EditorState>,
    mode: LineNumbers,
    window: &mut Window,
    cx: &mut App,
) {
    state.update(cx, |state, cx| {
        state.set_line_number(mode == LineNumbers::On, window, cx)
    });
}

struct EditorObservation {
    _subscription: Subscription,
}

#[derive(IntoElement)]
pub(crate) struct EditorSurface {
    state: Entity<EditorState>,
    preferences: EditorPreferences,
}

impl EditorSurface {
    pub(crate) fn new(state: &Entity<EditorState>, preferences: &EditorPreferences) -> Self {
        Self {
            state: state.clone(),
            preferences: preferences.clone(),
        }
    }
}

impl RenderOnce for EditorSurface {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        apply_line_number_mode(&self.state, self.preferences.line_numbers, window, cx);
        // Cursor movement, edits, folds and scrolling all notify EditorState.
        // Observe notifications, not just InputEvent::Change (which misses
        // cursor-only changes). Keyed state drops this subscription on unmount.
        let observed = self.state.clone();
        let _observation = window.use_keyed_state(
            ("editor-surface-observer", self.state.entity_id()),
            cx,
            |_, cx| EditorObservation {
                _subscription: cx.observe(&observed, |_, _, cx| cx.notify()),
            },
        );
        let custom_gutter = matches!(
            self.preferences.line_numbers,
            LineNumbers::Relative | LineNumbers::Interval
        );
        let digits = self
            .state
            .read(cx)
            .text()
            .lines_len()
            .max(1)
            .to_string()
            .len()
            .max(3);
        let gutter_width = if custom_gutter {
            digits as f32 * f32::from(cx.theme().mono_font_size) * 0.65 + 12.0
        } else {
            0.0
        };
        let map_left =
            self.preferences.minimap && self.preferences.minimap_side == MinimapSide::Left;
        div()
            .relative()
            .flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .when(map_left, |el| {
                el.child(div().w(px(MAP_WIDTH)).h_full().flex_shrink_0())
            })
            .when(custom_gutter, |el| {
                el.child(div().w(px(gutter_width)).h_full().flex_shrink_0())
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(Editor::new(&self.state).h_full()),
            )
            .when(self.preferences.minimap && !map_left, |el| {
                el.child(div().w(px(MAP_WIDTH)).h_full().flex_shrink_0())
            })
            .when(self.preferences.minimap, |el| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .h_full()
                        .w(px(MAP_WIDTH))
                        .when(map_left, |el| el.left_0())
                        .when(!map_left, |el| el.right_0())
                        .child(minimap(&self.state, self.preferences.tab_size)),
                )
            })
            // Paint after the editor, reading this frame's published geometry.
            .when(custom_gutter, |el| {
                el.child(
                    gutter(&self.state, self.preferences.line_numbers)
                        .absolute()
                        .top_0()
                        .left(px(if map_left { MAP_WIDTH } else { 0.0 }))
                        .w(px(gutter_width))
                        .h_full(),
                )
            })
    }
}

fn gutter(state: &Entity<EditorState>, mode: LineNumbers) -> impl IntoElement + Styled {
    let state = state.clone();
    canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            let state = state.read(cx);
            let Some(height) = state.line_height() else {
                return;
            };
            let Some(visible) = state.visible_row_range() else {
                return;
            };
            let Some(text_bounds) = state.text_bounds() else {
                return;
            };
            let cursor = state.cursor_position().line as usize;
            let color = cx.theme().muted_foreground;
            let font_size = cx.theme().mono_font_size;
            let font = font(cx.theme().mono_font_family.clone());
            let labels: Vec<_> = visible
                .filter_map(|line| {
                    let rows = state.display_map().buffer_line_to_display_row_range(line)?;
                    Some((rows.start, line_label(mode, line, cursor)?))
                })
                .collect();
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                window.paint_quad(fill(bounds, cx.theme().background));
                for (row, label) in labels {
                    let text: SharedString = label.to_string().into();
                    let shaped = window.text_system().shape_line(
                        text.clone(),
                        font_size,
                        &[TextRun {
                            len: text.len(),
                            font: font.clone(),
                            color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    );
                    let y = text_bounds.top() + height * row as f32;
                    let _ = shaped.paint(
                        point(bounds.right() - shaped.width - px(6.0), y),
                        height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            });
        },
    )
}

fn navigate(state: &Entity<EditorState>, bounds: Bounds<Pixels>, y: Pixels, cx: &mut App) {
    state.update(cx, |state, cx| {
        let rows = state.display_map().display_row_count().max(1);
        let row = map_row(
            f32::from(y - bounds.top()),
            rows,
            f32::from(bounds.size.height),
        );
        let height = state.line_height().unwrap_or(px(18.0));
        let viewport = state.input_bounds().size.height;
        let offset = state.scroll_offset();
        // Scroll only: do not change selection, focus, dirty state or undo.
        state.set_scroll_offset(
            point(
                offset.x,
                -(height * row as f32 - viewport / 2.0).max(px(0.0)),
            ),
            cx,
        );
    });
}

fn minimap(state: &Entity<EditorState>, tab_size: u8) -> impl IntoElement {
    let state = state.clone();
    div()
        .w(px(MAP_WIDTH))
        .h_full()
        .flex_shrink_0()
        .overflow_hidden()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, cx| {
                    let editor = state.read(cx);
                    let map = editor.display_map();
                    let rows = map.display_row_count().max(1);
                    let height = f32::from(bounds.size.height);
                    let step = map_step(rows, height);
                    let ink = cx.theme().muted_foreground.opacity(0.65);
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        window.paint_quad(fill(bounds, cx.theme().muted));
                        // Sample at pixel resolution for huge buffers; never build a
                        // second editor or allocate a thumbnail of the entire document.
                        let samples = (rows as f32 * step).ceil() as usize;
                        let mut previous = None;
                        for sample in 0..samples {
                            let row = map_row(sample as f32, rows, height);
                            let line = map.display_row_to_buffer_line(row);
                            if previous == Some((row, line)) {
                                continue;
                            }
                            previous = Some((row, line));
                            if line >= editor.text().lines_len() {
                                continue;
                            }
                            let start = map.display_pos_to_buffer_pos(DisplayPoint::new(row, 0));
                            let end = map.display_pos_to_buffer_pos(DisplayPoint::new(row + 1, 0));
                            let source = editor.text().slice_line(line);
                            let end_col = if row + 1 < rows && end.line == line {
                                end.col
                            } else {
                                source.len()
                            };
                            let snippet: String = source
                                .slice(
                                    start.col.min(source.len())
                                        ..end_col
                                            .min(source.len())
                                            .max(start.col.min(source.len())),
                                )
                                .chars()
                                .take(96)
                                .collect();
                            for (start, end) in ink_spans(&snippet, usize::from(tab_size)) {
                                let x = start as f32 * 0.58 + 3.0;
                                let width = (end - start) as f32 * 0.58;
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(
                                            bounds.left() + px(x),
                                            bounds.top() + px(row as f32 * step),
                                        ),
                                        size(px(width), px(step.min(1.5))),
                                    ),
                                    ink,
                                ));
                            }
                        }
                        if let Some(line_height) = editor.line_height() {
                            let scroll_row = f32::from(-editor.scroll_offset().y)
                                / f32::from(line_height).max(1.0);
                            let visible_rows = f32::from(editor.input_bounds().size.height)
                                / f32::from(line_height).max(1.0);
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(bounds.left(), bounds.top() + px(scroll_row * step)),
                                    size(bounds.size.width, px((visible_rows * step).max(3.0))),
                                ),
                                cx.theme().primary.opacity(0.18),
                            ));
                        }
                    });
                    let click_state = state.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble
                            && event.button == MouseButton::Left
                            && bounds.contains(&event.position)
                        {
                            navigate(&click_state, bounds, event.position.y, cx);
                            cx.stop_propagation();
                        }
                    });
                    let drag_state = state.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble
                            && event.dragging()
                            && bounds.contains(&event.position)
                        {
                            navigate(&drag_state, bounds, event.position.y, cx);
                            cx.stop_propagation();
                        }
                    });
                    let wheel_state = state.clone();
                    window.on_mouse_event(move |event: &ScrollEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble && bounds.contains(&event.position) {
                            wheel_state.update(cx, |state, cx| {
                                let delta = match event.delta {
                                    ScrollDelta::Pixels(delta) => delta.y,
                                    ScrollDelta::Lines(delta) => {
                                        state.line_height().unwrap_or(px(18.0)) * delta.y
                                    }
                                };
                                let offset = state.scroll_offset();
                                state.set_scroll_offset(point(offset.x, offset.y + delta), cx);
                            });
                            cx.stop_propagation();
                        }
                    });
                },
            )
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::{ink_spans, line_label, map_row, map_step};
    use iceon_core::preferences::LineNumbers;
    #[test]
    fn modes_are_distinct_and_cursor_is_absolute() {
        assert_eq!(line_label(LineNumbers::On, 6, 9), Some(7));
        assert_eq!(line_label(LineNumbers::Off, 6, 9), None);
        assert_eq!(line_label(LineNumbers::Relative, 6, 9), Some(3));
        assert_eq!(line_label(LineNumbers::Relative, 12, 9), Some(3));
        assert_eq!(line_label(LineNumbers::Relative, 9, 9), Some(10));
        assert_eq!(line_label(LineNumbers::Interval, 6, 9), None);
        assert_eq!(line_label(LineNumbers::Interval, 9, 6), Some(10));
        assert_eq!(line_label(LineNumbers::Interval, 6, 6), Some(7));
        assert_eq!(line_label(LineNumbers::Interval, 0, 6), Some(1));
    }
    #[test]
    fn thumbnail_navigation_clamps_and_short_documents_do_not_stretch() {
        assert_eq!(map_step(10, 300.0), 3.0);
        assert_eq!(map_row(-5.0, 10, 300.0), 0);
        assert_eq!(map_row(15.0, 10, 300.0), 5);
        assert_eq!(map_row(500.0, 10, 300.0), 9);
        assert_eq!(map_row(150.0, 1000, 300.0), 499);
        assert_eq!(map_row(0.0, 0, 0.0), 0);
    }
    #[test]
    fn thumbnail_ink_is_content_derived_and_bounded() {
        assert_eq!(ink_spans("\t世界:  x\n", 4), vec![(4, 7), (9, 10)]);
        assert!(ink_spans(" \t \r\n", 2).is_empty());
        assert_eq!(ink_spans(&"a".repeat(1000), 2), vec![(0, 96)]);
        assert_eq!(ink_spans("\tx", 0), vec![(1, 2)]);
    }
}
