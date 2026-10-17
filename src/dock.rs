//! Dónde va la barra de música dentro de la barra de tareas.
//!
//! Lógica pura (sin Win32) para poder probarla: recibe los rectángulos que
//! lee `taskbar` y decide el hueco.
//!
//! - Automático (`shift == 0`): pegada a la izquierda del área de
//!   notificación (reloj, iconos de la bandeja). Si se sabe dónde acaban los
//!   iconos de las apps (UI Automation), no los tapa: se estrecha hasta
//!   `MIN_WIDTH` para caber en el hueco.
//! - Manual: el usuario la arrastra a lo largo de la barra de tareas y se
//!   recuerda cuánto la movió (`shift`, px lógicos hacia la izquierda desde
//!   la posición de ancho completo junto a la bandeja).

use crate::platform::Rect;

/// Ancho normal de la barra (px lógicos; se escala con el DPI).
pub const WIDTH: f64 = 360.0;
/// Lo más que se estrecha para no tapar iconos (sin la onda de audio).
pub const MIN_WIDTH: f64 = 250.0;
/// Alto cuando no se puede anclar (barra de tareas vertical o inexistente).
pub const FLOAT_HEIGHT: f64 = 48.0;
/// Margen arriba y abajo dentro de la barra de tareas.
const INSET: f64 = 4.0;
/// Separación respecto a la bandeja, a los iconos y al borde izquierdo.
const GAP: f64 = 6.0;
const MIN_HEIGHT: f64 = 26.0;
/// Si no se encuentra el área de notificación, se supone que mide esto.
const TRAY_FALLBACK: f64 = 300.0;
/// Un elemento más ancho que esto no es un icono sino un contenedor.
const MAX_ITEM_WIDTH: f64 = 400.0;

/// Lo que se sabe de la barra de tareas principal (px físicos).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Taskbar {
    pub rect: Rect,
    /// Monitor donde está.
    pub monitor: Rect,
    /// Área de notificación (`TrayNotifyWnd`), si se encontró.
    pub tray: Option<Rect>,
    /// Borde derecho del último icono de app (si UI Automation respondió).
    pub icons_right: Option<i32>,
    /// DPI / 96.
    pub scale: f64,
}

/// Hueco disponible para la barra (px físicos).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub min_x: i32,
    /// Borde derecho máximo (justo antes de la bandeja).
    pub right: i32,
    pub y: i32,
    pub height: i32,
    /// Ancho completo y ancho en la posición automática.
    pub full_width: i32,
    pub auto_width: i32,
    pub scale: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    Docked(Layout),
    /// La barra de tareas está escondida (auto-ocultar) o tapada por una
    /// app a pantalla completa: la barra también se esconde.
    Hidden,
    /// Barra de tareas vertical, inexistente o sin sitio: barra flotante.
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
    // Con auto-ocultar, la barra de tareas se desliza fuera del monitor y
    // solo asoma una línea.
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
    // Hueco entre el último icono y la bandeja.
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
    /// Dónde va con el desplazamiento guardado (0 = automático).
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

    /// Límites de `x` al arrastrar una barra de ancho `width`.
    pub fn drag_range(&self, width: i32) -> (i32, i32) {
        (self.min_x, (self.right - width).max(self.min_x))
    }

    /// Desplazamiento (px lógicos) que corresponde a dejarla en `x`. Pegada
    /// a la bandeja vuelve a ser automática (0).
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

/// Borde derecho del último icono de app, a partir de los elementos que
/// UI Automation encontró en la barra de tareas: (rectángulo, clase).
///
/// Se descartan contenedores (muy anchos), lo que no está a la altura de la
/// barra de tareas, la bandeja y el reloj (por posición y por clase) y el
/// botón "Mostrar escritorio".
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

    /// Windows 11 a 100 %: barra de 48 px, bandeja de 260 px.
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
            other => panic!("esperaba anclada, fue {other:?}"),
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
        // Iconos centrados que acaban en x = 1200: quedan 448 px; cabe entera.
        let roomy = docked(place(&Taskbar { icons_right: Some(1200), ..win11() }, false));
        assert_eq!(roomy.auto_width, 360);
        // Acaban en 1360: hueco de 288 px → se estrecha.
        let tight = docked(place(&Taskbar { icons_right: Some(1360), ..win11() }, false));
        assert_eq!(tight.auto_width, 1654 - 1366);
        assert_eq!(tight.rect(0).left, 1366, "empieza justo después del último icono");
        // Sin sitio ni para el mínimo: usa el mínimo (y se puede arrastrar).
        let full = docked(place(&Taskbar { icons_right: Some(1600), ..win11() }, false));
        assert_eq!(full.auto_width, 250);
        // Un valor absurdo se ignora.
        let bogus = docked(place(&Taskbar { icons_right: Some(5000), ..win11() }, false));
        assert_eq!(bogus.auto_width, 360);
    }

    #[test]
    fn scales_with_dpi() {
        // 150 %: barra de tareas de 72 px.
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
        assert_eq!(l.rect(100).left, l.rect(0).left - 150, "el desplazamiento también se escala");
        assert_eq!(l.shift_for(l.rect(100).left), 100);
    }

    #[test]
    fn manual_position_is_clamped_and_round_trips() {
        let l = docked(place(&Taskbar { icons_right: Some(1360), ..win11() }, false));
        // Arrastrada a la izquierda: ancho completo, donde la dejó.
        let r = l.rect(500);
        assert_eq!((r.left, r.right - r.left), (1654 - 360 - 500, 360));
        assert_eq!(l.shift_for(r.left), 500);
        assert_eq!(l.rect(99_999).left, l.min_x, "no sale por la izquierda");
        assert_eq!(l.min_x, 6);
        // Soltada junto a la bandeja: vuelve a ser automática.
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
        // Auto-ocultar: solo asoman 2 px.
        let hidden = Taskbar { rect: Rect { left: 0, top: 1078, right: 1920, bottom: 1126 }, ..win11() };
        assert_eq!(place(&hidden, false), Placement::Hidden);
        assert_eq!(place(&win11(), true), Placement::Hidden, "app a pantalla completa delante");
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
            item(0, 1920, "Shell_TrayWnd"),                          // la barra entera
            item(10, 160, "Widgets"),                                // tiempo/widgets
            item(700, 744, "ToggleButton"),                          // Inicio
            item(748, 792, "Taskbar.TaskListButtonAutomationPeer"),
            item(1172, 1216, "Taskbar.TaskListButtonAutomationPeer"), // último icono
            item(560, 1360, "Taskbar.TaskbarFrameAutomationPeer"),   // contenedor ancho
            item(1640, 1660, "SystemTray.ChevronButton"),            // bandeja
            item(1700, 1800, "SystemTray.NormalButton"),
            item(1905, 1920, "ShowDesktopButton"),
            (Rect { left: 1300, top: 900, right: 1340, bottom: 940 }, "Popup".to_owned()), // fuera de la barra
            (Rect { left: 1250, top: 1050, right: 1290, bottom: 1056 }, "Línea".to_owned()), // demasiado bajo
        ];
        assert_eq!(icons_right(&items, tb, Some(1660), 1.0), Some(1216));
        assert_eq!(icons_right(&[], tb, Some(1660), 1.0), None);
        // Windows 10: botones a la izquierda, bandeja "TrayNotifyWnd" aparte.
        let win10 = vec![item(0, 48, "Start"), item(48, 392, "TrayButton"), item(400, 460, ""), item(460, 520, "")];
        assert_eq!(icons_right(&win10, tb, Some(1700), 1.0), Some(520));
    }
}
