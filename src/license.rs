//! Paid licenses with Lemon Squeezy (License API).
//!
//! - **Free**: library, player, duplicates and **2 playlists**.
//! - **Downloads** (monthly subscription): download songs and albums, and
//!   **5 playlists**.
//! - **Lifetime** (one-off payment): everything, with **unlimited playlists**.
//!
//! **Non-invasive** charging:
//! - Nothing gets locked all of a sudden and no windows interrupt: the
//!   limits are about quantity and are explained inline, where they apply.
//! - What was created is never taken away: if the subscription ends, the
//!   extra playlists stay (they can be played, edited and deleted); only new
//!   ones above the limit cannot be created.
//! - Hardly any contact with the store: buying happens in the browser; the
//!   app activates the key once and afterwards only checks it every 7 days
//!   (the subscription, which can be cancelled) or every 30 (lifetime, in
//!   case of a refund). Offline, the subscription keeps working 14 days and
//!   lifetime never expires. No telemetry or accounts.
//!
//! The key is checked to belong to *this* store and one of *these*
//! products: the API accepts keys from any store. The key and its state are
//! stored encrypted in the vault.
//!
//! The store data is fixed at build time (environment variables, see
//! README). Without it, the paid features stay locked and the page explains
//! why; with `YTMP_DEV_UNLOCK=1` (development only) everything unlocks and
//! the page says so in big letters.

use serde::{Deserialize, Serialize};

use crate::{
    i18n::Message,
    net::{Http, NetError},
};

pub const API: &str = "https://api.lemonsqueezy.com";
const DAY: u64 = 24 * 60 * 60;
const REVALIDATE_SUBSCRIPTION: u64 = 7 * DAY;
const REVALIDATE_LIFETIME: u64 = 30 * DAY;
const GRACE_SUBSCRIPTION: u64 = 14 * DAY;
/// Playlists per plan (`None` = no limit).
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

/// What each plan allows. This is the only place to change what each one
/// includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlements {
    /// Download songs (single ones or the whole album).
    pub download: bool,
    /// How many playlists one can have (`None` = no limit).
    pub max_playlists: Option<u32>,
}

impl Entitlements {
    /// Can one more playlist be created when there are `existing` already?
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

    /// How long the last validation lasts offline (`None` = forever).
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

/// Store data (at build time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreConfig {
    pub store_id: u64,
    /// Products (or variants) of the Downloads plan.
    pub downloads: Vec<u64>,
    /// Products (or variants) of the Lifetime plan.
    pub lifetime: Vec<u64>,
    pub checkout_downloads: Option<String>,
    pub checkout_lifetime: Option<String>,
    /// Customer portal (manage or cancel the subscription).
    pub portal: Option<String>,
}

fn ids(text: Option<&str>) -> Vec<u64> {
    text.unwrap_or_default().split(',').filter_map(|s| s.trim().parse().ok()).collect()
}

/// https only; anything else is ignored.
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

    /// This build's.
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

    /// The plan a purchase belongs to (by product or variant).
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

/// A development build with everything unlocked?
pub fn dev_unlock() -> bool {
    option_env!("YTMP_DEV_UNLOCK") == Some("1")
}

/// What is stored (encrypted) of the license activated on this PC.
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
    /// Expiry date (ISO 8601) if any.
    pub expires_at: Option<String>,
    /// Last successful validation (Unix seconds).
    pub validated_at: u64,
}

impl LicenseRecord {
    /// The plan in force now: the license's if it is active, has not expired
    /// and the last validation is not too old.
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

    /// Is it time to check it with the store? (A key that stopped being
    /// valid is only checked again if the user asks.)
    pub fn needs_validation(&self, now: u64) -> bool {
        self.status != "invalid" && now.saturating_sub(self.validated_at) >= self.plan.revalidate_after()
    }
}

/// Email with the local part hidden (a***@domain).
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((user, domain)) if !user.is_empty() => {
            let first: String = user.chars().take(1).collect();
            format!("{first}***@{domain}")
        }
        _ => String::new(),
    }
}

/// The key with everything hidden but the end (to show it).
pub fn mask_key(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    if key.chars().count() <= 4 {
        "****".into()
    } else {
        format!("****-{tail}")
    }
}

/// A reasonable key format (Lemon Squeezy's are uppercase UUIDs; other
/// shapes are accepted in case they change, but no odd spaces).
pub fn plausible_key(key: &str) -> bool {
    let len = key.len();
    (8..=128).contains(&len) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// ISO 8601 (`2026-01-24T14:15:07.000000Z`) to Unix seconds (UTC).
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
    // Days since 1970-01-01 (Howard Hinnant's algorithm).
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
    /// No connection: what there was is kept (within the grace period). The
    /// detail is for logs.
    Offline(String),
    /// The store says no (or the key is not for this app).
    Rejected(Message),
}

impl LicenseError {
    /// For the user, in the language the page shows.
    pub fn message(&self) -> Message {
        match self {
            LicenseError::Offline(_) => msg!("music.license.offline"),
            LicenseError::Rejected(message) => message.clone(),
        }
    }
}

