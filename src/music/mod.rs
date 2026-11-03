//! "My music": local library, downloads of openly licensed music from the
//! Internet Archive, playlists and finding repeated songs.
//!
//! - `library`: model and duplicates (pure).
//! - `scan`: reading files (tags, cover, content hash).
//! - `fingerprint`: acoustic fingerprint (Chromaprint).
//! - `archive`: Internet Archive search and albums (open licenses only).
//! - `download`: queue of resumable, verified downloads.
//! - `serve`: songs and covers for the page (own protocol).
//! - `service`: the thread that coordinates it all.
//! - `os`: Windows dialogs and Recycle Bin.

pub mod archive;
pub mod download;
pub mod fingerprint;
pub mod library;
pub mod os;
pub mod scan;
pub mod serve;
pub mod service;

pub use service::{start, Config, Handle, Request, DOWNLOADS_FOLDER};
