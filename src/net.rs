//! The app's HTTP client (Internet Archive and Lemon Squeezy).
//!
//! - On Windows it uses **SChannel**, the system's TLS stack: the same root
//!   certificates, revocation and policies as Edge. Verification is never
//!   turned off.
//! - A descriptive User-Agent (the Internet Archive's usage rules ask for it).
//! - Retries with growing waits on 429/5xx, honouring `Retry-After`.
//! - Size-bounded JSON responses; downloads to a `.part` file that resumes
//!   with `Range` if cut off.

use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use serde::de::DeserializeOwned;

pub const USER_AGENT: &str = concat!(
    "YTMiniPlayer/",
    env!("CARGO_PKG_VERSION"),
    " (Windows desktop music player; library downloads of openly licensed audio)"
);
/// No JSON the app asks for comes near this.
const MAX_JSON: u64 = 16 * 1024 * 1024;
/// An audio file bigger than this is not a song.
pub const MAX_DOWNLOAD: u64 = 2 * 1024 * 1024 * 1024;
const RETRIES: u32 = 3;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// No connection, DNS, TLS, timeout...
    Offline(String),
    /// An error response; includes the (bounded) body in case it has JSON.
    Status(u16, String),
    Cancelled,
    /// An unexpected response (invalid JSON, size).
    Invalid(String),
    /// The file arrived damaged (wrong size or checksum).
    Corrupt(String),
    /// A disk error.
    Io(String),
}

/// For logs (the user sees [`NetError::message`]).
impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Offline(e) => write!(f, "offline ({e})"),
            NetError::Status(code, _) => write!(f, "the server answered {code}"),
            NetError::Cancelled => write!(f, "cancelled"),
            NetError::Invalid(e) => write!(f, "invalid response: {e}"),
            NetError::Corrupt(e) => write!(f, "damaged file: {e}"),
            NetError::Io(e) => write!(f, "disk error: {e}"),
        }
    }
}

impl NetError {
    /// What went wrong, for the user, in the language the page shows.
    pub fn message(&self) -> crate::i18n::Message {
        match self {
            NetError::Offline(_) => msg!("net.offline"),
            NetError::Status(429, _) => msg!("net.busy"),
            NetError::Status(code, _) => msg!("net.status", code = *code),
            NetError::Cancelled => msg!("net.cancelled"),
            NetError::Invalid(_) => msg!("net.invalid"),
            NetError::Corrupt(_) => msg!("net.corrupt"),
            NetError::Io(e) => msg!("net.disk", error = e.as_str()),
        }
    }

    fn retryable(&self) -> bool {
        matches!(self, NetError::Offline(_)) || matches!(self, NetError::Status(code, _) if *code == 429 || *code >= 500)
    }
}

pub struct Http {
    agent: ureq::Agent,
}

/// Wait before retry `attempt` (0, 1, 2...): 1 s, 2 s, 4 s... or whatever
/// the server asks for with `Retry-After` (in seconds), up to a cap.
pub fn backoff(attempt: u32, retry_after: Option<&str>) -> Duration {
    if let Some(secs) = retry_after.and_then(|v| v.trim().parse::<u64>().ok()) {
        return Duration::from_secs(secs).min(MAX_RETRY_AFTER);
    }
    Duration::from_secs(1u64 << attempt.min(4)).min(MAX_RETRY_AFTER)
}

