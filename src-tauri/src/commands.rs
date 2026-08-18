//! Tauri command handlers. Thin wrappers over `project_store` and
//! `process_manager` — no business logic lives here.

use crate::env_resolver;
use crate::error_logger::AppError;
use crate::process_manager::{self, Context};
use crate::project_store::{Group, Project, ProjectStore};
use crate::resource_monitor::{self, ProcessStats};
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
pub fn list_projects(ctx: State<Context>) -> Vec<Project> {
    ctx.store.list()
}

#[tauri::command]
pub fn save_project(
    app: AppHandle,
    ctx: State<Context>,
    project: Project,
) -> Result<Project, AppError> {
    let saved = ctx.store.save(project)?;
    ctx.store.sync_all_group_membership()?;
    tray::refresh(&app);
    Ok(saved)
}

#[tauri::command]
pub fn delete_project(app: AppHandle, ctx: State<Context>, id: String) -> Result<(), AppError> {
    // Best-effort stop first — deleting a running project shouldn't leave an
    // orphaned process nobody can reach from the UI anymore.
    let _ = process_manager::stop(&ctx, &id);
    ctx.store.delete(&id)?;
    ctx.manager.forget(&id);
    tray::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn start_project(ctx: State<Context>, id: String) -> Result<(), AppError> {
    let project = find_project(&ctx.store, &id)?;
    process_manager::start(&ctx, project)
}

#[tauri::command]
pub fn stop_project(ctx: State<Context>, id: String) -> Result<(), AppError> {
    process_manager::stop(&ctx, &id)
}

#[tauri::command]
pub fn restart_project(ctx: State<Context>, id: String) -> Result<(), AppError> {
    let project = find_project(&ctx.store, &id)?;
    process_manager::restart(&ctx, project)
}

#[tauri::command]
pub fn get_process_output(ctx: State<Context>, id: String) -> String {
    process_manager::get_output(&ctx, &id)
}

#[tauri::command]
pub fn list_process_statuses(ctx: State<Context>) -> Vec<process_manager::StatusPayload> {
    ctx.manager.snapshot_all()
}

#[tauri::command]
pub fn get_error_count(ctx: State<Context>, id: String) -> u32 {
    ctx.manager.error_count(&id)
}

#[tauri::command]
pub fn reset_error_count(ctx: State<Context>, id: String) {
    ctx.manager.reset_error_count(&id);
}

#[tauri::command]
pub fn list_groups(ctx: State<Context>) -> Vec<Group> {
    ctx.store.list_groups()
}

#[tauri::command]
pub fn find_or_create_group(ctx: State<Context>, name: String) -> Result<Group, AppError> {
    let store = &ctx.store;
    store.find_or_create_group(&name)
}

#[tauri::command]
pub fn toggle_project_pin(ctx: State<Context>, id: String) -> Result<Project, AppError> {
    let store = &ctx.store;
    store.toggle_project_pin(&id)
}

#[tauri::command]
pub fn toggle_group_pin(ctx: State<Context>, id: String) -> Result<Group, AppError> {
    let store = &ctx.store;
    store.toggle_group_pin(&id)
}

#[tauri::command]
pub fn start_group(ctx: State<Context>, group_id: String) -> Result<(), AppError> {
    let store = &ctx.store;
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

    process_manager::start_group(&ctx, projects);
    Ok(())
}

#[tauri::command]
pub fn stop_group(ctx: State<Context>, group_id: String) -> Result<(), AppError> {
    let store = &ctx.store;
    let group = store.get_group(&group_id).ok_or_else(|| {
        AppError::new(
            "commands",
            "STORE_PROJECT_NOT_FOUND",
            format!("Group not found: {group_id}"),
        )
    })?;

    process_manager::stop_group(&ctx, group.project_ids);
    Ok(())
}

#[tauri::command]
pub fn get_project_stats(ctx: State<Context>, id: String) -> Option<ProcessStats> {
    let manager = &ctx.manager;
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
