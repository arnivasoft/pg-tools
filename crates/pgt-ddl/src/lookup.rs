//! Diyalogların açılır listeleri için salt okunur katalog sorguları (rol, şema, tablespace, eklenti, yöntem, tablo, kolon,
//! kolon tipi, collation; Database/Role diyalogları için üyelik rolleri, kodlama, şablon, yerel ayar, ayar adı;
//! Function/Trigger diyalogları için diller ve trigger fonksiyonları).
//!
//! Kapsam: `LookupRequest::server_wide` istekleri paylaşılan kataloglardan (ya da sabitten) okur ve veritabanı
//! gerektirmez (`database_oid = None` → bakım veritabanı); diğerleri bir veritabanı bağlantısı ister.

use pgt_catalog::version::{PgVersion, VersionedSql};
use pgt_core::Result;
use pgt_core::connection::{ConnectionManager, Target};
use serde::{Deserialize, Serialize};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use ts_rs::TS;
use uuid::Uuid;

use crate::build::database::SERVER_ENCODINGS;
use crate::model::LocaleProvider;

/// İndeks erişim yöntemleri: arayüzde gösterilen sabit sıra (sunucuda var olanlarla kesişir).
const INDEX_METHODS: [&str; 6] = ["btree", "hash", "gin", "gist", "brin", "spgist"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum LookupRequest {
    Roles,
    Schemas,
    Tablespaces,
    AvailableExtensions,
    /// Bir eklentinin kurulabilir sürümleri (`UPDATE TO` seçicisi): value = sürüm, detail = "yüklü" bayrağı + açıklama.
    ExtensionVersions { name: String },
    IndexMethods,
    Tables { schema: Option<String> },
    TableColumns { schema: String, table: String },
    /// Kolon tipi önerileri: value = `format_type` biçiminde kullanılabilir tip ifadesi (arama yolu dışındaysa şema
    /// nitelikli, dizi `x[]`), detail = tür (base/enum/domain/composite/range/multirange/array). `query`: büyük/küçük
    /// harf duyarsız alt dize (önek eşleşmeleri önce); en çok 200 sonuç.
    Types { query: Option<String> },
    /// Kolon collation'ları (pg_catalog + kullanıcı şemaları): value = niteliksiz ad (üretici tek tanımlayıcı yazar;
    /// birden fazla şemada bulunan adlar listelenmez), detail = sağlayıcı · locale [· şema].
    Collations,
    /// Üyelik için tüm roller (önceden tanımlı `pg_*` dahil): detail = "önceden tanımlı" | "superuser" | null.
    GrantableRoles,
    /// Sunucu kodlamaları (`SERVER_ENCODINGS`, sabit; sunucuya gidilmez).
    Encodings,
    /// Şablon olarak kullanılabilecek veritabanları (`datistemplate OR datallowconn`); detail = "şablon" | null.
    TemplateDatabases,
    /// Yerel ayar adları: libc → collcollate (+ C, POSIX), icu → ICU yerel ayarları (PG 15+), builtin (PG 17+) →
    /// yerleşik; sürümde sağlayıcı yoksa boş. detail = yerel ayarın bağlı olduğu kodlamalar | null.
    Locales { provider: LocaleProvider },
    /// `ALTER … SET` ile ayarlanabilen parametreler (`pg_settings.context IN ('user','superuser')`), ad küçük harfle:
    /// detail = "tip · kısa açıklama".
    Settings,
    /// Fonksiyon dilleri (`pg_language`: prosedürel diller + `sql`; `c`/`internal` hariç): value = dil adı, detail =
    /// "güvenilir" | "güvenilmez". Veritabanı kapsamlı.
    Languages,
    /// Trigger fonksiyonları (dönüşü `trigger`, argümansız): value = fonksiyon adı, detail = şema (ayrı alanlar:
    /// `TriggerDef.function_schema/function_name`; ad noktalı olabilir, birleşik metin ayrıştırılmaz). `schema`: yalnız o
    /// şema; `None` → kullanıcı şemaları + `pg_catalog` (kullanıcı şemaları önce). En çok 5000.
    TriggerFunctions { schema: Option<String> },
}

impl LookupRequest {
    /// `database_oid = None` ile (bakım veritabanında) çalışabilen istekler: paylaşılan kataloglar ya da sabit.
    pub fn server_wide(&self) -> bool {
        matches!(
            self,
            LookupRequest::Roles
                | LookupRequest::GrantableRoles
                | LookupRequest::Tablespaces
                | LookupRequest::Encodings
                | LookupRequest::TemplateDatabases
                | LookupRequest::Locales { .. }
                | LookupRequest::Settings
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LookupItem {
    pub value: String,
    pub detail: Option<String>,
}

static ROLES: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_roles.sql"))]);
static SCHEMAS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_schemas.sql"))]);
static TABLESPACES: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_tablespaces.sql"))]);
static EXTENSIONS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_available_extensions.sql"))]);
static EXTENSION_VERSIONS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_extension_versions.sql"))]);
static METHODS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_index_methods.sql"))]);
static TABLES: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_tables.sql"))]);
static COLUMNS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_table_columns.sql"))]);
static TYPES: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_types.sql"))]);
/// ICU locale sütunu: PG 15–16 `colliculocale`, PG 17+ `colllocale`.
static COLLATIONS: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/lookup_collations.pg10.sql")),
    (15, include_str!("sql/lookup_collations.pg15.sql")),
    (17, include_str!("sql/lookup_collations.sql")),
]);
static GRANTABLE_ROLES: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_grantable_roles.sql"))]);
static TEMPLATE_DATABASES: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_template_databases.sql"))]);
/// Veritabanı sağlayıcısı: PG 10–14 yalnızca libc; PG 15 ICU (`colliculocale`); PG 17 builtin (`colllocale`).
static LOCALES: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/lookup_locales.pg10.sql")),
    (15, include_str!("sql/lookup_locales.pg15.sql")),
    (17, include_str!("sql/lookup_locales.sql")),
]);
static SETTINGS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_settings.sql"))]);
static LANGUAGES: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_languages.sql"))]);
/// `prokind` (PG 11+) kullanılmaz: dönüş tipi `trigger` ve argümansızlık aggregate/procedure'ü zaten dışlar.
static TRIGGER_FUNCTIONS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_trigger_functions.sql"))]);

