//! DDL yöneticisi: önizleme, kapılı uygulama (kilit/geçerlilik/hash/onay), iptal, yükleme, varsayılanlar ve açılır listeler.
//!
//! Kapı sırası (`apply`; yalnızca (4) havuzdaki katalog bağlantısında tek gidiş-dönüş yapar, hiçbiri uygulama
//! bağlantısı açmaz): (1) kayıtsız sunucu, (2) `read_only_lock`, (2b) salt okunur nesne (`editable = false`
//! Function/Procedure/Trigger/Type/Domain'de alter/drop → kodsuz `invalid`, genel gerekçeyle), (2a) hedef çözümü (`target_for`: Database/Role →
//! bakım veritabanı; veritabanı kapsamlı türde `database_oid` yok → `Invalid`), (3) plan üretimi (geçersiz tanım /
//! boş plan), (4) nesne denetimi (yalnızca alter/drop: istemcinin `old`/`def`'i, `object`'in sunucudaki güncel
//! tanımına eşit olmalı; fark → `plan_changed`; yükleme hataları — yok/sistem nesnesi/identity dizisi — olduğu gibi
//! geçer), (4a) bakım veritabanı koruması (kayıttaki ve oturumun bağlı olduğu ad: yeniden adlandırma/taşıma/silme,
//! ALLOW_CONNECTIONS false ya da CONNECTION LIMIT 0 → `MAINTENANCE_DATABASE`; şablon olarak kopyalama →
//! `MAINTENANCE_TEMPLATE`), (5) hash
//! uyuşmazlığı, (6) yıkıcı plan onayı, (7) üretim sunucusunda veri kaybı onayı, (8) `lock_timeout` sınırı,
//! (9) parola kapısı (gizlenmiş ifade ↔ `ApplyOptions.password`), (10) somutlaştırma (gizlenmiş ifade yalnızca
//! çalıştırma listesinde istemci tarafı SCRAM doğrulayıcısıyla kurulur; `PlanView`'e, özete ve günlüğe girmez).
//! Plan istemciden alınmaz; her seferinde tanımdan yeniden üretilir. `preview` nesne denetimi yapmaz.
//!
//! Yetki istekleri (Plan 20): (2c) istek biçimi (`!acl_supported` → Karar 1 iletisi; `object.kind ≠ old.kind` ya da
//! kapsam rolü ≠ `old.role` → `INVALID_DEF`) salt okunur kapısından önce; (2b) `editable = false` ACL/varsayılan ACL de
//! reddedilir; (4) `old` ilgili yükleyicinin (`load_acl`/`load_default_acl`) çıktısına birebir eşit olmalı; (4a)
//! uygulanmaz. Başarılı uygulamadan sonra yetkiler yeniden yüklenir; `grants` istenenden farklıysa `ACL_WARNING`.
//!
//! Bakım istekleri (Plan 24): (2c) nesne türü bakım türü olmalı ve `target.kind`'a uymalı; (2b) `target.editable =
//! false` reddedilir; (2a) istisna: `target_for`'dan önce, Database türü dahil her zaman `Target::Database` (bkz.
//! `apply_target`); (4) hedef `load_maintenance_target` ile yeniden yüklenip eşitlik aranır; (4a) uygulanmaz (bakım
//! veritabanını kapatmaz); `created` ve uygulama sonrası doğrulama yoktur. İlerleme (`maintenance_progress`) yalnız
//! kendi uygulama pid'imizin `pg_stat_progress_*` satırını katalog havuzundan okur (pid yalnız bakım uygulamalarında
//! okunur).
//!
//! Günlük yalnızca işlem/nesne türü, ifade sayısı, süre ve sonuç kodunu içerir; SQL, ad, değer ve parola asla.

use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use pgt_core::connection::{ConnectionManager, DedicatedConn, MAX_STATEMENT_TIMEOUT_MS, Target};
use pgt_core::registry::Registry;
use pgt_core::{Error, ErrorPayload, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::acl::{AclDef, AclGrant, DefaultAclDef, DefaultAclScope, acl_supported, default_privileges_for, privileges_for, sort_grants};
use crate::apply::{self, CancelMap, ExecSql, ExecStatement, RunResult};
use crate::build::{self, BuildCtx, role};
use crate::error_codes::{
    CONFIRMATION_REQUIRED, INVALID_DEF, LOCKED_MESSAGE, MAINTENANCE_DATABASE, MAINTENANCE_TEMPLATE, NO_CHANGES,
    PASSWORD_NOT_USED,
    PLAN_CHANGED, PRODUCTION_CONFIRMATION_REQUIRED, ROLE_PASSWORD_MISSING,
};
use crate::load::{self, ObjectRef};
use crate::lookup::{self, LookupItem, LookupRequest};
use crate::maintenance::{MaintenanceOp, MaintenanceProgress, MaintenanceTarget};
use crate::model::*;
use crate::plan::{PlanView, Risk, Statement};
use crate::secret::Secret;
use crate::target::CreatePreset;

/// Arayüzün istediği değişiklik; plan arka uçta bu istekten üretilir.
// Tek seferlik IPC yükü (saklanmaz); varyant boyut farkı önemsiz, `Box` JSON'u değiştirmeden yalnızca kullanımı zorlaştırır.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum PlanRequest {
    Create { def: ObjectDef, concurrently: bool },
    /// `object`: değiştirilen nesne (`ddl_load`'a verilen kimlik). `apply` bu nesneyi yeniden yükler ve `old`'un
    /// yüklenen tanıma eşit olduğunu doğrular (istemciye güvenilmez).
    Alter { object: ObjectRef, old: ObjectDef, new: ObjectDef },
    /// `object`: silinen nesne; `def` yüklenen tanıma eşit olmalıdır (bkz. `Alter`).
    /// `force` yalnızca Database: `DROP DATABASE … WITH (FORCE)` (PG 13+).
    Drop {
        object: ObjectRef,
        def: ObjectDef,
        cascade: bool,
        #[serde(default)]
        force: bool,
    },
    /// Nesne yetkileri (GRANT/REVOKE; Plan 20). `old` yüklenen etkin ACL'ye eşit olmalı (bkz. `Alter`).
    Privileges { object: ObjectRef, old: AclDef, new: AclDef, cascade: bool },
    /// Varsayılan yetkiler (ALTER DEFAULT PRIVILEGES; Plan 20). Veritabanı kapsamlıdır.
    DefaultPrivileges { scope: DefaultAclScope, old: DefaultAclDef, new: DefaultAclDef },
    /// Bakım işlemi (VACUUM / ANALYZE / REINDEX / CLUSTER; Plan 24). `target` `ddl_load_maintenance_target`'ın
    /// çıktısıdır ve uygulamada yeniden yüklenip eşitlik aranır (bkz. `Alter`). Her zaman bağlı veritabanında
    /// (`Target::Database`) çalışır, Database türünde de. İşlem JSON'da `operation` alanındadır: `op` istek etiketidir.
    Maintenance {
        object: ObjectRef,
        target: MaintenanceTarget,
        #[serde(rename = "operation")]
        op: MaintenanceOp,
    },
}

impl PlanRequest {
    fn op(&self) -> &'static str {
        match self {
            PlanRequest::Create { .. } => "create",
            PlanRequest::Alter { .. } => "alter",
            PlanRequest::Drop { .. } => "drop",
            PlanRequest::Privileges { .. } => "privileges",
            PlanRequest::DefaultPrivileges { .. } => "default_privileges",
            PlanRequest::Maintenance { .. } => "maintenance",
        }
    }

    fn kind(&self) -> ObjectKind {
        match self {
            PlanRequest::Create { def, .. } | PlanRequest::Drop { def, .. } => def.kind(),
            PlanRequest::Alter { new, .. } => new.kind(),
            PlanRequest::Privileges { object, .. } | PlanRequest::Maintenance { object, .. } => object.kind,
            // Veritabanı kapsamlı vekil: hedef çözümü `Schema` ile aynı kuraldır (veritabanı OID'i gerekir).
            PlanRequest::DefaultPrivileges { .. } => ObjectKind::Schema,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApplyOptions {
    /// İptal için istemci üretir.
    pub apply_id: Uuid,
    /// Kilit bekleme sınırı (ms; `0` = sınırsız).
    #[ts(type = "number")]
    pub lock_timeout_ms: u32,
    /// Önizlemedeki `PlanView.hash`.
    pub expected_hash: String,
    /// Yıkıcı/veri kaybı olan plan için kullanıcı onayı.
    pub confirmed: bool,
    /// Üretim sunucusunda veri kaybı olan plan için ek (ad yazarak) onay.
    pub confirmed_production: bool,
    /// Rol parolası (`PasswordChange::Set`); yalnızca uygulamada gelir, `Debug`/`Serialize` değeri yazmaz.
    #[serde(default)]
    #[ts(type = "string | null")]
    pub password: Option<Secret>,
}

/// Uygulama sırasında sunucunun gönderdiği NOTICE/INFO/WARNING iletisi (ör. `DROP … CASCADE`, `VACUUM VERBOSE`).
/// Alanlar en çok 4096 bayttır (UTF-8 sınırında kesilip `…` eklenir); metin günlüğe/diske yazılmaz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerMessage {
    pub severity: String,
    /// SQLSTATE; sınır aşımını bildiren yapay iletide yok.
    pub code: Option<String>,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
}

/// Uygulama sonucu. Günlüğe yazmayın (`Debug` dahil): sunucu ileti metni (`messages`, `error`) içerir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "outcome", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ApplyOutcome {
    Applied {
        statements: u32,
        #[ts(type = "number")]
        elapsed_ms: u64,
        /// Create (ve kimliği değişen alter/yeniden kurma) sonrası yeni nesne; en iyi çaba, bulunamazsa `None`.
        created: Option<ObjectRef>,
        /// Uygulama sonrası denetim uyarıları (ör. PG 16+ etkisiz REVOKE: yeniden yüklenen üyelikler beklenenden
        /// farklı). Yoksa alan yazılmaz.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        warnings: Option<Vec<String>>,
        /// Sunucu iletileri (en çok 500; aşılırsa son ileti kalan sayıyı bildirir). Yoksa alan yazılmaz.
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        #[ts(as = "Option<Vec<ServerMessage>>", optional)]
        messages: Vec<ServerMessage>,
    },
    /// `failed_index`: başarısız ifadenin 0 tabanlı sırası; `committed`: kalıcılaşmış (transaction dışı ya da önceki blok)
    /// ifade sayısı; `sqlstate`: PG hata kodu (ör. `57014` iptal, `55P03` kilit zaman aşımı; bağlantı hatasında yok).
    ///
    /// `position`: PG'nin hata konumu (1 tabanlı karakter sapması), başarısız ifadenin metnine göredir; gizlenmiş
    /// (parola) ifadede her zaman `None`.
    Failed {
        failed_index: u32,
        committed: u32,
        sqlstate: Option<String>,
        position: Option<u32>,
        error: ErrorPayload,
        /// Kısmi uygulama (`committed > 0`; ör. CREATE DATABASE başarılı, sonraki ayar/yorum başarısız): oluşturulan ya
        /// da yeniden adlandırılan nesne. En iyi çaba; yoksa alan yazılmaz.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        created: Option<ObjectRef>,
        /// Hataya kadar gelen sunucu iletileri (`Applied.messages` ile aynı sınırlar). Yoksa alan yazılmaz.
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        #[ts(as = "Option<Vec<ServerMessage>>", optional)]
        messages: Vec<ServerMessage>,
    },
}

/// "Yeni …" diyaloğunun açıldığı koleksiyon düğümü bağlamı.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateContext {
    #[ts(type = "number | null")]
    pub schema_oid: Option<u32>,
    #[ts(type = "number | null")]
    pub table_oid: Option<u32>,
}

/// İptal isteğinin yutulmasına karşı yeniden gönderme: sayı ve aralık.
pub(crate) const CANCEL_RESENDS: u32 = 3;
pub(crate) const CANCEL_RESEND_AFTER: Duration = Duration::from_millis(200);

const FIND_SCHEMA_SQL: &str = "SELECT oid FROM pg_namespace WHERE nspname = $1";
const FIND_EXTENSION_SQL: &str = "SELECT oid FROM pg_extension WHERE extname = $1";
const FIND_SEQUENCE_SQL: &str = "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = 'S'";
const FIND_INDEX_SQL: &str = "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('i', 'I')";
const FIND_TABLE_SQL: &str = "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r', 'p')";
const FIND_VIEW_SQL: &str = "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = 'v'";
const FIND_MATVIEW_SQL: &str = "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = 'm'";
const FIND_DATABASE_SQL: &str = "SELECT oid FROM pg_database WHERE datname = $1";
const FIND_ROLE_SQL: &str = "SELECT oid FROM pg_roles WHERE rolname = $1";
/// Fonksiyon: şema + ad + giriş tipleri (`proargtypes`, OUT hariç). `to_regprocedure` kullanılmaz: tipler tek tek
/// `regtype` ile çözülür ve oid listesi (`oidvector` metni: boşlukla ayrık) karşılaştırılır; aşırı yüklemede doğru
/// nesne. `{kind}` sürüme göre tür süzgeci (PG 11+ `prokind`). `::regtype` çözümü havuzdaki katalog bağlantısının
/// `search_path`'ine dayanır (uygulama bağlantısıyla aynı varsayılan); niteliksiz bir tip orada farklı çözülür ya da
/// bulunamazsa sorgu boş/hatalı döner ve `created = None` olur (en iyi çaba; yanlış nesne değil, oid'ler eşleşmez).
const FIND_FUNCTION_SQL: &str = "WITH a AS (SELECT coalesce(string_agg(t::regtype::oid::text, ' ' ORDER BY i), '') AS sig \
     FROM unnest($3::text[]) WITH ORDINALITY AS u(t, i)) \
     SELECT p.oid FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace, a \
     WHERE n.nspname = $1 AND p.proname = $2 AND p.proargtypes::text = a.sig AND {kind}";
/// Procedure (PG 11+): TÜM argümanlar + modlar (PG 14+ OUT argümanları çağrı imzasına girer; `p(int, text)` ile
/// `p(int, OUT text)` birlikte var olabilir): tüm argüman tipleri (`proallargtypes`, yoksa `proargtypes`) ve
/// `proargmodes` metni (hepsi IN ise NULL → `''`).
const FIND_PROCEDURE_SQL: &str = "WITH a AS (SELECT coalesce(string_agg(t::regtype::oid::text, ' ' ORDER BY i), '') AS sig \
     FROM unnest($3::text[]) WITH ORDINALITY AS u(t, i)) \
     SELECT p.oid FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace, a \
     WHERE n.nspname = $1 AND p.proname = $2 AND p.prokind = 'p' AND coalesce(p.proargmodes::text, '') = $4 \
     AND CASE WHEN p.proallargtypes IS NULL THEN p.proargtypes::text ELSE array_to_string(p.proallargtypes, ' ') END = a.sig";
/// Trigger: tablo (şema + ad → oid) + trigger adı.
const FIND_TRIGGER_SQL: &str = "SELECT t.oid FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
     JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = $2 AND t.tgname = $3";
/// Type/Domain: şema + ad + `typtype` (`$3`: e/c/r/d).
const FIND_TYPE_SQL: &str = "SELECT t.oid FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace \
     WHERE n.nspname = $1 AND t.typname = $2 AND t.typtype::text = $3";

/// `find_created` sorgu parametresi.
#[derive(Clone)]
enum FindParam {
    Text(String),
    List(Vec<String>),
}

impl FindParam {
    fn as_sql(&self) -> &(dyn tokio_postgres::types::ToSql + Sync) {
        match self {
            FindParam::Text(s) => s,
            FindParam::List(v) => v,
        }
    }
}

/// `FIND_FUNCTION_SQL`/`FIND_PROCEDURE_SQL` parametreleri: tip listesi ve (procedure) `proargmodes` metni.
fn routine_find_args(kind: ObjectKind, d: &FunctionDef) -> (Vec<String>, Option<String>) {
    if kind != ObjectKind::Procedure {
        return (d.args.iter().filter(|a| a.mode != ArgMode::Out).map(|a| a.type_name.clone()).collect(), None);
    }
    let types = d.args.iter().map(|a| a.type_name.clone()).collect();
    let modes = if d.args.iter().all(|a| a.mode == ArgMode::In) {
        String::new()
    } else {
        let m: Vec<&str> = d
            .args
            .iter()
            .map(|a| match a.mode {
                ArgMode::In => "i",
                ArgMode::Out => "o",
                ArgMode::Inout => "b",
                ArgMode::Variadic => "v",
            })
            .collect();
        format!("{{{}}}", m.join(","))
    };
    (types, Some(modes))
}

// ---- hata ipuçları (yalnızca PG ipucu yoksa; SQL/ad/değer içermez) ----

/// Karar 1: `CREATE OR REPLACE VIEW` kolon uyumsuzluğu (42P16 / 42804).
pub const VIEW_COLUMNS_HINT: &str = "Kolonlar uyumsuz: 'Yeniden oluştur (DROP + CREATE)' seçeneğini işaretleyin.";
/// `REFRESH MATERIALIZED VIEW CONCURRENTLY` (55000).
pub const REFRESH_CONCURRENTLY_HINT: &str = "CONCURRENTLY için materialized view dolu olmalı ve tüm satırları kapsayan, \
     koşulsuz bir unique indeksi bulunmalı; normal REFRESH seçin ya da indeks oluşturun.";
/// Karar 8: veritabanına bağlı başka oturumlar (55006; RENAME / SET TABLESPACE / DROP).
pub const DATABASE_IN_USE_HINT: &str = "Veritabanına bağlı oturumlar var: Query Tool sekmelerini kapatın ya da Dashboard'dan \
     oturumları sonlandırın; PG 13+'ta silerken WITH (FORCE) kullanılabilir. Uygulamanın kendi bağlantısı (ör. Gezgin \
     yenilemesi) arada yeniden bağlanmış olabilir; birkaç saniye sonra yeniden denemek işe yarayabilir.";
/// `DROP DATABASE … WITH (FORCE)` sunucu sınırları (55006).
pub const FORCE_LIMITS_HINT: &str = "WITH (FORCE) oturumları sonlandırır, ancak hazırlanmış transaction'lar (pg_prepared_xacts), \
     mantıksal replikasyon yuvaları ya da etkin abonelikler varken veritabanı silinemez; önce bunları kaldırın.";
/// `CREATE DATABASE` şablonu kullanımda (55006).
pub const TEMPLATE_IN_USE_HINT: &str = "Şablon veritabanına bağlı oturumlar var; şablona bağlı oturumları kapatın.";
/// Karar 9: yetki yetersiz (42501).
pub const PRIVILEGE_HINT: &str = "Bağlı rolün yetkisi yetmiyor; SUPERUSER/REPLICATION/BYPASSRLS yalnızca süper kullanıcı \
     verebilir, sahip değiştirmek için yeni sahibin üyesi olmak ve nesnenin sahibi olmak gerekir.";
/// Karar 9: `DROP ROLE` bağımlılıkları (2BP01).
pub const ROLE_DEPENDENTS_HINT: &str =
    "Rolün sahip olduğu nesneler/yetkiler var: önce REASSIGN OWNED / DROP OWNED (her veritabanında) çalıştırın.";
/// Karar 2: görünüm/MV bağımlıları (2BP01).
pub const VIEW_DEPENDENTS_HINT: &str = "Bağımlı nesneler var; silmek için CASCADE seçin ya da bağımlıları kaldırın.";
/// Düzenlemede yeniden kurma (view `recreate`, MatView tanım değişikliği) bağımlılar yüzünden düşerse (2BP01): bu akışta
/// CASCADE seçeneği yoktur (inceleme M-3).
pub const REBUILD_DEPENDENTS_HINT: &str = "Bağımlı nesneler (ör. bu görünümü kullanan görünümler) yeniden kurmayı \
     engelliyor: önce bağımlıları kaldırın ya da sorgu değişikliğini kolonları koruyacak biçimde yapın.";
/// Plan 18: fonksiyon/procedure yeniden kurmasındaki `DROP` bağımlılar yüzünden düşerse (2BP01): bu akışta CASCADE
/// yoktur; PG'nin "CASCADE kullanın" ipucunun yerine geçer.
pub const ROUTINE_REBUILD_DEPENDENTS_HINT: &str = "Bağımlı nesneler (ör. bu fonksiyonu kullanan görünümler, trigger'lar ya \
     da varsayılanlar) yeniden oluşturmayı engelliyor: önce bağımlıları kaldırın ya da değişikliği imzayı koruyacak biçimde yapın.";
/// Plan 18 Karar 2: `CREATE OR REPLACE` sınırı (42P13; PG'nin SQL öneren ipucunun yerine geçer).
pub const OR_REPLACE_HINT: &str = "OR REPLACE bu değişikliği yapamaz; 'Yeniden oluştur' seçin.";
/// Plan 18: Function ↔ Procedure (42809).
pub const ROUTINE_KIND_HINT: &str = "Function ↔ Procedure dönüşümü yeniden oluşturma ister: nesneyi silip diğer türde yeniden oluşturun.";
/// Plan 18 Karar 7: typed table kullanan composite'te öznitelik değişimi (2BP01; PG'nin CASCADE ipucunun yerine geçer).
pub const TYPED_TABLE_HINT: &str = "Bu tip tablolarda kullanılıyor; CASCADE gerekir (desteklenmiyor).";
/// Plan 18 Karar 7: tablo kolonunda kullanılan composite'in öznitelik tipi değiştirilemez (0A000).
pub const TYPE_COLUMN_RETYPE_HINT: &str =
    "Bu tip tablo kolonlarında kullanılıyor; PostgreSQL kullanımdaki bir composite tipin öznitelik tipini değiştiremez.";
/// Plan 18: `LEAKPROOF` (42501).
pub const LEAKPROOF_HINT: &str = "LEAKPROOF yalnızca süper kullanıcı verebilir.";
/// Plan 18 salt okunur kapısı: kodsuz `invalid` iletisinin öneki; ardından genel gerekçe (`read_only_reason`).
pub const READ_ONLY_PREFIX: &str = "Bu nesne buradan düzenlenemez: ";
/// Kısmi uygulama (transaction dışı ifadeler): ilk ifadeler kalıcı.
pub const PARTIAL_APPLY_HINT: &str =
    "Önceki ifadeler uygulandı (transaction dışı); nesneyi yenileyip kalan değişiklikleri yeniden deneyin.";
/// Başarısız gizlenmiş ifadenin sunucu iletisi doğrulayıcı içeriyorsa yerine geçen sabit ileti (savunma).
const REDACTED_FAILURE_MESSAGE: &str = "Parola ifadesi uygulanamadı.";
/// PG 16+: GRANTED BY'sız REVOKE başka grantor'ın üyeliğini kaldırmaz (WARNING ile etkisiz); grant başka grantor ile
/// kaydedilebilir. Yeniden yüklenen üyelikler istenenden farklıysa.
pub const MEMBERSHIP_WARNING: &str = "Üyelikler uygulama sonrasında istenenden farklı: bazı GRANT/REVOKE işlemleri başka bir \
     grantor'a ait üyelikleri etkilemedi (PG 16+). Rolü yeniden yükleyip kontrol edin.";
/// Plan 20 Karar 14: yeniden yüklenen yetkiler (`grants`) istenenden farklı.
pub const ACL_WARNING: &str = "Yetkiler uygulama sonrasında istenenden farklı: bağlı rol nesnenin sahibi değilse yetkiler onun \
     adıyla kaydedilir ya da bazı GRANT/REVOKE işlemleri etkisiz kalır (PG uyarısı). Yetkileri yeniden yükleyip kontrol edin.";
/// Plan 20 Karar 15: bağımlı yetkiler (2BP01; PG'nin "Use CASCADE" ipucunun yerine arayüzdeki seçenek).
pub const ACL_DEPENDENTS_HINT: &str =
    "Alıcı bu yetkiyi başkalarına devretmiş; 'Bağımlı yetkilerle birlikte geri al (CASCADE)' seçin.";
/// Plan 20 Karar 15: nesne yetkisi verme/geri alma yetkisizliği (42501).
pub const ACL_PRIVILEGE_HINT: &str = "Yetki vermek/geri almak için nesnenin sahibi, sahip rolün üyesi ya da ilgili yetkide \
     GRANT OPTION sahibi olmak gerekir.";
