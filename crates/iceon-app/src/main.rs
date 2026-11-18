// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(debug_assertions)]
    if std::env::args().any(|arg| arg == "--sidebar-probe") {
        iceon_ui::run_sidebar_probe();
        return;
    }
    iceon_ui::run();
}
