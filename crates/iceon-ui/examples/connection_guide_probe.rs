//! Offline native preview: --connecting, --waiting, --missing-helper, --reviewing, --access-unavailable, --compact, --light. Quits after 12s.
#[cfg(debug_assertions)]
fn main() {
    iceon_ui::run_connection_guide_probe();
}
#[cfg(not(debug_assertions))]
fn main() {}
