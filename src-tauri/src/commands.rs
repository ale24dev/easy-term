//! Tauri command handlers. Thin wrappers over `project_store` and
//! `process_manager` — no business logic lives here.

use crate::daemon::client::DaemonClient;
use crate::daemon::protocol::{Request, ResponseBody};
use crate::env_resolver;
use crate::error_logger::AppError;
use crate::process_manager::StatusPayload;
use crate::project_store::{Group, Project, ProjectStore};
use crate::resource_monitor::ProcessStats;
use crate::tray;
use std::collections::HashMap;
use std::process::Command;
use tauri::{AppHandle, State};

fn find_project(store: &ProjectStore, id: &str) -> Result<Project, AppError> {
    store.get(id).ok_or_else(|| {
        AppError::new(
            "commands",
            "STORE_PROJECT_NOT_FOUND",
            format!("Project not found: {id}"),
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
    daemon: State<DaemonClient>,
    id: String,
) -> Result<(), AppError> {
    // Best-effort stop first — deleting a running project shouldn't leave an
    // orphaned process nobody can reach from the UI anymore.
    let _ = daemon.call(Request::Stop { id: id.clone() });
    store.delete(&id)?;
    let _ = daemon.call(Request::Forget { id: id.clone() });
    tray::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn start_project(
    store: State<ProjectStore>,
    daemon: State<DaemonClient>,
    id: String,
) -> Result<(), AppError> {
    let project = find_project(&store, &id)?;
    daemon
        .call(Request::Start {
            project: Box::new(project),
        })
        .map(|_| ())
}

#[tauri::command]
pub fn stop_project(daemon: State<DaemonClient>, id: String) -> Result<(), AppError> {
    daemon.call(Request::Stop { id }).map(|_| ())
}

#[tauri::command]
pub fn restart_project(
    store: State<ProjectStore>,
    daemon: State<DaemonClient>,
    id: String,
) -> Result<(), AppError> {
    let project = find_project(&store, &id)?;
    daemon
        .call(Request::Restart {
            project: Box::new(project),
        })
        .map(|_| ())
}

#[tauri::command]
pub fn get_process_output(daemon: State<DaemonClient>, id: String) -> String {
    match daemon.call(Request::GetOutput { id }) {
        Ok(ResponseBody::Output { text }) => text,
        _ => String::new(),
    }
}

#[tauri::command]
pub fn list_process_statuses(daemon: State<DaemonClient>) -> Vec<StatusPayload> {
    match daemon.call(Request::ListStatuses) {
        Ok(ResponseBody::Statuses { statuses }) => statuses,
        _ => Vec::new(),
    }
}

#[tauri::command]
pub fn get_error_count(daemon: State<DaemonClient>, id: String) -> u32 {
    match daemon.call(Request::ErrorCount { id }) {
        Ok(ResponseBody::Count { count }) => count,
        _ => 0,
    }
}

#[tauri::command]
pub fn reset_error_count(daemon: State<DaemonClient>, id: String) {
    let _ = daemon.call(Request::ResetErrorCount { id });
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
pub fn toggle_project_pin(store: State<ProjectStore>, id: String) -> Result<Project, AppError> {
    store.toggle_project_pin(&id)
}

#[tauri::command]
pub fn toggle_group_pin(store: State<ProjectStore>, id: String) -> Result<Group, AppError> {
    store.toggle_group_pin(&id)
}

#[tauri::command]
pub fn start_group(
    store: State<ProjectStore>,
    daemon: State<DaemonClient>,
    group_id: String,
) -> Result<(), AppError> {
    let group = store.get_group(&group_id).ok_or_else(|| {
        AppError::new(
            "commands",
            "STORE_PROJECT_NOT_FOUND",
            format!("Group not found: {group_id}"),
        )
    })?;

    let projects: Vec<Project> = group
        .project_ids
        .iter()
        .filter_map(|id| store.get(id))
        .collect();

    daemon.call(Request::StartGroup { projects }).map(|_| ())
}

#[tauri::command]
pub fn stop_group(
    store: State<ProjectStore>,
    daemon: State<DaemonClient>,
    group_id: String,
) -> Result<(), AppError> {
    let group = store.get_group(&group_id).ok_or_else(|| {
        AppError::new(
            "commands",
            "STORE_PROJECT_NOT_FOUND",
            format!("Group not found: {group_id}"),
        )
    })?;

    daemon
        .call(Request::StopGroup {
            project_ids: group.project_ids,
        })
        .map(|_| ())
}

#[tauri::command]
pub fn get_project_stats(daemon: State<DaemonClient>, id: String) -> Option<ProcessStats> {
    match daemon.call(Request::GetStats { id }) {
        Ok(ResponseBody::Stats { stats }) => stats,
        _ => None,
    }
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
        "Could not find \"cursor\" or \"code\" on PATH",
    ))
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    crate::quit(&app);
}

/// See `SuppressAutoHide` in `lib.rs`: the frontend calls this right before
/// opening a native dialog so losing focus to it doesn't hide (and thereby
/// close) the sheet attached to this window.
#[tauri::command]
pub fn begin_native_dialog(suppress: State<crate::SuppressAutoHide>) {
    suppress.0.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
pub fn end_native_dialog(suppress: State<crate::SuppressAutoHide>) {
    suppress.0.store(false, std::sync::atomic::Ordering::SeqCst);
}