impl Http {
    pub fn new() -> Result<Self, String> {
        let builder = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30))
            .timeout_write(Duration::from_secs(30))
            .redirects(5)
            // Never plain http, not even after a redirect (a downgrade would
            // let anyone on the network change the metadata or the license
            // answers). The tests' local server is the only exception.
            .https_only(!cfg!(test))
            .user_agent(USER_AGENT);
        #[cfg(windows)]
        let builder = {
            let tls = native_tls::TlsConnector::builder()
                .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
                .build()
                .map_err(|e| e.to_string())?;
            builder.tls_connector(std::sync::Arc::new(tls))
        };
        Ok(Self { agent: builder.build() })
    }

    /// A GET that returns JSON. Retries on passing failures.
    pub fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, NetError> {
        self.with_retries(|| {
            let response = self.agent.get(url).set("Accept", "application/json").call();
            let body = read_body(response)?;
            serde_json::from_slice(&body).map_err(|e| NetError::Invalid(e.to_string()))
        })
    }

    /// A form POST; returns the code and the JSON even for an error (Lemon
    /// Squeezy explains errors in the body).
    pub fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<(u16, serde_json::Value), NetError> {
        let response = self.agent.post(url).set("Accept", "application/json").send_form(form);
        let (status, body) = match response {
            Ok(r) => (r.status(), read_limited(r.into_reader(), MAX_JSON)?),
            Err(ureq::Error::Status(code, r)) => (code, read_limited(r.into_reader(), MAX_JSON).unwrap_or_default()),
            Err(ureq::Error::Transport(t)) => return Err(NetError::Offline(t.to_string())),
        };
        if status == 429 || status >= 500 {
            return Err(NetError::Status(status, String::from_utf8_lossy(&body).chars().take(300).collect()));
        }
        let json = serde_json::from_slice(&body).map_err(|e| NetError::Invalid(e.to_string()))?;
        Ok((status, json))
    }

    /// Downloads `url` to `part`, continuing whatever was there. Returns the
    /// final size. `progress` receives the bytes on disk.
    pub fn download(
        &self,
        url: &str,
        part: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, NetError> {
        self.with_retries(|| self.download_once(url, part, cancel, progress))
    }

    fn download_once(
        &self,
        url: &str,
        part: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, NetError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(NetError::Cancelled);
        }
        let have = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        let mut request = self.agent.get(url);
        if have > 0 {
            request = request.set("Range", &format!("bytes={have}-"));
        }
        let response = match request.call() {
            Ok(r) => r,
            // It was already complete.
            Err(ureq::Error::Status(416, _)) if have > 0 => return Ok(have),
            Err(ureq::Error::Status(code, r)) => {
                let body = read_limited(r.into_reader(), 4096).unwrap_or_default();
                return Err(NetError::Status(code, String::from_utf8_lossy(&body).into_owned()));
            }
            Err(ureq::Error::Transport(t)) => return Err(NetError::Offline(t.to_string())),
        };
        // 206 = continues; 200 = the server sends everything again.
        let resume = response.status() == 206 && have > 0;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(resume)
            .truncate(!resume)
            .open(part)
            .map_err(|e| NetError::Io(e.to_string()))?;
        let mut written = if resume { have } else { 0 };
        progress(written);
        let mut reader = response.into_reader();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(NetError::Cancelled);
            }
            let n = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(NetError::Offline(e.to_string())),
            };
            written += n as u64;
            if written > MAX_DOWNLOAD {
                return Err(NetError::Corrupt("file too large".into()));
            }
            file.write_all(&buffer[..n]).map_err(|e| NetError::Io(e.to_string()))?;
            progress(written);
        }
        file.flush().map_err(|e| NetError::Io(e.to_string()))?;
        Ok(written)
    }

    fn with_retries<T>(&self, mut attempt: impl FnMut() -> Result<T, NetError>) -> Result<T, NetError> {
        let mut tries = 0;
        loop {
            match attempt() {
                Err(err) if err.retryable() && tries < RETRIES => {
                    let retry_after = match &err {
                        NetError::Status(_, body) if body.starts_with("retry-after:") => {
                            body.strip_prefix("retry-after:").map(str::to_owned)
                        }
                        _ => None,
                    };
                    thread::sleep(backoff(tries, retry_after.as_deref()));
                    tries += 1;
                }
                other => return other,
            }
        }
    }
}

