//! Spawns and supervises project processes inside a real PTY.
//!
//! One managed process = 3 background threads: a reader (blocking read loop
//! on the pty), a batcher (coalesces reads into ~16ms UI flushes so a
//! chatty dev server can't flood the frontend), and a waiter (blocks on
//! `Child::wait()` to reap the process and report its exit).

use crate::env_resolver;
use crate::error_logger::{log_error, AppError, Level, Source};
use crate::project_store::Project;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const MODULE: &str = "process_manager";
const RING_BUFFER_MAX: usize = 1024 * 1024;
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(3);
const READ_CHUNK: usize = 8192;
const MAX_RESTART_ATTEMPTS: u32 = 5;
const RESTART_BASE_DELAY_MS: u64 = 1000;
const RESTART_MAX_DELAY_MS: u64 = 30_000;
const GROUP_READINESS_TIMEOUT: Duration = Duration::from_secs(8);
const GROUP_READINESS_POLL: Duration = Duration::from_millis(300);
/// A process must survive at least this long to count as a successful
/// (re)start — below it, a crash is treated as a continuation of the same
/// backoff sequence rather than a fresh one starting over at attempt 1.
const MIN_UPTIME_FOR_RESTART_RESET: Duration = Duration::from_secs(5);

static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:localhost|127\.0\.0\.1)(?::\d+)?[^\s\x1b]*").unwrap()
});

static ERROR_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(error|warn(?:ing)?)\b").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Stopped,
    Starting,
    Running,
    Crashed,
}

/// Signaled by the waiter thread when the child exits; `stop()` blocks on it
/// (with a timeout) to know when to escalate from SIGTERM to SIGKILL.
type ExitSignal = Arc<(Mutex<bool>, Condvar)>;

struct ProcessHandle {
    /// Also the process group id: portable-pty calls `setsid()` before exec,
    /// so the child is its own session/group leader (pid == pgid). Killing
    /// `-pid` therefore reaches the whole tree (e.g. `pnpm run dev` and its
    /// node/vite/esbuild children), not just the direct child.
    pid: i32,
    /// Kept alive for the process's lifetime: dropping it can hang up the
    /// pty depending on platform, and it's needed for future stdin/resize.
    _master: Box<dyn MasterPty + Send>,
    buffer: Arc<Mutex<Vec<u8>>>,
    exited: ExitSignal,
    stopping: Arc<AtomicBool>,
}

/// Tracks auto-restart backoff for one project. `epoch` is bumped by any
/// explicit `stop()` (manual stop, restart, or delete) so an in-flight
/// scheduled restart can detect it was cancelled and no-op instead of
/// reviving a project the user just asked to stop.
#[derive(Default, Clone, Copy)]
struct RestartState {
    attempts: u32,
    epoch: u64,
}

#[derive(Default)]
pub struct ProcessManager {
    processes: Mutex<HashMap<String, ProcessHandle>>,
    /// Last known status per project id, kept even after the process handle
    /// itself is removed (e.g. "crashed" needs to survive past the exit) —
    /// this is what the tray and project list badges read from.
    statuses: Mutex<HashMap<String, ProjectStatus>>,
    /// Count of error/warn-looking lines seen since the log was last opened.
    error_counts: Mutex<HashMap<String, u32>>,
    restart_state: Mutex<HashMap<String, RestartState>>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot_statuses(&self) -> HashMap<String, ProjectStatus> {
        self.statuses.lock().unwrap().clone()
    }

    pub fn error_count(&self, id: &str) -> u32 {
        self.error_counts
            .lock()
            .unwrap()
            .get(id)
            .copied()
            .unwrap_or(0)
    }

    pub fn reset_error_count(&self, id: &str) {
        self.error_counts.lock().unwrap().remove(id);
    }

