//! Licencias de pago con Lemon Squeezy (API de licencias).
//!
//! - **Gratis**: biblioteca, reproductor, duplicados y **2 playlists**.
//! - **Descargas** (suscripción mensual): descargar canciones y álbumes, y
//!   **5 playlists**.
//! - **De por vida** (pago único): todo, con **playlists ilimitadas**.
//!
//! Cobro **no invasivo**:
//! - Nada se bloquea de golpe ni hay ventanas que interrumpan: los límites
//!   son de cantidad y se explican en línea, donde se usan.
//! - Lo creado nunca se quita: si la suscripción termina, las playlists de
//!   más siguen ahí (se pueden escuchar, editar y borrar); solo no se crean
//!   nuevas por encima del límite.
//! - Casi sin contacto con la tienda: la compra es en el navegador; la app
//!   activa la clave una vez y después solo la comprueba cada 7 días (la
//!   suscripción, que puede cancelarse) o cada 30 (la de por vida, por si
//!   hubo reembolso). Sin conexión, la suscripción sigue 14 días y la de por
//!   vida no caduca nunca. Nada de telemetría ni cuentas.
//!
//! Se comprueba que la clave sea de *esta* tienda y de uno de *estos*
//! productos: la API acepta claves de cualquier tienda. La clave y el estado
//! se guardan cifrados en la bóveda.
//!
//! Los datos de la tienda se fijan al compilar (variables de entorno, ver
//! README). Sin ellos, las funciones de pago quedan bloqueadas y la página
//! lo explica; con `YTMP_DEV_UNLOCK=1` (solo para desarrollo) todo se
//! desbloquea y la página lo avisa en grande.

use serde::{Deserialize, Serialize};

use crate::net::{Http, NetError};

pub const API: &str = "https://api.lemonsqueezy.com";
const DAY: u64 = 24 * 60 * 60;
const REVALIDATE_SUBSCRIPTION: u64 = 7 * DAY;
const REVALIDATE_LIFETIME: u64 = 30 * DAY;
const GRACE_SUBSCRIPTION: u64 = 14 * DAY;
/// Playlists por plan (`None` = sin límite).
pub const FREE_PLAYLISTS: u32 = 2;
pub const DOWNLOADS_PLAYLISTS: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    #[default]
    Free,
    Downloads,
    Lifetime,
}

/// Lo que permite cada plan. Para cambiar qué incluye cada uno, este es el
/// único sitio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlements {
    /// Descargar canciones (sueltas o el álbum entero).
    pub download: bool,
    /// Cuántas playlists se pueden tener (`None` = sin límite).
    pub max_playlists: Option<u32>,
}

impl Entitlements {
    /// ¿Se puede crear una playlist más teniendo ya `existing`?
    pub fn can_add_playlist(&self, existing: usize) -> bool {
        self.max_playlists.is_none_or(|max| existing < max as usize)
    }
}

impl Plan {
    pub fn entitlements(self) -> Entitlements {
        match self {
            Plan::Free => Entitlements { download: false, max_playlists: Some(FREE_PLAYLISTS) },
            Plan::Downloads => Entitlements { download: true, max_playlists: Some(DOWNLOADS_PLAYLISTS) },
            Plan::Lifetime => Entitlements { download: true, max_playlists: None },
        }
    }

    /// Cuánto vale la última validación sin conexión (`None` = siempre).
    fn grace(self) -> Option<u64> {
        match self {
            Plan::Lifetime => None,
            _ => Some(GRACE_SUBSCRIPTION),
        }
    }

    fn revalidate_after(self) -> u64 {
        match self {
            Plan::Lifetime => REVALIDATE_LIFETIME,
            _ => REVALIDATE_SUBSCRIPTION,
        }
    }
}

/// Datos de la tienda (al compilar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreConfig {
    pub store_id: u64,
    /// Productos (o variantes) del plan Descargas.
    pub downloads: Vec<u64>,
    /// Productos (o variantes) del plan De por vida.
    pub lifetime: Vec<u64>,
    pub checkout_downloads: Option<String>,
    pub checkout_lifetime: Option<String>,
    /// Portal de cliente (gestionar o cancelar la suscripción).
    pub portal: Option<String>,
}

fn ids(text: Option<&str>) -> Vec<u64> {
    text.unwrap_or_default().split(',').filter_map(|s| s.trim().parse().ok()).collect()
}

