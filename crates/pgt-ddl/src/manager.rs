//! DDL yöneticisi: önizleme, kapılı uygulama (kilit/geçerlilik/hash/onay), iptal, yükleme, varsayılanlar ve açılır listeler.
//!
//! Kapı sırası (`apply`; yalnızca (4) havuzdaki katalog bağlantısında tek gidiş-dönüş yapar, hiçbiri uygulama
//! bağlantısı açmaz): (1) kayıtsız sunucu, (2) `read_only_lock`, (2a) hedef çözümü (`target_for`: Database/Role →
//! bakım veritabanı; veritabanı kapsamlı türde `database_oid` yok → `Invalid`), (3) plan üretimi (geçersiz tanım /
//! boş plan), (4) nesne denetimi (yalnızca alter/drop: istemcinin `old`/`def`'i, `object`'in sunucudaki güncel
//! tanımına eşit olmalı; fark → `plan_changed`; yükleme hataları — yok/sistem nesnesi/identity dizisi — olduğu gibi
//! geçer), (4a) bakım veritabanı koruması (yeniden adlandırma/taşıma/silme/bağlantı kapatma → `Invalid`), (5) hash
//! uyuşmazlığı, (6) yıkıcı plan onayı, (7) üretim sunucusunda veri kaybı onayı, (8) `lock_timeout` sınırı,
//! (9) parola kapısı (gizlenmiş ifade ↔ `ApplyOptions.password`), (10) somutlaştırma (gizlenmiş ifade yalnızca
//! çalıştırma listesinde istemci tarafı SCRAM doğrulayıcısıyla kurulur; `PlanView`'e, özete ve günlüğe girmez).
//! Plan istemciden alınmaz; her seferinde tanımdan yeniden üretilir. `preview` nesne denetimi yapmaz.
//!
//! Günlük yalnızca işlem/nesne türü, ifade sayısı, süre ve sonuç kodunu içerir; SQL, ad, değer ve parola asla.

use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use pgt_core::connection::{ConnectionManager, MAX_STATEMENT_TIMEOUT_MS, Target};
use pgt_core::registry::Registry;
use pgt_core::{Error, ErrorPayload, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::apply::{self, CancelMap, ExecSql, ExecStatement, RunResult};
use crate::build::{self, BuildCtx, role};
use crate::error_codes::{
    CONFIRMATION_REQUIRED, INVALID_DEF, LOCKED_MESSAGE, MAINTENANCE_DATABASE, MAINTENANCE_TEMPLATE, NO_CHANGES,
    PASSWORD_NOT_USED,
    PLAN_CHANGED, PRODUCTION_CONFIRMATION_REQUIRED, ROLE_PASSWORD_MISSING,
};
use crate::load::{self, ObjectRef};
use crate::lookup::{self, LookupItem, LookupRequest};
use crate::model::*;
use crate::plan::{PlanView, Risk, Statement};
use crate::secret::Secret;

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
}

