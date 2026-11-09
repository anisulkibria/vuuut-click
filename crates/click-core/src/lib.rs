//! Shared contracts. The collection token is public; it never grants query access.
pub mod totp;
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{net::IpAddr, path::Path};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserEvent {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub visitor_id: Option<Uuid>,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub referrer: String,
    #[serde(default)]
    pub utm_source: String,
    #[serde(default)]
    pub utm_medium: String,
    #[serde(default)]
    pub utm_campaign: String,
    #[serde(default)]
    pub occurred_at_ms: Option<i64>,
    #[serde(default)]
    pub vital: Option<Vital>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engagement: Option<Engagement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Engagement {
    pub duration_ms: u32,
    pub scroll_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanEvent {
    pub event_id: Uuid,
    pub session_id: String,
    pub visitor_id: String,
    pub name: String,
    pub path: String,
    pub referrer: String,
    pub utm_source: String,
    pub utm_medium: String,
    pub utm_campaign: String,
    #[serde(default)]
    pub occurred_at_ms: Option<i64>,
    #[serde(default)]
    pub vital: Option<Vital>,
    #[serde(default)]
    pub amount_minor: i64,
    #[serde(default)]
    pub currency: String,
    #[serde(default)]
    pub visitor_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engagement: Option<Engagement>,
    #[serde(default)]
    pub newsletter_id: String,
    #[serde(default)]
    pub recipient_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vital {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub value: f64,
}
impl Vital {
    pub fn valid(&self) -> bool {
        self.id.len() <= 100
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            && self.value.is_finite()
            && self.value >= 0.0
            && match self.name.as_str() {
                "CLS" => self.value <= 100.0,
                "LCP" | "INP" | "FCP" | "TTFB" => self.value <= 120000.0,
                _ => false,
            }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ValidationError(pub &'static str);

impl BrowserEvent {
    pub fn clean(
        self,
        website_id: Uuid,
        expected_origin: &str,
    ) -> Result<CleanEvent, ValidationError> {
        if self.name.starts_with("server.") {
            return Err(ValidationError("Reserved backend event name"));
        }
        if self.occurred_at_ms.is_some_and(|time| {
            let now = chrono::Utc::now().timestamp_millis();
            time < now - 7 * 86400000 || time > now + 60000
        }) {
            return Err(ValidationError("Event time must be within the last 7 days"));
        }
        if self.vital.as_ref().is_some_and(|v| !v.valid())
            || (self.name == "web_vital") != self.vital.is_some()
        {
            return Err(ValidationError("Invalid Web Vital event"));
        }
        if (self.name == "web_engagement") != self.engagement.is_some()
            || self
                .engagement
                .as_ref()
                .is_some_and(|e| e.duration_ms > 30000 || e.scroll_percent > 100)
        {
            return Err(ValidationError("Invalid engagement measurement"));
        }
        if self.event_id.is_nil()
            || self.session_id.is_nil()
            || self.visitor_id.is_some_and(|v| v.is_nil())
        {
            return Err(ValidationError("Identifiers must be nonzero UUIDs"));
        }
        if self.name.is_empty()
            || self.name.len() > 64
            || !self
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
        {
            return Err(ValidationError(
                "Event name must be 1–64 ASCII identifier characters",
            ));
        }
        if self.url.len() > 2048 {
            return Err(ValidationError("URL too long"));
        }
        let url = Url::parse(&self.url).map_err(|_| ValidationError("Invalid URL"))?;
        if !matches!(url.scheme(), "https" | "http")
            || url.origin().ascii_serialization() != expected_origin
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(ValidationError(
                "Event URL must match verified website origin",
            ));
        }
        for v in [&self.utm_source, &self.utm_medium, &self.utm_campaign] {
            if v.len() > 120 || v.chars().any(char::is_control) {
                return Err(ValidationError("Invalid campaign label"));
            }
        }
        let referrer = Url::parse(&self.referrer)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https"))
            .and_then(|u| u.host_str().map(str::to_owned))
            .filter(|h| Some(h.as_str()) != url.host_str())
            .unwrap_or_default();
        Ok(CleanEvent {
            event_id: self.event_id,
            session_id: digest(&format!("{website_id}:{}", self.session_id)),
            visitor_id: digest(&format!(
                "{website_id}:{}",
                self.visitor_id.unwrap_or(self.session_id)
            )),
            name: self.name,
            path: url.path().to_owned(),
            referrer,
            utm_source: self.utm_source,
            utm_medium: self.utm_medium,
            utm_campaign: self.utm_campaign,
            occurred_at_ms: self.occurred_at_ms,
            vital: self.vital,
            amount_minor: 0,
            currency: String::new(),
            visitor_mode: if self.visitor_id.is_some() {
                "provided"
            } else {
                "session"
            }
            .into(),
            engagement: self.engagement,
            newsletter_id: String::new(),
            recipient_id: String::new(),
        })
    }
}

/// Compare two strings without letting the time taken reveal where they
/// first differ.
///
/// Used for one-time codes and tokens, where an early-exit comparison leaks
/// how much of a guess was right.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Admin,
    Analyst,
    Viewer,
}
impl Role {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "analyst" => Some(Self::Analyst),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }
    pub fn can_manage(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
    pub fn can_edit_goals(self) -> bool {
        self != Self::Viewer
    }
}

/// Headers are honored only when the actual socket peer belongs to the configured proxy networks.
pub fn client_ip(peer: IpAddr, forwarded: Option<&str>, trusted: &[IpNet]) -> IpAddr {
    let is_trusted = |ip: IpAddr| trusted.iter().any(|n| n.contains(&ip));
    if !is_trusted(peer) {
        return peer;
    }
    let Some(chain) = forwarded else {
        return peer;
    };
    let mut current = peer;
    for part in chain.rsplit(',').take(32) {
        if !is_trusted(current) {
            break;
        }
        match part.trim().parse() {
            Ok(ip) => current = ip,
            Err(_) => return peer,
        }
    }
    current
}

pub struct GeoIp(Option<maxminddb::Reader<Vec<u8>>>);
impl GeoIp {
    pub fn open(path: Option<&Path>) -> Result<Self, maxminddb::MaxMindDbError> {
        Ok(Self(
            path.map(maxminddb::Reader::open_readfile).transpose()?,
        ))
    }
    pub fn country(&self, ip: IpAddr) -> String {
        self.0
            .as_ref()
            .and_then(|r| r.lookup(ip).ok())
            .and_then(|r| {
                r.decode_path::<String>(&maxminddb::path!["country", "iso_code"])
                    .ok()
                    .flatten()
            })
            .unwrap_or_else(|| "ZZ".to_owned())
    }
    pub fn database_type(&self) -> Option<&str> {
        self.0.as_ref().map(|r| r.metadata().database_type.as_str())
    }
    pub fn available(&self) -> bool {
        self.0.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event() -> BrowserEvent {
        BrowserEvent {
            event_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            visitor_id: None,
            name: "pageview".into(),
            url: "https://example.com/post?token=secret#private".into(),
            referrer: "https://search.example/find?q=secret".into(),
            utm_source: String::new(),
            utm_medium: String::new(),
            utm_campaign: String::new(),
            occurred_at_ms: None,
            vital: None,
            engagement: None,
        }
    }
    #[test]
    fn strips_secrets_and_scopes_identity() {
        let e = event();
        let a = e
            .clone()
            .clean(Uuid::new_v4(), "https://example.com")
            .unwrap();
        let b = e.clean(Uuid::new_v4(), "https://example.com").unwrap();
        assert_eq!(a.path, "/post");
        assert_eq!(a.referrer, "search.example");
        assert_ne!(a.visitor_id, b.visitor_id);
        assert!(!serde_json::to_string(&a).unwrap().contains("secret"));
    }
    #[test]
    fn rejects_foreign_origin_and_properties() {
        assert!(event()
            .clean(Uuid::new_v4(), "https://other.example")
            .is_err());
        let mut e = event();
        e.name = "<script>".into();
        assert!(e.clean(Uuid::new_v4(), "https://example.com").is_err());
        assert!(serde_json::from_str::<BrowserEvent>("{\"password\":\"secret\"}").is_err());
    }
    #[test]
    fn forwarding_cannot_be_spoofed() {
        let trusted = ["10.0.0.0/8".parse().unwrap()];
        assert_eq!(
            client_ip("8.8.8.8".parse().unwrap(), Some("1.1.1.1"), &trusted).to_string(),
            "8.8.8.8"
        );
        assert_eq!(
            client_ip(
                "10.0.0.1".parse().unwrap(),
                Some("1.1.1.1, 8.8.8.8, 10.0.0.2"),
                &trusted
            )
            .to_string(),
            "8.8.8.8"
        );
        assert_eq!(
            client_ip("10.0.0.1".parse().unwrap(), Some("bad"), &trusted).to_string(),
            "10.0.0.1"
        );
    }
    #[test]
    fn ipv6_and_unknown_country() {
        assert_eq!(
            GeoIp::open(None).unwrap().country("::1".parse().unwrap()),
            "ZZ"
        );
        let trusted = ["fd00::/8".parse().unwrap()];
        assert_eq!(
            client_ip(
                "fd00::1".parse().unwrap(),
                Some("2001:4860:4860::8888"),
                &trusted
            )
            .to_string(),
            "2001:4860:4860::8888"
        );
    }
    #[test]
    fn roles_fail_closed() {
        assert!(Role::parse("superuser").is_none());
        assert!(!Role::Viewer.can_edit_goals());
        assert!(!Role::Analyst.can_manage());
        assert!(Role::Owner.can_manage());
    }
    #[test]
    fn engagement_is_bounded_and_identity_coverage_is_explicit() {
        let site = Uuid::new_v4();
        let mut e = event();
        assert_eq!(
            e.clone()
                .clean(site, "https://example.com")
                .unwrap()
                .visitor_mode,
            "session"
        );
        e.visitor_id = Some(Uuid::new_v4());
        assert_eq!(
            e.clone()
                .clean(site, "https://example.com")
                .unwrap()
                .visitor_mode,
            "provided"
        );
        e.name = "web_engagement".into();
        e.engagement = Some(Engagement {
            duration_ms: 30000,
            scroll_percent: 100,
        });
        assert!(e.clone().clean(site, "https://example.com").is_ok());
        e.engagement.as_mut().unwrap().duration_ms = 30001;
        assert!(e.clone().clean(site, "https://example.com").is_err());
        e.engagement = Some(Engagement {
            duration_ms: 1,
            scroll_percent: 101,
        });
        assert!(e.clone().clean(site, "https://example.com").is_err());
        e.engagement = None;
        assert!(e.clean(site, "https://example.com").is_err());
    }
}
