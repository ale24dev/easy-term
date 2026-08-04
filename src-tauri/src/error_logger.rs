//! Internal diagnostics: structured JSONL error log for the app's own failures
//! (not the logs of the user's projects). See PLAN.md section 7.

use chrono::Utc;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const RETENTION_DAYS: i64 = 14;
const MAX_TOTAL_BYTES: u64 = 20 * 1024 * 1024;
const DEDUPE_WINDOW: Duration = Duration::from_secs(60);
const DEDUPE_THRESHOLD: u32 = 10;

static SESSION_ID: OnceLock<String> = OnceLock::new();
static APP_VERSION: OnceLock<String> = OnceLock::new();
static OS_VERSION: OnceLock<String> = OnceLock::new();
static SENDER: OnceLock<Sender<ErrorEvent>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Warn,
    Error,
    Fatal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Backend,
    Frontend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEvent {
    pub ts: String,
    pub level: Level,
    pub source: Source,
    pub module: String,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    pub session: String,
    #[serde(rename = "appVersion")]
    pub app_version: String,
    #[serde(rename = "osVersion")]
    pub os_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeats: Option<u32>,
}

impl ErrorEvent {
    fn dedupe_key(&self) -> (String, String) {
        (self.code.clone(), self.message.clone())
    }
}

/// Error type returned by Tauri commands. Carries a stable `code` from the
/// taxonomy in PLAN.md so failures can be grouped later.
///
/// Logging happens once, in `Serialize` (the point where Tauri converts the
/// `Err` into the IPC response) rather than at every call site — this is the
/// "impl de From/middleware" choke point the plan calls for, so command code
/// never has to remember to log.
#[derive(Debug, Clone)]
pub struct AppError {
    pub module: &'static str,
    pub code: &'static str,
    pub message: String,
    pub context: Option<Value>,
}

impl AppError {
    pub fn new(module: &'static str, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            module,
            code,
            message: message.into(),
            context: None,
        }
    }

    pub fn with_context(mut self, context: Value) -> Self {
        self.context = Some(context);
        self
    }

    /// Explicitly logs this error. Used outside the Tauri command-return path
    /// (startup, background threads) where the `Serialize` hook never fires
    /// because the error is never sent over IPC.
    pub fn emit(&self) {
        log_error(
            Level::Error,
            Source::Backend,
            self.module,
            self.code,
            self.message.clone(),
            self.context.clone(),
            None,
        );
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.emit();

        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("code", self.code)?;
        s.serialize_field("message", &self.message)?;
        s.end()
    }
}

fn logs_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("Library")
        .join("Logs")
        .join("easy-term")
}

fn today_log_path() -> PathBuf {
    let date = Utc::now().format("%Y-%m-%d").to_string();
    logs_dir().join(format!("errors-{date}.jsonl"))
}

/// Initializes the writer thread and panic hook. Call once, as early as
/// possible in `setup()`.
pub fn init(app_version: impl Into<String>, os_version: impl Into<String>) {
    let _ = SESSION_ID.set(uuid::Uuid::new_v4().to_string());
    let _ = APP_VERSION.set(app_version.into());
    let _ = OS_VERSION.set(os_version.into());

    let (tx, rx) = mpsc::channel::<ErrorEvent>();
    let _ = SENDER.set(tx);

    std::thread::Builder::new()
        .name("error-logger".into())
        .spawn(move || writer_loop(rx))
        .expect("failed to spawn error-logger thread");

    install_panic_hook();
}

#[allow(clippy::too_many_arguments)]
pub fn log_error(
    level: Level,
    source: Source,
    module: impl Into<String>,
    code: impl Into<String>,
    message: impl Into<String>,
    context: Option<Value>,
    stack: Option<String>,
) {
    let event = ErrorEvent {
        ts: Utc::now().to_rfc3339(),
        level,
        source,
        module: module.into(),
        code: code.into(),
        message: message.into(),
        context,
        stack,
        session: SESSION_ID.get().cloned().unwrap_or_default(),
        app_version: APP_VERSION.get().cloned().unwrap_or_default(),
        os_version: OS_VERSION.get().cloned().unwrap_or_default(),
        repeats: None,
    };

    match SENDER.get() {
        Some(tx) => {
            if tx.send(event).is_err() {
                eprintln!("[easy-term] error-logger channel closed, dropping error event");
            }
        }
        None => {
            eprintln!(
                "[easy-term] error-logger not initialized yet: {} {}",
                event.code, event.message
            );
        }
    }
}

struct DedupeState {
    window_start: Instant,
    count: u32,
    sample: ErrorEvent,
}