/// Metin parametreleriyle (`$1`, `$2`…; `None` = NULL) çalıştırır.
async fn fetch(mgr: &ConnectionManager, server: Uuid, target: Target, sql: &'static VersionedSql, params: Vec<Option<String>>) -> Result<Vec<Row>> {
    mgr.run(server, target, move |c| {
        let params = params.clone();
        async move {
            let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
            c.client.query(sql.pick(PgVersion(c.version_num())), &refs).await
        }
    })
    .await
}

fn items(rows: &[Row]) -> Vec<LookupItem> {
    rows.iter().map(|r| LookupItem { value: r.get("value"), detail: r.get("detail") }).collect()
}

fn provider_param(p: LocaleProvider) -> &'static str {
    match p {
        LocaleProvider::Libc => "libc",
        LocaleProvider::Icu => "icu",
        LocaleProvider::Builtin => "builtin",
    }
}

fn encodings() -> Vec<LookupItem> {
    SERVER_ENCODINGS.iter().map(|e| LookupItem { value: (*e).to_string(), detail: None }).collect()
}

/// Kapsam kuralı: `database_oid = Some(oid)` → o veritabanı; `None` → `server_wide` istekler bakım veritabanında,
/// diğerleri `Invalid(NEEDS_DATABASE)`.
pub async fn lookup(mgr: &ConnectionManager, server: Uuid, database_oid: impl Into<Option<u32>> + Send, req: LookupRequest) -> Result<Vec<LookupItem>> {
    let target = lookup_target(&req, database_oid.into())?;
    lookup_at(mgr, server, target, req).await
}

/// `lookup`'ın hedef çözümü (saf).
pub(crate) fn lookup_target(req: &LookupRequest, database_oid: Option<u32>) -> Result<Target> {
    match database_oid {
        Some(db) => Ok(Target::Database(db)),
        None if req.server_wide() => Ok(Target::Maintenance),
        None => Err(pgt_core::Error::Invalid(crate::error_codes::NEEDS_DATABASE.to_string())),
    }
}

