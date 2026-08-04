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
//!    tier the system's own menu bar/status items render at. Collection
//!    behavior alone was not enough in testing; the level increase is what
//!    actually made the window paint above a full-screen app's Space.
//!
//! Tauri/tao expose neither knob for this combination, so both are set
//! directly on the raw `NSWindow` Tauri already owns.

use objc2_app_kit::{NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior};
use tauri::{Runtime, WebviewWindow};

pub fn allow_join_fullscreen_space<R: Runtime>(window: &WebviewWindow<R>) {
    let Ok(ns_window_ptr) = window.ns_window() else {
        return;
    };
    if ns_window_ptr.is_null() {
        return;
    }

    // SAFETY: `ns_window()` returns the popover's own NSWindow as an
    // Objective-C object pointer, valid for as long as the window is —
    // we only borrow it here to read/write a couple of properties, never
    // retain or release it ourselves.
    let ns_window: &NSWindow = unsafe { &*ns_window_ptr.cast::<NSWindow>() };

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