/// Solo https; cualquier otra cosa se ignora.
fn https(text: Option<&str>) -> Option<String> {
    let url = url::Url::parse(text?.trim()).ok()?;
    (url.scheme() == "https" && url.host_str().is_some()).then(|| url.to_string())
}

impl StoreConfig {
    pub fn from_values(
        store: Option<&str>,
        downloads: Option<&str>,
        lifetime: Option<&str>,
        checkout_downloads: Option<&str>,
        checkout_lifetime: Option<&str>,
        portal: Option<&str>,
    ) -> Option<Self> {
        let store_id = store?.trim().parse().ok().filter(|&id: &u64| id > 0)?;
        let config = Self {
            store_id,
            downloads: ids(downloads),
            lifetime: ids(lifetime),
            checkout_downloads: https(checkout_downloads),
            checkout_lifetime: https(checkout_lifetime),
            portal: https(portal),
        };
        (!config.downloads.is_empty() || !config.lifetime.is_empty()).then_some(config)
    }

    /// La de esta compilación.
    pub fn built_in() -> Option<Self> {
        Self::from_values(
            option_env!("YTMP_LS_STORE_ID"),
            option_env!("YTMP_LS_DOWNLOADS_PRODUCTS"),
            option_env!("YTMP_LS_LIFETIME_PRODUCTS"),
            option_env!("YTMP_LS_DOWNLOADS_CHECKOUT"),
            option_env!("YTMP_LS_LIFETIME_CHECKOUT"),
            option_env!("YTMP_LS_PORTAL"),
        )
    }

    /// Plan al que pertenece una compra (por producto o variante).
    pub fn plan_for(&self, product_id: u64, variant_id: u64) -> Option<Plan> {
        let has = |list: &[u64]| list.contains(&product_id) || list.contains(&variant_id);
        if has(&self.lifetime) {
            Some(Plan::Lifetime)
        } else if has(&self.downloads) {
            Some(Plan::Downloads)
        } else {
            None
        }
    }
}

/// ¿Compilación de desarrollo con todo desbloqueado?
pub fn dev_unlock() -> bool {
    option_env!("YTMP_DEV_UNLOCK") == Some("1")
}

/// Lo que se guarda (cifrado) de la licencia activada en este equipo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct LicenseRecord {
    pub key: String,
    pub instance_id: String,
    pub plan: Plan,
    /// `active`, `inactive`, `expired`, `disabled`.
    pub status: String,
    pub product_name: String,
    pub customer_email: String,
    /// Fecha de caducidad (ISO 8601) si la hay.
    pub expires_at: Option<String>,
    /// Última validación correcta (segundos Unix).
    pub validated_at: u64,
}

impl LicenseRecord {
    /// Plan vigente ahora: el de la licencia si está activa, no caducó y la
    /// última validación no es demasiado antigua.
    pub fn effective_plan(&self, now: u64) -> Plan {
        let active = self.status == "active" || self.status == "inactive";
        let expired = self.expires_at.as_deref().and_then(parse_iso8601).is_some_and(|t| t <= now);
        let fresh = self.plan.grace().is_none_or(|grace| now.saturating_sub(self.validated_at) <= grace);
        if active && !expired && fresh {
            self.plan
        } else {
            Plan::Free
        }
    }

    /// ¿Toca comprobarla con la tienda? (Una clave que dejó de valer se
    /// vuelve a comprobar solo si el usuario lo pide.)
    pub fn needs_validation(&self, now: u64) -> bool {
        self.status != "invalid" && now.saturating_sub(self.validated_at) >= self.plan.revalidate_after()
    }
}

/// Correo con la parte local oculta (a***@dominio).
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((user, domain)) if !user.is_empty() => {
            let first: String = user.chars().take(1).collect();
            format!("{first}***@{domain}")
        }
        _ => String::new(),
    }
}

/// Clave con todo oculto menos el final (para mostrarla).
pub fn mask_key(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    if key.chars().count() <= 4 {
        "****".into()
    } else {
        format!("****-{tail}")
    }
}

