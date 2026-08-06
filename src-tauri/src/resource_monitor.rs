//! CPU/RAM usage for a project's whole process tree, via `ps`.
//!
//! Rather than pull in a cross-platform crate like `sysinfo` (which doesn't
//! expose process-group membership uniformly), this shells out to `ps` and
//! sums every row sharing our tracked pid as its process group id — cheap,
//! always available on macOS, and consistent with `port_checker`'s approach.

use crate::error_logger::AppError;
use serde::Serialize;
use std::process::Command;

const MODULE: &str = "resource_monitor";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStats {
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

pub fn stats_for_group(pgid: i32) -> Result<ProcessStats, AppError> {
    let output = Command::new("ps")
        .args(["-Ao", "pgid,pcpu,rss"])
        .output()
        .map_err(|e| {
            AppError::new(
                MODULE,
                "RESOURCE_STATS_FAILED",
                format!("Could not run ps: {e}"),
            )
        })?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut cpu_percent = 0.0f32;
    let mut memory_kb = 0u64;

    // Skip the header row `ps` prints.
    for line in text.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let row_pgid: Option<i32> = fields.next().and_then(|s| s.parse().ok());
        let row_cpu: Option<f32> = fields.next().and_then(|s| s.parse().ok());
        let row_rss: Option<u64> = fields.next().and_then(|s| s.parse().ok());

        if row_pgid == Some(pgid) {
            cpu_percent += row_cpu.unwrap_or(0.0);
            memory_kb += row_rss.unwrap_or(0);
        }
    }

    Ok(ProcessStats {
        cpu_percent,
        memory_bytes: memory_kb * 1024,
    })
}