/// Plan 20 Karar 15: `ALTER DEFAULT PRIVILEGES FOR ROLE` yetkisizliği (42501).
pub const DEFAULT_ACL_PRIVILEGE_HINT: &str = "FOR ROLE için o rolün üyesi (ya da süper kullanıcı) olmak gerekir.";
/// Plan 20 Karar 15: alıcı rol yok (42704).
pub const ACL_ROLE_MISSING_HINT: &str = "Alıcı rol bulunamadı; rol listesini yenileyin.";
/// Plan 24: bakım işleminde kilit zaman aşımı (55P03; genel lock_timeout ipucunun yerine geçer).
pub const MAINTENANCE_LOCK_HINT: &str = "Nesne başka bir oturumca kilitli; daha sonra deneyin ya da lock timeout'u artırın.";
/// Plan 24: bakım yetkisizliği (42501).
pub const MAINTENANCE_PRIVILEGE_HINT: &str = "Bakım işlemi için nesnenin sahibi (ya da PG 17+'da MAINTAIN yetkisi) gerekir.";
/// Plan 24: seçenek bu nesnede desteklenmiyor (0A000; ör. REINDEX CONCURRENTLY sistem kataloğunda).
pub const MAINTENANCE_UNSUPPORTED_HINT: &str = "Bu nesnede bu seçenek desteklenmiyor.";
/// Plan 24: transaction bloğu içinde çalıştırılamaz (25001). Bakım ifadeleri transaction dışıdır; beklenmez, savunma.
pub const MAINTENANCE_TX_HINT: &str = "Bu bakım işlemi transaction bloğu içinde çalıştırılamaz; yeniden deneyin.";

/// Çağıran gelecek düşerse sunucudaki ifadeyi iptal eder (`armed` yalnızca çalıştırıcı bitince kapanır).
pub(crate) struct CancelOnDrop {
    pub(crate) handle: pgt_core::connection::CancelHandle,
    pub(crate) rt: tokio::runtime::Handle,
    pub(crate) armed: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            let h = self.handle.clone();
            self.rt.spawn(async move {
                let _ = h.cancel().await;
            });
        }
    }
}

/// Alter sonrası nesnenin kimliği (şema/ad) değiştiyse ya da indeks yeniden kuruluyorsa (oid değişir) yeniden aranır.
/// Tablo yeniden adlandırma/şema taşıma oid'i korur; yeni adla arama aynı oid'i döndürür (arayüz yeni adla yeniler).
fn identity_changed(old: &ObjectDef, new: &ObjectDef) -> bool {
    match (old, new) {
        (ObjectDef::Schema(a), ObjectDef::Schema(b)) => a.name != b.name,
        (ObjectDef::Extension(a), ObjectDef::Extension(b)) => a.name != b.name,
        (ObjectDef::Sequence(a), ObjectDef::Sequence(b)) => (&a.schema, &a.name) != (&b.schema, &b.name),
        (ObjectDef::Index(_), ObjectDef::Index(_)) => true,
        (ObjectDef::Table(a), ObjectDef::Table(b)) => (&a.schema, &a.name) != (&b.schema, &b.name),
        // Görünüm yeniden kurulursa (recreate + sorgu değişimi) oid değişir; MV tanım değişikliği her zaman yeniden kurar.
        (ObjectDef::View(a), ObjectDef::View(b)) => (&a.schema, &a.name) != (&b.schema, &b.name) || (b.recreate && a.query != b.query),
        (ObjectDef::MaterializedView(a), ObjectDef::MaterializedView(b)) => {
            (&a.schema, &a.name) != (&b.schema, &b.name) || a.query != b.query
        }
        (ObjectDef::Database(a), ObjectDef::Database(b)) => a.name != b.name,
        (ObjectDef::Role(a), ObjectDef::Role(b)) => a.name != b.name,
        // İmza değişimi yalnızca yeniden kurmayla olur (DROP + CREATE: yeni oid); gereksiz `recreate` üreticide de yok
        // sayılır. Ad/şema değişimi oid'i korur, yeni adla arama aynı oid'i döndürür.
        (ObjectDef::Function(a), ObjectDef::Function(b)) => routine_identity_changed(ObjectKind::Function, a, b),
        (ObjectDef::Procedure(a), ObjectDef::Procedure(b)) => routine_identity_changed(ObjectKind::Procedure, a, b),
        // Yeniden kurma PG < 14'te ve constraint trigger'da DROP + CREATE'tir (yeni oid); OR REPLACE'te arama aynı oid'i bulur.
        (ObjectDef::Trigger(a), ObjectDef::Trigger(b)) => a.name != b.name || build::trigger::structure_changed(a, b),
        (ObjectDef::Type(a), ObjectDef::Type(b)) => (&a.schema, &a.name) != (&b.schema, &b.name),
        (ObjectDef::Domain(a), ObjectDef::Domain(b)) => (&a.schema, &a.name) != (&b.schema, &b.name),
        _ => false,
    }
}

fn routine_identity_changed(kind: ObjectKind, a: &FunctionDef, b: &FunctionDef) -> bool {
    (&a.schema, &a.name) != (&b.schema, &b.name) || (b.recreate && !build::routine::needs_recreate(kind, a, b).is_empty())
}

/// (2c) Yetki isteğinin biçimi (Plan 20 Karar 1, 13): bağlantıdan önce. Yetkisi olmayan tür → Karar 1 iletisi;
/// `object.kind ≠ old.kind` ya da (varsayılan yetkilerde) verilen kapsam rolü ≠ `old.role` → `INVALID_DEF`.
fn request_shape_refusal(req: &PlanRequest) -> Option<&'static str> {
    match req {
        PlanRequest::Privileges { object, .. } if !acl_supported(object.kind) => Some(crate::load_acl::UNSUPPORTED_KIND_MSG),
        PlanRequest::Privileges { object, old, .. } if object.kind != old.kind => Some(INVALID_DEF),
        PlanRequest::DefaultPrivileges { scope, old, .. } if scope.role.as_ref().is_some_and(|r| *r != old.role) => Some(INVALID_DEF),
        // Plan 24: bakımı olmayan tür → yükleyicinin iletisi; nesne türü ↔ hedef türü uyuşmazlığı → `INVALID_DEF`.
        PlanRequest::Maintenance { object, target, .. } => match crate::load_maintenance::target_kind(object.kind) {
            None => Some(crate::load_maintenance::NO_MAINTENANCE_MSG),
            Some(k) if k != target.kind => Some(INVALID_DEF),
            Some(_) => None,
        },
        _ => None,
    }
}

/// Salt okunur kapısı (Plan 18 Karar 4): `editable = false` nesnede alter/drop, bağlantı açılmadan kodsuz `invalid`.
/// Gerekçe istemcinin gönderdiği (yüklenmiş) tanımın ilk `unsupported` girdisinden genel metindir; ad/değer içermez.
/// İstemci `editable`'ı değiştirse bile nesne denetimi (4) yüklenen tanımla eşitliği ister, üretici de reddeder.
/// Plan 20: yetki isteklerinde `old` (ACL / varsayılan ACL) aynı kurala tabidir; varsayılan yetkilerin LARGE OBJECTS
/// bilgi notu gerekçe sayılmaz (tek başına `editable`'ı kapatmaz).
fn read_only_refusal(req: &PlanRequest) -> Option<String> {
    let refuse = |editable: bool, first: Option<&String>| {
        (!editable).then(|| format!("{READ_ONLY_PREFIX}{}", read_only_reason(first.map(String::as_str))))
    };
    let def = match req {
        PlanRequest::Create { .. } => return None,
        PlanRequest::Privileges { old, .. } => return refuse(old.editable, old.unsupported.first()),
        PlanRequest::DefaultPrivileges { old, .. } => {
            let first = old.unsupported.iter().find(|u| *u != crate::load_acl::LARGE_OBJECTS_REASON);
            return refuse(old.editable, first);
        }
        // Plan 24: salt okunur hedef (bölümlenmiş indeks, geçici şema, geçersiz UTF-8 ad).
        PlanRequest::Maintenance { target, .. } => return refuse(target.editable, target.unsupported.first()),
        PlanRequest::Alter { old, .. } => old,
        PlanRequest::Drop { def, .. } => def,
    };
    let (editable, unsupported) = match def {
        ObjectDef::Function(d) | ObjectDef::Procedure(d) => (d.editable, &d.unsupported),
        ObjectDef::Trigger(d) => (d.editable, &d.unsupported),
        ObjectDef::Type(d) => (d.editable, &d.unsupported),
        ObjectDef::Domain(d) => (d.editable, &d.unsupported),
        _ => return None,
    };
    refuse(editable, unsupported.first())
}

/// Yükleyicinin `unsupported` girdisini (bazıları ad/değer taşır: `EXTENSION <ad>`, `tip <tip>`, `CHECK <ad> …`) ad ve
/// değer içermeyen sabit gerekçeye indirir. Bilinmeyen biçim → genel gerekçe.
fn read_only_reason(entry: Option<&str>) -> &'static str {
    const GENERIC: &str = "desteklenmeyen özellik";
    let Some(e) = entry else { return GENERIC };
    // (önek, gerekçe): ilk eşleşen kazanır; sabit girdiler kendi metniyle.
    const PREFIXES: [(&str, &str); 41] = [
        ("LANGUAGE ", "desteklenmeyen dil (C/internal)"),
        ("BEGIN ATOMIC", "BEGIN ATOMIC (SQL standart gövde)"),
        ("AGGREGATE", "aggregate fonksiyonu"),
        ("WINDOW", "pencere fonksiyonu"),
        ("EXTENSION ", "eklentiye ait"),
        ("sistem şeması", "sistem şeması"),
        ("SUPPORT ", "SUPPORT fonksiyonu"),
        ("TRANSFORM", "TRANSFORM FOR TYPE"),
        ("tip ", "desteklenmeyen tip ifadesi"),
        ("argüman modu", "desteklenmeyen argüman modu"),
        ("adsız TABLE kolonu", "adsız TABLE kolonu"),
        ("COST ", "desteklenmeyen COST değeri"),
        ("ROWS ", "desteklenmeyen ROWS değeri"),
        ("iç trigger", "iç trigger (kısıt ya da sistem tarafından yönetilir)"),
        ("bölümlenmiş üst tablodan", "bölümlenmiş üst tablodan klonlanmış (üst tablodan yönetilir)"),
        ("FROM ", "constraint trigger başvurulan tablosu"),
        ("görünüm trigger", "görünüm trigger'ı"),
        ("yabancı tablo trigger", "yabancı tablo trigger'ı"),
        ("WHEN koşulu", "WHEN koşulu ayrıştırılamadı"),
        ("trigger argümanları", "trigger argümanları çözülemedi"),
        ("etkinlik durumu", "desteklenmeyen etkinlik durumu"),
        ("tablo satır tipi", "tablo satır tipi (ilişkiden yönetilir)"),
        ("ilişkisi bulunamayan", "ilişkisi bulunamayan composite"),
        ("range bilgisi", "range bilgisi bulunamadı"),
        ("fonksiyon ", "desteklenmeyen range fonksiyonu"),
        ("MULTIRANGE", "MULTIRANGE (range tipinden yönetilir)"),
        ("dizi tipi", "dizi tipi (öğe tipinden yönetilir)"),
        ("BASE TYPE", "BASE TYPE"),
        ("SHELL TYPE", "SHELL TYPE (tanımsız)"),
        ("PSEUDO TYPE", "PSEUDO TYPE"),
        ("CHECK ", "desteklenmeyen CHECK kısıtı"),
        ("NOT NULL ", "doğrulanmamış NOT NULL kısıtı"),
        // Plan 20 yetki yükleyicileri (`load_acl`): `yetki <metin>`, `imza`, `varsayılan yetki türü <harf>`.
        ("yetki ", "bilinmeyen yetki"),
        ("imza", "imza çözülemedi"),
        ("varsayılan yetki türü ", "desteklenmeyen varsayılan yetki türü"),
        ("veren rolden farklı", "veren rolden farklı (beklenmeyen varsayılan yetki girdisi)"),
        ("üreticinin kabul etmediği", "üreticinin kabul etmediği ad ya da şema"),
        ("geçerli UTF-8 olmayan ad", "geçerli UTF-8 olmayan ad"),
        // Plan 24 bakım hedefi yükleyicisi (`load_maintenance`).
        ("bölümlenmiş indeks", "bölümlenmiş indeks"),
        ("geçici şema", "geçici şema (başka oturumun nesneleri)"),
        // Yükleyicinin `'<kod>' türü …` biçimleri (ilişki/tip/kısıt türü).
        ("'", ""),
    ];
    for (prefix, reason) in PREFIXES {
        if e.starts_with(prefix) {
            if !reason.is_empty() {
                return reason;
            }
            return if e.ends_with("trigger'ı") {
                "desteklenmeyen ilişki türü"
            } else if e.ends_with(" türü tip") {
                "desteklenmeyen tip türü"
            } else if e.contains(" türü kısıt ") {
                "desteklenmeyen kısıt türü"
            } else {
                GENERIC
            };
        }
    }
    GENERIC
}

/// İpucu kararının istekten gelen bağlamı (başarısız ifadenin metni dışında).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct HintCtx {
    /// İstenen fonksiyon tanımı `LEAKPROOF` (create ya da alter'ın yeni tarafı).
    pub(crate) leakproof: bool,
}

impl HintCtx {
    pub(crate) fn of(req: &PlanRequest) -> Self {
        let leakproof = match req {
            PlanRequest::Create { def: ObjectDef::Function(d), .. } | PlanRequest::Alter { new: ObjectDef::Function(d), .. } => d.leakproof,
            _ => false,
        };
        HintCtx { leakproof }
    }
}

/// Uygulama hedefi: yükleyiciyle aynı tek kural (`load::target_for`).
pub(crate) use crate::load::target_for;

/// (2a) İsteğin uygulama hedefi. İstisna (Plan 24): bakım istekleri `target_for`'dan önce ele alınır ve Database türü
/// dahil her zaman bağlı veritabanında (`Target::Database(database_oid)`) çalışır — VACUUM/ANALYZE/REINDEX DATABASE
/// bağlı olunan veritabanını işler (yükleyici `load_maintenance_target` de bu veritabanını doğrular); bakım veritabanı
/// kullanılmaz. `database_oid = None` → `NEEDS_DATABASE`. Diğer istekler `target_for` kuralındadır.
fn apply_target(req: &PlanRequest, database_oid: Option<u32>) -> Result<Target> {
    match req {
        PlanRequest::Maintenance { .. } => database_oid
            .map(Target::Database)
            .ok_or_else(|| Error::Invalid(crate::error_codes::NEEDS_DATABASE.to_string())),
        _ => target_for(req.kind(), database_oid),
    }
}

/// (4a) Bakım veritabanının kendisini yeniden adlandıran, taşıyan, bağlantıya kapatan (ALLOW_CONNECTIONS false ya da
/// CONNECTION LIMIT 0) ya da silen istek (`old`/`def`
/// nesne denetiminden geçmiştir: sunucudaki güncel ad). Yorum/ayar/sahip/bağlantı sınırı değişimi serbesttir.
#[cfg(test)]
fn touches_maintenance(req: &PlanRequest, maintenance_db: &str) -> bool {
    maintenance_refusal(req, maintenance_db) == Some(MAINTENANCE_DATABASE)
}

/// (4a) Reddin iletisi: bakım veritabanının yeniden adlandırılması/taşınması/bağlantıya kapatılması/silinmesi →
/// `MAINTENANCE_DATABASE`; şablon olarak kopyalanması (uygulamanın kendi bakım bağlantısı ona bağlıdır, PG 55006
/// verirdi; inceleme G-5) → `MAINTENANCE_TEMPLATE`.
fn maintenance_refusal(req: &PlanRequest, maintenance_db: &str) -> Option<&'static str> {
    let refused = match req {
        PlanRequest::Alter { old: ObjectDef::Database(o), new: ObjectDef::Database(n), .. } => {
            o.name == maintenance_db
                && (o.name != n.name
                    || o.tablespace != n.tablespace
                    || (o.allow_connections && !n.allow_connections)
                    || (o.connection_limit != 0 && n.connection_limit == 0))
        }
        PlanRequest::Drop { def: ObjectDef::Database(d), .. } => d.name == maintenance_db,
        PlanRequest::Create { def: ObjectDef::Database(d), .. } if d.template.as_deref() == Some(maintenance_db) => {
            return Some(MAINTENANCE_TEMPLATE);
        }
        _ => false,
    };
    refused.then_some(MAINTENANCE_DATABASE)
}

/// Kendi havuz bağlantılarımızın kapatılması gereken veritabanı (yeniden adlandırma / taşıma / silme).
fn database_to_release(req: &PlanRequest) -> Option<u32> {
    match req {
        PlanRequest::Alter { object, old: ObjectDef::Database(o), new: ObjectDef::Database(n) }
            if o.name != n.name || o.tablespace != n.tablespace =>
        {
            Some(object.oid)
        }
        PlanRequest::Drop { object, def: ObjectDef::Database(_), .. } => Some(object.oid),
        _ => None,
    }
}

/// (9) Parola kapısı: gizlenmiş ifade ↔ `ApplyOptions.password` birlikte olmalı.
fn password_gate(statements: &[Statement], password: Option<&Secret>) -> Result<()> {
    match (statements.iter().any(|s| s.redacted), password) {
        (true, None) => Err(Error::Invalid(ROLE_PASSWORD_MISSING.into())),
        (false, Some(_)) => Err(Error::Invalid(PASSWORD_NOT_USED.into())),
        _ => Ok(()),
    }
}

/// İstekteki (yeni) rol adı: gizlenmiş parola ifadesinin ait olabileceği tek rol.
fn password_role(req: &PlanRequest) -> Option<&str> {
    match req {
        PlanRequest::Create { def: ObjectDef::Role(r), .. } | PlanRequest::Alter { new: ObjectDef::Role(r), .. } => Some(&r.name),
        _ => None,
    }
}

/// (10) Çalıştırma listesi: gizlenmiş ifade, istekteki rol adı ve parolayla somutlaşır (`role::materialize_password`:
/// kabul kuralı + istemci tarafı SCRAM-SHA-256); diğerleri plandaki metni ödünç alır. Plan (`PlanView`) değişmez.
pub(crate) fn materialize<'a>(statements: &'a [Statement], req: &PlanRequest, secret: Option<&Secret>) -> Result<Vec<ExecStatement<'a>>> {
    materialize_for(statements, password_role(req), secret)
}

fn materialize_for<'a>(statements: &'a [Statement], role: Option<&str>, secret: Option<&Secret>) -> Result<Vec<ExecStatement<'a>>> {
    statements
        .iter()
        .map(|s| {
            if !s.redacted {
                return Ok(ExecStatement::plain(s));
            }
            let (Some(role), Some(secret)) = (role, secret) else { return Err(Error::Invalid(ROLE_PASSWORD_MISSING.into())) };
            // Yalnızca bu rolün üretici yer tutucusu somutlaşır; başka bir gizlenmiş metin asla.
            if s.sql != role::password_redacted(role).sql {
                return Err(Error::Invalid(INVALID_DEF.into()));
            }
            let password = role::materialize_password(role, secret).map_err(|m| Error::Invalid(m.into()))?;
            Ok(ExecStatement { sql: ExecSql::Password(password), transactional: s.transactional, redacted: true })
        })
        .collect()
}

/// İfadeye ve SQLSTATE'e göre ipucu (Karar 1/8/9, inceleme m-2: kod tek başına yetmez, başarısız ifade de eşleşmeli).
fn hint_for(kind: ObjectKind, op: &str, ctx: HintCtx, sql: &str, code: &str) -> Option<&'static str> {
    let routine = matches!(kind, ObjectKind::Function | ObjectKind::Procedure);
    let acl_op = op == "privileges" || op == "default_privileges";
    // 42704 rol eksikliği yalnız yetki ifadesinde; Type/Domain'de aynı kod nesnenin kaybolması da olabilir (yeniden
    // yükleme eşitliğinden sonra yarış) — orada yanıltıcı ipucu verilmez, PG iletisi yeterlidir.
    let role_missing = acl_op
        && !matches!(kind, ObjectKind::Type | ObjectKind::Domain)
        && ["GRANT ", "REVOKE ", "ALTER DEFAULT PRIVILEGES "].iter().any(|p| sql.starts_with(p));
    let maintenance = op == "maintenance";
    Some(match (code, kind) {
        // Plan 24: bakım işlemleri türden bağımsız kendi ipuçlarını alır; diğer kollar (ör. Database 55006) uygulanmaz.
        // 57014 (iptal) ipucusuz: iptal iletisi yeter.
        ("42501", _) if maintenance => MAINTENANCE_PRIVILEGE_HINT,
        ("0A000", _) if maintenance => MAINTENANCE_UNSUPPORTED_HINT,
        ("25001", _) if maintenance => MAINTENANCE_TX_HINT,
        _ if maintenance => return None,
        ("2BP01", ObjectKind::Function | ObjectKind::Procedure | ObjectKind::Type | ObjectKind::Domain) if op == "drop" => {
            VIEW_DEPENDENTS_HINT
        }
        // Yeniden kurma ipucu yalnız tanım işlemlerinde (yetki ifadesinde 42809 yeniden kurmayla çözülmez).
        ("42809", _) if routine && !acl_op => ROUTINE_KIND_HINT,
        ("0A000", ObjectKind::Type) if sql.starts_with("ALTER TYPE ") && sql.contains(" ALTER ATTRIBUTE ") => TYPE_COLUMN_RETYPE_HINT,
        // Fonksiyon tanımı (CREATE [OR REPLACE]) LEAKPROOF isterken yetki hatası; sahip/yorum ifadeleri genel ipucu alır.
        // Ödünleşim: aynı CREATE ifadesindeki başka bir 42501 (ör. şemada CREATE yetkisi yok, dil USAGE yok) da
        // LEAKPROOF istendiğinde bu ipucunu alır; PG iletisi asıl nedeni yine söyler, ipucu yalnızca ek açıklamadır.
        ("42501", ObjectKind::Function) if ctx.leakproof && sql.starts_with("CREATE ") => LEAKPROOF_HINT,
        ("42P16" | "42804", ObjectKind::View) if sql.starts_with("CREATE OR REPLACE VIEW ") => VIEW_COLUMNS_HINT,
        ("55000", ObjectKind::MaterializedView) if sql.starts_with("REFRESH MATERIALIZED VIEW CONCURRENTLY ") => {
            REFRESH_CONCURRENTLY_HINT
        }
        ("55006", ObjectKind::Database) if sql.starts_with("CREATE DATABASE ") => TEMPLATE_IN_USE_HINT,
        ("55006", ObjectKind::Database) if sql.starts_with("DROP DATABASE ") && sql.ends_with(" WITH (FORCE)") => FORCE_LIMITS_HINT,
        ("55006", ObjectKind::Database) => DATABASE_IN_USE_HINT,
        // Plan 20 Karar 15: yetki istekleri işleme (`op`) göre; genel 42501 kolundan önce.
        ("42501", _) if op == "privileges" => ACL_PRIVILEGE_HINT,
        ("42501", _) if op == "default_privileges" => DEFAULT_ACL_PRIVILEGE_HINT,
        ("42704", _) if role_missing => ACL_ROLE_MISSING_HINT,
        ("42501", _) => PRIVILEGE_HINT,
        ("2BP01", ObjectKind::Role) => ROLE_DEPENDENTS_HINT,
        // Düzenlemedeki DROP (yeniden kurma) CASCADE sunmaz; yalnızca silme akışında CASCADE önerilir.
        ("2BP01", ObjectKind::View | ObjectKind::MaterializedView) if op == "alter" => REBUILD_DEPENDENTS_HINT,
        ("2BP01", ObjectKind::View | ObjectKind::MaterializedView) => VIEW_DEPENDENTS_HINT,
        _ => return None,
    })
}

/// PG'nin kendi ipucunun YERİNE geçen ipuçları: PG bu durumlarda arayüzün sunmadığı bir eylemi (CASCADE ya da SQL ile
/// önce DROP) önerir; bunun yerine arayüzdeki karşılığı söylenir. `pg_hint`: PG yükünde ipucu var mı.
///
/// 42P13 yalnızca PG ipucu varken: gerçek OR REPLACE sınırı hataları ("cannot change return type …" vb.) her zaman
/// "Use DROP FUNCTION … first." ipucu taşır; aynı kodu gövde doğrulaması da verir (ör. sql fonksiyonunda dönüş tipi
/// uyuşmazlığı) ve orada ipucu yoktur — oraya "Yeniden oluştur" önerisi eklenmez.
fn override_hint_for(kind: ObjectKind, op: &str, sql: &str, code: &str, pg_hint: bool) -> Option<&'static str> {
    let routine = matches!(kind, ObjectKind::Function | ObjectKind::Procedure);
    Some(match (code, kind) {
        // Plan 24: bakımda kilit zaman aşımı (`apply::failure_payload`'ın genel ipucunun yerine).
        ("55P03", _) if op == "maintenance" => MAINTENANCE_LOCK_HINT,
        // Plan 20 Karar 15: PG bağımlı yetkilerde "Use CASCADE to revoke them too." der; arayüzdeki karşılığı söylenir.
        ("2BP01", _) if op == "privileges" => ACL_DEPENDENTS_HINT,
        ("2BP01", _) if routine && op == "alter" && (sql.starts_with("DROP FUNCTION ") || sql.starts_with("DROP PROCEDURE ")) => {
            ROUTINE_REBUILD_DEPENDENTS_HINT
        }
        ("2BP01", ObjectKind::Type) if op == "alter" && sql.starts_with("ALTER TYPE ") && sql.contains(" ATTRIBUTE ") => TYPED_TABLE_HINT,
        ("42P13", _) if routine && pg_hint && sql.starts_with("CREATE OR REPLACE ") => OR_REPLACE_HINT,
        _ => return None,
    })
}

