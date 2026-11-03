//! Where the music bar goes inside the taskbar.
//!
//! Pure logic (no Win32) so it can be tested: it receives the rectangles
//! `taskbar` reads and decides the slot.
//!
//! - Automatic (`shift == 0`): right next to the left of the notification
//!   area (clock, tray icons). If it is known where the app icons end (UI
//!   Automation), it does not cover them: it narrows down to `MIN_WIDTH` to
//!   fit in the gap.
//! - Manual: the user drags it along the taskbar and how far it was moved is
//!   remembered (`shift`, logical px to the left of the full-width position
//!   next to the tray).

use crate::platform::Rect;

/// The bar's normal width (logical px; scaled with the DPI).
pub const WIDTH: f64 = 360.0;
/// The narrowest it gets so as not to cover icons (without the audio wave).
pub const MIN_WIDTH: f64 = 250.0;
/// Height when it cannot dock (vertical or missing taskbar).
pub const FLOAT_HEIGHT: f64 = 48.0;
/// Margin above and below inside the taskbar.
const INSET: f64 = 4.0;
/// Gap to the tray, the icons and the left edge.
const GAP: f64 = 6.0;
const MIN_HEIGHT: f64 = 26.0;
/// If the notification area is not found, it is assumed to be this wide.
const TRAY_FALLBACK: f64 = 300.0;
/// An item wider than this is not an icon but a container.
const MAX_ITEM_WIDTH: f64 = 400.0;

/// What is known about the main taskbar (physical px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Taskbar {
    pub rect: Rect,
    /// The monitor it is on.
    pub monitor: Rect,
    /// Notification area (`TrayNotifyWnd`), if found.
    pub tray: Option<Rect>,
    /// Right edge of the last app icon (if UI Automation answered).
    pub icons_right: Option<i32>,
    /// DPI / 96.
    pub scale: f64,
}

/// Slot available for the bar (physical px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub min_x: i32,
    /// Largest right edge (just before the tray).
    pub right: i32,
    pub y: i32,
    pub height: i32,
    /// Full width and width in the automatic position.
    pub full_width: i32,
    pub auto_width: i32,
    pub scale: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    Docked(Layout),
    /// The taskbar is hidden (auto-hide) or covered by a fullscreen app: the
    /// bar hides too.
    Hidden,
    /// Vertical, missing or full taskbar: a floating bar.
    Unavailable,
}

pub fn place(taskbar: &Taskbar, fullscreen_in_front: bool) -> Placement {
    let r = taskbar.rect;
    let scale = if taskbar.scale.is_finite() && taskbar.scale >= 0.5 { taskbar.scale } else { 1.0 };
    let px = |v: f64| (v * scale).round() as i32;
    let (width, height) = (r.right - r.left, r.bottom - r.top);
    if width <= 0 || height <= 0 || height > width {
        return Placement::Unavailable;
    }
    // With auto-hide, the taskbar slides off the monitor and only a line
    // peeks out.
    let top = r.top.max(taskbar.monitor.top);
    let bottom = r.bottom.min(taskbar.monitor.bottom);
    let visible = bottom - top;
    if fullscreen_in_front || visible * 10 < height * 6 {
        return Placement::Hidden;
    }

    let bar_height = (visible - 2 * px(INSET)).max(px(MIN_HEIGHT)).min(visible);
    let y = top + (visible - bar_height) / 2;
    let tray_left = tray_left(taskbar, top, bottom).unwrap_or(r.right - px(TRAY_FALLBACK));
    let right = tray_left - px(GAP);
    let min_x = r.left + px(GAP);
    let full_width = px(WIDTH);
    if right - full_width < min_x {
        return Placement::Unavailable;
    }
    // Gap between the last icon and the tray.
    let auto_width = match taskbar.icons_right.filter(|&x| x > r.left && x < right) {
        Some(icons_right) => (right - (icons_right + px(GAP))).clamp(px(MIN_WIDTH), full_width),
        None => full_width,
    };
    Placement::Docked(Layout { min_x, right, y, height: bar_height, full_width, auto_width, scale })
}

fn tray_left(taskbar: &Taskbar, top: i32, bottom: i32) -> Option<i32> {
    let r = taskbar.rect;
    taskbar
        .tray
        .filter(|t| t.right > t.left && t.left > r.left && t.left <= r.right && t.top < bottom && t.bottom > top)
        .map(|t| t.left)
}

