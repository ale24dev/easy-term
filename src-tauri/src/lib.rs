mod commands;
mod env_resolver;
mod error_logger;
#[cfg(target_os = "macos")]
mod macos_window;
mod notifications;
mod port_checker;
mod process_manager;
mod project_store;
mod resource_monitor;
mod script_detector;
mod tray;

use process_manager::{ProcessManager, ProjectStatus};
use project_store::ProjectStore;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, WebviewWindow, WindowEvent,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::ShortcutState;
use tauri_plugin_positioner::{Position, WindowExt};

/// Set while a native dialog (e.g. the folder picker) is on screen.
///
/// On macOS, `tauri-plugin-dialog` presents its panel as a sheet attached to
/// the popover window. Opening it moves focus away from that window, which
/// would normally trigger the hide-on-blur handler below — hiding the
/// window out from under its own sheet closes the sheet immediately. The
/// frontend brackets its `open()` call with the `begin_native_dialog`/
/// `end_native_dialog` commands (see `commands.rs`) to suppress that.
pub(crate) struct SuppressAutoHide(pub AtomicBool);

fn toggle_popover<R: Runtime>(window: &WebviewWindow<R>) {
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.move_window_constrained(Position::TrayCenter);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Snapshots which projects are live so the next launch can restore them
/// (best-effort: only covers a clean Quit, not a force-kill), then exits.
/// Reachable only from the popover UI — see `tray.rs` for why there's no
/// native tray menu item for this.
pub(crate) fn quit(app: &AppHandle) {
    let manager = app.state::<ProcessManager>();
    let store = app.state::<ProjectStore>();
    let running_ids: HashSet<String> = manager
        .snapshot_statuses()
        .into_iter()
        .filter(|(_, status)| matches!(status, ProjectStatus::Running | ProjectStatus::Starting))
        .map(|(id, _)| id)
        .collect();
    if let Err(e) = store.set_was_running(&running_ids) {
        e.emit();
    }
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let global_shortcut_plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_shortcut("Alt+Space")
        .expect("invalid global shortcut definition")
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                if let Some(window) = app.get_webview_window("main") {
                    toggle_popover(&window);
                }
            }
        })
        .build();

    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(global_shortcut_plugin)
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(ProjectStore::load())
        .manage(ProcessManager::new())
        .manage(SuppressAutoHide(AtomicBool::new(false)))
        .setup(|app| {
            error_logger::init(app.package_info().version.to_string(), std::env::consts::OS);
            env_resolver::init();
            notifications::init(app.handle());

            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // No `.menu(...)` here on purpose — see tray.rs's module comment.
            // Any attached menu makes macOS show it on every click, not just
            // right-click, which breaks left-click-to-toggle entirely.
            TrayIconBuilder::with_id(tray::TRAY_ID)
                .icon(app.default_window_icon().unwrap().clone())
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);

                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            toggle_popover(&window);
                        }
                    }
                })
                .build(app)?;

            // Sets the initial title/tooltip now that the icon exists;
            // subsequent refreshes happen on every status/project change.
            tray::refresh(app.handle());

            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                macos_window::allow_join_fullscreen_space(&window);

                let hide_on_blur = window.clone();
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let suppress = app_handle.state::<SuppressAutoHide>();
                        if !suppress.0.load(Ordering::SeqCst) {
                            let _ = hide_on_blur.hide();
                        }
                    }
                });
            }

            // Restore whatever was flagged `wasRunning` at the last clean
            // Quit. Runs on its own thread so a handful of dev servers
            // starting up can't delay the app from appearing.
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                let store = app_handle.state::<ProjectStore>();
                match store.take_was_running() {
                    Ok(projects) => {
                        for project in projects {
                            let _ = process_manager::start(&app_handle, project);
                        }
                    }
                    Err(e) => e.emit(),
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::save_project,
            commands::delete_project,
            commands::start_project,
            commands::stop_project,
            commands::restart_project,
            commands::get_process_output,
            commands::get_error_count,
            commands::reset_error_count,
            commands::list_groups,
            commands::find_or_create_group,
            commands::start_group,
            commands::stop_group,
            commands::get_project_stats,
            commands::open_in_editor,
            commands::quit_app,
            commands::begin_native_dialog,
            commands::end_native_dialog,
            script_detector::detect_scripts,
            port_checker::check_port,
            port_checker::kill_port_owner,
            error_logger::log_app_error,
            error_logger::read_error_log,
            error_logger::open_logs_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