/// The API's known error messages, explained (the API answers in English).
fn friendly(error: &str, status: &str) -> Message {
    let e = error.to_ascii_lowercase();
    if status == "expired" || e.contains("expired") {
        msg!("music.license.expired")
    } else if status == "disabled" || e.contains("disabled") {
        msg!("music.license.disabled")
    } else if e.contains("activation limit") {
        msg!("music.license.limit")
    } else if e.contains("instance") {
        msg!("music.license.instance")
    } else if e.contains("not found") || e.contains("invalid") {
        msg!("music.license.not_found")
    } else if error.is_empty() {
        msg!("music.license.rejected")
    } else {
        msg!("music.license.rejected_detail", detail = error.chars().take(200).collect::<String>())
    }
}

/// Interprets the answer of `activate` or `validate`.
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
        return Err(LicenseError::Rejected(msg!("music.license.other_store")));
    }
    let plan = config
        .plan_for(meta["product_id"].as_u64().unwrap_or(0), meta["variant_id"].as_u64().unwrap_or(0))
        .ok_or_else(|| LicenseError::Rejected(msg!("music.license.other_product")))?;
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
        return Err(LicenseError::Rejected(msg!("music.license.no_instance")));
    }
    if record.effective_plan(now) == Plan::Free {
        return Err(LicenseError::Rejected(friendly("", &status)));
    }
    Ok(record)
}

fn offline(err: NetError) -> LicenseError {
    LicenseError::Offline(err.to_string())
}

/// Activates the key on this PC.
pub fn activate(http: &Http, api: &str, config: &StoreConfig, key: &str, instance_name: &str, now: u64) -> Result<LicenseRecord, LicenseError> {
    let key = key.trim();
    if !plausible_key(key) {
        return Err(LicenseError::Rejected(msg!("music.license.bad_format")));
    }
    let (status, json) = http
        .post_form(&format!("{api}/v1/licenses/activate"), &[("license_key", key), ("instance_name", instance_name)])
        .map_err(offline)?;
    match interpret(config, status, &json, key, "", now) {
        Ok(record) => Ok(record),
        Err(err) => {
            // Activated but not for this app: that activation is released.
            if let Some(instance) = json["instance"]["id"].as_str().filter(|_| json["activated"] == true) {
                let _ = http.post_form(&format!("{api}/v1/licenses/deactivate"), &[("license_key", key), ("instance_id", instance)]);
            }
            Err(err)
        }
    }
}

/// Validates the saved activation.
pub fn validate(http: &Http, api: &str, config: &StoreConfig, record: &LicenseRecord, now: u64) -> Result<LicenseRecord, LicenseError> {
    let (status, json) = http
        .post_form(
            &format!("{api}/v1/licenses/validate"),
            &[("license_key", record.key.as_str()), ("instance_id", record.instance_id.as_str())],
        )
        .map_err(offline)?;
    interpret(config, status, &json, &record.key, &record.instance_id, now)
}

/// Releases this PC's activation.
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

