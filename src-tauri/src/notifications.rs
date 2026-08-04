//! Native crash notifications. The project id travels in the notification's
//! `extra` payload so the frontend's `onAction` listener can jump straight
//! to that project's logs when the user clicks it.

use crate::error_logger::{log_error, Level, Source};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

const MODULE: &str = "notifications";

/// Requests notification permission once, early, so the OS prompt doesn't
/// surprise the user the first time a project actually crashes.
pub fn init(app: &AppHandle) {
    if let Err(e) = app.notification().request_permission() {
        log_error(
            Level::Warn,
            Source::Backend,
            MODULE,
            "NOTIFY_PERMISSION_FAILED",
            format!("No se pudo solicitar permiso de notificaciones: {e}"),
            None,
            None,
        );
    }
}

pub fn notify_crash(app: &AppHandle, project_id: &str, project_name: &str, code: i32) {
    let result = app
        .notification()
        .builder()
        .title(format!("{project_name} se detuvo"))
        .body(format!("Terminó inesperadamente (código {code})"))
        .extra("projectId", project_id)
        .show();

    if let Err(e) = result {
        log_error(
            Level::Warn,
            Source::Backend,
            MODULE,
            "NOTIFY_SHOW_FAILED",
            format!("No se pudo mostrar la notificación de crash: {e}"),
            Some(serde_json::json!({ "projectId": project_id })),
            None,
        );
    }
}
