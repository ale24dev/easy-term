//! Tauri command handlers. Thin wrappers over `project_store` and
//! `process_manager` — no business logic lives here.

use crate::error_logger::AppError;
use crate::process_manager::{self, ProcessManager};
use crate::project_store::{Project, ProjectStore};
use crate::tray;
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
