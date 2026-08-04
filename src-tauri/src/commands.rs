//! Tauri command handlers. Thin wrappers over `project_store` and
//! `process_manager` — no business logic lives here.

use crate::error_logger::AppError;
use crate::process_manager;
use crate::project_store::{Project, ProjectStore};
use tauri::{AppHandle, State};

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
pub fn save_project(store: State<ProjectStore>, project: Project) -> Result<Project, AppError> {
    store.save(project)
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
    store.delete(&id)
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
