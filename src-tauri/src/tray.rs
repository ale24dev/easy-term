//! Keeps the menu-bar tray icon's badge and tooltip in sync with the
//! current set of projects and their live status.
//!
//! `NSStatusItem` has no equivalent of the Dock's little red badge, so the
//! running-project count is painted directly onto the tray icon's own
//! pixels (a red circle with the count in white) instead of relying on the
//! icon's title text — mirrors how the count would look as a Dock badge.
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
use tauri::{image::Image, AppHandle, Manager};

pub const TRAY_ID: &str = "main";

/// 3×5 pixel bitmap font, one row per byte (bits 2..0 = left..right column).
/// Covers exactly what the badge ever needs to render: digits and "+".
fn glyph_for(c: char) -> [u8; 5] {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b001, 0b001],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        _ => [0, 0, 0, 0, 0],
    }
}

/// Copies `base`'s pixels and paints a red badge (white ring, white digits)
/// over its top-right corner. Sized proportionally to `base` so it looks
/// right regardless of what resolution the platform hands back for the app
/// icon.
fn badge_icon(base: &Image, count: u32) -> Image<'static> {
    let width = base.width();
    let height = base.height();
    let mut rgba = base.rgba().to_vec();

    let diameter = (width.min(height) as f32 * 0.55) as i32;
    let margin = (width as f32 * 0.04) as i32;
    let radius = diameter / 2;
    let border = (diameter / 16).max(1);
    let cx = width as i32 - radius - margin;
    let cy = radius + margin;

    let min_x = (cx - radius - border).max(0);
    let max_x = (cx + radius + border).min(width as i32 - 1);
    let min_y = (cy - radius - border).max(0);
    let max_y = (cy + radius + border).min(height as i32 - 1);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = x - cx;
            let dy = y - cy;
            let dist2 = dx * dx + dy * dy;
            if dist2 > (radius + border) * (radius + border) {
                continue;
            }

            let idx = ((y as u32 * width + x as u32) * 4) as usize;
            if dist2 <= radius * radius {
                // Solid red fill — the badge itself.
                rgba[idx] = 220;
                rgba[idx + 1] = 38;
                rgba[idx + 2] = 38;
                rgba[idx + 3] = 255;
            } else {
                // A white ring so the badge stays legible over any menu bar
                // background or the icon's own colors right behind it.
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            }
        }
    }

    let label = if count > 9 {
        "9+".to_string()
    } else {
        count.to_string()
    };
    let glyphs: Vec<[u8; 5]> = label.chars().map(glyph_for).collect();
    let scale = (diameter * 3 / 25).max(1);
    let glyph_w = 3 * scale;
    let glyph_h = 5 * scale;
    let step = glyph_w + scale;
    let text_w = glyphs.len() as i32 * glyph_w + (glyphs.len() as i32 - 1).max(0) * scale;
    let text_x0 = cx - text_w / 2;
    let text_y0 = cy - glyph_h / 2;

    for (gi, glyph) in glyphs.iter().enumerate() {
        let gx0 = text_x0 + gi as i32 * step;
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..3i32 {
                if (bits >> (2 - col)) & 1 == 0 {
                    continue;
                }
                let px0 = gx0 + col * scale;
                let py0 = text_y0 + row as i32 * scale;
                for py in py0..py0 + scale {
                    for px in px0..px0 + scale {
                        if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                            continue;
                        }
                        let idx = ((py as u32 * width + px as u32) * 4) as usize;
                        rgba[idx] = 255;
                        rgba[idx + 1] = 255;
                        rgba[idx + 2] = 255;
                        rgba[idx + 3] = 255;
                    }
                }
            }
        }
    }

    Image::new_owned(rgba, width, height)
}

fn status_glyph(status: ProjectStatus) -> &'static str {
    match status {
        ProjectStatus::Running => "🟢",
        ProjectStatus::Starting => "🟡",
        ProjectStatus::Crashed => "🔴",
        ProjectStatus::Stopped => "⚪",
    }
}

/// Recomputes the tray icon's badge and tooltip (per-project breakdown).
/// Called after every status change and every project change.
pub fn refresh(app: &AppHandle) {
    let manager = app.state::<ProcessManager>();
    let statuses = manager.snapshot_statuses();

    let running = statuses
        .values()
        .filter(|s| matches!(s, ProjectStatus::Running | ProjectStatus::Starting))
        .count();

    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    if let Some(base) = app.default_window_icon() {
        let icon = if running > 0 {
            badge_icon(base, running as u32)
        } else {
            base.clone().to_owned()
        };
        let _ = tray.set_icon(Some(icon));
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_icon(size: u32, color: [u8; 4]) -> Image<'static> {
        let mut rgba = Vec::with_capacity((size * size * 4) as usize);
        for _ in 0..(size * size) {
            rgba.extend_from_slice(&color);
        }
        Image::new_owned(rgba, size, size)
    }

    fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * image.width() + x) * 4) as usize;
        let rgba = image.rgba();
        [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
    }

    #[test]
    fn glyph_for_digits_and_plus_are_non_empty() {
        for c in "0123456789+".chars() {
            assert_ne!(
                glyph_for(c),
                [0, 0, 0, 0, 0],
                "glyph for {c:?} should draw something"
            );
        }
    }

    #[test]
    fn glyph_for_unknown_char_is_blank() {
        assert_eq!(glyph_for('?'), [0, 0, 0, 0, 0]);
    }

    #[test]
    fn badge_icon_preserves_base_dimensions() {
        let base = solid_icon(64, [10, 10, 10, 255]);
        let badged = badge_icon(&base, 3);
        assert_eq!(badged.width(), 64);
        assert_eq!(badged.height(), 64);
        assert_eq!(badged.rgba().len(), base.rgba().len());
    }

    #[test]
    fn badge_icon_paints_a_red_badge_near_the_top_right_corner() {
        let base = solid_icon(64, [10, 10, 10, 255]);
        let badged = badge_icon(&base, 3);

        // Mirrors badge_icon's own center formula: for a 64px icon the
        // badge circle centers at (45, 19). Checked off to the side of
        // center (not dead center) so the "3" glyph's own white ink can't
        // land on the sampled pixel and produce a false negative.
        let red_area = pixel(&badged, 30, 19);
        assert_eq!(
            red_area,
            [220, 38, 38, 255],
            "badge circle should be solid red"
        );

        // Far from the badge, the original base color must be untouched.
        let untouched = pixel(&badged, 2, 62);
        assert_eq!(untouched, [10, 10, 10, 255]);
    }

    #[test]
    fn badge_icon_leaves_the_base_unmodified() {
        let base = solid_icon(64, [10, 10, 10, 255]);
        let before = base.rgba().to_vec();
        let _ = badge_icon(&base, 5);
        assert_eq!(base.rgba(), before.as_slice());
    }

    #[test]
    fn badge_icon_caps_the_displayed_label_at_nine_plus() {
        // 15 running projects should still render as the two-glyph "9+",
        // not silently truncate or panic trying to lay out a wider number.
        let base = solid_icon(64, [10, 10, 10, 255]);
        let badged = badge_icon(&base, 15);
        assert_eq!(badged.width(), 64);
    }
}
