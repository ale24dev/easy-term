//! Positions the menu bar popover under the tray icon.
//!
//! ## Why this exists instead of `tauri-plugin-positioner`'s `TrayCenter`
//!
//! The popover used to be placed with
//! `move_window_constrained(Position::TrayCenter)`, which worked on a single
//! display and broke as soon as a second monitor was attached: the popover
//! opened somewhere off-screen, so it looked like the app simply refused to
//! open.
//!
//! Root cause is upstream, in the `tray-icon` crate's macOS backend. It
//! converts the status item's Cocoa frame (bottom-left origin, y up) to the
//! top-left-origin space Tauri uses with:
//!
//! ```text
//! fn flip_window_screen_coordinates(y: f64) -> f64 {
//!     CGDisplayPixelsHigh(CGMainDisplayID()) as f64 - y
//! }
//! ```
//!
//! That flips against the height of the **main display only**, whichever
//! display the tray icon actually lives on. Cocoa's global space has its
//! origin at the main display's bottom-left corner, so points on a second
//! monitor are outside `0..=mainHeight` — for a display placed above the
//! main one the subtraction goes negative, and for one below it lands in the
//! wrong band. Either way the reported tray `y` doesn't correspond to a real
//! pixel row, so the positioner's `monitor_from_point(tray_x, tray_y)` finds
//! no monitor, silently skips its clamp, and computes a `y` from that same
//! bogus value.
//!
//! So: the tray rect's **x and size are trustworthy** (x needs no flipping,
//! and the size is used as-is), its **y is not**. This module never reads
//! the tray y. It picks the target display from the cursor — the user just
//! clicked the tray icon, so the pointer is on the menu bar that was
//! clicked, in both "Displays have separate Spaces" configurations — and
//! takes the vertical position from that monitor's *work area*, whose top
//! edge is by definition just below that monitor's menu bar, expressed in
//! Tauri's own (reliable) coordinate space.

use crate::error_logger::{log_error, Level, Source};
use tauri::{PhysicalPosition, Runtime, WebviewWindow};

const MODULE: &str = "popover";

/// A rectangle in Tauri's physical, top-left-origin coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Computes the popover's top-left corner: horizontally centered under the
/// tray icon, vertically at the top of the monitor's work area (i.e. flush
/// under that monitor's menu bar), clamped so the window stays fully within
/// the work area horizontally.
///
/// Split out as a pure function so the geometry — including the negative
/// coordinates a display left of or above the primary produces, which is
/// exactly what regressed — is unit-testable without a windowing system.
pub fn popover_position(
    tray_x: f64,
    tray_width: f64,
    window_width: f64,
    work_area: Rect,
) -> (i32, i32) {
    let centered_x = tray_x + tray_width / 2.0 - window_width / 2.0;

    let min_x = work_area.x;
    let max_x = work_area.x + work_area.width - window_width;
    // A window wider than the display inverts the range, and `clamp` panics
    // on that rather than picking a side — pin it to the left edge instead.
    let x = if max_x < min_x {
        min_x
    } else {
        centered_x.clamp(min_x, max_x)
    };

    (x.round() as i32, work_area.y.round() as i32)
}

