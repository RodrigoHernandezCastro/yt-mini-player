//! The Chrome DevTools Protocol of a webview, through WebView2's own bridge
//! (`CallDevToolsProtocolMethod` and its event receivers). No port is
//! opened and nothing outside the app can reach it.
//!
//! Used by window sharing: `Page.startScreencast` hands over the pictures of
//! the main page and `Input.dispatch*Event` applies a guest's mouse and
//! keyboard (only for guests the host allowed, see remote.rs).

use wry::WebView;

/// Calls `method` with `params` (a JSON object). `done` gets the result's
/// JSON or the error, on the window's thread.
#[cfg(windows)]
pub fn call(webview: &WebView, method: &str, params: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;
    use wry::WebViewExtWindows;

    let done = std::rc::Rc::new(std::cell::RefCell::new(Some(done)));
    let finish = {
        let done = done.clone();
        move |result: Result<String, String>| {
            if let Some(done) = done.borrow_mut().take() {
                done(result);
            }
        }
    };
    let on_complete = finish.clone();
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, json| {
        on_complete(result.map(|()| json).map_err(|e| e.message()));
        Ok(())
    }));
    let started = unsafe {
        webview.webview().CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params), &handler)
    };
    if let Err(err) = started {
        finish(Err(err.message()));
    }
}

#[cfg(not(windows))]
pub fn call(_: &WebView, _: &str, _: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    done(Err("no DevTools protocol outside Windows".into()));
}

/// Calls `method` without waiting for the answer.
pub fn send(webview: &WebView, method: &str, params: &str) {
    call(webview, method, params, |_| {});
}

/// Receives an event of the protocol until dropped.
#[cfg(windows)]
pub struct Subscription {
    receiver: webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2DevToolsProtocolEventReceiver,
    token: i64,
}

#[cfg(windows)]
impl Drop for Subscription {
    fn drop(&mut self) {
        unsafe {
            let _ = self.receiver.remove_DevToolsProtocolEventReceived(self.token);
        }
    }
}

/// `handler` gets each `event`'s parameters (JSON), on the window's thread.
#[cfg(windows)]
pub fn subscribe(webview: &WebView, event: &str, mut handler: impl FnMut(String) + 'static) -> Option<Subscription> {
    use webview2_com::{CoTaskMemPWSTR, DevToolsProtocolEventReceivedEventHandler};
    use windows::core::{HSTRING, PWSTR};
    use wry::WebViewExtWindows;

    let receiver = unsafe { webview.webview().GetDevToolsProtocolEventReceiver(&HSTRING::from(event)) }.ok()?;
    let callback = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
        if let Some(args) = args {
            let mut json = PWSTR::null();
            if unsafe { args.ParameterObjectAsJson(&mut json) }.is_ok() {
                handler(CoTaskMemPWSTR::from(json).to_string());
            }
        }
        Ok(())
    }));
    let mut token = 0i64;
    unsafe { receiver.add_DevToolsProtocolEventReceived(&callback, &mut token) }.ok()?;
    Some(Subscription { receiver, token })
}

#[cfg(not(windows))]
pub struct Subscription;

#[cfg(not(windows))]
pub fn subscribe(_: &WebView, _: &str, _: impl FnMut(String) + 'static) -> Option<Subscription> {
    None
}

/// Whether the page is playing sound now (WebView2's own view of it).
#[cfg(windows)]
pub fn playing_audio(webview: &WebView) -> Option<bool> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
    use windows::core::Interface;
    use wry::WebViewExtWindows;
    let core = webview.webview().cast::<ICoreWebView2_8>().ok()?;
    let mut playing = windows::core::BOOL::default();
    unsafe { core.IsDocumentPlayingAudio(&mut playing) }.ok()?;
    Some(playing.as_bool())
}

#[cfg(not(windows))]
pub fn playing_audio(_: &WebView) -> Option<bool> {
    None
}

/// Sends a text message to a page (`window.chrome.webview` gets a
/// `message` event). Cheaper than running a script for each picture.
#[cfg(windows)]
pub fn post(webview: &WebView, message: &str) -> bool {
    use windows::core::HSTRING;
    use wry::WebViewExtWindows;
    unsafe { webview.webview().PostWebMessageAsString(&HSTRING::from(message)) }.is_ok()
}

