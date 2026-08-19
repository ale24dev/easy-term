//! The property the whole daemon exists for: a project process survives the
//! app closing, and reconnecting finds it still running with its scrollback.
//!
//! Runs against a real `easy-term --daemon` subprocess over a real Unix
//! socket, with `HOME` and the socket path pointed at a scratch dir so it
//! can't touch (or be touched by) a real daemon or a real `projects.json`.

use easy_term_lib::daemon::protocol::{Envelope, Message, Request, ResponseBody};
use easy_term_lib::project_store::Project;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Kills the daemon (and with it everything it supervises) when the test
/// ends, however it ends.
struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "easy-term-daemon-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("could not create the scratch dir");
    dir
}

fn start_daemon(socket: &Path, home: &Path) -> DaemonGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_easy-term"))
        .arg("--daemon")
        .env("EASY_TERM_SOCKET", socket)
        .env("HOME", home)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("could not spawn the daemon");
    DaemonGuard(child)
}

/// A client connection. Deliberately a plain socket rather than the
/// `DaemonClient` helper: dropping this is what simulates the app quitting,
/// and the test wants that to be unmistakably a closed socket.
struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl Client {
    fn connect(socket: &Path) -> Option<Self> {
        let stream = UnixStream::connect(socket).ok()?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("could not set a read timeout");
        let reader = BufReader::new(stream.try_clone().expect("could not clone the socket"));
        Some(Self {
            stream,
            reader,
            next_id: 1,
        })
    }

    fn connect_within(socket: &Path, timeout: Duration) -> Self {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(client) = Self::connect(socket) {
                return client;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the daemon never started listening on {socket:?}");
    }

    /// Writes a request without waiting for its response, returning the id
    /// it was sent under.
    fn send(&mut self, request: Request) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let mut line = serde_json::to_string(&Envelope { id, request }).unwrap();
        line.push('\n');
        self.stream.write_all(line.as_bytes()).unwrap();
        self.stream.flush().unwrap();
        id
    }

    /// Reads until the next response, skipping over any events in between.
    fn next_response(&mut self) -> (u64, ResponseBody) {
        loop {
            let mut buf = String::new();
            if self.reader.read_line(&mut buf).unwrap_or(0) == 0 {
                panic!("the daemon closed the connection while waiting for a response");
            }
            match serde_json::from_str::<Message>(buf.trim()) {
                Ok(Message::Response { id, body }) => return (id, body),
                Ok(_) => continue, // an event
                Err(e) => panic!("could not parse {buf:?}: {e}"),
            }
        }
    }

    /// Sends a request and returns the response, skipping over any events
    /// that arrive in the meantime.
    fn request(&mut self, request: Request) -> ResponseBody {
        let id = self.send(request);

        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match self.next_response() {
                (got, body) if got == id => return body,
                _ => continue, // a reply to something else
            }
        }
        panic!("timed out waiting for a response to request {id}");
    }
}

fn probe_project(id: &str, marker: &str) -> Project {
    Project {
        id: id.to_string(),
        name: "probe".to_string(),
        path: "/".to_string(),
        // Prints a marker immediately, then stays alive. The marker proves
        // the scrollback survived; staying alive proves the process did.
        command: format!("echo {marker}; exec sleep 300"),
        port: None,
        env: HashMap::new(),
        auto_restart: false,
        group_id: None,
        was_running: false,
        color: None,
        pinned: false,
    }
}

fn wait_for<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = f() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_project_outlives_the_client_that_started_it() {
    let dir = scratch_dir();
    let socket = dir.join("daemon.sock");
    let _daemon = start_daemon(&socket, &dir);

    let marker = "scrollback-marker-42";
    let project_id = "probe-1";

    // The "app" starts a project, then quits.
    let pid = {
        let mut app = Client::connect_within(&socket, Duration::from_secs(10));

        assert!(
            matches!(
                app.request(Request::Start {
                    project: Box::new(probe_project(project_id, marker)),
                }),
                ResponseBody::Ok
            ),
            "the daemon should have started the project"
        );

        let pid = wait_for(Duration::from_secs(10), || {
            match app.request(Request::ListStatuses) {
                ResponseBody::Statuses { statuses } => statuses
                    .iter()
                    .find(|s| s.id == project_id)
                    .and_then(|s| s.pid),
                other => panic!("expected statuses, got {other:?}"),
            }
        })
        .expect("the project never reported a pid");

        // Let the process actually write its marker before we disconnect.
        wait_for(Duration::from_secs(10), || {
            match app.request(Request::GetOutput {
                id: project_id.to_string(),
            }) {
                ResponseBody::Output { text } if text.contains(marker) => Some(()),
                _ => None,
            }
        })
        .expect("the marker never showed up in the daemon's buffer");

        pid
        // `app` drops here: the socket closes, exactly as if the GUI quit.
    };

    // Give the daemon a moment to notice the disconnect — if it were going
    // to tear anything down in response, this is when it would.
    std::thread::sleep(Duration::from_secs(1));

    assert!(
        process_is_alive(pid),
        "the project process (pid {pid}) died when the client disconnected — \
         this is the exact regression the daemon exists to prevent"
    );

    // A fresh "app launch" reconnects and finds everything where it was.
    let mut relaunched = Client::connect_within(&socket, Duration::from_secs(10));

    match relaunched.request(Request::ListStatuses) {
        ResponseBody::Statuses { statuses } => {
            let status = statuses
                .iter()
                .find(|s| s.id == project_id)
                .expect("the reconnected client should still see the project");
            assert_eq!(
                status.pid,
                Some(pid),
                "it should be the same process, not a fresh one"
            );
        }
        other => panic!("expected statuses, got {other:?}"),
    }

    match relaunched.request(Request::GetOutput {
        id: project_id.to_string(),
    }) {
        ResponseBody::Output { text } => assert!(
            text.contains(marker),
            "the scrollback from before the client disconnected should still \
             be there, but got: {text:?}"
        ),
        other => panic!("expected output, got {other:?}"),
    }

    // Stopping through the new connection must reach the process the *old*
    // connection started.
    assert!(matches!(
        relaunched.request(Request::Stop {
            id: project_id.to_string(),
        }),
        ResponseBody::Ok
    ));

    assert!(
        wait_for(Duration::from_secs(10), || (!process_is_alive(pid))
            .then_some(()))
        .is_some(),
        "the process should be gone after Stop"
    );
}