    /// Drops all bookkeeping for a project that's been deleted, so a reused
    /// id (unlikely with uuids, but cheap to guard) or long-lived sessions
    /// with many create/delete cycles don't accumulate stale map entries.
    pub fn forget(&self, id: &str) {
        self.statuses.lock().unwrap().remove(id);
        self.error_counts.lock().unwrap().remove(id);
        self.restart_state.lock().unwrap().remove(id);
    }

    pub fn pid_of(&self, id: &str) -> Option<i32> {
        self.processes.lock().unwrap().get(id).map(|h| h.pid)
    }

    /// Full status+pid snapshot for every project the manager has ever
    /// touched this session. Lets the frontend reconcile its runtime state
    /// with reality on (re)mount — e.g. after a dev-mode HMR full reload,
    /// or right after launch when a restored project's own `starting`/
    /// `running` events may have fired before the UI was listening.
    pub fn snapshot_all(&self) -> Vec<StatusPayload> {
        let statuses = self.statuses.lock().unwrap();
        let processes = self.processes.lock().unwrap();
        statuses
            .iter()
            .map(|(id, status)| StatusPayload {
                id: id.clone(),
                status: *status,
                pid: processes.get(id).map(|h| h.pid as u32),
            })
            .collect()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StatusPayload {
    pub id: String,
    pub status: ProjectStatus,
    pub pid: Option<u32>,
}

#[derive(Serialize, Clone)]
struct OutputPayload {
    id: String,
    chunk: String,
}

#[derive(Serialize, Clone)]
struct ExitPayload {
    id: String,
    code: i32,
}

#[derive(Serialize, Clone)]
struct UrlPayload {
    id: String,
    url: String,
}

#[derive(Serialize, Clone)]
struct ErrorCountPayload {
    id: String,
    count: u32,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RestartScheduledPayload {
    id: String,
    attempt: u32,
    max_attempts: u32,
    delay_ms: u64,
}

#[derive(Serialize, Clone)]
struct RestartExhaustedPayload {
    id: String,
}

fn emit_status(app: &AppHandle, id: &str, status: ProjectStatus, pid: Option<u32>) {
    let manager = app.state::<ProcessManager>();
    manager
        .statuses
        .lock()
        .unwrap()
        .insert(id.to_string(), status);

    // Deliberately NOT resetting restart attempts here: "Running" fires
    // optimistically right after spawn, before we know the process will
    // actually stay up. Resetting on it would let a process that crashes
    // instantly on every attempt "succeed" every time and never hit the
    // backoff cap. The waiter thread resets attempts instead, based on how
    // long the process actually survived.

    let _ = app.emit(
        "process:status",
        StatusPayload {
            id: id.to_string(),
            status,
            pid,
        },
    );

    crate::tray::refresh(app);
}

pub fn start(app: &AppHandle, project: Project) -> Result<(), AppError> {
    let manager = app.state::<ProcessManager>();
    {
        let processes = manager.processes.lock().unwrap();
        if processes.contains_key(&project.id) {
            return Err(AppError::new(
                MODULE,
                "PROC_ALREADY_RUNNING",
                format!("Project \"{}\" is already running", project.name),
            ));
        }
    }

    emit_status(app, &project.id, ProjectStatus::Starting, None);

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 50,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| {
            emit_status(app, &project.id, ProjectStatus::Crashed, None);
            AppError::new(
                MODULE,
                "PTY_SPAWN_FAILED",
                format!("Could not open the PTY: {e}"),
            )
            .with_context(serde_json::json!({ "projectId": project.id }))
        })?;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.arg("-c");
    cmd.arg(&project.command);
    cmd.cwd(&project.path);
    // Most dev servers (Next.js, Create React App, Express, ...) pick their
    // port from $PORT by convention — without this, the configured port was
    // only ever used for our own pre-start busy check and the "open in
    // browser" URL guess, never actually telling the process to bind there.
    // Set before the user's own env rows so an explicit PORT override there
    // still wins.
    if let Some(port) = project.port {
        cmd.env("PORT", port.to_string());
    }
    for (key, value) in env_resolver::overrides(&project.env) {
        cmd.env(key, value);
    }

    let started_at = Instant::now();
    let child = pair.slave.spawn_command(cmd).map_err(|e| {
        emit_status(app, &project.id, ProjectStatus::Crashed, None);
        AppError::new(
            MODULE,
            "PTY_SPAWN_FAILED",
            format!("Could not start \"{}\": {e}", project.command),
        )
        .with_context(serde_json::json!({ "projectId": project.id, "command": project.command }))
    })?;

    // The parent must not keep the slave end open: the master's read() only
    // sees EOF once every slave-side file descriptor is closed, and this
    // lingering copy would otherwise block that forever after the child exits.
    drop(pair.slave);

    let pid = child.process_id().ok_or_else(|| {
        AppError::new(
            MODULE,
            "PTY_SPAWN_FAILED",
            "The process didn't return a PID",
        )
    })? as i32;

    let reader = pair.master.try_clone_reader().map_err(|e| {
        AppError::new(
            MODULE,
            "PTY_READ_ERROR",
            format!("Could not get the PTY reader: {e}"),
        )
    })?;

    let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let exited: ExitSignal = Arc::new((Mutex::new(false), Condvar::new()));
    let stopping = Arc::new(AtomicBool::new(false));

    {
        let mut processes = manager.processes.lock().unwrap();
        processes.insert(
            project.id.clone(),
            ProcessHandle {
                pid,
                _master: pair.master,
                buffer: buffer.clone(),
                exited: exited.clone(),
                stopping: stopping.clone(),
            },
        );
    }

    spawn_reader_thread(app.clone(), project.id.clone(), reader, buffer);
    spawn_waiter_thread(
        app.clone(),
        project.id.clone(),
        project.name.clone(),
        child,
        exited,
        stopping,
        started_at,
    );

    emit_status(app, &project.id, ProjectStatus::Running, Some(pid as u32));

    Ok(())
}

pub fn stop(app: &AppHandle, id: &str) -> Result<(), AppError> {
    let manager = app.state::<ProcessManager>();

    // Any explicit stop — manual, via restart(), or before a delete —
    // cancels a pending auto-restart so it can't revive the project right
    // after the user asked to stop it.
    manager
        .restart_state
        .lock()
        .unwrap()
        .entry(id.to_string())
        .or_default()
        .epoch += 1;

    let (pid, exited) = {
        let processes = manager.processes.lock().unwrap();
        match processes.get(id) {
            Some(handle) => (handle.pid, handle.exited.clone()),
            None => return Ok(()), // already stopped — idempotent
        }
    };

    // Re-fetch stopping flag separately to keep the lock above short-lived.
    let stopping = {
        let processes = manager.processes.lock().unwrap();
        processes.get(id).map(|h| h.stopping.clone())
    };
    if let Some(stopping) = stopping {
        stopping.store(true, Ordering::Relaxed);
    }

    send_signal(id, pid, libc::SIGTERM)?;

    let (lock, cvar) = &*exited;
    let guard = lock.lock().unwrap();
    let (done, timeout) = cvar
        .wait_timeout_while(guard, GRACEFUL_TIMEOUT, |done| !*done)
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if !*done && timeout.timed_out() {
        send_signal(id, pid, libc::SIGKILL)?;
    }

    Ok(())
}

pub fn restart(app: &AppHandle, project: Project) -> Result<(), AppError> {
    stop(app, &project.id)?;
    start(app, project)
}

/// Exponential backoff for restart attempt `attempt` (1-indexed): 1s, 2s,
/// 4s, 8s, 16s, … capped at `RESTART_MAX_DELAY_MS`.
fn backoff_delay_ms(attempt: u32) -> u64 {
    (RESTART_BASE_DELAY_MS.saturating_mul(1 << (attempt - 1))).min(RESTART_MAX_DELAY_MS)
}

/// Schedules an auto-restart attempt with exponential backoff (1s, 2s, 4s,
/// … capped at 30s), up to `MAX_RESTART_ATTEMPTS`. Called only when a
/// project with `auto_restart` enabled crashes.
fn schedule_restart(app: &AppHandle, project: Project) {
    let manager = app.state::<ProcessManager>();
    let (attempt, epoch) = {
        let mut state = manager.restart_state.lock().unwrap();
        let entry = state.entry(project.id.clone()).or_default();
        entry.attempts += 1;
        (entry.attempts, entry.epoch)
    };

    if attempt > MAX_RESTART_ATTEMPTS {
        log_error(
            Level::Warn,
            Source::Backend,
            MODULE,
            "PROC_RESTART_LIMIT_REACHED",
            format!(
                "\"{}\" exceeded {MAX_RESTART_ATTEMPTS} automatic restart attempts",
                project.name
            ),
            Some(serde_json::json!({ "projectId": project.id })),
            None,
        );
        let _ = app.emit(
            "process:restart-exhausted",
            RestartExhaustedPayload {
                id: project.id.clone(),
            },
        );
        return;
    }

    let delay_ms = backoff_delay_ms(attempt);

    let _ = app.emit(
        "process:restart-scheduled",
        RestartScheduledPayload {
            id: project.id.clone(),
            attempt,
            max_attempts: MAX_RESTART_ATTEMPTS,
            delay_ms,
        },
    );

    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(delay_ms));

        let manager = app.state::<ProcessManager>();
        let still_valid = manager
            .restart_state
            .lock()
            .unwrap()
            .get(&project.id)
            .map(|s| s.epoch)
            == Some(epoch);

        if still_valid {
            let _ = start(&app, project);
        }
    });
}

