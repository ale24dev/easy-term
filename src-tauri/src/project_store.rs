//! CRUD and JSON persistence for the user's projects.

use crate::error_logger::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

const MODULE: &str = "project_store";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub path: String,
    pub command: String,
    pub port: Option<u16>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default)]
    pub group_id: Option<String>,
}

pub struct ProjectStore {
    inner: Mutex<Vec<Project>>,
}

impl ProjectStore {
    pub fn load() -> Self {
        let projects = match read_from_disk() {
            Ok(projects) => projects,
            Err(err) => {
                err.emit();
                Vec::new()
            }
        };
        Self {
            inner: Mutex::new(projects),
        }
    }

    pub fn list(&self) -> Vec<Project> {
        self.inner.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Project> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }

    pub fn save(&self, mut project: Project) -> Result<Project, AppError> {
        if project.id.is_empty() {
            project.id = uuid::Uuid::new_v4().to_string();
        }

        let mut guard = self.inner.lock().unwrap();
        match guard.iter_mut().find(|p| p.id == project.id) {
            Some(existing) => *existing = project.clone(),
            None => guard.push(project.clone()),
        }
        persist(&guard)?;
        Ok(project)
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        let mut guard = self.inner.lock().unwrap();
        guard.retain(|p| p.id != id);
        persist(&guard)
    }
}

fn store_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("easy-term")
        .join("projects.json")
}

fn read_from_disk() -> Result<Vec<Project>, AppError> {
    let path = store_path();
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&path).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_READ_FAILED",
            format!("No se pudo leer projects.json: {e}"),
        )
    })?;

    serde_json::from_str(&content).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_PARSE_ERROR",
            format!("projects.json está corrupto: {e}"),
        )
    })
}

/// Atomic write: write to a temp file in the same directory, then rename —
/// a crash mid-write can never leave `projects.json` truncated or corrupt.
fn persist(projects: &[Project]) -> Result<(), AppError> {
    let path = store_path();
    let dir = path.parent().expect("store path always has a parent");

    fs::create_dir_all(dir).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_WRITE_FAILED",
            format!("No se pudo crear el directorio de datos: {e}"),
        )
    })?;

    let json = serde_json::to_string_pretty(projects).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_WRITE_FAILED",
            format!("No se pudo serializar los proyectos: {e}"),
        )
    })?;

    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, json).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_WRITE_FAILED",
            format!("No se pudo escribir el archivo temporal: {e}"),
        )
    })?;

    fs::rename(&tmp_path, &path).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_WRITE_FAILED",
            format!("No se pudo confirmar la escritura de projects.json: {e}"),
        )
    })
}
