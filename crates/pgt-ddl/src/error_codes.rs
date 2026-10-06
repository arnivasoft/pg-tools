//! DDL kapı/durum reddi hatalarının arayüze giden ayrı kodları (`pgt-query` kalıbı). Arayüz `ErrorPayload.code`
//! üzerinden eşleşir, metin üzerinden asla.

use pgt_core::{Error, ErrorPayload};
use serde::Serialize;
use ts_rs::TS;

/// Sunucu salt okunur kilitli; DDL uygulaması reddedildi.
pub const LOCKED_MESSAGE: &str = "Sunucu salt okunur kilitli; nesne değişikliği uygulanamaz.";
/// Yıkıcı plan onaysız uygulanmak istendi.
pub const CONFIRMATION_REQUIRED: &str = "Bu değişiklik yıkıcı ifadeler içeriyor ve onay gerektiriyor.";
/// Veri kaybı olan plan üretim sunucusunda ek onaysız uygulanmak istendi.
pub const PRODUCTION_CONFIRMATION_REQUIRED: &str = "Üretim sunucusunda veri kaybı olan değişiklik ek onay gerektiriyor.";
/// Önizlenen plan ile yeniden üretilen plan farklı (tanım ya da sunucu durumu değişti).
pub const PLAN_CHANGED: &str = "Plan önizlemeden sonra değişti; lütfen yeniden önizleyin.";
/// Tanım doğrulamadan geçmedi (alan hataları önizlemede ayrıca döner).
pub const INVALID_DEF: &str = "Nesne tanımı geçersiz.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DdlErrorCode {
    Locked,
    ConfirmationRequired,
    ProductionConfirmationRequired,
    PlanChanged,
    InvalidDef,
}

/// Arayüzün eşleştirebileceği kodların tam listesi (`DdlErrorCode`'un serde adları).
pub const DDL_ERROR_CODES: [&str; 5] =
    ["locked", "confirmation_required", "production_confirmation_required", "plan_changed", "invalid_def"];

impl DdlErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            DdlErrorCode::Locked => DDL_ERROR_CODES[0],
            DdlErrorCode::ConfirmationRequired => DDL_ERROR_CODES[1],
            DdlErrorCode::ProductionConfirmationRequired => DDL_ERROR_CODES[2],
            DdlErrorCode::PlanChanged => DDL_ERROR_CODES[3],
            DdlErrorCode::InvalidDef => DDL_ERROR_CODES[4],
        }
    }

    /// İşaret metnini koda ve ileti metnine çevirir; bilinmeyen metin `None`.
    pub fn from_marker(marker: &str) -> Option<(Self, &'static str)> {
        Some(match marker {
            m if m == LOCKED_MESSAGE => (Self::Locked, LOCKED_MESSAGE),
            m if m == CONFIRMATION_REQUIRED => (Self::ConfirmationRequired, CONFIRMATION_REQUIRED),
            m if m == PRODUCTION_CONFIRMATION_REQUIRED => (Self::ProductionConfirmationRequired, PRODUCTION_CONFIRMATION_REQUIRED),
            m if m == PLAN_CHANGED => (Self::PlanChanged, PLAN_CHANGED),
            m if m == INVALID_DEF => (Self::InvalidDef, INVALID_DEF),
            _ => return None,
        })
    }
}

/// Komut katmanı hata eşlemesi: işaretli `Invalid` hataları ayrı koda, gerisi `ErrorPayload::from`.
pub fn ddl_payload(e: &Error) -> ErrorPayload {
    if let Error::Invalid(m) = e
        && let Some((code, message)) = DdlErrorCode::from_marker(m)
    {
        return ErrorPayload { code: code.as_str().to_string(), message: message.to_string(), detail: None, hint: None };
    }
    ErrorPayload::from(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_marker_maps_to_its_code_without_prefix() {
        let cases = [
            (LOCKED_MESSAGE, "locked"),
            (CONFIRMATION_REQUIRED, "confirmation_required"),
            (PRODUCTION_CONFIRMATION_REQUIRED, "production_confirmation_required"),
            (PLAN_CHANGED, "plan_changed"),
            (INVALID_DEF, "invalid_def"),
        ];
        for (marker, code) in cases {
            let p = ddl_payload(&Error::Invalid(marker.into()));
            assert_eq!(p.code, code, "{marker}");
            assert!(!p.message.contains("Geçersiz ayar"), "{}", p.message);
            assert_eq!(p.message, marker);
        }
    }

    #[test]
    fn unknown_invalid_and_other_errors_keep_their_generic_codes() {
        let p = ddl_payload(&Error::Invalid("başka bir şey".into()));
        assert_eq!(p.code, "invalid");
        assert_eq!(p.message, "Geçersiz ayar: başka bir şey");
        assert_eq!(ddl_payload(&Error::NotConnected).code, "not_connected");
        assert_eq!(ddl_payload(&Error::NotFound("sunucu".into())).code, "not_found");
    }

    #[test]
    fn code_list_matches_serde_names() {
        let all = [
            DdlErrorCode::Locked,
            DdlErrorCode::ConfirmationRequired,
            DdlErrorCode::ProductionConfirmationRequired,
            DdlErrorCode::PlanChanged,
            DdlErrorCode::InvalidDef,
        ];
        for c in all {
            assert_eq!(serde_json::to_value(c).unwrap(), c.as_str());
            assert!(DDL_ERROR_CODES.contains(&c.as_str()));
        }
        assert_eq!(all.len(), DDL_ERROR_CODES.len());
    }
}
