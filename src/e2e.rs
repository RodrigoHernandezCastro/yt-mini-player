//! The drivable test build, for the end-to-end tests on real Windows
//! (`cargo build --release --features e2e`, tests/windows_e2e.py).
//!
//! Only with the feature, and only when the variables are set:
//! - `YTMP_E2E_PORT=<port>`: WebView2's DevTools port on 127.0.0.1, so
//!   Playwright drives the app's real windows over CDP (start screen,
//!   Settings, Watch together...);
//! - `YTMP_E2E_DATA=<folder>`: a data folder of its own and an instance of
//!   its own, so several copies run side by side (a host and a guest) and
//!   the tester's real session is never touched.
//!
//! A release never has it: `tools/package.ps1` and `release.yml` refuse an
//! .exe that carries [`MARKER`] (only this build writes it, in the diary).
//! Without the feature every function here says "not set".

use std::path::PathBuf;

/// Written to the diary at start; looked for in the release .exe files.
#[cfg(feature = "e2e")]
pub const MARKER: &str = "YTMP-E2E-BUILD-NOT-FOR-RELEASE";

/// The DevTools port, if this is the test build and one was asked for.
pub fn port() -> Option<u16> {
    if cfg!(feature = "e2e") {
        parse_port(&std::env::var("YTMP_E2E_PORT").ok()?)
    } else {
        None
    }
}

/// The data folder of its own, if this is the test build and one was given.
pub fn data_dir() -> Option<PathBuf> {
    if cfg!(feature = "e2e") {
        parse_dir(&std::env::var("YTMP_E2E_DATA").ok()?)
    } else {
        None
    }
}

/// A name for this copy's single-instance event (one per data folder).
pub fn instance_name() -> Option<String> {
    data_dir().map(|dir| instance_for(&dir))
}

/// A port above the privileged range.
fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|port| *port >= 1024)
}

/// An absolute folder (a relative one would depend on where it started).
fn parse_dir(text: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(text.trim());
    (!text.trim().is_empty() && (dir.is_absolute() || dir.has_root())).then_some(dir)
}

fn instance_for(dir: &std::path::Path) -> String {
    // FNV-1a of the folder: short, stable, and free of path characters.
    let hash = dir.to_string_lossy().to_lowercase().bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("e2e-{hash:016x}")
}

/// The DevTools argument for WebView2's browser process.
pub fn browser_arg() -> Option<String> {
    port().map(|port| format!(" --remote-debugging-port={port} --remote-debugging-address=127.0.0.1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_checked() {
        assert_eq!(parse_port("9333"), Some(9333));
        assert_eq!(parse_port(" 9333 "), Some(9333));
        for bad in ["80", "0", "-1", "70000", "x", ""] {
            assert_eq!(parse_port(bad), None, "{bad}");
        }
        assert!(parse_dir("relative\\folder").is_none());
        assert!(parse_dir("").is_none());
        assert!(parse_dir("/tmp/ytmp-a").is_some());
        let a = instance_for(std::path::Path::new("/tmp/ytmp-a"));
        assert_eq!(a, instance_for(std::path::Path::new("/TMP/YTMP-A")), "the same folder, the same instance");
        assert_ne!(a, instance_for(std::path::Path::new("/tmp/ytmp-b")));
        assert!(a.starts_with("e2e-") && a.len() == 20 && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }

    #[test]
    fn without_the_feature_nothing_is_on() {
        if cfg!(feature = "e2e") {
            return;
        }
        std::env::set_var("YTMP_E2E_PORT", "9333");
        std::env::set_var("YTMP_E2E_DATA", "/tmp/ytmp-a");
        assert_eq!((port(), data_dir(), instance_name(), browser_arg()), (None, None, None, None));
        std::env::remove_var("YTMP_E2E_PORT");
        std::env::remove_var("YTMP_E2E_DATA");
    }
}

#[cfg(test)]
#[path = "tests/e2e_cases.rs"]
mod cases;
