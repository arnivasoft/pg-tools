use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectErrorKind {
    Auth,
    Network,
    Tls,
    Timeout,
    Other,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Yapılandırma hatası: {0}")]
    Config(String),
    #[error("Geçersiz ayar: {0}")]
    Invalid(String),
    #[error("Kimlik bilgisi deposu hatası: {0}")]
    Credential(String),
    #[error("Bu sunucu için parola gerekli.")]
    PasswordRequired,
    #[error("{message}")]
    Connect { kind: ConnectErrorKind, sqlstate: Option<String>, message: String },
    #[error("{message}")]
    Query { sqlstate: Option<String>, message: String, detail: Option<String>, hint: Option<String> },
    #[error("Nesne artık mevcut değil: {0}")]
    NotFound(String),
    #[error("Sunucu kayıtlı değil: {0}")]
    UnknownServer(Uuid),
    #[error("Sunucuya bağlı değil.")]
    NotConnected,
    /// Oturum vardı ama bağlantı koptu ve sessiz yeniden bağlanma başarısız oldu; oturum kapatıldı.
    /// İçerik, yeniden bağlanmayı engelleyen asıl hatadır.
    #[error("Sunucu bağlantısı koptu; yeniden bağlanmak için sunucuyu açın.")]
    ConnectionLost(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
}

fn friendly_sqlstate(code: &str) -> Option<&'static str> {
    match code {
        "28P01" | "28000" => Some("Kimlik doğrulama başarısız: kullanıcı adı veya parola hatalı."),
        "3D000" => Some("Veritabanı bulunamadı."),
        "42501" => Some("Bu işlem için yetkiniz yok."),
        "57014" => Some("Sorgu zaman aşımına uğradı (statement_timeout)."),
        "53300" => Some("Sunucu bağlantı sınırına ulaşıldı."),
        _ => None,
    }
}

fn payload(code: &str, message: String, detail: Option<String>, hint: Option<String>) -> ErrorPayload {
    ErrorPayload { code: code.to_string(), message, detail, hint }
}

impl From<&Error> for ErrorPayload {
    fn from(e: &Error) -> Self {
        match e {
            Error::Config(_) => payload("config", e.to_string(), None, None),
            Error::Invalid(_) => payload("invalid", e.to_string(), None, None),
            Error::Credential(_) => payload("credential", e.to_string(), None, None),
            Error::PasswordRequired => payload("password_required", e.to_string(), None, None),
            Error::Connect { kind, sqlstate, message } => {
                let friendly = sqlstate.as_deref().and_then(friendly_sqlstate);
                let (code, text) = match kind {
                    ConnectErrorKind::Auth => ("auth", friendly.map(str::to_string).unwrap_or_else(|| message.clone())),
                    ConnectErrorKind::Network => ("network", format!("Sunucuya ulaşılamadı: {message}")),
                    ConnectErrorKind::Tls => ("tls", format!("TLS/sertifika hatası: {message}")),
                    ConnectErrorKind::Timeout => ("timeout", "Bağlantı zaman aşımına uğradı (10 sn).".to_string()),
                    ConnectErrorKind::Other => ("connect", friendly.map(str::to_string).unwrap_or_else(|| message.clone())),
                };
                let detail = (text != *message).then(|| message.clone());
                payload(code, text, detail, None)
            }
            Error::Query { sqlstate, message, detail, hint } => match sqlstate.as_deref().and_then(friendly_sqlstate) {
                Some(f) => payload("query", f.to_string(), detail.clone().or_else(|| Some(message.clone())), hint.clone()),
                None => payload("query", message.clone(), detail.clone(), hint.clone()),
            },
            Error::NotFound(_) => payload("not_found", e.to_string(), None, None),
            Error::UnknownServer(_) => payload("unknown_server", e.to_string(), None, None),
            Error::NotConnected => payload("not_connected", e.to_string(), None, None),
            Error::ConnectionLost(cause) => payload("connection_lost", e.to_string(), Some(cause.clone()), None),
        }
    }
}

impl From<Error> for ErrorPayload {
    fn from(e: Error) -> Self {
        ErrorPayload::from(&e)
    }
}

impl Error {
    /// Sorgu sırasında oluşan tokio-postgres hatasını dönüştürür.
    pub fn from_query(e: tokio_postgres::Error) -> Self {
        match e.as_db_error() {
            Some(db) => Error::Query {
                sqlstate: Some(db.code().code().to_string()),
                message: db.message().to_string(),
                detail: db.detail().map(str::to_string),
                hint: db.hint().map(str::to_string),
            },
            None => Error::Query { sqlstate: None, message: chain_message(&e), detail: None, hint: None },
        }
    }

