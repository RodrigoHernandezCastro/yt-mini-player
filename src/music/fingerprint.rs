//! Huella acústica (Chromaprint, el algoritmo de AcoustID) para reconocer la
//! misma grabación aunque cambien el formato, la calidad o las etiquetas.
//!
//! Se decodifican los primeros `SECONDS` segundos con Symphonia (Rust puro)
//! y se calcula la huella con rusty-chromaprint. Dos huellas son del mismo
//! audio si coinciden en la mayor parte del tramo comparado.

use std::{fs::File, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine};
use rusty_chromaprint::{match_fingerprints, Configuration, Fingerprinter};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{DecoderOptions, CODEC_TYPE_NULL},
    errors::Error as SymphoniaError,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

/// Segundos que se analizan desde el principio.
pub const SECONDS: u32 = 60;
/// Parte del tramo comparado que debe coincidir.
const MIN_COVERAGE: f32 = 0.6;
/// Huellas de menos de esto (≈ 5 s) no dicen nada.
const MIN_ITEMS: usize = 40;

fn config() -> Configuration {
    Configuration::preset_test2()
}

/// Calcula la huella de un archivo.
pub fn compute(path: &Path) -> Result<Vec<u32>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("formato no reconocido: {e}"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("sin pista de audio")?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.ok_or("frecuencia desconocida")?;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2).clamp(1, 8);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("códec no admitido: {e}"))?;

    let config = config();
    let mut printer = Fingerprinter::new(&config);
    printer.start(rate, channels as u32).map_err(|e| format!("{e:?}"))?;
    let limit = u64::from(rate) * u64::from(SECONDS) * channels as u64;
    let mut consumed = 0u64;
    let mut buffer: Option<SampleBuffer<i16>> = None;
    while consumed < limit {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // Un paquete dañado no invalida la huella.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let spec = *decoded.spec();
        if spec.channels.count() != channels {
            return Err("el número de canales cambia a mitad del archivo".into());
        }
        let buf = buffer.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if buf.capacity() < decoded.capacity() * channels {
            *buf = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        buf.copy_interleaved_ref(decoded);
        let samples = buf.samples();
        let take = samples.len().min((limit - consumed) as usize);
        printer.consume(&samples[..take]);
        consumed += take as u64;
    }
    printer.finish();
    let fingerprint = printer.fingerprint().to_vec();
    if fingerprint.len() < MIN_ITEMS {
        return Err("audio demasiado corto".into());
    }
    Ok(fingerprint)
}

/// ¿Son el mismo audio?
pub fn same_audio(a: &[u32], b: &[u32]) -> bool {
    if a.len() < MIN_ITEMS || b.len() < MIN_ITEMS {
        return false;
    }
    let Ok(segments) = match_fingerprints(a, b, &config()) else { return false };
    let matched: usize = segments.iter().map(|s| s.items_count).sum();
    let shorter = a.len().min(b.len());
    matched as f32 >= shorter as f32 * MIN_COVERAGE
}

/// Para guardarla: base64 de los enteros en little-endian.
pub fn encode(fingerprint: &[u32]) -> String {
    let bytes: Vec<u8> = fingerprint.iter().flat_map(|v| v.to_le_bytes()).collect();
    STANDARD.encode(bytes)
}

pub fn decode(text: &str) -> Option<Vec<u32>> {
    let bytes = STANDARD.decode(text).ok()?;
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::scan::tests::{temp_dir, wav};

    #[test]
    fn same_recording_matches_and_different_ones_do_not() {
        let dir = temp_dir("fp");
        let melody = [261.6, 329.6, 392.0, 523.3, 440.0, 349.2, 293.7, 246.9];
        let other = [196.0, 207.7, 233.1, 155.6, 185.0, 277.2, 311.1, 370.0];
        let a = dir.join("a.wav");
        let a_low = dir.join("a_22k.wav");
        let b = dir.join("b.wav");
        std::fs::write(&a, wav(20.0, &melody, 44100)).unwrap();
        // Misma melodía, otra frecuencia de muestreo (como otra codificación).
        std::fs::write(&a_low, wav(20.0, &melody, 22050)).unwrap();
        std::fs::write(&b, wav(20.0, &other, 44100)).unwrap();
        let fa = compute(&a).unwrap();
        let fa2 = compute(&a_low).unwrap();
        let fb = compute(&b).unwrap();
        assert!(fa.len() > MIN_ITEMS, "{}", fa.len());
        assert!(same_audio(&fa, &fa2), "misma grabación, distinta calidad");
        assert!(same_audio(&fa, &fa));
        assert!(!same_audio(&fa, &fb), "otra canción");
        assert_eq!(decode(&encode(&fa)).unwrap(), fa);
        assert_eq!(decode("no es base64!"), None);
        assert!(compute(&dir.join("no-existe.wav")).is_err());
        std::fs::write(dir.join("corto.wav"), wav(1.0, &melody, 44100)).unwrap();
        assert!(compute(&dir.join("corto.wav")).is_err(), "demasiado corto");
        let _ = std::fs::remove_dir_all(dir);
    }
}
