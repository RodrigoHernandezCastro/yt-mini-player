//! Watch together: the connection to the relay.
//!
//! A WebSocket in its own thread. On Windows `wss://` goes through
//! native-tls, which is SChannel: the system's certificates and policies,
//! never without verification. `ws://` is only accepted for this computer
//! (a relay being tried out with `wrangler dev`).
//!
//! If the connection drops after it was up, it tries again (1, 2, 4, 8 s…)
//! for a while, with the address the session gives it for coming back. A
//! refusal from the relay (an HTTP status, or closing with a code from 4000)
//! is final.

use std::{
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use tungstenite::{
    client::IntoClientRequest,
    handshake::HandshakeError,
    http::HeaderValue,
    protocol::WebSocketConfig,
    Error, Message, WebSocket,
};
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// "ping" this often; the relay answers "pong" itself.
const KEEPALIVE: Duration = Duration::from_secs(25);
/// Nothing at all for this long: the connection is dead.
const SILENCE: Duration = Duration::from_secs(70);
/// When idle, how long to wait before looking again.
const IDLE: Duration = Duration::from_millis(20);
const CLOSE_WAIT: Duration = Duration::from_secs(2);
const MAX_MESSAGE: usize = 64 * 1024;
const USER_AGENT: &str = concat!("YTMiniPlayer/", env!("CARGO_PKG_VERSION"), " (watch together)");

/// Where to connect: a path under the relay's address and its headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dial {
    pub path: String,
    pub headers: Vec<(&'static str, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    Up,
    /// A message from the relay.
    Frame(String),
    /// Lost; trying again.
    Down,
    /// Over for good.
    Failed(Failure),
    /// Closed as asked.
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The relay said no (HTTP status).
    Refused(u16),
    /// The relay closed it with this code (4000 ended, 4001 removed,
    /// 4002 replaced).
    ClosedBy(u16),
    /// The relay could not be reached at all.
    Unreachable(String),
    /// Lost and not back in time.
    Lost,
}

enum Cmd {
    Send(String),
    Redial(Dial),
    Close,
}

pub struct Link {
    tx: Sender<Cmd>,
    done: Receiver<()>,
}

impl Link {
    /// Connects in the background; `events` gets everything that happens.
    pub fn start(relay: Url, dial: Dial, retry_for: Duration, events: impl Fn(LinkEvent) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let spawned = thread::Builder::new().name("together".into()).spawn(move || {
            run(&relay, dial, retry_for, &rx, &events);
            let _ = done_tx.send(());
        });
        if let Err(err) = spawned {
            eprintln!("could not start the watch together connection: {err}");
        }
        Self { tx, done }
    }

    pub fn send(&self, text: String) {
        let _ = self.tx.send(Cmd::Send(text));
    }

    /// The address for coming back if the connection drops.
    pub fn redial(&self, dial: Dial) {
        let _ = self.tx.send(Cmd::Redial(dial));
    }

    /// Sends what is queued, closes, and waits for that at most `wait`.
    pub fn close(self, wait: Duration) {
        let _ = self.tx.send(Cmd::Close);
        let _ = self.done.recv_timeout(wait);
    }
}

/// `relay` + `path`: the relay's address may have a path of its own.
pub fn url_for(relay: &Url, path: &str) -> String {
    format!("{}{}", relay.as_str().trim_end_matches('/'), path)
}

/// A relay address the app accepts: `wss://host[/path]`, or `ws://` to this
/// computer.
pub fn parse_relay(text: &str) -> Option<Url> {
    let text = text.trim();
    if text.is_empty() || text.len() > 512 {
        return None;
    }
    let with_scheme;
    let candidate = if text.contains("://") {
        text
    } else {
        with_scheme = format!("wss://{text}");
        &with_scheme
    };
    let mut url = Url::parse(candidate).ok()?;
    // https:// copied from the browser: the same server.
    if url.scheme() == "https" {
        url = Url::parse(&candidate.replacen("https://", "wss://", 1)).ok()?;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
    let ok_scheme = url.scheme() == "wss" || (url.scheme() == "ws" && local);
    if !ok_scheme || url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if !local && !host.contains('.') {
        return None;
    }
    Some(url)
}

// ---------------------------------------------------------------------------
// The thread
// ---------------------------------------------------------------------------

enum Exit {
    Quit,
    Lost,
    Failed(Failure),
}

fn run(relay: &Url, mut dial: Dial, retry_for: Duration, rx: &Receiver<Cmd>, events: &dyn Fn(LinkEvent)) {
    let mut ever_up = false;
    let mut down_since: Option<Instant> = None;
    let mut attempt = 0u32;
    loop {
        match connect(relay, &dial) {
            Ok(mut ws) => {
                ever_up = true;
                attempt = 0;
                down_since = None;
                events(LinkEvent::Up);
                match pump(&mut ws, rx, &mut dial, events) {
                    Exit::Quit => {
                        events(LinkEvent::Closed);
                        return;
                    }
                    Exit::Failed(failure) => {
                        events(LinkEvent::Failed(failure));
                        return;
                    }
                    Exit::Lost => events(LinkEvent::Down),
                }
            }
            Err(Exit::Failed(failure)) => {
                // Unreachable (or a passing server error) on the first try is
                // said at once; later, it is one more try.
                let passing = matches!(failure, Failure::Unreachable(_) | Failure::Refused(500..=599));
                if !ever_up || !passing {
                    events(LinkEvent::Failed(failure));
                    return;
                }
            }
            Err(_) => {}
        }
        let since = *down_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= retry_for {
            events(LinkEvent::Failed(Failure::Lost));
            return;
        }
        let wait = Duration::from_secs(1u64 << attempt.min(3));
        attempt += 1;
        if wait_or_quit(rx, &mut dial, wait) {
            events(LinkEvent::Closed);
            return;
        }
    }
}

/// Waits before trying again; `true` if asked to close meanwhile.
fn wait_or_quit(rx: &Receiver<Cmd>, dial: &mut Dial, wait: Duration) -> bool {
    let until = Instant::now() + wait;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        match rx.recv_timeout(left) {
            Ok(Cmd::Redial(d)) => *dial = d,
            // Nobody can hear it now; the session sends what matters again.
            Ok(Cmd::Send(_)) => {}
            Ok(Cmd::Close) | Err(RecvTimeoutError::Disconnected) => return true,
            Err(RecvTimeoutError::Timeout) => return false,
        }
    }
}

fn would_block(err: &Error) -> bool {
    matches!(err, Error::Io(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted))
}

fn pump(ws: &mut WebSocket<Stream>, rx: &Receiver<Cmd>, dial: &mut Dial, events: &dyn Fn(LinkEvent)) -> Exit {
    let mut last_heard = Instant::now();
    let mut last_ping = Instant::now();
    let mut closing: Option<Instant> = None;
    loop {
        loop {
            match rx.try_recv() {
                Ok(Cmd::Send(text)) => {
                    if closing.is_none() {
                        if let Err(err) = ws.write(Message::text(text)) {
                            if !would_block(&err) {
                                return Exit::Lost;
                            }
                        }
                    }
                }
                Ok(Cmd::Redial(d)) => *dial = d,
                Ok(Cmd::Close) | Err(TryRecvError::Disconnected) => {
                    if closing.is_none() {
                        let _ = ws.close(None);
                        closing = Some(Instant::now());
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        match ws.flush() {
            Ok(()) => {}
            Err(err) if would_block(&err) => {}
            Err(_) => return if closing.is_some() { Exit::Quit } else { Exit::Lost },
        }
        let mut idle = true;
        match ws.read() {
            Ok(Message::Text(text)) => {
                idle = false;
                last_heard = Instant::now();
                if text.as_str() != "pong" {
                    events(LinkEvent::Frame(text.as_str().to_owned()));
                }
            }
            Ok(Message::Close(frame)) => {
                if closing.is_none() {
                    let code = frame.map(|f| u16::from(f.code)).unwrap_or(1005);
                    let _ = ws.flush();
                    return if (4000..5000).contains(&code) { Exit::Failed(Failure::ClosedBy(code)) } else { Exit::Lost };
                }
            }
            Ok(_) => {
                idle = false;
                last_heard = Instant::now();
            }
            Err(err) if would_block(&err) => {}
            Err(_) => return if closing.is_some() { Exit::Quit } else { Exit::Lost },
        }
        if closing.is_some_and(|at| at.elapsed() >= CLOSE_WAIT) {
            return Exit::Quit;
        }
        if closing.is_none() && last_ping.elapsed() >= KEEPALIVE {
            last_ping = Instant::now();
            if let Err(err) = ws.write(Message::text("ping")) {
                if !would_block(&err) {
                    return Exit::Lost;
                }
            }
        }
        if closing.is_none() && last_heard.elapsed() >= SILENCE {
            return Exit::Lost;
        }
        if idle {
            thread::sleep(IDLE);
        }
    }
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

enum Stream {
    Plain(TcpStream),
    #[cfg(windows)]
    Tls(Box<native_tls::TlsStream<TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(tcp) => tcp,
            #[cfg(windows)]
            Stream::Tls(tls) => tls.get_ref(),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            #[cfg(windows)]
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            #[cfg(windows)]
            Stream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            #[cfg(windows)]
            Stream::Tls(s) => s.flush(),
        }
    }
}

fn unreachable(err: impl std::fmt::Display) -> Exit {
    Exit::Failed(Failure::Unreachable(err.to_string()))
}

fn connect(relay: &Url, dial: &Dial) -> Result<WebSocket<Stream>, Exit> {
    let address = url_for(relay, &dial.path);
    let url = Url::parse(&address).map_err(unreachable)?;
    let host = url.host_str().ok_or_else(|| unreachable("no host"))?.trim_matches(|c| c == '[' || c == ']').to_owned();
    let port = url.port_or_known_default().ok_or_else(|| unreachable("no port"))?;
    let mut last = io::Error::other("no address");
    let mut tcp = None;
    for addr in (host.as_str(), port).to_socket_addrs().map_err(unreachable)? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(err) => last = err,
        }
    }
    let tcp = tcp.ok_or_else(|| unreachable(last))?;
    tcp.set_read_timeout(Some(CONNECT_TIMEOUT)).map_err(unreachable)?;
    tcp.set_write_timeout(Some(CONNECT_TIMEOUT)).map_err(unreachable)?;
    let _ = tcp.set_nodelay(true);
    let stream = match url.scheme() {
        "ws" => Stream::Plain(tcp),
        #[cfg(windows)]
        "wss" => {
            let tls = native_tls::TlsConnector::builder()
                .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
                .build()
                .map_err(unreachable)?;
            Stream::Tls(Box::new(tls.connect(&host, tcp).map_err(unreachable)?))
        }
        other => return Err(unreachable(format!("{other}:// is not supported here"))),
    };
    let mut request = address.as_str().into_client_request().map_err(unreachable)?;
    let headers = request.headers_mut();
    headers.insert("User-Agent", HeaderValue::from_static(USER_AGENT));
    for (name, value) in &dial.headers {
        headers.insert(*name, HeaderValue::from_str(value).map_err(unreachable)?);
    }
    let config = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE));
    match tungstenite::client::client_with_config(request, stream, Some(config)) {
        Ok((ws, _)) => {
            // From now on reads do not block: the thread also has to send.
            ws.get_ref().tcp().set_nonblocking(true).map_err(unreachable)?;
            Ok(ws)
        }
        Err(HandshakeError::Failure(Error::Http(response))) => Err(Exit::Failed(Failure::Refused(response.status().as_u16()))),
        Err(err) => Err(unreachable(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_addresses() {
        assert_eq!(parse_relay("ytmp-together.me.workers.dev").unwrap().as_str(), "wss://ytmp-together.me.workers.dev/");
        assert_eq!(parse_relay("https://relay.example.org/base").unwrap().as_str(), "wss://relay.example.org/base");
        assert_eq!(parse_relay("ws://127.0.0.1:8799").unwrap().as_str(), "ws://127.0.0.1:8799/");
        assert!(parse_relay("ws://localhost:8787").is_some());
        for bad in [
            "",
            "ws://relay.example.org",
            "http://relay.example.org",
            "wss://relay",
            "wss://user:pass@relay.example.org",
            "wss://relay.example.org/?x=1",
            "file:///C:/x",
            "javascript:alert(1)",
        ] {
            assert!(parse_relay(bad).is_none(), "{bad}");
        }
        let relay = parse_relay("wss://relay.example.org/base/").unwrap();
        assert_eq!(url_for(&relay, "/v1/health"), "wss://relay.example.org/base/v1/health");
    }
}
