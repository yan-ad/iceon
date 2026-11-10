//! Native, presentation-only Kubernetes exec terminal. No local PTY or process is created.
//!
//! The default vt100 callback implementation is deliberately inert: OSC titles, hyperlinks,
//! clipboard requests (OSC 52), bells and terminal queries cannot cause host side effects.
//! Only explicit keyboard/paste gestures emit input; received output is never locally echoed.

use std::num::NonZeroU16;

use gpui_kit::component::input::{Copy, Paste};
use gpui_kit::prelude::*;
use gpui_kit::*;

const SCROLLBACK_LINES: usize = 2500;
const LINE_HEIGHT: f32 = 18.0;
const KEY_CONTEXT: &str = "IceonTerminal";

gpui_kit::actions!(iceon_terminal, [CopySnapshot, PasteClipboard]);

/// Register after gpui_kit::init. Bindings only apply to a focused terminal.
pub(crate) fn init(cx: &mut App) {
    // GPUI resolves keybindings before low-level key events. Mask shell-level
    // ctrl-letter shortcuts in this scope so e.g. ctrl-B/K/Q reach the process
    // rather than toggling the sidebar, opening a palette, or quitting on Linux.
    const CONTROL_KEYS: [&str; 26] = [
        "ctrl-a", "ctrl-b", "ctrl-c", "ctrl-d", "ctrl-e", "ctrl-f", "ctrl-g", "ctrl-h", "ctrl-i",
        "ctrl-j", "ctrl-k", "ctrl-l", "ctrl-m", "ctrl-n", "ctrl-o", "ctrl-p", "ctrl-q", "ctrl-r",
        "ctrl-s", "ctrl-t", "ctrl-u", "ctrl-v", "ctrl-w", "ctrl-x", "ctrl-y", "ctrl-z",
    ];
    cx.bind_keys(CONTROL_KEYS.map(|key| KeyBinding::new(key, NoAction, Some(KEY_CONTEXT))));
    cx.bind_keys([
        KeyBinding::new("cmd-c", CopySnapshot, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-v", PasteClipboard, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-c", CopySnapshot, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-v", PasteClipboard, Some(KEY_CONTEXT)),
    ]);
}

#[derive(Default)]
struct DeviceReplies {
    enabled: bool,
    pending: String,
}

impl vt100::Callbacks for DeviceReplies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        prefix: Option<u8>,
        intermediate: Option<u8>,
        params: &[&[u16]],
        command: char,
    ) {
        if !self.enabled || intermediate.is_some() || self.pending.len() > 4096 {
            return;
        }
        let parameter = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (prefix, parameter, command) {
            (None, 0, 'c') => self.pending.push_str("\x1b[?1;2c"),
            (Some(b'>'), 0, 'c') => self.pending.push_str("\x1b[>0;0;0c"),
            (None, 5, 'n') => self.pending.push_str("\x1b[0n"),
            (None, 6, 'n') => {
                let (row, col) = screen.cursor_position();
                self.pending
                    .push_str(&format!("\x1b[{};{}R", row + 1, col + 1));
            }
            _ => {}
        }
    }
}

