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
    /// User-chosen accent color (CSS color string), shown as a stripe next
    /// to the project's name in the list. `None` means no accent.
    #[serde(default)]
    pub color: Option<String>,
    /// Pinned projects sort above unpinned ones within their list (their
    /// group's members, or the ungrouped section).
    #[serde(default)]
    pub pinned: bool,
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
    /// Pinned groups sort above unpinned ones in the project list.
    #[serde(default)]
    pub pinned: bool,
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
    path: PathBuf,
}

impl ProjectStore {
    pub fn load() -> Self {
        Self::load_from(store_path())
    }

    /// Split out from `load()` so tests can point the store at a tempdir
    /// instead of the real `~/Library/Application Support/easy-term`.
    fn load_from(path: PathBuf) -> Self {
        let data = match read_from_disk(&path) {
            Ok(data) => data,
            Err(err) => {
                err.emit();
                StoreData::default()
            }
        };
        Self {
            inner: Mutex::new(data),
            path,
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
        persist(&self.path, &guard)?;
        Ok(project)
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        let mut guard = self.inner.lock().unwrap();
        guard.projects.retain(|p| p.id != id);
        for group in &mut guard.groups {
            group.project_ids.retain(|pid| pid != id);
        }
        persist(&self.path, &guard)
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
            pinned: false,
        };
        guard.groups.push(group.clone());
        persist(&self.path, &guard)?;
        Ok(group)
    }

    pub fn toggle_project_pin(&self, id: &str) -> Result<Project, AppError> {
        let mut guard = self.inner.lock().unwrap();
        let project = guard
            .projects
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| {
                AppError::new(
                    MODULE,
                    "STORE_PROJECT_NOT_FOUND",
                    format!("Project not found: {id}"),
                )
            })?;
        project.pinned = !project.pinned;
        let updated = project.clone();
        persist(&self.path, &guard)?;
        Ok(updated)
    }

    pub fn toggle_group_pin(&self, id: &str) -> Result<Group, AppError> {
        let mut guard = self.inner.lock().unwrap();
        let group = guard
            .groups
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or_else(|| {
                AppError::new(
                    MODULE,
                    "STORE_GROUP_NOT_FOUND",
                    format!("Group not found: {id}"),
                )
            })?;
        group.pinned = !group.pinned;
        let updated = group.clone();
        persist(&self.path, &guard)?;
        Ok(updated)
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

        persist(&self.path, &guard)
    }

    /// Marks exactly the given projects as `wasRunning`, clearing the flag
    /// on everyone else — called once, right before a clean Quit.
    pub fn set_was_running(&self, running_ids: &HashSet<String>) -> Result<(), AppError> {
        let mut guard = self.inner.lock().unwrap();
        for project in &mut guard.projects {
            project.was_running = running_ids.contains(&project.id);
        }
        persist(&self.path, &guard)
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
        persist(&self.path, &guard)?;
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

fn read_from_disk(path: &PathBuf) -> Result<StoreData, AppError> {
    if !path.exists() {
        return Ok(StoreData::default());
    }

    let content = fs::read_to_string(path).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_READ_FAILED",
            format!("Could not read projects.json: {e}"),
        )
    })?;

    serde_json::from_str(&content).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_PARSE_ERROR",
            format!("projects.json is corrupt: {e}"),
        )
    })
}

