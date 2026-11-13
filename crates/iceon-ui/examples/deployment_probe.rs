//! Native offline audit. The supervisor also rejects platform exits which bypass
//! Rust cleanup (macOS can exit successfully before a GPUI callback completes).
//! Pass `--daemon-set` for the zero-count scheduling fixture, `--populated` for
//! owned pod statuses, and `--compact --light` for the narrow light-theme audit.
#[cfg(debug_assertions)]
fn main() {
    use std::{
        io::Write as _,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    if std::env::args().any(|arg| arg == "--native-child") {
        iceon_ui::run_deployment_probe();
        return;
    }
    let mut child = Command::new(std::env::current_exe().expect("probe executable"))
        .args(std::env::args().skip(1))
        .arg("--native-child")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("launch native probe child");
    let started = Instant::now();
    loop {
        if child.try_wait().expect("native child status").is_some() {
            break;
        }
        if started.elapsed() > Duration::from_secs(25) {
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("FAIL deployment_probe: native child watchdog expired");
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let output = child.wait_with_output().expect("native probe output");
    std::io::stdout()
        .write_all(&output.stdout)
        .expect("relay probe output");
    if !output.status.success()
        || !String::from_utf8_lossy(&output.stdout).contains("PASS deployment_probe:")
        || (std::env::args().any(|arg| arg == "--daemon-set")
            && !String::from_utf8_lossy(&output.stdout).contains("real native DaemonSet inspector"))
    {
        eprintln!(
            "FAIL deployment_probe: native process did not complete measured checks (status {})",
            output.status
        );
        std::process::exit(1);
    }
}
#[cfg(not(debug_assertions))]
fn main() {
    panic!("deployment_probe requires a debug build");
}