/// Başarısız ifadenin arayüz yükü: `apply::failure_payload` + (PG ipucu yoksa) türe/ifadeye göre ipucu; kısmi
/// uygulamada (`committed > 0`, transaction dışı) ipucu yoksa bunu söyler. Gizlenmiş ifadede sunucu iletisi
/// doğrulayıcı izi taşıyorsa sabit iletiyle değiştirilir (savunma; konum zaten `None`).
#[cfg(test)]
pub(crate) fn failure_payload_for(kind: ObjectKind, op: &str, failed: Option<&Statement>, committed: u32, e: &Error) -> ErrorPayload {
    failure_payload_with(kind, op, HintCtx::default(), failed, committed, e)
}

/// `failure_payload_for` + istek bağlamı (`HintCtx`).
pub(crate) fn failure_payload_with(
    kind: ObjectKind,
    op: &str,
    ctx: HintCtx,
    failed: Option<&Statement>,
    committed: u32,
    e: &Error,
) -> ErrorPayload {
    let mut p = apply::failure_payload(e);
    if failed.is_some_and(|s| s.redacted) {
        let leaks = |v: &Option<String>| v.as_deref().is_some_and(|t| t.contains("SCRAM-SHA-256$"));
        if p.message.contains("SCRAM-SHA-256$") || leaks(&p.detail) || leaks(&p.hint) {
            p.message = REDACTED_FAILURE_MESSAGE.into();
            p.detail = None;
            p.hint = None;
        }
    }
    if let (Some(s), Some(code)) = (failed, apply::sqlstate(e)) {
        if let Some(h) = override_hint_for(kind, op, &s.sql, code, p.hint.is_some()) {
            p.hint = Some(h.to_string());
        } else if p.hint.is_none() {
            p.hint = hint_for(kind, op, ctx, &s.sql, code).map(str::to_string);
        }
    }
    if p.hint.is_none() && committed > 0 && failed.is_some_and(|s| !s.transactional) {
        p.hint = Some(PARTIAL_APPLY_HINT.into());
    }
    p
}

/// Üyelik kümesi (rol, admin) farklı mı: PG 16+ salt okunur `inheritOption`/`setOption` karşılaştırılmaz.
/// Yüklemede salt okunur (`unsupported`: başka grantor / birden çok grantor) listelenen üyelikler de VARDIR: süper
/// kullanıcı olmayan bağlantının başarılı GRANT'ı kendi adıyla grantor olur ve oraya düşer (yanlış uyarı olmasın).
/// Onların admin bayrağı bilinmez → istenen değer varsayılır (istenmeyen bir üyelik kaldıysa yine fark sayılır).
fn memberships_differ(intended: &RoleDef, loaded: &RoleDef) -> bool {
    let key = |m: &RoleMembership| (m.role.clone(), m.admin);
    let a: std::collections::BTreeSet<_> = intended.member_of.iter().map(key).collect();
    let mut b: std::collections::BTreeSet<_> = loaded.member_of.iter().map(key).collect();
    for role in loaded.unsupported.iter().filter_map(|e| load::unsupported_membership_role(e)) {
        let admin = intended.member_of.iter().find(|m| m.role == role).is_some_and(|m| m.admin);
        b.insert((role.to_string(), admin));
    }
    a != b
}

/// Karşılaştırma biçimi: boş satırlar (alıcının tüm yetkileri kaldırıldı) düşer, satırlar/yetkiler kanonik sırada.
fn canonical_grants(grants: &[AclGrant], order: &[crate::acl::Privilege]) -> Vec<AclGrant> {
    let mut v: Vec<AclGrant> = grants.iter().filter(|g| !g.privileges.is_empty()).cloned().collect();
    sort_grants(&mut v, order);
    v
}

/// Plan 20 Karar 14: uygulama sonrası yeniden yüklenip karşılaştırılacak istenen `grants` (kanonik).
#[derive(Debug, Clone, PartialEq, Eq)]
enum VerifyAcl {
    /// Nesne ACL'si: nesne ve istenen `grants`.
    Object(ObjectRef, Vec<AclGrant>),
    /// Varsayılan yetkiler: kapsam ve bölüm sırasıyla istenen `grants`.
    Default(DefaultAclScope, Vec<Vec<AclGrant>>),
}

impl VerifyAcl {
    fn of(req: &PlanRequest) -> Option<Self> {
        // `170000`: yalnız kanonik sıra için tüm sürümlerin kümesi (MAINTAIN dahil); sürüm kapısı değildir.
        match req {
            PlanRequest::Privileges { object, new, .. } => {
                Some(VerifyAcl::Object(*object, canonical_grants(&new.grants, privileges_for(object.kind, 170000))))
            }
            PlanRequest::DefaultPrivileges { scope, new, .. } => Some(VerifyAcl::Default(
                scope.clone(),
                new.sections.iter().map(|s| canonical_grants(&s.grants, default_privileges_for(s.object_type, 170000))).collect(),
            )),
            _ => None,
        }
    }
}

/// Yeniden yüklenen nesne ACL'sinin `grants`'ı istenenden farklı mı (`foreign`/`columns` karşılaştırılmaz: sahip
/// olmayan bağlı rolün GRANT'ı `foreign`'a düşer ve burada fark olarak görünür).
fn acl_grants_differ(intended: &[AclGrant], loaded: &AclDef) -> bool {
    // `170000`: yalnız kanonik sıra (tüm sürümlerin kümesi).
    intended != canonical_grants(&loaded.grants, privileges_for(loaded.kind, 170000))
}

/// Yeniden yüklenen varsayılan yetkilerin bölüm `grants`'ı istenenden farklı mı (`baseline` karşılaştırılmaz).
fn default_acl_grants_differ(intended: &[Vec<AclGrant>], loaded: &DefaultAclDef) -> bool {
    // `170000`: yalnız kanonik sıra (tüm sürümlerin kümesi).
    let got: Vec<Vec<AclGrant>> =
        loaded.sections.iter().map(|s| canonical_grants(&s.grants, default_privileges_for(s.object_type, 170000))).collect();
    intended != got.as_slice()
}

const SCHEMA_NAME_SQL: &str = "SELECT nspname::text AS name FROM pg_namespace WHERE oid = $1";
const TABLE_NAME_SQL: &str = "SELECT n.nspname::text AS schema, c.relname::text AS name \
     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = $1 AND c.relkind IN ('r', 'm', 'p')";

const TRIGGER_TABLE_SQL: &str = "SELECT n.nspname::text AS schema, c.relname::text AS name \
     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = $1 AND c.relkind IN ('r', 'p')";
/// Yeni fonksiyon/procedure gövde şablonu (plpgsql).
const FUNCTION_BODY: &str = "BEGIN\n\nEND;";
/// Trigger fonksiyonu gövde şablonu.
const TRIGGER_FUNCTION_BODY: &str = "BEGIN\n  RETURN NEW;\nEND;";

pub struct DdlManager {
    mgr: Arc<ConnectionManager>,
    registry: Arc<RwLock<Registry>>,
    cancels: CancelMap,
    /// Son `apply`'ın çalışma zamanı: `cancel_all` Tokio bağlamı olmayan iş parçacığından (Tauri ana iş parçacığı) çağrılabilir.
    rt: Mutex<Option<tokio::runtime::Handle>>,
}

/// Çalıştırmanın günlük/iptal bilgileri.
struct Job {
    op: &'static str,
    kind: ObjectKind,
    target: Target,
    lock_timeout_ms: u32,
    apply_id: Uuid,
    /// Uygulama sonrası kimliği aranacak nesne (Create / kimliği değişen alter).
    created: Option<ObjectDef>,
    /// Değiştirilen nesne (alter/drop).
    object: Option<ObjectRef>,
    /// Çalıştırma öncesi ve sonrası havuzdan bırakılacak veritabanları (Database yeniden adlandırma/taşıma/silme;
    /// CREATE DATABASE'in şablonu).
    release: Vec<u32>,
    /// Uygulama sonrası üyelikleri denetlenecek rol tanımı (Role create/alter).
    verify_role: Option<RoleDef>,
    /// Uygulama sonrası yetkileri denetlenecek istek (Plan 20 Karar 14).
    verify_acl: Option<VerifyAcl>,
    /// Hata ipucu bağlamı (istekten).
    hint: HintCtx,
}

/// Kayıttan okunan koruma bayrakları.
struct Guard {
    locked: bool,
    production: bool,
    maintenance_db: String,
}

impl DdlManager {
    pub fn new(mgr: Arc<ConnectionManager>, registry: Arc<RwLock<Registry>>) -> Self {
        DdlManager { mgr, registry, cancels: CancelMap::default(), rt: Mutex::new(None) }
    }

    fn guard(&self, server: Uuid) -> Result<Guard> {
        let registry = self.registry.read().unwrap_or_else(PoisonError::into_inner);
        let config = registry.get(server).ok_or(Error::UnknownServer(server))?;
        Ok(Guard { locked: config.read_only_lock, production: config.is_production(), maintenance_db: config.maintenance_db.clone() })
    }

    /// Önizleme: doğrulama hataları `PlanView.errors`'tadır (Err değil). Hiçbir şey yazmaz; veritabanı gerekmez.
    pub async fn preview(&self, server: Uuid, _database_oid: impl Into<Option<u32>> + Send, req: &PlanRequest) -> Result<PlanView> {
        let version_num = load::server_version(&self.mgr, server).await?;
        let view = plan_view(req, version_num);
        tracing::debug!(op = req.op(), kind = ?req.kind(), statements = view.statements.len(), errors = view.errors.len(), "DDL önizlemesi");
        Ok(view)
    }

    /// Kapılardan geçen planı ayrı bağlantıda uygular. Kapı reddi ve bağlantı kurulamaması `Err`; ifade
    /// hatası/iptali `Ok(ApplyOutcome::Failed)`. Database/Role için `database_oid` yok sayılır (bakım veritabanı).
    pub async fn apply(
        &self,
        server: Uuid,
        database_oid: impl Into<Option<u32>> + Send,
        req: &PlanRequest,
        opts: ApplyOptions,
    ) -> Result<ApplyOutcome> {
        let database_oid = database_oid.into();
        // (1) + (2)
        let guard = self.guard(server)?;
        if guard.locked {
            return Err(Error::Invalid(LOCKED_MESSAGE.into()));
        }
        // (2c) yetki isteğinin biçimi (Plan 20): bağlantıdan önce.
        if let Some(m) = request_shape_refusal(req) {
            return Err(Error::Invalid(m.into()));
        }
        // (2b) salt okunur nesne (Plan 18 Karar 4): alter/drop, bağlantı ve plan üretiminden önce kodsuz `invalid`.
        if let Some(m) = read_only_refusal(req) {
            return Err(Error::Invalid(m));
        }
        // (2a)
        let target = apply_target(req, database_oid)?;
        // (3) — sürüm oturumun önbelleğinden okunur (bağlantı açılmaz).
        let view = plan_view(req, load::server_version(&self.mgr, server).await?);
        if !view.errors.is_empty() {
            return Err(Error::Invalid(INVALID_DEF.into()));
        }
        if view.empty {
            return Err(Error::Invalid(NO_CHANGES.into()));
        }
        // (4) nesne denetimi: istemcinin `old`/`def`'i sunucudaki güncel tanımla birebir aynı olmalı (sunucu kapsamlı
        // türler bakım bağlantısında yüklenir; rolde yüklü parola niyeti her zaman `keep`'tir).
        // Yetki isteklerinde `old` ilgili yükleyicinin çıktısıyla karşılaştırılır (`viewer` dahil).
        let unchanged = match object_check(req) {
            None => true,
            Some(Expected::Def(object, expected)) => {
                if object.kind != expected.kind() {
                    return Err(Error::Invalid(INVALID_DEF.into()));
                }
                load::load(&self.mgr, server, database_oid, object).await? == *expected
            }
            Some(Expected::Acl(object, expected)) => {
                crate::load_acl::load_acl(&self.mgr, server, database_oid, object).await? == *expected
            }
            Some(Expected::DefaultAcl(scope, expected)) => {
                // (2a) veritabanı kapsamlı hedefi doğruladı.
                let Target::Database(db) = target else { return Err(Error::Invalid(INVALID_DEF.into())) };
                crate::load_acl::load_default_acl(&self.mgr, server, db, scope.clone()).await? == *expected
            }
            // Plan 24: hedef yeniden yüklenir (yeniden adlandırma, silme, sahte şema/ad → fark ya da `NotFound`).
            Some(Expected::Maintenance(object, expected)) => {
                let Target::Database(db) = target else { return Err(Error::Invalid(INVALID_DEF.into())) };
                crate::load_maintenance::load_maintenance_target(&self.mgr, server, db, object).await? == *expected
            }
        };
        if !unchanged {
            return Err(Error::Invalid(PLAN_CHANGED.into()));
        }
        // (4a) Oturumun gerçekten bağlı olduğu bakım veritabanı da (kayıt bağlantıdan sonra değişmiş olabilir; inceleme
        // G-7) kayıttaki ad da korunur.
        let session_maintenance = self.mgr.session_maintenance_db(server).await;
        for name in [Some(guard.maintenance_db.as_str()), session_maintenance.as_deref()].into_iter().flatten() {
            if let Some(m) = maintenance_refusal(req, name) {
                return Err(Error::Invalid(m.into()));
            }
        }
        // (5)
        if view.hash != opts.expected_hash {
            return Err(Error::Invalid(PLAN_CHANGED.into()));
        }
        // (6)
        if view.risk >= Risk::Destructive && !opts.confirmed {
            return Err(Error::Invalid(CONFIRMATION_REQUIRED.into()));
        }
        // (7)
        if view.risk == Risk::DataLoss && guard.production && !opts.confirmed_production {
            return Err(Error::Invalid(PRODUCTION_CONFIRMATION_REQUIRED.into()));
        }
        // (8)
        check_lock_timeout(opts.lock_timeout_ms)?;
        // (9) + (10)
        password_gate(&view.statements, opts.password.as_ref())?;
        let exec = materialize(&view.statements, req, opts.password.as_ref())?;
        let created = match req {
            PlanRequest::Create { def, .. } => Some(def.clone()),
            PlanRequest::Alter { old, new, .. } if identity_changed(old, new) => Some(new.clone()),
            _ => None,
        };
        let verify_role = match req {
            PlanRequest::Create { def: ObjectDef::Role(r), .. } | PlanRequest::Alter { new: ObjectDef::Role(r), .. } => Some(r.clone()),
            _ => None,
        };
        let job = Job {
            op: req.op(),
            kind: req.kind(),
            target,
            lock_timeout_ms: opts.lock_timeout_ms,
            apply_id: opts.apply_id,
            created,
            object: object_check(req).and_then(|e| e.object()),
            release: self.databases_to_release(server, req).await,
            verify_role,
            verify_acl: VerifyAcl::of(req),
            hint: HintCtx::of(req),
        };
        self.execute(server, &view.statements, exec, job).await
    }

