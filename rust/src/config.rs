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
    subscriber: Listener,
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
    pub secret: String,
    pub storage: Option<PathBuf>,
    pub cache_items: usize,
    pub limits: Limits,
    pub heartbeat: Duration,
}
impl Config {
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
            subscriber: raw.subscriber,
            secret,
            storage,
            cache_items: raw.persister.memory.max_items,
            limits: raw.limits,
            heartbeat: Duration::from_secs(30),
        })
    }
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
