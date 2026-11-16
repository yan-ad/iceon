#[cfg(debug_assertions)]
use gpui_kit::private::anyhow;
// Offline native Users table audit. The supervisor requires an explicit success
// marker, including when a platform launch exits without invoking callbacks.
#[cfg(debug_assertions)]
fn main() -> anyhow::Result<()> {
    use std::{
        io::Write as _,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    if std::env::args().any(|arg| arg == "--native-child") {
        return iceon_ui::run_rbac_users_probe();
    }
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--native-child")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let start = Instant::now();
    while child.try_wait()?.is_none() {
        if start.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("RBAC Users native child watchdog expired");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let output = child.wait_with_output()?;
    std::io::stdout().write_all(&output.stdout)?;
    anyhow::ensure!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("RBAC_USERS_PROBE_OK:"),
        "RBAC Users native verification incomplete ({})",
        output.status
    );
    Ok(())
}
#[cfg(not(debug_assertions))]
fn main() {
    panic!("rbac_users_probe requires a debug build");
}