    /// Bağlantı kurulurken oluşan tokio-postgres hatasını sınıflandırır.
    pub fn from_connect(e: tokio_postgres::Error) -> Self {
        if let Some(db) = e.as_db_error() {
            let code = db.code().code();
            let kind = if code.starts_with("28") { ConnectErrorKind::Auth } else { ConnectErrorKind::Other };
            return Error::Connect { kind, sqlstate: Some(code.to_string()), message: db.message().to_string() };
        }
        let message = chain_message(&e);
        if message.contains("password missing") {
            return Error::PasswordRequired;
        }
        Error::Connect { kind: classify_source(&e), sqlstate: None, message }
    }
}

fn chain_message(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![e.to_string()];
    let mut cur = e.source();
    while let Some(err) = cur {
        let text = err.to_string();
        if !parts.iter().any(|p| p.contains(&text)) {
            parts.push(text);
        }
        cur = err.source();
    }
    parts.join(": ")
}

fn classify_source(e: &(dyn std::error::Error + 'static)) -> ConnectErrorKind {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = cur {
        if err.downcast_ref::<rustls::Error>().is_some() {
            return ConnectErrorKind::Tls;
        }
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            if io.get_ref().is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_some()) {
                return ConnectErrorKind::Tls;
            }
            if io.kind() == std::io::ErrorKind::TimedOut {
                return ConnectErrorKind::Timeout;
            }
            return ConnectErrorKind::Network;
        }
        cur = err.source();
    }
    ConnectErrorKind::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_connect_error_gets_turkish_message() {
        let e = Error::Connect {
            kind: ConnectErrorKind::Auth,
            sqlstate: Some("28P01".into()),
            message: "password authentication failed for user \"x\"".into(),
        };
        let p = ErrorPayload::from(&e);
        assert_eq!(p.code, "auth");
        assert_eq!(p.message, "Kimlik doğrulama başarısız: kullanıcı adı veya parola hatalı.");
        assert_eq!(p.detail.as_deref(), Some("password authentication failed for user \"x\""));
    }

    #[test]
    fn network_error_wraps_original_message() {
        let e = Error::Connect { kind: ConnectErrorKind::Network, sqlstate: None, message: "connection refused".into() };
        let p = ErrorPayload::from(&e);
        assert_eq!(p.code, "network");
        assert_eq!(p.message, "Sunucuya ulaşılamadı: connection refused");
    }

    #[test]
    fn timeout_and_tls_codes() {
        let t = ErrorPayload::from(&Error::Connect { kind: ConnectErrorKind::Timeout, sqlstate: None, message: "x".into() });
        assert_eq!((t.code.as_str(), t.message.as_str()), ("timeout", "Bağlantı zaman aşımına uğradı (10 sn)."));
        let s = ErrorPayload::from(&Error::Connect { kind: ConnectErrorKind::Tls, sqlstate: None, message: "bad cert".into() });
        assert_eq!((s.code.as_str(), s.message.as_str()), ("tls", "TLS/sertifika hatası: bad cert"));
    }

    #[test]
    fn query_error_with_known_sqlstate_is_translated_and_keeps_hint() {
        let e = Error::Query {
            sqlstate: Some("42501".into()),
            message: "permission denied for table x".into(),
            detail: None,
            hint: Some("ipucu".into()),
        };
        let p = ErrorPayload::from(&e);
        assert_eq!(p.code, "query");
        assert_eq!(p.message, "Bu işlem için yetkiniz yok.");
        assert_eq!(p.detail.as_deref(), Some("permission denied for table x"));
        assert_eq!(p.hint.as_deref(), Some("ipucu"));
    }

    #[test]
    fn query_error_with_unknown_sqlstate_keeps_message() {
        let e = Error::Query { sqlstate: Some("XX000".into()), message: "boom".into(), detail: Some("d".into()), hint: None };
        let p = ErrorPayload::from(&e);
        assert_eq!((p.message.as_str(), p.detail.as_deref()), ("boom", Some("d")));
    }

    #[test]
    fn connection_lost_is_distinct_from_not_connected_and_keeps_cause() {
        let p = ErrorPayload::from(&Error::ConnectionLost("connection refused".into()));
        assert_eq!(p.code, "connection_lost");
        assert_ne!(p.code, ErrorPayload::from(&Error::NotConnected).code);
        assert_eq!(p.detail.as_deref(), Some("connection refused"));
        assert!(p.message.contains("koptu"), "{}", p.message);
    }

    #[test]
    fn simple_variants_have_stable_codes() {
        let cases = [
            (Error::Config("c".into()), "config"),
            (Error::Invalid("i".into()), "invalid"),
            (Error::Credential("k".into()), "credential"),
            (Error::PasswordRequired, "password_required"),
            (Error::NotFound("n".into()), "not_found"),
            (Error::UnknownServer(uuid::Uuid::nil()), "unknown_server"),
            (Error::NotConnected, "not_connected"),
        ];
        for (e, code) in cases {
            let p = ErrorPayload::from(&e);
            assert_eq!(p.code, code);
            assert_eq!(p.message, e.to_string());
        }
    }
}
