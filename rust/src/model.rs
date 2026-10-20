use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fmt, sync::Arc};

/// Subscriber resource identifiers are paths, never filesystem paths.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Resource(String);
impl TryFrom<String> for Resource {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if !value.starts_with('/') {
            return Err("resource must start with /");
        }
        Ok(Self(value))
    }
}
impl Resource {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Version(pub String);
impl Version {
    /// Preserve v2's actual comparison, including its shorter-target behavior.
    pub fn is_older_than(&self, target: &Self) -> bool {
        target.0.encode_utf16().count() > self.0.encode_utf16().count()
            || self.0.encode_utf16().cmp(target.0.encode_utf16()).is_lt()
    }
    pub fn from_subscription(value: &Value) -> Option<Self> {
        match value {
            Value::String(s) => Some(Self(s.clone())),
            Value::Number(n) => {
                let number = n.as_f64().filter(|n| n.is_finite())?;
                Some(Self(ryu_js::Buffer::new().format(number).to_owned()))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub version: Version,
    pub data: Value,
}
#[derive(Serialize)]
struct Update<'a> {
    r#type: &'static str,
    resource: &'a Resource,
    version: &'a Version,
    data: &'a Value,
}
impl Record {
    pub fn message(&self, resource: &Resource) -> Arc<str> {
        // All fields are JSON values/strings, so serialization is infallible.
        serde_json::to_string(&Update {
            r#type: "update",
            resource,
            version: &self.version,
            data: &self.data,
        })
        .expect("JSON update serialization")
        .into()
    }
}

pub enum SubscriptionCommand {
    Subscribe(Resource, Version),
    Unsubscribe(Resource),
}
impl SubscriptionCommand {
    pub fn parse(value: Value) -> Option<Self> {
        let resource = Resource::try_from(value.get("resource")?.as_str()?.to_owned()).ok()?;
        match value.get("type")?.as_str()? {
            "subscribe" => Some(Self::Subscribe(
                resource,
                Version::from_subscription(value.get("version")?)?,
            )),
            "unsubscribe" => Some(Self::Unsubscribe(resource)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn comparison_matches_v2_even_when_not_numeric_order() {
        for (current, target, expected) in [
            ("1", "2", true),
            ("51", "502", true),
            ("10", "9", true),
            ("9", "10", true),
            ("2", "2", false),
            ("5", "3", false),
            ("😀", "\u{e000}", true),
            ("", "0", true),
        ] {
            assert_eq!(
                Version(current.into()).is_older_than(&Version(target.into())),
                expected
            );
        }
    }
    #[test]
    fn numeric_subscriptions_use_javascript_number_strings() {
        for (input, expected) in [
            (json!(1.0), "1"),
            (json!(-0.0), "0"),
            (json!(1e21), "1e+21"),
            (json!(9007199254740993u64), "9007199254740992"),
        ] {
            assert_eq!(Version::from_subscription(&input).unwrap().0, expected);
        }
        assert!(Version::from_subscription(&json!(null)).is_none());
    }
}
