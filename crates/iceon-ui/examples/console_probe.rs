//! Run with `cargo run -p iceon-ui --example console_probe --locked`.
//! Debug-only, offline fixtures: no cluster, local shell, or PTY is started.
#[cfg(debug_assertions)]
fn main() {
    iceon_ui::run_console_probe();
}

#[cfg(not(debug_assertions))]
fn main() {}
