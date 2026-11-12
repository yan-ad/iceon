//! Native, presentation-only Kubernetes exec terminal. No local PTY or process is created.
//!
//! The default vt100 callback implementation is deliberately inert: OSC titles, hyperlinks,
//! clipboard requests (OSC 52), bells and terminal queries cannot cause host side effects.
//! Only explicit keyboard/paste gestures emit input; received output is never locally echoed.

use std::num::NonZeroU16;

use gpui_kit::component::input::{Copy, Paste, SelectAll};
use gpui_kit::prelude::*;
use gpui_kit::*;

const SCROLLBACK_LINES: usize = 2500;
const LINE_HEIGHT: f32 = 18.0;
const PADDING_X: f32 = 12.0;
const PADDING_Y: f32 = 2.0;
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
    cx.bind_keys(
        CONTROL_KEYS
            .into_iter()
            .filter(|key| cfg!(target_os = "macos") || *key != "ctrl-w")
            .map(|key| KeyBinding::new(key, NoAction, Some(KEY_CONTEXT))),
    );
    cx.bind_keys([KeyBinding::new(
        if cfg!(target_os = "macos") {
            "cmd-w"
        } else {
            "ctrl-w"
        },
        crate::native_menu::CloseWindow,
        Some(KEY_CONTEXT),
    )]);
    cx.bind_keys([
        KeyBinding::new("cmd-c", CopySnapshot, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-v", PasteClipboard, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-c", CopySnapshot, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-v", PasteClipboard, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-a", SelectAll, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-a", SelectAll, Some(KEY_CONTEXT)),
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
    metrics: Option<(Font, f32)>,
    grid_cache: GridCache,
    #[cfg(debug_assertions)]
    render_count: usize,
    #[cfg(debug_assertions)]
    grid_build_count: usize,
}

#[derive(Clone, PartialEq)]
struct GridStyle {
    font: Font,
    font_size: f32,
    palette: crate::preference_runtime::TerminalPalette,
}

#[derive(Default)]
struct GridCache {
    dirty: bool,
    style: Option<GridStyle>,
    rows: Vec<(SharedString, Vec<TextRun>)>,
}

impl GridCache {
    fn needs_rebuild(&self, style: &GridStyle) -> bool {
        self.dirty || self.style.as_ref() != Some(style)
    }

    fn process_output(&mut self, parser: &mut vt100::Parser<DeviceReplies>, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }
        parser.process(bytes);
        self.dirty = true;
        true
    }

    fn resize(&mut self, parser: &mut vt100::Parser<DeviceReplies>, cols: u16, rows: u16) -> bool {
        let (cols, rows) = dimensions(cols, rows);
        if parser.screen().size() == (rows, cols) {
            return false;
        }
        parser.set_size(rows, cols);
        self.dirty = true;
        true
    }
}

#[cfg(test)]
fn inner_bounds(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let origin = point(
        bounds.origin.x + px(PADDING_X),
        bounds.origin.y + px(PADDING_Y),
    );
    Bounds::from_corners(
        origin,
        point(
            (bounds.origin.x + bounds.size.width - px(PADDING_X)).max(origin.x),
            (bounds.origin.y + bounds.size.height - px(PADDING_Y)).max(origin.y),
        ),
    )
}

fn grid_dimensions(bounds: Bounds<Pixels>, cell_width: Pixels, line_height: f32) -> (u16, u16) {
    let cols = (f32::from(bounds.size.width) / f32::from(cell_width).max(1.)).floor() as u16;
    let rows = (f32::from(bounds.size.height) / line_height.max(1.)).floor() as u16;
    let (cols, rows) = dimensions(cols, rows);
    (cols.get(), rows.get())
}

fn cell_at_position(
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: f32,
    cols: u16,
    rows: u16,
) -> (u16, u16) {
    let col = (f32::from(position.x - bounds.origin.x) / f32::from(cell_width).max(1.))
        .floor()
        .max(0.) as u16;
    let row = (f32::from(position.y - bounds.origin.y) / line_height.max(1.))
        .floor()
        .max(0.) as u16;
    (row.min(rows.saturating_sub(1)), col.min(cols))
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
    #[cfg(debug_assertions)]
    pub(crate) fn performance_snapshot(&self) -> (usize, usize) {
        (self.render_count, self.grid_build_count)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn selection_snapshot(&self) -> Option<String> {
        self.selected_text()
    }

    pub(crate) fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_size(80, 24, cx)
    }

    /// Construct with the size used by the owning transport.  Keeping the
    /// parser at the transport's initial size avoids the first exec snapshot
    /// being laid out in the fallback 80x24 grid before the first paint.
    pub(crate) fn with_size(cols: u16, rows: u16, cx: &mut Context<Self>) -> Self {
        Self {
            parser: surface_parser(cols, rows, false),
            focus_handle: cx.focus_handle(),
            bounds: Bounds::default(),
            selection: None,
            selecting: false,
            cell_width: px(8.),
            line_height: LINE_HEIGHT,
            metrics: None,
            grid_cache: GridCache::default(),
            #[cfg(debug_assertions)]
            render_count: 0,
            #[cfg(debug_assertions)]
            grid_build_count: 0,
        }
    }

    /// Process the exact stream chunk, retaining partial escape/UTF-8 parser state.
    pub(crate) fn feed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.feed_bytes(text.as_bytes(), cx);
    }

    /// Prefer this entry point if transport chunks may split a UTF-8 code point.
    pub(crate) fn feed_bytes(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if !self.grid_cache.process_output(&mut self.parser, bytes) {
            return;
        }
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
        self.grid_cache.dirty = true;
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
        if !self
            .grid_cache
            .resize(&mut self.parser, cols.get(), rows.get())
        {
            return;
        }
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
        cell_at_position(
            position,
            self.bounds,
            self.cell_width,
            self.line_height,
            cols,
            rows,
        )
    }

    fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection?;
        if a == b {
            return None;
        }
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        Some(self.parser.screen().contents_between(a.0, a.1, b.0, b.1))
    }

    fn select_clicked_unit(&mut self, row: u16, col: u16, clicks: usize) {
        let (cols, _) = self.size();
        let col = col.min(cols.saturating_sub(1));
        let screen = self.parser.screen();
        let (start, end) = if clicks >= 3 {
            (0, cols)
        } else {
            let cells: Vec<_> = (0..cols)
                .map(|index| {
                    screen
                        .cell(row, index)
                        .map(|cell| {
                            if cell.is_wide_continuation() {
                                "\u{fffd}".to_owned()
                            } else {
                                cell.contents().to_owned()
                            }
                        })
                        .unwrap_or_default()
                })
                .collect();
            word_columns(&cells, col)
        };
        self.selection = Some(((row, start), (row, end)));
        self.selecting = false;
        self.grid_cache.dirty = true;
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let (cols, rows) = self.size();
        self.selection = Some(((0, 0), (rows.saturating_sub(1), cols)));
        self.selecting = false;
        self.grid_cache.dirty = true;
        cx.notify();
    }

    fn finish_selection(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        self.grid_cache.dirty = true;
        let end = self.cell_at(position);
        if let Some((_, selected_end)) = &mut self.selection {
            *selected_end = end;
        }
        let copy_on_select = cx
            .try_global::<crate::preferences_view::GlobalPreferencesState>()
            .is_some_and(|state| state.preferences.terminal.copy_on_select);
        if copy_on_select && let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
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
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
            && !text.is_empty()
        {
            let input = encode_paste(&text, self.parser.screen().bracketed_paste());
            self.send_input(input, cx);
        }
    }

    fn send_input(&mut self, input: String, cx: &mut Context<Self>) {
        self.selection = None;
        self.grid_cache.dirty = true;
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

fn ansi_color(
    color: vt100::Color,
    foreground: bool,
    bold: bool,
    palette: &crate::preference_runtime::TerminalPalette,
) -> Hsla {
    match color {
        vt100::Color::Default => {
            if foreground {
                palette.foreground
            } else {
                palette.background
            }
        }
        vt100::Color::Rgb(r, g, b) => {
            rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)).into()
        }
        vt100::Color::Idx(index) => match index {
            0..=15 => {
                palette.ansi[usize::from(if foreground && bold && index < 8 {
                    index + 8
                } else {
                    index
                })]
            }
            16..=231 => {
                let n = index - 16;
                let component = |v: u8| if v == 0 { 0 } else { 55 + u32::from(v) * 40 };
                rgb((component(n / 36) << 16) | (component(n / 6 % 6) << 8) | component(n % 6))
                    .into()
            }
            _ => {
                let gray = 8 + u32::from(index - 232) * 10;
                rgb(gray * 0x010101).into()
            }
        },
    }
}