#[cfg(not(windows))]
pub fn post(webview: &WebView, message: &str) -> bool {
    // Outside Windows (development), through a script.
    let Ok(json) = serde_json::to_string(message) else { return false };
    webview.evaluate_script(&format!("window.__ytmpPost && window.__ytmpPost({json})")).is_ok()
}

/// The main page's pictures, as they are drawn (`Page.startScreencast`):
/// each one goes to `target` (the page that sends the window) as
/// `"f" + base64 JPEG`, at most `MAX_FPS` a second on average. The page's size in CSS
/// px is kept in `viewport` (to place a guest's clicks).
pub struct Screencast {
    #[cfg(windows)]
    _frames: Subscription,
    #[cfg(windows)]
    main: webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
}

/// Pictures per second sent on (the screencast may give more).
pub const MAX_FPS: u32 = 30;
#[cfg(windows)]
pub fn start_screencast(
    main: &WebView,
    target: &WebView,
    viewport: std::rc::Rc<std::cell::Cell<Option<(f64, f64)>>>,
) -> Option<Screencast> {
    use std::{cell::Cell, time::Instant};
    use windows::core::HSTRING;
    use wry::WebViewExtWindows;

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Metadata {
        device_width: f64,
        device_height: f64,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Frame {
        data: String,
        metadata: Metadata,
        session_id: u64,
    }

    let main_core = main.webview();
    let ack_core = main_core.clone();
    let target_core = target.webview();
    let pacer = Cell::new(crate::pacer::Pacer::new(MAX_FPS));
    let first = Cell::new(true);
    let meter = Cell::new(crate::diag::PictureMeter::new(Instant::now()));
    let frames = subscribe(main, "Page.screencastFrame", move |json| {
        let Ok(frame) = serde_json::from_str::<Frame>(&json) else { return };
        let (w, h) = (frame.metadata.device_width, frame.metadata.device_height);
        if first.replace(false) {
            crate::diag::note(format!("screencast: first picture ({w}x{h}, {} KB)", frame.data.len() / 1024));
        }
        if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 {
            viewport.set(Some((w, h)));
        }
        // At most MAX_FPS a second on average, keeping pictures that come
        // a little early (see pacer.rs).
        let mut pace = pacer.get();
        let send = !frame.data.is_empty() && pace.take(Instant::now());
        pacer.set(pace);
        if send {
            let message = HSTRING::from(format!("f{}", frame.data));
            unsafe {
                let _ = target_core.PostWebMessageAsString(&message);
            }
        }
        let mut counts = meter.get();
        if let Some(line) = counts.picture(Instant::now(), frame.data.len(), (w as u32, h as u32), send) {
            crate::diag::note(line);
        }
        meter.set(counts);
        // The next picture comes only after this one is acknowledged.
        let params = format!(r#"{{"sessionId":{}}}"#, frame.session_id);
        let handler = webview2_com::CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
        unsafe {
            let _ = ack_core.CallDevToolsProtocolMethod(
                &HSTRING::from("Page.screencastFrameAck"),
                &HSTRING::from(params),
                &handler,
            );
        }
    })?;
    let params = r#"{"format":"jpeg","quality":72,"maxWidth":1280,"maxHeight":720,"everyNthFrame":1}"#;
    call(main, "Page.startScreencast", params, |result| {
        crate::diag::note(match result {
            Ok(_) => "screencast: started".to_owned(),
            Err(err) => format!("screencast: refused ({err})"),
        });
    });
    Some(Screencast { _frames: frames, main: main_core })
}

#[cfg(windows)]
impl Drop for Screencast {
    fn drop(&mut self) {
        use windows::core::HSTRING;
        let handler = webview2_com::CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
        unsafe {
            let _ = self.main.CallDevToolsProtocolMethod(&HSTRING::from("Page.stopScreencast"), &HSTRING::from("{}"), &handler);
        }
    }
}

#[cfg(not(windows))]
pub fn start_screencast(
    _: &WebView,
    _: &WebView,
    _: std::rc::Rc<std::cell::Cell<Option<(f64, f64)>>>,
) -> Option<Screencast> {
    None
}