/// The response body (bounded). On a 429/503 `Retry-After` is kept so the
/// retry waits as long as the server asks.
fn read_body(response: Result<ureq::Response, ureq::Error>) -> Result<Vec<u8>, NetError> {
    match response {
        Ok(r) => read_limited(r.into_reader(), MAX_JSON),
        Err(ureq::Error::Status(code, r)) => {
            let body = match r.header("Retry-After") {
                Some(after) => format!("retry-after:{after}"),
                None => String::from_utf8_lossy(&read_limited(r.into_reader(), 4096).unwrap_or_default()).into_owned(),
            };
            Err(NetError::Status(code, body))
        }
        Err(ureq::Error::Transport(t)) => Err(NetError::Offline(t.to_string())),
    }
}

fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>, NetError> {
    let mut body = Vec::new();
    reader.take(limit + 1).read_to_end(&mut body).map_err(|e| NetError::Offline(e.to_string()))?;
    if body.len() as u64 > limit {
        return Err(NetError::Invalid("response too large".into()));
    }
    Ok(body)
}

#[cfg(test)]
pub mod testserver {
    //! A minimal HTTP server for tests (127.0.0.1 only, no TLS).
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    #[derive(Debug, Clone)]
    pub struct Seen {
        pub method: String,
        pub path: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    pub struct Reply {
        pub status: u16,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    impl Reply {
        pub fn json(status: u16, body: &str) -> Self {
            Self { status, headers: vec![("Content-Type".into(), "application/json".into())], body: body.as_bytes().to_vec() }
        }
    }

    pub type Handler = dyn Fn(&Seen) -> Reply + Send + Sync;

    /// Starts the server; returns the base (`http://127.0.0.1:port`) and the
    /// request log.
    pub fn serve(handler: Arc<Handler>) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen_log = log.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let handler = handler.clone();
                let log = seen_log.clone();
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or_default().to_owned();
                    let path = parts.next().unwrap_or_default().to_owned();
                    let mut headers = Vec::new();
                    let mut length = 0usize;
                    loop {
                        let mut h = String::new();
                        reader.read_line(&mut h).unwrap();
                        let h = h.trim_end();
                        if h.is_empty() {
                            break;
                        }
                        if let Some((k, v)) = h.split_once(':') {
                            if k.eq_ignore_ascii_case("content-length") {
                                length = v.trim().parse().unwrap_or(0);
                            }
                            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
                        }
                    }
                    let mut body = vec![0u8; length];
                    reader.read_exact(&mut body).unwrap();
                    let seen = Seen { method, path, headers, body: String::from_utf8_lossy(&body).into_owned() };
                    let reply = handler(&seen);
                    log.lock().unwrap().push(seen);
                    let mut out = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, reply.body.len());
                    for (k, v) in &reply.headers {
                        out.push_str(&format!("{k}: {v}\r\n"));
                    }
                    out.push_str("\r\n");
                    let _ = stream.write_all(out.as_bytes());
                    let _ = stream.write_all(&reply.body);
                });
            }
        });
        (base, log)
    }
}

#[cfg(test)]
mod tests {
    use super::{testserver::*, *};
    use std::sync::{atomic::AtomicUsize, Arc};

    #[test]
    fn backoff_grows_and_honors_retry_after() {
        assert_eq!(backoff(0, None), Duration::from_secs(1));
        assert_eq!(backoff(1, None), Duration::from_secs(2));
        assert_eq!(backoff(2, None), Duration::from_secs(4));
        assert_eq!(backoff(0, Some("7")), Duration::from_secs(7));
        assert_eq!(backoff(0, Some("99999")), MAX_RETRY_AFTER);
        assert_eq!(backoff(9, None), Duration::from_secs(16));
    }

