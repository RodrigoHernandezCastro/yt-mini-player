// Release builds open no console; debug builds keep it for the logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// First: the other modules use its `t!` and `msg!` macros.
#[macro_use]
mod i18n;

mod app;
mod cast;
mod config;
mod devtools;
mod diag;
mod e2e;
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
mod loopback;
mod media;
mod mini;
mod music;
mod net;
mod pacer;
mod platform;
mod policy;
mod remote;
mod security;
mod services;
mod settings_panel;
mod taskbar;
mod together;
mod together_panel;
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
    // Until the settings are read, the Windows display language (startup
    // errors and the WebView2 prompt use it).
    i18n::set(i18n::detect());
    if let Err(err) = run() {
        platform::show_error(&t!("startup.failed", error = err));
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    // `--hidden`: start hidden in the tray (used when starting with Windows).
    let start_hidden = std::env::args().skip(1).any(|a| a == "--hidden");
    // `--wait-pid=N`: restarted by the app itself (a setting that needs a
    // fresh start): the previous copy finishes quitting first.
    if let Some(pid) = std::env::args().skip(1).find_map(|a| a.strip_prefix("--wait-pid=").and_then(|p| p.parse::<u32>().ok())) {
        platform::wait_for_exit(pid, std::time::Duration::from_secs(15));
    }

    // If it is already running, ask it to show itself and quit this copy.
    // (The test build with its own data folder is an instance of its own.)
    let activation = match platform::claim_instance(e2e::instance_name().as_deref()) {
        Instance::Primary(listener) => listener,
        Instance::Secondary => return Ok(()),
    };

    // `webview_version()` is more reliable than reading registry keys by hand.
    let runtime_version = match wry::webview_version() {
        Ok(version) => version,
        Err(_) => {
            platform::prompt_install_webview2();
            return Ok(());
        }
    };

    let paths = match e2e::data_dir() {
        Some(dir) => Paths::under(dir)?,
        None => Paths::resolve()?,
    };
    // A diary of notable steps, to know where it stopped if it crashes.
    diag::start(&paths.root, &runtime_version);
    #[cfg(feature = "e2e")]
    diag::note(format!("{}: DevTools port {:?}", e2e::MARKER, e2e::port()));
    // 0.6.0 encrypted the folder with EFS (and Windows showed a scary
    // prompt): it is decrypted before WebView2 opens the profile.
    let legacy_efs = security::remove_legacy_efs(&paths.root, &paths.profile);
    // Vault: AES-256-GCM; its key is bound to the PC's TPM (if any) and to
    // the Windows account (DPAPI).
    let hardware = vault::hardware_protector();
    let (vault, vault_state) =
        vault::Vault::open_with(&paths.vault_key, vault::system_protector().as_ref(), hardware.as_deref())?;
    let store = Store::new(vault);
    // Moving the key to the TPM is slow the first time (it creates an RSA
    // key inside the chip): in the background, without delaying startup.
    if hardware.is_some() && vault::key_protection(&paths.vault_key) != Some(vault::KeyProtection::DeviceAndAccount) {
        let vault = store.vault();
        let key_path = paths.vault_key.clone();
        let _ = std::thread::Builder::new().name("tpm".into()).spawn(move || {
            if let Some(hardware) = vault::hardware_protector() {
                if let Err(err) = vault.upgrade(&key_path, vault::system_protector().as_ref(), hardware.as_ref()) {
                    eprintln!("the key is still protected by the account only: {err}");
                }
            }
        });
    }
    let settings = Settings::load(&store, &paths.settings);
    // The language the user picked, or else the Windows one.
    i18n::set(settings.language());
    // Before the first webview: they all share these arguments.
    webview::configure(settings.hardware_acceleration);
    diag::note(format!("hardware acceleration {}", if settings.hardware_acceleration { "on" } else { "off" }));
    let saved_geometry = WindowGeometry::load(&store, &paths.window_state);
    // IPC session keys (new on every start).
    let keys = webview::Keys::new()?;
    let overlay = platform::has_uiaccess();
    let extension_ids = config::load_extension_ids(&store, &paths.extension_ids);
    let allowed_extensions = Arc::new(Mutex::new(extension_ids.values().cloned().collect::<Vec<_>>()));

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    // After creating the loop: tao sets its own theme mode there.
    platform::use_dark_menus();
    let proxy = event_loop.create_proxy();
    let window = window::build(&event_loop, saved_geometry)?;
    let mini_window = mini::build_window(&event_loop)?;
    let together_window = together_panel::build_window(&event_loop)?;
    let settings_window = settings_panel::build_window(&event_loop)?;
    let cast_window = cast::build_window(&event_loop)?;

    // Start on the menu, or straight in the last service if so chosen.
    let agents = user_agent::Agents::new(&runtime_version);
    let start = services::startup_service(settings.start_with_last, settings.last_service.as_deref());
    let (start_url, start_agent) = match start {
        Some(service) => (service.url, service.agent),
        None => (services::HOME_URL, services::Agent::Chrome),
    };
    let home_state: home::SharedHomeState = Arc::new(Mutex::new(home::HomeState {
        last_service: settings.last_service.clone(),
        start_with_last: settings.start_with_last,
        welcome: !settings.welcome_seen,
    }));

    // My music: its own thread with the library, the downloads and the license.
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

    webview::watch_processes(&webview, "main");

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
        together_window,
        settings_window,
        cast_window,
        webview_version: runtime_version.clone(),
    });

    // `ControlFlow::Wait` keeps the thread asleep until a message arrives:
    // zero host CPU while idle.
    event_loop.run(move |event, _target, control_flow| {
        let keep_running = match event {
            // A timer is due (check the music bar).
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
                // Alt+F4 or closing from the taskbar.
                WindowEvent::CloseRequested => app.handle(Command::Quit),
                _ => true,
            },
            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. }
                if window_id == app.mini_window_id() =>
            {
                app.handle(Command::MiniHide)
            }
            // Closing "Watch together" only hides it: the session goes on.
            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. }
                if window_id == app.together_window_id() =>
            {
                app.handle(Command::TogetherClose)
            }
            // Closing Settings only hides it.
            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. }
                if window_id == app.settings_window_id() =>
            {
                app.handle(Command::SettingsClose)
            }
            _ => true,
        };
        // `Wait` sleeps until a message arrives (zero CPU while idle); with
        // the music bar showing or a watch together session on, until the
        // next check.
        *control_flow = if !keep_running {
            ControlFlow::Exit
        } else if let Some(deadline) = app.deadline() {
            ControlFlow::WaitUntil(deadline)
        } else {
            ControlFlow::Wait
        };
    })
}
