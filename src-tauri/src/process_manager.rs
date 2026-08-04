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
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const MODULE: &str = "process_manager";
const RING_BUFFER_MAX: usize = 1024 * 1024;
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(3);
const READ_CHUNK: usize = 8192;

static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:localhost|127\.0\.0\.1)(?::\d+)?[^\s\x1b]*").unwrap()
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Default)]
pub struct ProcessManager {
    processes: Mutex<HashMap<String, ProcessHandle>>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Serialize, Clone)]
struct StatusPayload {
    id: String,
    status: ProjectStatus,
    pid: Option<u32>,
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

fn emit_status(app: &AppHandle, id: &str, status: ProjectStatus, pid: Option<u32>) {
    let _ = app.emit(
        "process:status",
        StatusPayload {
            id: id.to_string(),
            status,
            pid,
        },
    );
}

pub fn start(app: &AppHandle, project: Project) -> Result<(), AppError> {
    let manager = app.state::<ProcessManager>();
    {
        let processes = manager.processes.lock().unwrap();
        if processes.contains_key(&project.id) {
            return Err(AppError::new(
                MODULE,
                "PROC_ALREADY_RUNNING",
                format!("El proyecto \"{}\" ya está corriendo", project.name),
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
                format!("No se pudo abrir el PTY: {e}"),
            )
            .with_context(serde_json::json!({ "projectId": project.id }))
        })?;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.arg("-c");
    cmd.arg(&project.command);
    cmd.cwd(&project.path);
    for (key, value) in env_resolver::overrides(&project.env) {
        cmd.env(key, value);
    }

    let child = pair.slave.spawn_command(cmd).map_err(|e| {
        emit_status(app, &project.id, ProjectStatus::Crashed, None);
        AppError::new(
            MODULE,
            "PTY_SPAWN_FAILED",
            format!("No se pudo iniciar \"{}\": {e}", project.command),
        )
        .with_context(serde_json::json!({ "projectId": project.id, "command": project.command }))
    })?;

    // The parent must not keep the slave end open: the master's read() only
    // sees EOF once every slave-side file descriptor is closed, and this
    // lingering copy would otherwise block that forever after the child exits.
    drop(pair.slave);

    let pid = child
        .process_id()
        .ok_or_else(|| AppError::new(MODULE, "PTY_SPAWN_FAILED", "El proceso no devolvió un PID"))?
        as i32;

    let reader = pair.master.try_clone_reader().map_err(|e| {
        AppError::new(
            MODULE,
            "PTY_READ_ERROR",
            format!("No se pudo obtener el lector del PTY: {e}"),
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
    );

    emit_status(app, &project.id, ProjectStatus::Running, Some(pid as u32));

    Ok(())
}

pub fn stop(app: &AppHandle, id: &str) -> Result<(), AppError> {
    let manager = app.state::<ProcessManager>();

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
                format!("No se pudo enviar la señal {signal} al proceso {pid}: {err}"),
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
                    format!("\"{name}\" terminó inesperadamente (code {code})"),
                    Some(serde_json::json!({ "projectId": id, "code": code })),
                    None,
                );
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