/// Formato razonable de clave (las de Lemon Squeezy son UUID en mayúsculas;
/// se aceptan otras formas por si cambian, pero sin espacios raros).
pub fn plausible_key(key: &str) -> bool {
    let len = key.len();
    (8..=128).contains(&len) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// ISO 8601 (`2026-01-24T14:15:07.000000Z`) a segundos Unix (UTC).
pub fn parse_iso8601(text: &str) -> Option<u64> {
    let text = text.trim();
    let (date, time) = text.split_once(['T', ' '])?;
    let mut d = date.split('-');
    let (y, m, day): (i64, u32, u32) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    let time = time.trim_end_matches('Z');
    let time = time.split(['+']).next()?;
    let mut t = time.split(':');
    let (hh, mm): (u64, u64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    let ss: u64 = t.next().and_then(|s| s.split('.').next()).and_then(|s| s.parse().ok()).unwrap_or(0);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Días desde 1970-01-01 (algoritmo de Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days).ok().map(|days| days * DAY + hh * 3600 + mm * 60 + ss)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseError {
    /// No hay conexión: se mantiene lo que había (dentro del margen).
    Offline(String),
    /// La tienda dice que no (o la clave no es de esta app). Mensaje para el
    /// usuario.
    Rejected(String),
}

impl std::fmt::Display for LicenseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LicenseError::Offline(e) => write!(f, "No se pudo contactar con la tienda ({e}). Inténtalo más tarde."),
            LicenseError::Rejected(e) => f.write_str(e),
        }
    }
}

/// Traduce los mensajes de error conocidos de la API.
fn friendly(error: &str, status: &str) -> String {
    let e = error.to_ascii_lowercase();
    if status == "expired" || e.contains("expired") {
        "La licencia caducó: la suscripción terminó. Puedes renovarla desde la página de compra.".into()
    } else if status == "disabled" || e.contains("disabled") {
        "La licencia está desactivada (reembolso o cancelación).".into()
    } else if e.contains("activation limit") {
        "Esta licencia ya está activada en el máximo de equipos. Desactívala en otro equipo y vuelve a intentarlo.".into()
    } else if e.contains("instance") {
        "Esta activación ya no es válida en este equipo. Vuelve a activar la licencia.".into()
    } else if e.contains("not found") || e.contains("invalid") {
        "Esa clave de licencia no existe. Cópiala tal cual del correo de compra.".into()
    } else if error.is_empty() {
        "La tienda no aceptó la licencia.".into()
    } else {
        format!("La tienda no aceptó la licencia: {error}")
    }
}

/// Interpreta la respuesta de `activate` o `validate`.
pub fn interpret(
    config: &StoreConfig,
    status_code: u16,
    json: &serde_json::Value,
    key: &str,
    known_instance: &str,
    now: u64,
) -> Result<LicenseRecord, LicenseError> {
    let ok = json["activated"].as_bool().or_else(|| json["valid"].as_bool()).unwrap_or(false);
    let lk = &json["license_key"];
    let status = lk["status"].as_str().unwrap_or_default().to_owned();
    if !(200..300).contains(&status_code) || !ok {
        return Err(LicenseError::Rejected(friendly(json["error"].as_str().unwrap_or_default(), &status)));
    }
    let meta = &json["meta"];
    if meta["store_id"].as_u64() != Some(config.store_id) {
        return Err(LicenseError::Rejected("Esa licencia no es de YT Mini Player.".into()));
    }
    let plan = config
        .plan_for(meta["product_id"].as_u64().unwrap_or(0), meta["variant_id"].as_u64().unwrap_or(0))
        .ok_or_else(|| LicenseError::Rejected("Esa licencia es de otro producto de la tienda.".into()))?;
    let record = LicenseRecord {
        key: key.to_owned(),
        instance_id: json["instance"]["id"].as_str().unwrap_or(known_instance).to_owned(),
        plan,
        status: status.clone(),
        product_name: meta["product_name"].as_str().unwrap_or_default().chars().take(80).collect(),
        customer_email: meta["customer_email"].as_str().unwrap_or_default().chars().take(120).collect(),
        expires_at: lk["expires_at"].as_str().map(str::to_owned),
        validated_at: now,
    };
    if record.instance_id.is_empty() {
        return Err(LicenseError::Rejected("La tienda no devolvió la activación.".into()));
    }
    if record.effective_plan(now) == Plan::Free {
        return Err(LicenseError::Rejected(friendly("", &status)));
    }
    Ok(record)
}