pub fn get_output(app: &AppHandle, id: &str) -> String {
    let manager = app.state::<ProcessManager>();
    let processes = manager.processes.lock().unwrap();
    match processes.get(id) {
        Some(handle) => {
            let buf = handle.buffer.lock().unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        }
        None => String::new(),
    }
}

/// Starts every project in a group one at a time, in order, waiting for
/// each to look "ready" (its port opens, or a fixed grace period for
/// projects with no configured port) before moving on to the next. Runs on
/// its own thread — the caller gets progress via the usual `process:status`
/// events, not a blocking return.
pub fn start_group(app: &AppHandle, projects: Vec<Project>) {
    let app = app.clone();
    std::thread::spawn(move || {
        for project in projects {
            let port = project.port;
            let id = project.id.clone();
            let name = project.name.clone();

            if let Err(e) = start(&app, project) {
                log_error(
                    Level::Warn,
                    Source::Backend,
                    MODULE,
                    "PROC_GROUP_START_FAILED",
                    format!("Could not start \"{name}\" inside the group: {e}"),
                    Some(serde_json::json!({ "projectId": id })),
                    None,
                );
                continue;
            }

            match port {
                Some(port) => wait_for_port_ready(port, GROUP_READINESS_TIMEOUT),
                None => std::thread::sleep(Duration::from_secs(2)),
            }
        }
    });
}