/// Moves `window` under the tray icon, whose last known rect is `tray`.
///
/// Returns `false` (having logged why) if the position couldn't be
/// determined, so the caller can decide whether showing the window anyway is
/// better than not showing it at all.
pub fn position_under_tray<R: Runtime>(window: &WebviewWindow<R>, tray: Rect) -> bool {
    // The cursor is on the menu bar the user just clicked, which is the
    // display the tray icon is on — the one reliable signal available here,
    // since the tray rect's own y can't be trusted (see the module comment).
    let monitor = window
        .cursor_position()
        .ok()
        .and_then(|cursor| window.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten());

    let Some(monitor) = monitor else {
        log_error(
            Level::Warn,
            Source::Backend,
            MODULE,
            "POPOVER_NO_MONITOR",
            "Could not resolve a monitor to position the popover on",
            None,
            None,
        );
        return false;
    };

    let Ok(window_size) = window.outer_size() else {
        log_error(
            Level::Warn,
            Source::Backend,
            MODULE,
            "POPOVER_NO_WINDOW_SIZE",
            "Could not read the popover's size",
            None,
            None,
        );
        return false;
    };

    let area = monitor.work_area();
    let work_area = Rect {
        x: area.position.x as f64,
        y: area.position.y as f64,
        width: area.size.width as f64,
        height: area.size.height as f64,
    };

    let (x, y) = popover_position(tray.x, tray.width, window_size.width as f64, work_area);

    if let Err(e) = window.set_position(PhysicalPosition::new(x, y)) {
        log_error(
            Level::Warn,
            Source::Backend,
            MODULE,
            "POPOVER_SET_POSITION_FAILED",
            format!("Could not move the popover: {e}"),
            Some(serde_json::json!({ "x": x, "y": y })),
            None,
        );
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1512x982 work area at the origin — a primary display with the menu
    /// bar taking the top 38px.
    fn primary() -> Rect {
        Rect {
            x: 0.0,
            y: 38.0,
            width: 1512.0,
            height: 944.0,
        }
    }

    #[test]
    fn centers_the_window_under_the_tray_icon() {
        // Tray icon 24px wide starting at x=1000 → its center is 1012.
        // A 380px window centered there starts at 1012 - 190 = 822.
        let (x, y) = popover_position(1000.0, 24.0, 380.0, primary());
        assert_eq!(x, 822);
        assert_eq!(y, 38, "y comes from the work area's top, not the tray");
    }

    #[test]
    fn clamps_against_the_right_edge_instead_of_overflowing() {
        // Tray icon near the right edge: centering would push the window
        // past 1512, so it should stop flush with the edge.
        let (x, _) = popover_position(1500.0, 24.0, 380.0, primary());
        assert_eq!(x, 1512 - 380);
    }

    #[test]
    fn clamps_against_the_left_edge() {
        let (x, _) = popover_position(0.0, 24.0, 380.0, primary());
        assert_eq!(x, 0);
    }

    /// The regression this module exists for: a second display placed to the
    /// LEFT of the primary lives at negative x, so the whole computation has
    /// to work in that monitor's own space rather than assuming a 0 origin.
    #[test]
    fn positions_on_a_second_monitor_left_of_the_primary() {
        let left_monitor = Rect {
            x: -1920.0,
            y: 38.0,
            width: 1920.0,
            height: 1042.0,
        };

        // Tray icon at x=-500 on that display: its center is -488, so a
        // 380px window centered there starts at -488 - 190 = -678.
        let (x, y) = popover_position(-500.0, 24.0, 380.0, left_monitor);
        assert_eq!(x, -678, "centered under the tray icon at negative x");
        assert_eq!(y, 38);
        assert!(
            x >= -1920 && x + 380 <= 0,
            "window must stay within the left monitor's bounds"
        );
    }

    /// A display ABOVE the primary is the case where the upstream flip goes
    /// negative; the popover must still land on that display's menu bar.
    #[test]
    fn positions_on_a_second_monitor_above_the_primary() {
        let top_monitor = Rect {
            x: 0.0,
            y: -1042.0,
            width: 1920.0,
            height: 1042.0,
        };

        let (x, y) = popover_position(900.0, 24.0, 380.0, top_monitor);
        assert_eq!(x, 722);
        assert_eq!(
            y, -1042,
            "negative y is a legitimate position, not an error"
        );
    }

    #[test]
    fn clamps_to_the_left_edge_when_the_window_is_wider_than_the_display() {
        // Inverted clamp range — must not panic, and must pick the left edge.
        let narrow = Rect {
            x: 100.0,
            y: 20.0,
            width: 200.0,
            height: 400.0,
        };
        let (x, y) = popover_position(150.0, 24.0, 380.0, narrow);
        assert_eq!(x, 100);
        assert_eq!(y, 20);
    }
}