fn writer_loop(rx: mpsc::Receiver<ErrorEvent>) {
    enforce_retention();
    let mut current_day = Utc::now().format("%Y-%m-%d").to_string();
    let mut dedupe: HashMap<(String, String), DedupeState> = HashMap::new();

    loop {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(event) => {
                let day = event.ts.get(0..10).unwrap_or(&current_day).to_string();
                if day != current_day {
                    current_day = day;
                    enforce_retention();
                }
                handle_event(event, &mut dedupe);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => sweep_expired(&mut dedupe),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn handle_event(event: ErrorEvent, dedupe: &mut HashMap<(String, String), DedupeState>) {
    let key = event.dedupe_key();
    match dedupe.get_mut(&key) {
        Some(state) if state.window_start.elapsed() < DEDUPE_WINDOW => {
            state.count += 1;
            if state.count <= DEDUPE_THRESHOLD {
                write_event(&event);
            }
        }
        _ => {
            if let Some(prev) = dedupe.remove(&key) {
                flush_dedupe(prev);
            }
            write_event(&event);
            dedupe.insert(
                key,
                DedupeState {
                    window_start: Instant::now(),
                    count: 1,
                    sample: event,
                },
            );
        }
    }
}

fn sweep_expired(dedupe: &mut HashMap<(String, String), DedupeState>) {
    let expired: Vec<_> = dedupe
        .iter()
        .filter(|(_, s)| s.window_start.elapsed() >= DEDUPE_WINDOW)
        .map(|(k, _)| k.clone())
        .collect();
    for key in expired {
        if let Some(state) = dedupe.remove(&key) {
            flush_dedupe(state);
        }
    }
}

fn flush_dedupe(state: DedupeState) {
    if state.count > DEDUPE_THRESHOLD {
        let mut event = state.sample;
        event.ts = Utc::now().to_rfc3339();
        event.repeats = Some(state.count - DEDUPE_THRESHOLD);
        write_event(&event);
    }
}

fn write_event(event: &ErrorEvent) {
    let dir = logs_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("[easy-term] failed to create logs dir: {e}");
        return;
    }

    let line = match serde_json::to_string(event) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[easy-term] failed to serialize error event: {e}");
            return;
        }
    };

    let path = today_log_path();
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut file) => {
            let _ = writeln!(file, "{line}");
        }
        Err(e) => {
            eprintln!(
                "[easy-term] failed to write error log ({}): {e}; event: {line}",
                path.display()
            );
        }
    }
}

fn enforce_retention() {
    let dir = logs_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };

    let mut files: Vec<(PathBuf, chrono::NaiveDate, u64)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(date_str) = name
            .strip_prefix("errors-")
            .and_then(|s| s.strip_suffix(".jsonl"))
        else {
            continue;
        };
        let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        files.push((path, date, size));
    }

    let cutoff = Utc::now().date_naive() - chrono::Duration::days(RETENTION_DAYS);
    files.retain(|(path, date, _)| {
        if *date < cutoff {
            let _ = fs::remove_file(path);
            false
        } else {
            true
        }
    });

    files.sort_by_key(|(_, date, _)| *date);
    let mut total: u64 = files.iter().map(|(_, _, size)| size).sum();
    for (path, _, size) in &files {
        if total <= MAX_TOTAL_BYTES {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(*size);
        }
    }
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        let location = info
            .location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "unknown location".to_string());

        log_error(
            Level::Fatal,
            Source::Backend,
            "panic",
            "BACKEND_PANIC",
            format!("{message} at {location}"),
            None,
            Some(std::backtrace::Backtrace::force_capture().to_string()),
        );

        // Give the writer thread a brief moment to flush before the process aborts.
        std::thread::sleep(Duration::from_millis(50));

        default_hook(info);
    }));
}

/// Error payload sent from the frontend's global error handlers.
#[derive(Debug, Deserialize)]
pub struct FrontendError {
    pub level: Level,
    pub module: String,
    pub code: String,
    pub message: String,
    pub context: Option<Value>,
    pub stack: Option<String>,
}

#[tauri::command]
pub fn log_app_error(payload: FrontendError) {
    log_error(
        payload.level,
        Source::Frontend,
        payload.module,
        payload.code,
        payload.message,
        payload.context,
        payload.stack,
    );
}

#[tauri::command]
pub fn read_error_log(day: Option<String>, limit: Option<usize>) -> Result<Vec<Value>, AppError> {
    let date = day.unwrap_or_else(|| Utc::now().format("%Y-%m-%d").to_string());
    let path = logs_dir().join(format!("errors-{date}.jsonl"));

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(AppError::new(
                "error_logger",
                "STORE_READ_FAILED",
                format!("No se pudo leer el log de errores: {e}"),
            ))
        }
    };

    let mut events: Vec<Value> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    if let Some(limit) = limit {
        let len = events.len();
        if len > limit {
            events.drain(0..len - limit);
        }
    }

    Ok(events)
}

#[tauri::command]
pub fn open_logs_folder() -> Result<(), AppError> {
    let dir = logs_dir();
    fs::create_dir_all(&dir).map_err(|e| {
        AppError::new(
            "error_logger",
            "IO_CREATE_DIR_FAILED",
            format!("No se pudo crear la carpeta de logs: {e}"),
        )
    })?;
    tauri_plugin_opener::open_path(dir.to_string_lossy().to_string(), None::<&str>).map_err(|e| {
        AppError::new(
            "error_logger",
            "IO_OPEN_FAILED",
            format!("No se pudo abrir la carpeta de logs: {e}"),
        )
    })
}