fn offline(err: NetError) -> LicenseError {
    LicenseError::Offline(err.to_string())
}

/// Activa la clave en este equipo.
pub fn activate(http: &Http, api: &str, config: &StoreConfig, key: &str, instance_name: &str, now: u64) -> Result<LicenseRecord, LicenseError> {
    let key = key.trim();
    if !plausible_key(key) {
        return Err(LicenseError::Rejected("Esa clave no tiene el formato de una licencia.".into()));
    }
    let (status, json) = http
        .post_form(&format!("{api}/v1/licenses/activate"), &[("license_key", key), ("instance_name", instance_name)])
        .map_err(offline)?;
    match interpret(config, status, &json, key, "", now) {
        Ok(record) => Ok(record),
        Err(err) => {
            // Activada pero no es de esta app: se libera esa activación.
            if let Some(instance) = json["instance"]["id"].as_str().filter(|_| json["activated"] == true) {
                let _ = http.post_form(&format!("{api}/v1/licenses/deactivate"), &[("license_key", key), ("instance_id", instance)]);
            }
            Err(err)
        }
    }
}

/// Valida la activación guardada.
pub fn validate(http: &Http, api: &str, config: &StoreConfig, record: &LicenseRecord, now: u64) -> Result<LicenseRecord, LicenseError> {
    let (status, json) = http
        .post_form(
            &format!("{api}/v1/licenses/validate"),
            &[("license_key", record.key.as_str()), ("instance_id", record.instance_id.as_str())],
        )
        .map_err(offline)?;
    interpret(config, status, &json, &record.key, &record.instance_id, now)
}

/// Libera la activación de este equipo.
pub fn deactivate(http: &Http, api: &str, record: &LicenseRecord) -> Result<(), LicenseError> {
    let (status, json) = http
        .post_form(
            &format!("{api}/v1/licenses/deactivate"),
            &[("license_key", record.key.as_str()), ("instance_id", record.instance_id.as_str())],
        )
        .map_err(offline)?;
    if (200..300).contains(&status) && json["deactivated"].as_bool() == Some(true) {
        Ok(())
    } else {
        Err(LicenseError::Rejected(friendly(json["error"].as_str().unwrap_or_default(), "")))
    }
}