    /// Bağlantı açar, ifadeleri blok blok çalıştırır, sonucu günlüğe (yalnızca tür/sayı/süre/sonuç) yazar.
    /// `statements` plandaki (gizlenmiş) metinlerdir: ipucu/konum kararları bunlara bakar; `exec` çalıştırılan listedir.
    async fn execute(&self, server: Uuid, statements: &[Statement], exec: Vec<ExecStatement<'_>>, job: Job) -> Result<ApplyOutcome> {
        let Job { op, kind, target, lock_timeout_ms, apply_id, created, object, release, verify_role, verify_acl, hint } = job;
        let rt = tokio::runtime::Handle::current();
        *self.rt.lock().unwrap_or_else(PoisonError::into_inner) = Some(rt.clone());
        let started = Instant::now();
        // Yuva bağlantıdan önce açılır: bağlantı kurulurken gelen iptal de dikkate alınır.
        let reg = self.cancels.register(apply_id)?;
        let count = u32::try_from(exec.len()).unwrap_or(u32::MAX);
        // Kayıttan önce gelen iptal (mezar taşı): bağlantı açmadan, hiçbir ifade çalışmadan iptal sonucu.
        if reg.cancelled() {
            return Ok(ApplyOutcome::Failed {
                failed_index: 0,
                committed: 0,
                sqlstate: Some(apply::CANCELED_SQLSTATE.into()),
                position: None,
                error: apply::failure_payload(&apply::cancelled_error()),
                created: None,
                messages: vec![],
            });
        }
        // Kendi havuz bağlantımız (Gezgin/katalog) yeniden adlandırma/taşıma/silmeyi 55006 ile engellemesin (Karar 8).
        for &oid in &release {
            self.mgr.release_database(server, oid).await;
        }
        let conn = match apply::open_apply_conn(&self.mgr, server, target, lock_timeout_ms).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(op, ?kind, statements = count, outcome = apply::sqlstate(&e).unwrap_or("connect"), "DDL bağlantısı açılamadı");
                return Err(e);
            }
        };
        let DedicatedConn { client, mut notices, cancel, .. } = conn;
        reg.set_handle(cancel.clone());
        // Çağıran gelecek çalışırken düşerse (ör. pencere kapanışı) sunucudaki ifade iptal edilir.
        let mut guard = CancelOnDrop { handle: cancel, rt, armed: true };
        // Sıra: yuva → bağlantı → pid. Yalnız bakım uygulamalarında (ilerleme yalnız onlar için sorulur; diğer DDL
        // uygulamaları fazladan gidiş-dönüş yapmaz). En iyi çaba: okunamazsa ilerleme sorgusu `None` döner, uygulama
        // sürer (bağlantı koptuysa ilk ifade zaten hata verir).
        if op == "maintenance" {
            match client.query_one("SELECT pg_backend_pid()", &[]).await {
                Ok(row) => reg.set_backend_pid(row.get(0)),
                Err(e) => tracing::debug!(op, sqlstate = e.code().map(|c| c.code()), "arka uç pid'i okunamadı"),
            }
        }
        // İletiler ifadelerle eşzamanlı okunur (kanal birikmez); metin hiçbir yere yazılmaz.
        let mut messages = apply::MessageBuf::default();
        let result = apply::with_messages(apply::run_statements(&client, &exec, &reg), &mut notices, &mut messages).await;
        guard.armed = false;
        drop(reg);
        // Somutlaşmış parola ifadesi (varsa) burada sıfırlanarak düşer.
        drop(exec);
        // Uygulama bağlantısı, havuzdaki katalog sorgusundan (`find_created`) önce kapanır; kapanışa dek gelen
        // son iletiler de alınır.
        drop(client);
        apply::drain_closed(&mut notices, &mut messages).await;
        drop(notices);
        let messages = messages.finish();
        // Arada (ör. Gezgin yenilemesi) açılmış bağlantı ve eski ad önbelleği de bırakılır.
        for &oid in &release {
            self.mgr.release_database(server, oid).await;
        }
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(match result {
            RunResult::Done => {
                tracing::info!(op, ?kind, statements = count, elapsed_ms, outcome = "applied", "DDL uygulandı");
                let created = match &created {
                    Some(def) => self.find_created(server, target, def).await,
                    None => None,
                };
                let mut warnings = vec![];
                if let Some(intended) = &verify_role
                    && let Some(oid) = created.map(|c| c.oid).or(object.map(|o| o.oid))
                    && let Ok(ObjectDef::Role(loaded)) = load::load(&self.mgr, server, None::<u32>, ObjectRef { kind: ObjectKind::Role, oid }).await
                    && memberships_differ(intended, &loaded)
                {
                    tracing::info!(op, ?kind, outcome = "membership_mismatch", "DDL sonrası üyelik farkı");
                    warnings.push(MEMBERSHIP_WARNING.to_string());
                }
                if let Some(v) = &verify_acl
                    && self.acl_mismatch(server, target, v).await
                {
                    tracing::info!(op, ?kind, outcome = "acl_mismatch", "DDL sonrası yetki farkı");
                    warnings.push(ACL_WARNING.to_string());
                }
                ApplyOutcome::Applied { statements: count, elapsed_ms, created, warnings: (!warnings.is_empty()).then_some(warnings), messages }
            }
            RunResult::Failed { failed_index, committed, error, outcome_unknown, position } => {
                let sqlstate = apply::sqlstate(&error).map(str::to_string);
                tracing::info!(
                    op,
                    ?kind,
                    statements = count,
                    elapsed_ms,
                    outcome = sqlstate.as_deref().unwrap_or("error"),
                    "DDL uygulanamadı"
                );
                let failed = statements.get(failed_index as usize);
                let position = if failed.is_some_and(|s| s.redacted) { None } else { position };
                let error = if outcome_unknown {
                    apply::unknown_outcome_payload(&error)
                } else {
                    failure_payload_with(kind, op, hint, failed, committed, &error)
                };
                // Kısmi uygulama: oluşturulan/yeniden adlandırılan nesne yine de bulunur (ör. CREATE DATABASE kalıcı).
                let created = match (&created, committed) {
                    (Some(def), 1..) => self.find_created(server, target, def).await,
                    _ => None,
                };
                ApplyOutcome::Failed { failed_index, committed, sqlstate, position, error, created, messages }
            }
        })
    }

    /// Test yardımcısı: kapıları atlayıp verilen ifade listesini aynı blok çalıştırıcısıyla uygular
    /// (karışık transactional/transaction dışı planlar üreticilerle kurulamadığı için). `password`: gizlenmiş
    /// ifadeleri somutlaştırmak için (rol adı, parola).
    #[cfg(test)]
    pub(crate) async fn apply_statements(
        &self,
        server: Uuid,
        target: Target,
        statements: &[Statement],
        lock_timeout_ms: u32,
        password: Option<(&str, &Secret)>,
    ) -> Result<ApplyOutcome> {
        self.apply_statements_as(Uuid::new_v4(), "test", server, target, statements, lock_timeout_ms, password).await
    }

    /// `apply_statements`; `apply_id` ve `op` çağırandan (iptal/pid testleri: pid yalnız `op = "maintenance"`).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn apply_statements_as(
        &self,
        apply_id: Uuid,
        op: &'static str,
        server: Uuid,
        target: Target,
        statements: &[Statement],
        lock_timeout_ms: u32,
        password: Option<(&str, &Secret)>,
    ) -> Result<ApplyOutcome> {
        let exec = materialize_for(statements, password.map(|p| p.0), password.map(|p| p.1))?;
        let job = Job {
            op,
            kind: ObjectKind::Schema,
            target,
            lock_timeout_ms,
            apply_id,
            created: None,
            object: None,
            release: vec![],
            verify_role: None,
            verify_acl: None,
            hint: HintCtx::default(),
        };
        self.execute(server, statements, exec, job).await
    }

    /// Plan 20 Karar 14: yetkileri uygulama hedefinden yeniden yükleyip istenenle karşılaştırır. En iyi çaba:
    /// yeniden yükleme hatası uyarı üretmez (`false`).
    async fn acl_mismatch(&self, server: Uuid, target: Target, v: &VerifyAcl) -> bool {
        let db = match target {
            Target::Database(db) => Some(db),
            Target::Maintenance => None,
        };
        match v {
            VerifyAcl::Object(object, want) => crate::load_acl::load_acl(&self.mgr, server, db, *object)
                .await
                .is_ok_and(|loaded| acl_grants_differ(want, &loaded)),
            VerifyAcl::Default(scope, want) => match db {
                Some(db) => crate::load_acl::load_default_acl(&self.mgr, server, db, scope.clone())
                    .await
                    .is_ok_and(|loaded| default_acl_grants_differ(want, &loaded)),
                None => false,
            },
        }
    }

    /// Süren uygulamayı iptal eder; bilinmeyen/bitmiş kimlik → `Ok` (idempotent). Kimlik henüz kayıtlı değilse
    /// kısa ömürlü mezar taşı bırakılır (`apply` kaydında iptalle doğar). İstek ifade başlamadan önce ulaşıp
    /// yutulmuş olabilir: yuva hâlâ duruyorsa kısa aralıklarla (en çok 3 kez) yeniden gönderilir.
    pub async fn cancel(&self, apply_id: Uuid) -> Result<()> {
        let Some(Some(handle)) = self.cancels.request(apply_id) else { return Ok(()) };
        let first = handle.cancel().await;
        for _ in 0..CANCEL_RESENDS {
            tokio::time::sleep(CANCEL_RESEND_AFTER).await;
            match self.cancels.pending_cancel(apply_id) {
                Some(h) => {
                    let _ = h.cancel().await;
                }
                None => break,
            }
        }
        first
    }

    /// Tüm süren uygulamaları iptal eder (uygulama kapanışı/yeniden yükleme); istekler arka planda gönderilir.
    /// Ortam Tokio bağlamı olmayan iş parçacığından (Tauri ana iş parçacığı) da çalışır: bağlam yoksa son
    /// `apply`'ın çalışma zamanı kullanılır (uçuşta uygulama varsa zaten vardır).
    /// Gönderilmek üzere sıraya alınan iptal sayısını döndürür.
    pub fn cancel_all(&self) -> usize {
        let handles = self.cancels.request_all();
        if handles.is_empty() {
            return 0;
        }
        let rt = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.rt.lock().unwrap_or_else(PoisonError::into_inner).clone());
        let Some(rt) = rt else { return 0 };
        let n = handles.len();
        for h in handles {
            rt.spawn(async move {
                if h.cancel().await.is_err() {
                    tracing::debug!("DDL iptal isteği gönderilemedi");
                }
            });
        }
        n
    }

    /// Bırakılacak veritabanları: yeniden adlandırılan/taşınan/silinen veritabanı ve CREATE DATABASE şablonu (kendi havuz
    /// bağlantımız kopyalamayı 55006 ile engellemesin; inceleme G-5). Şablon oid'i bulunamazsa (yok/yetki) atlanır.
    async fn databases_to_release(&self, server: Uuid, req: &PlanRequest) -> Vec<u32> {
        let mut out: Vec<u32> = database_to_release(req).into_iter().collect();
        if let PlanRequest::Create { def: ObjectDef::Database(d), .. } = req
            && let Some(template) = d.template.clone()
        {
            let row = self
                .mgr
                .run(server, Target::Maintenance, move |c| {
                    let template = template.clone();
                    async move { c.client.query_opt(FIND_DATABASE_SQL, &[&template]).await }
                })
                .await;
            if let Ok(Some(r)) = row {
                out.push(r.get::<_, u32>(0));
            }
        }
        out
    }

    /// Yeni nesnenin oid'i (kimlik tür + şema + addan, havuzdaki katalog bağlantısıyla; Database/Role bakım
    /// bağlantısında). En iyi çaba: hata → `None`.
    async fn find_created(&self, server: Uuid, target: Target, def: &ObjectDef) -> Option<ObjectRef> {
        let text = |v: &[&String]| v.iter().map(|s| FindParam::Text((*s).clone())).collect::<Vec<_>>();
        let (kind, sql, params): (ObjectKind, std::borrow::Cow<'static, str>, Vec<FindParam>) = match def {
            ObjectDef::Schema(d) => (ObjectKind::Schema, FIND_SCHEMA_SQL.into(), text(&[&d.name])),
            ObjectDef::Extension(d) => (ObjectKind::Extension, FIND_EXTENSION_SQL.into(), text(&[&d.name])),
            ObjectDef::Sequence(d) => (ObjectKind::Sequence, FIND_SEQUENCE_SQL.into(), text(&[&d.schema, &d.name])),
            ObjectDef::Index(d) => (ObjectKind::Index, FIND_INDEX_SQL.into(), text(&[&d.schema, &d.name])),
            ObjectDef::Table(d) => (ObjectKind::Table, FIND_TABLE_SQL.into(), text(&[&d.schema, &d.name])),
            ObjectDef::View(d) => (ObjectKind::View, FIND_VIEW_SQL.into(), text(&[&d.schema, &d.name])),
            ObjectDef::MaterializedView(d) => (ObjectKind::MaterializedView, FIND_MATVIEW_SQL.into(), text(&[&d.schema, &d.name])),
            ObjectDef::Database(d) => (ObjectKind::Database, FIND_DATABASE_SQL.into(), text(&[&d.name])),
            ObjectDef::Role(d) => (ObjectKind::Role, FIND_ROLE_SQL.into(), text(&[&d.name])),
            ObjectDef::Function(d) => {
                // Sürüm oturum önbelleğinden (bağlantı açılmaz); PG 10'da `prokind` yok.
                let version = load::server_version(&self.mgr, server).await.ok()?;
                let filter = if version >= 110000 { "p.prokind = 'f'" } else { "NOT p.proisagg AND NOT p.proiswindow" };
                let (types, _) = routine_find_args(ObjectKind::Function, d);
                let mut p = text(&[&d.schema, &d.name]);
                p.push(FindParam::List(types));
                (ObjectKind::Function, FIND_FUNCTION_SQL.replace("{kind}", filter).into(), p)
            }
            ObjectDef::Procedure(d) => {
                let (types, modes) = routine_find_args(ObjectKind::Procedure, d);
                let mut p = text(&[&d.schema, &d.name]);
                p.push(FindParam::List(types));
                p.push(FindParam::Text(modes.unwrap_or_default()));
                (ObjectKind::Procedure, FIND_PROCEDURE_SQL.into(), p)
            }
            ObjectDef::Trigger(d) => (ObjectKind::Trigger, FIND_TRIGGER_SQL.into(), text(&[&d.schema, &d.table, &d.name])),
            ObjectDef::Type(d) => {
                let typtype = match d.body {
                    TypeBody::Enum { .. } => "e",
                    TypeBody::Composite { .. } => "c",
                    TypeBody::Range { .. } => "r",
                };
                (ObjectKind::Type, FIND_TYPE_SQL.into(), text(&[&d.schema, &d.name, &typtype.to_string()]))
            }
            ObjectDef::Domain(d) => (ObjectKind::Domain, FIND_TYPE_SQL.into(), text(&[&d.schema, &d.name, &"d".to_string()])),
        };
        let row = self
            .mgr
            .run(server, target, move |c| {
                let (sql, params) = (sql.clone(), params.clone());
                async move {
                    let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params.iter().map(FindParam::as_sql).collect();
                    c.client.query_opt(sql.as_ref(), &refs).await
                }
            })
            .await
            .ok()
            .flatten()?;
        Some(ObjectRef { kind, oid: row.get::<_, u32>(0) })
    }

    /// Tanımı yükler (Database/Role için `database_oid` yok sayılır; diğerlerinde `None` → `Invalid`).
    pub async fn load(&self, server: Uuid, database_oid: impl Into<Option<u32>> + Send, obj: ObjectRef) -> Result<ObjectDef> {
        load::load(&self.mgr, server, database_oid, obj).await
    }

    /// Nesnenin etkin ACL'si (Plan 20; Database için `database_oid` yok sayılır; diğerlerinde `None` → `Invalid`).
    pub async fn load_acl(&self, server: Uuid, database_oid: impl Into<Option<u32>> + Send, obj: ObjectRef) -> Result<AclDef> {
        let r = crate::load_acl::load_acl(&self.mgr, server, database_oid, obj).await;
        tracing::debug!(op = "load_acl", kind = ?obj.kind, outcome = if r.is_ok() { "ok" } else { "error" }, "DDL yetki yüklemesi");
        r
    }

    /// Bakım hedefi (Plan 24; her türde veritabanı kapsamlı: Database türünde de bağlı veritabanı).
    pub async fn load_maintenance_target(&self, server: Uuid, database_oid: u32, obj: ObjectRef) -> Result<MaintenanceTarget> {
        let r = crate::load_maintenance::load_maintenance_target(&self.mgr, server, database_oid, obj).await;
        tracing::debug!(op = "load_maintenance_target", kind = ?obj.kind, outcome = if r.is_ok() { "ok" } else { "error" }, "DDL bakım hedefi yüklemesi");
        r
    }

    /// Süren uygulamanın bakım ilerlemesi (Plan 24): yalnız bu `apply_id`'nin uygulama bağlantısının pid'i için,
    /// havuzdaki katalog bağlantısıyla (uygulama bağlantısı kullanılmaz). Bilinmeyen/bitmiş kimlik ya da pid henüz
    /// okunmadıysa bağlantı açmadan `Ok(None)`; görünümde satır yoksa (ilerleme bildirmeyen aşama/işlem) `Ok(None)`.
    /// Sorgu sürerken uygulama biterse (yuva düştüyse ya da pid değiştiyse) sonuç atılır: başka oturumun satırı
    /// gösterilmez. Ad/aşama metni günlüğe yazılmaz.
    pub async fn maintenance_progress(&self, server: Uuid, database_oid: u32, apply_id: Uuid) -> Result<Option<MaintenanceProgress>> {
        let Some(pid) = self.cancels.backend_pid(apply_id) else { return Ok(None) };
        let sql = progress_sql(load::server_version(&self.mgr, server).await?);
        let row = self
            .mgr
            .run(server, Target::Database(database_oid), move |c| {
                let sql = sql.clone();
                async move { c.client.query_opt(sql.as_str(), &[&pid]).await }
            })
            .await;
        tracing::debug!(op = "maintenance_progress", outcome = if row.is_ok() { "ok" } else { "error" }, "DDL bakım ilerlemesi");
        let Some(row) = row? else { return Ok(None) };
        if self.cancels.backend_pid(apply_id) != Some(pid) {
            return Ok(None);
        }
        // Paniksiz okuma: SQL `phase`'i `coalesce` ile doldurur; beklenmeyen tür/NULL yine de belirsiz ilerlemeye düşer.
        let opt_i64 = |c: &str| row.try_get::<_, Option<i64>>(c).ok().flatten();
        let (done, total) = progress_counts(opt_i64("done"), opt_i64("total"));
        let phase = row.try_get::<_, Option<String>>("phase").ok().flatten().unwrap_or_else(|| "unknown".into());
        let unit = row.try_get::<_, Option<String>>("unit").ok().flatten();
        Ok(Some(MaintenanceProgress { phase, done, total, unit }))
    }

    /// Bir rolün (genel ya da şema kapsamlı) varsayılan yetkileri (Plan 20; veritabanı kapsamlı).
    pub async fn load_default_acl(&self, server: Uuid, database_oid: u32, scope: DefaultAclScope) -> Result<DefaultAclDef> {
        let r = crate::load_acl::load_default_acl(&self.mgr, server, database_oid, scope).await;
        tracing::debug!(op = "load_default_acl", outcome = if r.is_ok() { "ok" } else { "error" }, "DDL varsayılan yetki yüklemesi");
        r
    }

    /// Açılır liste; `database_oid = None` yalnızca sunucu genelindeki isteklerde (`LookupRequest::server_wide`).
    pub async fn lookup(&self, server: Uuid, database_oid: impl Into<Option<u32>> + Send, req: LookupRequest) -> Result<Vec<LookupItem>> {
        lookup::lookup(&self.mgr, server, database_oid, req).await
    }

    /// "Yeni …" diyaloğunun önceden doldurulmuş tanımı. Şema/tablo adları oid'den katalogdan çözülür.
    /// Geçersiz bağlam (sequence/tablo/görünüm için şema yok ya da tablo var, indeks için tablo yok, şema/eklenti için
    /// tablo, veritabanı/rol için herhangi bir bağlam) → `Invalid`; veritabanı kapsamlı türde `database_oid` yok →
    /// `Invalid(NEEDS_DATABASE)`.
    ///
    /// Plan 18: Function/Procedure/Type/Domain şema altında; Trigger tablo altında (`table_oid`; şema/tablo adı
    /// katalogdan). `preset` yalnızca Function'da (`TriggerFunction` → dönüş `trigger`, gövde `RETURN NEW` şablonu);
    /// başka türde `Invalid`.
    pub async fn defaults(
        &self,
        server: Uuid,
        database_oid: impl Into<Option<u32>> + Send,
        kind: ObjectKind,
        ctx: CreateContext,
        preset: Option<CreatePreset>,
    ) -> Result<ObjectDef> {
        let invalid = |m: &str| Err(Error::Invalid(m.to_string()));
        if preset.is_some() && kind != ObjectKind::Function {
            return invalid("Bu ön ayar yalnızca fonksiyon için geçerlidir.");
        }
        // Sunucu kapsamlı türlerde kullanılmaz (bakım veritabanı); diğerlerinde zorunlu.
        let database_oid = match target_for(kind, database_oid.into())? {
            Target::Database(db) => db,
            Target::Maintenance => 0,
        };
        Ok(match kind {
            ObjectKind::Schema | ObjectKind::Extension if ctx.table_oid.is_some() => {
                return invalid("Bu nesne tablo bağlamında oluşturulamaz.");
            }
            ObjectKind::Schema => ObjectDef::Schema(SchemaDef { name: String::new(), owner: None, comment: None }),
            ObjectKind::Extension => {
                ObjectDef::Extension(ExtensionDef { name: String::new(), schema: None, version: None, cascade: false })
            }
            ObjectKind::Sequence => {
                let (Some(schema_oid), None) = (ctx.schema_oid, ctx.table_oid) else {
                    return invalid("Sequence yalnızca bir şema altında oluşturulabilir.");
                };
                let schema = self.schema_name(server, database_oid, schema_oid).await?;
                ObjectDef::Sequence(SequenceDef {
                    schema,
                    name: String::new(),
                    owner: None,
                    data_type: SeqType::Bigint,
                    increment: 1,
                    min_value: None,
                    max_value: None,
                    start: 1,
                    cache: 1,
                    cycle: false,
                    owned_by: None,
                    comment: None,
                })
            }
            ObjectKind::Table => {
                let (Some(schema_oid), None) = (ctx.schema_oid, ctx.table_oid) else {
                    return invalid("Tablo yalnızca bir şema altında oluşturulabilir.");
                };
                let schema = self.schema_name(server, database_oid, schema_oid).await?;
                ObjectDef::Table(TableDef {
                    schema,
                    name: String::new(),
                    owner: None,
                    tablespace: None,
                    persistence: Persistence::Permanent,
                    columns: vec![],
                    constraints: vec![],
                    partition_by: None,
                    fillfactor: None,
                    comment: None,
                    partition_text: None,
                    is_partition: false,
                    inherits: false,
                    unsupported: vec![],
                })
            }
            ObjectKind::Index => {
                let Some(table_oid) = ctx.table_oid else {
                    return invalid("İndeks yalnızca bir tablo altında oluşturulabilir.");
                };
                let (schema, table) = self.table_name(server, database_oid, table_oid).await?;
                ObjectDef::Index(IndexDef {
                    schema,
                    table,
                    name: String::new(),
                    method: "btree".into(),
                    unique: false,
                    columns: vec![],
                    include: vec![],
                    where_clause: None,
                    tablespace: None,
                    fillfactor: None,
                    comment: None,
                    constraint: None,
                    valid: true,
                })
            }
            ObjectKind::View | ObjectKind::MaterializedView => {
                let (Some(schema_oid), None) = (ctx.schema_oid, ctx.table_oid) else {
                    return invalid("Görünüm yalnızca bir şema altında oluşturulabilir.");
                };
                let schema = self.schema_name(server, database_oid, schema_oid).await?;
                if kind == ObjectKind::View {
                    ObjectDef::View(ViewDef {
                        schema,
                        name: String::new(),
                        owner: None,
                        query: String::new(),
                        check_option: None,
                        security_barrier: false,
                        security_invoker: false,
                        comment: None,
                        recreate: false,
                        columns: vec![],
                        unsupported: vec![],
                    })
                } else {
                    ObjectDef::MaterializedView(MatViewDef {
                        schema,
                        name: String::new(),
                        owner: None,
                        query: String::new(),
                        tablespace: None,
                        fillfactor: None,
                        with_data: true,
                        comment: None,
                        refresh: None,
                        columns: vec![],
                        unsupported: vec![],
                    })
                }
            }
            ObjectKind::Database | ObjectKind::Role if ctx != CreateContext::default() => {
                return invalid("Veritabanı ve rol sunucu düzeyinde oluşturulur (şema/tablo bağlamı olmadan).");
            }
            ObjectKind::Database => ObjectDef::Database(DatabaseDef {
                name: String::new(),
                owner: None,
                template: None,
                encoding: None,
                locale_provider: None,
                lc_collate: None,
                lc_ctype: None,
                provider_locale: None,
                tablespace: None,
                connection_limit: -1,
                is_template: false,
                allow_connections: true,
                settings: vec![],
                comment: None,
            }),
            ObjectKind::Role => ObjectDef::Role(RoleDef {
                name: String::new(),
                login: false,
                superuser: false,
                createdb: false,
                createrole: false,
                inherit: true,
                replication: false,
                bypassrls: false,
                connection_limit: -1,
                valid_until: None,
                password: PasswordChange::Keep,
                member_of: vec![],
                settings: vec![],
                comment: None,
                unsupported: vec![],
            }),
            ObjectKind::Function | ObjectKind::Procedure | ObjectKind::Type | ObjectKind::Domain => {
                let (Some(schema_oid), None) = (ctx.schema_oid, ctx.table_oid) else {
                    return invalid("Bu nesne yalnızca bir şema altında oluşturulabilir.");
                };
                let schema = self.schema_name(server, database_oid, schema_oid).await?;
                match kind {
                    ObjectKind::Function | ObjectKind::Procedure => {
                        let trigger = preset == Some(CreatePreset::TriggerFunction);
                        let def = FunctionDef {
                            schema,
                            name: String::new(),
                            owner: None,
                            comment: None,
                            language: "plpgsql".into(),
                            args: vec![],
                            returns: match (kind, trigger) {
                                (ObjectKind::Procedure, _) => None,
                                (_, true) => Some(FunctionReturn::Trigger),
                                (_, false) => Some(FunctionReturn::Void),
                            },
                            body: if trigger { TRIGGER_FUNCTION_BODY } else { FUNCTION_BODY }.into(),
                            volatility: Volatility::Volatile,
                            strict: false,
                            security_definer: false,
                            leakproof: false,
                            parallel: Parallel::Unsafe,
                            cost: None,
                            rows: None,
                            settings: vec![],
                            recreate: false,
                            editable: true,
                            unsupported: vec![],
                        };
                        if kind == ObjectKind::Procedure { ObjectDef::Procedure(def) } else { ObjectDef::Function(def) }
                    }
                    ObjectKind::Type => ObjectDef::Type(TypeDef {
                        schema,
                        name: String::new(),
                        owner: None,
                        comment: None,
                        body: TypeBody::Enum { values: vec![] },
                        editable: true,
                        unsupported: vec![],
                    }),
                    _ => ObjectDef::Domain(DomainDef {
                        schema,
                        name: String::new(),
                        owner: None,
                        comment: None,
                        base_type: "text".into(),
                        collation: None,
                        default: None,
                        not_null: false,
                        constraints: vec![],
                        editable: true,
                        unsupported: vec![],
                    }),
                }
            }
            ObjectKind::Trigger => {
                let Some(table_oid) = ctx.table_oid else {
                    return invalid("Trigger yalnızca bir tablo altında oluşturulabilir.");
                };
                let (schema, table) = self.trigger_table_name(server, database_oid, table_oid).await?;
                ObjectDef::Trigger(TriggerDef {
                    schema,
                    table,
                    name: String::new(),
                    timing: TriggerTiming::After,
                    events: TriggerEvents { insert: true, update: false, update_columns: vec![], delete: false, truncate: false },
                    level: TriggerLevel::Row,
                    when: None,
                    function_schema: String::new(),
                    function_name: String::new(),
                    args: vec![],
                    old_table: None,
                    new_table: None,
                    constraint: false,
                    deferrable: false,
                    initially_deferred: false,
                    enabled: TriggerEnabled::Origin,
                    comment: None,
                    editable: true,
                    unsupported: vec![],
                })
            }
        })
    }

    async fn schema_name(&self, server: Uuid, database_oid: u32, oid: u32) -> Result<String> {
        let row = self
            .mgr
            .run(server, Target::Database(database_oid), move |c| async move { c.client.query_opt(SCHEMA_NAME_SQL, &[&oid]).await })
            .await?;
        row.map(|r| r.get("name")).ok_or_else(|| Error::NotFound("şema".into()))
    }

    async fn table_name(&self, server: Uuid, database_oid: u32, oid: u32) -> Result<(String, String)> {
        let row = self
            .mgr
            .run(server, Target::Database(database_oid), move |c| async move { c.client.query_opt(TABLE_NAME_SQL, &[&oid]).await })
            .await?;
        row.map(|r| (r.get("schema"), r.get("name"))).ok_or_else(|| Error::NotFound("tablo".into()))
    }

    /// Trigger'ın tablosu: yalnızca sıradan ve bölümlenmiş tablo (görünüm/MV/yabancı tablo trigger'ı kapsam dışı).
    async fn trigger_table_name(&self, server: Uuid, database_oid: u32, oid: u32) -> Result<(String, String)> {
        let row = self
            .mgr
            .run(server, Target::Database(database_oid), move |c| async move { c.client.query_opt(TRIGGER_TABLE_SQL, &[&oid]).await })
            .await?;
        row.map(|r| (r.get("schema"), r.get("name"))).ok_or_else(|| Error::NotFound("tablo".into()))
    }
}

/// (4) Sunucudaki güncel tanımla karşılaştırılacak istemci tanımı ve onu üreten yükleyicinin girdisi.
enum Expected<'a> {
    /// Alter/Drop: `load::load(object)`.
    Def(ObjectRef, &'a ObjectDef),
    /// Privileges: `load_acl(object)`.
    Acl(ObjectRef, &'a AclDef),
    /// DefaultPrivileges: `load_default_acl(scope)`.
    DefaultAcl(&'a DefaultAclScope, &'a DefaultAclDef),
    /// Maintenance: `load_maintenance_target(object)` (Plan 24).
    Maintenance(ObjectRef, &'a MaintenanceTarget),
}

impl Expected<'_> {
    /// Değiştirilen nesne (varsayılan yetkilerde yok).
    fn object(&self) -> Option<ObjectRef> {
        match self {
            Expected::Def(o, _) | Expected::Acl(o, _) | Expected::Maintenance(o, _) => Some(*o),
            Expected::DefaultAcl(..) => None,
        }
    }
}

/// Nesne denetiminin girdisi; `Create` için yok.
fn object_check(req: &PlanRequest) -> Option<Expected<'_>> {
    match req {
        PlanRequest::Create { .. } => None,
        PlanRequest::Alter { object, old, .. } => Some(Expected::Def(*object, old)),
        PlanRequest::Drop { object, def, .. } => Some(Expected::Def(*object, def)),
        PlanRequest::Privileges { object, old, .. } => Some(Expected::Acl(*object, old)),
        PlanRequest::DefaultPrivileges { scope, old, .. } => Some(Expected::DefaultAcl(scope, old)),
        PlanRequest::Maintenance { object, target, .. } => Some(Expected::Maintenance(*object, target)),
    }
}

/// İstekten saf plan üretimi (önizleme ve uygulama aynı yolu kullanır; hash yalnızca ifadelerden).
pub(crate) fn plan_view(req: &PlanRequest, version_num: u32) -> PlanView {
    let ctx = |concurrently| BuildCtx { version_num, concurrently };
    PlanView::from_build(match req {
        PlanRequest::Privileges { old, new, cascade, .. } => build::acl::privileges(&ctx(false), old, new, *cascade),
        PlanRequest::DefaultPrivileges { old, new, .. } => build::default_acl::default_privileges(&ctx(false), old, new),
        PlanRequest::Create { def, concurrently } => build::create(def, &ctx(*concurrently)),
        PlanRequest::Alter { old, new, .. } => build::alter(old, new, &ctx(false)),
        PlanRequest::Drop { def, cascade, force, .. } => {
            build::drop_with(def, build::DropOptions { cascade: *cascade, force: *force }, &ctx(false))
        }
        // REINDEX CONCURRENTLY `ReindexOptions`'tan gelir (`ctx.concurrently` kullanılmaz).
        PlanRequest::Maintenance { target, op, .. } => build::maintenance::maintenance(&ctx(false), target, op),
    })
}

// ---- bakım ilerlemesi (Plan 24) ----

const PROGRESS_VACUUM_SQL: &str = include_str!("sql/maintenance_progress_vacuum.sql");
const PROGRESS_CLUSTER_SQL: &str = include_str!("sql/maintenance_progress_cluster.sql");
const PROGRESS_CREATE_INDEX_SQL: &str = include_str!("sql/maintenance_progress_create_index.sql");
const PROGRESS_ANALYZE_SQL: &str = include_str!("sql/maintenance_progress_analyze.sql");