    #[test]
    fn json_with_retry_after_on_429() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let (base, log) = serve(Arc::new(move |_seen: &Seen| {
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                Reply { status: 429, headers: vec![("Retry-After".into(), "1".into())], body: b"slow down".to_vec() }
            } else {
                Reply::json(200, r#"{"ok":true}"#)
            }
        }));
        let http = Http::new().unwrap();
        let value: serde_json::Value = http.get_json(&format!("{base}/x")).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let seen = log.lock().unwrap();
        assert!(seen[0].headers.iter().any(|(k, v)| k == "user-agent" && v.starts_with("YTMiniPlayer/")));
    }

    #[test]
    fn json_errors_are_not_retried_forever() {
        let (base, log) = serve(Arc::new(|_s: &Seen| Reply::json(404, r#"{"error":"no"}"#)));
        let http = Http::new().unwrap();
        let err = http.get_json::<serde_json::Value>(&format!("{base}/x")).unwrap_err();
        assert!(matches!(err, NetError::Status(404, _)), "{err:?}");
        assert_eq!(log.lock().unwrap().len(), 1, "a 404 is not retried");
    }

    #[test]
    fn form_post_returns_error_bodies() {
        let (base, log) = serve(Arc::new(|s: &Seen| {
            if s.body.contains("license_key=BAD") {
                Reply::json(404, r#"{"activated":false,"error":"license_key not found."}"#)
            } else {
                Reply::json(200, r#"{"activated":true}"#)
            }
        }));
        let http = Http::new().unwrap();
        let (status, json) = http.post_form(&format!("{base}/v1/licenses/activate"), &[("license_key", "BAD")]).unwrap();
        assert_eq!(status, 404);
        assert_eq!(json["error"], "license_key not found.");
        let (status, json) = http.post_form(&format!("{base}/v1/licenses/activate"), &[("license_key", "OK"), ("instance_name", "Ana PC")]).unwrap();
        assert_eq!((status, json["activated"].as_bool()), (200, Some(true)));
        let seen = log.lock().unwrap();
        assert_eq!((seen[1].method.as_str(), seen[1].body.as_str()), ("POST", "license_key=OK&instance_name=Ana+PC"));
        assert!(seen[1].headers.iter().any(|(k, v)| k == "accept" && v == "application/json"));
    }

    #[test]
    fn download_resumes_with_range() {
        let content: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let served = content.clone();
        let (base, log) = serve(Arc::new(move |s: &Seen| {
            let range = s.headers.iter().find(|(k, _)| k == "range").map(|(_, v)| v.clone());
            match range.and_then(|r| r.strip_prefix("bytes=").and_then(|r| r.strip_suffix('-')).and_then(|n| n.parse::<usize>().ok())) {
                Some(start) if start >= served.len() => Reply { status: 416, headers: vec![], body: vec![] },
                Some(start) => Reply {
                    status: 206,
                    headers: vec![("Content-Range".into(), format!("bytes {start}-{}/{}", served.len() - 1, served.len()))],
                    body: served[start..].to_vec(),
                },
                None => Reply { status: 200, headers: vec![], body: served.clone() },
            }
        }));
        let dir = std::env::temp_dir().join(format!("ytmp-net-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let part = dir.join("song.mp3.part");
        // The first half already downloaded.
        fs::write(&part, &content[..80_000]).unwrap();
        let http = Http::new().unwrap();
        let cancel = AtomicBool::new(false);
        let mut last = 0;
        let size = http.download(&format!("{base}/download/item/song.mp3"), &part, &cancel, &mut |n| last = n).unwrap();
        assert_eq!(size, 200_000);
        assert_eq!(last, 200_000);
        assert_eq!(fs::read(&part).unwrap(), content);
        assert_eq!(log.lock().unwrap()[0].headers.iter().find(|(k, _)| k == "range").unwrap().1, "bytes=80000-");
        // Complete: 416 → nothing to do.
        assert_eq!(http.download(&format!("{base}/x"), &part, &cancel, &mut |_| {}).unwrap(), 200_000);
        // Cancelled before starting.
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(http.download(&format!("{base}/x"), &dir.join("other.part"), &cancel, &mut |_| {}), Err(NetError::Cancelled));
        assert_eq!(NetError::Status(503, String::new()).message(), msg!("net.status", code = 503u16));
        let _ = fs::remove_dir_all(dir);
    }
}
