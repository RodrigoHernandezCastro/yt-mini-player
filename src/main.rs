// En release no se abre consola; en debug se conserva para ver los logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod extensions;
mod hotkeys;
mod icon;
mod ipc;
mod lifecycle;
mod links;
mod platform;
mod tray;
mod user_agent;
mod webview;
mod window;

use std::error::Error;

use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
};
use wry::WebContext;

use crate::{
    app::{App, Startup},
    config::{Paths, Settings, WindowGeometry},
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
    let settings = Settings::load(&paths.settings);
    let saved_geometry = WindowGeometry::load(&paths.window_state);

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = window::build(&event_loop, saved_geometry, settings.always_on_top)?;

    let mut web_context = WebContext::new(Some(paths.profile.clone()));
    let webview = webview::build(&window, &mut web_context, &runtime_version, proxy.clone())?;

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
    });

    // `ControlFlow::Wait` deja el hilo dormido hasta que llega un mensaje:
    // cero CPU del host en reposo.
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        let keep_running = match event {
            Event::UserEvent(event) => app.on_user_event(event),
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => {
                app.on_resized();
                true
            }
            // Alt+F4 o cerrar desde la barra de tareas.
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => app.handle(Command::Quit),
            _ => true,
        };
        if !keep_running {
            *control_flow = ControlFlow::Exit;
        }
    })
}