/// İlerleme sorgusu: sürümde bulunan `pg_stat_progress_*` görünümleri tek gidiş-dönüşte (`UNION ALL`, `$1` = pid).
/// vacuum 9.6+, cluster ve create_index 12+, analyze 13+; PG 10–11'de yalnız vacuum görünümü sorgulanır (diğerleri
/// yoktur, sorgu hata verirdi). Bir pid aynı anda en çok bir görünümde görünür (VACUUM FULL yalnız cluster'da).
fn progress_sql(version_num: u32) -> String {
    let mut parts = vec![PROGRESS_VACUUM_SQL];
    if version_num >= 120000 {
        parts.extend([PROGRESS_CLUSTER_SQL, PROGRESS_CREATE_INDEX_SQL]);
    }
    if version_num >= 130000 {
        parts.push(PROGRESS_ANALYZE_SQL);
    }
    // Satır sonu: dosyaların son satırı yorumsuzdur ama yine de `)` yeni satırda (yorum parantezi yutmasın).
    let body: Vec<String> = parts.iter().map(|p| format!("({}\n)", p.trim_end())).collect();
    format!("{} LIMIT 1", body.join("\nUNION ALL\n"))
}

/// Sayaçlar `u64`'e; negatif (beklenmez) ya da boş → `None`.
fn progress_counts(done: Option<i64>, total: Option<i64>) -> (Option<u64>, Option<u64>) {
    (done.and_then(|v| u64::try_from(v).ok()), total.and_then(|v| u64::try_from(v).ok()))
}

