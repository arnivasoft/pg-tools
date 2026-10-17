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
/// Boş plan uygulanmak istendi; kodlu değildir (düz `invalid`).
pub const NO_CHANGES: &str = "Değişiklik yok";
/// Aynı `apply_id` ile süren bir uygulama var; kodlu değildir (düz `invalid`).
pub const APPLY_ID_IN_USE: &str = "Bu uygulama kimliği zaten kullanımda.";
/// Plan gizlenmiş parola ifadesi içeriyor ama `ApplyOptions.password` gelmedi (pgt-core bağlantı `password_required`
/// koduyla karışmasın diye ayrı ad).
pub const ROLE_PASSWORD_MISSING: &str = "Bu plan rol parolası ayarlıyor; parola girilmedi.";
/// Parola gönderildi ama planda gizlenmiş ifade yok; kodlu değildir (düz `invalid`).
pub const PASSWORD_NOT_USED: &str = "Bu planda parola kullanılmıyor.";
/// Bakım veritabanının kendisi hedeflendi; kodlu değildir (düz `invalid`).
pub const MAINTENANCE_DATABASE: &str =
    "Bakım veritabanı yeniden adlandırılamaz, silinemez ya da taşınamaz; sunucu ayarlarından başka bir bakım veritabanı seçin.";
/// Bakım veritabanı şablon olarak kopyalanmak istendi; kodlu değildir (düz `invalid`).
pub const MAINTENANCE_TEMPLATE: &str = "Bakım veritabanı şablon olarak kullanılamaz (uygulamanın kendi bağlantısı ona bağlıdır); \
     template0, template1 ya da başka bir veritabanı seçin.";
/// Veritabanı kapsamlı türde `database_oid` yok; kodlu değildir (düz `invalid`).
pub const NEEDS_DATABASE: &str = "Bu nesne türü bir veritabanı gerektirir.";
/// Veri düzenleyici: güncellenen/silinen satır başka oturumca değişmiş ya da silinmiş (anahtar + `xmin` 0 satır
/// eşledi); tüm transaction geri alındı.
pub const ROW_CONFLICT_MESSAGE: &str =
    "Satır başka bir oturum tarafından değiştirilmiş ya da silinmiş; değişiklikler uygulanmadı.";

/// `Error::NotFound` konusu: rol (rol tanımı yüklemesi ve varsayılan yetkilerde kapsamın rolü). İleti ad içermez.
pub const ROLE_SUBJECT: &str = "rol";
/// Rol yokluğunun ayrı kodu (`not_found` alt türü; ileti aynı, adsız). Arayüz varsayılan yetkilerde kapsamın rolünün
/// yokluğunu (bağlı role düş / satır içi hata) şema/veritabanı yokluğundan (`not_found` → sekme düşer) böyle ayırır.
/// Kodsuz `DdlErrorCode` listesinde değildir (kapı reddi değil, yokluk sınıfıdır).
pub const ROLE_NOT_FOUND_CODE: &str = "role_not_found";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DdlErrorCode {
    Locked,
    ConfirmationRequired,
    ProductionConfirmationRequired,
    PlanChanged,
    InvalidDef,
    RolePasswordMissing,
    RowConflict,
}

/// Arayüzün eşleştirebileceği kodların tam listesi (`DdlErrorCode`'un serde adları).
pub const DDL_ERROR_CODES: [&str; 7] = [
    "locked",
    "confirmation_required",
    "production_confirmation_required",
    "plan_changed",
    "invalid_def",
    "role_password_missing",
    "row_conflict",
];

impl DdlErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            DdlErrorCode::Locked => DDL_ERROR_CODES[0],
            DdlErrorCode::ConfirmationRequired => DDL_ERROR_CODES[1],
            DdlErrorCode::ProductionConfirmationRequired => DDL_ERROR_CODES[2],
            DdlErrorCode::PlanChanged => DDL_ERROR_CODES[3],
            DdlErrorCode::InvalidDef => DDL_ERROR_CODES[4],
            DdlErrorCode::RolePasswordMissing => DDL_ERROR_CODES[5],
            DdlErrorCode::RowConflict => DDL_ERROR_CODES[6],
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
            m if m == ROLE_PASSWORD_MISSING => (Self::RolePasswordMissing, ROLE_PASSWORD_MISSING),
            m if m == ROW_CONFLICT_MESSAGE => (Self::RowConflict, ROW_CONFLICT_MESSAGE),
            _ => return None,
        })
    }
}

