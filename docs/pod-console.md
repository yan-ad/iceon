# Native Pod console

The bottom Pod console uses GPUI Kit 0.9's standard `DockArea` and `DockSkin`. Only the bottom console has a Dock indicator, title, collapse and maximize controls. The resource sidebar and inspector are ordinary views; the main pane has no Dock chrome. Console resizing, collapse, maximize, and restore come from the library rather than a custom docking system.

There is one persistent console tab row: **Terminal**, additional device sessions, Pod sessions, and **+ New**. There is no separate Dock title row. When collapsed the same row remains visible, including on the catalog page. Clicking **Terminal** explicitly opens or focuses this device's shell; **+ New** opens another local device session. The dropdown also opens shell, log, or attach sessions for the selected Pod/container. Closing a background tab does not change the selected session, and closing the primary device shell leaves the permanent Terminal action available.

Device terminals use `portable-pty` and the local user's validated `$SHELL` (or the Windows command shell), never the remote Pod shell preference. They are distinct from Kubernetes streams, bounded to a combined 12 console sessions, and killed/reaped when closed. Application quit closes every device session, including pending opens. Merely displaying Terminal or an empty console does not spawn a process.

## Rendering and fonts

JetBrains Mono regular, bold, italic, and bold-italic faces are bundled under the SIL Open Font License and registered once at startup. Terminal and log rendering never enumerate installed fonts per frame. Terminal text runs with identical styles are coalesced, and log filtering/wrapping is cached until output or presentation settings change.

Idle polls do not notify the workspace. Terminal chunks redraw only the terminal entity; active log output/status updates invalidate only the console pane. Rendering never invalidates all Dock panes, and unchanged visibility never rebuilds or updates the layout. Follow-to-bottom scrolling occurs on incoming output or explicit user actions, not on every frame.

## Logs

The viewer uses GPUI Kit tabs, searchable container Select, selectable text, checkboxes, and a virtualized list. It provides search, follow/to-bottom, pause/resume, timestamps, previous-container logs, word wrapping, copy, and explicit download. Changing backend log options or the container reconnects the existing Kubernetes stream; rendering-only options do not.

Retention is bounded to 2,500 lines and 1 MiB, with incomplete lines capped at 256 KiB. Chunk boundaries and split CRLF are preserved. Pausing stops snapshot draining, allowing the backend's bounded ring to retain the backlog. Export contains only retained logs, not a new server-side fetch.

## Terminal choice

A terminal needs a stateful screen emulator, not newline splitting. The shell/attach surface uses the pure-Rust [`vt100` 0.18.1](https://docs.rs/vt100/0.18.1/vt100/) parser and GPUI text primitives for rendering. Kubernetes exec/attach remains the transport for Pod sessions; only explicit device-terminal actions start a local PTY.

The evaluated [`gpui-terminal`](https://github.com/zortax/gpui-terminal) component uses `alacritty_terminal`, but its published dependency targets GPUI 0.2.2 rather than this workspace's GPUI Kit 0.9 backend. Its README also lists missing mouse selection and scrollback navigation. Adding a second GPUI engine would not be a compatible drop-in, so it is not used.

Supported behavior includes incremental ANSI/VT parsing, cursor positioning and clearing, alternate screens, ANSI/256/RGB colors, bold/italic/underline, raw keyboard input, application-cursor arrows, control keys, explicit paste, bracketed paste, native resize messages, bounded scrollback, and mouse selection/copy. Input is not locally echoed; remote output drives the screen.

Terminal output cannot trigger host clipboard writes, URL opening, local process creation, or other OSC side effects. Clipboard reads and writes require user gestures. Existing Pod UID checks, session lease ownership, stream limits, and cancellation remain in the core engine. Device shell input is intentionally local and is sent only from an explicit user input or paste gesture.

Limitations: this is not a full OS terminal application; advanced keyboard protocols, mouse reporting to remote applications, and IME preedit/composition are not implemented. Log search is over retained output, and log styling is a readable text projection rather than a full terminal screen.

## Validation

- `cargo test --workspace --all-features --locked`
- `cargo run -p iceon-ui --example console_probe --locked`

The offline probe exercises the real terminal surface and standard Dock with fixtures: ANSI cells, raw key events, no local echo, measured terminal resize, virtualized logs, Dock resize/collapse/maximize/restore, bundled font registration, and idle repaint counts. It does not connect to a cluster or spawn a local process.

Dock reference: https://gpui-kit.com/versions/v0.9.0/component/dock
