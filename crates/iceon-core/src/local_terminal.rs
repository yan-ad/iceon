//! Explicitly opened, device-local terminals. No Kubernetes configuration is used.
//!
//! `open` performs process/PTY setup and must run on a blocking executor. All
//! other public methods only touch bounded memory; PTY I/O and child reaping run
//! on owned threads. Output is raw terminal bytes (including ANSI and split UTF-8).
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

const OUTPUT_LIMIT: usize = 512 * 1024;
const INPUT_LIMIT: usize = 64 * 1024;
const INPUT_CHUNK_LIMIT: usize = 16 * 1024;
const INPUT_QUEUE_LIMIT: usize = 64;
const SESSION_LIMIT: usize = 12;
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static SESSIONS: Mutex<Vec<Weak<Inner>>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Running,
    Closed,
    Exited(u32),
    Failed,
}

/// Await child supervisors after shutdown without blocking the UI thread.
pub async fn drain() {
    while ACTIVE.load(Ordering::Acquire) > 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[derive(Debug)]
pub struct Snapshot {
    pub output: Vec<u8>,
    pub status: Status,
    /// Output was discarded since the previous read. Reset the terminal parser
    /// before consuming this snapshot if continuity is required.
    pub dropped: bool,
}

struct State {
    output: VecDeque<u8>,
    dropped: bool,
    status: Status,
    input: VecDeque<Vec<u8>>,
    // Includes the chunk currently being written by the I/O worker.
    input_bytes: usize,
    resize: Option<PtySize>,
}

impl State {
    fn new() -> Self {
        Self {
            output: VecDeque::new(),
            dropped: false,
            status: Status::Running,
            input: VecDeque::new(),
            input_bytes: 0,
            resize: None,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        let excess = (self.output.len() + bytes.len()).saturating_sub(OUTPUT_LIMIT);
        if excess > 0 {
            self.dropped = true;
            self.output.drain(..excess.min(self.output.len()));
        }
        self.output
            .extend(bytes.iter().rev().take(OUTPUT_LIMIT).rev().copied());
    }
}

struct Inner {
    state: Mutex<State>,
    wake: Condvar,
    closed: AtomicBool,
}

impl Inner {
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let mut state = lock(&self.state);
        if state.status == Status::Running {
            state.status = Status::Closed;
        }
        state.input.clear();
        state.resize = None;
        self.wake.notify_all();
    }

    fn fail(&self) {
        let mut state = lock(&self.state);
        if state.status == Status::Running {
            state.status = Status::Failed;
        }
        drop(state);
        self.close();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Permit;
impl Permit {
    fn acquire() -> Result<Self, String> {
        ACTIVE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < SESSION_LIMIT).then_some(active + 1)
            })
            .map(|_| Self)
            .map_err(|_| "Local terminal limit reached (12)".into())
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

// Startup failure must also kill and reap a successfully spawned child. This
// guard is only ever dropped on the blocking opener or supervisor, never UI.
struct OwnedChild {
    child: Box<dyn Child + Send + Sync>,
    reaped: bool,
    _permit: Permit,
}
impl OwnedChild {
    fn supervise(mut self, supervisor: Arc<Inner>) {
        loop {
            if supervisor.closed.load(Ordering::Acquire) {
                // Reaping and OS operations never run on the calling/UI thread.
                let _ = self.child.kill();
                if self.child.wait().is_ok() {
                    self.reaped = true;
                }
                break;
            }
            match self.child.try_wait() {
                Ok(Some(exit)) => {
                    self.reaped = true;
                    let mut state = lock(&supervisor.state);
                    if state.status == Status::Running {
                        state.status = Status::Exited(exit.exit_code());
                    }
                    drop(state);
                    supervisor.close();
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(_) => {
                    supervisor.fail();
                    break;
                }
            }
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Owns a single shell on this device. Share with `Arc<LocalTerminal>` rather
/// than cloning the owner: dropping the owner always requests shutdown.
pub struct LocalTerminal {
    inner: Arc<Inner>,
    shell_name: String,
}

impl LocalTerminal {
    /// Launch the local user's validated shell, with no injected commands.
    pub fn open(cols: u16, rows: u16) -> Result<Self, String> {
        let shell = default_shell()?;
        let name = shell
            .file_name()
            .unwrap_or(shell.as_os_str())
            .to_string_lossy()
            .into_owned();
        let mut command = CommandBuilder::new(&shell);
        command.env("TERM", "xterm-256color");
        #[cfg(unix)]
        command.env("SHELL", &shell);
        Self::open_command(cols, rows, command, name)
    }

    fn open_command(
        cols: u16,
        rows: u16,
        command: CommandBuilder,
        shell_name: String,
    ) -> Result<Self, String> {
        let size = size(cols, rows)?;
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err("Application is shutting down".into());
        }
        let permit = Permit::acquire()?;
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| format!("Open local PTY: {e}"))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Read local PTY: {e}"))?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Write local PTY: {e}"))?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("Start local shell: {e}"))?;
        let child = OwnedChild {
            child,
            reaped: false,
            _permit: permit,
        };
        drop(pair.slave);
        let inner = Arc::new(Inner {
            state: Mutex::new(State::new()),
            wake: Condvar::new(),
            closed: AtomicBool::new(false),
        });
        let terminal = Self {
            inner: inner.clone(),
            shell_name,
        };
        {
            let mut sessions = lock(&SESSIONS);
            if SHUTTING_DOWN.load(Ordering::Acquire) {
                return Err("Application is shutting down".into());
            }
            sessions.retain(|session| session.strong_count() > 0);
            sessions.push(Arc::downgrade(&inner));
        }
        let supervisor = inner.clone();
        thread::Builder::new()
            .name("local-terminal-child".into())
            .spawn(move || child.supervise(supervisor))
            .map_err(|e| format!("Start local child supervisor: {e}"))?;
        let output = inner.clone();
        thread::Builder::new()
            .name("local-terminal-read".into())
            .spawn(move || {
                let mut buffer = [0; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => lock(&output.state).append(&buffer[..count]),
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        // Unix PTYs commonly return EIO instead of EOF after exit.
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| format!("Start local PTY reader: {e}"))?;
        thread::Builder::new()
            .name("local-terminal-write".into())
            .spawn(move || {
                let master = pair.master;
                loop {
                    let (input, resize) = {
                        let mut state = lock(&inner.state);
                        while state.input.is_empty()
                            && state.resize.is_none()
                            && !inner.closed.load(Ordering::Acquire)
                        {
                            state = inner.wake.wait(state).unwrap_or_else(|e| e.into_inner());
                        }
                        if inner.closed.load(Ordering::Acquire) {
                            break;
                        }
                        (state.input.pop_front(), state.resize.take())
                    };
                    if let Some(size) = resize {
                        if master.resize(size).is_err() {
                            inner.fail();
                            break;
                        }
                    }
                    if let Some(input) = input {
                        let result = writer.write_all(&input).and_then(|_| writer.flush());
                        lock(&inner.state).input_bytes -= input.len();
                        if result.is_err() {
                            inner.fail();
                            break;
                        }
                    }
                }
            })
            .map_err(|e| format!("Start local PTY writer: {e}"))?;
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err("Application is shutting down".into());
        }
        Ok(terminal)
    }

    /// Drain pending bytes without decoding them or doing device I/O.
    pub fn read(&self) -> Snapshot {
        let mut state = lock(&self.inner.state);
        Snapshot {
            output: state.output.drain(..).collect(),
            status: state.status,
            dropped: std::mem::take(&mut state.dropped),
        }
    }

    /// Enqueue input atomically, or reject it without partially sending it.
    /// Callers may split larger pastes into chunks of at most 16 KiB.
    pub fn write(&self, input: &[u8]) -> Result<(), String> {
        if input.len() > INPUT_CHUNK_LIMIT {
            return Err("Local terminal input chunk exceeds 16 KiB".into());
        }
        let mut state = lock(&self.inner.state);
        if self.inner.closed.load(Ordering::Acquire) {
            return Err("Local terminal is closed".into());
        }
        if input.is_empty() {
            return Ok(());
        }
        if state.input_bytes + input.len() > INPUT_LIMIT || state.input.len() >= INPUT_QUEUE_LIMIT {
            return Err("Local terminal input queue is full".into());
        }
        state.input.push_back(input.to_vec());
        state.input_bytes += input.len();
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Coalesce resize events; the worker applies only the latest pending size.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let size = size(cols, rows)?;
        let mut state = lock(&self.inner.state);
        if self.inner.closed.load(Ordering::Acquire) {
            return Err("Local terminal is closed".into());
        }
        state.resize = Some(size);
        self.inner.wake.notify_one();
        Ok(())
    }

    pub fn shell_name(&self) -> &str {
        &self.shell_name
    }

    /// Idempotent, nonblocking shutdown request. The supervisor kills/reaps.
    pub fn close(&self) {
        self.inner.close();
    }
}
impl Drop for LocalTerminal {
    fn drop(&mut self) {
        self.close();
    }
}

/// Permanently reject new opens and request shutdown of every owned terminal.
/// Call from application quit, including when an asynchronous open is pending.
pub fn shutdown_all() {
    let mut sessions = lock(&SESSIONS);
    SHUTTING_DOWN.store(true, Ordering::Release);
    for session in sessions.drain(..).filter_map(|session| session.upgrade()) {
        session.close();
    }
}

fn size(cols: u16, rows: u16) -> Result<PtySize, String> {
    if cols == 0 || rows == 0 {
        return Err("Local terminal dimensions must be nonzero".into());
    }
    Ok(PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    })
}

fn valid_shell(path: &Path) -> bool {
    if !path.is_absolute() || !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn default_shell() -> Result<PathBuf, String> {
    #[cfg(unix)]
    {
        Ok(std::env::var_os("SHELL")
            .map(PathBuf::from)
            .filter(|path| valid_shell(path))
            .unwrap_or_else(|| PathBuf::from("/bin/sh")))
    }
    #[cfg(windows)]
    {
        if let Some(shell) = std::env::var_os("COMSPEC")
            .map(PathBuf::from)
            .filter(|path| valid_shell(path))
        {
            return Ok(shell);
        }
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let shell = root.join("System32").join("cmd.exe");
        if valid_shell(&shell) {
            Ok(shell)
        } else {
            Err("No absolute local command shell is available".into())
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err("Local terminals are unsupported on this platform".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_output_remains_raw_bytes() {
        let mut state = State::new();
        state.append(&[b'x'; OUTPUT_LIMIT]);
        state.append(b"\x1b[31m\xf0\x9f");
        assert_eq!(state.output.len(), OUTPUT_LIMIT);
        assert!(state.dropped);
        assert!(
            state
                .output
                .make_contiguous()
                .ends_with(b"\x1b[31m\xf0\x9f")
        );
        state.append(&vec![b'y'; OUTPUT_LIMIT * 2]);
        assert_eq!(state.output.len(), OUTPUT_LIMIT);
        assert!(state.output.iter().all(|byte| *byte == b'y'));
    }

    #[test]
    fn validates_dimensions_and_absolute_shells() {
        assert!(size(0, 24).is_err());
        assert!(size(80, 0).is_err());
        assert!(!valid_shell(Path::new("sh")));
        assert!(!valid_shell(Path::new("/missing/iceon-shell")));
    }

    #[test]
    fn queue_bounds_and_close_are_memory_only() {
        let terminal = LocalTerminal {
            inner: Arc::new(Inner {
                state: Mutex::new(State::new()),
                wake: Condvar::new(),
                closed: AtomicBool::new(false),
            }),
            shell_name: "test".into(),
        };
        assert!(terminal.write(&vec![0; INPUT_CHUNK_LIMIT + 1]).is_err());
        for _ in 0..4 {
            terminal.write(&vec![0; INPUT_CHUNK_LIMIT]).unwrap();
        }
        assert!(terminal.write(b"x").is_err());
        terminal.resize(90, 30).unwrap();
        terminal.resize(100, 40).unwrap();
        assert_eq!(lock(&terminal.inner.state).resize.unwrap().cols, 100);
        terminal.close();
        terminal.close();
        assert!(terminal.write(b"x").is_err());
        assert_eq!(terminal.read().status, Status::Closed);
    }

    #[cfg(unix)]
    #[test]
    fn real_pty_preserves_ansi_and_accepts_input() {
        let mut command = CommandBuilder::new("/bin/sh");
        command.args([
            "-c",
            "printf '\\033[32mready\\033[0m\\n'; read value; printf 'received:%s\\n' \"$value\"",
        ]);
        let terminal = LocalTerminal::open_command(80, 24, command, "sh".into()).unwrap();
        terminal.write(b"hello\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        loop {
            let snapshot = terminal.read();
            output.extend(snapshot.output);
            if output
                .windows(b"received:hello".len())
                .any(|bytes| bytes == b"received:hello")
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "PTY output timed out");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(output.windows(5).any(|bytes| bytes == b"\x1b[32m"));
        terminal.close();
    }

    #[cfg(unix)]
    #[test]
    fn dropping_terminal_releases_threads_and_reaps_child() {
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "exec sleep 30"]);
        let terminal = LocalTerminal::open_command(80, 24, command, "sh".into()).unwrap();
        let inner = Arc::downgrade(&terminal.inner);
        drop(terminal);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while inner.upgrade().is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "PTY threads did not stop"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
