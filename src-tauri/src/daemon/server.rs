//! The headless half: owns every PTY and outlives the GUI.
//!
//! Run with `easy-term --daemon`. Serves one Unix socket, accepting any
//! number of GUI clients at once (a second app window, or a stale connection
//! that hasn't noticed it's gone). Every client sees the same event stream.

use super::protocol::{Envelope, Event, Message, Request, ResponseBody, WireError};
use crate::error_logger::{log_error, Level, Source};
use crate::process_manager::{self, Context, EventSink, ProcessManager};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

const MODULE: &str = "daemon";

/// Where the daemon listens. Alongside `projects.json`, so everything the
/// app owns lives under one directory.
pub fn socket_path() -> PathBuf {
    // Overridable so a test can point a daemon at its own scratch socket
    // instead of the real one — and so two daemons can be run side by side
    // while debugging.
    if let Ok(path) = std::env::var("EASY_TERM_SOCKET") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("easy-term")
        .join("daemon.sock")
}

/// Fans process events out to every connected client.
///
/// A dead client is dropped on the first failed send rather than tracked
/// separately: a broken pipe *is* the disconnect notification, and the
/// alternative (a liveness check per event) would cost more than it saves
/// on a stream this chatty.
#[derive(Default)]
pub struct Broadcaster {
    clients: Mutex<Vec<Sender<Message>>>,
}

impl Broadcaster {
    pub fn subscribe(&self, tx: Sender<Message>) {
        self.clients.lock().unwrap().push(tx);
    }
}

impl EventSink for Broadcaster {
    fn emit(&self, event: Event) {
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|tx| {
            tx.send(Message::Event {
                event: event.clone(),
            })
            .is_ok()
        });
    }
}

/// Serves until killed. Returns only if the socket can't be set up.
pub fn run() -> Result<(), String> {
    let path = socket_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("could not create {parent:?}: {e}"))?;
    }

    // A socket file left behind by a daemon that died holds the path
    // hostage — bind() fails with EADDRINUSE even though nothing is
    // listening. Removing it is only safe because the client pings an
    // existing socket before deciding a daemon is alive (see client.rs), so
    // by the time we get here any competing daemon has already lost.
    let _ = std::fs::remove_file(&path);

    let listener =
        UnixListener::bind(&path).map_err(|e| format!("could not bind {path:?}: {e}"))?;

    let broadcaster = Arc::new(Broadcaster::default());
    // No ProjectStore here on purpose: the daemon supervises exactly what
    // the GUI hands it (Start carries the whole Project), so there's no
    // second copy of the config that could disagree with the one the user
    // is editing.
    let ctx = Context {
        manager: Arc::new(ProcessManager::new()),
        sink: broadcaster.clone(),
    };

    log_error(
        Level::Warn,
        Source::Backend,
        MODULE,
        "DAEMON_STARTED",
        format!("daemon listening on {path:?}"),
        None,
        None,
    );

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let ctx = ctx.clone();
                let broadcaster = broadcaster.clone();
                std::thread::Builder::new()
                    .name("daemon-client".into())
                    .spawn(move || serve_client(stream, ctx, broadcaster))
                    .ok();
            }
            Err(e) => log_error(
                Level::Warn,
                Source::Backend,
                MODULE,
                "DAEMON_ACCEPT_FAILED",
                format!("could not accept a client: {e}"),
                None,
                None,
            ),
        }
    }

    Ok(())
}

fn serve_client(stream: UnixStream, ctx: Context, broadcaster: Arc<Broadcaster>) {
    let Ok(write_half) = stream.try_clone() else {
        return;
    };

    // One writer thread per client so a slow reader can't block the process
    // threads that produce output: everything outbound — responses and
    // broadcasts alike — funnels through this channel.
    let (tx, rx) = channel::<Message>();
    broadcaster.subscribe(tx.clone());
    std::thread::Builder::new()
        .name("daemon-client-writer".into())
        .spawn(move || {
            let mut out = write_half;
            for message in rx {
                let Ok(mut line) = serde_json::to_string(&message) else {
                    continue;
                };
                line.push('\n');
                if out.write_all(line.as_bytes()).is_err() || out.flush().is_err() {
                    break; // client hung up
                }
            }
        })
        .ok();

    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }

        let envelope: Envelope = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(e) => {
                log_error(
                    Level::Warn,
                    Source::Backend,
                    MODULE,
                    "DAEMON_BAD_REQUEST",
                    format!("could not parse a request: {e}"),
                    None,
                    None,
                );
                continue;
            }
        };

        let body = handle(&ctx, envelope.request);
        if tx
            .send(Message::Response {
                id: envelope.id,
                body,
            })
            .is_err()
        {
            break; // writer thread is gone
        }
    }
}

fn handle(ctx: &Context, request: Request) -> ResponseBody {
    /// `Result<(), AppError>` → a response, logging the error exactly once
    /// on the way (see `WireError`'s note on the double-logging trap).
    fn ok_or_error(result: Result<(), crate::error_logger::AppError>) -> ResponseBody {
        match result {
            Ok(()) => ResponseBody::Ok,
            Err(e) => ResponseBody::Error {
                error: WireError::from(e),
            },
        }
    }

    match request {
        Request::Ping => ResponseBody::Ok,
        Request::Start { project } => ok_or_error(process_manager::start(ctx, *project)),
        Request::Stop { id } => ok_or_error(process_manager::stop(ctx, &id)),
        Request::Restart { project } => ok_or_error(process_manager::restart(ctx, *project)),
        Request::StartGroup { projects } => {
            process_manager::start_group(ctx, projects);
            ResponseBody::Ok
        }
        Request::StopGroup { project_ids } => {
            process_manager::stop_group(ctx, project_ids);
            ResponseBody::Ok
        }
        Request::GetOutput { id } => ResponseBody::Output {
            text: process_manager::get_output(ctx, &id),
        },
        Request::ListStatuses => ResponseBody::Statuses {
            statuses: ctx.manager.snapshot_all(),
        },
        Request::ErrorCount { id } => ResponseBody::Count {
            count: ctx.manager.error_count(&id),
        },
        Request::ResetErrorCount { id } => {
            ctx.manager.reset_error_count(&id);
            ResponseBody::Ok
        }
        Request::Forget { id } => {
            ctx.manager.forget(&id);
            ResponseBody::Ok
        }
        Request::GetStats { id } => ResponseBody::Stats {
            stats: ctx
                .manager
                .pid_of(&id)
                .and_then(|pgid| crate::resource_monitor::stats_for_group(pgid).ok()),
        },
    }
}
