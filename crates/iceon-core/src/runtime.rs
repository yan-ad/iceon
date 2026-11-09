//! Shared Tokio async runtime and platform directory paths for the pure-Rust GPUI workspace.
use std::{
    future::Future,
    path::PathBuf,
    sync::OnceLock,
};
use tokio::{runtime::{Builder, Handle, Runtime}, task::JoinHandle};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn global_runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .thread_name("iceon-tokio")
            .build()
            .expect("failed to initialize Tokio runtime")
    })
}

/// Returns the active Tokio runtime handle if called from within a Tokio context
/// (such as `#[tokio::test]`), or the global application runtime handle when called
/// from the GPUI main thread or background executor.
pub fn handle() -> Handle {
    Handle::try_current().unwrap_or_else(|_| global_runtime().handle().clone())
}

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    handle().spawn(future)
}

pub fn spawn_blocking<F, R>(f: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    handle().spawn_blocking(f)
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    global_runtime().block_on(future)
}

#[cfg(test)]
mod tests {
    #[test]
    fn entered_shared_runtime_supports_foreign_executor_timers() {
        // Model GPUI's foreground executor: no Tokio runtime is initially entered,
        // and a different executor polls the future while Tokio drives its timer.
        assert!(tokio::runtime::Handle::try_current().is_err());
        let runtime = super::handle();
        let guard = runtime.enter();
        futures::executor::block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            })
            .await
            .expect("Tokio timers must work when polled by the foreground executor");
        });
        drop(guard);
        assert!(tokio::runtime::Handle::try_current().is_err());
    }
}

pub fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    }
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    }
}

pub fn app_data_dir() -> PathBuf {
    if let Some(override_dir) = std::env::var_os("ICEON_DATA_DIR")
        .or_else(|| std::env::var_os("ICEON_DEV_DATA_DIR"))
    {
        let path = PathBuf::from(override_dir);
        if path.is_absolute() {
            return path;
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = home_dir() {
        return home
            .join("Library")
            .join("Application Support")
            .join("ad.yan.iceon");
    }
    #[cfg(target_os = "windows")]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let path = PathBuf::from(appdata);
        if path.is_absolute() {
            return path.join("ad.yan.iceon");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            let path = PathBuf::from(xdg);
            if path.is_absolute() {
                return path.join("ad.yan.iceon");
            }
        }
        if let Some(home) = home_dir() {
            return home.join(".local").join("share").join("ad.yan.iceon");
        }
    }
    std::env::temp_dir().join("ad.yan.iceon")
}
