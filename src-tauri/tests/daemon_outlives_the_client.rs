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

    /// Sends a request and returns the response, skipping over any events
    /// that arrive in the meantime.
    fn request(&mut self, request: Request) -> ResponseBody {
        let id = self.next_id;
        self.next_id += 1;

        let mut line = serde_json::to_string(&Envelope { id, request }).unwrap();
        line.push('\n');
        self.stream.write_all(line.as_bytes()).unwrap();
        self.stream.flush().unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let mut buf = String::new();
            if self.reader.read_line(&mut buf).unwrap_or(0) == 0 {
                panic!("the daemon closed the connection while waiting for a response");
            }
            match serde_json::from_str::<Message>(buf.trim()) {
                Ok(Message::Response { id: got, body }) if got == id => return body,
                Ok(_) => continue, // an event, or a reply to something else
                Err(e) => panic!("could not parse {buf:?}: {e}"),
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

fn process_is_alive(pid: u32) -> bool {
    // Signal 0 checks for the process's existence without touching it.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}
