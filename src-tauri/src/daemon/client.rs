
//! The GUI's side of the socket: talks to the daemon, starting one if
//! there isn't a live one already.

use super::protocol::{Envelope, Event, Message, Request, ResponseBody, WireError};
use super::server::socket_path;
use crate::error_logger::{log_error, AppError, Level, Source};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const MODULE: &str = "daemon_client";
/// How long to wait for a freshly spawned daemon to start listening.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(5);
const SPAWN_POLL: Duration = Duration::from_millis(50);
/// A request that gets no answer in this long is treated as a dead daemon
/// rather than blocking a Tauri command (and with it the UI) forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

type Pending = Arc<(Mutex<HashMap<u64, Option<ResponseBody>>>, Condvar)>;

pub struct DaemonClient {
    outbound: Sender<Envelope>,
    pending: Pending,
    next_id: AtomicU64,
}

impl DaemonClient {
    /// Connects to the running daemon, spawning one if needed, and starts
    /// pumping its event stream into `on_event`.
    ///
    /// `on_event` runs on the reader thread, so it must not block.
    pub fn connect_or_spawn(on_event: impl Fn(Event) + Send + 'static) -> Result<Self, AppError> {
        let stream = connect_or_spawn_stream()?;
        let write_half = stream.try_clone().map_err(|e| {
            AppError::new(
                MODULE,
                "DAEMON_CONNECT_FAILED",
                format!("could not clone the daemon socket: {e}"),
            )
        })?;

        let pending: Pending = Arc::new((Mutex::new(HashMap::new()), Condvar::new()));

        // Writer: serializes requests onto the socket.
        let (outbound, rx) = channel::<Envelope>();
        std::thread::Builder::new()
            .name("daemon-writer".into())
            .spawn(move || {
                let mut out = write_half;
                for envelope in rx {
                    let Ok(mut line) = serde_json::to_string(&envelope) else {
                        continue;
                    };
                    line.push('\n');
                    if out.write_all(line.as_bytes()).is_err() || out.flush().is_err() {
                        break;
                    }
                }
            })
            .ok();

        // Reader: routes responses to whoever is waiting, events to the sink.
        let reader_pending = pending.clone();
        std::thread::Builder::new()
            .name("daemon-reader".into())
            .spawn(move || {
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    match serde_json::from_str::<Message>(&line) {
                        Ok(Message::Event { event }) => on_event(event),
                        Ok(Message::Response { id, body }) => {
                            let (lock, cv) = &*reader_pending;
                            lock.lock().unwrap().insert(id, Some(body));
                            cv.notify_all();
                        }
                        Err(e) => log_error(
                            Level::Warn,
                            Source::Backend,
                            MODULE,
                            "DAEMON_BAD_MESSAGE",
                            format!("could not parse a daemon message: {e}"),
                            None,
                            None,
                        ),
                    }
                }

                // Socket closed: wake everyone still waiting so they fail
                // fast instead of sitting out the full timeout.
                let (lock, cv) = &*reader_pending;
                let mut waiting = lock.lock().unwrap();
                for slot in waiting.values_mut() {
                    *slot = Some(ResponseBody::Error {
                        error: WireError {
                            code: "DAEMON_DISCONNECTED".to_string(),
                            message: "the daemon closed the connection".to_string(),
                        },
                    });
                }
                cv.notify_all();
            })
            .ok();

