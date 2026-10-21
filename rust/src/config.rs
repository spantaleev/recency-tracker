use serde::Deserialize;
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    time::Duration,
};

/// A listener address accepted by the old Node server: a literal IP or hostname.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(try_from = "String")]
pub struct Host(String);
impl TryFrom<String> for Host {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.parse::<IpAddr>().is_ok() {
            return Ok(Self(value));
        }
        if value.is_empty()
            || value.chars().any(char::is_whitespace)
            || value
                .bytes()
                .any(|byte| matches!(byte, b'/' | b'\\' | b'\0'))
        {
            return Err("listener host must be a hostname or IP address");
        }
        Ok(Self(value))
    }
}
impl Host {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Exact serialized browser origin; matching never uses wildcards or substrings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(try_from = "String")]
pub struct Origin(String);
impl TryFrom<String> for Origin {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let invalid =
            "origin must be an exact HTTP(S) origin without path, credentials or wildcard";
        let uri: axum::http::Uri = value.parse().map_err(|_| invalid)?;
        let scheme = uri
            .scheme_str()
            .filter(|s| matches!(*s, "http" | "https"))
            .ok_or(invalid)?;
        let authority = uri.authority().ok_or(invalid)?;
        let host = authority.host();
        if host.is_empty()
            || authority.as_str().contains(['@', '*'])
            || value != format!("{scheme}://{authority}")
            || (authority.as_str() != host && authority.port_u16().is_none())
        {
            return Err(invalid);
        }
        Ok(Self(value))
    }
}
impl Origin {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
fn deserialize_origins<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<Origin>>, D::Error> {
    // Absent means legacy unrestricted behavior; explicit null must not disable a policy.
    Vec::<Origin>::deserialize(deserializer).map(Some)
}
#[derive(Deserialize)]
struct Subscriber {
    #[serde(flatten)]
    listener: Listener,
    #[serde(
        default,
        rename = "allowedOrigins",
        deserialize_with = "deserialize_origins"
    )]
    allowed_origins: Option<Vec<Origin>>,
}