fn push_text_run(runs: &mut Vec<TextRun>, run: TextRun) {
    if let Some(last) = runs.last_mut()
        && last.font == run.font
        && last.color == run.color
        && last.background_color == run.background_color
        && last.underline == run.underline
        && last.strikethrough == run.strikethrough
    {
        last.len += run.len;
        return;
    }
    runs.push(run);
}

fn word_columns(cells: &[String], column: u16) -> (u16, u16) {
    if cells.is_empty() {
        return (0, 0);
    }
    let column = usize::from(column).min(cells.len() - 1);
    let kind = |cell: &str| {
        if cell.is_empty() || cell.chars().all(char::is_whitespace) {
            0
        } else if cell
            .chars()
            .any(|ch| ch.is_alphanumeric() || ch == '_' || ch == '\u{fffd}')
        {
            1
        } else {
            2
        }
    };
    let selected = kind(&cells[column]);
    let mut start = column;
    let mut end = column + 1;
    while start > 0 && kind(&cells[start - 1]) == selected {
        start -= 1;
    }
    while end < cells.len() && kind(&cells[end]) == selected {
        end += 1;
    }
    (start as u16, end as u16)
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
        let palette = crate::preference_runtime::terminal_palette(preferences.theme, cx);
        let (default_bg, default_fg) = (palette.background, palette.foreground);
        let (cols, rows) = self.size();
        #[cfg(debug_assertions)]
        {
            self.render_count += 1;
        }
        if self.metrics.as_ref() != Some(&(console_font.clone(), font_size)) {
            self.cell_width = window
                .text_system()
                .ch_advance(
                    window.text_system().resolve_font(&console_font),
                    px(font_size),
                )
                .unwrap_or(px(8.0));
            self.line_height = line_height;
            self.metrics = Some((console_font.clone(), font_size));
        }
        let cell_width = self.cell_width;
        let style = GridStyle {
            font: console_font.clone(),
            font_size,
            palette: palette.clone(),
        };
        if self.grid_cache.needs_rebuild(&style) {
            #[cfg(debug_assertions)]
            {
                self.grid_build_count += 1;
            }
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
                    let mut foreground = ansi_color(cell.fgcolor(), true, cell.bold(), &palette);
                    let mut background = ansi_color(cell.bgcolor(), false, false, &palette);
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
                        foreground = background.blend(foreground.alpha(0.5));
                    }
                    if show_cursor
                        && cursor.0 == row
                        && (cursor.1 == col || (cell.is_wide() && cursor.1 == col + 1))
                    {
                        foreground = palette.cursor_text;
                        background = palette.cursor;
                    }
                    if let Some((a, b)) = self.selection {
                        let (a, b) = if a < b { (a, b) } else { (b, a) };
                        if (row, col) >= a && (row, col) < b {
                            background = background.blend(palette.selection);
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
                            color: foreground,
                            background_color: Some(background),
                            underline: cell.underline().then_some(UnderlineStyle {
                                thickness: px(1.0),
                                color: Some(foreground),
                                wavy: false,
                            }),
                            strikethrough: None,
                        },
                    );
                }
                lines.push((SharedString::from(text), runs));
            }
            self.grid_cache.rows = lines;
            self.grid_cache.style = Some(style);
            self.grid_cache.dirty = false;
        }
        let lines = self.grid_cache.rows.iter().map(|(text, runs)| {
            div()
                .h(px(line_height))
                .flex_shrink_0()
                .overflow_hidden()
                .child(StyledText::new(text.clone()).with_runs(runs.clone()))
        });
        let weak = cx.entity().downgrade();
        let previous_bounds = self.bounds;
        div()
            .id("terminal-surface")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .bg(default_bg)
            .text_color(default_fg)
            .font_family(font_family)
            .text_size(px(font_size))
            .line_height(px(line_height))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    let start = this.cell_at(event.position);
                    if event.click_count >= 2 {
                        this.select_clicked_unit(start.0, start.1, event.click_count);
                    } else {
                        this.selection = Some((start, start));
                        this.selecting = true;
                    }
                    cx.stop_propagation();
                    this.grid_cache.dirty = true;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                // On native windows GPUI may report the drag without preserving
                // `pressed_button` on every move event. `dragging()` is the
                // stable signal and keeps selection working across dock panes.
                if this.selecting && event.dragging() {
                    let end = this.cell_at(event.position);
                    if let Some((_, selected_end)) = &mut this.selection {
                        if *selected_end == end {
                            return;
                        }
                        *selected_end = end;
                    }
                    this.grid_cache.dirty = true;
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
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
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
                this.grid_cache.dirty = true;
                cx.notify();
                cx.stop_propagation();
            }))
            .child(
                div()
                    .absolute()
                    .left(px(PADDING_X))
                    .top(px(PADDING_Y))
                    .right(px(PADDING_X))
                    .bottom(px(PADDING_Y))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .children(lines),
            )
            .child(
                canvas(
                    move |bounds, window, _| {
                        // Canvas is already inset: do not subtract padding twice.
                        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
                            return;
                        }
                        let measured = grid_dimensions(bounds, cell_width, line_height);
                        if bounds != previous_bounds || measured != (cols, rows) {
                            window.on_next_frame(move |_, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.bounds = bounds;
                                    // Preferences may have changed since layout.
                                    let (cols, rows) =
                                        grid_dimensions(bounds, this.cell_width, this.line_height);
                                    this.resize(cols, rows, cx);
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .left(px(PADDING_X))
                .top(px(PADDING_Y))
                .right(px(PADDING_X))
                .bottom(px(PADDING_Y)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{ansi_color, dimensions, encode_key, encode_paste, parser};
    use gpui_kit::{Keystroke, Modifiers};

    #[::core::prelude::v1::test]
    fn imported_ansi_preserves_truecolor_and_bold_background_semantics() {
        let mut palette = crate::preference_runtime::TerminalPalette::default_for(true);
        palette.ansi[1] = gpui_kit::rgb(0x112233).into();
        palette.ansi[9] = gpui_kit::rgb(0x445566).into();
        assert_eq!(
            ansi_color(vt100::Color::Idx(1), true, false, &palette),
            palette.ansi[1]
        );
        assert_eq!(
            ansi_color(vt100::Color::Idx(1), true, true, &palette),
            palette.ansi[9]
        );
        assert_eq!(
            ansi_color(vt100::Color::Idx(1), false, true, &palette),
            palette.ansi[1]
        );
        assert_eq!(
            ansi_color(vt100::Color::Default, true, false, &palette),
            palette.foreground
        );
        assert_eq!(
            ansi_color(vt100::Color::Rgb(1, 2, 3), true, true, &palette),
            gpui_kit::Hsla::from(gpui_kit::rgb(0x010203))
        );
    }

    #[::core::prelude::v1::test]
    fn double_click_word_boundaries_preserve_unicode_and_separators() {
        let cells = "hello pod_name 世界 42"
            .chars()
            .map(|ch| ch.to_string())
            .collect::<Vec<_>>();
        assert_eq!(super::word_columns(&cells, 2), (0, 5));
        assert_eq!(super::word_columns(&cells, 8), (6, 14));
        assert_eq!(super::word_columns(&cells, 15), (15, 17));
        assert_eq!(super::word_columns(&[], 0), (0, 0));
        let punctuation = ["a", ":", "/", "b"].map(str::to_owned);
        assert_eq!(super::word_columns(&punctuation, 1), (1, 3));
    }

    #[::core::prelude::v1::test]
    fn padded_grid_and_selection_use_the_same_inner_origin() {
        use gpui_kit::{Bounds, point, px, size};
        let bounds = super::inner_bounds(Bounds {
            origin: point(px(100.), px(200.)),
            size: size(px(824.), px(452.)),
        });
        assert_eq!(bounds.origin, point(px(112.), px(202.)));
        assert_eq!(bounds.size, size(px(800.), px(448.)));
        assert_eq!(super::grid_dimensions(bounds, px(10.), 18.), (80, 24));
        let cell =
            |x, y| super::cell_at_position(point(px(x), px(y)), bounds, px(10.), 18., 80, 24);
        assert_eq!(cell(112., 202.), (0, 0));
        assert_eq!(cell(132., 238.), (2, 2));
        // Dragging into padding or outside the terminal clamps safely.
        assert_eq!(cell(100., 200.), (0, 0));
        assert_eq!(cell(2000., 2000.), (23, 80));
        assert_eq!(super::grid_dimensions(bounds, px(20.), 36.), (40, 12));
        let tiny = super::inner_bounds(Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(10.), px(10.)),
        });
        assert_eq!(tiny.size, size(px(0.), px(6.)));
        assert_eq!(super::grid_dimensions(tiny, px(10.), 18.), (1, 1));
    }

    #[::core::prelude::v1::test]
    fn grid_cache_invalidates_for_output_cursor_resize_and_style() {
        let style = super::GridStyle {
            font: gpui_kit::font("JetBrains Mono"),
            font_size: 12.,
            palette: crate::preference_runtime::TerminalPalette::default_for(true),
        };
        let mut cache = super::GridCache::default();
        let mut parser = super::surface_parser(80, 24, false);
        assert!(cache.needs_rebuild(&style));
        cache.style = Some(style.clone());
        assert!(!cache.needs_rebuild(&style));
        assert!(!cache.process_output(&mut parser, b""));
        assert!(!cache.needs_rebuild(&style));
        assert!(cache.process_output(&mut parser, b"ready"));
        assert!(cache.needs_rebuild(&style));
        assert_eq!(parser.screen().contents(), "ready");
        cache.dirty = false;
        assert!(cache.process_output(&mut parser, b"\x1b[2;3H"));
        assert_eq!(parser.screen().cursor_position(), (1, 2));
        assert!(cache.needs_rebuild(&style));
        cache.dirty = false;
        assert!(!cache.resize(&mut parser, 80, 24));
        assert!(!cache.needs_rebuild(&style));
        assert!(cache.resize(&mut parser, 40, 12));
        assert!(cache.needs_rebuild(&style));
        cache.dirty = false;
        for changed in [
            super::GridStyle {
                font_size: 14.,
                ..style.clone()
            },
            super::GridStyle {
                font: gpui_kit::font("monospace"),
                ..style.clone()
            },
            super::GridStyle {
                palette: crate::preference_runtime::TerminalPalette::default_for(false),
                ..style.clone()
            },
            super::GridStyle {
                palette: {
                    let mut p = style.palette.clone();
                    p.ansi[1] = gpui_kit::rgb(0x111111).into();
                    p
                },
                ..style.clone()
            },
        ] {
            assert!(cache.needs_rebuild(&changed));
        }
        assert!(!cache.needs_rebuild(&style));
    }

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
        assert_eq!(
            ansi_color(
                vt100::Color::Idx(196),
                true,
                false,
                &crate::preference_runtime::TerminalPalette::default_for(true)
            ),
            gpui_kit::Hsla::from(gpui_kit::rgb(0xff0000))
        );
        assert_eq!(
            ansi_color(
                vt100::Color::Idx(255),
                true,
                false,
                &crate::preference_runtime::TerminalPalette::default_for(true)
            ),
            gpui_kit::Hsla::from(gpui_kit::rgb(0xeeeeee))
        );
    }
}
