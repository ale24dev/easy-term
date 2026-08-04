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
                format!("No se pudo ejecutar lsof: {e}"),
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
                    format!("No se pudo matar el proceso {pid} en el puerto {port}: {err}"),
                ));
            }
        }
    }

    Ok(())
}
