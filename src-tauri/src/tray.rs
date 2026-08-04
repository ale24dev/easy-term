//! Keeps the menu-bar tray icon's title and context menu in sync with the
//! current set of projects and their live status.

use crate::process_manager::{ProcessManager, ProjectStatus};
use crate::project_store::ProjectStore;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    AppHandle, Manager,
};

pub const TRAY_ID: &str = "main";

fn status_glyph(status: ProjectStatus) -> &'static str {
    match status {
        ProjectStatus::Running => "🟢",
        ProjectStatus::Starting => "🟡",
        ProjectStatus::Crashed => "🔴",
        ProjectStatus::Stopped => "⚪",
    }
}

/// Recomputes the tray's title (aggregate status) and rebuilds its context
/// menu. Called after every status change and every project list change —
/// cheap enough (a handful of menu items) to just rebuild rather than diff.
pub fn refresh(app: &AppHandle) {
    let manager = app.state::<ProcessManager>();
    let statuses = manager.snapshot_statuses();

    let running = statuses
        .values()
        .filter(|s| matches!(s, ProjectStatus::Running | ProjectStatus::Starting))
        .count();
    let any_crashed = statuses.values().any(|s| *s == ProjectStatus::Crashed);

    let title = if any_crashed {
        Some("🔴".to_string())
    } else if running > 0 {
        Some(format!("🟢 {running}"))
    } else {
        None
    };

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_title(title);
    }

    rebuild_menu(app, &statuses);
}

fn rebuild_menu(app: &AppHandle, statuses: &std::collections::HashMap<String, ProjectStatus>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    let store = app.state::<ProjectStore>();
    let projects = store.list();

    let project_items: Vec<MenuItem<tauri::Wry>> = projects
        .iter()
        .filter_map(|project| {
            let status = statuses
                .get(&project.id)
                .copied()
                .unwrap_or(ProjectStatus::Stopped);
            let label = format!("{} {}", status_glyph(status), project.name);
            MenuItem::with_id(
                app,
                format!("toggle:{}", project.id),
                label,
                true,
                None::<&str>,
            )
            .ok()
        })
        .collect();

    let mut refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = Vec::new();
    for item in &project_items {
        refs.push(item);
    }

    let separator = PredefinedMenuItem::separator(app).ok();
    if let Some(sep) = &separator {
        refs.push(sep);
    }

    let quit_item = MenuItem::with_id(app, "quit", "Quit easy-term", true, None::<&str>).ok();
    if let Some(quit) = &quit_item {
        refs.push(quit);
    }

    if let Ok(menu) = Menu::with_items(app, &refs) {
        let _ = tray.set_menu(Some(menu));
    }
}
