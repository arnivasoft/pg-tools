use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::config::{SslConfig, SslMode};
use crate::{Error, Result};

pub fn pg_ssl_mode(mode: SslMode) -> tokio_postgres::config::SslMode {
    use tokio_postgres::config::SslMode as Pg;
    match mode {
        SslMode::Disable => Pg::Disable,
        SslMode::Prefer => Pg::Prefer,
        SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => Pg::Require,
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn config_err(e: impl std::fmt::Display) -> Error {
    Error::Config(format!("TLS yapılandırılamadı: {e}"))
}

pub fn client_config(ssl: &SslConfig) -> Result<ClientConfig> {
    if ssl.client_cert.is_some() != ssl.client_key.is_some() {
        return Err(Error::Invalid("İstemci sertifikası ve anahtarı birlikte verilmelidir.".into()));
    }
    let provider = provider();
    let verifier: Arc<dyn ServerCertVerifier> = match ssl.mode {
        SslMode::Disable | SslMode::Prefer | SslMode::Require => Arc::new(NoVerification(provider.clone())),
        SslMode::VerifyCa => Arc::new(IgnoreHostname(base_verifier(ssl, provider.clone())?)),
        SslMode::VerifyFull => base_verifier(ssl, provider.clone())?,
    };
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(config_err)?
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    match (&ssl.client_cert, &ssl.client_key) {
        (Some(cert), Some(key)) => builder.with_client_auth_cert(load_certs(cert)?, load_key(key)?).map_err(config_err),
        _ => Ok(builder.with_no_client_auth()),
    }
}

pub fn make_connector(ssl: &SslConfig) -> Result<MakeRustlsConnect> {
    Ok(MakeRustlsConnect::new(client_config(ssl)?))
}

fn base_verifier(ssl: &SslConfig, provider: Arc<CryptoProvider>) -> Result<Arc<dyn ServerCertVerifier>> {
    match &ssl.root_cert {
        Some(path) => {
            let mut roots = RootCertStore::empty();
            for cert in load_certs(path)? {
                roots.add(cert).map_err(config_err)?;
            }
            let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
                .build()
                .map_err(config_err)?;
            Ok(verifier)
        }
        None => Ok(Arc::new(rustls_platform_verifier::Verifier::new(provider).map_err(config_err)?)),
    }
}

fn load_certs(path: &str) -> Result<Vec<CertificateDer<'static>>> {
    let read_err = |e: &dyn std::fmt::Display| Error::Config(format!("Sertifika dosyası okunamadı ({path}): {e}"));
    let certs = CertificateDer::pem_file_iter(path)
        .map_err(|e| read_err(&e))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| read_err(&e))?;
    if certs.is_empty() {
        return Err(read_err(&"dosyada sertifika yok"));
    }
    Ok(certs)
}

fn load_key(path: &str) -> Result<PrivateKeyDer<'static>> {
    PrivateKeyDer::from_pem_file(path).map_err(|e| {
        Error::Config(format!(
            "İstemci anahtarı okunamadı ({path}): {e}. Şifreli anahtarlar desteklenmez; \
             `openssl pkcs8 -topk8 -nocrypt -in eski.key -out yeni.key` ile dönüştürün."
        ))
    })
}

