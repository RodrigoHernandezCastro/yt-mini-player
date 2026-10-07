//! User-Agent de Chrome alineado con la versión real de WebView2.
//!
//! Se usa el formato "reducido" de Chrome (`Chrome/<mayor>.0.0.0`), que es el
//! que envía Chrome estable hoy. Al derivar la versión mayor del runtime
//! instalado, la cadena nunca queda desfasada respecto al motor.
//!
//! Limitación conocida: esto no cambia las Client Hints (`Sec-CH-UA`,
//! `navigator.userAgentData`), que pueden seguir identificando WebView2.

/// Construye el User-Agent a partir de una versión tipo `"133.0.3065.92"`.
pub fn chrome_user_agent(runtime_version: &str) -> Option<String> {
    let major: u32 = runtime_version.trim().split('.').next()?.parse().ok()?;
    Some(format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_reduced_chrome_ua() {
        assert_eq!(
            chrome_user_agent("133.0.3065.92").as_deref(),
            Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36"
            )
        );
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(chrome_user_agent(""), None);
        assert_eq!(chrome_user_agent("abc.1"), None);
    }
}
