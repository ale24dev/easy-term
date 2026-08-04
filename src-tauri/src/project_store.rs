//! CRUD and JSON persistence for the user's projects and groups. Both
//! collections live in one file, written together atomically.

use crate::error_logger::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    /// Set right before a clean Quit for every project that was running at
    /// the time, so the next launch can restore it; consumed (cleared) as
    /// soon as it's read at startup.
    #[serde(default)]
    pub was_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// Start order for "start all" — projects launch one at a time in this
    /// sequence, each waited on for readiness before the next begins.
    #[serde(default)]
    pub project_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoreData {
    #[serde(default)]
    projects: Vec<Project>,
    #[serde(default)]
    groups: Vec<Group>,
}

pub struct ProjectStore {
    inner: Mutex<StoreData>,
}

impl ProjectStore {
    pub fn load() -> Self {
        let data = match read_from_disk() {
            Ok(data) => data,
            Err(err) => {
                err.emit();
                StoreData::default()
            }
        };
        Self {
            inner: Mutex::new(data),
        }
    }

    pub fn list(&self) -> Vec<Project> {
        self.inner.lock().unwrap().projects.clone()
    }

    pub fn get(&self, id: &str) -> Option<Project> {
        self.inner
            .lock()
            .unwrap()
            .projects
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }

    pub fn save(&self, mut project: Project) -> Result<Project, AppError> {
        if project.id.is_empty() {
            project.id = uuid::Uuid::new_v4().to_string();
        }

        let mut guard = self.inner.lock().unwrap();
        match guard.projects.iter_mut().find(|p| p.id == project.id) {
            Some(existing) => *existing = project.clone(),
            None => guard.projects.push(project.clone()),
        }
        persist(&guard)?;
        Ok(project)
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        let mut guard = self.inner.lock().unwrap();
        guard.projects.retain(|p| p.id != id);
        for group in &mut guard.groups {
            group.project_ids.retain(|pid| pid != id);
        }
        persist(&guard)
    }

    pub fn list_groups(&self) -> Vec<Group> {
        self.inner.lock().unwrap().groups.clone()
    }

    pub fn get_group(&self, id: &str) -> Option<Group> {
        self.inner
            .lock()
            .unwrap()
            .groups
            .iter()
            .find(|g| g.id == id)
            .cloned()
    }

    /// Finds a group by case-insensitive name, or creates it. Lets the UI
    /// stay a plain text field ("Grupo: backend") instead of needing a
    /// separate group-management screen.
    pub fn find_or_create_group(&self, name: &str) -> Result<Group, AppError> {
        let mut guard = self.inner.lock().unwrap();

        if let Some(existing) = guard
            .groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(name))
        {
            return Ok(existing.clone());
        }

        let group = Group {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            project_ids: Vec::new(),
        };
        guard.groups.push(group.clone());
        persist(&guard)?;
        Ok(group)
    }

    /// Keeps every group's `project_ids` in sync with which projects
    /// currently point at it via `group_id` — called after every project
    /// save/delete (cheap: at most a handful of groups) so "start all"
    /// always reflects live membership, including a project that just
    /// moved from one group to another.
    pub fn sync_all_group_membership(&self) -> Result<(), AppError> {
        let mut guard = self.inner.lock().unwrap();

        // Snapshot first: mutating `guard.groups` while still borrowing
        // `guard.projects` doesn't split-borrow through a MutexGuard deref.
        let project_group_ids: Vec<(String, Option<String>)> = guard
            .projects
            .iter()
            .map(|p| (p.id.clone(), p.group_id.clone()))
            .collect();

        for group in &mut guard.groups {
            let member_ids: Vec<String> = project_group_ids
                .iter()
                .filter(|(_, gid)| gid.as_deref() == Some(group.id.as_str()))
                .map(|(pid, _)| pid.clone())
                .collect();

            group.project_ids.retain(|id| member_ids.contains(id));
            for id in member_ids {
                if !group.project_ids.contains(&id) {
                    group.project_ids.push(id);
                }
            }
        }

        persist(&guard)
    }

    /// Marks exactly the given projects as `wasRunning`, clearing the flag
    /// on everyone else — called once, right before a clean Quit.
    pub fn set_was_running(&self, running_ids: &HashSet<String>) -> Result<(), AppError> {
        let mut guard = self.inner.lock().unwrap();
        for project in &mut guard.projects {
            project.was_running = running_ids.contains(&project.id);
        }
        persist(&guard)
    }

    /// Returns the projects flagged `wasRunning` and immediately clears the
    /// flag for all of them — restoring is a one-shot action per launch.
    pub fn take_was_running(&self) -> Result<Vec<Project>, AppError> {
        let mut guard = self.inner.lock().unwrap();
        let restored: Vec<Project> = guard
            .projects
            .iter()
            .filter(|p| p.was_running)
            .cloned()
            .collect();

        if restored.is_empty() {
            return Ok(restored);
        }

        for project in &mut guard.projects {
            project.was_running = false;
        }
        persist(&guard)?;
        Ok(restored)
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

fn read_from_disk() -> Result<StoreData, AppError> {
    let path = store_path();
    if !path.exists() {
        return Ok(StoreData::default());
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
fn persist(data: &StoreData) -> Result<(), AppError> {
    let path = store_path();
    let dir = path.parent().expect("store path always has a parent");

    fs::create_dir_all(dir).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_WRITE_FAILED",
            format!("No se pudo crear el directorio de datos: {e}"),
        )
    })?;

    let json = serde_json::to_string_pretty(data).map_err(|e| {
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