fn surface_parser(cols: u16, rows: u16, enabled: bool) -> vt100::Parser<DeviceReplies> {
    let (cols, rows) = dimensions(cols, rows);
    vt100::Parser::new_with_callbacks(
        rows,
        cols,
        SCROLLBACK_LINES,
        DeviceReplies {
            enabled,
            pending: String::new(),
        },
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalEvent {
    Input(String),
    Resize { cols: u16, rows: u16 },
}

pub(crate) struct TerminalSurface {
    parser: vt100::Parser<DeviceReplies>,
    focus_handle: FocusHandle,
    bounds: Bounds<Pixels>,
    selection: Option<((u16, u16), (u16, u16))>,
    selecting: bool,
    cell_width: Pixels,
    line_height: f32,
}

impl EventEmitter<TerminalEvent> for TerminalSurface {}

impl Focusable for TerminalSurface {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn dimensions(cols: u16, rows: u16) -> (NonZeroU16, NonZeroU16) {
    (
        NonZeroU16::new(cols.clamp(1, 300)).unwrap(),
        NonZeroU16::new(rows.clamp(1, 120)).unwrap(),
    )
}

#[cfg(test)]
fn parser(cols: u16, rows: u16) -> vt100::Parser {
    let (cols, rows) = dimensions(cols, rows);
    vt100::Parser::new(rows, cols, SCROLLBACK_LINES)
}

impl TerminalSurface {
    pub(crate) fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            parser: surface_parser(80, 24, false),
            focus_handle: cx.focus_handle(),
            bounds: Bounds::default(),
            selection: None,
            selecting: false,
            cell_width: px(8.),
            line_height: LINE_HEIGHT,
        }
    }

    /// Process the exact stream chunk, retaining partial escape/UTF-8 parser state.
    pub(crate) fn feed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.feed_bytes(text.as_bytes(), cx);
    }

    /// Prefer this entry point if transport chunks may split a UTF-8 code point.
    pub(crate) fn feed_bytes(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        self.parser.process(bytes);
        let reply = std::mem::take(&mut self.parser.callbacks_mut().pending);
        if !reply.is_empty() {
            cx.emit(TerminalEvent::Input(reply));
        }
        cx.notify();
    }

    pub(crate) fn enable_device_replies(&mut self) {
        self.parser.callbacks_mut().enabled = true;
    }

    pub(crate) fn reset(&mut self, cx: &mut Context<Self>) {
        let (cols, rows) = self.size();
        let enabled = self.parser.callbacks().enabled;
        self.parser = surface_parser(cols, rows, enabled);
        self.selection = None;
        cx.notify();
    }

    /// Returns (columns, rows), matching the backend exec resize message.
    pub(crate) fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (cols.get(), rows.get())
    }

    /// Clamp and notify the remote consumer only when the cell grid actually changes.
    pub(crate) fn resize(&mut self, cols: u16, rows: u16, cx: &mut Context<Self>) {
        let (cols, rows) = dimensions(cols, rows);
        if self.size() == (cols.get(), rows.get()) {
            return;
        }
        self.parser.set_size(rows, cols);
        cx.emit(TerminalEvent::Resize {
            cols: cols.get(),
            rows: rows.get(),
        });
        cx.notify();
    }

    /// Plain text of the currently visible viewport, with no ANSI/OSC sequences.
    pub(crate) fn contents(&self) -> String {
        self.parser.screen().contents()
    }

    fn cell_at(&self, position: Point<Pixels>) -> (u16, u16) {
        let (cols, rows) = self.size();
        let col = (f32::from(position.x - self.bounds.origin.x) / f32::from(self.cell_width))
            .floor()
            .max(0.) as u16;
        let row = (f32::from(position.y - self.bounds.origin.y) / self.line_height)
            .floor()
            .max(0.) as u16;
        (row.min(rows - 1), col.min(cols))
    }

    fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection?;
        if a == b {
            return None;
        }
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        Some(self.parser.screen().contents_between(a.0, a.1, b.0, b.1))
    }

    fn finish_selection(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        let end = self.cell_at(position);
        if let Some((_, selected_end)) = &mut self.selection {
            *selected_end = end;
        }
        let copy_on_select = cx
            .try_global::<crate::preferences_view::GlobalPreferencesState>()
            .is_some_and(|state| state.preferences.terminal.copy_on_select);
        if copy_on_select {
            if let Some(text) = self.selected_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
        cx.notify();
    }

    /// Explicit copy uses the selection, falling back to the visible snapshot.
    pub(crate) fn copy_snapshot(&self, cx: &mut App) {
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.selected_text().unwrap_or_else(|| self.contents()),
        ));
    }

    pub(crate) fn paste(&mut self, cx: &mut Context<Self>) {
        // Clipboard access is only reachable from explicit Paste actions/gestures.
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if !text.is_empty() {
                let input = encode_paste(&text, self.parser.screen().bracketed_paste());
                self.send_input(input, cx);
            }
        }
    }

    fn send_input(&mut self, input: String, cx: &mut Context<Self>) {
        self.selection = None;
        self.parser.screen_mut().set_scrollback(0);
        cx.emit(TerminalEvent::Input(input));
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        // Let the scoped action bindings handle clipboard and preserve host commands
        // (notably cmd-Q). Do not turn ctrl-shift-V into a remote control-V.
        if key.modifiers.platform
            || (key.modifiers.control
                && key.modifiers.shift
                && matches!(key.key.to_ascii_lowercase().as_str(), "c" | "v"))
        {
            return;
        }
        if let Some(input) = encode_key(key, self.parser.screen().application_cursor()) {
            self.send_input(input, cx);
            window.prevent_default();
            cx.stop_propagation();
        }
    }
}