impl PlanRequest {
    fn op(&self) -> &'static str {
        match self {
            PlanRequest::Create { .. } => "create",
            PlanRequest::Alter { .. } => "alter",
            PlanRequest::Drop { .. } => "drop",
        }
    }

    fn kind(&self) -> ObjectKind {
        match self {
            PlanRequest::Create { def, .. } | PlanRequest::Drop { def, .. } => def.kind(),
            PlanRequest::Alter { new, .. } => new.kind(),
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
const CANCEL_RESENDS: u32 = 3;
const CANCEL_RESEND_AFTER: Duration = Duration::from_millis(200);

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
/// Kısmi uygulama (transaction dışı ifadeler): ilk ifadeler kalıcı.
pub const PARTIAL_APPLY_HINT: &str =
    "Önceki ifadeler uygulandı (transaction dışı); nesneyi yenileyip kalan değişiklikleri yeniden deneyin.";
/// Başarısız gizlenmiş ifadenin sunucu iletisi doğrulayıcı içeriyorsa yerine geçen sabit ileti (savunma).
const REDACTED_FAILURE_MESSAGE: &str = "Parola ifadesi uygulanamadı.";
/// PG 16+: GRANTED BY'sız REVOKE başka grantor'ın üyeliğini kaldırmaz (WARNING ile etkisiz); grant başka grantor ile
/// kaydedilebilir. Yeniden yüklenen üyelikler istenenden farklıysa.
pub const MEMBERSHIP_WARNING: &str = "Üyelikler uygulama sonrasında istenenden farklı: bazı GRANT/REVOKE işlemleri başka bir \
     grantor'a ait üyelikleri etkilemedi (PG 16+). Rolü yeniden yükleyip kontrol edin.";

/// Çağıran gelecek düşerse sunucudaki ifadeyi iptal eder (`armed` yalnızca çalıştırıcı bitince kapanır).
struct CancelOnDrop {
    handle: pgt_core::connection::CancelHandle,
    rt: tokio::runtime::Handle,
    armed: bool,
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
        _ => false,
    }
}

/// Uygulama hedefi: yükleyiciyle aynı tek kural (`load::target_for`).
pub(crate) use crate::load::target_for;

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
fn hint_for(kind: ObjectKind, sql: &str, code: &str) -> Option<&'static str> {
    Some(match (code, kind) {
        ("42P16" | "42804", ObjectKind::View) if sql.starts_with("CREATE OR REPLACE VIEW ") => VIEW_COLUMNS_HINT,
        ("55000", ObjectKind::MaterializedView) if sql.starts_with("REFRESH MATERIALIZED VIEW CONCURRENTLY ") => {
            REFRESH_CONCURRENTLY_HINT
        }
        ("55006", ObjectKind::Database) if sql.starts_with("CREATE DATABASE ") => TEMPLATE_IN_USE_HINT,
        ("55006", ObjectKind::Database) if sql.starts_with("DROP DATABASE ") && sql.ends_with(" WITH (FORCE)") => FORCE_LIMITS_HINT,
        ("55006", ObjectKind::Database) => DATABASE_IN_USE_HINT,
        ("42501", _) => PRIVILEGE_HINT,
        ("2BP01", ObjectKind::Role) => ROLE_DEPENDENTS_HINT,
        ("2BP01", ObjectKind::View | ObjectKind::MaterializedView) => VIEW_DEPENDENTS_HINT,
        _ => return None,
    })
}