/// Stops every project in a group concurrently (order doesn't matter for
/// shutdown), so an N-project group stops in ~3s total instead of N × 3s.
pub fn stop_group(app: &AppHandle, project_ids: Vec<String>) {
    for id in project_ids {
        let app = app.clone();
        std::thread::spawn(move || {
            let _ = stop(&app, &id);
        });
    }
}

fn wait_for_port_ready(port: u16, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(result) = crate::port_checker::check_port(port) {
            if !result.free {
                return;
            }
        }
        std::thread::sleep(GROUP_READINESS_POLL);
    }
}

fn send_signal(id: &str, pid: i32, signal: i32) -> Result<(), AppError> {
    let result = unsafe { libc::kill(-pid, signal) };
    if result != 0 {
        let err = std::io::Error::last_os_error();
        // ESRCH just means the process group is already gone — a stop() that
        // wins a race against the process's own natural exit, not a failure.
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(AppError::new(
                MODULE,
                "PROC_KILL_FAILED",
                format!("Could not send signal {signal} to process {pid}: {err}"),
            )
            .with_context(serde_json::json!({ "projectId": id, "pid": pid })));
        }
    }
    Ok(())
}

fn spawn_reader_thread(
    app: AppHandle,
    id: String,
    mut reader: Box<dyn Read + Send>,
    buffer: Arc<Mutex<Vec<u8>>>,
) {
    std::thread::Builder::new()
        .name(format!("pty-reader-{id}"))
        .spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();

            let batcher_app = app.clone();
            let batcher_id = id.clone();
            let batcher = std::thread::Builder::new()
                .name(format!("pty-batch-{batcher_id}"))
                .spawn(move || batch_and_emit(batcher_app, batcher_id, rx))
                .ok();

            let mut url_found = false;
            let mut read_buf = [0u8; READ_CHUNK];

            loop {
                match reader.read(&mut read_buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = read_buf[..n].to_vec();

                        {
                            let mut ring = buffer.lock().unwrap();
                            ring.extend_from_slice(&chunk);
                            if ring.len() > RING_BUFFER_MAX {
                                let excess = ring.len() - RING_BUFFER_MAX;
                                ring.drain(0..excess);
                            }
                        }

                        if !url_found {
                            if let Some(url) = detect_url(&chunk) {
                                url_found = true;
                                let _ = app.emit(
                                    "process:url-detected",
                                    UrlPayload {
                                        id: id.clone(),
                                        url,
                                    },
                                );
                            }
                        }

                        if let Some(count) = count_error_lines(&app, &id, &chunk) {
                            let _ = app.emit(
                                "process:error-count",
                                ErrorCountPayload {
                                    id: id.clone(),
                                    count,
                                },
                            );
                        }

                        if tx.send(chunk).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        log_error(
                            Level::Warn,
                            Source::Backend,
                            MODULE,
                            "PTY_READ_ERROR",
                            format!("Error leyendo el PTY: {e}"),
                            Some(serde_json::json!({ "projectId": id })),
                            None,
                        );
                        break;
                    }
                }
            }

            drop(tx);
            if let Some(handle) = batcher {
                let _ = handle.join();
            }
        })
        .expect("failed to spawn pty reader thread");
}

