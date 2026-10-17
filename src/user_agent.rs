//! User-Agent de Chrome alineado con la versión real de WebView2.
//!
//! Se usa el formato "reducido" de Chrome (`Chrome/<mayor>.0.0.0`), que es el
//! que envía Chrome estable hoy. Al derivar la versión mayor del runtime
//! instalado, la cadena nunca queda desfasada respecto al motor.
//!
//! Limitación conocida: esto no cambia las Client Hints (`Sec-CH-UA`,
//! `navigator.userAgentData`), que pueden seguir identificando WebView2.

fn major(runtime_version: &str) -> Option<u32> {
    runtime_version.trim().split('.').next()?.parse().ok()
}

/// Construye el User-Agent a partir de una versión tipo `"133.0.3065.92"`.
pub fn chrome_user_agent(runtime_version: &str) -> Option<String> {
    let major = major(runtime_version)?;
    Some(format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"
    ))
}

/// User-Agent de Edge (lo que WebView2 es de verdad), en formato reducido.
/// Los servicios con DRM lo usan para elegir el sistema de protección que
/// el motor ofrece.
pub fn edge_user_agent(runtime_version: &str) -> Option<String> {
    let major = major(runtime_version)?;
    Some(format!("{} Edg/{major}.0.0.0", chrome_user_agent(runtime_version)?))
}

/// Los dos User-Agent que usa la app, calculados una vez al arrancar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Agents {
    pub chrome: Option<String>,
    pub edge: Option<String>,
}

impl Agents {
    pub fn new(runtime_version: &str) -> Self {
        Self { chrome: chrome_user_agent(runtime_version), edge: edge_user_agent(runtime_version) }
    }

    pub fn for_agent(&self, agent: crate::services::Agent) -> Option<&str> {
        match agent {
            crate::services::Agent::Chrome => self.chrome.as_deref(),
            crate::services::Agent::Edge => self.edge.as_deref(),
        }
    }
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
    fn builds_reduced_edge_ua() {
        assert_eq!(
            edge_user_agent("141.0.3537.71").as_deref(),
            Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0"
            )
        );
        let agents = Agents::new("141.0.1");
        assert!(agents.for_agent(crate::services::Agent::Chrome).unwrap().ends_with("Safari/537.36"));
        assert!(agents.for_agent(crate::services::Agent::Edge).unwrap().ends_with("Edg/141.0.0.0"));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(chrome_user_agent(""), None);
        assert_eq!(chrome_user_agent("abc.1"), None);
        assert_eq!(edge_user_agent(""), None);
    }
}