/// prefer/require: sertifika zinciri doğrulanmaz (libpq davranışı), el sıkışma imzası doğrulanır.
#[derive(Debug)]
struct NoVerification(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerification {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// verify-ca: zinciri iç doğrulayıcıya bırakır, yalnızca host adı uyuşmazlığını yok sayar.
#[derive(Debug)]
struct IgnoreHostname(Arc<dyn ServerCertVerifier>);

impl ServerCertVerifier for IgnoreHostname {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, intermediates: &[CertificateDer<'_>], server_name: &ServerName<'_>, ocsp: &[u8], now: UnixTime) -> std::result::Result<ServerCertVerified, rustls::Error> {
        match self.0.verify_server_cert(end_entity, intermediates, server_name, ocsp, now) {
            Err(rustls::Error::InvalidCertificate(CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. })) => {
                Ok(ServerCertVerified::assertion())
            }
            other => other,
        }
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    fn ssl(mode: SslMode) -> SslConfig {
        SslConfig { mode, ..SslConfig::default() }
    }

    fn write_ca(dir: &tempfile::TempDir) -> String {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["test-ca".to_string()]).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key).unwrap();
        let path = dir.path().join("ca.pem");
        std::fs::write(&path, cert.pem()).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn maps_ssl_modes_to_tokio_postgres() {
        use tokio_postgres::config::SslMode as Pg;
        assert_eq!(pg_ssl_mode(SslMode::Disable), Pg::Disable);
        assert_eq!(pg_ssl_mode(SslMode::Prefer), Pg::Prefer);
        assert_eq!(pg_ssl_mode(SslMode::Require), Pg::Require);
        assert_eq!(pg_ssl_mode(SslMode::VerifyCa), Pg::Require);
        assert_eq!(pg_ssl_mode(SslMode::VerifyFull), Pg::Require);
    }

    #[test]
    fn builds_config_for_every_mode_without_files() {
        for mode in [SslMode::Disable, SslMode::Prefer, SslMode::Require, SslMode::VerifyCa, SslMode::VerifyFull] {
            assert!(client_config(&ssl(mode)).is_ok(), "{mode:?}");
        }
    }

    #[test]
    fn loads_root_cert_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = ssl(SslMode::VerifyFull);
        c.root_cert = Some(write_ca(&dir));
        assert!(client_config(&c).is_ok());
    }

    #[test]
    fn missing_root_cert_file_is_config_error_with_path() {
        let mut c = ssl(SslMode::VerifyCa);
        c.root_cert = Some("C:\\yok\\ca.pem".into());
        match client_config(&c) {
            Err(Error::Config(m)) => assert!(m.contains("C:\\yok\\ca.pem"), "{m}"),
            Err(other) => panic!("beklenmeyen hata: {other:?}"),
            Ok(_) => panic!("hata bekleniyordu"),
        }
    }

    #[test]
    fn cert_without_key_is_rejected() {
        let mut c = ssl(SslMode::Require);
        c.client_cert = Some("c.pem".into());
        assert!(matches!(client_config(&c), Err(Error::Invalid(_))));
    }

    #[test]
    fn unreadable_client_key_mentions_encrypted_keys() {
        let dir = tempfile::tempdir().unwrap();
        let cert = write_ca(&dir);
        let key = dir.path().join("enc.key");
        std::fs::write(&key, "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n").unwrap();
        let mut c = ssl(SslMode::Require);
        c.client_cert = Some(cert);
        c.client_key = Some(key.to_string_lossy().into_owned());
        match client_config(&c) {
            Err(Error::Config(m)) => assert!(m.contains("pkcs8"), "{m}"),
            Err(other) => panic!("beklenmeyen hata: {other:?}"),
            Ok(_) => panic!("hata bekleniyordu"),
        }
    }

    #[derive(Debug)]
    struct Fixed(rustls::Error);
    impl ServerCertVerifier for Fixed {
        fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> std::result::Result<ServerCertVerified, rustls::Error> {
            Err(self.0.clone())
        }
        fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![]
        }
    }

    fn verify(v: &dyn ServerCertVerifier) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let cert = CertificateDer::from(vec![0u8]);
        v.verify_server_cert(&cert, &[], &ServerName::try_from("db.local").unwrap(), &[], UnixTime::now())
    }

    #[test]
    fn ignore_hostname_accepts_name_mismatch_only() {
        let mismatch = IgnoreHostname(Arc::new(Fixed(rustls::Error::InvalidCertificate(CertificateError::NotValidForName))));
        assert!(verify(&mismatch).is_ok());
        let untrusted = IgnoreHostname(Arc::new(Fixed(rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer))));
        assert!(verify(&untrusted).is_err());
    }
}
