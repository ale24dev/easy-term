//! Checks whether a TCP port is free and, if not, who owns it — so a start
//! attempt that would fail with EADDRINUSE can instead offer "free it and
//! start" in one click.

use crate::error_logger::AppError;
use serde::Serialize;
use std::process::Command;
use std::time::Duration;

const MODULE: &str = "port_checker";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortOwner {
    pub pid: i32,
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortCheckResult {
    pub free: bool,
    pub owner: Option<PortOwner>,
}

fn list_pids_on_port(port: u16) -> Result<Vec<i32>, AppError> {
    let output = Command::new("lsof")
        .arg("-nP")
        .arg(format!("-iTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .arg("-t")
        .output()
        .map_err(|e| {
            AppError::new(
                MODULE,
                "PORT_CHECK_FAILED",
                format!("Could not run lsof: {e}"),
            )
        })?;

    // lsof exits with status 1 when nothing matches — that means the port is
    // free, not a failure, so we don't check output.status here.
    let pids = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();

    Ok(pids)
}

fn process_name(pid: i32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return None;
    }

    Some(raw.rsplit('/').next().unwrap_or(&raw).to_string())
}

#[tauri::command]
pub fn check_port(port: u16) -> Result<PortCheckResult, AppError> {
    let pids = list_pids_on_port(port)?;
    let Some(&pid) = pids.first() else {
        return Ok(PortCheckResult {
            free: true,
            owner: None,
        });
    };

    let name = process_name(pid).unwrap_or_else(|| "proceso desconocido".to_string());
    Ok(PortCheckResult {
        free: false,
        owner: Some(PortOwner { pid, name }),
    })
}

#[tauri::command]
pub fn kill_port_owner(port: u16) -> Result<(), AppError> {
    let pids = list_pids_on_port(port)?;
    if pids.is_empty() {
        return Ok(());
    }

    for pid in &pids {
        unsafe { libc::kill(*pid, libc::SIGTERM) };
    }

    std::thread::sleep(Duration::from_millis(500));

    for pid in list_pids_on_port(port)? {
        let result = unsafe { libc::kill(pid, libc::SIGKILL) };
        if result != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::ESRCH) {
                return Err(AppError::new(
                    MODULE,
                    "PORT_KILL_OWNER_FAILED",
                    format!("Could not kill process {pid} on port {port}: {err}"),
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::process::Child;
    use std::time::Instant;

    /// Kills the wrapped child on drop, so a failing assertion mid-test
    /// can't leak a listening process into the rest of the test run.
    struct KillOnDrop(Child);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn free_port() -> u16 {
        // Bind to port 0 to let the OS hand back an unused ephemeral port,
        // then release it immediately — nothing ever connected, so there's
        // no TIME_WAIT state to make it linger as "busy".
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if condition() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn spawn_listener(port: u16) -> KillOnDrop {
        let child = Command::new("python3")
            .args([
                "-m",
                "http.server",
                &port.to_string(),
                "--bind",
                "127.0.0.1",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("failed to spawn python3 http.server for the test");
        KillOnDrop(child)
    }

    #[test]
    fn check_port_reports_free_for_an_unused_port() {
        let port = free_port();
        let result = check_port(port).unwrap();
        assert!(result.free);
        assert!(result.owner.is_none());
    }

    #[test]
    fn check_port_reports_the_owner_of_a_listening_process() {
        let port = free_port();
        let guard = spawn_listener(port);

        let became_busy = wait_until(Duration::from_secs(5), || {
            check_port(port).map(|r| !r.free).unwrap_or(false)
        });
        assert!(
            became_busy,
            "listener never showed up as busy in check_port"
        );

        let result = check_port(port).unwrap();
        assert!(!result.free);
        let owner = result.owner.expect("busy port must report an owner");
        assert_eq!(owner.pid, guard.0.id() as i32);
    }

    #[test]
    fn kill_port_owner_frees_the_port_and_stops_the_process() {
        let port = free_port();
        let mut guard = spawn_listener(port);

        wait_until(Duration::from_secs(5), || {
            check_port(port).map(|r| !r.free).unwrap_or(false)
        });

        kill_port_owner(port).unwrap();

        let became_free = wait_until(Duration::from_secs(5), || {
            check_port(port).map(|r| r.free).unwrap_or(false)
        });
        assert!(
            became_free,
            "port was still reported busy after kill_port_owner"
        );

        let exited = wait_until(Duration::from_secs(2), || {
            matches!(guard.0.try_wait(), Ok(Some(_)))
        });
        assert!(
            exited,
            "listening process was still alive after kill_port_owner"
        );
    }

    #[test]
    fn kill_port_owner_on_an_already_free_port_is_a_no_op() {
        let port = free_port();
        kill_port_owner(port).unwrap();
    }
}
