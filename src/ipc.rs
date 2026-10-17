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

pub use crate::media::{MediaAction, MediaState};

/// Fases del arrastre de la barra de música a lo largo de la barra de
/// tareas. La posición la lee el host del cursor (no la manda la página).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragPhase {
    Start,
    Move,
    End,
}

/// Todo lo que la aplicación sabe hacer. Llega desde la página (IPC), la
/// bandeja del sistema, la barra de música o los atajos globales.
#[derive(Debug, Clone, PartialEq)]
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
    // Servicios
    Home,
    OpenService(String),
    OpenUrl(String),
    SetStartWithLast(bool),
    // Página
    Ready,
    ToggleVideoOnly,
    OpenClipboard,
    Media(MediaAction),
    /// Lo que suena ahora (informe de media.js).
    MediaReport(Box<MediaState>),
    // Bandeja y barra de música
    /// Clic izquierdo en el icono; posición del icono en píxeles físicos.
    TrayClick(i32, i32),
    TrayDoubleClick,
    /// "Mostrar barra de controles" del menú de la bandeja.
    MiniShow,
    MiniHide,
    MiniHover(bool),
    MiniReady,
    MiniDrag(DragPhase),
    /// Ajuste: barra de música en la barra de tareas con servicios de música.
    ToggleTaskbarBar,
    // Extensiones
    ExtensionsFolder,
    ExtensionsReload,
    ExtensionToggle(String),
    ExtensionOptions(String),
    // Sistema
    ToggleAutostart,
    // Navegación (desde los filtros de webview.rs; la página no puede pedirlas)
    /// Navegar la ventana principal (http mejorado a https, ventana nueva
    /// de un servicio del catálogo).
    Navigate(String),
    /// Abrir en el navegador predeterminado (ventana nueva de otro sitio).
    OpenExternal(String),
    /// Algo se bloqueó; el texto es el motivo, para el aviso.
    Blocked(String),
    // Seguridad (bandeja y pantalla de bloqueo; nunca desde la página)
    LockNow,
    /// Elegir el modo de bloqueo (el mismo que ya está = desactivarlo).
    SetLock(crate::config::LockMode),
    SignOutAll,
    SecurityStatus,
    LockReady,
    LockHello,
    LockPin(String),
    LockNewPin(String),
    LockCancel,
    /// "¿Olvidaste el PIN?" en la pantalla de bloqueo.
    LockForgot,
    /// Resultado de verificar con Windows Hello para quitar un PIN olvidado.
    ForgotResult(Result<bool, String>),
    /// Resultados de hilos de trabajo.
    HelloResult(Result<bool, String>),
    PinChecked(bool),
    PinHashed(Result<String, String>),
    BrowsingDataCleared(bool),
    // Mi música (solo desde la página https://ytmp.localhost/music)
    Music(crate::music::Request),
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
        ("media-state", Some(json)) => Command::MediaReport(Box::new(MediaState::from_report(json)?)),
        ("home", None) => Command::Home,
        ("service", Some(id)) if crate::services::is_service_id(id) => Command::OpenService(id.to_owned()),
        // La dirección se valida al ejecutar la orden (services::parse_user_url).
        ("open-url", Some(url)) if !url.is_empty() && url.len() <= 2048 => Command::OpenUrl(url.to_owned()),
        ("start-with-last", Some("1")) => Command::SetStartWithLast(true),
        ("start-with-last", Some("0")) => Command::SetStartWithLast(false),
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

/// Mensajes de la barra de música (otra lista cerrada, en otra ventana).
pub fn parse_mini(message: &str) -> Option<Command> {
    if let Some(action) = message.strip_prefix("media:") {
        return MediaAction::parse(action).map(Command::Media);
    }
    Some(match message {
        "player:show" => Command::Show,
        "mini:close" => Command::MiniHide,
        "mini:hover:1" => Command::MiniHover(true),
        "mini:hover:0" => Command::MiniHover(false),
        "mini:ready" => Command::MiniReady,
        "mini:drag:start" => Command::MiniDrag(DragPhase::Start),
        "mini:drag:move" => Command::MiniDrag(DragPhase::Move),
        "mini:drag:end" => Command::MiniDrag(DragPhase::End),
        _ => return None,
    })
}

/// Mensajes de la página "Mi música" (`music:...`). El webview solo los
/// acepta si vienen de esa página (ver webview.rs).
pub fn parse_music(message: &str) -> Option<Command> {
    crate::music::Request::parse(message.strip_prefix("music:")?).map(Command::Music)
}