#[derive(Clone, Deserialize)]
pub struct Listener {
    pub ip: Host,
    pub port: u16,
}
#[derive(Deserialize)]
struct Publisher {
    ip: Host,
    port: u16,
    secret: Option<String>,
}
#[derive(Clone, Deserialize)]
pub struct Memory {
    #[serde(rename = "maxItems")]
    pub max_items: usize,
}
#[derive(Clone, Deserialize)]
pub struct Filesystem {
    #[serde(rename = "storagePath")]
    pub storage_path: PathBuf,
}
#[derive(Deserialize)]
struct Persistence {
    r#type: Option<String>,
    memory: Memory,
    filesystem: Option<Filesystem>,
}
#[derive(Deserialize)]
struct RawConfig {
    publisher: Publisher,
    subscriber: Subscriber,
    persister: Persistence,
    #[serde(default)]
    limits: Limits,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub connections: usize,
    pub subscriptions_per_connection: usize,
    pub queued_updates: usize,
    pub queued_bytes_per_connection: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            connections: 10000,
            subscriptions_per_connection: 1024,
            queued_updates: 256,
            queued_bytes_per_connection: 1024 * 1024,
        }
    }
}
#[derive(Clone)]
pub struct Config {
    pub publisher: Listener,
    pub subscriber: Listener,
    pub allowed_origins: Option<Vec<Origin>>,
    pub secret: String,
    pub storage: Option<PathBuf>,
    pub cache_items: usize,
    pub limits: Limits,
    pub heartbeat: Duration,
}
impl Config {
    pub fn override_allowed_origins(&mut self, json: &str) -> Result<(), String> {
        let origins = Some(serde_json::from_str::<Vec<Origin>>(json).map_err(
            |_| "allowed origins override must be a JSON array of exact HTTP(S) origins",
        )?);
        validate_origins(&origins)?;
        self.allowed_origins = origins;
        Ok(())
    }
    pub fn load(path: &Path, secret_override: Option<String>) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("Cannot read configuration ({}): {e}", path.display()))?;
        Self::parse(&bytes, secret_override)
    }
    pub fn parse(bytes: &[u8], secret_override: Option<String>) -> Result<Self, String> {
        let raw: RawConfig = serde_json::from_slice(bytes)
            .map_err(|e| format!("Cannot parse configuration: {e}"))?;
        let secret = secret_override
            .or(raw.publisher.secret)
            .filter(|s| !s.is_empty())
            .ok_or("publisher secret must not be empty")?;
        if secret == "YOUR_PUBLISHING_SECRET_HERE" {
            return Err("publisher secret must replace the example placeholder".into());
        }
        let storage = match raw.persister.r#type.as_deref() {
            None | Some("none") => None,
            Some("filesystem") => Some(
                raw.persister
                    .filesystem
                    .ok_or("missing filesystem configuration")?
                    .storage_path,
            ),
            Some(_) => return Err("unsupported persister type; expected filesystem or none".into()),
        };
        validate_origins(&raw.subscriber.allowed_origins)?;
        let l = &raw.limits;
        if l.connections == 0
            || l.connections > 1_000_000
            || l.subscriptions_per_connection == 0
            || l.subscriptions_per_connection > 65536
            || l.queued_updates == 0
            || l.queued_updates > 65536
            || l.queued_bytes_per_connection == 0
            || l.queued_bytes_per_connection > 64 * 1024 * 1024
        {
            return Err("limits must be positive and within documented bounds".into());
        }
        Ok(Self {
            publisher: Listener {
                ip: raw.publisher.ip,
                port: raw.publisher.port,
            },
            subscriber: raw.subscriber.listener,
            allowed_origins: raw.subscriber.allowed_origins,
            secret,
            storage,
            cache_items: raw.persister.memory.max_items,
            limits: raw.limits,
            heartbeat: Duration::from_secs(30),
        })
    }
}
fn validate_origins(origins: &Option<Vec<Origin>>) -> Result<(), String> {
    if origins.as_ref().is_some_and(|values| values.len() > 256) {
        return Err("at most 256 allowed subscriber origins are supported".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_configuration_and_secret_override_remain_valid() {
        let bytes = include_bytes!("../../config/container.json.dist");
        let c = Config::parse(bytes, Some("override".into())).unwrap();
        assert_eq!(c.secret, "override");
        assert_eq!(c.storage, Some(PathBuf::from("/data")));
        assert!(Config::parse(bytes, Some(String::new())).is_err());
        assert!(Config::parse(b"{", None).is_err());
    }
    #[test]
    fn example_secret_is_rejected_in_file_and_override() {
        let bytes = include_bytes!("../../config/container.json.dist");
        assert!(Config::parse(bytes, None).is_err());
        assert!(Config::parse(bytes, Some("YOUR_PUBLISHING_SECRET_HERE".into())).is_err());
        assert!(Config::parse(bytes, Some("synthetic-real-secret".into())).is_ok());
    }
    #[test]
    fn origins_are_exact_and_explicit_null_is_rejected() {
        for value in [
            "https://app.example.test",
            "http://localhost:20080",
            "http://[::1]:20080",
        ] {
            assert!(Origin::try_from(value.to_owned()).is_ok(), "{value}");
        }
        for value in [
            "*",
            "null",
            "https://*.example.test",
            "https://user@app.example.test",
            "https://app.example.test/path",
            "https://app.example.test/",
            "https://app.example.test?query=1",
            "https://app.example.test#fragment",
            "https://app.example.test:bad",
            "https://app.example.test:65536",
        ] {
            assert!(Origin::try_from(value.to_owned()).is_err(), "{value}");
        }
        let mut json: serde_json::Value =
            serde_json::from_slice(include_bytes!("../../config/container.json.dist")).unwrap();
        json["subscriber"]["allowedOrigins"] = serde_json::Value::Null;
        assert!(
            Config::parse(&serde_json::to_vec(&json).unwrap(), Some("override".into())).is_err()
        );
    }
    #[test]
    fn origin_override_is_validated_and_empty_array_denies_all() {
        let mut config = Config::parse(
            include_bytes!("../../config/container.json.dist"),
            Some("override".into()),
        )
        .unwrap();
        assert!(config.allowed_origins.is_none());
        config
            .override_allowed_origins(r#"["https://app.example.test"]"#)
            .unwrap();
        assert_eq!(
            config.allowed_origins.as_ref().unwrap()[0].as_str(),
            "https://app.example.test"
        );
        for invalid in ["", "null", r#"["*"]"#] {
            assert!(config.override_allowed_origins(invalid).is_err());
        }
        assert_eq!(config.allowed_origins.as_ref().unwrap().len(), 1);
        config.override_allowed_origins("[]").unwrap();
        assert_eq!(config.allowed_origins, Some(Vec::new()));
    }
    #[test]
    fn listener_hosts_accept_existing_ip_and_dns_configurations() {
        for host in ["127.0.0.1", "::1", "localhost", "tracker.example.test"] {
            let mut json: serde_json::Value =
                serde_json::from_slice(include_bytes!("../../config/container.json.dist")).unwrap();
            json["publisher"]["ip"] = host.into();
            json["subscriber"]["ip"] = host.into();
            let c = Config::parse(&serde_json::to_vec(&json).unwrap(), Some("override".into()))
                .unwrap();
            assert_eq!(c.publisher.ip.as_str(), host);
            assert_eq!(c.subscriber.ip.as_str(), host);
        }
    }
    #[test]
    fn listener_hosts_reject_obviously_invalid_values() {
        for host in [
            "",
            " localhost",
            "localhost ",
            "host/path",
            "host\\name",
            "host\0name",
        ] {
            assert!(
                Host::try_from(host.to_owned()).is_err(),
                "accepted {host:?}"
            );
        }
    }
}
