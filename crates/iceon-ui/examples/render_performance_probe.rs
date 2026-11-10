//! Run with `cargo run -p iceon-ui --example render_performance_probe --locked`.
//! Debug-only native rendering audit; fixture data only, no cluster or shell.
#[cfg(debug_assertions)]
fn main() {
    iceon_ui::run_render_performance_probe();
}

#[cfg(not(debug_assertions))]
fn main() {}
