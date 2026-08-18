mod commands;
pub mod daemon;
mod env_resolver;
mod error_logger;
#[cfg(target_os = "macos")]
mod macos_window;
mod notifications;
mod popover;
mod port_checker;
mod process_manager;
pub mod project_store;
mod resource_monitor;
mod script_detector;
mod tauri_sink;
mod tray;

use popover::Rect;
use project_store::ProjectStore;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, WebviewWindow, WindowEvent,
};
use tauri_plugin_autostart::MacosLauncher;

/// The GUI's read-only copy of what the daemon reports about each project.
///
/// The tray needs an aggregate status on every change, and the daemon is the
/// only one who knows it — but asking over the socket from inside an event
/// callback would block the reader thread on its own reply. So the sink
/// mirrors each Status event here as it passes, and the tray reads locally.
pub(crate) struct StatusMirror(Mutex<HashMap<String, process_manager::ProjectStatus>>);

impl StatusMirror {
    pub fn snapshot(&self) -> HashMap<String, process_manager::ProjectStatus> {
        self.0.lock().map(|m| m.clone()).unwrap_or_default()
    }

    pub fn set(&self, id: String, status: process_manager::ProjectStatus) {
        if let Ok(mut m) = self.0.lock() {
            m.insert(id, status);
        }
    }

    pub fn replace_all(&self, statuses: HashMap<String, process_manager::ProjectStatus>) {
        if let Ok(mut m) = self.0.lock() {
            *m = statuses;
        }
    }
}

/// The tray icon's on-screen rect, refreshed on every tray event.
///
/// Only the x and width are ever read — see `popover.rs` for why the y that
/// comes with it can't be trusted on a multi-monitor setup.
pub(crate) struct TrayRect(pub Mutex<Option<Rect>>);

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
    // The window is a non-activating NSPanel on macOS (see macos_window.rs)
    // and must be shown through the panel API: tao's show()/set_focus() go
    // through makeKeyAndOrderFront + activateIgnoringOtherApps, and
    // activating the app is exactly what a full-screen Space rejects —
    // that bounce was the whole bug. panel.show() orders the panel front
    // and makes it key *without* activating the app.
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::ManagerExt;
        if let Ok(panel) = window.get_webview_panel("main") {
            if panel.is_visible() {
                panel.order_out(None);
            } else {
                reposition_under_tray(window);
                panel.show();
                macos_window::log_window_diagnostics(window);
            }
            return;
        }
        // Panel conversion failed at setup (already logged) — fall back to
        // the plain-window path below.
    }

    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        reposition_under_tray(window);
        let _ = window.show();
        let _ = window.set_focus();

        #[cfg(target_os = "macos")]
        macos_window::log_window_diagnostics(window);
    }
}

/// Moves the popover under the tray icon, if a tray rect has been seen.
///
/// Showing the window at a stale position is still better than not showing
/// it, so a failure here only logs — but it *does* log, which the previous
/// `let _ = move_window_constrained(...)` did not, and that silence is what
/// made the multi-monitor breakage look like "the app just won't open".
fn reposition_under_tray<R: Runtime>(window: &WebviewWindow<R>) {
    let tray = window
        .app_handle()
        .state::<TrayRect>()
        .0
        .lock()
        .ok()
        .and_then(|rect| *rect);

    match tray {
        Some(tray) => {
            popover::position_under_tray(window, tray);
        }
        None => error_logger::log_error(
            error_logger::Level::Warn,
            error_logger::Source::Backend,
            "lib",
            "POPOVER_NO_TRAY_RECT",
            "No tray rect recorded yet; showing the popover at its last position",
            None,
            None,
        ),
    }
}

/// Snapshots which projects are live so the next launch can restore them
/// (best-effort: only covers a clean Quit, not a force-kill), then exits.
/// Reachable only from the popover UI — see `tray.rs` for why there's no
/// native tray menu item for this.
pub(crate) fn quit(app: &AppHandle) {
    // Nothing to tear down or remember: the daemon keeps the processes
    // running, and the next launch simply reconnects and finds them. The old
    // `wasRunning` snapshot-and-restore existed only because quitting used
    // to kill everything.
    app.exit(0);
}

