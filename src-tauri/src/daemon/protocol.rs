// The server and client that consume these types land in the next
// commit; until then the whole module is legitimately unreferenced.
#![allow(dead_code)]

//! Wire format shared by the daemon and the GUI client.
//!
//! One JSON object per line over a Unix socket. Line-delimited JSON keeps
//! the framing trivial to reason about (and to inspect by hand with `nc`
//! while debugging) at the cost of not being able to send raw bytes — which
//! is fine here, since process output is already converted to a `String`
//! before it ever leaves `process_manager`.

use crate::error_logger::AppError;
use crate::process_manager::{ProjectStatus, StatusPayload};
use crate::project_store::Project;
use crate::resource_monitor::ProcessStats;
use serde::{Deserialize, Serialize};

/// An `AppError` as it travels over the socket.
///
/// Deliberately a separate type rather than reusing `AppError`:
///
/// - `AppError`'s own `Serialize` impl *logs the error as a side effect* —
///   that's the choke point guaranteeing no command error goes unrecorded.
///   The daemon already logs on its side before replying, so serializing an
///   `AppError` again in the GUI would double-count every failure.
/// - Its `module`/`code` are `&'static str`, which can't be deserialized
///   into owned data anyway.
///
/// The serialized shape is identical to `AppError`'s (`{code, message}`), so
/// the frontend can't tell the difference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
}

impl From<AppError> for WireError {
    fn from(e: AppError) -> Self {
        // Log it here, on the daemon side, where the context is: this is the
        // one place the `AppError` is consumed, and it replaces the logging
        // its `Serialize` impl would otherwise have done.
        e.emit();
        Self {
            code: e.code.to_string(),
            message: e.message,
        }
    }
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

/// A command from the GUI to the daemon.
///
/// Mirrors the process-related half of `commands.rs`: everything that used
/// to call `process_manager` directly now travels as one of these.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Request {
    /// Round-trips to prove the socket belongs to a live daemon rather than
    /// being a stale file left behind by one that died.
    Ping,
    Start {
        project: Box<Project>,
    },
    Stop {
        id: String,
    },
    Restart {
        project: Box<Project>,
    },
    StartGroup {
        projects: Vec<Project>,
    },
    StopGroup {
        project_ids: Vec<String>,
    },
    /// The scrollback the daemon has buffered for a project — this is what
    /// makes reopening the app show the logs from before it was closed.
    GetOutput {
        id: String,
    },
    ListStatuses,
    ErrorCount {
        id: String,
    },
    ResetErrorCount {
        id: String,
    },
    Forget {
        id: String,
    },
    GetStats {
        id: String,
    },
}

/// The daemon's reply to a [`Request`], correlated by `id`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResponseBody {
    Ok,
    Error { error: WireError },
    Output { text: String },
    Statuses { statuses: Vec<StatusPayload> },
    Count { count: u32 },
    Stats { stats: Option<ProcessStats> },
}

/// Everything the daemon sends back, tagged so a client can tell an
/// unsolicited broadcast apart from the answer it's waiting on.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Message {
    /// Reply to the request that carried the same `id`.
    Response { id: u64, body: ResponseBody },
    /// Broadcast to every connected client.
    Event { event: Event },
}

/// A request paired with the id its response will carry.
///
/// The request is a nested object rather than `#[serde(flatten)]`ed in:
/// several `Request` variants carry their own `id` field (the *project* id),
/// which would collide with the correlation id and silently produce JSON
/// with a duplicate key.
#[derive(Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub id: u64,
    pub request: Request,
}

/// Something that happened to a managed process.
///
/// These map almost one-to-one onto the `process:*` events the frontend
/// already listens for — see [`Event::tauri_event`] — so the GUI can forward
/// them verbatim and no frontend code needs to know a daemon exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    Status {
        id: String,
        status: ProjectStatus,
        pid: Option<u32>,
    },
    Output {
        id: String,
        chunk: String,
    },
    Exit {
        id: String,
        code: i32,
    },
    UrlDetected {
        id: String,
        url: String,
    },
    ErrorCount {
        id: String,
        count: u32,
    },
    RestartScheduled {
        id: String,
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
    },
    RestartExhausted {
        id: String,
    },
    /// Distinct from `Exit`: the GUI turns this into a native notification.
    /// Lives here rather than in the daemon because a headless daemon has no
    /// business posting UI, and only the GUI knows if anyone's looking.
    Crashed {
        id: String,
        name: String,
        code: i32,
    },
}