impl Layout {
    /// Where it goes with the saved offset (0 = automatic).
    pub fn rect(&self, shift: i32) -> Rect {
        let (x, width) = if shift <= 0 {
            (self.right - self.auto_width, self.auto_width)
        } else {
            let offset = (f64::from(shift) * self.scale).round() as i32;
            let x = (self.right - self.full_width).saturating_sub(offset).clamp(self.min_x, self.right - self.full_width);
            (x, self.full_width)
        };
        Rect { left: x, top: self.y, right: x + width, bottom: self.y + self.height }
    }

    /// Limits of `x` when dragging a bar `width` wide.
    pub fn drag_range(&self, width: i32) -> (i32, i32) {
        (self.min_x, (self.right - width).max(self.min_x))
    }

    /// Offset (logical px) that leaving it at `x` means. Next to the tray it
    /// becomes automatic again (0).
    pub fn shift_for(&self, x: i32) -> i32 {
        let default_x = self.right - self.full_width;
        let x = x.max(self.min_x);
        if x >= default_x {
            0
        } else {
            (f64::from(default_x - x) / self.scale).round() as i32
        }
    }
}

/// Right edge of the last app icon, from the items UI Automation found in
/// the taskbar: (rectangle, class).
///
/// Containers (very wide), whatever is not at the taskbar's height, the tray
/// and the clock (by position and by class) and the "Show desktop" button
/// are left out.
pub fn icons_right(items: &[(Rect, String)], taskbar: Rect, tray_left: Option<i32>, scale: f64) -> Option<i32> {
    let height = taskbar.bottom - taskbar.top;
    let limit = tray_left.unwrap_or(taskbar.right);
    let max_width = (MAX_ITEM_WIDTH * scale).round() as i32;
    items
        .iter()
        .filter(|(r, class)| {
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            w > 4
                && w <= max_width
                && h * 10 >= height * 4
                && r.top >= taskbar.top - 2
                && r.bottom <= taskbar.bottom + 2
                && r.left >= taskbar.left
                && r.right <= limit + 2
                && !["Tray", "Notify", "Clock", "ShowDesktop"].iter().any(|word| class.contains(word))
        })
        .map(|(r, _)| r.right)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };

    /// Windows 11 at 100%: a 48 px taskbar, a 260 px tray.
    fn win11() -> Taskbar {
        Taskbar {
            rect: Rect { left: 0, top: 1032, right: 1920, bottom: 1080 },
            monitor: MONITOR,
            tray: Some(Rect { left: 1660, top: 1032, right: 1920, bottom: 1080 }),
            icons_right: None,
            scale: 1.0,
        }
    }

    fn docked(p: Placement) -> Layout {
        match p {
            Placement::Docked(layout) => layout,
            other => panic!("expected docked, got {other:?}"),
        }
    }

    #[test]
    fn sits_left_of_the_tray_inside_the_taskbar() {
        let l = docked(place(&win11(), false));
        assert_eq!((l.full_width, l.auto_width, l.height), (360, 360, 40));
        let r = l.rect(0);
        assert_eq!(r, Rect { left: 1660 - 6 - 360, top: 1036, right: 1654, bottom: 1076 });
    }

    #[test]
    fn narrows_to_leave_the_app_icons_visible() {
        // Centred icons ending at x = 1200: 448 px left; it fits whole.
        let roomy = docked(place(&Taskbar { icons_right: Some(1200), ..win11() }, false));
        assert_eq!(roomy.auto_width, 360);
        // Ending at 1360: a 288 px gap → it narrows.
        let tight = docked(place(&Taskbar { icons_right: Some(1360), ..win11() }, false));
        assert_eq!(tight.auto_width, 1654 - 1366);
        assert_eq!(tight.rect(0).left, 1366, "it starts right after the last icon");
        // No room even for the minimum: it uses the minimum (and can be dragged).
        let full = docked(place(&Taskbar { icons_right: Some(1600), ..win11() }, false));
        assert_eq!(full.auto_width, 250);
        // An absurd value is ignored.
        let bogus = docked(place(&Taskbar { icons_right: Some(5000), ..win11() }, false));
        assert_eq!(bogus.auto_width, 360);
    }

    #[test]
    fn scales_with_dpi() {
        // 150%: a 72 px taskbar.
        let tb = Taskbar {
            rect: Rect { left: 0, top: 2088, right: 3840, bottom: 2160 },
            monitor: Rect { left: 0, top: 0, right: 3840, bottom: 2160 },
            tray: Some(Rect { left: 3400, top: 2088, right: 3840, bottom: 2160 }),
            icons_right: None,
            scale: 1.5,
        };
        let l = docked(place(&tb, false));
        assert_eq!((l.full_width, l.height), (540, 60));
        assert_eq!(l.rect(0).left, 3400 - 9 - 540);
        assert_eq!(l.rect(100).left, l.rect(0).left - 150, "the offset scales too");
        assert_eq!(l.shift_for(l.rect(100).left), 100);
    }

    #[test]
    fn manual_position_is_clamped_and_round_trips() {
        let l = docked(place(&Taskbar { icons_right: Some(1360), ..win11() }, false));
        // Dragged to the left: full width, where it was left.
        let r = l.rect(500);
        assert_eq!((r.left, r.right - r.left), (1654 - 360 - 500, 360));
        assert_eq!(l.shift_for(r.left), 500);
        assert_eq!(l.rect(99_999).left, l.min_x, "it does not leave on the left");
        assert_eq!(l.min_x, 6);
        // Dropped next to the tray: it becomes automatic again.
        assert_eq!(l.shift_for(1600), 0);
        assert_eq!(l.shift_for(l.right - l.full_width), 0);
        assert_eq!(l.drag_range(288), (6, 1654 - 288));
    }

    #[test]
    fn top_taskbar_and_windows_10_height() {
        let tb = Taskbar {
            rect: Rect { left: 0, top: 0, right: 1920, bottom: 40 },
            tray: Some(Rect { left: 1700, top: 0, right: 1920, bottom: 40 }),
            ..win11()
        };
        let l = docked(place(&tb, false));
        assert_eq!((l.y, l.height), (4, 32));
    }

    #[test]
    fn hides_when_taskbar_is_hidden_or_covered() {
        // Auto-hide: only 2 px peek out.
        let hidden = Taskbar { rect: Rect { left: 0, top: 1078, right: 1920, bottom: 1126 }, ..win11() };
        assert_eq!(place(&hidden, false), Placement::Hidden);
        assert_eq!(place(&win11(), true), Placement::Hidden, "a fullscreen app in front");
    }

    #[test]
    fn vertical_or_tiny_taskbar_is_unavailable() {
        let vertical = Taskbar { rect: Rect { left: 0, top: 0, right: 62, bottom: 1040 }, tray: None, ..win11() };
        assert_eq!(place(&vertical, false), Placement::Unavailable);
        let narrow = Taskbar { rect: Rect { left: 0, top: 1032, right: 500, bottom: 1080 }, tray: None, ..win11() };
        assert_eq!(place(&narrow, false), Placement::Unavailable);
        let empty = Taskbar { rect: Rect { left: 0, top: 0, right: 0, bottom: 0 }, ..win11() };
        assert_eq!(place(&empty, false), Placement::Unavailable);
    }

    #[test]
    fn bogus_tray_falls_back_to_an_estimate() {
        for tray in [None, Some(Rect { left: 0, top: 0, right: 0, bottom: 0 }), Some(Rect { left: 100, top: 0, right: 300, bottom: 40 })] {
            let l = docked(place(&Taskbar { tray, ..win11() }, false));
            assert_eq!(l.right, 1920 - 300 - 6, "{tray:?}");
        }
        let weird = Taskbar { scale: f64::NAN, ..win11() };
        assert_eq!(docked(place(&weird, false)).full_width, 360);
    }

    #[test]
    fn finds_where_the_app_icons_end() {
        let tb = win11().rect;
        let item = |l: i32, r: i32, class: &str| (Rect { left: l, top: 1032, right: r, bottom: 1080 }, class.to_owned());
        let items = vec![
            item(0, 1920, "Shell_TrayWnd"),                          // the whole taskbar
            item(10, 160, "Widgets"),                                // weather/widgets
            item(700, 744, "ToggleButton"),                          // Start
            item(748, 792, "Taskbar.TaskListButtonAutomationPeer"),
            item(1172, 1216, "Taskbar.TaskListButtonAutomationPeer"), // last icon
            item(560, 1360, "Taskbar.TaskbarFrameAutomationPeer"),   // wide container
            item(1640, 1660, "SystemTray.ChevronButton"),            // tray
            item(1700, 1800, "SystemTray.NormalButton"),
            item(1905, 1920, "ShowDesktopButton"),
            (Rect { left: 1300, top: 900, right: 1340, bottom: 940 }, "Popup".to_owned()), // outside the taskbar
            (Rect { left: 1250, top: 1050, right: 1290, bottom: 1056 }, "Line".to_owned()), // too short
        ];
        assert_eq!(icons_right(&items, tb, Some(1660), 1.0), Some(1216));
        assert_eq!(icons_right(&[], tb, Some(1660), 1.0), None);
        // Windows 10: buttons on the left, a separate "TrayNotifyWnd" tray.
        let win10 = vec![item(0, 48, "Start"), item(48, 392, "TrayButton"), item(400, 460, ""), item(460, 520, "")];
        assert_eq!(icons_right(&win10, tb, Some(1700), 1.0), Some(520));
    }
}
