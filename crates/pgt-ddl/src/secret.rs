//! Uygulamada gelen parola: Debug/Serialize asla değeri yazmaz; düşerken bayt sıfırlanır (en iyi çaba).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, PartialEq, Eq, TS)]
#[ts(export, type = "string")]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub fn new(s: &str) -> Self {
        Secret(s.to_string())
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(********)")
    }
}

/// Girdi düz dizgedir (`serde(transparent)` ile aynı; elle yazıldı çünkü ts-rs o özniteliği ayrıştıramıyor).
impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Secret)
    }
}

impl Serialize for Secret {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(crate::plan::REDACTED)
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let mut b = std::mem::take(&mut self.0).into_bytes();
        b.fill(0);
        std::hint::black_box(&b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_and_serialize_hide_value() {
        let s = Secret::new("gizli-değer");
        assert_eq!(s.expose(), "gizli-değer");
        assert_eq!(format!("{s:?}"), "Secret(********)");
        assert_eq!(format!("{:?}", Some(s.clone())), "Some(Secret(********))");
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"********\"");
        // Girdi düz dizgedir (transparent); başka biçim reddedilir.
        let d: Secret = serde_json::from_str("\"x\"").unwrap();
        assert_eq!(d, Secret::new("x"));
        assert!(serde_json::from_str::<Secret>("{\"0\": \"x\"}").is_err());
        assert!(serde_json::from_str::<Secret>("5").is_err());
    }
}
