//! macOS-only: lets the popover window display over a currently full-screen
//! app's Space, instead of the tray click silently doing nothing.
//!
//! Two things are needed, confirmed by cross-checking real, working
//! open-source menu-bar/overlay apps (NotchDrop, Lunar, Ice-style panels,
//! and others) that all do both together:
//!
//! 1. **Collection behavior.** By default an `NSWindow` can only appear on
//!    the Space it was shown on; `visible_on_all_workspaces` (which Tauri
//!    does expose) adds it to every *regular* Space, but that still
//!    excludes a Space a full-screen app currently occupies — full-screen
//!    apps get an exclusive Space of their own.
//!    `NSWindowCollectionBehaviorFullScreenAuxiliary` opts in to exactly
//!    that case.
//!
//! 2. **Window level.** Tauri's `alwaysOnTop` maps to `NSFloatingWindowLevel`
//!    (3) — well below where a full-screen space's own compositing sits.
//!    Every real-world example of a menu-bar-anchored popover doing this
//!    correctly raises the window to `NSStatusWindowLevel` (25), the same
//!    tier the system's own menu bar/status items render at.
//!
//! Tauri/tao expose neither knob for this combination, so both are set
//! directly on the raw `NSWindow` Tauri already owns.
//!
//! Reported still broken after both of the above, with no visible effect at
//! all when the tray icon is clicked while another app is full screen. That
//! can't be diagnosed further by reading AppKit source — it needs to be
//! observed on a real Mac. `log_window_diagnostics` (called right after
//! `show()`) dumps exactly the properties in play — `isVisible`,
//! `isOnActiveSpace`, `occlusionState`, `level`, `collectionBehavior`, and
//! the window's actual on-screen `frame` — into the same diagnostics log the
//! Settings view already shows, so the next test produces facts about what
//! AppKit actually did instead of just a yes/no.

use crate::error_logger::{log_error, Level, Source};
use objc2_app_kit::{NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior};
use tauri::{Runtime, WebviewWindow};

const MODULE: &str = "macos_window";

/// # Safety
/// The returned reference borrows the `NSWindow` Tauri owns for `window`;
/// callers must not retain or release it, and must not use it past the
/// scope of the call that produced the raw pointer.
unsafe fn as_ns_window<R: Runtime>(window: &WebviewWindow<R>) -> Option<&NSWindow> {
    let ns_window_ptr = window.ns_window().ok()?;
    if ns_window_ptr.is_null() {
        return None;
    }
    Some(unsafe { &*ns_window_ptr.cast::<NSWindow>() })
}

pub fn allow_join_fullscreen_space<R: Runtime>(window: &WebviewWindow<R>) {
    // SAFETY: see `as_ns_window` — used only within this call.
    let Some(ns_window) = (unsafe { as_ns_window(window) }) else {
        return;
    };

    let behavior = ns_window.collectionBehavior()
        | NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary;
    ns_window.setCollectionBehavior(behavior);

    // `alwaysOnTop: true` in tauri.conf.json only gets us NSFloatingWindowLevel
    // (3) — not high enough to paint over a full-screen Space regardless of
    // collection behavior. NSStatusWindowLevel (25) is what actually does it.
    if ns_window.level() < NSStatusWindowLevel {
        ns_window.setLevel(NSStatusWindowLevel);
    }
}

/// Writes a snapshot of the window's actual AppKit state to the diagnostics
/// log (Settings → Diagnóstico → "Copiar último error" picks this up like
/// any other entry). Call right after `show()`, ideally once inside a
/// full-screen repro and once outside one, so the two can be compared.
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