/// Atomic write: write to a temp file in the same directory, then rename —
/// a crash mid-write can never leave `projects.json` truncated or corrupt.
fn persist(path: &PathBuf, data: &StoreData) -> Result<(), AppError> {
    let dir = path.parent().expect("store path always has a parent");

    fs::create_dir_all(dir).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_WRITE_FAILED",
            format!("Could not create the data directory: {e}"),
        )
    })?;

    let json = serde_json::to_string_pretty(data).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_WRITE_FAILED",
            format!("Could not serialize projects: {e}"),
        )
    })?;

    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, json).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_WRITE_FAILED",
            format!("Could not write the temp file: {e}"),
        )
    })?;

    fs::rename(&tmp_path, path).map_err(|e| {
        AppError::new(
            MODULE,
            "STORE_WRITE_FAILED",
            format!("Could not commit the write to projects.json: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_project(name: &str) -> Project {
        Project {
            id: String::new(),
            name: name.to_string(),
            path: format!("/tmp/{name}"),
            command: "pnpm run dev".to_string(),
            port: None,
            env: HashMap::new(),
            auto_restart: false,
            group_id: None,
            was_running: false,
            color: None,
            pinned: false,
        }
    }

    fn store_at(dir: &tempfile::TempDir) -> ProjectStore {
        ProjectStore::load_from(dir.path().join("projects.json"))
    }

    #[test]
    fn save_assigns_an_id_and_list_returns_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir);

        let saved = store.save(blank_project("api")).unwrap();
        assert!(!saved.id.is_empty());

        let listed = store.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, saved.id);
        assert_eq!(listed[0].name, "api");
    }

    #[test]
    fn save_with_existing_id_updates_in_place_instead_of_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir);

        let saved = store.save(blank_project("api")).unwrap();
        let mut edited = saved.clone();
        edited.command = "pnpm run dev --port 4000".to_string();
        store.save(edited).unwrap();

        let listed = store.list();
        assert_eq!(listed.len(), 1, "editing must not create a second project");
        assert_eq!(listed[0].command, "pnpm run dev --port 4000");
    }

    #[test]
    fn get_returns_none_for_unknown_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir);
        assert!(store.get("does-not-exist").is_none());
    }

    #[test]
    fn delete_removes_the_project_and_its_group_membership() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir);

        let group = store.find_or_create_group("backend").unwrap();
        let mut project = blank_project("api");
        project.group_id = Some(group.id.clone());
        let saved = store.save(project).unwrap();
        store.sync_all_group_membership().unwrap();

        assert!(store
            .get_group(&group.id)
            .unwrap()
            .project_ids
            .contains(&saved.id));

        store.delete(&saved.id).unwrap();

        assert!(store.get(&saved.id).is_none());
        assert!(!store
            .get_group(&group.id)
            .unwrap()
            .project_ids
            .contains(&saved.id));
    }

    #[test]
    fn persistence_survives_a_reload_from_the_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.json");

        let saved_id = {
            let store = ProjectStore::load_from(path.clone());
            store.save(blank_project("api")).unwrap().id
        };

        let reloaded = ProjectStore::load_from(path);
        let listed = reloaded.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, saved_id);
    }

    #[test]
    fn a_corrupt_store_file_loads_as_empty_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.json");
        fs::write(&path, "{ this is not valid json").unwrap();

        let store = ProjectStore::load_from(path);
        assert!(store.list().is_empty());
        assert!(store.list_groups().is_empty());
    }

    #[test]
    fn find_or_create_group_is_case_insensitive_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir);

        let first = store.find_or_create_group("Backend").unwrap();
        let second = store.find_or_create_group("backend").unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(store.list_groups().len(), 1);
    }

    #[test]
    fn sync_all_group_membership_follows_a_project_moving_between_groups() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir);

        let backend = store.find_or_create_group("backend").unwrap();
        let frontend = store.find_or_create_group("frontend").unwrap();

        let mut project = blank_project("api");
        project.group_id = Some(backend.id.clone());
        let saved = store.save(project).unwrap();
        store.sync_all_group_membership().unwrap();

        assert!(store
            .get_group(&backend.id)
            .unwrap()
            .project_ids
            .contains(&saved.id));

        let mut moved = saved.clone();
        moved.group_id = Some(frontend.id.clone());
        store.save(moved).unwrap();
        store.sync_all_group_membership().unwrap();

        assert!(!store
            .get_group(&backend.id)
            .unwrap()
            .project_ids
            .contains(&saved.id));
        assert!(store
            .get_group(&frontend.id)
            .unwrap()
            .project_ids
            .contains(&saved.id));
    }

    #[test]
    fn take_was_running_returns_exactly_the_flagged_projects_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir);

        let a = store.save(blank_project("a")).unwrap();
        let b = store.save(blank_project("b")).unwrap();
        store.save(blank_project("c")).unwrap();

        let running: HashSet<String> = [a.id.clone(), b.id.clone()].into_iter().collect();
        store.set_was_running(&running).unwrap();

        let restored = store.take_was_running().unwrap();
        let restored_ids: HashSet<String> = restored.iter().map(|p| p.id.clone()).collect();
        assert_eq!(restored_ids, running);

        // Consumed: a second call must come back empty.
        assert!(store.take_was_running().unwrap().is_empty());
    }

    #[test]
    fn toggle_project_pin_flips_the_flag_and_persists_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir);
        let saved = store.save(blank_project("api")).unwrap();
        assert!(!saved.pinned);

        let pinned = store.toggle_project_pin(&saved.id).unwrap();
        assert!(pinned.pinned);
        assert!(store.get(&saved.id).unwrap().pinned);

        let unpinned = store.toggle_project_pin(&saved.id).unwrap();
        assert!(!unpinned.pinned);
    }

    #[test]
    fn toggle_project_pin_on_an_unknown_id_returns_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir);
        assert!(store.toggle_project_pin("does-not-exist").is_err());
    }

    #[test]
    fn toggle_group_pin_flips_the_flag_and_persists_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir);
        let group = store.find_or_create_group("backend").unwrap();
        assert!(!group.pinned);

        let pinned = store.toggle_group_pin(&group.id).unwrap();
        assert!(pinned.pinned);
        assert!(store.get_group(&group.id).unwrap().pinned);
    }
}