/// Encode terminal input without mutating the terminal display. Modifiers are xterm-style.
fn encode_key(key: &Keystroke, application_cursor: bool) -> Option<String> {
    let m = key.modifiers;
    if m.platform || m.function {
        return None;
    }
    let name = key.key.to_ascii_lowercase();
    let arrow = match name.as_str() {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        _ => None,
    };
    if let Some(final_char) = arrow {
        let modifier = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.control);
        return Some(if modifier != 1 {
            format!("\x1b[1;{modifier}{final_char}")
        } else if application_cursor {
            format!("\x1bO{final_char}")
        } else {
            format!("\x1b[{final_char}")
        });
    }
    let input = match name.as_str() {
        "enter" | "return" => "\r".into(),
        "backspace" => "\x7f".into(),
        "tab" if m.shift => "\x1b[Z".into(),
        "tab" => "\t".into(),
        "escape" => "\x1b".into(),
        "insert" => "\x1b[2~".into(),
        "delete" => "\x1b[3~".into(),
        "pageup" => "\x1b[5~".into(),
        "pagedown" => "\x1b[6~".into(),
        _ if m.control => {
            let code = match name.as_str() {
                "space" | "@" | "2" => 0,
                "[" => 27,
                "\\" => 28,
                "]" => 29,
                "^" | "6" => 30,
                "_" | "-" => 31,
                "?" => 127,
                _ if name.len() == 1 && name.as_bytes()[0].is_ascii_lowercase() => {
                    name.as_bytes()[0] - b'a' + 1
                }
                _ => return None,
            };
            char::from(code).to_string()
        }
        "space" => " ".into(),
        _ if m.alt && key.key.chars().count() == 1 => key.key.clone(),
        _ => key
            .key_char
            .clone()
            .or_else(|| (key.key.chars().count() == 1).then(|| key.key.clone()))?,
    };
    Some(if m.alt { format!("\x1b{input}") } else { input })
}

fn encode_paste(text: &str, bracketed: bool) -> String {
    // Terminal Enter is CR; normalize CRLF before lone LF to avoid doubled Enter.
    // Strip controls (especially ESC) so pasted text cannot terminate bracketed paste.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    let text = text.replace('\n', "\r");
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~")
    } else {
        text
    }
}