/// Başarısız ifadenin arayüz yükü: `apply::failure_payload` + (PG ipucu yoksa) türe/ifadeye göre ipucu; kısmi
/// uygulamada (`committed > 0`, transaction dışı) ipucu yoksa bunu söyler. Gizlenmiş ifadede sunucu iletisi
/// doğrulayıcı izi taşıyorsa sabit iletiyle değiştirilir (savunma; konum zaten `None`).
pub(crate) fn failure_payload_for(kind: ObjectKind, failed: Option<&Statement>, committed: u32, e: &Error) -> ErrorPayload {
    let mut p = apply::failure_payload(e);
    if failed.is_some_and(|s| s.redacted) {
        let leaks = |v: &Option<String>| v.as_deref().is_some_and(|t| t.contains("SCRAM-SHA-256$"));
        if p.message.contains("SCRAM-SHA-256$") || leaks(&p.detail) || leaks(&p.hint) {
            p.message = REDACTED_FAILURE_MESSAGE.into();
            p.detail = None;
            p.hint = None;
        }
    }
    if p.hint.is_none()
        && let (Some(s), Some(code)) = (failed, apply::sqlstate(e))
    {
        p.hint = hint_for(kind, &s.sql, code).map(str::to_string);
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

const SCHEMA_NAME_SQL: &str = "SELECT nspname::text AS name FROM pg_namespace WHERE oid = $1";
const TABLE_NAME_SQL: &str = "SELECT n.nspname::text AS schema, c.relname::text AS name \
     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = $1 AND c.relkind IN ('r', 'm', 'p')";

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
        // (2a)
        let target = target_for(req.kind(), database_oid)?;
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
        if let Some((object, expected)) = object_check(req) {
            if object.kind != expected.kind() {
                return Err(Error::Invalid(INVALID_DEF.into()));
            }
            let loaded = load::load(&self.mgr, server, database_oid, object).await?;
            if loaded != *expected {
                return Err(Error::Invalid(PLAN_CHANGED.into()));
            }
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
            object: object_check(req).map(|(o, _)| o),
            release: self.databases_to_release(server, req).await,
            verify_role,
        };
        self.execute(server, &view.statements, exec, job).await
    }

    /// Bağlantı açar, ifadeleri blok blok çalıştırır, sonucu günlüğe (yalnızca tür/sayı/süre/sonuç) yazar.
    /// `statements` plandaki (gizlenmiş) metinlerdir: ipucu/konum kararları bunlara bakar; `exec` çalıştırılan listedir.
    async fn execute(&self, server: Uuid, statements: &[Statement], exec: Vec<ExecStatement<'_>>, job: Job) -> Result<ApplyOutcome> {
        let Job { op, kind, target, lock_timeout_ms, apply_id, created, object, release, verify_role } = job;
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
        reg.set_handle(conn.cancel.clone());
        // Çağıran gelecek çalışırken düşerse (ör. pencere kapanışı) sunucudaki ifade iptal edilir.
        let mut guard = CancelOnDrop { handle: conn.cancel.clone(), rt, armed: true };
        let result = apply::run_statements(&conn.client, &exec, &reg).await;
        guard.armed = false;
        drop(reg);
        // Somutlaşmış parola ifadesi (varsa) burada sıfırlanarak düşer.
        drop(exec);
        // Uygulama bağlantısı, havuzdaki katalog sorgusundan (`find_created`) önce kapanır.
        drop(conn);
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
                ApplyOutcome::Applied { statements: count, elapsed_ms, created, warnings: (!warnings.is_empty()).then_some(warnings) }
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
                    failure_payload_for(kind, failed, committed, &error)
                };
                // Kısmi uygulama: oluşturulan/yeniden adlandırılan nesne yine de bulunur (ör. CREATE DATABASE kalıcı).
                let created = match (&created, committed) {
                    (Some(def), 1..) => self.find_created(server, target, def).await,
                    _ => None,
                };
                ApplyOutcome::Failed { failed_index, committed, sqlstate, position, error, created }
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
        let exec = materialize_for(statements, password.map(|p| p.0), password.map(|p| p.1))?;
        let job = Job {
            op: "test",
            kind: ObjectKind::Schema,
            target,
            lock_timeout_ms,
            apply_id: Uuid::new_v4(),
            created: None,
            object: None,
            release: vec![],
            verify_role: None,
        };
        self.execute(server, statements, exec, job).await
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
        let (kind, sql, schema, name) = match def {
            ObjectDef::Schema(d) => (ObjectKind::Schema, FIND_SCHEMA_SQL, None, d.name.clone()),
            ObjectDef::Extension(d) => (ObjectKind::Extension, FIND_EXTENSION_SQL, None, d.name.clone()),
            ObjectDef::Sequence(d) => (ObjectKind::Sequence, FIND_SEQUENCE_SQL, Some(d.schema.clone()), d.name.clone()),
            ObjectDef::Index(d) => (ObjectKind::Index, FIND_INDEX_SQL, Some(d.schema.clone()), d.name.clone()),
            ObjectDef::Table(d) => (ObjectKind::Table, FIND_TABLE_SQL, Some(d.schema.clone()), d.name.clone()),
            ObjectDef::View(d) => (ObjectKind::View, FIND_VIEW_SQL, Some(d.schema.clone()), d.name.clone()),
            ObjectDef::MaterializedView(d) => (ObjectKind::MaterializedView, FIND_MATVIEW_SQL, Some(d.schema.clone()), d.name.clone()),
            ObjectDef::Database(d) => (ObjectKind::Database, FIND_DATABASE_SQL, None, d.name.clone()),
            ObjectDef::Role(d) => (ObjectKind::Role, FIND_ROLE_SQL, None, d.name.clone()),
        };
        let row = self
            .mgr
            .run(server, target, move |c| {
                let (schema, name) = (schema.clone(), name.clone());
                async move {
                    match schema {
                        Some(s) => c.client.query_opt(sql, &[&s, &name]).await,
                        None => c.client.query_opt(sql, &[&name]).await,
                    }
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

    /// Açılır liste; `database_oid = None` yalnızca sunucu genelindeki isteklerde (`LookupRequest::server_wide`).
    pub async fn lookup(&self, server: Uuid, database_oid: impl Into<Option<u32>> + Send, req: LookupRequest) -> Result<Vec<LookupItem>> {
        lookup::lookup(&self.mgr, server, database_oid, req).await
    }

    /// "Yeni …" diyaloğunun önceden doldurulmuş tanımı. Şema/tablo adları oid'den katalogdan çözülür.
    /// Geçersiz bağlam (sequence/tablo/görünüm için şema yok ya da tablo var, indeks için tablo yok, şema/eklenti için
    /// tablo, veritabanı/rol için herhangi bir bağlam) → `Invalid`; veritabanı kapsamlı türde `database_oid` yok →
    /// `Invalid(NEEDS_DATABASE)`.
    pub async fn defaults(
        &self,
        server: Uuid,
        database_oid: impl Into<Option<u32>> + Send,
        kind: ObjectKind,
        ctx: CreateContext,
    ) -> Result<ObjectDef> {
        let invalid = |m: &str| Err(Error::Invalid(m.to_string()));
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
}

/// Sunucudaki güncel tanımla karşılaştırılacak (nesne, istemci tanımı); `Create` için yok.
fn object_check(req: &PlanRequest) -> Option<(ObjectRef, &ObjectDef)> {
    match req {
        PlanRequest::Create { .. } => None,
        PlanRequest::Alter { object, old, .. } => Some((*object, old)),
        PlanRequest::Drop { object, def, .. } => Some((*object, def)),
    }
}

/// İstekten saf plan üretimi (önizleme ve uygulama aynı yolu kullanır; hash yalnızca ifadelerden).
pub(crate) fn plan_view(req: &PlanRequest, version_num: u32) -> PlanView {
    let ctx = |concurrently| BuildCtx { version_num, concurrently };
    PlanView::from_build(match req {
        PlanRequest::Create { def, concurrently } => build::create(def, &ctx(*concurrently)),
        PlanRequest::Alter { old, new, .. } => build::alter(old, new, &ctx(false)),
        PlanRequest::Drop { def, cascade, force, .. } => {
            build::drop_with(def, build::DropOptions { cascade: *cascade, force: *force }, &ctx(false))
        }
    })
}

/// (8) `lock_timeout` sınırı (`SessionProfile::Ddl` doğrulamasıyla aynı; bağlantıdan önce).
fn check_lock_timeout(ms: u32) -> Result<()> {
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
        let a = serde_json::to_value(ApplyOutcome::Applied { statements: 2, elapsed_ms: 7, created: None, warnings: None }).unwrap();
        assert_eq!(a, json!({"outcome": "applied", "statements": 2, "elapsedMs": 7, "created": null}));
        let w = ApplyOutcome::Applied { statements: 1, elapsed_ms: 1, created: None, warnings: Some(vec!["u".into()]) };
        assert_eq!(serde_json::to_value(w).unwrap()["warnings"], json!(["u"]));
        let f = ApplyOutcome::Failed {
            failed_index: 1,
            committed: 0,
            sqlstate: Some("42P07".into()),
            position: Some(12),
            error: ErrorPayload { code: "query".into(), message: "m".into(), detail: None, hint: None },
            created: None,
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
            let p = failure_payload_for(kind, Some(&st), 0, &q(code, None));
            assert_eq!(p.hint.as_deref(), want, "{kind:?} {code} {}", st.sql);
            // PG'nin kendi ipucu korunur.
            let p = failure_payload_for(kind, Some(&st), 0, &q(code, Some("pg")));
            assert_eq!(p.hint.as_deref(), Some("pg"), "{kind:?} {code}");
        }
        // Kısmi uygulama (transaction dışı, ipucu yok).
        let p = failure_payload_for(ObjectKind::Database, Some(&nt("COMMENT ON DATABASE d IS 'x'")), 1, &q("42704", None));
        assert_eq!(p.hint.as_deref(), Some(PARTIAL_APPLY_HINT));
        let p = failure_payload_for(ObjectKind::Database, Some(&nt("COMMENT ON DATABASE d IS 'x'")), 0, &q("42704", None));
        assert_eq!(p.hint, None);
        // İptal/kilit iletileri ve ipuçları korunur.
        let p = failure_payload_for(ObjectKind::Database, Some(&nt("DROP DATABASE d")), 0, &q("55P03", None));
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
        let p = failure_payload_for(ObjectKind::Role, Some(&st), 0, &e);
        assert_eq!((p.message.as_str(), p.detail, p.hint), (REDACTED_FAILURE_MESSAGE, None, None));
        // Gizlenmemiş ifadede ileti olduğu gibi kalır.
        let p = failure_payload_for(ObjectKind::Role, Some(&stmt("CREATE ROLE app", false)), 0, &q("42710", None));
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
