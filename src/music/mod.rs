//! "Mi música": biblioteca local, descargas de música libre del Internet
//! Archive, playlists y detección de canciones repetidas.
//!
//! - `library`: modelo y duplicados (puro).
//! - `scan`: leer archivos (etiquetas, portada, huella del contenido).
//! - `fingerprint`: huella acústica (Chromaprint).
//! - `archive`: búsqueda y álbumes del Internet Archive (solo licencias libres).
//! - `download`: cola de descargas reanudables y verificadas.
//! - `serve`: canciones y portadas para la página (protocolo propio).
//! - `service`: el hilo que lo coordina todo.
//! - `os`: diálogos y Papelera de Windows.

pub mod archive;
pub mod download;
pub mod fingerprint;
pub mod library;
pub mod os;
pub mod scan;
pub mod serve;
pub mod service;

pub use service::{start, Config, Handle, Request, DOWNLOADS_FOLDER};