fn detect_url(chunk: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(chunk);
    URL_RE
        .find(&text)
        .map(|m| m.as_str().trim_end_matches(['/', ')', '"']).to_string())
}

/// Counts lines that look like an error/warning (case-insensitive whole-word
/// "error"/"warn"/"warning").
fn count_error_matches(text: &str) -> u32 {
    text.lines()
        .filter(|line| ERROR_LINE_RE.is_match(line))
        .count() as u32
}

/// Counts lines in `chunk` that look like an error/warning, adds them to the
/// project's running total, and returns the new total — or `None` when the
/// chunk contributed nothing, so callers can skip emitting a no-op event.
fn count_error_lines(app: &AppHandle, id: &str, chunk: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(chunk);
    let matches = count_error_matches(&text);
    if matches == 0 {
        return None;
    }

    let manager = app.state::<ProcessManager>();
    let mut counts = manager.error_counts.lock().unwrap();
    let total = counts.entry(id.to_string()).or_insert(0);
    *total += matches;
    Some(*total)
}

fn batch_and_emit(app: AppHandle, id: String, rx: std::sync::mpsc::Receiver<Vec<u8>>) {
    let mut pending: Vec<u8> = Vec::new();
    loop {
        match rx.recv_timeout(FLUSH_INTERVAL) {
            Ok(chunk) => {
                pending.extend_from_slice(&chunk);
                // Drain whatever else is already queued so a read() burst
                // coalesces into a single flush instead of one emit each.
                while let Ok(more) = rx.try_recv() {
                    pending.extend_from_slice(&more);
                }
                flush_pending(&app, &id, &mut pending);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                flush_pending(&app, &id, &mut pending);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                flush_pending(&app, &id, &mut pending);
                break;
            }
        }
    }
}