/// Name of the activation (the customer sees it in their portal).
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
        StoreConfig::from_values(Some("1234"), Some("501"), Some("502, 9502"), Some("https://shop.lemonsqueezy.com/buy/abc"), Some("javascript:alert(1)"), None).unwrap()
    }

    fn response(store: u64, product: u64, status: &str, expires: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "activated": true, "valid": true, "error": null,
            "license_key": { "id": 1, "status": status, "key": "K", "activation_limit": 3, "activation_usage": 1,
                             "created_at": "2026-01-24T14:15:07.000000Z", "expires_at": expires },
            "instance": { "id": "inst-1", "name": "YT Mini Player - PC", "created_at": "2026-01-24T14:15:07.000000Z" },
            "meta": { "store_id": store, "order_id": 2, "order_item_id": 3, "product_id": product, "product_name": "Lifetime",
                      "variant_id": 77, "variant_name": "Default", "customer_id": 6, "customer_name": "Ana", "customer_email": "ana@example.com" }
        })
    }

    #[test]
    fn store_config_from_build_values() {
        let c = config();
        assert_eq!(c.store_id, 1234);
        assert_eq!(c.downloads, vec![501]);
        assert_eq!(c.lifetime, vec![502, 9502]);
        assert_eq!(c.checkout_downloads.as_deref(), Some("https://shop.lemonsqueezy.com/buy/abc"));
        assert_eq!(c.checkout_lifetime, None, "https only");
        assert_eq!(c.plan_for(501, 0), Some(Plan::Downloads));
        assert_eq!(c.plan_for(0, 9502), Some(Plan::Lifetime));
        assert_eq!(c.plan_for(7, 8), None);
        assert!(StoreConfig::from_values(None, Some("1"), None, None, None, None).is_none());
        assert!(StoreConfig::from_values(Some("1234"), None, Some(""), None, None, None).is_none(), "no products, no store");
        assert!(StoreConfig::from_values(Some("0"), Some("1"), None, None, None, None).is_none());
    }

    #[test]
    fn plans_and_entitlements() {
        let free = Plan::Free.entitlements();
        assert!(!free.download);
        assert!(free.can_add_playlist(0) && free.can_add_playlist(1) && !free.can_add_playlist(2), "2 for free");
        let d = Plan::Downloads.entitlements();
        assert!(d.download && d.can_add_playlist(4) && !d.can_add_playlist(5), "5 with the subscription");
        let l = Plan::Lifetime.entitlements();
        assert!(l.download && l.can_add_playlist(10_000) && l.max_playlists.is_none(), "unlimited for lifetime");
    }

    #[test]
    fn interprets_valid_and_rejected_answers() {
        let c = config();
        let ok = interpret(&c, 200, &response(1234, 502, "active", None), "K", "", NOW).unwrap();
        assert_eq!((ok.plan, ok.instance_id.as_str(), ok.validated_at), (Plan::Lifetime, "inst-1", NOW));
        assert_eq!(mask_email(&ok.customer_email), "a***@example.com");

        let other_store = interpret(&c, 200, &response(999, 502, "active", None), "K", "", NOW).unwrap_err();
        assert_eq!(other_store, LicenseError::Rejected(msg!("music.license.other_store")));
        let other_product = interpret(&c, 200, &response(1234, 600, "active", None), "K", "", NOW).unwrap_err();
        assert_eq!(other_product.message(), msg!("music.license.other_product"));

        let mut expired = response(1234, 501, "expired", Some("2026-01-01T00:00:00.000000Z"));
        expired["valid"] = false.into();
        expired["activated"] = serde_json::Value::Null;
        assert_eq!(interpret(&c, 200, &expired, "K", "", NOW).unwrap_err().message(), msg!("music.license.expired"));
        // Active by its status but with a past date: expired all the same.
        let past = response(1234, 501, "active", Some("2020-01-01T00:00:00Z"));
        assert!(interpret(&c, 200, &past, "K", "", NOW).is_err());

        let not_found = serde_json::json!({ "activated": false, "error": "license_key not found.", "license_key": null, "meta": null });
        assert_eq!(interpret(&c, 404, &not_found, "K", "", NOW).unwrap_err().message(), msg!("music.license.not_found"));
        let limit = serde_json::json!({ "activated": false, "error": "This license key has reached the activation limit." });
        assert_eq!(interpret(&c, 400, &limit, "K", "", NOW).unwrap_err().message(), msg!("music.license.limit"));
        let odd = serde_json::json!({ "activated": false, "error": "Something new." });
        assert_eq!(interpret(&c, 400, &odd, "K", "", NOW).unwrap_err().message(), msg!("music.license.rejected_detail", detail = "Something new."));
    }

    #[test]
    fn offline_grace_and_revalidation() {
        let mut r = LicenseRecord { plan: Plan::Downloads, status: "active".into(), validated_at: NOW, ..Default::default() };
        assert_eq!(r.effective_plan(NOW), Plan::Downloads);
        assert!(!r.needs_validation(NOW + 6 * DAY), "the subscription is checked once a week");
        assert!(r.needs_validation(NOW + 7 * DAY));
        assert_eq!(r.effective_plan(NOW + 13 * DAY), Plan::Downloads, "almost two weeks offline, still fine");
        assert_eq!(r.effective_plan(NOW + 15 * DAY), Plan::Free, "two weeks without checking: back to Free");
        r.plan = Plan::Lifetime;
        assert!(!r.needs_validation(NOW + 29 * DAY), "lifetime, once a month");
        assert!(r.needs_validation(NOW + 30 * DAY));
        assert_eq!(r.effective_plan(NOW + 400 * DAY), Plan::Lifetime, "lifetime never expires offline");
        r.status = "invalid".into();
        assert!(!r.needs_validation(NOW + 400 * DAY), "a rejected key is not retried by itself");
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
        for bad in ["", "2021-13-01T00:00:00Z", "yesterday", "2021-01-24", "2021-01-24T25:00:00Z"] {
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
        assert_eq!(mask_email("no-at-sign"), "");
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
        assert_eq!((again.instance_id.as_str(), again.validated_at), ("inst-1", NOW + DAY), "the activation is kept");

        let gone = LicenseRecord { instance_id: "other".into(), ..record.clone() };
        assert_eq!(validate(&http, &base, &c, &gone, NOW).unwrap_err().message(), msg!("music.license.instance"));

        assert!(activate(&http, &base, &c, "NOPE-NOPE-NOPE", "x", NOW).is_err());
        assert!(matches!(activate(&http, &base, &c, "not valid", "x", NOW).unwrap_err(), LicenseError::Rejected(_)));
        // A key from another store: rejected, and its activation released.
        assert!(activate(&http, &base, &c, "FOREIGN-KEY-01", "x", NOW).is_err());
        assert_eq!(deactivate(&http, &base, &record), Ok(()));
        let paths: Vec<String> = log.lock().unwrap().iter().map(|s| s.path.clone()).collect();
        assert_eq!(paths.iter().filter(|p| p.ends_with("/deactivate")).count(), 2, "{paths:?}");

        // Offline.
        let err = validate(&http, "http://127.0.0.1:9", &c, &record, NOW).unwrap_err();
        assert!(matches!(err, LicenseError::Offline(_)));
    }
}
