use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::config::{ServerConfig, SslMode};
use crate::registry::Registry;
use crate::{Error, Result};

const NOT_PGADMIN: &str = "Dosya pgAdmin servers.json biçiminde değil.";
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportItem {
    pub index: u32,
    pub name: String,
    pub group: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub duplicate: bool,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct Parsed {
    pub item: ImportItem,
    pub config: Option<ServerConfig>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn number(v: &Value, key: &str) -> Option<i64> {
    match v.get(key)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn truthy(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().is_some_and(|n| n != 0),
        Some(Value::String(s)) => matches!(s.trim(), "1" | "true" | "True"),
        _ => false,
    }
}

/// `ConnectionParameters.<param>` önce, yoksa düz `<flat>` alanı.
fn pick(v: &Value, flat: &str, param: &str) -> Option<String> {
    v.get("ConnectionParameters")
        .and_then(|p| text(p, param))
        .or_else(|| text(v, flat))
}

fn ssl_mode(raw: Option<String>, warnings: &mut Vec<String>) -> SslMode {
    let Some(raw) = raw else {
        return SslMode::Prefer;
    };
    match raw.to_ascii_lowercase().as_str() {
        "disable" => SslMode::Disable,
        "prefer" => SslMode::Prefer,
        "require" => SslMode::Require,
        "verify-ca" => SslMode::VerifyCa,
        "verify-full" => SslMode::VerifyFull,
        "allow" => {
            warnings.push("SSL modu “allow” desteklenmiyor; “prefer” olarak aktarıldı.".into());
            SslMode::Prefer
        }
        other => {
            warnings.push(format!(
                "Bilinmeyen SSL modu “{other}”; “prefer” olarak aktarıldı."
            ));
            SslMode::Prefer
        }
    }
}

/// Yinelenme ölçütü: host (büyük/küçük harf duyarsız) + port + kullanıcı + ad.
fn same_server(c: &ServerConfig, host: &str, port: u16, username: &str, name: &str) -> bool {
    c.host.eq_ignore_ascii_case(host) && c.port == port && c.username == username && c.name == name
}

fn parse_entry(index: u32, v: &Value, existing: &[ServerConfig]) -> Parsed {
    let mut warnings = Vec::new();
    let host = text(v, "Host").unwrap_or_default();
    let username = text(v, "Username").unwrap_or_default();
    let group = text(v, "Group");
    let mut error = None;
    let port = match v.get("Port") {
        None | Some(Value::Null) => 5432,
        Some(_) => match number(v, "Port") {
            Some(p) if (1..=65535).contains(&p) => p as u16,
            _ => {
                error = Some("Geçersiz port.".to_string());
                0
            }
        },
    };
    if error.is_none() && host.is_empty() {
        error = Some("Host alanı yok.".into());
    }
    if error.is_none() && username.is_empty() {
        error = Some("Kullanıcı adı yok.".into());
    }
    let name = text(v, "Name").unwrap_or_else(|| format!("{host}:{port}"));
    if truthy(v, "UseSSHTunnel") {
        warnings.push("SSH tüneli ayarları yok sayıldı.".into());
    }
    if pick(v, "PassFile", "passfile").is_some() {
        warnings.push("passfile yok sayıldı (pgpass desteklenmiyor).".into());
    }
    if text(v, "Service").is_some() {
        warnings.push("service tanımı yok sayıldı.".into());
    }
    let mode = ssl_mode(pick(v, "SSLMode", "sslmode"), &mut warnings);
    let (mut cert, mut key) = (pick(v, "SSLCert", "sslcert"), pick(v, "SSLKey", "sslkey"));
    if cert.is_some() != key.is_some() {
        warnings.push(
            "İstemci sertifikası ve anahtarı birlikte verilmediği için ikisi de yok sayıldı."
                .into(),
        );
        (cert, key) = (None, None);
    }
    let duplicate = error.is_none()
        && existing
            .iter()
            .any(|s| same_server(s, &host, port, &username, &name));

    let mut config = None;
    if error.is_none() {
        let mut c = ServerConfig::new(name.clone(), host.clone(), port, username.clone());
        c.group = group.clone();
        c.maintenance_db = text(v, "MaintenanceDB").unwrap_or_else(|| "postgres".into());
        c.ssl.mode = mode;
        c.ssl.root_cert = pick(v, "SSLRootCert", "sslrootcert");
        c.ssl.client_cert = cert;
        c.ssl.client_key = key;
        let c = c.normalized();
        match c.validate() {
            Ok(()) => config = Some(c),
            Err(e) => error = Some(e.to_string()),
        }
    }
    Parsed {
        item: ImportItem {
            index,
            name,
            group,
            host,
            port,
            username,
            duplicate,
            warnings,
            error,
        },
        config,
    }
}

pub fn parse(text: &str, existing: &[ServerConfig]) -> Result<Vec<Parsed>> {
    let invalid = || Error::Invalid(NOT_PGADMIN.into());
    let root: Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| invalid())?;
    let servers: &Map<String, Value> = root
        .get("Servers")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let mut entries: Vec<(&String, &Value)> = servers.iter().collect();
    entries.sort_by_key(|(k, _)| (k.parse::<u64>().unwrap_or(u64::MAX), k.to_string()));
    let mut parsed: Vec<Parsed> = Vec::with_capacity(entries.len());
    for (i, (_, v)) in entries.into_iter().enumerate() {
        let mut p = parse_entry(i as u32, v, existing);
        // Aynı dosyada daha önce geçen geçerli bir kaydın kopyası da "zaten var" sayılır.
        if p.config.is_some()
            && parsed
                .iter()
                .filter_map(|q| q.config.as_ref())
                .any(|c| same_server(c, &p.item.host, p.item.port, &p.item.username, &p.item.name))
        {
            p.item.duplicate = true;
        }
        parsed.push(p);
    }
    Ok(parsed)
}

pub fn preview(text: &str, existing: &[ServerConfig]) -> Result<Vec<ImportItem>> {
    Ok(parse(text, existing)?.into_iter().map(|p| p.item).collect())
}

/// Seçilen (ve geçerli) kayıtları yeni UUID'lerle ekler; eklenen sayıyı döner.
/// Aynı dosya içindeki kopyalardan (host+port+kullanıcı+ad) yalnızca ilk seçilen eklenir.
pub fn apply(registry: &mut Registry, text: &str, indices: &[u32]) -> Result<u32> {
    let existing = registry.servers().to_vec();
    let mut added: Vec<ServerConfig> = Vec::new();
    for p in parse(text, &existing)? {
        let Some(config) = p.config.filter(|_| indices.contains(&p.item.index)) else {
            continue;
        };
        if added
            .iter()
            .any(|a| same_server(a, &config.host, config.port, &config.username, &config.name))
        {
            continue;
        }
        registry.upsert(config.clone())?;
        added.push(config);
    }
    Ok(added.len() as u32)
}

/// Dosyayı sınırlı boyutta okur.
pub fn read_file(path: &Path) -> Result<String> {
    let meta =
        std::fs::metadata(path).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(Error::Invalid("Dosya çok büyük (en fazla 5 MB).".into()));
    }
    let bytes =
        std::fs::read(path).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    decode(bytes)
}

