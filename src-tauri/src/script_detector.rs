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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_package_manager_defaults_to_npm_with_no_lockfile() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_package_manager(dir.path()), "npm");
    }

    #[test]
    fn detect_package_manager_reads_pnpm_lockfile() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_package_manager(dir.path()), "pnpm");
    }

    #[test]
    fn detect_package_manager_reads_yarn_lockfile() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("yarn.lock"), "").unwrap();
        assert_eq!(detect_package_manager(dir.path()), "yarn");
    }

    #[test]
    fn detect_package_manager_reads_either_bun_lockfile_spelling() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bun.lock"), "").unwrap();
        assert_eq!(detect_package_manager(dir.path()), "bun");

        let dir2 = tempfile::tempdir().unwrap();
        fs::write(dir2.path().join("bun.lockb"), "").unwrap();
        assert_eq!(detect_package_manager(dir2.path()), "bun");
    }

    #[test]
    fn detect_package_manager_prefers_pnpm_when_multiple_lockfiles_exist() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        fs::write(dir.path().join("yarn.lock"), "").unwrap();
        assert_eq!(detect_package_manager(dir.path()), "pnpm");
    }

    #[test]
    fn detect_scripts_without_a_package_json_returns_empty_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = detect_scripts(dir.path().to_str().unwrap().to_string()).unwrap();

        assert_eq!(config.name, None);
        assert_eq!(config.package_manager, "npm");
        assert!(config.scripts.is_empty());
    }

    #[test]
    fn detect_scripts_parses_name_and_scripts_with_package_manager_prefix() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name": "my-app", "scripts": {"dev": "vite", "build": "vite build"}}"#,
        )
        .unwrap();

        let config = detect_scripts(dir.path().to_str().unwrap().to_string()).unwrap();

        assert_eq!(config.name, Some("my-app".to_string()));
        assert_eq!(config.package_manager, "pnpm");
        assert_eq!(config.scripts.len(), 2);
        let dev = config.scripts.iter().find(|s| s.name == "dev").unwrap();
        assert_eq!(dev.command, "pnpm run dev");
    }

    #[test]
    fn detect_scripts_ignores_non_string_script_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts": {"dev": "vite", "weird": {"nested": true}}}"#,
        )
        .unwrap();

        let config = detect_scripts(dir.path().to_str().unwrap().to_string()).unwrap();

        assert_eq!(config.scripts.len(), 1);
        assert_eq!(config.scripts[0].name, "dev");
    }

    #[test]
    fn detect_scripts_tolerates_a_package_json_with_no_scripts_field() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("package.json"), r#"{"name": "no-scripts"}"#).unwrap();

        let config = detect_scripts(dir.path().to_str().unwrap().to_string()).unwrap();

        assert_eq!(config.name, Some("no-scripts".to_string()));
        assert!(config.scripts.is_empty());
    }

    #[test]
    fn detect_scripts_rejects_malformed_package_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{ not json").unwrap();

        let err = detect_scripts(dir.path().to_str().unwrap().to_string()).unwrap_err();
        assert_eq!(err.code, "DETECT_PKG_JSON_INVALID");
    }
}