fn ansi_color(color: vt100::Color, foreground: bool, bold: bool) -> u32 {
    const PALETTE: [u32; 16] = [
        0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666,
        0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
    ];
    match color {
        vt100::Color::Default => {
            if foreground {
                0xd4d4d4
            } else {
                0x000000
            }
        }
        vt100::Color::Rgb(r, g, b) => (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
        vt100::Color::Idx(index) => match index {
            0..=15 => {
                PALETTE[usize::from(if foreground && bold && index < 8 {
                    index + 8
                } else {
                    index
                })]
            }
            16..=231 => {
                let n = index - 16;
                let component = |v: u8| if v == 0 { 0 } else { 55 + u32::from(v) * 40 };
                (component(n / 36) << 16) | (component(n / 6 % 6) << 8) | component(n % 6)
            }
            _ => {
                let gray = 8 + u32::from(index - 232) * 10;
                gray * 0x010101
            }
        },
    }
}

fn push_text_run(runs: &mut Vec<TextRun>, run: TextRun) {
    if let Some(last) = runs.last_mut() {
        if last.font == run.font
            && last.color == run.color
            && last.background_color == run.background_color
            && last.underline == run.underline
            && last.strikethrough == run.strikethrough
        {
            last.len += run.len;
            return;
        }
    }
    runs.push(run);
}

impl Render for TerminalSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let preferences = cx
            .try_global::<crate::preferences_view::GlobalPreferencesState>()
            .map(|state| state.preferences.terminal.clone())
            .unwrap_or_default();
        let font_family = crate::preference_runtime::font_family(&preferences.font_family, cx);
        let console_font = if font_family.as_ref() == crate::console_font::FAMILY {
            crate::console_font::font(cx)
        } else {
            gpui_kit::font(font_family.clone())
        };
        let font_size = f32::from(preferences.font_size);
        let line_height = font_size * 1.5;
        let dark = gpui_kit::component::theme::Theme::global(cx).mode.is_dark();
        let (default_bg, default_fg) =
            crate::preference_runtime::terminal_colors(preferences.theme, dark);
        let (cols, rows) = self.size();
        let cell_width = window
            .text_system()
            .ch_advance(
                window.text_system().resolve_font(&console_font),
                px(font_size),
            )
            .unwrap_or(px(8.0));
        self.cell_width = cell_width;
        self.line_height = line_height;
        let screen = self.parser.screen();
        let cursor = screen.cursor_position();
        let show_cursor = !screen.hide_cursor() && screen.scrollback() == 0;
        let mut lines = Vec::with_capacity(usize::from(rows));
        for row in 0..rows {
            let mut text = String::new();
            let mut runs = Vec::new();
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let content = if cell.contents().is_empty() {
                    " "
                } else {
                    cell.contents()
                };
                text.push_str(content);
                let mut foreground = ansi_color(cell.fgcolor(), true, cell.bold());
                let mut background = ansi_color(cell.bgcolor(), false, false);
                if cell.fgcolor() == vt100::Color::Default {
                    foreground = default_fg;
                }
                if cell.bgcolor() == vt100::Color::Default {
                    background = default_bg;
                }
                if cell.inverse() {
                    std::mem::swap(&mut foreground, &mut background);
                }
                if cell.dim() {
                    foreground = (foreground & 0xfefefe) >> 1;
                }
                if show_cursor
                    && cursor.0 == row
                    && (cursor.1 == col || (cell.is_wide() && cursor.1 == col + 1))
                {
                    foreground = 0x000000;
                    background = 0xd4d4d4;
                }
                if let Some((a, b)) = self.selection {
                    let (a, b) = if a < b { (a, b) } else { (b, a) };
                    if (row, col) >= a && (row, col) < b {
                        background = 0x365f8c;
                        foreground = 0xffffff;
                    }
                }
                let mut face = console_font.clone();
                if cell.bold() {
                    face = face.bold();
                }
                if cell.italic() {
                    face = face.italic();
                }
                push_text_run(
                    &mut runs,
                    TextRun {
                        len: content.len(),
                        font: face,
                        color: rgb(foreground).into(),
                        background_color: Some(rgb(background).into()),
                        underline: cell.underline().then_some(UnderlineStyle {
                            thickness: px(1.0),
                            color: Some(rgb(foreground).into()),
                            wavy: false,
                        }),
                        strikethrough: None,
                    },
                );
            }
            lines.push(
                div()
                    .h(px(line_height))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(StyledText::new(text).with_runs(runs)),
            );
        }
        let weak = cx.entity().downgrade();
        let bounds_owner = weak.clone();
        div()
            .id("terminal-surface")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .bg(rgb(default_bg))
            .text_color(rgb(default_fg))
            .font_family(font_family)
            .text_size(px(font_size))
            .line_height(px(line_height))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    let start = this.cell_at(event.position);
                    this.selection = Some((start, start));
                    this.selecting = true;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.selecting && event.pressed_button == Some(MouseButton::Left) {
                    let end = this.cell_at(event.position);
                    if let Some((_, selected_end)) = &mut this.selection {
                        *selected_end = end;
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.finish_selection(event.position, cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.finish_selection(event.position, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.paste(cx);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|this, _: &CopySnapshot, _, cx| this.copy_snapshot(cx)))
            .on_action(cx.listener(|this, _: &PasteClipboard, _, cx| this.paste(cx)))
            // Also handle native Edit-menu actions (the component input action types).
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy_snapshot(cx)))
            .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                if this.parser.screen().alternate_screen() {
                    return;
                }
                let delta =
                    f32::from(event.delta.pixel_delta(px(this.line_height)).y) / this.line_height;
                if delta.abs() < 0.1 {
                    return;
                }
                let current = this.parser.screen().scrollback() as isize;
                let offset = (current + delta.round() as isize).max(0) as usize;
                this.selection = None;
                this.parser.screen_mut().set_scrollback(offset);
                cx.notify();
                cx.stop_propagation();
            }))
            .child(div().flex().flex_col().children(lines))
            .child(
                canvas(
                    move |bounds, window, _| {
                        // Ignore hidden/zero-sized views. Layout measurement never mutates an
                        // entity during paint; defer and check again to avoid a render loop.
                        if bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0) {
                            return;
                        }
                        // Bounds are hit-test metadata only. Defer outside paint.
                        window.on_next_frame(move |_, cx| {
                            let _ = bounds_owner.update(cx, |this, _| this.bounds = bounds);
                        });
                        let measured_cols =
                            (f32::from(bounds.size.width) / f32::from(cell_width)).floor() as u16;
                        let measured_rows =
                            (f32::from(bounds.size.height) / line_height).floor() as u16;
                        let (new_cols, new_rows) = dimensions(measured_cols, measured_rows);
                        if (new_cols.get(), new_rows.get()) != (cols, rows) {
                            window.on_next_frame(move |_, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.resize(new_cols.get(), new_rows.get(), cx)
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{ansi_color, dimensions, encode_key, encode_paste, parser};
    use gpui_kit::{Keystroke, Modifiers};

    #[::core::prelude::v1::test]
    fn adjacent_cells_with_same_style_share_one_text_run() {
        let mut runs = Vec::new();
        let run = gpui_kit::TextRun {
            len: 1,
            font: gpui_kit::font("JetBrains Mono"),
            color: gpui_kit::rgb(0xffffff).into(),
            ..Default::default()
        };
        for _ in 0..300 {
            super::push_text_run(&mut runs, run.clone());
        }
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len, 300);
        super::push_text_run(
            &mut runs,
            gpui_kit::TextRun {
                color: gpui_kit::rgb(0xff0000).into(),
                ..run
            },
        );
        assert_eq!(runs.len(), 2);
    }

    #[::core::prelude::v1::test]
    fn device_query_replies_are_bounded_and_pod_callbacks_stay_inert() {
        let mut local = super::surface_parser(80, 24, true);
        local.process(b"\x1b[");
        local.process(b"c\x1b[5n\x1b[6n");
        assert_eq!(local.callbacks().pending, "\x1b[?1;2c\x1b[0n\x1b[1;1R");
        local.callbacks_mut().pending.clear();
        local.process(b"\x1b]52;c;secret\x07\x1b]52;c;?\x07");
        assert!(local.callbacks().pending.is_empty());
        for _ in 0..1000 {
            local.process(b"\x1b[c");
        }
        assert!(local.callbacks().pending.len() < 4120);
        let mut pod = super::surface_parser(80, 24, false);
        pod.process(b"\x1b[c\x1b[5n\x1b[6n");
        assert!(pod.callbacks().pending.is_empty());
    }

    fn key(name: &str, control: bool, alt: bool, shift: bool) -> Keystroke {
        Keystroke {
            key: name.into(),
            key_char: None,
            modifiers: Modifiers {
                control,
                alt,
                shift,
                ..Default::default()
            },
        }
    }

    #[::core::prelude::v1::test]
    fn carriage_return_cursor_and_clear_are_not_appended_lines() {
        let mut p = parser(80, 24);
        p.process(b"progress 10%\rprogress 20%\x1b[K");
        assert_eq!(p.screen().contents(), "progress 20%");
        p.process(b"\x1b[2J\x1b[Hready");
        assert_eq!(p.screen().contents(), "ready");
        assert_eq!(p.screen().cursor_position(), (0, 5));
    }

    #[::core::prelude::v1::test]
    fn split_escapes_and_utf8_preserve_parser_state() {
        let mut p = parser(80, 24);
        for part in [b"\x1b[3".as_slice(), b"1m", "hé界".as_bytes()] {
            for byte in part {
                p.process(&[*byte]);
            }
        }
        assert_eq!(p.screen().contents(), "hé界");
        assert_eq!(
            p.screen().cell(0, 0).unwrap().fgcolor(),
            vt100::Color::Idx(1)
        );
        assert_eq!(p.screen().cursor_position(), (0, 4));
    }

    #[::core::prelude::v1::test]
    fn bold_truecolor_indexed_colors_and_alternate_screen() {
        let mut p = parser(80, 24);
        p.process(b"\x1b[1;38;2;20;30;40;48;5;200mA");
        let cell = p.screen().cell(0, 0).unwrap();
        assert!(cell.bold());
        assert_eq!(cell.fgcolor(), vt100::Color::Rgb(20, 30, 40));
        assert_eq!(cell.bgcolor(), vt100::Color::Idx(200));
        p.process(b"\x1b[?1049h\x1b[Halternate");
        assert!(p.screen().alternate_screen());
        assert!(p.screen().contents().contains("alternate"));
        p.process(b"\x1b[?1049l");
        assert!(!p.screen().alternate_screen());
        assert_eq!(p.screen().contents(), "A");
    }

    #[::core::prelude::v1::test]
    fn raw_keys_control_letters_and_application_arrows() {
        assert_eq!(
            encode_key(&key("enter", false, false, false), false).as_deref(),
            Some("\r")
        );
        assert_eq!(
            encode_key(&key("backspace", false, false, false), false).as_deref(),
            Some("\x7f")
        );
        assert_eq!(
            encode_key(&key("tab", false, false, false), false).as_deref(),
            Some("\t")
        );
        for letter in b'a'..=b'z' {
            assert_eq!(
                encode_key(
                    &key(&(letter as char).to_string(), true, false, false),
                    false
                ),
                Some(char::from(letter - b'a' + 1).to_string())
            );
        }
        for (name, tail) in [("up", "A"), ("down", "B"), ("right", "C"), ("left", "D")] {
            assert_eq!(
                encode_key(&key(name, false, false, false), false),
                Some(format!("\x1b[{tail}"))
            );
            assert_eq!(
                encode_key(&key(name, false, false, false), true),
                Some(format!("\x1bO{tail}"))
            );
        }
        assert_eq!(
            encode_key(&key("up", true, false, false), false).as_deref(),
            Some("\x1b[1;5A")
        );
        assert_eq!(
            encode_key(&key("x", false, true, false), false).as_deref(),
            Some("\x1bx")
        );
        let mut command = key("q", false, false, false);
        command.modifiers.platform = true;
        assert_eq!(encode_key(&command, false), None);
    }

    #[::core::prelude::v1::test]
    fn modes_drive_bracketed_paste_and_arrows() {
        let mut p = parser(80, 24);
        p.process(b"\x1b[?1h\x1b[?2004h");
        assert_eq!(
            encode_key(
                &key("up", false, false, false),
                p.screen().application_cursor()
            )
            .as_deref(),
            Some("\x1bOA")
        );
        assert_eq!(
            encode_paste("hé\r\nworld\n", p.screen().bracketed_paste()),
            "\x1b[200~hé\rworld\r\x1b[201~"
        );
        assert_eq!(
            encode_paste("a\x1b[201~\x03b", true),
            "\x1b[200~a[201~b\x1b[201~"
        );
        assert_eq!(encode_paste("one\ntwo", false), "one\rtwo");
    }

    #[::core::prelude::v1::test]
    fn bounded_scrollback_resize_and_inert_osc() {
        let mut p = parser(80, 2);
        for _ in 0..3000 {
            p.process(b"line\r\n");
        }
        assert_eq!(p.screen().scrollback_len(), 2500);
        p.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(p.screen().scrollback(), 2500);
        let (cols, rows) = dimensions(0, u16::MAX);
        assert_eq!((cols.get(), rows.get()), (1, 120));
        p.set_size(rows, cols);
        assert_eq!(p.screen().size(), (rows, cols));
        p.process(b"\x1b]52;c;c2VjcmV0\x07\x1b]8;;https://example.test\x1b\\link\x1b]8;;\x1b\\");
        p.screen_mut().set_scrollback(0);
        assert!(!p.screen().contents().contains("c2VjcmV0"));
        assert_eq!(ansi_color(vt100::Color::Idx(196), true, false), 0xff0000);
        assert_eq!(ansi_color(vt100::Color::Idx(255), true, false), 0xeeeeee);
    }
}
