//! macOS-only: makes the popover reachable while another app is full
//! screen, by converting it from a regular `NSWindow` into a
//! non-activating `NSPanel`.
//!
//! ## Why the previous attempts (collectionBehavior + window level) failed
//!
//! Root cause, confirmed by reading tao's show/focus path and every working
//! open-source implementation of this exact feature:
//!
//! - tao's `set_visible(true)` calls `makeKeyAndOrderFront`, and
//!   `set_focus()` additionally calls `activateIgnoringOtherApps:YES`
//!   (tao 0.35, `platform_impl/macos/util/async.rs`).
//! - A regular `NSWindow` can only become key while its app is *active*.
//!   Activating this (Accessory) app while another app owns the frontmost
//!   full-screen Space makes macOS attempt a focus/Space transition and
//!   then bounce activation straight back to the full-screen app — that
//!   bounce is the reported "the fullscreen app flickers like it gains and
//!   loses focus" symptom. Our window either never orders in on that Space
//!   or holds key for a moment and loses it, after which the hide-on-blur
//!   handler hid it before it was ever seen ("nothing happens").
//! - So `NSWindowCollectionBehaviorFullScreenAuxiliary` + a high window
//!   level were necessary but not sufficient: the *show path itself* was
//!   the problem.
//!
//! The macOS-native answer — used by Spotlight and by every Swift menu-bar
//! app checked (NotchDrop, NotesOllama, Telegram's PIP, …) — is an
//! `NSPanel` with `NSWindowStyleMaskNonActivatingPanel`: such a panel can
//! become key **without activating the app**, so the full-screen Space is
//! never disturbed. Tauri doesn't expose panels (tauri-apps/tao#136 is the
//! open feature request; tauri-apps/tauri#5793 was closed as not planned),
//! so this uses the community-standard `tauri-nspanel` crate (also by the
//! author of the canonical `tauri-macos-menubar-app-example`), which
//! swizzles the existing window's Objective-C class to an `NSPanel`
//! subclass. Configuration below mirrors that crate's own `fullscreen`
//! example and the menubar example's `swizzle_to_menubar_panel`.
//!
//! Side effect handled here: `set_delegate` replaces the NSWindowDelegate
//! tao had installed, so Tauri's `WindowEvent::Focused` events stop firing
//! for this window. The panel delegate below takes over both jobs that
//! relied on them: hide-on-blur (via `window_did_resign_key`, still honoring
//! `SuppressAutoHide` for the folder-picker sheet) and the frontend's
//! focus-gated resource polling (via `easyterm://panel-focus` events).

use crate::error_logger::{log_error, Level, Source};
use crate::SuppressAutoHide;
use objc2_app_kit::NSWindow;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};
use tauri_nspanel::{
    cocoa::appkit::{NSMainMenuWindowLevel, NSWindowCollectionBehavior},
    objc::{class, msg_send, sel, sel_impl},
    panel_delegate, ManagerExt, WebviewWindowExt,
};

const MODULE: &str = "macos_window";

#[allow(non_upper_case_globals)]
const NSWindowStyleMaskNonActivatingPanel: i32 = 1 << 7;

/// Converts the main window into a non-activating menu-bar panel. Called
/// once from `setup()`. On failure the window just stays a regular
/// NSWindow and `toggle_popover` falls back to the old show/hide path.
pub fn convert_to_menubar_panel(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        log_error(
            Level::Error,
            Source::Backend,
            MODULE,
            "PANEL_CONVERT_FAILED",
            "main window not found during panel conversion",
            None,
            None,
        );
        return;
    };

    let panel = match window.to_panel() {
        Ok(panel) => panel,
        Err(e) => {
            log_error(
                Level::Error,
                Source::Backend,
                MODULE,
                "PANEL_CONVERT_FAILED",
                format!("to_panel() failed: {e}"),
                None,
                None,
            );
            return;
        }
    };

    let delegate = panel_delegate!(EasyTermPanelDelegate {
        window_did_become_key,
        window_did_resign_key
    });

    let handle = app.clone();
    delegate.set_listener(Box::new(move |delegate_name: String| {
        match delegate_name.as_str() {
            "window_did_become_key" => {
                let _ = handle.emit("easyterm://panel-focus", true);
            }
            "window_did_resign_key" => {
                let _ = handle.emit("easyterm://panel-focus", false);
                hide_panel_on_resign_key(&handle);
            }
            _ => {}
        }
    }));

    // NSMainMenuWindowLevel + 1 == the status-bar tier; same value the
    // canonical menubar example uses so the panel paints over a
    // full-screen app's own content.
    panel.set_level(NSMainMenuWindowLevel + 1);

    // FullScreenAuxiliary lets the panel join a full-screen app's Space;
    // CanJoinAllSpaces + Stationary keep it available and pinned on every
    // other Space.
    panel.set_collection_behaviour(
        NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorStationary
            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary,
    );

    // The key piece: the panel can become key without activating the app,
    // which is what regular NSWindows can't do and why every show attempt
    // over a full-screen Space bounced.
    panel.set_style_mask(NSWindowStyleMaskNonActivatingPanel);

    panel.set_delegate(delegate);
}

