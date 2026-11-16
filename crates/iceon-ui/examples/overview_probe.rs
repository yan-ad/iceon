//! Offline native overview geometry fixture. --compact, --light. Quits after 4s.
#[cfg(debug_assertions)]
fn main() {
    iceon_ui::run_overview_probe();
}
#[cfg(not(debug_assertions))]
fn main() {}
