//! macOS-only: lets the popover window display over a currently full-screen
//! app's Space, instead of the tray click silently doing nothing.
//!
//! By default an `NSWindow` can only appear on the Space it was shown on;
//! `visible_on_all_workspaces` (which Tauri does expose) adds it to every
//! *regular* Space, but that still excludes a Space a full-screen app
//! currently occupies — full-screen apps get an exclusive Space of their
//! own. Menu-bar utilities that need to stay reachable while some other app
//! is full screen (Bartender, Ice, iStat Menus, ...) opt into
//! `NSWindowCollectionBehaviorFullScreenAuxiliary` for exactly this case.
//! Tauri/tao don't expose that flag, so it's set directly on the raw
//! `NSWindow` Tauri already owns.

use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
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
    // we only borrow it here to read/write `collectionBehavior`, never
    // retain or release it ourselves.
    let ns_window: &NSWindow = unsafe { &*ns_window_ptr.cast::<NSWindow>() };

    let behavior = ns_window.collectionBehavior()
        | NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary;
    ns_window.setCollectionBehavior(behavior);
}