fn flush_pending(app: &AppHandle, id: &str, pending: &mut Vec<u8>) {
    if pending.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(pending).into_owned();
    pending.clear();
    let _ = app.emit(
        "process:output",
        OutputPayload {
            id: id.to_string(),
            chunk: text,
        },
    );
}

fn spawn_waiter_thread(
    app: AppHandle,
    id: String,
    name: String,
    mut child: Box<dyn Child + Send + Sync>,
    exited: ExitSignal,
    stopping: Arc<AtomicBool>,
    started_at: Instant,
) {
    std::thread::Builder::new()
        .name(format!("pty-wait-{id}"))
        .spawn(move || {
            let status = child.wait();

            // Remove from the map before signaling `exited`, so that a
            // restart (stop() then start()) waiting on this same signal can
            // never observe a stale entry still occupying the id.
            let manager = app.state::<ProcessManager>();
            manager.processes.lock().unwrap().remove(&id);

            {
                let (lock, cvar) = &*exited;
                let mut done = lock.lock().unwrap();
                *done = true;
                cvar.notify_all();
            }

            let was_stopping = stopping.load(Ordering::Relaxed);
            let (code, clean) = match &status {
                Ok(s) => (s.exit_code() as i32, s.success()),
                Err(_) => (-1, false),
            };

            let final_status = if was_stopping || clean {
                ProjectStatus::Stopped
            } else {
                ProjectStatus::Crashed
            };

            if final_status == ProjectStatus::Crashed {
                log_error(
                    Level::Warn,
                    Source::Backend,
                    MODULE,
                    "PROC_UNEXPECTED_EXIT",
                    format!("\"{name}\" exited unexpectedly (code {code})"),
                    Some(serde_json::json!({ "projectId": id, "code": code })),
                    None,
                );
                crate::notifications::notify_crash(&app, &id, &name, code);

                // A process that ran for a while before dying gets a clean
                // slate: this crash starts a new backoff sequence at attempt
                // 1, rather than compounding on however many attempts a much
                // earlier, unrelated crash loop had already burned through.
                if started_at.elapsed() >= MIN_UPTIME_FOR_RESTART_RESET {
                    manager.restart_state.lock().unwrap().remove(&id);
                }

                let store = app.state::<crate::project_store::ProjectStore>();
                if let Some(project) = store.get(&id) {
                    if project.auto_restart {
                        schedule_restart(&app, project);
                    }
                }
            }

            emit_status(&app, &id, final_status, None);
            let _ = app.emit(
                "process:exit",
                ExitPayload {
                    id: id.clone(),
                    code,
                },
            );
        })
        .expect("failed to spawn pty waiter thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn backoff_delay_doubles_each_attempt_until_the_cap() {
        assert_eq!(backoff_delay_ms(1), 1_000);
        assert_eq!(backoff_delay_ms(2), 2_000);
        assert_eq!(backoff_delay_ms(3), 4_000);
        assert_eq!(backoff_delay_ms(4), 8_000);
        assert_eq!(backoff_delay_ms(5), 16_000);
        // MAX_RESTART_ATTEMPTS is 5, but the formula itself must not
        // overflow or exceed the cap if it were ever called past that.
        assert_eq!(backoff_delay_ms(6), 30_000);
        assert_eq!(backoff_delay_ms(20), 30_000);
    }

    #[test]
    fn detect_url_finds_a_localhost_url_with_port() {
        let chunk = b"Local:   http://localhost:5173/\n";
        assert_eq!(detect_url(chunk), Some("http://localhost:5173".to_string()));
    }

    #[test]
    fn detect_url_finds_a_127_0_0_1_url() {
        let chunk = b"Server running at http://127.0.0.1:3000";
        assert_eq!(detect_url(chunk), Some("http://127.0.0.1:3000".to_string()));
    }

    #[test]
    fn detect_url_strips_trailing_punctuation_and_slash() {
        let chunk = b"see (http://localhost:8080/) for details";
        assert_eq!(detect_url(chunk), Some("http://localhost:8080".to_string()));
    }

    #[test]
    fn detect_url_ignores_non_local_hosts() {
        let chunk = b"Deployed to https://example.com";
        assert_eq!(detect_url(chunk), None);
    }

    #[test]
    fn detect_url_returns_none_when_nothing_matches() {
        let chunk = b"just some ordinary log output\n";
        assert_eq!(detect_url(chunk), None);
    }

    #[test]
    fn count_error_matches_is_case_insensitive_and_whole_word() {
        let text = "Error: build failed\nWARNING: deprecated api\nWarn: low disk\nall good here";
        assert_eq!(count_error_matches(text), 3);
    }

    #[test]
    fn count_error_matches_ignores_substrings_that_are_not_whole_words() {
        // "terrorize" and "forward" contain "error"/"warn" as substrings —
        // the regex is word-bounded, so these must not count.
        let text = "terrorize the forward warehouse";
        assert_eq!(count_error_matches(text), 0);
    }

    #[test]
    fn count_error_matches_returns_zero_for_clean_output() {
        assert_eq!(
            count_error_matches("vite dev server running\nready in 200ms"),
            0
        );
    }

    /// Not a process_manager unit test per se — it exercises the raw
    /// `portable_pty` primitives the same way `start()`/`stop()` do, to pin
    /// down the OS-level assumptions the rest of this module relies on:
    /// `setsid()` makes the child its own process-group leader (so
    /// `pid == pgid`), and `kill(-pid, SIGTERM)` reaches it. `start()` itself
    /// isn't called directly here because it's wired to a concrete
    /// `tauri::AppHandle` (events, tray refresh, managed state) that only a
    /// running Tauri app provides.
    #[test]
    fn pty_spawn_reports_stdout_and_responds_to_process_group_kill() {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg("echo pty-test-marker; sleep 30");

        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);

        let pid = child.process_id().expect("child must report a pid") as i32;

        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut output = Vec::new();
        let mut buf = [0u8; 256];
        let deadline = Instant::now() + Duration::from_secs(5);
        while !String::from_utf8_lossy(&output).contains("pty-test-marker")
            && Instant::now() < deadline
        {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => output.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        assert!(
            String::from_utf8_lossy(&output).contains("pty-test-marker"),
            "did not observe the child's stdout in time"
        );

        // portable-pty calls setsid() before exec, so the child leads its
        // own process group — this is what makes `kill(-pid, ...)` in
        // send_signal() reach the whole tree instead of just the shell.
        let pgid = unsafe { libc::getpgid(pid) };
        assert_eq!(pgid, pid, "child must be its own process group leader");

        let result = unsafe { libc::kill(-pid, libc::SIGTERM) };
        assert_eq!(result, 0, "kill(-pid, SIGTERM) must succeed");

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "child did not exit after SIGTERM"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn pty_child_exit_status_reflects_a_nonzero_exit_code() {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg("exit 7");

        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        // Drop our copy of the reader so the PTY doesn't keep the exited
        // child's slave fd referenced past its own lifetime.
        let _ = pair.master.try_clone_reader().unwrap();

        let status = child.wait().unwrap();
        assert!(!status.success());
        assert_eq!(status.exit_code(), 7);
    }

    #[test]
    fn stdin_written_to_the_master_reaches_the_child() {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg("read line; echo \"got:$line\"");

        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);

        let mut writer = pair.master.take_writer().unwrap();
        writer.write_all(b"hello-from-test\n").unwrap();
        drop(writer);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut output = Vec::new();
        let mut buf = [0u8; 256];
        let deadline = Instant::now() + Duration::from_secs(5);
        while !String::from_utf8_lossy(&output).contains("got:hello-from-test")
            && Instant::now() < deadline
        {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => output.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        assert!(String::from_utf8_lossy(&output).contains("got:hello-from-test"));

        let _ = child.wait();
    }
}
