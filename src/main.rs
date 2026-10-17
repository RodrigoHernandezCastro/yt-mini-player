// En release no se abre consola; en debug se conserva para ver los logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod dock;
mod extensions;
mod home;
mod hotkeys;
mod icon;
mod ipc;
mod lifecycle;
mod links;
mod lock;
mod license;
mod lockview;
mod media;
mod mini;
mod music;
mod net;
mod platform;
mod policy;
mod security;
mod services;
mod taskbar;
mod topmost;
mod tray;
mod user_agent;
mod vault;
mod webview;
mod window;

use std::{
    error::Error,
    sync::{Arc, Mutex},
};

use tao::{
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
};
use wry::WebContext;

use crate::{
    app::{App, Startup},
    config::{Paths, Settings, Store, WindowGeometry},
    ipc::{Command, UserEvent},
    platform::Instance,
};

fn main() {
    if let Err(err) = run() {
        platform::show_error(&format!("No se pudo iniciar el reproductor.\n\n{err}"));
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    // `--hidden`: arrancar oculto en la bandeja (lo usa el inicio con Windows).
    let start_hidden = std::env::args().skip(1).any(|a| a == "--hidden");

    // Si ya está abierto, se le pide que se muestre y esta copia termina.
    let activation = match platform::claim_instance() {
        Instance::Primary(listener) => listener,
        Instance::Secondary => return Ok(()),
    };

    // `webview_version()` es más fiable que leer claves de registro a mano.
    let runtime_version = match wry::webview_version() {
        Ok(version) => version,
        Err(_) => {
            platform::prompt_install_webview2();
            return Ok(());
        }
    };

    let paths = Paths::resolve()?;
    // La 0.6.0 cifraba la carpeta con EFS (y Windows mostraba un aviso que
    // asustaba): se descifra antes de que WebView2 abra el perfil.
    let legacy_efs = security::remove_legacy_efs(&paths.root, &paths.profile);
    // Bóveda: AES-256-GCM; la clave, atada al TPM del equipo (si hay) y a la
    // cuenta de Windows (DPAPI).
    let hardware = vault::hardware_protector();
    let (vault, vault_state) =
        vault::Vault::open_with(&paths.vault_key, vault::system_protector().as_ref(), hardware.as_deref())?;
    let store = Store::new(vault);
    // Pasar la clave al TPM es lento la primera vez (genera una clave RSA
    // dentro del chip): en segundo plano, sin retrasar el arranque.
    if hardware.is_some() && vault::key_protection(&paths.vault_key) != Some(vault::KeyProtection::DeviceAndAccount) {
        let vault = store.vault();
        let key_path = paths.vault_key.clone();
        let _ = std::thread::Builder::new().name("tpm".into()).spawn(move || {
            if let Some(hardware) = vault::hardware_protector() {
                if let Err(err) = vault.upgrade(&key_path, vault::system_protector().as_ref(), hardware.as_ref()) {
                    eprintln!("la clave sigue protegida solo por la cuenta: {err}");
                }
            }
        });
    }
    let settings = Settings::load(&store, &paths.settings);
    let saved_geometry = WindowGeometry::load(&store, &paths.window_state);
    // Claves de sesión del IPC (nuevas en cada arranque).
    let keys = webview::Keys::new()?;
    let overlay = platform::has_uiaccess();
    let extension_ids = config::load_extension_ids(&store, &paths.extension_ids);
    let allowed_extensions = Arc::new(Mutex::new(extension_ids.values().cloned().collect::<Vec<_>>()));

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    // Después de crear el bucle: tao fija ahí su propio modo de tema.
    platform::use_dark_menus();
    let proxy = event_loop.create_proxy();
    let window = window::build(&event_loop, saved_geometry)?;
    let mini_window = mini::build_window(&event_loop)?;

    // Arranca en el menú, o directo en el último servicio si así se pidió.
    let agents = user_agent::Agents::new(&runtime_version);
    let start = services::startup_service(settings.start_with_last, settings.last_service.as_deref());
    let (start_url, start_agent) = match start {
        Some(service) => (service.url, service.agent),
        None => (services::HOME_URL, services::Agent::Chrome),
    };
    let home_state: home::SharedHomeState = Arc::new(Mutex::new(home::HomeState {
        last_service: settings.last_service.clone(),
        start_with_last: settings.start_with_last,
    }));

    // Mi música: un hilo propio con la biblioteca, las descargas y la licencia.
    let music_proxy = proxy.clone();
    let music = music::start(
        music::Config {
            dir: paths.root.join("music"),
            downloads: music::os::music_dir().map(|dir| dir.join(music::DOWNLOADS_FOLDER)),
            store: store.clone(),
            archive: music::archive::BASE.to_owned(),
            license_api: license::API.to_owned(),
            shop: license::StoreConfig::built_in(),
            dev_unlock: license::dev_unlock(),
        },
        move |json| {
            let _ = music_proxy.send_event(UserEvent::Music(json));
        },
    );

    let mut web_context = WebContext::new(Some(paths.profile.clone()));
    let webview = webview::build(
        &window,
        &mut web_context,
        webview::Options {
            start_url,
            user_agent: agents.for_agent(start_agent),
            home_state: home_state.clone(),
            proxy: proxy.clone(),
            keys: keys.clone(),
            overlay,
            extension_ids: allowed_extensions.clone(),
        },
    )?;

    let show_proxy = proxy.clone();
    activation.listen(move || {
        let _ = show_proxy.send_event(UserEvent::Command(Command::Show));
    });

    let mut app = App::new(Startup {
        window,
        webview,
        web_context,
        paths,
        settings,
        proxy,
        start_hidden,
        agents,
        start_agent,
        home_state,
        mini_window,
        store,
        keys,
        legacy_efs,
        vault_reset: vault_state == vault::Opened::Replaced,
        extension_ids,
        allowed_extensions,
        music: Some(music),
    });

    // `ControlFlow::Wait` deja el hilo dormido hasta que llega un mensaje:
    // cero CPU del host en reposo.
    event_loop.run(move |event, _target, control_flow| {
        let keep_running = match event {
            // Llegó la hora de un temporizador (revisar la barra de música).
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                app.on_timer();
                true
            }
            Event::UserEvent(event) => app.on_user_event(event),
            Event::WindowEvent { window_id, event, .. } if window_id == app.main_window_id() => match event {
                WindowEvent::Resized(_) => {
                    app.on_resized();
                    true
                }
                // Alt+F4 o cerrar desde la barra de tareas.
                WindowEvent::CloseRequested => app.handle(Command::Quit),
                _ => true,
            },
            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. }
                if window_id == app.mini_window_id() =>
            {
                app.handle(Command::MiniHide)
            }
            _ => true,
        };
        // `Wait` deja el hilo dormido hasta que llega un mensaje (cero CPU
        // en reposo); con la barra de música a la vista, hasta su próxima revisión.
        *control_flow = if !keep_running {
            ControlFlow::Exit
        } else if let Some(deadline) = app.deadline() {
            ControlFlow::WaitUntil(deadline)
        } else {
            ControlFlow::Wait
        };
    })
}