impl Event {
    /// The frontend event name and JSON payload to forward this as.
    ///
    /// `Crashed` is handled natively by the GUI instead, so it has no
    /// frontend counterpart.
    pub fn tauri_event(&self) -> Option<(&'static str, serde_json::Value)> {
        use serde_json::json;
        match self {
            Event::Status { id, status, pid } => Some((
                "process:status",
                json!({ "id": id, "status": status, "pid": pid }),
            )),
            Event::Output { id, chunk } => {
                Some(("process:output", json!({ "id": id, "chunk": chunk })))
            }
            Event::Exit { id, code } => Some(("process:exit", json!({ "id": id, "code": code }))),
            Event::UrlDetected { id, url } => {
                Some(("process:url-detected", json!({ "id": id, "url": url })))
            }
            Event::ErrorCount { id, count } => {
                Some(("process:error-count", json!({ "id": id, "count": count })))
            }
            Event::RestartScheduled {
                id,
                attempt,
                max_attempts,
                delay_ms,
            } => Some((
                "process:restart-scheduled",
                json!({
                    "id": id,
                    "attempt": attempt,
                    "maxAttempts": max_attempts,
                    "delayMs": delay_ms,
                }),
            )),
            Event::RestartExhausted { id } => {
                Some(("process:restart-exhausted", json!({ "id": id })))
            }
            Event::Crashed { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the line protocol: a message has to survive a
    /// round trip through text without changing shape.
    #[test]
    fn requests_round_trip_through_json() {
        let envelope = Envelope {
            id: 7,
            request: Request::Stop {
                id: "abc".to_string(),
            },
        };
        let line = serde_json::to_string(&envelope).unwrap();
        assert!(!line.contains('\n'), "a message must fit on a single line");

        let back: Envelope = serde_json::from_str(&line).unwrap();
        assert_eq!(back.id, 7);
        assert!(matches!(back.request, Request::Stop { id } if id == "abc"));
    }

    /// Regression: `Envelope` used to `#[serde(flatten)]` its request, which
    /// emitted a duplicate `id` key for every variant carrying a project id
    /// — the correlation id and the project id landed on the same field, and
    /// deserializing the result failed outright.
    #[test]
    fn the_correlation_id_does_not_collide_with_a_project_id() {
        let line = serde_json::to_string(&Envelope {
            id: 99,
            request: Request::Stop {
                id: "project-42".to_string(),
            },
        })
        .unwrap();

        let raw: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(raw["id"], 99, "the envelope keeps the correlation id");
        assert_eq!(
            raw["request"]["id"], "project-42",
            "the project id stays inside the nested request"
        );
    }

    #[test]
    fn events_round_trip_through_json() {
        let event = Event::RestartScheduled {
            id: "p1".to_string(),
            attempt: 3,
            max_attempts: 5,
            delay_ms: 4000,
        };
        let line = serde_json::to_string(&Message::Event {
            event: event.clone(),
        })
        .unwrap();

        let back: Message = serde_json::from_str(&line).unwrap();
        match back {
            Message::Event {
                event:
                    Event::RestartScheduled {
                        attempt, delay_ms, ..
                    },
            } => {
                assert_eq!(attempt, 3);
                assert_eq!(delay_ms, 4000);
            }
            other => panic!("expected a RestartScheduled event, got {other:?}"),
        }
    }

    /// Output can contain anything a dev server prints — newlines above all,
    /// which would break the line framing if they weren't escaped.
    #[test]
    fn output_with_newlines_stays_on_one_line() {
        let line = serde_json::to_string(&Message::Event {
            event: Event::Output {
                id: "p1".to_string(),
                chunk: "first\nsecond\r\n\x1b[31mred\x1b[0m\n".to_string(),
            },
        })
        .unwrap();
        assert!(
            !line.contains('\n'),
            "newlines must be escaped, not literal"
        );

        let back: Message = serde_json::from_str(&line).unwrap();
        match back {
            Message::Event {
                event: Event::Output { chunk, .. },
            } => assert!(chunk.contains("\x1b[31mred"), "ANSI must survive intact"),
            other => panic!("expected an Output event, got {other:?}"),
        }
    }

    /// The frontend contract: these names and payload keys are what
    /// `App.tsx` already listens for, so they can't drift.
    #[test]
    fn events_map_onto_the_frontend_event_names() {
        let (name, payload) = Event::Status {
            id: "p1".to_string(),
            status: ProjectStatus::Running,
            pid: Some(4242),
        }
        .tauri_event()
        .expect("status has a frontend counterpart");

        assert_eq!(name, "process:status");
        assert_eq!(payload["id"], "p1");
        assert_eq!(payload["status"], "running");
        assert_eq!(payload["pid"], 4242);
    }

    #[test]
    fn restart_scheduled_keeps_its_camel_case_payload_keys() {
        let (_, payload) = Event::RestartScheduled {
            id: "p1".to_string(),
            attempt: 2,
            max_attempts: 5,
            delay_ms: 2000,
        }
        .tauri_event()
        .unwrap();

        assert_eq!(payload["maxAttempts"], 5, "frontend reads maxAttempts");
        assert_eq!(payload["delayMs"], 2000, "frontend reads delayMs");
    }

    #[test]
    fn crash_notifications_are_the_guis_job_not_the_frontends() {
        assert!(Event::Crashed {
            id: "p1".to_string(),
            name: "web".to_string(),
            code: 1,
        }
        .tauri_event()
        .is_none());
    }
}
