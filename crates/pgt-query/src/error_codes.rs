//! Sorgu aracının kapı/durum reddi hatalarının arayüze giden ayrı hata kodları. Arayüz `ErrorPayload.code`
//! üzerinden eşleşir, metin üzerinden asla.

use pgt_core::{Error, ErrorPayload};
use serde::Serialize;
use ts_rs::TS;

use crate::session::{
    BUSY_MESSAGE, CONFIRMATION_REQUIRED, LOCKED_SETTING_MESSAGE, LOCKED_WRITE_MESSAGE, MODE_IN_TX_MESSAGE, OPEN_TX_MESSAGE,
    TX_FAILED_MESSAGE,
};

/// `QueryManager`'ın `Error::Invalid(<işaret>)` ile bildirdiği reddin kodu. `invalid` (diğer her geçersiz girdi)
/// ve öteki genel kodlar (`query`, `not_found`, `not_connected`, …) `pgt_core::ErrorPayload`'dan gelir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum QueryErrorCode {
    /// Tehlikeli / üretimde yazan ifade onaysız çalıştırıldı; `confirmed = true` ile yeniden denenir.
    ConfirmationRequired,
    /// Sunucu salt okunur kilitli; ifade ya da ayar reddedildi.
    Locked,
    /// Oturumda bir işlem zaten çalışıyor.
    Busy,
    /// Transaction hata durumunda; yalnızca ROLLBACK kabul edilir.
    TxFailed,
    /// Açık transaction var (kapatma reddi ya da mod değişimi reddi).
    OpenTransaction,
}

/// Arayüzün eşleştirebileceği kodların tam listesi (`QueryErrorCode`'un serde adları).
pub const QUERY_ERROR_CODES: [&str; 5] = ["confirmation_required", "locked", "busy", "tx_failed", "open_transaction"];

impl QueryErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            QueryErrorCode::ConfirmationRequired => QUERY_ERROR_CODES[0],
            QueryErrorCode::Locked => QUERY_ERROR_CODES[1],
            QueryErrorCode::Busy => QUERY_ERROR_CODES[2],
            QueryErrorCode::TxFailed => QUERY_ERROR_CODES[3],
            QueryErrorCode::OpenTransaction => QUERY_ERROR_CODES[4],
        }
    }

    /// Yönetici işaret metnini koda ve Türkçe iletiye çevirir; bilinmeyen metin `None`.
    pub fn from_marker(marker: &str) -> Option<(Self, &'static str)> {
        Some(match marker {
            m if m == CONFIRMATION_REQUIRED => (Self::ConfirmationRequired, "Bu betik onay gerektiriyor."),
            m if m == LOCKED_WRITE_MESSAGE => (Self::Locked, LOCKED_WRITE_MESSAGE),
            m if m == LOCKED_SETTING_MESSAGE => (Self::Locked, LOCKED_SETTING_MESSAGE),
            m if m == BUSY_MESSAGE => (Self::Busy, BUSY_MESSAGE),
            m if m == TX_FAILED_MESSAGE => (Self::TxFailed, TX_FAILED_MESSAGE),
            m if m == OPEN_TX_MESSAGE => (Self::OpenTransaction, OPEN_TX_MESSAGE),
            m if m == MODE_IN_TX_MESSAGE => (Self::OpenTransaction, MODE_IN_TX_MESSAGE),
            _ => return None,
        })
    }
}

/// Komut katmanı hata eşlemesi: işaretli `Invalid` hataları ayrı koda, gerisi `ErrorPayload::from`.
pub fn query_payload(e: &Error) -> ErrorPayload {
    if let Error::Invalid(m) = e
        && let Some((code, message)) = QueryErrorCode::from_marker(m)
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
            (CONFIRMATION_REQUIRED, "confirmation_required"),
            (LOCKED_WRITE_MESSAGE, "locked"),
            (LOCKED_SETTING_MESSAGE, "locked"),
            (BUSY_MESSAGE, "busy"),
            (TX_FAILED_MESSAGE, "tx_failed"),
            (OPEN_TX_MESSAGE, "open_transaction"),
            (MODE_IN_TX_MESSAGE, "open_transaction"),
        ];
        for (marker, code) in cases {
            let p = query_payload(&Error::Invalid(marker.into()));
            assert_eq!(p.code, code, "{marker}");
            assert!(!p.message.contains("Geçersiz ayar"), "{}", p.message);
            assert!(!p.message.is_empty());
            if marker != CONFIRMATION_REQUIRED {
                assert_eq!(p.message, marker);
            }
        }
    }

    #[test]
    fn unknown_invalid_and_other_errors_keep_their_generic_codes() {
        let p = query_payload(&Error::Invalid("açık imleç yok".into()));
        assert_eq!(p.code, "invalid");
        assert_eq!(p.message, "Geçersiz ayar: açık imleç yok");
        assert_eq!(query_payload(&Error::NotConnected).code, "not_connected");
        assert_eq!(query_payload(&Error::NotFound("sorgu oturumu".into())).code, "not_found");
    }

    #[test]
    fn code_list_matches_serde_names() {
        let all = [
            QueryErrorCode::ConfirmationRequired,
            QueryErrorCode::Locked,
            QueryErrorCode::Busy,
            QueryErrorCode::TxFailed,
            QueryErrorCode::OpenTransaction,
        ];
        for c in all {
            assert_eq!(serde_json::to_value(c).unwrap(), c.as_str());
            assert!(QUERY_ERROR_CODES.contains(&c.as_str()));
        }
        assert_eq!(all.len(), QUERY_ERROR_CODES.len());
    }
}