/// Mensajes de la pantalla de bloqueo (otra lista cerrada, otro webview).
pub fn parse_lock(message: &str) -> Option<Command> {
    let pin = |p: &str| (!p.is_empty() && p.chars().count() <= crate::lock::PIN_MAX).then(|| p.to_owned());
    Some(match message.split_once(':') {
        Some(("lock", "ready")) => Command::LockReady,
        Some(("lock", "hello")) => Command::LockHello,
        Some(("lock", "cancel")) => Command::LockCancel,
        Some(("lock", "forgot")) => Command::LockForgot,
        Some(("lock", rest)) => match rest.split_once(':') {
            Some(("pin", p)) => Command::LockPin(pin(p)?),
            Some(("new-pin", p)) => Command::LockNewPin(pin(p)?),
            _ => return None,
        },
        _ => return None,
    })
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
#[derive(Debug, Clone, PartialEq)]
pub enum UserEvent {
    Command(Command),
    Extension(crate::extensions::ExtensionEvent),
    /// Aviso del servicio de música para su página (JSON).
    Music(String),
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
    fn parses_service_messages() {
        assert_eq!(parse("home"), Some(Command::Home));
        assert_eq!(parse("service:netflix"), Some(Command::OpenService("netflix".into())));
        assert_eq!(parse("open-url:https://plex.tv/web"), Some(Command::OpenUrl("https://plex.tv/web".into())));
        assert_eq!(parse("start-with-last:1"), Some(Command::SetStartWithLast(true)));
        assert_eq!(parse("start-with-last:0"), Some(Command::SetStartWithLast(false)));
        for bad in ["home:1", "service:", "service:Netflix", "service:../x", "open-url:", "start-with-last:2"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert_eq!(parse(&format!("open-url:{}", "a".repeat(3000))), None);
    }

    #[test]
    fn parses_media_reports() {
        match parse(r#"media-state:{"hasMedia":true,"title":"Hola","playing":true}"#) {
            Some(Command::MediaReport(state)) => assert!(state.playing && state.title == "Hola"),
            other => panic!("{other:?}"),
        }
        assert_eq!(parse("media-state:no es json"), None);
        assert_eq!(parse("media-state:"), None);
    }

    #[test]
    fn mini_protocol_is_separate_and_closed() {
        assert_eq!(parse_mini("media:next"), Some(Command::Media(MediaAction::Next)));
        assert_eq!(parse_mini("media:volume:30"), Some(Command::Media(MediaAction::Volume(30))));
        assert_eq!(parse_mini("player:show"), Some(Command::Show));
        assert_eq!(parse_mini("mini:hover:1"), Some(Command::MiniHover(true)));
        assert_eq!(parse_mini("mini:close"), Some(Command::MiniHide));
        assert_eq!(parse_mini("mini:drag:start"), Some(Command::MiniDrag(DragPhase::Start)));
        assert_eq!(parse_mini("mini:drag:move"), Some(Command::MiniDrag(DragPhase::Move)));
        assert_eq!(parse_mini("mini:drag:end"), Some(Command::MiniDrag(DragPhase::End)));
        assert_eq!(parse_mini("mini:drag:200"), None, "la posición no la decide la página");
        // La barra de música no puede pedir nada de la página principal...
        for msg in ["quit", "drag", "pin", "service:netflix", "open-url:x.com", "media:borrar", "mini:hover:2"] {
            assert_eq!(parse_mini(msg), None, "{msg}");
        }
        // ...y la página no puede usar el protocolo de la barra de música.
        for msg in ["media:next", "player:show", "mini:close", "mini:drag:start"] {
            assert_eq!(parse(msg), None, "{msg}");
        }
    }

    #[test]
    fn lock_protocol_is_separate_and_closed() {
        assert_eq!(parse_lock("lock:ready"), Some(Command::LockReady));
        assert_eq!(parse_lock("lock:hello"), Some(Command::LockHello));
        assert_eq!(parse_lock("lock:forgot"), Some(Command::LockForgot));
        assert_eq!(parse_lock("lock:pin:482915"), Some(Command::LockPin("482915".into())));
        assert_eq!(parse_lock("lock:pin:a:b"), Some(Command::LockPin("a:b".into())), "el PIN puede llevar ':'");
        assert_eq!(parse_lock("lock:new-pin:clave larga"), Some(Command::LockNewPin("clave larga".into())));
        let long = format!("lock:pin:{}", "1".repeat(65));
        for bad in ["lock:pin:", "lock:off", "lock", "quit", "media:next", "mini:close", long.as_str()] {
            assert_eq!(parse_lock(bad), None, "{bad}");
        }
        // Ni la página ni la barra pueden usar el protocolo de bloqueo.
        for msg in ["lock:hello", "lock:pin:482915", "lock:cancel"] {
            assert_eq!(parse(msg), None, "{msg}");
            assert_eq!(parse_mini(msg), None, "{msg}");
        }
    }

    #[test]
    fn music_protocol_is_separate() {
        assert_eq!(parse_music("music:ready"), Some(Command::Music(crate::music::Request::Ready)));
        assert_eq!(parse_music("ready"), None);
        assert_eq!(parse_music("music:borrar-todo"), None);
        // La página normal, la barra y el bloqueo no lo entienden.
        for msg in ["music:ready", "music:pick-folder", "music:buy:lifetime"] {
            assert_eq!(parse(msg), None, "{msg}");
            assert_eq!(parse_mini(msg), None, "{msg}");
            assert_eq!(parse_lock(msg), None, "{msg}");
        }
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