/// Komut katmanı hata eşlemesi: işaretli `Invalid` hataları ayrı koda, rol yokluğu (`NotFound(ROLE_SUBJECT)`)
/// `role_not_found` koduna (ileti `not_found` ile aynı), gerisi `ErrorPayload::from`.
pub fn ddl_payload(e: &Error) -> ErrorPayload {
    if let Error::Invalid(m) = e
        && let Some((code, message)) = DdlErrorCode::from_marker(m)
    {
        return ErrorPayload { code: code.as_str().to_string(), message: message.to_string(), detail: None, hint: None };
    }
    if let Error::NotFound(what) = e
        && what == ROLE_SUBJECT
    {
        return ErrorPayload { code: ROLE_NOT_FOUND_CODE.to_string(), ..ErrorPayload::from(e) };
    }
    ErrorPayload::from(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker_of(c: DdlErrorCode) -> &'static str {
        match c {
            DdlErrorCode::Locked => LOCKED_MESSAGE,
            DdlErrorCode::ConfirmationRequired => CONFIRMATION_REQUIRED,
            DdlErrorCode::ProductionConfirmationRequired => PRODUCTION_CONFIRMATION_REQUIRED,
            DdlErrorCode::PlanChanged => PLAN_CHANGED,
            DdlErrorCode::InvalidDef => INVALID_DEF,
            DdlErrorCode::RolePasswordMissing => ROLE_PASSWORD_MISSING,
            DdlErrorCode::RowConflict => ROW_CONFLICT_MESSAGE,
        }
    }

    #[test]
    fn every_marker_maps_to_its_code_without_prefix() {
        let cases = [
            (LOCKED_MESSAGE, "locked"),
            (CONFIRMATION_REQUIRED, "confirmation_required"),
            (PRODUCTION_CONFIRMATION_REQUIRED, "production_confirmation_required"),
            (PLAN_CHANGED, "plan_changed"),
            (INVALID_DEF, "invalid_def"),
            (ROLE_PASSWORD_MISSING, "role_password_missing"),
            (ROW_CONFLICT_MESSAGE, "row_conflict"),
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
        // Kodsuz düz iletiler.
        for m in [PASSWORD_NOT_USED, MAINTENANCE_DATABASE, MAINTENANCE_TEMPLATE, NEEDS_DATABASE, NO_CHANGES, APPLY_ID_IN_USE] {
            assert_eq!(ddl_payload(&Error::Invalid(m.into())).code, "invalid", "{m}");
        }
    }

    #[test]
    fn role_not_found_has_its_own_code_and_the_generic_message() {
        let p = ddl_payload(&Error::NotFound(ROLE_SUBJECT.into()));
        assert_eq!(p.code, ROLE_NOT_FOUND_CODE);
        assert_eq!(p.message, ErrorPayload::from(&Error::NotFound(ROLE_SUBJECT.into())).message);
        // Şema/veritabanı/nesne yokluğu düz `not_found` kalır.
        for w in ["şema", "veritabanı", "nesne", "tablo"] {
            assert_eq!(ddl_payload(&Error::NotFound(w.into())).code, "not_found", "{w}");
        }
        assert!(!DDL_ERROR_CODES.contains(&ROLE_NOT_FOUND_CODE));
    }

    #[test]
    fn code_list_matches_serde_names() {
        let all = [
            DdlErrorCode::Locked,
            DdlErrorCode::ConfirmationRequired,
            DdlErrorCode::ProductionConfirmationRequired,
            DdlErrorCode::PlanChanged,
            DdlErrorCode::InvalidDef,
            DdlErrorCode::RolePasswordMissing,
            DdlErrorCode::RowConflict,
        ];
        for c in all {
            assert_eq!(serde_json::to_value(c).unwrap(), c.as_str());
            assert!(DDL_ERROR_CODES.contains(&c.as_str()));
            // as_str → işaret metni → from_marker gidiş-dönüşü.
            let (_, marker) = DdlErrorCode::from_marker(marker_of(c)).unwrap();
            assert_eq!(DdlErrorCode::from_marker(marker).map(|(k, _)| k), Some(c));
        }
        assert_eq!(all.len(), DDL_ERROR_CODES.len());
        assert_eq!(DDL_ERROR_CODES.len(), 7);
        assert_eq!(DDL_ERROR_CODES[6], "row_conflict");
        // pgt-core bağlantı kodu ile çakışmaz.
        assert!(!DDL_ERROR_CODES.contains(&"password_required"));
    }
}
