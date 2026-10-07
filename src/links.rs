//! Convierte texto del portapapeles en una URL de YouTube segura de abrir.
//!
//! Solo se aceptan dominios de YouTube; cualquier otra cosa se rechaza para
//! que "abrir enlace copiado" no sirva para navegar a sitios arbitrarios.

use url::Url;

const VIDEO_ID_LEN: usize = 11;

/// Devuelve la URL a abrir, o `None` si el texto no es un enlace de YouTube.
///
/// Acepta:
/// - `youtube.com`, `www.`, `m.` y `music.youtube.com` (http o https).
/// - `youtu.be/<id>` (con `t=` y `list=` si los trae).
/// - Un id de video suelto de 11 caracteres.
pub fn normalize(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.len() > 2048 {
        return None;
    }

    if is_video_id(text) {
        return Some(format!("https://www.youtube.com/watch?v={text}"));
    }

    // Permite pegar "youtube.com/watch?v=..." sin esquema.
    let with_scheme;
    let candidate = if text.contains("://") {
        text
    } else {
        with_scheme = format!("https://{text}");
        &with_scheme
    };

    let mut url = Url::parse(candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_scheme("https").ok()?;
    let host = url.host_str()?.to_ascii_lowercase();

    match host.as_str() {
        "youtube.com" | "www.youtube.com" | "m.youtube.com" => {
            url.set_host(Some("www.youtube.com")).ok()?;
            Some(url.into())
        }
        "music.youtube.com" => Some(url.into()),
        "youtu.be" | "www.youtu.be" => {
            let id = url.path().trim_start_matches('/');
            if !is_video_id(id) {
                return None;
            }
            let mut watch = Url::parse("https://www.youtube.com/watch").ok()?;
            {
                let mut query = watch.query_pairs_mut();
                query.append_pair("v", id);
                for (key, value) in url.query_pairs() {
                    // `si` es un parámetro de seguimiento: se descarta.
                    if matches!(key.as_ref(), "t" | "list" | "index") {
                        query.append_pair(&key, &value);
                    }
                }
            }
            Some(watch.into())
        }
        _ => None,
    }
}

fn is_video_id(s: &str) -> bool {
    s.len() == VIDEO_ID_LEN && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_urls() {
        assert_eq!(
            normalize("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s").as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s")
        );
        assert_eq!(
            normalize("  http://m.youtube.com/watch?v=dQw4w9WgXcQ \n").as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
        );
        assert_eq!(
            normalize("youtube.com/playlist?list=PL123").as_deref(),
            Some("https://www.youtube.com/playlist?list=PL123")
        );
    }

    #[test]
    fn short_links_keep_time_and_list_but_drop_tracking() {
        assert_eq!(
            normalize("https://youtu.be/dQw4w9WgXcQ?si=abc&t=90").as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=90")
        );
        assert_eq!(
            normalize("youtu.be/dQw4w9WgXcQ?list=PLx").as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PLx")
        );
        assert_eq!(normalize("https://youtu.be/short"), None);
    }

    #[test]
    fn music_and_bare_ids() {
        assert_eq!(
            normalize("https://music.youtube.com/watch?v=abcdefghijk").as_deref(),
            Some("https://music.youtube.com/watch?v=abcdefghijk")
        );
        assert_eq!(
            normalize("dQw4w9WgXcQ").as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
        );
    }

    #[test]
    fn rejects_other_sites_and_schemes() {
        for bad in [
            "",
            "hola mundo",
            "https://evil.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com.evil.com/",
            "https://notyoutube.com/",
            "javascript:alert(1)",
            "file:///C:/Windows",
            "ftp://youtube.com/",
            "chrome-extension://abc/options.html",
        ] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
    }
}
