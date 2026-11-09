#[cfg(debug_assertions)]
#[tokio::main]
async fn main() {
    if let Err(code) = iceon_core::dev_engine::run().await {
        eprintln!("{code}");
        std::process::exit(1);
    }
    // Tokio stdin uses a blocking reader; do not let it delay signal shutdown.
    std::process::exit(0);
}
#[cfg(not(debug_assertions))]
fn main() {}
