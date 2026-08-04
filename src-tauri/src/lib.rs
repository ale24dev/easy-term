mod commands;
mod env_resolver;
mod error_logger;
mod notifications;
mod port_checker;
mod process_manager;
mod project_store;
mod script_detector;
mod tray;

use process_manager::{ProcessManager, ProjectStatus};
use project_store::ProjectStore;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Runtime, WebviewWindow, WindowEvent,
};
use tauri_plugin_positioner::{Position, WindowExt};

fn toggle_popover<R: Runtime>(window: &WebviewWindow<R>) {
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.move_window_constrained(Position::TrayCenter);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(ProjectStore::load())
        .manage(ProcessManager::new())
        .setup(|app| {
            error_logger::init(app.package_info().version.to_string(), std::env::consts::OS);
            env_resolver::init();
            notifications::init(app.handle());

            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            TrayIconBuilder::with_id(tray::TRAY_ID)
                .icon(app.default_window_icon().unwrap().clone())
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let id = event.id().as_ref();
                    if id == "quit" {
                        app.exit(0);
                        return;
                    }

                    if let Some(project_id) = id.strip_prefix("toggle:") {
                        let app = app.clone();
                        let project_id = project_id.to_string();
                        // start()/stop() can block briefly (stop waits up to
                        // 3s for a graceful exit) — never do that on the
                        // menu event callback.
                        std::thread::spawn(move || {
                            let manager = app.state::<ProcessManager>();
                            let running = matches!(
                                manager.status_of(&project_id),
                                ProjectStatus::Running | ProjectStatus::Starting
                            );
                            if running {
                                let _ = process_manager::stop(&app, &project_id);
                            } else {
                                let store = app.state::<ProjectStore>();
                                if let Some(project) = store.get(&project_id) {
                                    let _ = process_manager::start(&app, project);
                                }
                            }
                        });
                    }
                })
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

            // Populates the tray menu (project list + Quit) now that the
            // icon exists; subsequent refreshes happen on every status/
            // project change.
            tray::refresh(app.handle());

            if let Some(window) = app.get_webview_window("main") {
                let hide_on_blur = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let _ = hide_on_blur.hide();
                    }
                });
            }

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
