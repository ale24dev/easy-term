//! Tauri command handlers. Thin wrappers over `project_store` and
//! `process_manager` — no business logic lives here.

use crate::env_resolver;
use crate::error_logger::AppError;
use crate::process_manager::{self, ProcessManager};
use crate::project_store::{Group, Project, ProjectStore};
use crate::resource_monitor::{self, ProcessStats};
use crate::tray;
use std::collections::HashMap;
use std::process::Command;
use tauri::{AppHandle, Manager, State};

fn find_project(store: &ProjectStore, id: &str) -> Result<Project, AppError> {
    store.get(id).ok_or_else(|| {
        AppError::new(
            "commands",
            "STORE_PROJECT_NOT_FOUND",
            format!("Proyecto no encontrado: {id}"),
        )
    })
}

#[tauri::command]
pub fn list_projects(store: State<ProjectStore>) -> Vec<Project> {
    store.list()
}

#[tauri::command]
pub fn save_project(
    app: AppHandle,
    store: State<ProjectStore>,
    project: Project,
) -> Result<Project, AppError> {
    let saved = store.save(project)?;
    store.sync_all_group_membership()?;
    tray::refresh(&app);
    Ok(saved)
}

#[tauri::command]
pub fn delete_project(
    app: AppHandle,
    store: State<ProjectStore>,
    id: String,
) -> Result<(), AppError> {
    // Best-effort stop first — deleting a running project shouldn't leave an
    // orphaned process nobody can reach from the UI anymore.
    let _ = process_manager::stop(&app, &id);
    store.delete(&id)?;
    app.state::<ProcessManager>().forget(&id);
    tray::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn start_project(
    app: AppHandle,
    store: State<ProjectStore>,
    id: String,
) -> Result<(), AppError> {
    let project = find_project(&store, &id)?;
    process_manager::start(&app, project)
}

#[tauri::command]
pub fn stop_project(app: AppHandle, id: String) -> Result<(), AppError> {
    process_manager::stop(&app, &id)
}

#[tauri::command]
pub fn restart_project(
    app: AppHandle,
    store: State<ProjectStore>,
    id: String,
) -> Result<(), AppError> {
    let project = find_project(&store, &id)?;
    process_manager::restart(&app, project)
}

#[tauri::command]
pub fn get_process_output(app: AppHandle, id: String) -> String {
    process_manager::get_output(&app, &id)
}

#[tauri::command]
pub fn get_error_count(manager: State<ProcessManager>, id: String) -> u32 {
    manager.error_count(&id)
}

#[tauri::command]
pub fn reset_error_count(manager: State<ProcessManager>, id: String) {
    manager.reset_error_count(&id);
}

#[tauri::command]
pub fn list_groups(store: State<ProjectStore>) -> Vec<Group> {
    store.list_groups()
}

#[tauri::command]
pub fn find_or_create_group(store: State<ProjectStore>, name: String) -> Result<Group, AppError> {
    store.find_or_create_group(&name)
}

#[tauri::command]
pub fn start_group(
    app: AppHandle,
    store: State<ProjectStore>,
    group_id: String,
) -> Result<(), AppError> {
    let group = store.get_group(&group_id).ok_or_else(|| {
        AppError::new(
            "commands",
            "STORE_PROJECT_NOT_FOUND",
            format!("Grupo no encontrado: {group_id}"),
        )
    })?;

    let projects: Vec<Project> = group
        .project_ids
        .iter()
        .filter_map(|id| store.get(id))
        .collect();

    process_manager::start_group(&app, projects);
    Ok(())
}

#[tauri::command]
pub fn stop_group(
    app: AppHandle,
    store: State<ProjectStore>,
    group_id: String,
) -> Result<(), AppError> {
    let group = store.get_group(&group_id).ok_or_else(|| {
        AppError::new(
            "commands",
            "STORE_PROJECT_NOT_FOUND",
            format!("Grupo no encontrado: {group_id}"),
        )
    })?;

    process_manager::stop_group(&app, group.project_ids);
    Ok(())
}

#[tauri::command]
pub fn get_project_stats(manager: State<ProcessManager>, id: String) -> Option<ProcessStats> {
    let pid = manager.pid_of(&id)?;
    resource_monitor::stats_for_group(pid).ok()
}

#[tauri::command]
pub fn open_in_editor(path: String) -> Result<(), AppError> {
    let overrides = env_resolver::overrides(&HashMap::new());

    for editor in ["cursor", "code"] {
        let mut cmd = Command::new(editor);
        cmd.arg(&path);
        for (key, value) in &overrides {
            cmd.env(key, value);
        }
        if cmd.spawn().is_ok() {
            return Ok(());
        }
    }

    Err(AppError::new(
        "commands",
        "EDITOR_NOT_FOUND",
        "No se encontró \"cursor\" ni \"code\" en el PATH",
    ))
}