/// (8) `lock_timeout` sınırı (`SessionProfile::Ddl` doğrulamasıyla aynı; bağlantıdan önce).
pub(crate) fn check_lock_timeout(ms: u32) -> Result<()> {
    if ms > MAX_STATEMENT_TIMEOUT_MS {
        return Err(Error::Invalid(format!("lock_timeout en fazla {MAX_STATEMENT_TIMEOUT_MS} ms olabilir.")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error_codes::NEEDS_DATABASE;
    use serde_json::json;

    fn schema(name: &str) -> ObjectDef {
        ObjectDef::Schema(SchemaDef { name: name.into(), owner: None, comment: None })
    }

    fn acl_def(kind: ObjectKind) -> AclDef {
        AclDef {
            kind,
            schema: None,
            name: "d".into(),
            signature: None,
            owner: "o".into(),
            viewer: "o".into(),
            explicit: false,
            grants: vec![],
            foreign: vec![],
            columns: vec![],
            editable: true,
            unsupported: vec![],
            available: vec![],
        }
    }

    fn dacl_def(role: &str) -> DefaultAclDef {
        DefaultAclDef { role: role.into(), schema: None, viewer: role.into(), sections: vec![], editable: true, unsupported: vec![] }
    }

    fn privileges_req(object_kind: ObjectKind, old: AclDef, new: AclDef) -> PlanRequest {
        PlanRequest::Privileges { object: ObjectRef { kind: object_kind, oid: 1 }, old, new, cascade: false }
    }

    #[test]
    fn privilege_requests_op_kind_plan_and_object_check() {
        let p = privileges_req(ObjectKind::Database, acl_def(ObjectKind::Database), acl_def(ObjectKind::Database));
        assert_eq!((p.op(), p.kind()), ("privileges", ObjectKind::Database));
        let scope = DefaultAclScope { role: None, schema_oid: None };
        let d = PlanRequest::DefaultPrivileges { scope: scope.clone(), old: dacl_def("r"), new: dacl_def("r") };
        assert_eq!((d.op(), d.kind()), ("default_privileges", ObjectKind::Schema));
        for r in [&p, &d] {
            assert!(read_only_refusal(r).is_none() && request_shape_refusal(r).is_none());
            assert_eq!(HintCtx::of(r), HintCtx::default());
        }
        // Nesne denetimi yetki isteklerinde ilgili yükleyicinin tanımıyla yapılır; `created`/üyelik denetimi yok.
        assert!(matches!(object_check(&p), Some(Expected::Acl(o, old)) if o.kind == ObjectKind::Database && *old == acl_def(ObjectKind::Database)));
        assert!(matches!(object_check(&d), Some(Expected::DefaultAcl(s, old)) if *s == scope && *old == dacl_def("r")));
        assert_eq!(object_check(&p).and_then(|e| e.object()).map(|o| o.oid), Some(1));
        assert_eq!(object_check(&d).and_then(|e| e.object()), None);
        // Nesne yetkileri üreticiye bağlı: fark yok → boş plan; yeni alıcı → GRANT; kimlik değişimi → `object`.
        let v = plan_view(&p, 170000);
        assert!(v.statements.is_empty() && v.errors.is_empty() && v.empty, "{v:?}");
        use crate::acl::{AclGrant, Privilege, PrivilegeGrant};
        let mut granted = acl_def(ObjectKind::Database);
        granted.grants.push(AclGrant { grantee: None, privileges: vec![PrivilegeGrant { privilege: Privilege::Connect, grant_option: false }] });
        let g = PlanRequest::Privileges { object: ObjectRef { kind: ObjectKind::Database, oid: 1 }, old: acl_def(ObjectKind::Database), new: granted, cascade: true };
        let v = plan_view(&g, 170000);
        assert_eq!(v.statements.iter().map(|s| s.sql.as_str()).collect::<Vec<_>>(), ["GRANT CONNECT ON DATABASE d TO PUBLIC"]);
        let renamed = privileges_req(ObjectKind::Database, acl_def(ObjectKind::Database), AclDef { name: "e".into(), ..acl_def(ObjectKind::Database) });
        assert_eq!(plan_view(&renamed, 170000).errors[0].path, "object");
        // Bölümsüz varsayılan yetki tanımı üreticide alan hatasıdır.
        let v = plan_view(&d, 170000);
        assert!(v.statements.is_empty() && v.errors.len() == 1);
        // Uygulama sonrası denetim: istenen `grants` kanonik biçimde saklanır.
        assert!(matches!(VerifyAcl::of(&g), Some(VerifyAcl::Object(o, want)) if o.oid == 1 && want.len() == 1));
        assert!(matches!(VerifyAcl::of(&d), Some(VerifyAcl::Default(s, want)) if s == scope && want.is_empty()));
        assert!(VerifyAcl::of(&PlanRequest::Create { def: schema("a"), concurrently: false }).is_none());
    }

    /// Yetki isteklerinin kapıları bağlantı açmadan çalışır (bağlantısız yönetici: kapı geçilirse `not_connected`).
    #[tokio::test]
    async fn privilege_gates_refuse_before_connecting() {
        use crate::error_codes::ddl_payload;
        use pgt_core::config::ServerConfig;
        use pgt_core::registry::Registry;
        let run = |locked: bool, req: PlanRequest, db: Option<u32>| async move {
            let dir = tempfile::tempdir().unwrap();
            let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
            let mut config = ServerConfig::new("x", "127.0.0.1", 1, "u");
            config.read_only_lock = locked;
            let server = config.id;
            registry.upsert(config).unwrap();
            let dm = DdlManager::new(Arc::new(ConnectionManager::new()), Arc::new(RwLock::new(registry)));
            let o = ApplyOptions {
                apply_id: Uuid::new_v4(),
                lock_timeout_ms: 5000,
                expected_hash: String::new(),
                confirmed: true,
                confirmed_production: true,
                password: None,
            };
            dm.apply(server, db, &req, o).await
        };
        let message = |r: Result<ApplyOutcome>| match r {
            Err(Error::Invalid(m)) => m,
            other => panic!("Invalid bekleniyordu: {other:?}"),
        };
        let schema_acl = || privileges_req(ObjectKind::Schema, acl_def(ObjectKind::Schema), acl_def(ObjectKind::Schema));
        let scope = |role: Option<&str>| DefaultAclScope { role: role.map(Into::into), schema_oid: None };
        let default_req = |role: Option<&str>| PlanRequest::DefaultPrivileges { scope: scope(role), old: dacl_def("r"), new: dacl_def("r") };
        // Kilit her şeyden önce.
        for r in [schema_acl(), default_req(None)] {
            assert_eq!(message(run(true, r, Some(1)).await), LOCKED_MESSAGE);
        }
        // Karar 1: yetkisi olmayan tür.
        let index = privileges_req(ObjectKind::Index, acl_def(ObjectKind::Index), acl_def(ObjectKind::Index));
        assert_eq!(message(run(false, index, Some(1)).await), crate::load_acl::UNSUPPORTED_KIND_MSG);
        // Kimlik uyuşmazlıkları: `object.kind ≠ old.kind`; kapsam rolü ≠ `old.role`.
        let mismatched = privileges_req(ObjectKind::Schema, acl_def(ObjectKind::Table), acl_def(ObjectKind::Table));
        assert_eq!(message(run(false, mismatched, Some(1)).await), INVALID_DEF);
        assert_eq!(message(run(false, default_req(Some("baska")), Some(1)).await), INVALID_DEF);
        // Salt okunur ACL: genel gerekçe, ad/değer yok.
        let ro = AclDef { editable: false, unsupported: vec!["yetki ZZ_GIZLI".into()], ..acl_def(ObjectKind::Schema) };
        let m = message(run(false, privileges_req(ObjectKind::Schema, ro.clone(), ro), Some(1)).await);
        assert!(m.starts_with(READ_ONLY_PREFIX) && !m.contains("ZZ_GIZLI"), "{m}");
        // Varsayılan yetkiler veritabanı kapsamlıdır.
        assert_eq!(message(run(false, default_req(None), None).await), NEEDS_DATABASE);
        // Geçerli istekler kapıları geçer (geçici kapı yok): bağlantısız sunucuda sürüm okunamaz.
        for (r, db) in [(default_req(None), Some(1)), (default_req(Some("r")), Some(1)), (schema_acl(), Some(1))] {
            let e = run(false, r, db).await.unwrap_err();
            assert_eq!(ddl_payload(&e).code, "not_connected", "{e:?}");
        }
        // Database ACL bakım bağlantısında: `database_oid` gerekmez.
        let dbacl = privileges_req(ObjectKind::Database, acl_def(ObjectKind::Database), acl_def(ObjectKind::Database));
        assert_eq!(ddl_payload(&run(false, dbacl, None).await.unwrap_err()).code, "not_connected");
    }

    #[test]
    fn acl_read_only_gate_uses_first_real_reason() {
        let ro = AclDef { editable: false, unsupported: vec!["imza".into(), "yetki X".into()], ..acl_def(ObjectKind::Function) };
        let r = privileges_req(ObjectKind::Function, ro.clone(), ro);
        assert_eq!(read_only_refusal(&r).unwrap(), format!("{READ_ONLY_PREFIX}{}", read_only_reason(Some("imza"))));
        // Varsayılan yetkiler: LARGE OBJECTS bilgi notu gerekçe sayılmaz; düzenlenebilir tanım geçer.
        let lo = crate::load_acl::LARGE_OBJECTS_REASON.to_string();
        let mut d = dacl_def("r");
        d.unsupported = vec![lo.clone()];
        let ok = PlanRequest::DefaultPrivileges { scope: DefaultAclScope { role: None, schema_oid: None }, old: d.clone(), new: d.clone() };
        assert_eq!(read_only_refusal(&ok), None);
        d.editable = false;
        d.unsupported = vec![lo, crate::load_acl::DEFAULT_GRANTOR_REASON.into()];
        let ro = PlanRequest::DefaultPrivileges { scope: DefaultAclScope { role: None, schema_oid: None }, old: d.clone(), new: d };
        assert_eq!(
            read_only_refusal(&ro).unwrap(),
            format!("{READ_ONLY_PREFIX}{}", read_only_reason(Some(crate::load_acl::DEFAULT_GRANTOR_REASON)))
        );
        // Yetki isteği bakım veritabanı korumasına girmez (yetki değişimi veritabanını kapatmaz; Karar 9 yalnız uyarı).
        let dbacl = AclDef { name: "postgres".into(), ..acl_def(ObjectKind::Database) };
        let r = privileges_req(ObjectKind::Database, dbacl.clone(), dbacl);
        assert_eq!(maintenance_refusal(&r, "postgres"), None);
        assert_eq!(database_to_release(&r), None);
    }

    #[test]
    fn privilege_hints_follow_op() {
        let st = |sql: &str| Statement { sql: sql.into(), risk: Risk::Safe, transactional: true, note: None, redacted: false };
        let revoke = st("REVOKE SELECT ON TABLE public.t FROM r");
        let adp = st("ALTER DEFAULT PRIVILEGES FOR ROLE r GRANT SELECT ON TABLES TO x");
        let hint = |kind, op: &str, s: &Statement, code: &str, pg: Option<&str>| failure_payload_for(kind, op, Some(s), 0, &q(code, pg)).hint;
        // 2BP01: PG "Use CASCADE…" ipucunun yerine arayüzdeki karşılık.
        for pg in [None, Some("Use CASCADE to revoke them too.")] {
            assert_eq!(hint(ObjectKind::Table, "privileges", &revoke, "2BP01", pg).as_deref(), Some(ACL_DEPENDENTS_HINT));
        }
        assert_eq!(hint(ObjectKind::Table, "privileges", &revoke, "42501", None).as_deref(), Some(ACL_PRIVILEGE_HINT));
        assert_eq!(hint(ObjectKind::Database, "privileges", &revoke, "42501", None).as_deref(), Some(ACL_PRIVILEGE_HINT));
        assert_eq!(hint(ObjectKind::Schema, "default_privileges", &adp, "42501", None).as_deref(), Some(DEFAULT_ACL_PRIVILEGE_HINT));
        for (kind, op, s) in [(ObjectKind::Table, "privileges", &revoke), (ObjectKind::Schema, "default_privileges", &adp)] {
            assert_eq!(hint(kind, op, s, "42704", None).as_deref(), Some(ACL_ROLE_MISSING_HINT));
            // PG'nin kendi ipucu (2BP01 dışında) korunur.
            assert_eq!(hint(kind, op, s, "42501", Some("pg")).as_deref(), Some("pg"));
        }
        // 42704 yalnız GRANT/REVOKE/ALTER DEFAULT PRIVILEGES ifadesinde ve Type/Domain dışında rol eksikliğidir;
        // Type/Domain'de nesnenin kendisi kaybolmuş olabilir (`type … does not exist` da 42704).
        for kind in [ObjectKind::Type, ObjectKind::Domain] {
            assert_eq!(hint(kind, "privileges", &st("REVOKE USAGE ON TYPE public.e FROM r"), "42704", None), None, "{kind:?}");
        }
        assert_eq!(hint(ObjectKind::Table, "privileges", &st("SELECT 1"), "42704", None), None);
        assert_eq!(hint(ObjectKind::Function, "privileges", &st("GRANT EXECUTE ON FUNCTION public.f() TO r"), "42704", None).as_deref(), Some(ACL_ROLE_MISSING_HINT));
        // 42809 (Function ↔ Procedure) yeniden kurma ipucu yetki işlemlerinde verilmez.
        for (kind, op) in [(ObjectKind::Function, "privileges"), (ObjectKind::Procedure, "privileges"), (ObjectKind::Function, "default_privileges")] {
            assert_eq!(hint(kind, op, &st("GRANT EXECUTE ON FUNCTION public.f() TO r"), "42809", None), None, "{kind:?} {op}");
        }
        // Genel 42501 diğer işlemlerde değişmez; 42704 başka işlemde ipucu almaz.
        let alter = st("ALTER VIEW v OWNER TO r");
        assert_eq!(hint(ObjectKind::View, "alter", &alter, "42501", None).as_deref(), Some(PRIVILEGE_HINT));
        assert_eq!(hint(ObjectKind::Table, "alter", &alter, "42704", None), None);
        assert_eq!(hint(ObjectKind::Table, "drop", &st("DROP TABLE t"), "2BP01", Some("pg")).as_deref(), Some("pg"));
        // İpuçları ad/değer içermez.
        for h in [ACL_DEPENDENTS_HINT, ACL_PRIVILEGE_HINT, DEFAULT_ACL_PRIVILEGE_HINT, ACL_ROLE_MISSING_HINT, ACL_WARNING] {
            assert!(!h.is_empty() && !h.contains("public.t"));
        }
    }

    #[test]
    fn acl_verification_compares_canonical_grants() {
        use crate::acl::{AclGrant, DefaultAclObjectType, DefaultAclSection, Privilege::*, PrivilegeGrant};
        let pg = |p, g| PrivilegeGrant { privilege: p, grant_option: g };
        let row = |who: Option<&str>, ps: Vec<PrivilegeGrant>| AclGrant { grantee: who.map(Into::into), privileges: ps };
        // İstenen: sırasız, boş satırlı (alıcının tüm yetkileri kaldırıldı).
        let mut new = acl_def(ObjectKind::Table);
        new.grants = vec![row(Some("b"), vec![pg(Insert, false), pg(Select, true)]), row(Some("a"), vec![]), row(None, vec![pg(Select, false)])];
        let req = privileges_req(ObjectKind::Table, acl_def(ObjectKind::Table), new);
        let Some(VerifyAcl::Object(_, want)) = VerifyAcl::of(&req) else { panic!() };
        let mut loaded = acl_def(ObjectKind::Table);
        loaded.grants = vec![row(None, vec![pg(Select, false)]), row(Some("b"), vec![pg(Select, true), pg(Insert, false)])];
        assert!(!acl_grants_differ(&want, &loaded));
        // Sahip olmayan bağlı rolün GRANT'ı `foreign`'a düşer → `grants` farklı.
        loaded.grants.pop();
        loaded.foreign = vec![crate::acl::ForeignGrant { grantor: "c".into(), grantee: Some("b".into()), privilege: Select, grant_option: true }];
        assert!(acl_grants_differ(&want, &loaded));
        // Varsayılan yetkiler: bölüm bölüm.
        let sec = |t, grants| DefaultAclSection { object_type: t, grants, baseline: vec![], available: vec![] };
        let mut new = dacl_def("r");
        new.sections = vec![sec(DefaultAclObjectType::Tables, vec![row(Some("x"), vec![pg(Insert, false), pg(Select, false)]), row(Some("y"), vec![])])];
        let req = PlanRequest::DefaultPrivileges { scope: DefaultAclScope { role: None, schema_oid: None }, old: dacl_def("r"), new };
        let Some(VerifyAcl::Default(_, want)) = VerifyAcl::of(&req) else { panic!() };
        let mut loaded = dacl_def("r");
        loaded.sections = vec![sec(DefaultAclObjectType::Tables, vec![row(Some("x"), vec![pg(Select, false), pg(Insert, false)])])];
        assert!(!default_acl_grants_differ(&want, &loaded));
        loaded.sections[0].grants.clear();
        assert!(default_acl_grants_differ(&want, &loaded));
    }

    #[test]
    fn plan_view_builds_default_privileges() {
        use crate::acl::{AclGrant, DefaultAclSection, Privilege, PrivilegeGrant};
        use crate::acl::DefaultAclObjectType::Schemas;
        let mk = |grants: Vec<AclGrant>| DefaultAclDef {
            role: "r".into(),
            schema: None,
            viewer: "r".into(),
            sections: crate::acl::default_sections(false)
                .iter()
                .map(|&t| DefaultAclSection { object_type: t, grants: if t == Schemas { grants.clone() } else { vec![] }, baseline: vec![], available: vec![] })
                .collect(),
            editable: true,
            unsupported: vec![],
        };
        let new_grants = vec![AclGrant { grantee: Some("x".into()), privileges: vec![PrivilegeGrant { privilege: Privilege::Usage, grant_option: false }] }];
        let req = PlanRequest::DefaultPrivileges {
            scope: DefaultAclScope { role: None, schema_oid: None },
            old: mk(vec![]),
            new: mk(new_grants),
        };
        let view = plan_view(&req, 160000);
        assert_eq!(view.statements.len(), 1);
        assert_eq!(view.statements[0].sql, "ALTER DEFAULT PRIVILEGES FOR ROLE r GRANT USAGE ON SCHEMAS TO x");
        assert_eq!(view.warnings.len(), 1);
    }

    #[test]
    fn request_and_outcome_json_shapes() {
        let r = PlanRequest::Drop { object: ObjectRef { kind: ObjectKind::Schema, oid: 1 }, def: schema("a"), cascade: true, force: false };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["op"], "drop");
        assert_eq!(v["def"]["kind"], "schema");
        assert_eq!(v["force"], false);
        assert_eq!(serde_json::from_value::<PlanRequest>(v.clone()).unwrap(), r);
        // `force` yokken `false`.
        let mut v = v;
        v.as_object_mut().unwrap().remove("force");
        assert_eq!(serde_json::from_value::<PlanRequest>(v).unwrap(), r);
        let o = ApplyOptions {
            apply_id: Uuid::nil(),
            lock_timeout_ms: 5000,
            expected_hash: "h".into(),
            confirmed: true,
            confirmed_production: false,
            password: None,
        };
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["lockTimeoutMs"], 5000);
        assert_eq!(v["confirmedProduction"], false);
        assert_eq!(v["applyId"], Uuid::nil().to_string());
        assert!(v["password"].is_null());
        // `password` yokken `None`.
        let mut v = v;
        v.as_object_mut().unwrap().remove("password");
        assert_eq!(serde_json::from_value::<ApplyOptions>(v).unwrap(), o);
        let a = serde_json::to_value(ApplyOutcome::Applied { statements: 2, elapsed_ms: 7, created: None, warnings: None, messages: vec![] }).unwrap();
        assert_eq!(a, json!({"outcome": "applied", "statements": 2, "elapsedMs": 7, "created": null}));
        let w = ApplyOutcome::Applied { statements: 1, elapsed_ms: 1, created: None, warnings: Some(vec!["u".into()]), messages: vec![] };
        assert_eq!(serde_json::to_value(w).unwrap()["warnings"], json!(["u"]));
        let f = ApplyOutcome::Failed {
            failed_index: 1,
            committed: 0,
            sqlstate: Some("42P07".into()),
            position: Some(12),
            error: ErrorPayload { code: "query".into(), message: "m".into(), detail: None, hint: None },
            created: None,
            messages: vec![],
        };
        let f = serde_json::to_value(f).unwrap();
        assert_eq!((f["outcome"].clone(), f["failedIndex"].clone(), f["sqlstate"].clone()), (json!("failed"), json!(1), json!("42P07")));
        assert!(f.get("created").is_none(), "kısmi uygulama yoksa alan yazılmaz");
        let partial = ApplyOutcome::Failed {
            failed_index: 1,
            committed: 1,
            sqlstate: None,
            position: None,
            error: ErrorPayload { code: "query".into(), message: "m".into(), detail: None, hint: None },
            created: Some(ObjectRef { kind: ObjectKind::Database, oid: 9 }),
            messages: vec![],
        };
        assert_eq!(serde_json::to_value(partial).unwrap()["created"], json!({"kind": "database", "oid": 9}));
        let c: CreateContext = serde_json::from_value(json!({"schemaOid": 5, "tableOid": null})).unwrap();
        assert_eq!(c, CreateContext { schema_oid: Some(5), table_oid: None });
    }

    #[test]
    fn plan_view_matches_builders_and_hash() {
        let req = PlanRequest::Create { def: schema("a"), concurrently: false };
        let v = plan_view(&req, 160000);
        assert_eq!(v.statements.len(), 1);
        assert_eq!(v.hash, crate::plan_hash(&v.statements));
        let bad = plan_view(&PlanRequest::Create { def: schema(""), concurrently: false }, 160000);
        assert!(bad.empty && !bad.errors.is_empty());
        assert_eq!(bad.hash, crate::plan_hash(&[]));
        let noop = plan_view(&PlanRequest::Alter { object: ObjectRef { kind: ObjectKind::Schema, oid: 1 }, old: schema("a"), new: schema("a") }, 160000);
        assert!(noop.empty && noop.errors.is_empty());
    }

    fn table(schema: &str, name: &str) -> ObjectDef {
        ObjectDef::Table(TableDef {
            schema: schema.into(),
            name: name.into(),
            owner: None,
            tablespace: None,
            persistence: Persistence::Permanent,
            columns: vec![],
            constraints: vec![],
            partition_by: None,
            fillfactor: None,
            comment: None,
            partition_text: None,
            is_partition: false,
            inherits: false,
            unsupported: vec![],
        })
    }

    #[test]
    fn table_identity_is_schema_and_name() {
        let base = table("public", "t");
        assert!(!identity_changed(&base, &base));
        let ObjectDef::Table(mut commented) = base.clone() else { unreachable!() };
        commented.comment = Some("c".into());
        commented.persistence = Persistence::Unlogged;
        assert!(!identity_changed(&base, &ObjectDef::Table(commented)));
        assert!(identity_changed(&base, &table("public", "u")));
        assert!(identity_changed(&base, &table("other", "t")));
        // Tür uyuşmazlığı kimlik değişimi sayılmaz (nesne denetimi zaten reddeder).
        assert!(!identity_changed(&base, &schema("t")));
        // Tablo isteği yöneticinin saf yolunda tablo üreticisine ulaşır (kolonsuz create: alan hatası).
        let v = plan_view(&PlanRequest::Create { def: base, concurrently: false }, 160000);
        assert!(v.empty && v.errors.iter().any(|e| e.path == "columns"), "{v:?}");
    }

    #[test]
    fn secret_never_prints_or_serializes_its_value() {
        let s: Secret = serde_json::from_value(serde_json::json!("p@ss'w\\rd")).unwrap();
        assert_eq!(s.expose(), "p@ss'w\\rd");
        assert!(!format!("{s:?}").contains("p@ss"));
        assert_eq!(serde_json::to_value(&s).unwrap(), serde_json::json!("********"));
        let o = ApplyOptions {
            apply_id: Uuid::nil(),
            lock_timeout_ms: 0,
            expected_hash: "h".into(),
            confirmed: false,
            confirmed_production: false,
            password: Some(s),
        };
        assert!(!format!("{o:?}").contains("p@ss"));
        assert!(!serde_json::to_string(&o).unwrap().contains("p@ss"));
    }

    #[test]
    fn plan_view_drop_passes_force() {
        let object = ObjectRef { kind: ObjectKind::Schema, oid: 1 };
        let v = plan_view(&PlanRequest::Drop { object, def: schema("a"), cascade: false, force: true }, 160000);
        assert!(v.empty);
        assert_eq!(v.errors[0].path, "force");
        let v = plan_view(&PlanRequest::Drop { object, def: schema("a"), cascade: false, force: false }, 160000);
        assert!(!v.empty && v.errors.is_empty());
    }

    #[test]
    fn lock_timeout_bound() {
        assert!(check_lock_timeout(MAX_STATEMENT_TIMEOUT_MS).is_ok());
        assert!(matches!(check_lock_timeout(MAX_STATEMENT_TIMEOUT_MS + 1), Err(Error::Invalid(_))));
    }

    #[test]
    fn load_and_apply_targets_agree_for_every_kind() {
        // Yükleme ve uygulama aynı işlevi kullanır; tüm türlerde ve her `database_oid` biçiminde aynı sonuç.
        let kinds = [
            ObjectKind::Schema,
            ObjectKind::Sequence,
            ObjectKind::Extension,
            ObjectKind::Index,
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::MaterializedView,
            ObjectKind::Database,
            ObjectKind::Role,
            ObjectKind::Function,
            ObjectKind::Procedure,
            ObjectKind::Trigger,
            ObjectKind::Type,
            ObjectKind::Domain,
        ];
        for k in kinds {
            for db in [None, Some(5)] {
                let a = target_for(k, db).map_err(|e| format!("{e:?}"));
                let l = crate::load::target_for(k, db).map_err(|e| format!("{e:?}"));
                assert_eq!(a, l, "{k:?} {db:?}");
                let expected = match (k.server_scoped(), db) {
                    (true, _) => Ok(Target::Maintenance),
                    (false, Some(d)) => Ok(Target::Database(d)),
                    (false, None) => Err(format!("{:?}", Error::Invalid(NEEDS_DATABASE.into()))),
                };
                assert_eq!(a, expected, "{k:?} {db:?}");
            }
        }
    }

    #[test]
    fn target_for_scopes() {
        for k in [ObjectKind::Database, ObjectKind::Role] {
            assert_eq!(target_for(k, None).unwrap(), Target::Maintenance, "{k:?}");
            assert_eq!(target_for(k, Some(5)).unwrap(), Target::Maintenance, "{k:?}: database_oid yok sayılır");
        }
        for k in [
            ObjectKind::Schema,
            ObjectKind::Sequence,
            ObjectKind::Extension,
            ObjectKind::Index,
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::MaterializedView,
            ObjectKind::Function,
            ObjectKind::Procedure,
            ObjectKind::Trigger,
            ObjectKind::Type,
            ObjectKind::Domain,
        ] {
            assert_eq!(target_for(k, Some(5)).unwrap(), Target::Database(5), "{k:?}");
            assert!(matches!(target_for(k, None), Err(Error::Invalid(m)) if m == NEEDS_DATABASE), "{k:?}");
        }
    }

    pub(super) fn role(name: &str) -> RoleDef {
        RoleDef {
            name: name.into(),
            login: true,
            superuser: false,
            createdb: false,
            createrole: false,
            inherit: true,
            replication: false,
            bypassrls: false,
            connection_limit: -1,
            valid_until: None,
            password: PasswordChange::Keep,
            member_of: vec![],
            settings: vec![],
            comment: None,
            unsupported: vec![],
        }
    }

    fn stmt(sql: &str, redacted: bool) -> Statement {
        Statement { sql: sql.into(), risk: Risk::Safe, transactional: true, note: None, redacted }
    }

    #[test]
    fn materialize_replaces_only_the_redacted_statement() {
        let mut new = role("app");
        new.password = PasswordChange::Set;
        new.comment = Some("c".into());
        let req = PlanRequest::Create { def: ObjectDef::Role(new), concurrently: false };
        let view = plan_view(&req, 160000);
        assert!(view.statements.iter().any(|s| s.redacted), "{view:?}");
        let before = view.clone();
        let secret = Secret::new("gizli-parola-M1");
        let exec = materialize(&view.statements, &req, Some(&secret)).unwrap();
        assert_eq!(exec.len(), view.statements.len());
        for (e, s) in exec.iter().zip(&view.statements) {
            assert_eq!((e.transactional, e.redacted), (s.transactional, s.redacted));
            match &e.sql {
                ExecSql::Plan(t) => assert!(!s.redacted && *t == s.sql),
                ExecSql::Password(p) => {
                    assert!(s.redacted);
                    assert!(p.sql().starts_with("ALTER ROLE app PASSWORD 'SCRAM-SHA-256$4096:"), "biçim");
                    assert!(!p.sql().contains("gizli-parola-M1"), "düz metin olmamalı");
                }
            }
            // Debug çıktısı gizlenmiş metni yazmaz.
            let dbg = format!("{e:?}");
            assert!(!dbg.contains("SCRAM-SHA-256$") && !dbg.contains("gizli-parola-M1"), "{dbg}");
        }
        drop(exec);
        // Plan (önizleme kopyası) değişmez: yer tutucu kalır, hash parolaya bağlı değildir.
        assert_eq!(view, before);
        assert!(view.statements.iter().any(|s| s.redacted && s.sql.ends_with(crate::plan::REDACTED)));
        // Rename + set: yeni ad kullanılır.
        let mut old = role("app");
        old.login = true;
        let mut renamed = old.clone();
        renamed.name = "app2".into();
        renamed.password = PasswordChange::Set;
        let req = PlanRequest::Alter { object: ObjectRef { kind: ObjectKind::Role, oid: 1 }, old: ObjectDef::Role(old), new: ObjectDef::Role(renamed) };
        let view = plan_view(&req, 160000);
        let exec = materialize(&view.statements, &req, Some(&secret)).unwrap();
        assert!(exec.iter().any(|e| matches!(&e.sql, ExecSql::Password(p) if p.sql().starts_with("ALTER ROLE app2 PASSWORD 'SCRAM"))));
    }

    #[test]
    fn materialize_refuses_mismatched_or_unusable_secrets() {
        let st = [stmt("CREATE ROLE app", false), role::password_redacted("app")];
        let ok = Secret::new("p");
        assert!(materialize_for(&st, Some("app"), Some(&ok)).is_ok());
        // Gizlenmiş ifade başka bir role ait: somutlaştırılmaz.
        assert!(matches!(materialize_for(&st, Some("other"), Some(&ok)), Err(Error::Invalid(m)) if m == INVALID_DEF));
        // Plan dışı gizlenmiş metin somutlaşmaz.
        let forged = [stmt("DROP ROLE app", true)];
        assert!(matches!(materialize_for(&forged, Some("app"), Some(&ok)), Err(Error::Invalid(m)) if m == INVALID_DEF));
        assert!(matches!(materialize_for(&st, Some("app"), None), Err(Error::Invalid(m)) if m == ROLE_PASSWORD_MISSING));
        assert!(matches!(materialize_for(&st, None, Some(&ok)), Err(Error::Invalid(m)) if m == ROLE_PASSWORD_MISSING));
        let empty = Secret::new("");
        assert!(matches!(materialize_for(&st, Some("app"), Some(&empty)), Err(Error::Invalid(m)) if m == role::SECRET_EMPTY_MSG));
        let nul = Secret::new(&format!("a{}b", char::from(0)));
        let e = materialize_for(&st, Some("app"), Some(&nul)).unwrap_err();
        assert!(matches!(&e, Error::Invalid(m) if m == role::SECRET_NUL_MSG));
        // Gizlenmiş ifade yoksa parola metinleri hiç işlenmez.
        let plain = [stmt("CREATE ROLE app", false)];
        assert_eq!(materialize_for(&plain, None, None).unwrap().len(), 1);
    }

    #[test]
    fn password_gate_pairs_redacted_statement_and_secret() {
        let with = [stmt("CREATE ROLE app", false), role::password_redacted("app")];
        let without = [stmt("CREATE ROLE app", false)];
        let s = Secret::new("p");
        assert!(password_gate(&with, Some(&s)).is_ok());
        assert!(password_gate(&without, None).is_ok());
        assert!(matches!(password_gate(&with, None), Err(Error::Invalid(m)) if m == ROLE_PASSWORD_MISSING));
        assert!(matches!(password_gate(&without, Some(&s)), Err(Error::Invalid(m)) if m == PASSWORD_NOT_USED));
        assert_eq!(crate::ddl_payload(&password_gate(&with, None).unwrap_err()).code, "role_password_missing");
    }

    fn q(code: &str, hint: Option<&str>) -> Error {
        Error::Query { sqlstate: Some(code.into()), message: "m".into(), detail: None, hint: hint.map(Into::into) }
    }

    #[test]
    fn failure_hints_follow_kind_statement_and_code() {
        let nt = |sql: &str| Statement { sql: sql.into(), risk: Risk::Safe, transactional: false, note: None, redacted: false };
        let cases: [(ObjectKind, Statement, &str, Option<&str>); 16] = [
            (ObjectKind::View, stmt("CREATE OR REPLACE VIEW public.v AS (SELECT 1\n)", false), "42P16", Some(VIEW_COLUMNS_HINT)),
            (ObjectKind::View, stmt("CREATE OR REPLACE VIEW public.v AS (SELECT 1\n)", false), "42804", Some(VIEW_COLUMNS_HINT)),
            // Aynı kod başka ifadede (ör. ALTER VIEW) ipucu almaz.
            (ObjectKind::View, stmt("ALTER VIEW public.v RENAME TO w", false), "42P16", None),
            (ObjectKind::Table, stmt("CREATE OR REPLACE VIEW x", false), "42P16", None),
            (ObjectKind::MaterializedView, stmt("REFRESH MATERIALIZED VIEW CONCURRENTLY public.m", false), "55000", Some(REFRESH_CONCURRENTLY_HINT)),
            (ObjectKind::MaterializedView, stmt("REFRESH MATERIALIZED VIEW public.m", false), "55000", None),
            (ObjectKind::Database, nt("ALTER DATABASE d RENAME TO e"), "55006", Some(DATABASE_IN_USE_HINT)),
            (ObjectKind::Database, nt("ALTER DATABASE d SET TABLESPACE t"), "55006", Some(DATABASE_IN_USE_HINT)),
            (ObjectKind::Database, nt("DROP DATABASE d"), "55006", Some(DATABASE_IN_USE_HINT)),
            (ObjectKind::Database, nt("DROP DATABASE d WITH (FORCE)"), "55006", Some(FORCE_LIMITS_HINT)),
            (ObjectKind::Database, nt("CREATE DATABASE d TEMPLATE = t"), "55006", Some(TEMPLATE_IN_USE_HINT)),
            (ObjectKind::Role, stmt("CREATE ROLE r SUPERUSER", false), "42501", Some(PRIVILEGE_HINT)),
            (ObjectKind::View, stmt("ALTER VIEW v OWNER TO r", false), "42501", Some(PRIVILEGE_HINT)),
            (ObjectKind::Role, stmt("DROP ROLE r", false), "2BP01", Some(ROLE_DEPENDENTS_HINT)),
            (ObjectKind::View, stmt("DROP VIEW v", false), "2BP01", Some(VIEW_DEPENDENTS_HINT)),
            (ObjectKind::MaterializedView, stmt("DROP MATERIALIZED VIEW m", false), "2BP01", Some(VIEW_DEPENDENTS_HINT)),
        ];
        for (kind, st, code, want) in cases {
            let p = failure_payload_for(kind, "drop", Some(&st), 0, &q(code, None));
            assert_eq!(p.hint.as_deref(), want, "{kind:?} {code} {}", st.sql);
            // PG'nin kendi ipucu korunur.
            let p = failure_payload_for(kind, "drop", Some(&st), 0, &q(code, Some("pg")));
            assert_eq!(p.hint.as_deref(), Some("pg"), "{kind:?} {code}");
        }
        // Düzenlemede yeniden kurma: aynı DROP ifadesi CASCADE önermez (inceleme M-3).
        for (kind, sql) in [(ObjectKind::View, "DROP VIEW public.v"), (ObjectKind::MaterializedView, "DROP MATERIALIZED VIEW public.m")] {
            let p = failure_payload_for(kind, "alter", Some(&stmt(sql, false)), 0, &q("2BP01", None));
            assert_eq!(p.hint.as_deref(), Some(REBUILD_DEPENDENTS_HINT), "{kind:?}");
            assert!(!REBUILD_DEPENDENTS_HINT.contains("CASCADE"));
            let p = failure_payload_for(kind, "drop", Some(&stmt(sql, false)), 0, &q("2BP01", None));
            assert_eq!(p.hint.as_deref(), Some(VIEW_DEPENDENTS_HINT), "{kind:?}");
        }
        // Kısmi uygulama (transaction dışı, ipucu yok).
        let p = failure_payload_for(ObjectKind::Database, "drop", Some(&nt("COMMENT ON DATABASE d IS 'x'")), 1, &q("42704", None));
        assert_eq!(p.hint.as_deref(), Some(PARTIAL_APPLY_HINT));
        let p = failure_payload_for(ObjectKind::Database, "drop", Some(&nt("COMMENT ON DATABASE d IS 'x'")), 0, &q("42704", None));
        assert_eq!(p.hint, None);
        // İptal/kilit iletileri ve ipuçları korunur.
        let p = failure_payload_for(ObjectKind::Database, "drop", Some(&nt("DROP DATABASE d")), 0, &q("55P03", None));
        assert!(p.hint.is_some() && p.hint.as_deref() != Some(DATABASE_IN_USE_HINT));
    }

    #[test]
    fn redacted_failure_never_echoes_a_verifier() {
        let st = role::password_redacted("app");
        let e = Error::Query {
            sqlstate: Some("42601".into()),
            message: "near 'SCRAM-SHA-256$4096:abc'".into(),
            detail: Some("SCRAM-SHA-256$x".into()),
            hint: None,
        };
        let p = failure_payload_for(ObjectKind::Role, "drop", Some(&st), 0, &e);
        assert_eq!((p.message.as_str(), p.detail, p.hint), (REDACTED_FAILURE_MESSAGE, None, None));
        // Gizlenmemiş ifadede ileti olduğu gibi kalır.
        let p = failure_payload_for(ObjectKind::Role, "drop", Some(&stmt("CREATE ROLE app", false)), 0, &q("42710", None));
        assert_eq!(p.message, "m");
    }

    #[test]
    fn identity_changed_for_view_matview_database_role() {
        let v = |schema: &str, name: &str, query: &str, recreate: bool| {
            ObjectDef::View(ViewDef {
                schema: schema.into(),
                name: name.into(),
                owner: None,
                query: query.into(),
                check_option: None,
                security_barrier: false,
                security_invoker: false,
                comment: None,
                recreate,
                columns: vec![],
                unsupported: vec![],
            })
        };
        let base = v("public", "v", "SELECT 1", false);
        assert!(!identity_changed(&base, &v("public", "v", "SELECT 2", false)), "C-O-R oid'i korur");
        assert!(identity_changed(&base, &v("public", "v", "SELECT 2", true)), "recreate oid'i değiştirir");
        assert!(!identity_changed(&base, &v("public", "v", "SELECT 1", true)), "sorgu aynıysa recreate yok sayılır");
        assert!(identity_changed(&base, &v("public", "w", "SELECT 1", false)));
        assert!(identity_changed(&base, &v("app", "v", "SELECT 1", false)));
        let m = |name: &str, query: &str, comment: Option<&str>| {
            ObjectDef::MaterializedView(MatViewDef {
                schema: "public".into(),
                name: name.into(),
                owner: None,
                query: query.into(),
                tablespace: None,
                fillfactor: None,
                with_data: true,
                comment: comment.map(Into::into),
                refresh: None,
                columns: vec![],
                unsupported: vec![],
            })
        };
        assert!(!identity_changed(&m("m", "SELECT 1", None), &m("m", "SELECT 1", Some("c"))));
        assert!(identity_changed(&m("m", "SELECT 1", None), &m("m", "SELECT 2", None)));
        assert!(identity_changed(&m("m", "SELECT 1", None), &m("n", "SELECT 1", None)));
        let mut r2 = role("b");
        assert!(identity_changed(&ObjectDef::Role(role("a")), &ObjectDef::Role(r2.clone())));
        r2.name = "a".into();
        r2.login = false;
        assert!(!identity_changed(&ObjectDef::Role(role("a")), &ObjectDef::Role(r2)));
        let d = |name: &str, comment: Option<&str>| ObjectDef::Database(DatabaseDef {
            name: name.into(),
            owner: None,
            template: None,
            encoding: None,
            locale_provider: None,
            lc_collate: None,
            lc_ctype: None,
            provider_locale: None,
            tablespace: None,
            connection_limit: -1,
            is_template: false,
            allow_connections: true,
            settings: vec![],
            comment: comment.map(Into::into),
        });
        assert!(identity_changed(&d("a", None), &d("b", None)));
        assert!(!identity_changed(&d("a", None), &d("a", Some("c"))));
    }

    // ---- Plan 18: Function / Procedure / Trigger / Type / Domain ----

    fn func(name: &str, args: &[(&str, &str)]) -> FunctionDef {
        FunctionDef {
            schema: "public".into(),
            name: name.into(),
            owner: None,
            comment: None,
            language: "plpgsql".into(),
            args: args
                .iter()
                .map(|(n, t)| FunctionArg { mode: ArgMode::In, name: Some((*n).into()), type_name: (*t).into(), default: None })
                .collect(),
            returns: Some(FunctionReturn::Void),
            body: "BEGIN\n\nEND;".into(),
            volatility: Volatility::Volatile,
            strict: false,
            security_definer: false,
            leakproof: false,
            parallel: Parallel::Unsafe,
            cost: None,
            rows: None,
            settings: vec![],
            recreate: false,
            editable: true,
            unsupported: vec![],
        }
    }

    fn trig(name: &str) -> TriggerDef {
        TriggerDef {
            schema: "public".into(),
            table: "t".into(),
            name: name.into(),
            timing: TriggerTiming::After,
            events: TriggerEvents { insert: true, update: false, update_columns: vec![], delete: false, truncate: false },
            level: TriggerLevel::Row,
            when: None,
            function_schema: "public".into(),
            function_name: "tf".into(),
            args: vec![],
            old_table: None,
            new_table: None,
            constraint: false,
            deferrable: false,
            initially_deferred: false,
            enabled: TriggerEnabled::Origin,
            comment: None,
            editable: true,
            unsupported: vec![],
        }
    }

    fn pgtype(schema: &str, name: &str) -> TypeDef {
        TypeDef {
            schema: schema.into(),
            name: name.into(),
            owner: None,
            comment: None,
            body: TypeBody::Enum { values: vec![EnumValue { label: "a".into(), original: Some("a".into()) }] },
            editable: true,
            unsupported: vec![],
        }
    }

    fn domain(schema: &str, name: &str) -> DomainDef {
        DomainDef {
            schema: schema.into(),
            name: name.into(),
            owner: None,
            comment: None,
            base_type: "text".into(),
            collation: None,
            default: None,
            not_null: false,
            constraints: vec![],
            editable: true,
            unsupported: vec![],
        }
    }

    #[test]
    fn identity_changed_for_routines_triggers_types_domains() {
        let f = |d: &FunctionDef| ObjectDef::Function(d.clone());
        let base = func("f", &[("a", "integer")]);
        let mut body = base.clone();
        body.body = "BEGIN RETURN; END;".into();
        body.comment = Some("c".into());
        assert!(!identity_changed(&f(&base), &f(&body)), "OR REPLACE oid'i korur");
        let mut renamed = base.clone();
        renamed.name = "g".into();
        assert!(identity_changed(&f(&base), &f(&renamed)));
        let mut moved = base.clone();
        moved.schema = "app".into();
        assert!(identity_changed(&f(&base), &f(&moved)));
        // İmza değişimi yalnızca yeniden kurmayla (yeni oid).
        let mut retyped = func("f", &[("a", "text")]);
        assert!(!identity_changed(&f(&base), &f(&retyped)), "recreate'siz plan zaten alan hatası");
        retyped.recreate = true;
        assert!(identity_changed(&f(&base), &f(&retyped)));
        // Gereksiz `recreate` yok sayılır (üretici de sessizce yok sayar).
        let mut needless = body.clone();
        needless.recreate = true;
        assert!(!identity_changed(&f(&base), &f(&needless)));
        let p = |d: &FunctionDef| ObjectDef::Procedure(FunctionDef { returns: None, ..d.clone() });
        assert!(identity_changed(&p(&base), &p(&renamed)));
        assert!(!identity_changed(&p(&base), &p(&body)));
        assert!(identity_changed(&p(&base), &p(&retyped)));

        let t = |d: TriggerDef| ObjectDef::Trigger(d);
        assert!(identity_changed(&t(trig("a")), &t(trig("b"))));
        let mut commented = trig("a");
        commented.comment = Some("c".into());
        commented.enabled = TriggerEnabled::Disabled;
        assert!(!identity_changed(&t(trig("a")), &t(commented)));
        // Yeniden kurma (PG < 14 / constraint trigger: DROP + CREATE) oid'i değiştirebilir.
        let mut rebuilt = trig("a");
        rebuilt.timing = TriggerTiming::Before;
        assert!(identity_changed(&t(trig("a")), &t(rebuilt)));

        let ty = |d: TypeDef| ObjectDef::Type(d);
        assert!(identity_changed(&ty(pgtype("public", "e")), &ty(pgtype("public", "e2"))));
        assert!(identity_changed(&ty(pgtype("public", "e")), &ty(pgtype("app", "e"))));
        let mut added = pgtype("public", "e");
        added.body = TypeBody::Enum { values: vec![EnumValue { label: "a".into(), original: Some("a".into()) }, EnumValue { label: "b".into(), original: None }] };
        assert!(!identity_changed(&ty(pgtype("public", "e")), &ty(added)));
        let d = |d: DomainDef| ObjectDef::Domain(d);
        assert!(identity_changed(&d(domain("public", "d")), &d(domain("public", "d2"))));
        assert!(identity_changed(&d(domain("public", "d")), &d(domain("app", "d"))));
        let mut nn = domain("public", "d");
        nn.not_null = true;
        assert!(!identity_changed(&d(domain("public", "d")), &d(nn)));
    }

    #[test]
    fn read_only_gate_refuses_alter_and_drop_with_a_generic_reason() {
        let object = ObjectRef { kind: ObjectKind::Function, oid: 1 };
        let mut ro = func("gizli_fn_adi", &[]);
        ro.editable = false;
        ro.language = "c".into();
        ro.unsupported = vec!["LANGUAGE c".into(), "EXTENSION gizli_eklenti (eklentiye ait)".into()];
        let mut new = ro.clone();
        new.comment = Some("x".into());
        let alter = PlanRequest::Alter { object, old: ObjectDef::Function(ro.clone()), new: ObjectDef::Function(new) };
        let drop = PlanRequest::Drop { object, def: ObjectDef::Function(ro.clone()), cascade: false, force: false };
        for req in [&alter, &drop] {
            let m = read_only_refusal(req).expect("salt okunur reddedilmeli");
            assert!(m.starts_with(READ_ONLY_PREFIX), "{m}");
            // Kodsuz düz `invalid`.
            assert_eq!(crate::ddl_payload(&Error::Invalid(m.clone())).code, "invalid");
            assert!(!m.contains("gizli"), "ad/değer yok: {m}");
        }
        // Gerekçe ilk girdiden, genel biçimde.
        assert_eq!(read_only_refusal(&drop).unwrap(), format!("{READ_ONLY_PREFIX}{}", read_only_reason(Some("LANGUAGE c"))));
        // Düzenlenebilir nesne ve create geçer.
        let ok = PlanRequest::Drop { object, def: ObjectDef::Function(func("f", &[])), cascade: false, force: false };
        assert_eq!(read_only_refusal(&ok), None);
        assert_eq!(read_only_refusal(&PlanRequest::Create { def: ObjectDef::Function(ro.clone()), concurrently: false }), None);
        // Beş türün her biri.
        let mut t = trig("t");
        (t.editable, t.unsupported) = (false, vec!["iç trigger (kısıt ya da sistem tarafından yönetilir)".into()]);
        let mut ty = pgtype("public", "e");
        (ty.editable, ty.unsupported) = (false, vec!["BASE TYPE".into()]);
        let mut dm = domain("public", "d");
        (dm.editable, dm.unsupported) = (false, vec![]);
        let p = ObjectDef::Procedure(FunctionDef { returns: None, ..ro.clone() });
        for def in [p, ObjectDef::Trigger(t), ObjectDef::Type(ty), ObjectDef::Domain(dm)] {
            let req = PlanRequest::Drop { object: ObjectRef { kind: def.kind(), oid: 1 }, def, cascade: true, force: false };
            assert!(read_only_refusal(&req).is_some_and(|m| m.starts_with(READ_ONLY_PREFIX)), "{req:?}");
        }
        // Eski türler bu kapıdan etkilenmez.
        assert_eq!(read_only_refusal(&PlanRequest::Drop { object, def: schema("s"), cascade: false, force: false }), None);
    }

    #[test]
    fn read_only_reasons_are_generic() {
        // Yükleyicinin ürettiği her gerekçe biçimi (ad/değer taşıyanlar dahil) ad/değer içermeyen sabit metne iner.
        let entries = [
            "LANGUAGE zz_gizli",
            "BEGIN ATOMIC (SQL standart gövde)",
            "AGGREGATE",
            "WINDOW",
            "EXTENSION zz_gizli (eklentiye ait)",
            "sistem şeması",
            "SUPPORT zz_gizli",
            "TRANSFORM FOR TYPE",
            "tip zz_gizli",
            "argüman modu 'zz_gizli'",
            "adsız TABLE kolonu",
            "COST zz_gizli",
            "ROWS zz_gizli",
            "iç trigger (kısıt ya da sistem tarafından yönetilir)",
            "bölümlenmiş üst tablodan klonlanmış (üst tablodan yönetilir)",
            "FROM zz_gizli (constraint trigger başvurulan tablosu)",
            "görünüm trigger'ı",
            "yabancı tablo trigger'ı",
            "'zz_gizli' türü ilişkinin trigger'ı",
            "WHEN koşulu ayrıştırılamadı",
            "trigger argümanları çözülemedi",
            "etkinlik durumu 'zz_gizli'",
            "tablo satır tipi (relkind 'zz_gizli'; ilişkiden yönetilir)",
            "ilişkisi bulunamayan composite",
            "range bilgisi bulunamadı",
            "fonksiyon zz_gizli",
            "MULTIRANGE (range tipinden yönetilir)",
            "dizi tipi (ARRAY; öğe tipinden yönetilir)",
            "BASE TYPE",
            "SHELL TYPE (tanımsız)",
            "PSEUDO TYPE",
            "'zz_gizli' türü tip",
            "CHECK zz_gizli (ayrıştırılamadı)",
            "NOT NULL zz_gizli (NOT VALID)",
            "'zz_gizli' türü kısıt zz_gizli",
            // Plan 20 yetki yükleyicileri.
            crate::load_acl::ARRAY_REASON,
            crate::load_acl::MULTIRANGE_REASON,
            crate::load_acl::ROW_TYPE_REASON,
            crate::load_acl::AGGREGATE_REASON,
            crate::load_acl::SIGNATURE_REASON,
            crate::load_acl::DEFAULT_GRANTOR_REASON,
            crate::load_acl::BUILDER_REJECTS_REASON,
            crate::load::INVALID_UTF8_REASON,
            "yetki zz_gizli",
            "varsayılan yetki türü zz_gizli",
            "zz_gizli bilinmeyen",
        ];
        let generic = read_only_reason(None);
        for e in entries {
            let r = read_only_reason(Some(e));
            assert!(!r.contains("zz_gizli") && !r.is_empty(), "{e} → {r}");
            if e != "zz_gizli bilinmeyen" {
                assert_ne!(r, generic, "bilinen gerekçe genel metne düşmemeli: {e}");
            }
        }
        assert_eq!(read_only_reason(Some("zz_gizli bilinmeyen")), generic);
    }

    #[test]
    fn routine_type_domain_hints() {
        let none = HintCtx { leakproof: false };
        let lp = HintCtx { leakproof: true };
        type Case<'a> = (ObjectKind, &'a str, &'a str, &'a str, HintCtx, Option<&'a str>);
        let cases: [Case; 22] = [
            // Silme: bağımlılar (CASCADE önerilir).
            (ObjectKind::Function, "drop", "DROP FUNCTION public.f(integer)", "2BP01", none, Some(VIEW_DEPENDENTS_HINT)),
            (ObjectKind::Procedure, "drop", "DROP PROCEDURE public.p(IN integer)", "2BP01", none, Some(VIEW_DEPENDENTS_HINT)),
            (ObjectKind::Type, "drop", "DROP TYPE public.e", "2BP01", none, Some(VIEW_DEPENDENTS_HINT)),
            (ObjectKind::Domain, "drop", "DROP DOMAIN public.d", "2BP01", none, Some(VIEW_DEPENDENTS_HINT)),
            // Yeniden kurma: bu akışta CASCADE yok.
            (ObjectKind::Function, "alter", "DROP FUNCTION public.f(integer)", "2BP01", none, Some(ROUTINE_REBUILD_DEPENDENTS_HINT)),
            (ObjectKind::Procedure, "alter", "DROP PROCEDURE public.p(IN integer)", "2BP01", none, Some(ROUTINE_REBUILD_DEPENDENTS_HINT)),
            // OR REPLACE sınırı.
            // PG ipucu yoksa (gövde doğrulaması, ör. sql fonksiyonunda dönüş tipi uyuşmazlığı) OR REPLACE ipucu eklenmez.
            (ObjectKind::Function, "alter", "CREATE OR REPLACE FUNCTION public.f() RETURNS void", "42P13", none, None),
            (ObjectKind::Procedure, "alter", "CREATE OR REPLACE PROCEDURE public.p()", "42P13", none, None),
            (ObjectKind::Function, "create", "CREATE FUNCTION public.f() RETURNS void", "42P13", none, None),
            // Function ↔ Procedure.
            (ObjectKind::Function, "alter", "CREATE OR REPLACE FUNCTION public.f() RETURNS void", "42809", none, Some(ROUTINE_KIND_HINT)),
            (ObjectKind::Procedure, "create", "CREATE PROCEDURE public.p()", "42809", none, Some(ROUTINE_KIND_HINT)),
            (ObjectKind::Type, "alter", "ALTER TYPE public.e RENAME TO f", "42809", none, None),
            // Typed table.
            (ObjectKind::Type, "alter", "ALTER TYPE public.c DROP ATTRIBUTE a", "2BP01", none, Some(TYPED_TABLE_HINT)),
            (ObjectKind::Type, "alter", "ALTER TYPE public.c ADD ATTRIBUTE b integer", "2BP01", none, Some(TYPED_TABLE_HINT)),
            (ObjectKind::Type, "alter", "ALTER TYPE public.c ALTER ATTRIBUTE a TYPE text", "2BP01", none, Some(TYPED_TABLE_HINT)),
            (ObjectKind::Type, "alter", "ALTER TYPE public.c ALTER ATTRIBUTE a TYPE text", "0A000", none, Some(TYPE_COLUMN_RETYPE_HINT)),
            (ObjectKind::Type, "alter", "ALTER TYPE public.c RENAME TO d", "0A000", none, None),
            // LEAKPROOF yalnız süper kullanıcı; başka 42501 genel yetki ipucu.
            (ObjectKind::Function, "create", "CREATE FUNCTION public.f() RETURNS integer LEAKPROOF", "42501", lp, Some(LEAKPROOF_HINT)),
            (ObjectKind::Function, "alter", "CREATE OR REPLACE FUNCTION public.f() RETURNS integer", "42501", lp, Some(LEAKPROOF_HINT)),
            (ObjectKind::Function, "alter", "ALTER FUNCTION public.f() OWNER TO r", "42501", lp, Some(PRIVILEGE_HINT)),
            (ObjectKind::Function, "create", "CREATE FUNCTION public.f() RETURNS integer", "42501", none, Some(PRIVILEGE_HINT)),
            (ObjectKind::Trigger, "drop", "DROP TRIGGER t ON public.x", "2BP01", none, None),
        ];
        for (kind, op, sql, code, ctx, want) in cases {
            let p = failure_payload_with(kind, op, ctx, Some(&stmt(sql, false)), 0, &q(code, None));
            assert_eq!(p.hint.as_deref(), want, "{kind:?} {op} {code} {sql}");
        }
        // PG'nin kendi ipucu: CASCADE ya da SQL öneren ipuçları bu akışlarda yanıltıcıdır → bizimki geçer; diğerleri korunur.
        for (kind, op, sql, code, want) in [
            (ObjectKind::Function, "alter", "DROP FUNCTION public.f(integer)", "2BP01", ROUTINE_REBUILD_DEPENDENTS_HINT),
            (ObjectKind::Type, "alter", "ALTER TYPE public.c DROP ATTRIBUTE a", "2BP01", TYPED_TABLE_HINT),
            (ObjectKind::Function, "alter", "CREATE OR REPLACE FUNCTION public.f() RETURNS void", "42P13", OR_REPLACE_HINT),
            (ObjectKind::Procedure, "alter", "CREATE OR REPLACE PROCEDURE public.p()", "42P13", OR_REPLACE_HINT),
        ] {
            let p = failure_payload_with(kind, op, none, Some(&stmt(sql, false)), 0, &q(code, Some("Use CASCADE")));
            assert_eq!(p.hint.as_deref(), Some(want), "{kind:?} {code}");
        }
        let p = failure_payload_with(ObjectKind::Function, "drop", none, Some(&stmt("DROP FUNCTION f()", false)), 0, &q("2BP01", Some("pg")));
        assert_eq!(p.hint.as_deref(), Some("pg"));
        // İpuçları SQL/ad/değer taşımaz.
        for h in [ROUTINE_REBUILD_DEPENDENTS_HINT, OR_REPLACE_HINT, ROUTINE_KIND_HINT, TYPED_TABLE_HINT, TYPE_COLUMN_RETYPE_HINT, LEAKPROOF_HINT] {
            for sql in ["DROP ", "ALTER ", "CREATE ", "public", "$"] {
                assert!(!h.contains(sql), "{h}");
            }
        }
        assert!(!ROUTINE_REBUILD_DEPENDENTS_HINT.contains("CASCADE"));
    }

    #[test]
    fn leakproof_hint_context_follows_the_request() {
        let mut f = func("f", &[]);
        f.returns = Some(FunctionReturn::Type { type_name: "integer".into(), setof: false });
        let create = |d: &FunctionDef| PlanRequest::Create { def: ObjectDef::Function(d.clone()), concurrently: false };
        assert!(!HintCtx::of(&create(&f)).leakproof);
        let mut l = f.clone();
        l.leakproof = true;
        assert!(HintCtx::of(&create(&l)).leakproof);
        let object = ObjectRef { kind: ObjectKind::Function, oid: 1 };
        let alter = PlanRequest::Alter { object, old: ObjectDef::Function(f.clone()), new: ObjectDef::Function(l.clone()) };
        assert!(HintCtx::of(&alter).leakproof);
        let drop = PlanRequest::Drop { object, def: ObjectDef::Function(l), cascade: false, force: false };
        assert!(!HintCtx::of(&drop).leakproof);
    }

    #[test]
    fn routine_find_parameters_use_the_identity_signature() {
        let mut f = func("f", &[("a", "integer"), ("b", "text")]);
        f.args.push(FunctionArg { mode: ArgMode::Out, name: Some("o".into()), type_name: "text".into(), default: None });
        f.args.push(FunctionArg { mode: ArgMode::Variadic, name: Some("v".into()), type_name: "int[]".into(), default: None });
        // Fonksiyon: giriş tipleri (OUT hariç), modlar karşılaştırılmaz.
        assert_eq!(routine_find_args(ObjectKind::Function, &f), (vec!["integer".to_string(), "text".into(), "int[]".into()], None));
        // Procedure: tüm argümanlar + `proargmodes` metni (hepsi IN ise boş).
        assert_eq!(
            routine_find_args(ObjectKind::Procedure, &f),
            (vec!["integer".to_string(), "text".into(), "text".into(), "int[]".into()], Some("{i,i,o,v}".to_string()))
        );
        let plain = func("p", &[("a", "integer")]);
        assert_eq!(routine_find_args(ObjectKind::Procedure, &plain), (vec!["integer".to_string()], Some(String::new())));
        let mut io = plain.clone();
        io.args[0].mode = ArgMode::Inout;
        assert_eq!(routine_find_args(ObjectKind::Procedure, &io).1.as_deref(), Some("{b}"));
    }

    #[test]
    fn maintenance_database_gate_and_release() {
        let object = ObjectRef { kind: ObjectKind::Database, oid: 7 };
        let d = |name: &str| DatabaseDef {
            name: name.into(),
            owner: Some("o".into()),
            template: None,
            encoding: Some("UTF8".into()),
            locale_provider: None,
            lc_collate: Some("C".into()),
            lc_ctype: Some("C".into()),
            provider_locale: None,
            tablespace: Some("pg_default".into()),
            connection_limit: -1,
            is_template: false,
            allow_connections: true,
            settings: vec![],
            comment: None,
        };
        let alter = |edit: &dyn Fn(&mut DatabaseDef)| {
            let mut new = d("postgres");
            edit(&mut new);
            PlanRequest::Alter { object, old: ObjectDef::Database(d("postgres")), new: ObjectDef::Database(new) }
        };
        let rename = alter(&|n| n.name = "x".into());
        let move_ts = alter(&|n| n.tablespace = Some("ts".into()));
        let close = alter(&|n| n.allow_connections = false);
        let comment = alter(&|n| n.comment = Some("c".into()));
        let limit = alter(&|n| n.connection_limit = 5);
        let no_conn = alter(&|n| n.connection_limit = 0);
        let drop = PlanRequest::Drop { object, def: ObjectDef::Database(d("postgres")), cascade: false, force: false };
        for r in [&rename, &move_ts, &close, &no_conn, &drop] {
            assert!(touches_maintenance(r, "postgres"), "{r:?}");
            assert!(!touches_maintenance(r, "other"), "{r:?}");
        }
        for r in [&comment, &limit] {
            assert!(!touches_maintenance(r, "postgres"), "{r:?}");
        }
        // Bakım veritabanını şablon alan CREATE DATABASE ayrı iletiyle reddedilir.
        let mut copy = d("kopya");
        copy.template = Some("postgres".into());
        let copy = PlanRequest::Create { def: ObjectDef::Database(copy), concurrently: false };
        assert_eq!(maintenance_refusal(&copy, "postgres"), Some(MAINTENANCE_TEMPLATE));
        assert_eq!(maintenance_refusal(&copy, "other"), None);
        assert_eq!(maintenance_refusal(&drop, "postgres"), Some(MAINTENANCE_DATABASE));
        assert_eq!(database_to_release(&rename), Some(7));
        assert_eq!(database_to_release(&move_ts), Some(7));
        assert_eq!(database_to_release(&drop), Some(7));
        assert_eq!(database_to_release(&comment), None);
        assert_eq!(database_to_release(&close), None);
        assert_eq!(database_to_release(&PlanRequest::Create { def: ObjectDef::Database(d("n")), concurrently: false }), None);
    }

    fn mtarget(kind: crate::maintenance::MaintenanceTargetKind, schema: Option<&str>, name: &str) -> crate::maintenance::MaintenanceTarget {
        crate::maintenance::MaintenanceTarget {
            kind,
            schema: schema.map(Into::into),
            name: name.into(),
            table: None,
            partitioned: false,
            system: false,
            cluster_index: None,
            indexes: vec![],
            editable: true,
            unsupported: vec![],
        }
    }

    fn vacuum_op(full: bool) -> crate::maintenance::MaintenanceOp {
        crate::maintenance::MaintenanceOp::Vacuum(crate::maintenance::VacuumOptions {
            full,
            freeze: false,
            analyze: false,
            verbose: false,
            disable_page_skipping: false,
            skip_locked: false,
            index_cleanup: None,
            parallel: None,
        })
    }

    fn maintenance_req(object_kind: ObjectKind, target: crate::maintenance::MaintenanceTarget, full: bool) -> PlanRequest {
        PlanRequest::Maintenance { object: ObjectRef { kind: object_kind, oid: 3 }, target, op: vacuum_op(full) }
    }

    /// Plan 24: bakım isteği kapıları (saf kısım): op/kind, plan, salt okunur, biçim, hedef, nesne denetimi.
    #[test]
    fn maintenance_request_gates() {
        use crate::maintenance::MaintenanceTargetKind as K;
        let t = maintenance_req(ObjectKind::Table, mtarget(K::Table, Some("public"), "t"), false);
        assert_eq!((t.op(), t.kind()), ("maintenance", ObjectKind::Table));
        // Plan üreticiye bağlı: tek, transaction dışı ifade; FULL yıkıcıdır.
        let v = plan_view(&t, 170000);
        assert_eq!(v.statements.iter().map(|s| s.sql.as_str()).collect::<Vec<_>>(), ["VACUUM public.t"]);
        assert!(!v.statements[0].transactional && v.risk == Risk::Safe && v.errors.is_empty());
        let full = maintenance_req(ObjectKind::Table, mtarget(K::Table, Some("public"), "t"), true);
        assert_eq!(plan_view(&full, 170000).risk, Risk::Destructive);
        // Salt okunur hedef: genel gerekçe (ad içermez).
        let mut ro = mtarget(K::Index, Some("public"), "i");
        ro.editable = false;
        ro.unsupported = vec![crate::load_maintenance::PARTITIONED_INDEX_REASON.into()];
        let r = maintenance_req(ObjectKind::Index, ro, false);
        assert_eq!(read_only_refusal(&r).unwrap(), format!("{READ_ONLY_PREFIX}bölümlenmiş indeks"));
        let mut tmp = mtarget(K::Table, Some("pg_temp_3"), "tt");
        tmp.editable = false;
        tmp.unsupported = vec![crate::load_maintenance::TEMP_SCHEMA_REASON.into()];
        let m = read_only_refusal(&maintenance_req(ObjectKind::Table, tmp, false)).unwrap();
        assert!(m.starts_with(READ_ONLY_PREFIX) && !m.contains("pg_temp_3"), "{m}");
        assert_eq!(read_only_refusal(&t), None);
        // Biçim: nesne türü ile hedef türü uyuşmalı; bakımı olmayan tür ayrı iletiyle.
        assert_eq!(request_shape_refusal(&t), None);
        let mismatched = maintenance_req(ObjectKind::Index, mtarget(K::Table, Some("public"), "t"), false);
        assert_eq!(request_shape_refusal(&mismatched), Some(INVALID_DEF));
        let view = maintenance_req(ObjectKind::View, mtarget(K::Table, Some("public"), "t"), false);
        assert_eq!(request_shape_refusal(&view), Some(crate::load_maintenance::NO_MAINTENANCE_MSG));
        // Hedef: Database türü dahil her zaman bağlı veritabanı (bakım veritabanı değil); `None` → NEEDS_DATABASE.
        let db = maintenance_req(ObjectKind::Database, mtarget(K::Database, None, "d"), false);
        for r in [&t, &db] {
            assert_eq!(apply_target(r, Some(5)).unwrap(), Target::Database(5), "{r:?}");
            assert!(matches!(apply_target(r, None), Err(Error::Invalid(m)) if m == NEEDS_DATABASE), "{r:?}");
        }
        // Diğer istekler `target_for` kuralında kalır.
        let dbdrop = PlanRequest::Create { def: ObjectDef::Database(DatabaseDef {
            name: "n".into(), owner: None, template: None, encoding: None, locale_provider: None, lc_collate: None, lc_ctype: None,
            provider_locale: None, tablespace: None, connection_limit: -1, is_template: false, allow_connections: true, settings: vec![], comment: None,
        }), concurrently: false };
        assert_eq!(apply_target(&dbdrop, Some(5)).unwrap(), Target::Maintenance);
        // Nesne denetimi yükleyiciyle; bakım veritabanı koruması, bırakma, created/doğrulama yok.
        assert!(matches!(object_check(&db), Some(Expected::Maintenance(o, tg)) if o.kind == ObjectKind::Database && tg.name == "d"));
        assert_eq!(object_check(&t).and_then(|e| e.object()).map(|o| o.oid), Some(3));
        let named = maintenance_req(ObjectKind::Database, mtarget(K::Database, None, "postgres"), true);
        assert_eq!(maintenance_refusal(&named, "postgres"), None);
        assert_eq!(database_to_release(&named), None);
        assert!(VerifyAcl::of(&named).is_none());
        assert_eq!(HintCtx::of(&named), HintCtx::default());
        // JSON biçimi: `op = "maintenance"` (istek etiketi); işlem `operation` alanında (kendi `op` etiketiyle),
        // hedef camelCase.
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["op"], "maintenance");
        assert_eq!(v["operation"]["op"], "vacuum");
        assert_eq!(v["operation"]["full"], false);
        assert_eq!(v["target"]["clusterIndex"], serde_json::Value::Null);
        assert_eq!(serde_json::from_value::<PlanRequest>(v).unwrap(), t);
    }

    #[test]
    fn maintenance_hints() {
        let st = |sql: &str| Statement { sql: sql.into(), risk: Risk::Destructive, transactional: false, note: None, redacted: false };
        let hint = |kind, s: &Statement, code: &str, pg: Option<&str>| failure_payload_for(kind, "maintenance", Some(s), 0, &q(code, pg)).hint;
        let vac = st("VACUUM (FULL) public.t");
        // 55P03: kilit ipucu (genel lock_timeout ipucunun yerine).
        assert_eq!(hint(ObjectKind::Table, &vac, "55P03", None).as_deref(), Some(MAINTENANCE_LOCK_HINT));
        let p = failure_payload_for(ObjectKind::Table, "maintenance", Some(&vac), 0, &q("55P03", None));
        assert!(p.detail.is_some(), "kilit zaman aşımında PG iletisi detayda kalır");
        // 57014: ipucu yok (iptal iletisi yeter).
        assert_eq!(hint(ObjectKind::Table, &vac, "57014", None), None);
        for kind in [ObjectKind::Table, ObjectKind::Index, ObjectKind::Schema, ObjectKind::Database, ObjectKind::MaterializedView] {
            assert_eq!(hint(kind, &st("REINDEX TABLE public.t"), "42501", None).as_deref(), Some(MAINTENANCE_PRIVILEGE_HINT), "{kind:?}");
            assert_eq!(hint(kind, &st("REINDEX TABLE CONCURRENTLY public.t"), "0A000", None).as_deref(), Some(MAINTENANCE_UNSUPPORTED_HINT));
            assert_eq!(hint(kind, &st("VACUUM public.t"), "25001", None).as_deref(), Some(MAINTENANCE_TX_HINT));
            // Database türünde 55006 genel veritabanı ipucu bakımda verilmez.
            assert_eq!(hint(kind, &st("REINDEX DATABASE d"), "55006", None), None);
            // PG'nin kendi ipucu (55P03 dışında) korunur.
            assert_eq!(hint(kind, &vac, "42501", Some("pg")).as_deref(), Some("pg"));
        }
        // Diğer işlemler etkilenmez.
        assert_eq!(failure_payload_for(ObjectKind::Table, "alter", Some(&vac), 0, &q("42501", None)).hint.as_deref(), Some(PRIVILEGE_HINT));
        for h in [MAINTENANCE_LOCK_HINT, MAINTENANCE_PRIVILEGE_HINT, MAINTENANCE_UNSUPPORTED_HINT, MAINTENANCE_TX_HINT] {
            assert!(!h.is_empty() && !h.contains("public.t"));
        }
        assert_eq!(MAINTENANCE_LOCK_HINT, "Nesne başka bir oturumca kilitli; daha sonra deneyin ya da lock timeout'u artırın.");
    }

    /// İlerleme sorgusu sürüme göre görünüm seçer: PG 10–11'de cluster/create_index/analyze görünümleri hiç yok.
    #[test]
    fn progress_sql_follows_server_version() {
        let views = ["pg_stat_progress_vacuum", "pg_stat_progress_cluster", "pg_stat_progress_create_index", "pg_stat_progress_analyze"];
        let has = |v: u32| views.map(|name| progress_sql(v).contains(name));
        assert_eq!(has(100000), [true, false, false, false]);
        assert_eq!(has(110000), [true, false, false, false]);
        assert_eq!(has(120000), [true, true, true, false]);
        assert_eq!(has(130000), [true, true, true, true]);
        assert_eq!(has(180000), [true, true, true, true]);
        for v in [100000, 130000] {
            let sql = progress_sql(v);
            assert!(sql.matches("$1").count() >= 1 && !sql.trim_end().ends_with(';'), "{sql}");
        }
        // Satır → model: negatif/boş sayılar `None`.
        assert_eq!(progress_counts(Some(3), Some(10)), (Some(3), Some(10)));
        assert_eq!(progress_counts(Some(-1), None), (None, None));
    }

    /// Bakım isteğinin kapıları bağlantı açmadan çalışır (kilit, salt okunur, biçim, NEEDS_DATABASE); ilerleme bilinmeyen
    /// kimlikte bağlantısız da `None`.
    #[tokio::test]
    async fn maintenance_gates_refuse_before_connecting() {
        use crate::error_codes::ddl_payload;
        use crate::maintenance::MaintenanceTargetKind as K;
        use pgt_core::config::ServerConfig;
        let make = |locked: bool| {
            let dir = tempfile::tempdir().unwrap();
            let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
            let mut config = ServerConfig::new("x", "127.0.0.1", 1, "u");
            config.read_only_lock = locked;
            let server = config.id;
            registry.upsert(config).unwrap();
            (DdlManager::new(Arc::new(ConnectionManager::new()), Arc::new(RwLock::new(registry))), server, dir)
        };
        let o = || ApplyOptions {
            apply_id: Uuid::new_v4(),
            lock_timeout_ms: 5000,
            expected_hash: String::new(),
            confirmed: true,
            confirmed_production: true,
            password: None,
        };
        let req = maintenance_req(ObjectKind::Table, mtarget(K::Table, Some("public"), "t"), false);
        let (locked, server, _d1) = make(true);
        assert_eq!(ddl_payload(&locked.apply(server, Some(1), &req, o()).await.unwrap_err()).code, "locked");
        let (dm, server, _d2) = make(false);
        let db = maintenance_req(ObjectKind::Database, mtarget(K::Database, None, "d"), false);
        assert!(matches!(dm.apply(server, None, &db, o()).await, Err(Error::Invalid(m)) if m == NEEDS_DATABASE));
        let mut ro = mtarget(K::Table, Some("public"), "t");
        ro.editable = false;
        let m = match dm.apply(server, Some(1), &maintenance_req(ObjectKind::Table, ro, false), o()).await {
            Err(Error::Invalid(m)) => m,
            other => panic!("{other:?}"),
        };
        assert!(m.starts_with(READ_ONLY_PREFIX));
        // Geçerli istek kapıları geçer: bağlantısız sunucuda sürüm okunamaz.
        assert_eq!(ddl_payload(&dm.apply(server, Some(1), &db, o()).await.unwrap_err()).code, "not_connected");
        // İlerleme: kayıtlı olmayan kimlik bağlantı açmadan `None`.
        assert_eq!(dm.maintenance_progress(server, 1, Uuid::new_v4()).await.unwrap(), None);
    }

    #[test]
    fn membership_comparison_ignores_read_only_options() {
        let mut a = role("a");
        a.member_of = vec![RoleMembership { role: "g".into(), admin: false, inherit_option: None, set_option: None }];
        let mut b = a.clone();
        b.member_of[0].inherit_option = Some(true);
        b.member_of[0].set_option = Some(false);
        assert!(!memberships_differ(&a, &b));
        b.member_of[0].admin = true;
        assert!(memberships_differ(&a, &b));
        b.member_of.clear();
        assert!(memberships_differ(&a, &b));
        // Başka grantor ile kaydedilmiş (salt okunur) üyelik var sayılır.
        b.unsupported = vec![crate::load::grantor_label("g", "creator")];
        assert!(!memberships_differ(&a, &b));
        b.unsupported = vec![crate::load::multi_grantor_label("g")];
        assert!(!memberships_differ(&a, &b));
        // İstenmeyen ama kalan salt okunur üyelik fark sayılır.
        let mut none = a.clone();
        none.member_of.clear();
        assert!(memberships_differ(&none, &b));
        b.unsupported = vec!["IN DATABASE d: work_mem=1".into()];
        assert!(memberships_differ(&a, &b));
    }

    /// Docker testleri (`PGT_TEST_URLS` yoksa atlanır): crate içi yardımcıları (`apply_statements`,
    /// `open_apply_conn`, `prepare_session`) kullandıkları için birim testi olarak buradadır.
    mod docker {
        use super::*;
        use crate::apply::{open_apply_conn, prepare_session};
        use pgt_core::connection::SessionProfile;
        use pgt_core::testing::{TestServer, admin_client, drop_database, test_servers};

        fn st(sql: &str, transactional: bool) -> Statement {
            Statement { sql: sql.into(), risk: Risk::Safe, transactional, note: None, redacted: false }
        }

        async fn db_name(ts: &TestServer, tag: &str) -> String {
            let admin = admin_client(ts, &ts.config.maintenance_db).await;
            let v: i32 = admin.query_one("SELECT current_setting('server_version_num')::int4", &[]).await.unwrap().get(0);
            format!("pgt_ddl_mu_{}_{tag}", v / 10_000)
        }

        async fn cleanup(ts: &TestServer, tag: &str) {
            let admin = admin_client(ts, &ts.config.maintenance_db).await;
            drop_database(&admin, &db_name(ts, tag).await).await;
        }

        /// Veritabanını kurar; yönetici, `DdlManager`, veritabanı oid'i ve geçici dizini döndürür.
        async fn setup(ts: &TestServer, tag: &str) -> (tokio_postgres::Client, DdlManager, u32, tempfile::TempDir) {
            cleanup(ts, tag).await;
            let name = db_name(ts, tag).await;
            let maint = admin_client(ts, &ts.config.maintenance_db).await;
            maint.batch_execute(&format!("CREATE DATABASE {name} TEMPLATE template0")).await.unwrap();
            let oid: u32 = maint.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&name]).await.unwrap().get(0);
            let dir = tempfile::tempdir().unwrap();
            let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
            registry.upsert(ts.config.clone()).unwrap();
            let mgr = Arc::new(ConnectionManager::new());
            mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
            (admin_client(ts, &name).await, DdlManager::new(mgr, Arc::new(RwLock::new(registry))), oid, dir)
        }

        async fn guarded(ts: &TestServer, tag: &str, body: impl std::future::Future<Output = ()> + Send + 'static) {
            let result = tokio::spawn(body).await;
            cleanup(ts, tag).await;
            if let Err(e) = result {
                std::panic::resume_unwind(e.into_panic());
            }
        }

        async fn exists(admin: &tokio_postgres::Client, sql: &str) -> bool {
            admin.query_one(&format!("SELECT ({sql}) IS NOT NULL"), &[]).await.unwrap().get(0)
        }

        async fn show(client: &tokio_postgres::Client, name: &str) -> String {
            client.query_one(&format!("SHOW {name}"), &[]).await.unwrap().get(0)
        }

        #[tokio::test]
        async fn mixed_plan_reports_partial_commit() {
            for ts in test_servers() {
                let tag = "mixed";
                let t = ts.clone();
                guarded(&ts, tag, async move {
                    let (admin, dm, db, _dir) = setup(&t, tag).await;
                    let plan = [
                        st("CREATE TABLE public.mx_a (id int)", true),
                        st("CREATE INDEX CONCURRENTLY mx_i ON public.mx_a (id)", false),
                        st("CREATE TABLE public.mx_b (id int)", true),
                        st("CREATE TABLE public.mx_a (id int)", true),
                    ];
                    let out = dm.apply_statements(t.config.id, Target::Database(db), &plan, 5000, None).await.unwrap();
                    let ApplyOutcome::Failed { failed_index, committed, sqlstate, .. } = out else { panic!("{out:?}") };
                    assert_eq!((failed_index, committed, sqlstate.as_deref()), (3, 2, Some("42P07")));
                    assert!(exists(&admin, "to_regclass('public.mx_a')").await);
                    assert!(exists(&admin, "to_regclass('public.mx_i')").await);
                    assert!(!exists(&admin, "to_regclass('public.mx_b')").await, "başarısız blok geri alınmalı");

                    // transactional blok → hata veren CONCURRENTLY (aynı ad): önceki blok kalıcı.
                    let plan = [
                        st("CREATE TABLE public.mx_c (id int)", true),
                        st("CREATE INDEX CONCURRENTLY mx_i ON public.mx_a (id)", false),
                    ];
                    let out = dm.apply_statements(t.config.id, Target::Database(db), &plan, 5000, None).await.unwrap();
                    let ApplyOutcome::Failed { failed_index, committed, .. } = out else { panic!("{out:?}") };
                    assert_eq!((failed_index, committed), (1, 1));
                    assert!(exists(&admin, "to_regclass('public.mx_c')").await);

                    // Başarılı karışık plan.
                    let plan = [st("CREATE TABLE public.mx_d (id int)", true), st("CREATE INDEX CONCURRENTLY mx_j ON public.mx_d (id)", false)];
                    let out = dm.apply_statements(t.config.id, Target::Database(db), &plan, 5000, None).await.unwrap();
                    assert!(matches!(out, ApplyOutcome::Applied { statements: 2, .. }), "{out:?}");
                    assert_eq!(dm.cancels.len(), 0);
                })
                .await;
            }
        }

        /// Gizlenmiş (parola) ifadesi başarısız olursa konum verilmez ve hata yükü parolayı/doğrulayıcıyı içermez.
        #[tokio::test]
        async fn redacted_failure_hides_position() {
            for ts in test_servers() {
                let tag = "redact";
                let t = ts.clone();
                guarded(&ts, tag, async move {
                    let (_admin, dm, _db, _dir) = setup(&t, tag).await;
                    let secret = Secret::new("gizli-parola-Z9");
                    let plan = [crate::build::role::password_redacted("pgt_yok_rol")];
                    let out = dm.apply_statements(t.config.id, Target::Maintenance, &plan, 5000, Some(("pgt_yok_rol", &secret))).await.unwrap();
                    let ApplyOutcome::Failed { sqlstate, position, error, committed, .. } = &out else { panic!("{out:?}") };
                    assert_eq!((sqlstate.as_deref(), *position, *committed), (Some("42704"), None, 0));
                    for text in [Some(error.message.as_str()), error.detail.as_deref(), error.hint.as_deref()].into_iter().flatten() {
                        assert!(!text.contains("gizli-parola-Z9") && !text.contains("SCRAM-SHA-256$"), "{text}");
                    }
                    let dbg = format!("{out:?}");
                    assert!(!dbg.contains("gizli-parola-Z9") && !dbg.contains("SCRAM-SHA-256$"), "{dbg}");
                    // Konum verebilecek bir hata (sözdizimi) gizlenmemiş ifadede konumu korur.
                    let bad = [st("SELEC 1", true)];
                    let out = dm.apply_statements(t.config.id, Target::Maintenance, &bad, 5000, None).await.unwrap();
                    assert!(matches!(out, ApplyOutcome::Failed { position: Some(_), .. }), "{out:?}");
                })
                .await;
            }
        }

        fn outcome_messages(out: &ApplyOutcome) -> &[ServerMessage] {
            match out {
                ApplyOutcome::Applied { messages, .. } | ApplyOutcome::Failed { messages, .. } => messages,
            }
        }

        /// NOTICE'lar sonuca girer: tek ileti, sınır (600 → 500), uzun ileti kesimi, başarısız ifade.
        #[tokio::test]
        async fn apply_collects_server_messages() {
            for ts in test_servers() {
                let tag = "msgs";
                let t = ts.clone();
                guarded(&ts, tag, async move {
                    let (_admin, dm, db, _dir) = setup(&t, tag).await;
                    let target = Target::Database(db);
                    let plan = [st("DO $$ BEGIN RAISE NOTICE 'x'; END $$", true)];
                    let out = dm.apply_statements(t.config.id, target, &plan, 5000, None).await.unwrap();
                    assert!(matches!(out, ApplyOutcome::Applied { .. }), "{out:?}");
                    let m = outcome_messages(&out);
                    assert_eq!(m.len(), 1, "{m:?}");
                    assert_eq!((m[0].severity.as_str(), m[0].code.as_deref(), m[0].message.as_str()), ("NOTICE", Some("00000"), "x"));

                    // İleti yoksa boş (alan yazılmaz).
                    let out = dm.apply_statements(t.config.id, target, &[st("SELECT 1", true)], 5000, None).await.unwrap();
                    assert!(outcome_messages(&out).is_empty(), "{out:?}");

                    // 600 NOTICE (transaction dışı ifade): 499 + taşma iletisi.
                    let plan = [st("DO $$ BEGIN FOR i IN 1..600 LOOP RAISE NOTICE 'n%', i; END LOOP; END $$", false)];
                    let out = dm.apply_statements(t.config.id, target, &plan, 5000, None).await.unwrap();
                    let m = outcome_messages(&out);
                    assert_eq!(m.len(), 500);
                    assert_eq!(m[0].message, "n1");
                    assert_eq!(m[498].message, "n499");
                    assert_eq!(m[499].message, "101 ileti daha gösterilmedi");

                    // Uzun ileti 4096 bayta kesilir.
                    let plan = [st("DO $$ BEGIN RAISE NOTICE '%', repeat('ğ', 5000); END $$", true)];
                    let out = dm.apply_statements(t.config.id, target, &plan, 5000, None).await.unwrap();
                    let m = outcome_messages(&out);
                    assert!(m[0].message.len() <= 4096 && m[0].message.ends_with('…'), "{}", m[0].message.len());

                    // Başarısız ifadede de hataya kadar gelen iletiler döner.
                    let plan = [st("DO $$ BEGIN RAISE NOTICE 'once'; END $$", true), st("SELEC 1", true)];
                    let out = dm.apply_statements(t.config.id, target, &plan, 5000, None).await.unwrap();
                    assert!(matches!(out, ApplyOutcome::Failed { failed_index: 1, .. }), "{out:?}");
                    assert_eq!(outcome_messages(&out).iter().map(|m| m.message.as_str()).collect::<Vec<_>>(), ["once"]);
                })
                .await;
            }
        }

        /// İlerleme sorgularının sayaç seçimi aşamaya göredir: görünüm yerine sabit satırlarla (`VALUES`) sunucuda
        /// çalıştırılır (görünümlere satır eklenemez; her sürümde aynı). `(aşama, done, total, unit)` beklenir.
        #[tokio::test]
        async fn progress_counters_follow_phase() {
            type Row = (String, Option<i64>, Option<i64>, Option<String>);
            async fn run(c: &tokio_postgres::Client, sql: &str, view: &str, values: &str) -> Vec<Row> {
                let from = format!("FROM {view} p");
                assert!(sql.contains(&from), "{view}");
                let sql = sql.replace(&from, &format!("FROM ({values}) p")).replace("p.pid = $1", "true");
                let rows = c.query(sql.as_str(), &[]).await.unwrap_or_else(|e| panic!("{view}: {e}"));
                rows.iter().map(|r| (r.get(0), r.get(1), r.get(2), r.get(3))).collect()
            }
            let row = |phase: &str, done: Option<i64>, total: Option<i64>, unit: Option<&str>| -> Row {
                (phase.into(), done, total, unit.map(Into::into))
            };
            for ts in test_servers() {
                let c = admin_client(&ts, &ts.config.maintenance_db).await;
                // create_index: tarama aşamalarında blok; ağaca yüklemede satır (eski blok sayacı dolu kalsa da).
                let ci = "SELECT * FROM (VALUES \
                    ('building index: scanning table'::text, 10::int8, 4::int8, 0::int8, 0::int8), \
                    ('building index: loading tuples in tree', 10, 10, 500, 120), \
                    ('index validation: scanning index', 8, 2, 0, 0), \
                    ('index validation: sorting tuples', 8, 8, 0, 0), \
                    ('waiting for writers before build', 0, 0, 0, 0), \
                    ('building index: scanning table', 0, 0, 0, 0), \
                    (NULL, NULL, NULL, NULL, NULL)) v(phase, blocks_total, blocks_done, tuples_total, tuples_done)";
                let got = run(&c, PROGRESS_CREATE_INDEX_SQL, "pg_stat_progress_create_index", ci).await;
                assert_eq!(
                    got,
                    [
                        row("building index: scanning table", Some(4), Some(10), Some("blocks")),
                        row("building index: loading tuples in tree", Some(120), Some(500), Some("tuples")),
                        row("index validation: scanning index", Some(2), Some(8), Some("blocks")),
                        row("index validation: sorting tuples", None, None, None),
                        row("waiting for writers before build", None, None, None),
                        row("building index: scanning table", None, None, None),
                        row("unknown", None, None, None),
                    ]
                );
                // vacuum: taramada taranan, yığın vakumunda vakumlanan blok; diğer aşamalarda belirsiz.
                let vac = "SELECT * FROM (VALUES \
                    ('scanning heap'::text, 100::int8, 40::int8, 0::int8), \
                    ('vacuuming heap', 100, 100, 30), \
                    ('vacuuming indexes', 100, 100, 0), \
                    ('scanning heap', NULL, NULL, NULL), \
                    (NULL, NULL, NULL, NULL)) v(phase, heap_blks_total, heap_blks_scanned, heap_blks_vacuumed)";
                let got = run(&c, PROGRESS_VACUUM_SQL, "pg_stat_progress_vacuum", vac).await;
                assert_eq!(
                    got,
                    [
                        row("scanning heap", Some(40), Some(100), Some("blocks")),
                        row("vacuuming heap", Some(30), Some(100), Some("blocks")),
                        row("vacuuming indexes", None, None, None),
                        row("scanning heap", None, None, None),
                        row("unknown", None, None, None),
                    ]
                );
            }
        }

        /// Bakım uygulaması sürerken yuvada arka uç pid'i vardır (uygulama bağlantısının pid'i), bitince düşer.
        /// Diğer DDL uygulamaları pid okumaz (fazladan gidiş-dönüş yok).
        #[tokio::test]
        async fn backend_pid_is_recorded_during_apply() {
            for ts in test_servers() {
                let tag = "pid";
                let t = ts.clone();
                guarded(&ts, tag, async move {
                    let (admin, dm, db, _dir) = setup(&t, tag).await;
                    let plan = [st("SELECT pg_sleep(0.5)", false)];
                    // Bakım dışı işlem: uygulama süresince pid kaydedilmez.
                    let other = Uuid::new_v4();
                    let apply = dm.apply_statements_as(other, "alter", t.config.id, Target::Database(db), &plan, 5000, None);
                    // Yuva açılana dek, sonra yuva düşene dek yoklanır (sınırlı süre).
                    let watch = async {
                        let (mut registered, mut seen) = (false, false);
                        let deadline = Instant::now() + Duration::from_secs(10);
                        while Instant::now() < deadline {
                            let open = dm.cancels.len() > 0;
                            if registered && !open {
                                break;
                            }
                            registered |= open;
                            seen |= dm.cancels.backend_pid(other).is_some();
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        (registered, seen)
                    };
                    let (out, (registered, seen)) = tokio::join!(apply, watch);
                    assert!(matches!(out, Ok(ApplyOutcome::Applied { .. })), "{out:?}");
                    assert!(registered, "uygulama yuvası görülmedi");
                    assert!(!seen, "bakım dışı uygulamada pid okunmamalı");

                    let id = Uuid::new_v4();
                    let plan = [st("SELECT pg_sleep(1.5)", false)];
                    let apply = dm.apply_statements_as(id, "maintenance", t.config.id, Target::Database(db), &plan, 5000, None);
                    let watch = async {
                        for _ in 0..100 {
                            if let Some(pid) = dm.cancels.backend_pid(id) {
                                return Some(pid);
                            }
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        None
                    };
                    let (out, pid) = tokio::join!(apply, watch);
                    assert!(matches!(out, Ok(ApplyOutcome::Applied { .. })), "{out:?}");
                    let pid = pid.expect("uygulama sırasında pid kaydedilmeli");
                    assert!(pid > 0);
                    assert_eq!(dm.cancels.backend_pid(id), None, "yuva düşünce pid de düşer");
                    // Pid uygulama bağlantısınındı: artık o arka uç yok (kısa bekleme: kapanış eşzamansız).
                    let mut gone = false;
                    for _ in 0..50 {
                        let n: i64 = admin.query_one("SELECT count(*) FROM pg_stat_activity WHERE pid = $1", &[&pid]).await.unwrap().get(0);
                        if n == 0 {
                            gone = true;
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    assert!(gone);
                })
                .await;
            }
        }

        /// İptal (57014): hataya kadar gelen iletiler sonuçta kalır, pid yuvayla birlikte düşer (ilerleme `None`).
        #[tokio::test]
        async fn cancel_keeps_messages_and_clears_pid() {
            for ts in test_servers() {
                let tag = "cnmsg";
                let t = ts.clone();
                guarded(&ts, tag, async move {
                    let (admin, dm, db, _dir) = setup(&t, tag).await;
                    let dm = Arc::new(dm);
                    let (id, server) = (Uuid::new_v4(), t.config.id);
                    let dm2 = dm.clone();
                    let task = tokio::spawn(async move {
                        let plan = [st("DO $$ BEGIN RAISE NOTICE 'once'; PERFORM pg_sleep(30); END $$", false)];
                        dm2.apply_statements_as(id, "maintenance", server, Target::Database(db), &plan, 5000, None).await
                    });
                    // İfade uykuda (NOTICE gönderilmiş) olana dek bekle.
                    let mut sleeping = false;
                    for _ in 0..300 {
                        if let Some(pid) = dm.cancels.backend_pid(id) {
                            let n: i64 = admin
                                .query_one("SELECT count(*) FROM pg_stat_activity WHERE pid = $1 AND wait_event = 'PgSleep'", &[&pid])
                                .await
                                .unwrap()
                                .get(0);
                            if n == 1 {
                                sleeping = true;
                                break;
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    // Başarısızlıkta da iptal gönderilir ve görev sınırlı beklenir: temizlik asılı kalmasın.
                    dm.cancel(id).await.unwrap();
                    let joined = tokio::time::timeout(Duration::from_secs(15), task).await;
                    assert!(sleeping, "ifade uykuya geçmedi");
                    let out = joined.expect("iptal 15 sn içinde sonuçlanmalı").unwrap().unwrap();
                    let ApplyOutcome::Failed { sqlstate, messages, error, .. } = &out else { panic!("{out:?}") };
                    assert_eq!(sqlstate.as_deref(), Some("57014"));
                    assert_eq!(error.hint, None);
                    assert_eq!(messages.iter().map(|m| m.message.as_str()).collect::<Vec<_>>(), ["once"]);
                    assert_eq!(dm.cancels.backend_pid(id), None);
                    assert_eq!(dm.maintenance_progress(server, db, id).await.unwrap(), None);
                })
                .await;
            }
        }

        #[tokio::test]
        async fn applier_sets_session_explicitly() {
            for ts in test_servers() {
                let tag = "sets";
                let t = ts.clone();
                guarded(&ts, tag, async move {
                    let (_admin, dm, db, _dir) = setup(&t, tag).await;
                    // Uygulayıcının bağlantı yolu.
                    let conn = open_apply_conn(&dm.mgr, t.config.id, Target::Database(db), 4321).await.unwrap();
                    assert_eq!(show(&conn.client, "lock_timeout").await, "4321ms");
                    assert_eq!(show(&conn.client, "statement_timeout").await, "0");
                    assert_eq!(show(&conn.client, "default_transaction_read_only").await, "off");
                    assert_eq!(show(&conn.client, "standard_conforming_strings").await, "on");
                    assert_eq!(show(&conn.client, "application_name").await, "pg-tools ddl");

                    // Başlangıç ayarları yok sayılmış/farklı bir oturumda bile açık SET'ler değerleri kurar.
                    let other = dm
                        .mgr
                        .open_dedicated(t.config.id, Target::Database(db), SessionProfile::Query { read_only: true, statement_timeout_ms: 1234 })
                        .await
                        .unwrap();
                    other.client.batch_execute("SET standard_conforming_strings = off; SET lock_timeout = 77").await.unwrap();
                    prepare_session(&other.client, 0).await.unwrap();
                    assert_eq!(show(&other.client, "lock_timeout").await, "0");
                    assert_eq!(show(&other.client, "statement_timeout").await, "0");
                    assert_eq!(show(&other.client, "default_transaction_read_only").await, "off");
                    assert_eq!(show(&other.client, "standard_conforming_strings").await, "on");
                })
                .await;
            }
        }
    }
}