/// Nombre de la activación (lo ve el cliente en su portal).
pub fn instance_name() -> String {
    let host = std::env::var("COMPUTERNAME").unwrap_or_default();
    let host: String = host.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect();
    if host.is_empty() {
        "YT Mini Player".into()
    } else {
        format!("YT Mini Player - {host}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::testserver::{serve, Reply, Seen};
    use std::sync::Arc;

    const NOW: u64 = 1_790_000_000;

    fn config() -> StoreConfig {
        StoreConfig::from_values(Some("1234"), Some("501"), Some("502, 9502"), Some("https://tienda.lemonsqueezy.com/buy/abc"), Some("javascript:alert(1)"), None).unwrap()
    }

    fn response(store: u64, product: u64, status: &str, expires: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "activated": true, "valid": true, "error": null,
            "license_key": { "id": 1, "status": status, "key": "K", "activation_limit": 3, "activation_usage": 1,
                             "created_at": "2026-01-24T14:15:07.000000Z", "expires_at": expires },
            "instance": { "id": "inst-1", "name": "YT Mini Player - PC", "created_at": "2026-01-24T14:15:07.000000Z" },
            "meta": { "store_id": store, "order_id": 2, "order_item_id": 3, "product_id": product, "product_name": "De por vida",
                      "variant_id": 77, "variant_name": "Default", "customer_id": 6, "customer_name": "Ana", "customer_email": "ana@example.com" }
        })
    }

    #[test]
    fn store_config_from_build_values() {
        let c = config();
        assert_eq!(c.store_id, 1234);
        assert_eq!(c.downloads, vec![501]);
        assert_eq!(c.lifetime, vec![502, 9502]);
        assert_eq!(c.checkout_downloads.as_deref(), Some("https://tienda.lemonsqueezy.com/buy/abc"));
        assert_eq!(c.checkout_lifetime, None, "solo https");
        assert_eq!(c.plan_for(501, 0), Some(Plan::Downloads));
        assert_eq!(c.plan_for(0, 9502), Some(Plan::Lifetime));
        assert_eq!(c.plan_for(7, 8), None);
        assert!(StoreConfig::from_values(None, Some("1"), None, None, None, None).is_none());
        assert!(StoreConfig::from_values(Some("1234"), None, Some(""), None, None, None).is_none(), "sin productos no hay tienda");
        assert!(StoreConfig::from_values(Some("0"), Some("1"), None, None, None, None).is_none());
    }

    #[test]
    fn plans_and_entitlements() {
        let free = Plan::Free.entitlements();
        assert!(!free.download);
        assert!(free.can_add_playlist(0) && free.can_add_playlist(1) && !free.can_add_playlist(2), "2 gratis");
        let d = Plan::Downloads.entitlements();
        assert!(d.download && d.can_add_playlist(4) && !d.can_add_playlist(5), "5 con la suscripción");
        let l = Plan::Lifetime.entitlements();
        assert!(l.download && l.can_add_playlist(10_000) && l.max_playlists.is_none(), "ilimitadas de por vida");
    }

    #[test]
    fn interprets_valid_and_rejected_answers() {
        let c = config();
        let ok = interpret(&c, 200, &response(1234, 502, "active", None), "K", "", NOW).unwrap();
        assert_eq!((ok.plan, ok.instance_id.as_str(), ok.validated_at), (Plan::Lifetime, "inst-1", NOW));
        assert_eq!(mask_email(&ok.customer_email), "a***@example.com");

        let other_store = interpret(&c, 200, &response(999, 502, "active", None), "K", "", NOW).unwrap_err();
        assert_eq!(other_store, LicenseError::Rejected("Esa licencia no es de YT Mini Player.".into()));
        let other_product = interpret(&c, 200, &response(1234, 600, "active", None), "K", "", NOW).unwrap_err();
        assert!(matches!(other_product, LicenseError::Rejected(m) if m.contains("otro producto")));

        let mut expired = response(1234, 501, "expired", Some("2026-01-01T00:00:00.000000Z"));
        expired["valid"] = false.into();
        expired["activated"] = serde_json::Value::Null;
        assert!(matches!(interpret(&c, 200, &expired, "K", "", NOW).unwrap_err(), LicenseError::Rejected(m) if m.contains("caducó")));
        // Activa según el estado pero con fecha pasada: caducada igual.
        let past = response(1234, 501, "active", Some("2020-01-01T00:00:00Z"));
        assert!(interpret(&c, 200, &past, "K", "", NOW).is_err());

        let not_found = serde_json::json!({ "activated": false, "error": "license_key not found.", "license_key": null, "meta": null });
        assert!(matches!(interpret(&c, 404, &not_found, "K", "", NOW).unwrap_err(), LicenseError::Rejected(m) if m.contains("no existe")));
        let limit = serde_json::json!({ "activated": false, "error": "This license key has reached the activation limit." });
        assert!(matches!(interpret(&c, 400, &limit, "K", "", NOW).unwrap_err(), LicenseError::Rejected(m) if m.contains("máximo de equipos")));
    }

    #[test]
    fn offline_grace_and_revalidation() {
        let mut r = LicenseRecord { plan: Plan::Downloads, status: "active".into(), validated_at: NOW, ..Default::default() };
        assert_eq!(r.effective_plan(NOW), Plan::Downloads);
        assert!(!r.needs_validation(NOW + 6 * DAY), "la suscripción se comprueba una vez por semana");
        assert!(r.needs_validation(NOW + 7 * DAY));
        assert_eq!(r.effective_plan(NOW + 13 * DAY), Plan::Downloads, "sin conexión casi dos semanas, sigue");
        assert_eq!(r.effective_plan(NOW + 15 * DAY), Plan::Free, "dos semanas sin poder comprobarla: vuelve a Gratis");
        r.plan = Plan::Lifetime;
        assert!(!r.needs_validation(NOW + 29 * DAY), "la de por vida, una vez al mes");
        assert!(r.needs_validation(NOW + 30 * DAY));
        assert_eq!(r.effective_plan(NOW + 400 * DAY), Plan::Lifetime, "la de por vida no caduca sin conexión");
        r.status = "invalid".into();
        assert!(!r.needs_validation(NOW + 400 * DAY), "una clave rechazada no se reintenta sola");
        r.status = "disabled".into();
        assert_eq!(r.effective_plan(NOW), Plan::Free);
        r.status = "active".into();
        r.expires_at = Some("2026-09-21T10:00:00.000000Z".into());
        assert_eq!(r.effective_plan(parse_iso8601("2026-09-21T09:59:59Z").unwrap()), Plan::Lifetime);
        r.validated_at = parse_iso8601("2026-09-21T09:00:00Z").unwrap();
        assert_eq!(r.effective_plan(parse_iso8601("2026-09-21T10:00:01Z").unwrap()), Plan::Free);
    }

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601("2021-01-24T14:15:07.000000Z"), Some(1_611_497_707));
        assert_eq!(parse_iso8601("2000-02-29 12:00:00"), Some(951_825_600));
        for bad in ["", "2021-13-01T00:00:00Z", "ayer", "2021-01-24", "2021-01-24T25:00:00Z"] {
            assert_eq!(parse_iso8601(bad), None, "{bad}");
        }
    }

    #[test]
    fn keys_are_checked_and_masked() {
        assert!(plausible_key("38B1460A-5104-4067-A91D-77B872934D51"));
        for bad in ["", "abc", "38b1460a 5104", "<script>", &"a".repeat(200)] {
            assert!(!plausible_key(bad), "{bad}");
        }
        assert_eq!(mask_key("38B1460A-5104-4067-A91D-77B872934D51"), "****-4D51");
        assert_eq!(mask_email("sin-arroba"), "");
    }

    #[test]
    fn activate_validate_deactivate_against_a_fake_store() {
        let c = config();
        let (base, log) = serve(Arc::new(|s: &Seen| {
            let body = &s.body;
            match s.path.as_str() {
                "/v1/licenses/activate" if body.contains("license_key=GOOD-KEY-0001") => {
                    Reply::json(200, &response(1234, 501, "active", None).to_string())
                }
                "/v1/licenses/activate" if body.contains("license_key=FOREIGN-KEY-01") => {
                    Reply::json(200, &response(4321, 501, "active", None).to_string())
                }
                "/v1/licenses/activate" => Reply::json(404, r#"{"activated":false,"error":"license_key not found."}"#),
                "/v1/licenses/validate" if body.contains("instance_id=inst-1") => {
                    let mut v = response(1234, 501, "active", None);
                    v["instance"] = serde_json::Value::Null;
                    Reply::json(200, &v.to_string())
                }
                "/v1/licenses/validate" => Reply::json(404, r#"{"valid":false,"error":"instance_id not found."}"#),
                "/v1/licenses/deactivate" => Reply::json(200, r#"{"deactivated":true,"error":null}"#),
                _ => Reply::json(500, "{}"),
            }
        }));
        let http = Http::new().unwrap();
        let record = activate(&http, &base, &c, " GOOD-KEY-0001 ", "YT Mini Player - PC", NOW).unwrap();
        assert_eq!((record.plan, record.key.as_str()), (Plan::Downloads, "GOOD-KEY-0001"));

        let again = validate(&http, &base, &c, &record, NOW + DAY).unwrap();
        assert_eq!((again.instance_id.as_str(), again.validated_at), ("inst-1", NOW + DAY), "se conserva la activación");

        let gone = LicenseRecord { instance_id: "otra".into(), ..record.clone() };
        assert!(matches!(validate(&http, &base, &c, &gone, NOW).unwrap_err(), LicenseError::Rejected(m) if m.contains("activación")));

        assert!(activate(&http, &base, &c, "NOPE-NOPE-NOPE", "x", NOW).is_err());
        assert!(matches!(activate(&http, &base, &c, "no válida", "x", NOW).unwrap_err(), LicenseError::Rejected(_)));
        // Clave de otra tienda: se rechaza y se libera la activación.
        assert!(activate(&http, &base, &c, "FOREIGN-KEY-01", "x", NOW).is_err());
        assert_eq!(deactivate(&http, &base, &record), Ok(()));
        let paths: Vec<String> = log.lock().unwrap().iter().map(|s| s.path.clone()).collect();
        assert_eq!(paths.iter().filter(|p| p.ends_with("/deactivate")).count(), 2, "{paths:?}");

        // Sin conexión.
        let err = validate(&http, "http://127.0.0.1:9", &c, &record, NOW).unwrap_err();
        assert!(matches!(err, LicenseError::Offline(_)));
    }
}
