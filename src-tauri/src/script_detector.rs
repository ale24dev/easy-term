//! Detects runnable scripts and the package manager for a project folder,
//! so `ProjectForm` can offer a script picker instead of a blank command box.

use crate::error_logger::AppError;
use serde::Serialize;
use std::fs;
use std::path::Path;

const MODULE: &str = "script_detector";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedScript {
    pub name: String,
    pub command: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedConfig {
    pub name: Option<String>,
    pub package_manager: String,
    pub scripts: Vec<DetectedScript>,
}

fn detect_package_manager(dir: &Path) -> &'static str {
    if dir.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if dir.join("bun.lockb").exists() || dir.join("bun.lock").exists() {
        "bun"
    } else if dir.join("yarn.lock").exists() {
        "yarn"
    } else {
        "npm"
    }
}

#[tauri::command]
pub fn detect_scripts(path: String) -> Result<DetectedConfig, AppError> {
    let dir = Path::new(&path);
    let package_manager = detect_package_manager(dir).to_string();
    let pkg_path = dir.join("package.json");

    let content = match fs::read_to_string(&pkg_path) {
        Ok(c) => c,
        // No package.json is a normal case (non-Node project) — the form
        // just falls back to a manually-typed command, not an error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DetectedConfig {
                name: None,
                package_manager,
                scripts: Vec::new(),
            });
        }
        Err(e) => {
            return Err(AppError::new(
                MODULE,
                "DETECT_IO_ERROR",
                format!("No se pudo leer package.json: {e}"),
            ))
        }
    };

    let json: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
        AppError::new(
            MODULE,
            "DETECT_PKG_JSON_INVALID",
            format!("package.json inválido: {e}"),
        )
    })?;

    let name = json
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let scripts = json
        .get("scripts")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter(|(_, cmd)| cmd.is_string())
                .map(|(name, _)| DetectedScript {
                    command: format!("{package_manager} run {name}"),
                    name: name.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(DetectedConfig {
        name,
        package_manager,
        scripts,
    })
}
