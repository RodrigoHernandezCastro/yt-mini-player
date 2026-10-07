//! Protocolo entre la página y el host, y órdenes internas de la aplicación.
//!
//! La página solo puede enviar los mensajes de [`parse`]: una lista cerrada.
//! Cualquier otra cosa se ignora, así que YouTube (o cualquier script de la
//! página) no puede provocar acciones fuera de este conjunto. Las órdenes que
//! solo deben venir del sistema (bandeja, atajos globales) no tienen mensaje.

/// Borde o esquina desde el que se redimensiona.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

/// Esquina del área de trabajo del monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Tamaños predefinidos de la ventana (16:9, en píxeles lógicos).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizePreset {
    Small,
    Medium,
    Large,
}

impl SizePreset {
    pub fn logical_size(self) -> (f64, f64) {
        match self {
            SizePreset::Small => (480.0, 270.0),
            SizePreset::Medium => (640.0, 360.0),
            SizePreset::Large => (960.0, 540.0),
        }
    }
}

/// Acciones sobre el reproductor de YouTube (atajos globales).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    PlayPause,
    Next,
    Previous,
}

impl Media {
    pub fn js_name(self) -> &'static str {
        match self {
            Media::PlayPause => "toggle",
            Media::Next => "next",
            Media::Previous => "previous",
        }
    }
}

/// Todo lo que la aplicación sabe hacer. Llega desde la página (IPC), la
/// bandeja del sistema o los atajos globales.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    // Ventana
    Drag,
    Resize(Edge),
    TogglePin,
    SetFullscreen(bool),
    SetOpacity(u8),
    AdjustOpacity(i8),
    ResetOpacity,
    ToggleGhost,
    Size(SizePreset),
    Corner(Corner),
    // Visibilidad y ciclo de vida
    Show,
    Hide,
    ToggleVisible,
    Quit,
    // Página
    Ready,
    ToggleVideoOnly,
    OpenClipboard,
    Media(Media),
    // Extensiones
    ExtensionsFolder,
    ExtensionsReload,
    ExtensionToggle(String),
    ExtensionOptions(String),
    // Sistema
    ToggleAutostart,
}

/// Opacidad mínima: por debajo la ventana es prácticamente invisible y el
/// usuario podría perderla.
pub const MIN_OPACITY: u8 = 20;

/// Convierte un mensaje de la página en una orden.
pub fn parse(message: &str) -> Option<Command> {
    let (head, arg) = match message.split_once(':') {
        Some((head, arg)) => (head, Some(arg)),
        None => (message, None),
    };
    let command = match (head, arg) {
        ("drag", None) => Command::Drag,
        ("resize", Some(dir)) => Command::Resize(parse_edge(dir)?),
        ("ready", None) => Command::Ready,
        ("pin", None) => Command::TogglePin,
        ("video-only", None) => Command::ToggleVideoOnly,
        ("ghost", None) => Command::ToggleGhost,
        ("autostart", None) => Command::ToggleAutostart,
        ("hide", None) => Command::Hide,
        ("quit", None) => Command::Quit,
        ("open-clipboard", None) => Command::OpenClipboard,
        ("fullscreen", Some("1")) => Command::SetFullscreen(true),
        ("fullscreen", Some("0")) => Command::SetFullscreen(false),
        ("opacity", Some(value)) => {
            let value: u8 = value.parse().ok()?;
            (MIN_OPACITY..=100).contains(&value).then_some(Command::SetOpacity(value))?
        }
        ("size", Some(size)) => Command::Size(match size {
            "s" => SizePreset::Small,
            "m" => SizePreset::Medium,
            "l" => SizePreset::Large,
            _ => return None,
        }),
        ("corner", Some(corner)) => Command::Corner(match corner {
            "nw" => Corner::TopLeft,
            "ne" => Corner::TopRight,
            "sw" => Corner::BottomLeft,
            "se" => Corner::BottomRight,
            _ => return None,
        }),
        ("ext-folder", None) => Command::ExtensionsFolder,
        ("ext-reload", None) => Command::ExtensionsReload,
        ("ext-toggle", Some(id)) if is_extension_id(id) => Command::ExtensionToggle(id.to_owned()),
        ("ext-options", Some(id)) if is_extension_id(id) => {
            Command::ExtensionOptions(id.to_owned())
        }
        _ => return None,
    };
    Some(command)
}