        Ok(Self {
            outbound,
            pending,
            next_id: AtomicU64::new(1),
        })
    }

    /// Sends a request and blocks until the matching response arrives.
    pub fn request(&self, request: Request) -> Result<ResponseBody, AppError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        {
            let (lock, _) = &*self.pending;
            lock.lock().unwrap().insert(id, None);
        }

        self.outbound.send(Envelope { id, request }).map_err(|_| {
            AppError::new(
                MODULE,
                "DAEMON_DISCONNECTED",
                "the daemon connection is gone",
            )
        })?;

        let (lock, cv) = &*self.pending;
        let mut waiting = lock.lock().unwrap();
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            if let Some(Some(body)) = waiting.remove(&id) {
                return Ok(body);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                waiting.remove(&id);
                return Err(AppError::new(
                    MODULE,
                    "DAEMON_TIMEOUT",
                    "the daemon did not answer in time",
                ));
            }
            // `remove` above took the entry out, so put it back before
            // waiting again — otherwise the reader has nowhere to deliver.
            waiting.entry(id).or_insert(None);
            let (guard, _) = cv.wait_timeout(waiting, remaining).unwrap();
            waiting = guard;
        }
    }

    /// `request`, collapsing an error response into an `Err`.
    pub fn call(&self, request: Request) -> Result<ResponseBody, AppError> {
        match self.request(request)? {
            ResponseBody::Error { error } => Err(AppError::new(
                MODULE,
                "DAEMON_REQUEST_FAILED",
                error.to_string(),
            )),
            other => Ok(other),
        }
    }
}

/// The connection as the Tauri commands see it — always present as managed
/// state, even when there's no daemon behind it.
///
/// `State<DaemonClient>` would panic at runtime for every process command if
/// the connection had failed, since Tauri's `state()` panics on an
/// unmanaged type. Wrapping the `Option` moves that failure into an ordinary
/// error the UI can show, and keeps the call sites identical.
pub struct Daemon(pub Option<DaemonClient>);

impl Daemon {
    pub fn call(&self, request: Request) -> Result<ResponseBody, AppError> {
        self.0
            .as_ref()
            .ok_or_else(|| {
                AppError::new(
                    MODULE,
                    "DAEMON_UNAVAILABLE",
                    "not connected to the easy-term daemon",
                )
            })?
            .call(request)
    }
}

fn connect_or_spawn_stream() -> Result<UnixStream, AppError> {
    if let Some(stream) = connect_if_alive() {
        return Ok(stream);
    }

    spawn_daemon()?;

    let deadline = Instant::now() + SPAWN_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(stream) = connect_if_alive() {
            return Ok(stream);
        }
        std::thread::sleep(SPAWN_POLL);
    }

    Err(AppError::new(
        MODULE,
        "DAEMON_SPAWN_TIMEOUT",
        "the daemon did not start listening in time",
    ))
}

/// Connects only if something is actually accepting.
///
/// A socket *file* proves nothing — one left behind by a daemon that was
/// killed still exists on disk, and connecting to it fails with
/// ECONNREFUSED. Treating the file's existence as "a daemon is running"
/// would deadlock startup against a corpse.
fn connect_if_alive() -> Option<UnixStream> {
    UnixStream::connect(socket_path()).ok()
}

fn spawn_daemon() -> Result<(), AppError> {
    let exe = std::env::current_exe().map_err(|e| {
        AppError::new(
            MODULE,
            "DAEMON_SPAWN_FAILED",
            format!("could not find our own executable: {e}"),
        )
    })?;

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    // The whole point is outliving this process, so the daemon gets its own
    // session: no controlling terminal to be hung up on, and no signal sent
    // to the GUI's process group ever reaches it.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }

    cmd.spawn().map(|_| ()).map_err(|e| {
        AppError::new(
            MODULE,
            "DAEMON_SPAWN_FAILED",
            format!("could not start the daemon: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: process commands take `State<Daemon>`, and Tauri's
    /// `state()` panics on an unmanaged type — so a failed connection used
    /// to mean every one of them aborted the app rather than reporting an
    /// error. `Daemon` is always managed; the absence lives inside it.
    #[test]
    fn a_disconnected_daemon_reports_an_error_instead_of_panicking() {
        let daemon = Daemon(None);
        let err = daemon
            .call(Request::ListStatuses)
            .expect_err("a call with no daemon behind it must fail, not panic");
        assert_eq!(err.code, "DAEMON_UNAVAILABLE");
    }
}