/// The panel-world replacement for the old tao hide-on-blur handler.
fn hide_panel_on_resign_key(app: &AppHandle) {
    // Same guard as before: a native dialog (folder picker sheet) takes key
    // status from the panel; hiding the panel would tear the sheet down.
    let suppress = app.state::<SuppressAutoHide>();
    if suppress.0.load(Ordering::SeqCst) {
        log_error(
            Level::Warn,
            Source::Backend,
            "lib",
            "BLUR_SUPPRESSED",
            "panel resigned key but hide was suppressed (native dialog open)",
            None,
            None,
        );
        return;
    }

    // Belt and suspenders for the same case: if our own app is the
    // frontmost app right now (e.g. an NSOpenPanel just activated us), the
    // resign-key isn't the user clicking away — don't hide. Mirrors
    // `check_menubar_frontmost` in the canonical menubar example.
    if app_is_frontmost() {
        log_error(
            Level::Warn,
            Source::Backend,
            "lib",
            "BLUR_SUPPRESSED",
            "panel resigned key while app is frontmost (native dialog) — not hiding",
            None,
            None,
        );
        return;
    }

    if let Ok(panel) = app.get_webview_panel("main") {
        log_error(
            Level::Warn,
            Source::Backend,
            "lib",
            "BLUR_HID_WINDOW",
            "panel resigned key and was hidden",
            None,
            None,
        );
        panel.order_out(None);
    }
}

fn app_is_frontmost() -> bool {
    use tauri_nspanel::cocoa::base::id;

    let our_pid = std::process::id() as i32;
    let frontmost_pid: i32 = unsafe {
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let frontmost_app: id = msg_send![workspace, frontmostApplication];
        if frontmost_app.is_null() {
            return false;
        }
        msg_send![frontmost_app, processIdentifier]
    };
    frontmost_pid == our_pid
}

/// # Safety
/// The returned reference borrows the `NSWindow` Tauri owns for `window`;
/// callers must not retain or release it, and must not use it past the
/// scope of the call that produced the raw pointer. (The object is a
/// swizzled NSPanel subclass by the time this runs, but every property read
/// here is inherited unchanged from NSWindow.)
unsafe fn as_ns_window<R: Runtime>(window: &WebviewWindow<R>) -> Option<&NSWindow> {
    let ns_window_ptr = window.ns_window().ok()?;
    if ns_window_ptr.is_null() {
        return None;
    }
    Some(unsafe { &*ns_window_ptr.cast::<NSWindow>() })
}

/// Writes a snapshot of the window's actual AppKit state to the diagnostics
/// log (Settings → Diagnóstico shows it; the JSONL file has it either way).
/// Called right after each show so a repro produces facts, not a yes/no.
pub fn log_window_diagnostics<R: Runtime>(window: &WebviewWindow<R>) {
    // SAFETY: see `as_ns_window` — used only within this call.
    let Some(ns_window) = (unsafe { as_ns_window(window) }) else {
        log_error(
            Level::Warn,
            Source::Backend,
            MODULE,
            "TRAY_WINDOW_DIAG_NO_NSWINDOW",
            "ns_window() returned null or an error right after show()",
            None,
            None,
        );
        return;
    };

    let frame = ns_window.frame();
    log_error(
        Level::Warn,
        Source::Backend,
        MODULE,
        "TRAY_WINDOW_DIAG",
        "Window state snapshot right after show()",
        Some(serde_json::json!({
            "isVisible": ns_window.isVisible(),
            "isOnActiveSpace": ns_window.isOnActiveSpace(),
            "isKeyWindow": ns_window.isKeyWindow(),
            "isMiniaturized": ns_window.isMiniaturized(),
            "occlusionState": ns_window.occlusionState().0,
            "level": ns_window.level(),
            "collectionBehavior": ns_window.collectionBehavior().0,
            "alphaValue": ns_window.alphaValue(),
            "frame": {
                "x": frame.origin.x,
                "y": frame.origin.y,
                "width": frame.size.width,
                "height": frame.size.height,
            },
        })),
        None,
    );
}