/// `lookup`'ın hedefli biçimi (`lookup_target` kuralıyla çağrılır).
pub(crate) async fn lookup_at(mgr: &ConnectionManager, server: Uuid, target: Target, req: LookupRequest) -> Result<Vec<LookupItem>> {
    let db = target;
    Ok(match req {
        LookupRequest::Roles => items(&fetch(mgr, server, db, &ROLES, vec![]).await?),
        LookupRequest::Schemas => items(&fetch(mgr, server, db, &SCHEMAS, vec![]).await?),
        LookupRequest::Tablespaces => items(&fetch(mgr, server, db, &TABLESPACES, vec![]).await?),
        LookupRequest::AvailableExtensions => fetch(mgr, server, db, &EXTENSIONS, vec![])
            .await?
            .iter()
            .map(|r| {
                let installed: Option<String> = r.get("installed_version");
                let comment: Option<String> = r.get("comment");
                let default: Option<String> = r.get("default_version");
                LookupItem {
                    value: r.get("value"),
                    detail: Some(format!(
                        "default {} · yüklü {} · {}",
                        default.unwrap_or_default(),
                        installed.unwrap_or_else(|| "—".to_string()),
                        comment.unwrap_or_default()
                    )),
                }
            })
            .collect(),
        LookupRequest::ExtensionVersions { name } => items(&fetch(mgr, server, db, &EXTENSION_VERSIONS, vec![Some(name)]).await?),
        LookupRequest::IndexMethods => {
            let present = items(&fetch(mgr, server, db, &METHODS, vec![]).await?);
            INDEX_METHODS
                .iter()
                .filter(|m| present.iter().any(|p| p.value == **m))
                .map(|m| LookupItem { value: (*m).to_string(), detail: None })
                .collect()
        }
        LookupRequest::Tables { schema } => items(&fetch(mgr, server, db, &TABLES, vec![schema]).await?),
        LookupRequest::TableColumns { schema, table } => items(&fetch(mgr, server, db, &COLUMNS, vec![Some(schema), Some(table)]).await?),
        // Boş/boşluk arama metni = filtre yok.
        LookupRequest::Types { query } => {
            let q = query.map(|q| q.trim().to_string()).filter(|q| !q.is_empty());
            items(&fetch(mgr, server, db, &TYPES, vec![q]).await?)
        }
        LookupRequest::Collations => items(&fetch(mgr, server, db, &COLLATIONS, vec![]).await?),
        LookupRequest::GrantableRoles => items(&fetch(mgr, server, db, &GRANTABLE_ROLES, vec![]).await?),
        // Sabit küme: sunucuya gidilmez.
        LookupRequest::Encodings => encodings(),
        LookupRequest::TemplateDatabases => items(&fetch(mgr, server, db, &TEMPLATE_DATABASES, vec![]).await?),
        LookupRequest::Locales { provider } => {
            items(&fetch(mgr, server, db, &LOCALES, vec![Some(provider_param(provider).to_string())]).await?)
        }
        LookupRequest::Settings => items(&fetch(mgr, server, db, &SETTINGS, vec![]).await?),
        LookupRequest::Languages => items(&fetch(mgr, server, db, &LANGUAGES, vec![]).await?),
        LookupRequest::TriggerFunctions { schema } => items(&fetch(mgr, server, db, &TRIGGER_FUNCTIONS, vec![schema]).await?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_json_shape() {
        let v = serde_json::to_value(LookupRequest::Tables { schema: Some("app".into()) }).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "tables", "schema": "app"}));
        let r: LookupRequest = serde_json::from_value(serde_json::json!({"kind": "available_extensions"})).unwrap();
        assert_eq!(r, LookupRequest::AvailableExtensions);
        let r: LookupRequest = serde_json::from_value(serde_json::json!({"kind": "extension_versions", "name": "citext"})).unwrap();
        assert_eq!(r, LookupRequest::ExtensionVersions { name: "citext".into() });
        let r: LookupRequest = serde_json::from_value(serde_json::json!({"kind": "table_columns", "schema": "a", "table": "t"})).unwrap();
        assert_eq!(r, LookupRequest::TableColumns { schema: "a".into(), table: "t".into() });
        let r: LookupRequest = serde_json::from_value(serde_json::json!({"kind": "types", "query": "int"})).unwrap();
        assert_eq!(r, LookupRequest::Types { query: Some("int".into()) });
        let r: LookupRequest = serde_json::from_value(serde_json::json!({"kind": "types", "query": null})).unwrap();
        assert_eq!(r, LookupRequest::Types { query: None });
        let r: LookupRequest = serde_json::from_value(serde_json::json!({"kind": "collations"})).unwrap();
        assert_eq!(r, LookupRequest::Collations);
        let r: LookupRequest = serde_json::from_value(serde_json::json!({"kind": "locales", "provider": "icu"})).unwrap();
        assert_eq!(r, LookupRequest::Locales { provider: LocaleProvider::Icu });
        for (kind, req) in [
            ("grantable_roles", LookupRequest::GrantableRoles),
            ("encodings", LookupRequest::Encodings),
            ("template_databases", LookupRequest::TemplateDatabases),
            ("settings", LookupRequest::Settings),
        ] {
            assert_eq!(serde_json::to_value(&req).unwrap(), serde_json::json!({ "kind": kind }));
        }
    }

    #[test]
    fn server_wide_requests() {
        let wide = [
            LookupRequest::Roles,
            LookupRequest::GrantableRoles,
            LookupRequest::Tablespaces,
            LookupRequest::Encodings,
            LookupRequest::TemplateDatabases,
            LookupRequest::Locales { provider: LocaleProvider::Libc },
            LookupRequest::Settings,
        ];
        assert!(wide.iter().all(LookupRequest::server_wide));
        let per_db = [
            LookupRequest::Schemas,
            LookupRequest::AvailableExtensions,
            LookupRequest::ExtensionVersions { name: "x".into() },
            LookupRequest::IndexMethods,
            LookupRequest::Tables { schema: None },
            LookupRequest::TableColumns { schema: "a".into(), table: "t".into() },
            LookupRequest::Types { query: None },
            LookupRequest::Collations,
            LookupRequest::Languages,
            LookupRequest::TriggerFunctions { schema: None },
            LookupRequest::TriggerFunctions { schema: Some("app".into()) },
        ];
        assert!(!per_db.iter().any(LookupRequest::server_wide));
    }

    #[test]
    fn lookup_target_follows_scope() {
        assert_eq!(lookup_target(&LookupRequest::Schemas, Some(5)).unwrap(), Target::Database(5));
        assert_eq!(lookup_target(&LookupRequest::Roles, Some(5)).unwrap(), Target::Database(5));
        assert_eq!(lookup_target(&LookupRequest::GrantableRoles, None).unwrap(), Target::Maintenance);
        assert_eq!(lookup_target(&LookupRequest::Locales { provider: LocaleProvider::Icu }, None).unwrap(), Target::Maintenance);
        let e = lookup_target(&LookupRequest::Schemas, None).unwrap_err();
        assert!(matches!(e, pgt_core::Error::Invalid(m) if m == crate::error_codes::NEEDS_DATABASE));
        // Diller ve trigger fonksiyonları veritabanı kapsamlıdır.
        for req in [LookupRequest::Languages, LookupRequest::TriggerFunctions { schema: None }] {
            assert_eq!(lookup_target(&req, Some(7)).unwrap(), Target::Database(7));
            let e = lookup_target(&req, None).unwrap_err();
            assert!(matches!(e, pgt_core::Error::Invalid(m) if m == crate::error_codes::NEEDS_DATABASE), "{req:?}");
        }
    }

    #[test]
    fn language_and_trigger_function_requests_json_shape() {
        assert_eq!(serde_json::to_value(LookupRequest::Languages).unwrap(), serde_json::json!({"kind": "languages"}));
        let v = serde_json::to_value(LookupRequest::TriggerFunctions { schema: Some("app".into()) }).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "trigger_functions", "schema": "app"}));
        let r: LookupRequest = serde_json::from_value(serde_json::json!({"kind": "trigger_functions", "schema": null})).unwrap();
        assert_eq!(r, LookupRequest::TriggerFunctions { schema: None });
    }

    #[test]
    fn new_lookup_sql_is_static_and_parameterized() {
        // Tek ifade, `$1` dışında yerleştirme yok; diller c/internal'ı dışlar, trigger fonksiyonları argümansız.
        for sql in [LANGUAGES.pick(PgVersion(100000)), TRIGGER_FUNCTIONS.pick(PgVersion(180000))] {
            assert!(crate::build::scan_semicolons(sql).is_clean(), "{sql}");
            assert!(!sql.contains("format("), "{sql}");
        }
        let langs = LANGUAGES.pick(PgVersion(160000));
        assert!(langs.contains("'c'") && langs.contains("'internal'") && !langs.contains("$1"));
        let trig = TRIGGER_FUNCTIONS.pick(PgVersion(160000));
        assert!(trig.contains("$1::text") && trig.contains("'trigger'::regtype") && trig.contains("pronargs = 0"));
    }

    #[test]
    fn encodings_are_server_encodings_without_round_trip() {
        let v: Vec<String> = encodings().into_iter().map(|i| i.value).collect();
        assert!(v.iter().any(|e| e == "UTF8") && !v.iter().any(|e| e == "SJIS"));
        assert_eq!(v.len(), SERVER_ENCODINGS.len());
    }
}
