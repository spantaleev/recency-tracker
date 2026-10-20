use serde::Deserialize;
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Deserialize)]
pub struct Listener {
    pub ip: IpAddr,
    pub port: u16,
}
#[derive(Deserialize)]
struct Publisher {
    ip: IpAddr,
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
}
