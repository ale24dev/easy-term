//! Keeps the menu-bar tray icon's title and tooltip in sync with the
//! current set of projects and their live status.
//!
//! Deliberately never attaches a native context menu to this tray icon: on
//! macOS, `NSStatusItem.setMenu()` makes AppKit show that menu on *every*
//! click (left or right) once one is attached, regardless of
//! `show_menu_on_left_click` — a long-standing, unresolved upstream bug
//! (tauri-apps/tauri#4002). With a menu attached, left-click stopped
//! opening the popover entirely. Instead: left-click always toggles the
//! popover (see `lib.rs`), "Quit" lives in the popover itself, and
//! per-project status is surfaced via the tray tooltip, which has no such
//! click side effect.

use crate::process_manager::{ProcessManager, ProjectStatus};
use crate::project_store::ProjectStore;
use std::collections::HashMap;
use tauri::{AppHandle, Manager};

pub const TRAY_ID: &str = "main";

fn status_glyph(status: ProjectStatus) -> &'static str {
    match status {
        ProjectStatus::Running => "🟢",
        ProjectStatus::Starting => "🟡",
        ProjectStatus::Crashed => "🔴",
        ProjectStatus::Stopped => "⚪",
    }
}

/// Recomputes the tray's title (aggregate status) and tooltip (per-project
/// breakdown). Called after every status change and every project change.
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

    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let _ = tray.set_title(title);
    let _ = tray.set_tooltip(Some(build_tooltip(app, &statuses)));
}

fn build_tooltip(app: &AppHandle, statuses: &HashMap<String, ProjectStatus>) -> String {
    let store = app.state::<ProjectStore>();
    let projects = store.list();

    if projects.is_empty() {
        return "easy-term".to_string();
    }

    projects
        .iter()
        .map(|project| {
            let status = statuses
                .get(&project.id)
                .copied()
                .unwrap_or(ProjectStatus::Stopped);
            format!("{} {}", status_glyph(status), project.name)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