const UTF16: &str = "Dosya UTF-16 kodlanmış; lütfen UTF-8 olarak kaydedip yeniden deneyin.";

fn decode(bytes: Vec<u8>) -> Result<String> {
    // BOM'lu UTF-16 ya da BOM'suz UTF-16 (JSON metninde ham NUL baytı geçemez; UTF-16'da
    // ASCII karakterlerin yarısı NUL'dur).
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) || bytes.contains(&0) {
        return Err(Error::Invalid(UTF16.into()));
    }
    String::from_utf8(bytes).map_err(|_| {
        Error::Invalid("Dosya UTF-8 olarak okunamadı; lütfen UTF-8 olarak kaydedin.".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = r#"{"Servers":{"1":{"Name":"prod","Group":"Üretim","Host":"db1","Port":5433,"MaintenanceDB":"app","Username":"u","SSLMode":"verify-full","SSLRootCert":"C:\\ca.pem"}}}"#;
    const NEW: &str = r#"{"Servers":{"1":{"Name":"n","Group":"G","Host":"h","Port":"5432","Username":"u","ConnectionParameters":{"sslmode":"require","sslcert":"c.pem","sslkey":"k.pem"}}}}"#;

    fn one(json: &str) -> Parsed {
        parse(json, &[]).unwrap().remove(0)
    }

    #[test]
    fn old_flat_format_maps_every_field() {
        let c = one(OLD).config.unwrap();
        assert_eq!(
            (c.name.as_str(), c.group.as_deref(), c.host.as_str(), c.port),
            ("prod", Some("Üretim"), "db1", 5433)
        );
        assert_eq!(
            (c.maintenance_db.as_str(), c.username.as_str()),
            ("app", "u")
        );
        assert_eq!(c.ssl.mode, SslMode::VerifyFull);
        assert_eq!(c.ssl.root_cert.as_deref(), Some("C:\\ca.pem"));
        assert!(!c.save_password);
    }

    #[test]
    fn connection_parameters_format_and_string_port() {
        let p = one(NEW);
        let c = p.config.unwrap();
        assert_eq!((c.port, c.ssl.mode), (5432, SslMode::Require));
        assert_eq!(
            (c.ssl.client_cert.as_deref(), c.ssl.client_key.as_deref()),
            (Some("c.pem"), Some("k.pem"))
        );
        assert_eq!(c.maintenance_db, "postgres");
    }

    #[test]
    fn connection_parameters_win_over_flat_fields() {
        let j = r#"{"Servers":{"1":{"Name":"n","Host":"h","Username":"u","SSLMode":"disable","ConnectionParameters":{"sslmode":"verify-ca"}}}}"#;
        assert_eq!(one(j).config.unwrap().ssl.mode, SslMode::VerifyCa);
    }

    #[test]
    fn utf8_bom_is_accepted_and_numeric_keys_sort_numerically() {
        let j = format!(
            "\u{feff}{}",
            r#"{"Servers":{"10":{"Name":"ten","Host":"h","Username":"u"},"2":{"Name":"two","Host":"h","Username":"u"},"1":{"Name":"one","Host":"h","Username":"u"}}}"#
        );
        let names: Vec<_> = parse(&j, &[])
            .unwrap()
            .into_iter()
            .map(|p| p.item.name)
            .collect();
        assert_eq!(names, ["one", "two", "ten"]);
    }

    #[test]
    fn indices_are_stable_positions() {
        let items = preview(OLD, &[]).unwrap();
        assert_eq!(items[0].index, 0);
    }

    #[test]
    fn not_pgadmin_files_are_rejected_with_a_clear_message() {
        for bad in [
            "",
            "not json",
            "[]",
            r#"{"servers":{}}"#,
            r#"{"Servers":[1]}"#,
        ] {
            let err = parse(bad, &[]).unwrap_err();
            assert!(
                matches!(&err, Error::Invalid(m) if m.contains("pgAdmin")),
                "{bad:?} -> {err:?}"
            );
        }
    }

    #[test]
    fn missing_or_bad_fields_become_item_errors_not_failures() {
        let j = r#"{"Servers":{"1":{"Name":"a","Username":"u"},"2":{"Name":"b","Host":"h"},"3":{"Name":"c","Host":"h","Username":"u","Port":70000},"4":{"Name":"d","Host":"h","Username":"u","Port":"x"}}}"#;
        let items = preview(j, &[]).unwrap();
        assert_eq!(items.len(), 4);
        assert!(items.iter().all(|i| i.error.is_some()), "{items:?}");
        assert!(items[0].error.as_deref().unwrap().contains("Host"));
        assert!(items[1].error.as_deref().unwrap().contains("Kullanıcı"));
        assert!(items[2].error.as_deref().unwrap().contains("port"));
    }

    #[test]
    fn missing_port_defaults_to_5432_and_missing_name_falls_back_to_host_port() {
        let c = one(r#"{"Servers":{"1":{"Host":"h","Username":"u"}}}"#)
            .config
            .unwrap();
        assert_eq!((c.port, c.name.as_str()), (5432, "h:5432"));
    }

    #[test]
    fn unsupported_fields_produce_warnings() {
        let j = r#"{"Servers":{"1":{"Name":"n","Host":"h","Username":"u","UseSSHTunnel":1,"TunnelHost":"x","PassFile":"/p","Service":"s","SSLMode":"allow"}}}"#;
        let p = one(j);
        let w = p.item.warnings.join("|");
        assert!(w.contains("SSH"), "{w}");
        assert!(w.contains("passfile"), "{w}");
        assert!(w.contains("service"), "{w}");
        assert!(w.contains("allow"), "{w}");
        assert_eq!(p.config.unwrap().ssl.mode, SslMode::Prefer);
    }

    #[test]
    fn client_cert_without_key_is_dropped_with_a_warning() {
        let j = r#"{"Servers":{"1":{"Name":"n","Host":"h","Username":"u","SSLCert":"c.pem"}}}"#;
        let p = one(j);
        assert!(p.item.warnings.iter().any(|w| w.contains("birlikte")));
        let c = p.config.unwrap();
        assert!(c.ssl.client_cert.is_none() && c.ssl.client_key.is_none());
    }

    #[test]
    fn duplicates_match_on_host_port_user_and_name() {
        let existing = [ServerConfig::new("prod", "DB1", 5433, "u")];
        let items = preview(OLD, &existing).unwrap();
        assert!(
            items[0].duplicate,
            "host karşılaştırması büyük/küçük harf duyarsız"
        );
        let other = [ServerConfig::new("prod2", "db1", 5433, "u")];
        assert!(!preview(OLD, &other).unwrap()[0].duplicate);
    }

    #[test]
    fn passwords_in_the_file_are_never_read() {
        let j = r#"{"Servers":{"1":{"Name":"n","Host":"h","Username":"u","Password":"secret","password":"secret"}}}"#;
        let p = parse(j, &[]).unwrap().remove(0);
        let config = p.config.expect("geçerli kayıt");
        let dumps = [
            format!("{:?}", p.item),
            format!("{config:?}"),
            serde_json::to_string(&config).unwrap(),
        ];
        for dump in dumps {
            assert!(!dump.contains("secret"), "{dump}");
        }
    }

    const DUPES: &str = r#"{"Servers":{"1":{"Name":"a","Host":"h","Username":"u"},"2":{"Name":"a","Host":"H","Username":"u","Port":5432},"3":{"Name":"a","Host":"h","Username":"u","Port":5433}}}"#;

    #[test]
    fn duplicates_within_the_same_file_are_flagged_after_the_first() {
        let items = preview(DUPES, &[]).unwrap();
        let flags: Vec<_> = items.iter().map(|i| i.duplicate).collect();
        assert_eq!(flags, [false, true, false], "farklı port kopya değildir");
    }

    #[test]
    fn apply_does_not_add_a_duplicate_from_the_same_file_twice() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = crate::registry::Registry::load(dir.path().join("servers.json"))
            .unwrap()
            .registry;
        let n = apply(&mut reg, DUPES, &[0, 1, 2]).unwrap();
        assert_eq!(n, 2);
        let ports: Vec<_> = reg.servers().iter().map(|s| s.port).collect();
        assert_eq!(ports, [5432, 5433]);
        // Yalnızca kopya seçilirse (ilki seçilmemiş) o eklenir.
        let dir2 = tempfile::tempdir().unwrap();
        let mut reg2 = crate::registry::Registry::load(dir2.path().join("servers.json"))
            .unwrap()
            .registry;
        assert_eq!(apply(&mut reg2, DUPES, &[1]).unwrap(), 1);
        assert_eq!(reg2.servers()[0].host, "H");
    }

    #[test]
    fn utf16_files_get_a_clear_message() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{"Servers":{}}"#;
        let le: Vec<u8> = json.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = json.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let cases = [
            ("le-bom.json", [vec![0xFF, 0xFE], le.clone()].concat()),
            ("be-bom.json", [vec![0xFE, 0xFF], be].concat()),
            ("le-nobom.json", le),
        ];
        for (name, bytes) in cases {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            let err = read_file(&path).unwrap_err();
            assert!(
                matches!(&err, Error::Invalid(m) if m.contains("UTF-16")),
                "{name}: {err:?}"
            );
        }
        let latin = dir.path().join("latin1.json");
        std::fs::write(&latin, b"{\"Servers\":{\"1\":{\"Name\":\"\xFC\"}}}").unwrap();
        let err = read_file(&latin).unwrap_err();
        assert!(
            matches!(&err, Error::Invalid(m) if m.contains("UTF-8") && !m.contains("UTF-16")),
            "{err:?}"
        );
    }

    #[test]
    fn apply_imports_only_selected_valid_items_with_fresh_ids() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = crate::registry::Registry::load(dir.path().join("servers.json"))
            .unwrap()
            .registry;
        let j = r#"{"Servers":{"1":{"Name":"a","Host":"h","Username":"u"},"2":{"Name":"b","Host":"h"},"3":{"Name":"c","Host":"h","Username":"u"}}}"#;
        let n = apply(&mut reg, j, &[0, 1]).unwrap();
        assert_eq!(
            n, 1,
            "hatalı kayıt (index 1) atlanır, seçilmeyen (2) alınmaz"
        );
        assert_eq!(reg.servers().len(), 1);
        assert_eq!(reg.servers()[0].name, "a");
        assert_ne!(reg.servers()[0].id, uuid::Uuid::nil());
    }

    #[test]
    fn read_file_enforces_size_limit_and_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.json");
        std::fs::write(&big, vec![b' '; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        assert!(matches!(read_file(&big), Err(Error::Invalid(_))));
        assert!(matches!(
            read_file(&dir.path().join("yok.json")),
            Err(Error::Config(_))
        ));
        let ok = dir.path().join("ok.json");
        std::fs::write(&ok, "{}").unwrap();
        assert_eq!(read_file(&ok).unwrap(), "{}");
    }
}