/// Entry point for `easy-term --daemon`: serves the socket forever.
pub fn run_daemon() -> Result<(), String> {
    error_logger::init(env!("CARGO_PKG_VERSION").to_string(), std::env::consts::OS);
    env_resolver::init();
    daemon::server::run()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ));

    // Registers the panel store `to_panel()`/`get_webview_panel()` use —
    // see macos_window.rs for why the popover must be an NSPanel.
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    builder
        .manage(SuppressAutoHide(AtomicBool::new(false)))
        .manage(TrayRect(Mutex::new(None)))
        .setup(|app| {
            app.manage(ProjectStore::load());
            app.manage(StatusMirror(Mutex::new(HashMap::new())));

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
                    // Every tray event carries the icon's current rect; keep
                    // the latest so the popover can be centered under it even
                    // when the icon shifts (other menu bar items coming and
                    // going move it around).
                    if let TrayIconEvent::Click { rect, .. }
                    | TrayIconEvent::Enter { rect, .. }
                    | TrayIconEvent::Move { rect, .. }
                    | TrayIconEvent::Leave { rect, .. } = &event
                    {
                        // tray-icon already reports physical units, so the
                        // scale factor here is a no-op conversion.
                        let position = rect.position.to_physical::<f64>(1.0);
                        let size = rect.size.to_physical::<f64>(1.0);
                        let seen = Rect {
                            x: position.x,
                            y: position.y,
                            width: size.width,
                            height: size.height,
                        };
                        if let Ok(mut stored) =
                            tray.app_handle().state::<TrayRect>().0.lock()
                        {
                            *stored = Some(seen);
                        }
                    }

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

            // Swizzle the popover into a non-activating NSPanel so it can
            // open over a full-screen app's Space (see macos_window.rs).
            #[cfg(target_os = "macos")]
            macos_window::convert_to_menubar_panel(app.handle());

            if let Some(window) = app.get_webview_window("main") {
                // On macOS this handler goes quiet once the panel delegate
                // is installed above (the swizzle replaces the
                // NSWindowDelegate tao events flow through) — the
                // equivalent hide-on-resign-key logic lives in
                // macos_window.rs. Kept for other platforms and for the
                // fallback path when panel conversion fails.
                let hide_on_blur = window.clone();
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let suppress = app_handle.state::<SuppressAutoHide>();
                        if suppress.0.load(Ordering::SeqCst) {
                            error_logger::log_error(
                                error_logger::Level::Warn,
                                error_logger::Source::Backend,
                                "lib",
                                "BLUR_SUPPRESSED",
                                "Focused(false) fired but was suppressed (dialog open or just shown)",
                                None,
                                None,
                            );
                        } else {
                            error_logger::log_error(
                                error_logger::Level::Warn,
                                error_logger::Source::Backend,
                                "lib",
                                "BLUR_HID_WINDOW",
                                "Focused(false) fired and hid the window",
                                None,
                                None,
                            );
                            let _ = hide_on_blur.hide();
                        }
                    }
                });
            }

            // Connect to the daemon that owns the processes, starting one if
            // this is the first launch since a reboot. Its event stream is
            // replayed through the same sink the in-process path used, so the
            // frontend sees identical `process:*` events either way.
            let sink = tauri_sink::TauriSink::new(app.handle().clone());
            let connected = daemon::client::DaemonClient::connect_or_spawn(move |event| {
                use crate::process_manager::EventSink;
                sink.emit(event);
            });

            let client = match connected {
                Ok(client) => {
                    // Seed from the daemon before the first paint: projects
                    // it kept running while the app was closed have already
                    // emitted their Status events, long before we connected.
                    if let Ok(daemon::protocol::ResponseBody::Statuses { statuses }) =
                        client.call(daemon::protocol::Request::ListStatuses)
                    {
                        app.state::<StatusMirror>().replace_all(
                            statuses.into_iter().map(|s| (s.id, s.status)).collect(),
                        );
                    }
                    Some(client)
                }
                Err(e) => {
                    // Without a daemon the app still opens and can browse and
                    // edit projects; every process command reports the failure
                    // instead of the window coming up dead.
                    e.emit();
                    None
                }
            };
            // Managed unconditionally: an unmanaged type makes Tauri's
            // `state()` panic, which would turn "no daemon" into a crash.
            app.manage(daemon::client::Daemon(client));
            tray::refresh(app.handle());

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
            commands::list_process_statuses,
            commands::get_error_count,
            commands::reset_error_count,
            commands::list_groups,
            commands::find_or_create_group,
            commands::toggle_project_pin,
            commands::toggle_group_pin,
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