#[test]
fn two_clients_both_see_the_same_events() {
    let dir = scratch_dir().join("two-clients");
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("daemon.sock");
    let _daemon = start_daemon(&socket, &dir);

    let mut first = Client::connect_within(&socket, Duration::from_secs(10));
    let mut second = Client::connect_within(&socket, Duration::from_secs(10));

    // Both connections answer independently — the second one isn't starved
    // by the first holding the daemon's attention.
    assert!(matches!(first.request(Request::Ping), ResponseBody::Ok));
    assert!(matches!(second.request(Request::Ping), ResponseBody::Ok));

    assert!(matches!(
        first.request(Request::Start {
            project: Box::new(probe_project("shared", "shared-marker")),
        }),
        ResponseBody::Ok
    ));

    // The client that did *not* issue the Start still sees the project.
    let seen = wait_for(Duration::from_secs(10), || {
        match second.request(Request::ListStatuses) {
            ResponseBody::Statuses { statuses } => {
                statuses.iter().find(|s| s.id == "shared").map(|_| ())
            }
            other => panic!("expected statuses, got {other:?}"),
        }
    });
    assert!(
        seen.is_some(),
        "a second client should see processes started by the first"
    );

    let _ = second.request(Request::Stop {
        id: "shared".to_string(),
    });
}

/// Regression: the daemon used to `handle()` each request inline in the
/// connection's read loop, so one slow request blocked every request behind
/// it on that connection. `Stop` is slow by construction — it waits up to
/// GRACEFUL_TIMEOUT (3s) for the process to die before escalating to
/// SIGKILL, and `Restart` pays that twice. Meanwhile the UI polls resource
/// stats every couple of seconds and re-syncs statuses whenever the popover
/// takes focus, so a single restart was enough to queue those past the
/// client's 10s request timeout: the app looked hung and reported
/// DAEMON_TIMEOUT.
#[test]
fn a_slow_request_does_not_block_the_ones_behind_it() {
    let dir = scratch_dir().join("head-of-line");
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("daemon.sock");
    let _daemon = start_daemon(&socket, &dir);

    let mut app = Client::connect_within(&socket, Duration::from_secs(10));

    // Ignores SIGTERM in the process-group leader, so `Stop` has to sit out
    // the full graceful timeout and escalate to SIGKILL. The inner `sleep`
    // children do die on TERM; the loop just starts another one. `READY` is
    // printed *after* the trap is installed, so it doubles as the handshake
    // for "the ignore is in effect now" — stopping before that lands leaves
    // the shell with a default disposition and the stop finishes in
    // microseconds, quietly making this whole test vacuous.
    let mut project = probe_project("stubborn", "stubborn-marker");
    project.command = "trap '' TERM; echo READY; while :; do sleep 1; done".to_string();

    assert!(matches!(
        app.request(Request::Start {
            project: Box::new(project),
        }),
        ResponseBody::Ok
    ));

    let pid = wait_for(Duration::from_secs(10), || {
        match app.request(Request::ListStatuses) {
            ResponseBody::Statuses { statuses } => statuses
                .iter()
                .find(|s| s.id == "stubborn")
                .and_then(|s| s.pid),
            other => panic!("expected statuses, got {other:?}"),
        }
    })
    .expect("the project never reported a pid");

    wait_for(Duration::from_secs(10), || {
        match app.request(Request::GetOutput {
            id: "stubborn".to_string(),
        }) {
            ResponseBody::Output { text } if text.contains("READY") => Some(()),
            _ => None,
        }
    })
    .expect("the shell never reported that its SIGTERM trap was installed");

    // Both go out before either answer comes back, so the daemon genuinely
    // has to choose an order rather than being handed one.
    let started = Instant::now();
    let stop_id = app.send(Request::Stop {
        id: "stubborn".to_string(),
    });
    let ping_id = app.send(Request::Ping);

    let (first_id, _) = app.next_response();
    let ping_took = started.elapsed();

    assert_eq!(
        first_id, ping_id,
        "the Ping (id {ping_id}) should have been answered while the Stop \
         (id {stop_id}) was still waiting out its graceful timeout, but the \
         Stop answered first — requests are being serialized per connection \
         again"
    );
    assert!(
        ping_took < Duration::from_secs(2),
        "the Ping took {ping_took:?}, which is long enough to have waited on \
         the Stop's 3s graceful timeout"
    );

    // And the slow one still answers, on its own schedule.
    let (second_id, _) = app.next_response();
    assert_eq!(second_id, stop_id, "the Stop should answer too, just later");
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "the Stop came back in {:?} — it never hit the graceful timeout, so \
         this test proved nothing about slow requests",
        started.elapsed()
    );

    assert!(
        wait_for(Duration::from_secs(10), || (!process_is_alive(pid))
            .then_some(()))
        .is_some(),
        "the process should be gone once the Stop escalates to SIGKILL"
    );
}

fn process_is_alive(pid: u32) -> bool {
    // Signal 0 checks for the process's existence without touching it.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}