fn parse_edge(dir: &str) -> Option<Edge> {
    Some(match dir {
        "n" => Edge::North,
        "s" => Edge::South,
        "e" => Edge::East,
        "w" => Edge::West,
        "ne" => Edge::NorthEast,
        "nw" => Edge::NorthWest,
        "se" => Edge::SouthEast,
        "sw" => Edge::SouthWest,
        _ => return None,
    })
}

/// Los identificadores de extensión de Chromium son 32 letras entre `a` y `p`.
pub fn is_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// Eventos que llegan al bucle principal desde otros hilos o callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserEvent {
    Command(Command),
    Extension(crate::extensions::ExtensionEvent),
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "cjpalhdlnbpafiamejdnhcphjbkeiagm";

    #[test]
    fn parses_window_messages() {
        assert_eq!(parse("drag"), Some(Command::Drag));
        assert_eq!(parse("resize:se"), Some(Command::Resize(Edge::SouthEast)));
        assert_eq!(parse("pin"), Some(Command::TogglePin));
        assert_eq!(parse("fullscreen:1"), Some(Command::SetFullscreen(true)));
        assert_eq!(parse("fullscreen:0"), Some(Command::SetFullscreen(false)));
        assert_eq!(parse("size:l"), Some(Command::Size(SizePreset::Large)));
        assert_eq!(parse("corner:sw"), Some(Command::Corner(Corner::BottomLeft)));
        assert_eq!(parse("hide"), Some(Command::Hide));
        assert_eq!(parse("quit"), Some(Command::Quit));
    }

    #[test]
    fn parses_every_edge() {
        for dir in ["n", "s", "e", "w", "ne", "nw", "se", "sw"] {
            assert!(matches!(parse(&format!("resize:{dir}")), Some(Command::Resize(_))), "{dir}");
        }
    }

    #[test]
    fn opacity_is_bounded() {
        assert_eq!(parse("opacity:85"), Some(Command::SetOpacity(85)));
        assert_eq!(parse("opacity:100"), Some(Command::SetOpacity(100)));
        assert_eq!(parse("opacity:20"), Some(Command::SetOpacity(20)));
        for bad in ["opacity:19", "opacity:0", "opacity:101", "opacity:-5", "opacity:", "opacity:x"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn extension_ids_are_validated() {
        assert_eq!(parse(&format!("ext-toggle:{ID}")), Some(Command::ExtensionToggle(ID.into())));
        assert_eq!(parse(&format!("ext-options:{ID}")), Some(Command::ExtensionOptions(ID.into())));
        assert_eq!(parse("ext-toggle:../../etc"), None);
        assert_eq!(parse("ext-toggle:CJPALHDLNBPAFIAMEJDNHCPHJBKEIAGM"), None);
        assert_eq!(parse("ext-options:abc"), None);
    }

    #[test]
    fn page_cannot_send_system_only_commands() {
        // Mostrar, reproducir o restablecer opacidad solo llegan desde la
        // bandeja o los atajos globales.
        for msg in ["show", "toggle-visible", "media:next", "reset-opacity", "adjust-opacity:10"] {
            assert_eq!(parse(msg), None, "{msg}");
        }
    }

    #[test]
    fn rejects_unknown_or_malformed() {
        for msg in ["", "Drag", "drag:1", "resize:", "resize:x", "pin:1", "size:xl", "corner:n", "close", "drag "] {
            assert_eq!(parse(msg), None, "{msg:?}");
        }
    }
}
