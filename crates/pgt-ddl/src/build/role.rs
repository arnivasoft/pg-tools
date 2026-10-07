//! Role üreticisi: CREATE (tüm öznitelikler), ALTER (rename, değişen öznitelikler, parola niyeti, üyelikler,
//! ayarlar, yorum), DROP (CASCADE yok; REASSIGN/DROP OWNED otomatikleştirilmez — Karar 9).
//!
//! Parola (Karar 6): tanımda yalnızca NİYET vardır (`keep | set | clear`). `set` için üretici tek bir gizlenmiş ifade
//! üretir (`ALTER ROLE <q> PASSWORD ********`, `redacted = true`; `********` bilerek tırnaksızdır: kopyalanıp
//! çalıştırılırsa sözdizimi hatası verir, parolayı "********" yapmaz). Gerçek ifade yalnızca uygulayıcıda, kapılardan
//! sonra `materialize_password` ile çalıştırma listesinde kurulur ve düz metin değil istemci tarafı SCRAM-SHA-256
//! doğrulayıcısı taşır (inceleme M-1); üreticiler değeri hiç görmez, plan özeti değerden bağımsızdır.
//!
//! Rolün sahibi yoktur (Karar 5); yalnızca "üye olduğu roller" düzenlenir. PG 16+ üyelik başına `INHERIT`/`SET`
//! seçenekleri salt okunurdur ve üretici tarafından OKUNMAZ (Karar 4). Tüm ifadeler transaction içindedir.

use std::collections::{BTreeMap, HashSet};

use super::settings::{diff_settings, guc_assignment_sql, guc_name_sql, validate_old_setting_names, validate_settings,
    validate_written_settings};
use super::{BuildCtx, check_ident, check_old_ident, check_text, err, stmt};
use crate::model::{ConfigSetting, FieldError, PasswordChange, RoleDef, RoleMembership};
use crate::plan::{Plan, REDACTED, Risk, Statement};
use crate::quote::{quote_ident, quote_literal};
use crate::secret::Secret;

pub const RESERVED_MSG: &str = "Bu rol adı ayrılmıştır (public, none ve pg_ ile başlayan adlar PostgreSQL'e aittir).";
pub const CONNECTION_LIMIT_MSG: &str = "Bağlantı sınırı -1 (sınırsız) ya da daha büyük olmalı.";
pub const VALID_UNTIL_MSG: &str = "Geçersiz süre sınırı: 'infinity' ya da YYYY-MM-DD[ HH:MM[:SS[.ffffff]]][Z|±HH[:MM]] \
     (ör. 2030-01-01, 2030-01-01T10:00:00Z).";
pub const VALID_UNTIL_CLEAR_MSG: &str = "Süre sınırı kaldırılamaz; süresiz için 'infinity' yazın.";
pub const SELF_MEMBER_MSG: &str = "Rol kendisine üye olamaz.";
pub const DUPLICATE_MEMBER_MSG: &str = "Bu rol birden çok kez listelenmiş.";
pub const CLEAR_ON_CREATE_MSG: &str = "Yeni rolde parola kaldırılamaz.";
pub const PASSWORD_NOTE: &str = "Parola gizlendi; uygulamada girilen değer kullanılır.";
pub const PASSWORD_WARNING: &str = "Parola istemcide SCRAM-SHA-256 ile özetlenir; düz metin sunucuya gönderilmez \
     (günlüklere ve pg_stat_statements'a yalnızca doğrulayıcı düşebilir)";
pub const MD5_RENAME_WARNING: &str = "MD5 ile saklanan parola ad değişince temizlenir (SCRAM etkilenmez)";
pub const NOLOGIN_NOTE: &str = "Mevcut oturumlar sürer; yeni girişler reddedilir";
pub const REVOKE_NOTE: &str = "Bu rolün yetkileri kaybolur";
pub const DROP_NOTE: &str = "Rolün sahip olduğu nesne ya da yetki varsa başarısız olur (REASSIGN OWNED / DROP OWNED gerekir)";
pub const SECRET_EMPTY_MSG: &str = "Parola boş olamaz.";
pub const SECRET_NUL_MSG: &str = "Parola NUL karakteri içeremez.";
pub const SECRET_TOO_LONG_MSG: &str = "Parola en çok 1024 bayt olabilir.";
const VERIFIER_MSG: &str = "Parola özeti üretilemedi.";
pub const RESERVED_MEMBER_MSG: &str = "public ve none üye olunacak rol olarak kullanılamaz.";
pub const PREDEFINED_ALTER_MSG: &str = "Önceden tanımlı (pg_*) rolün adı, öznitelikleri, parolası ve ayarları \
     değiştirilemez (PostgreSQL ayrılmış rolleri korur); yorum ve üyelikler düzenlenebilir.";
pub const PREDEFINED_DROP_MSG: &str = "Önceden tanımlı (pg_*) roller silinemez.";

/// Verilmesi plan uyarısı gerektiren öznitelikler ve etkileri (öznitelik sırasıyla).
pub const SUPERUSER_WARNING: &str = "SUPERUSER veriliyor: tüm yetki denetimleri atlanır (sunucu üzerinde tam denetim)";
pub const CREATEROLE_WARNING: &str = "CREATEROLE veriliyor: rol oluşturabilir, değiştirebilir ve silebilir";
pub const REPLICATION_WARNING: &str =
    "REPLICATION veriliyor: replikasyon bağlantısı açıp tüm veritabanı içeriğini okuyabilir";
pub const BYPASSRLS_WARNING: &str = "BYPASSRLS veriliyor: satır düzeyi güvenlik (RLS) ilkeleri atlanır";

/// Gizlenmiş parola ifadesinin önizleme metni (tırnaksız `********`: kopyalanıp çalıştırılırsa sözdizimi hatası verir).
pub(crate) fn password_redacted(role: &str) -> Statement {
    Statement {
        sql: format!("ALTER ROLE {} PASSWORD {}", quote_ident(role), REDACTED),
        risk: Risk::Safe,
        transactional: true,
        note: Some(PASSWORD_NOTE.into()),
        redacted: true,
    }
}

/// Somutlaştırılmış parola ifadesi: `ALTER ROLE <q> PASSWORD 'SCRAM-SHA-256$4096:<tuz>$<StoredKey>:<ServerKey>'`.
/// Düz metin İÇERMEZ (istemci tarafı özet, inceleme M-1); doğrulayıcı yine de çevrimdışı kırmaya açık olduğundan gizli
/// sayılır: `Debug` maskelidir, düşerken baytlar sıfırlanır (en iyi çaba), `Statement`'e hiç konmaz, günlüğe yazılmaz.
pub(crate) struct PasswordStatement(String);

#[cfg_attr(not(test), allow(dead_code))] // Task 8 (uygulayıcı) çağırır.
impl PasswordStatement {
    /// Yalnızca sürücüye vermek için (çalıştırma listesinde; hata yükünde/günlükte kullanılmaz).
    pub(crate) fn sql(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for PasswordStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PasswordStatement(********)")
    }
}

impl Drop for PasswordStatement {
    fn drop(&mut self) {
        zero(std::mem::take(&mut self.0));
    }
}

fn zero(s: String) {
    let mut b = s.into_bytes();
    b.fill(0);
    std::hint::black_box(&b);
}

/// Parola üst sınırı (bayt; ileti sabittir, uzunluğu yankılamaz).
pub const MAX_SECRET_BYTES: usize = 1024;

/// YALNIZCA uygulayıcı çağırır (kapılardan sonra, çalıştırma listesinde). Değer kabul kuralından geçer
/// (`check_secret`), ardından psql `\password` / libpq `PQencryptPasswordConn` gibi istemcide SCRAM-SHA-256 ile
/// özetlenir (rastgele tuz, 4096 tur, SASLprep); sunucuya, `log_min_error_statement` günlüğüne, `pg_stat_statements`'a
/// ya da hata metnine düz metin hiç gitmez. Sunucunun `password_encryption` ayarından bağımsızdır (PG 10+ SCRAM
/// doğrulayıcısını olduğu gibi saklar). Hata iletileri sabittir.
#[cfg_attr(not(test), allow(dead_code))] // Task 8 (uygulayıcı) çağırır.
pub(crate) fn materialize_password(role: &str, secret: &Secret) -> Result<PasswordStatement, &'static str> {
    check_secret(secret)?;
    let verifier = postgres_protocol::password::scram_sha_256(secret.expose().as_bytes());
    // Kütüphane sözleşmesi: doğrulayıcı yalnızca base64/ASCII; savunma olarak yine denetlenir (tırnak/ters bölü yok).
    if !verifier.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'$' | b':' | b'-')) {
        zero(verifier);
        return Err(VERIFIER_MSG);
    }
    let sql = format!("ALTER ROLE {} PASSWORD {}", quote_ident(role), quote_literal(&verifier));
    zero(verifier);
    Ok(PasswordStatement(sql))
}

/// Parola değerinin kabul kuralı (uygulama kapısı): boş değil, NUL yok, ≤ `MAX_SECRET_BYTES`. İleti değeri (ya da
/// uzunluğunu) taşımaz.
fn check_secret(secret: &Secret) -> Result<(), &'static str> {
    let v = secret.expose();
    if v.is_empty() {
        Err(SECRET_EMPTY_MSG)
    } else if v.contains('\0') {
        Err(SECRET_NUL_MSG)
    } else if v.len() > MAX_SECRET_BYTES {
        Err(SECRET_TOO_LONG_MSG)
    } else {
        Ok(())
    }
}

/// PG `IsReservedName` + `public`/`none` (CREATE ROLE / RENAME TO 42939). Büyük/küçük harfe duyarlı: `Public`, `pgx`
/// geçerlidir.
fn is_reserved_role_name(name: &str) -> bool {
    name == "public" || name == "none" || name.starts_with("pg_")
}

/// YENİ rol adı (create ya da rename hedefi): ad kuralları (bidi/C1 dahil) + ayrılmış adlar.
fn check_new_role_name(path: &str, name: &str, e: &mut Vec<FieldError>) {
    let before = e.len();
    check_ident(path, name, e);
    if e.len() == before && is_reserved_role_name(name) {
        e.push(err(path, RESERVED_MSG));
    }
}

fn check_connection_limit(r: &RoleDef, e: &mut Vec<FieldError>) {
    if r.connection_limit < -1 {
        e.push(err("connectionLimit", CONNECTION_LIMIT_MSG));
    }
}

/// `n` ASCII rakam → sayı; uzunluk tam olmalı.
fn digits(s: &str, n: usize) -> Option<u32> {
    (s.len() == n && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `VALID UNTIL` dilbilgisi: `infinity` | `YYYY-MM-DD[(T| )HH:MM[:SS[.f{1,6}]][Z|±HH[:MM]]]`. Saat dilimi yalnızca saatle
/// birlikte (plan dilbilgisinden daha katı; yükleyici her zaman saatli UTC biçimi döndürür). Alan aralıkları denetlenir
/// (ay 1–12, gün aya göre, saat 0–23, dakika/saniye 0–59, dilim ±00–15[:00–59]); değer yalnızca ASCII'dir, bu yüzden
/// `quote_literal` içinde tırnak/ters bölü taşımaz.
pub fn valid_until_ok(v: &str) -> bool {
    if v == "infinity" {
        return true;
    }
    if !v.is_ascii() || v.len() < 10 {
        return false;
    }
    let (date, rest) = v.split_at(10);
    let b = date.as_bytes();
    if b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let (Some(y), Some(m), Some(d)) = (digits(&date[0..4], 4), digits(&date[5..7], 2), digits(&date[8..10], 2)) else {
        return false;
    };
    if y == 0 || !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return false;
    }
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = rest.strip_prefix('T').or_else(|| rest.strip_prefix(' ')) else {
        return false;
    };
    // HH:MM
    if rest.len() < 5 || rest.as_bytes()[2] != b':' {
        return false;
    }
    let (Some(h), Some(mi)) = (digits(&rest[0..2], 2), digits(&rest[3..5], 2)) else {
        return false;
    };
    if h > 23 || mi > 59 {
        return false;
    }
    let mut rest = &rest[5..];
    // [:SS[.f{1,6}]]
    if let Some(r) = rest.strip_prefix(':') {
        let Some(s) = r.get(0..2).and_then(|s| digits(s, 2)) else {
            return false;
        };
        if s > 59 {
            return false;
        }
        rest = &r[2..];
        if let Some(r) = rest.strip_prefix('.') {
            let n = r.bytes().take_while(u8::is_ascii_digit).count();
            if !(1..=6).contains(&n) {
                return false;
            }
            rest = &r[n..];
        }
    }
    // [Z|±HH[:MM]]
    if rest.is_empty() || rest == "Z" {
        return true;
    }
    let Some(tz) = rest.strip_prefix('+').or_else(|| rest.strip_prefix('-')) else {
        return false;
    };
    let (hh, mm) = match tz.split_once(':') {
        Some((hh, mm)) => (digits(hh, 2), Some(digits(mm, 2))),
        None => (digits(tz, 2), None),
    };
    match (hh, mm) {
        (Some(hh), None) => hh <= 15,
        (Some(hh), Some(Some(mm))) => hh <= 15 && mm <= 59,
        _ => false,
    }
}

fn check_valid_until(v: &str, e: &mut Vec<FieldError>) {
    if !valid_until_ok(v) {
        e.push(err("validUntil", VALID_UNTIL_MSG));
    }
}

/// Üyelikler: yinelenen ve kendine üyelik TÜM listede; ad kuralı yalnızca `written(m)` olanlarda (alter: eklenen ya
/// da admin bayrağı değişen; create: hepsi). `inheritOption`/`setOption` okunmaz.
fn check_memberships(
    role: &str,
    list: &[RoleMembership],
    written: impl Fn(&RoleMembership) -> bool,
    e: &mut Vec<FieldError>,
) {
    let mut seen = HashSet::new();
    for (i, m) in list.iter().enumerate() {
        let path = format!("memberOf[{i}].role");
        let before = e.len();
        if written(m) {
            check_ident(&path, &m.role, e);
            // GRANT'ta `public`/`none` rol belirtimi biçimleridir; sunucu reddeder (inceleme n-1). `pg_*` meşrudur.
            if e.len() == before && (m.role == "public" || m.role == "none") {
                e.push(err(&path, RESERVED_MEMBER_MSG));
            }
        }
        if e.len() != before {
            continue;
        }
        if m.role == role {
            e.push(err(&path, SELF_MEMBER_MSG));
        } else if !seen.insert(m.role.as_str()) {
            e.push(err(&path, DUPLICATE_MEMBER_MSG));
        }
    }
}

/// Oluşturma doğrulaması (tüm alanlar).
pub fn validate(r: &RoleDef, _ctx: &BuildCtx) -> Vec<FieldError> {
    let mut e = vec![];
    check_new_role_name("name", &r.name, &mut e);
    check_connection_limit(r, &mut e);
    if let Some(v) = &r.valid_until {
        check_valid_until(v, &mut e);
    }
    if r.password == PasswordChange::Clear {
        e.push(err("password", CLEAR_ON_CREATE_MSG));
    }
    check_memberships(&r.name, &r.member_of, |_| true, &mut e);
    validate_settings(&r.settings, &mut e);
    if let Some(c) = &r.comment {
        check_text("comment", c, &mut e);
    }
    e
}

/// (öznitelik açık, kapalı) adları; deterministik sıra.
fn flags(r: &RoleDef) -> [(bool, &'static str, &'static str); 7] {
    [
        (r.login, "LOGIN", "NOLOGIN"),
        (r.superuser, "SUPERUSER", "NOSUPERUSER"),
        (r.createdb, "CREATEDB", "NOCREATEDB"),
        (r.createrole, "CREATEROLE", "NOCREATEROLE"),
        (r.inherit, "INHERIT", "NOINHERIT"),
        (r.replication, "REPLICATION", "NOREPLICATION"),
        (r.bypassrls, "BYPASSRLS", "NOBYPASSRLS"),
    ]
}

/// Verilen (false → true) yetkili özniteliklerin uyarıları; `old = None` (create) → true olanların hepsi.
fn grant_warnings(old: Option<&RoleDef>, new: &RoleDef, out: &mut Vec<String>) {
    let granted = |o: Option<bool>, n: bool| n && o != Some(true);
    for (o, n, w) in [
        (old.map(|o| o.superuser), new.superuser, SUPERUSER_WARNING),
        (old.map(|o| o.createrole), new.createrole, CREATEROLE_WARNING),
        (old.map(|o| o.replication), new.replication, REPLICATION_WARNING),
        (old.map(|o| o.bypassrls), new.bypassrls, BYPASSRLS_WARNING),
    ] {
        if granted(o, n) {
            out.push(w.to_string());
        }
    }
}

fn grant_stmt(member_of: &str, q: &str, admin: bool) -> Statement {
    let opt = if admin { " WITH ADMIN OPTION" } else { "" };
    stmt(format!("GRANT {} TO {q}{opt}", quote_ident(member_of)), Risk::Safe, None)
}

fn set_stmt(q: &str, s: &ConfigSetting) -> Statement {
    stmt(format!("ALTER ROLE {q} SET {}", guc_assignment_sql(s)), Risk::Safe, None)
}

fn comment_stmt(q: &str, comment: &Option<String>) -> Statement {
    let value = comment.as_deref().map_or_else(|| "NULL".to_string(), quote_literal);
    stmt(format!("COMMENT ON ROLE {q} IS {value}"), Risk::Safe, None)
}

/// Doğrulanmış tanım varsayılır (`build::create` önce `validate` çağırır).
///
/// `CREATE ROLE <q> WITH <7 öznitelik> CONNECTION LIMIT n [VALID UNTIL '…']`, parola `set` ise gizlenmiş ifade,
/// üyelikler (rol adı sırasıyla), ayarlar (ad sırasıyla), yorum.
pub fn create(r: &RoleDef) -> Plan {
    let q = quote_ident(&r.name);
    let mut attrs: Vec<String> = flags(r).iter().map(|&(on, y, n)| if on { y } else { n }.to_string()).collect();
    attrs.push(format!("CONNECTION LIMIT {}", r.connection_limit));
    if let Some(v) = &r.valid_until {
        attrs.push(format!("VALID UNTIL {}", quote_literal(v)));
    }
    let mut statements = vec![stmt(format!("CREATE ROLE {q} WITH {}", attrs.join(" ")), Risk::Safe, None)];
    let mut warnings = vec![];
    grant_warnings(None, r, &mut warnings);
    if r.password == PasswordChange::Set {
        statements.push(password_redacted(&r.name));
        warnings.push(PASSWORD_WARNING.to_string());
    }
    let members: BTreeMap<&str, bool> = r.member_of.iter().map(|m| (m.role.as_str(), m.admin)).collect();
    statements.extend(members.into_iter().map(|(role, admin)| grant_stmt(role, &q, admin)));
    let (_, set) = diff_settings(&[], &r.settings);
    statements.extend(set.into_iter().map(|s| set_stmt(&q, s)));
    if r.comment.is_some() {
        statements.push(comment_stmt(&q, &r.comment));
    }
    Plan { statements, warnings }
}

/// Alter doğrulaması: eskinin yalnızca SQL'e giren alanları (ad, RESET hedefi ayar adları, REVOKE hedefi üyelik
/// adları; gevşek), yeninin yalnızca değişen ya da ifadeye yazılan alanları (katı).
fn validate_alter(old: &RoleDef, new: &RoleDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_old_ident("old.name", &old.name, &mut e);
    validate_old_setting_names("old.", &old.settings, &mut e);
    for (i, m) in old.member_of.iter().enumerate() {
        check_old_ident(&format!("old.memberOf[{i}].role"), &m.role, &mut e);
    }
    if !e.is_empty() {
        return e;
    }
    // Yüklü önceden tanımlı rol (inceleme n-2): ad gevşek kalır, ama PG'nin "Cannot alter reserved roles" ile
    // reddedeceği değişiklikler (rename, öznitelik, parola, ayar) önceden açık iletiyle durdurulur; yorum ve üyelikler
    // serbest.
    if old.name.starts_with("pg_") {
        let attrs_changed = flags(old).iter().zip(flags(new)).any(|(a, b)| a.0 != b.0)
            || old.connection_limit != new.connection_limit
            || old.valid_until != new.valid_until;
        let (removed, set) = diff_settings(&old.settings, &new.settings);
        if old.name != new.name
            || attrs_changed
            || new.password != PasswordChange::Keep
            || !removed.is_empty()
            || !set.is_empty()
        {
            return vec![err("name", PREDEFINED_ALTER_MSG)];
        }
    }
    if new.name != old.name {
        check_new_role_name("name", &new.name, &mut e);
    }
    if new.connection_limit != old.connection_limit {
        check_connection_limit(new, &mut e);
    }
    match &new.valid_until {
        Some(v) if new.valid_until != old.valid_until => check_valid_until(v, &mut e),
        // PG `VALID UNTIL`'ı NULL'a döndüremez (Karar 11).
        None if old.valid_until.is_some() => e.push(err("validUntil", VALID_UNTIL_CLEAR_MSG)),
        _ => {}
    }
    let old_admin: BTreeMap<&str, bool> = old.member_of.iter().map(|m| (m.role.as_str(), m.admin)).collect();
    check_memberships(&new.name, &new.member_of, |m| old_admin.get(m.role.as_str()) != Some(&m.admin), &mut e);
    // Ayarlar: yalnızca yazılacak girdiler; yinelenen ad her zaman hata (Task 4 M-1).
    validate_written_settings(&old.settings, &new.settings, &mut e);
    if let Some(c) = &new.comment
        && new.comment != old.comment
    {
        check_text("comment", c, &mut e);
    }
    e
}

/// Sıra (sonrakiler YENİ adı kullanır): rename, `WITH` (yalnız değişenler), parola, REVOKE/REVOKE ADMIN OPTION FOR,
/// GRANT/GRANT … WITH ADMIN OPTION (ikisi de rol adı sırasıyla), ayarlar (RESET sonra SET), yorum.
///
/// Parola yalnızca YENİ tanımın niyetiyle belirlenir; üretici `old.password`'ı okumaz. Bunun güvenli olması yöneticinin
/// (Task 8) şu kapılarına bağlıdır (her biri açık testle):
/// 1. `old.password != keep` (yükleyici her zaman `keep` döndürür) → yeniden yükleme eşitliği bozulur → `plan_changed`;
/// 2. `ApplyOptions.password` verilmiş ama planda gizlenmiş (`redacted`) ifade yok → reddedilir (parola kullanılmaz);
/// 3. planda gizlenmiş ifade var ama `ApplyOptions.password` yok → `role_password_missing`;
/// 4. gizlenmiş ifade `materialize_password` ile yalnızca çalıştırma listesinde somutlaşır; bu ifade başarısız olursa
///    hata yükünde `position = None` döner ve sorgu metni/ayrıntı yankılanmaz.
pub fn alter(old: &RoleDef, new: &RoleDef, _ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    let errs = validate_alter(old, new);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut statements = vec![];
    let mut warnings = vec![];
    let q = quote_ident(&new.name);

    if old.name != new.name {
        statements.push(stmt(
            format!("ALTER ROLE {} RENAME TO {q}", quote_ident(&old.name)),
            Risk::Safe,
            Some(MD5_RENAME_WARNING),
        ));
        warnings.push(MD5_RENAME_WARNING.to_string());
    }
    let mut with = vec![];
    for ((o, ..), (n, y, no)) in flags(old).into_iter().zip(flags(new)) {
        if o != n {
            with.push(if n { y } else { no }.to_string());
        }
    }
    if new.connection_limit != old.connection_limit {
        with.push(format!("CONNECTION LIMIT {}", new.connection_limit));
    }
    if let Some(v) = &new.valid_until
        && new.valid_until != old.valid_until
    {
        with.push(format!("VALID UNTIL {}", quote_literal(v)));
    }
    if !with.is_empty() {
        let note = (old.login && !new.login).then_some(NOLOGIN_NOTE);
        statements.push(stmt(format!("ALTER ROLE {q} WITH {}", with.join(" ")), Risk::Safe, note));
    }
    grant_warnings(Some(old), new, &mut warnings);
    match new.password {
        PasswordChange::Keep => {}
        PasswordChange::Set => {
            statements.push(password_redacted(&new.name));
            warnings.push(PASSWORD_WARNING.to_string());
        }
        PasswordChange::Clear => statements.push(stmt(format!("ALTER ROLE {q} PASSWORD NULL"), Risk::Safe, None)),
    }
    // Üyelik eşlemesi rol adıyla; PG 16+ seçenekleri (inherit/set) okunmaz.
    let old_admin: BTreeMap<&str, bool> = old.member_of.iter().map(|m| (m.role.as_str(), m.admin)).collect();
    let new_admin: BTreeMap<&str, bool> = new.member_of.iter().map(|m| (m.role.as_str(), m.admin)).collect();
    for (&role, &was_admin) in &old_admin {
        let r = quote_ident(role);
        match new_admin.get(role) {
            None => statements.push(stmt(format!("REVOKE {r} FROM {q}"), Risk::Destructive, Some(REVOKE_NOTE))),
            Some(false) if was_admin => {
                statements.push(stmt(format!("REVOKE ADMIN OPTION FOR {r} FROM {q}"), Risk::Destructive, None));
            }
            _ => {}
        }
    }
    for (&role, &admin) in &new_admin {
        match old_admin.get(role) {
            None => statements.push(grant_stmt(role, &q, admin)),
            Some(false) if admin => statements.push(grant_stmt(role, &q, true)),
            _ => {}
        }
    }
    let (removed, set) = diff_settings(&old.settings, &new.settings);
    for name in removed {
        statements.push(stmt(format!("ALTER ROLE {q} RESET {}", guc_name_sql(name)), Risk::Safe, None));
    }
    statements.extend(set.into_iter().map(|s| set_stmt(&q, s)));
    if old.comment != new.comment {
        statements.push(comment_stmt(&q, &new.comment));
    }
    Ok(Plan { statements, warnings })
}

/// `DROP ROLE <q>` — Destructive. Ortak kurallar (CASCADE/FORCE yok, gevşek kimlik) `build::drop_with`'tedir; sahip
/// olunan nesne/yetki varsa PG 2BP01 verir, plan geri alınır (REASSIGN/DROP OWNED üretilmez).
/// Yüklü önceden tanımlı (`pg_*`) rol: sunucu reddeder; açık iletiyle önceden durdurulur (inceleme n-2).
pub fn drop(r: &RoleDef) -> Result<Plan, Vec<FieldError>> {
    if r.name.starts_with("pg_") {
        return Err(vec![err("name", PREDEFINED_DROP_MSG)]);
    }
    Ok(Plan {
        statements: vec![stmt(format!("DROP ROLE {}", quote_ident(&r.name)), Risk::Destructive, Some(DROP_NOTE))],
        warnings: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{
        DropOptions, alter as balter, assert_single_statements, create as bcreate, drop_with, render, scan_semicolons,
        validate_identity,
    };
    use crate::model::ObjectDef;
    use crate::plan::plan_hash;

    fn role(name: &str) -> RoleDef {
        RoleDef {
            name: name.into(),
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
        }
    }
    fn m(r: &str, admin: bool) -> RoleMembership {
        RoleMembership { role: r.into(), admin, inherit_option: None, set_option: None }
    }
    fn s(n: &str, v: &str) -> ConfigSetting {
        ConfigSetting { name: n.into(), value: v.into() }
    }
    fn o(r: &RoleDef) -> ObjectDef {
        ObjectDef::Role(r.clone())
    }
    fn ctx() -> BuildCtx {
        BuildCtx { version_num: 170000, ..Default::default() }
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn paths(errs: Vec<FieldError>) -> Vec<String> {
        errs.into_iter().map(|e| e.path).collect()
    }
    /// `build::create` üzerinden (doğrulama dahil); başarı beklenir.
    fn create(r: &RoleDef, ctx: &BuildCtx) -> Plan {
        let p = bcreate(&o(r), ctx).unwrap();
        assert_single_statements(&p);
        p
    }
    fn create_err(r: &RoleDef) -> Vec<FieldError> {
        bcreate(&o(r), &ctx()).unwrap_err()
    }
    fn alter_ok(old: &RoleDef, new: &RoleDef) -> Plan {
        let p = balter(&o(old), &o(new), &ctx()).unwrap();
        assert_single_statements(&p);
        p
    }
    fn alter_err(old: &RoleDef, new: &RoleDef) -> Vec<FieldError> {
        balter(&o(old), &o(new), &ctx()).unwrap_err()
    }

    const DEFAULT_ATTRS: &str =
        "NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1";

    #[test]
    fn password_is_redacted_in_every_plan_and_hash_is_value_free() {
        let mut r = role("app");
        r.password = PasswordChange::Set;
        let p = create(&r, &ctx());
        let red: Vec<_> = p.statements.iter().filter(|s| s.redacted).collect();
        assert_eq!(red.len(), 1);
        assert_eq!(red[0].sql, "ALTER ROLE app PASSWORD ********");
        assert!(!red[0].sql.contains('\''), "maske tırnaksız olmalı (kopyalanan betik çalışmasın)");
        // Aynı istek her zaman aynı hash (değer isteğin parçası değildir).
        assert_eq!(plan_hash(&p.statements), plan_hash(&create(&r, &ctx()).statements));
        // Somutlaştırma (inceleme M-1 kararı: istemci tarafı SCRAM) düz metni değil doğrulayıcıyı tek tırnaklı değer
        // olarak yazar; tek ifade kalır.
        let st = materialize_password("app", &Secret::new("x'; DROP ROLE app; --")).unwrap();
        assert!(st.sql().starts_with("ALTER ROLE app PASSWORD 'SCRAM-SHA-256$4096:"), "{}", st.sql());
        assert!(!st.sql().contains("DROP ROLE"));
        assert!(scan_semicolons(st.sql()).is_clean());
        assert!(materialize_password("Ünï", &Secret::new("a\\b")).unwrap().sql().starts_with("ALTER ROLE \"Ünï\" PASSWORD 'SCRAM"));
        assert!(materialize_password("app", &Secret::new("")).is_err() && materialize_password("app", &Secret::new("a\0b")).is_err());
    }

    /// `^SCRAM-SHA-256\$4096:[A-Za-z0-9+/=]+\$[A-Za-z0-9+/=]+:[A-Za-z0-9+/=]+$` (regex bağımlılığı olmadan).
    fn is_scram_verifier(v: &str) -> bool {
        let b64 = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
        let Some(rest) = v.strip_prefix("SCRAM-SHA-256$4096:") else { return false };
        let Some((salt, keys)) = rest.split_once('$') else { return false };
        let Some((stored, server)) = keys.split_once(':') else { return false };
        b64(salt) && b64(stored) && b64(server)
    }

    /// `ALTER ROLE <q> PASSWORD '<doğrulayıcı>'` → doğrulayıcı.
    fn verifier_of<'a>(sql: &'a str, q: &str) -> &'a str {
        let lit = sql.strip_prefix(&format!("ALTER ROLE {q} PASSWORD ")).expect(sql);
        lit.strip_prefix('\'').and_then(|l| l.strip_suffix('\'')).expect(sql)
    }

    #[test]
    fn materialized_password_is_client_side_scram() {
        let hostile = [
            "düz",
            "x'; DROP ROLE app; --",
            "a\\b",
            "E'\\x27'",
            "satır\nsonu\r\tsekme",
            "\"; --",
            "/* */ $$ $x$",
            "日本語パスワード",
            "\u{202e}ters\u{200b}",
            "SCRAM-SHA-256$4096:AAAA$BBBB:CCCC",
            "md5d41d8cd98f00b204e9800998ecf8427e",
        ];
        let long = "p".repeat(MAX_SECRET_BYTES);
        for pw in hostile.iter().copied().chain([long.as_str()]) {
            for role_name in ["app", "Ünï", "r'\"; --"] {
                let st = materialize_password(role_name, &Secret::new(pw)).unwrap();
                let q = quote_ident(role_name);
                let v = verifier_of(st.sql(), &q);
                assert!(is_scram_verifier(v), "{pw:?}: {v}");
                // Ad dışındaki tek parça doğrulayıcıdır; düz metin içermez.
                assert!(!v.contains(pw) && !st.sql().contains(&quote_literal(pw)), "düz metin sızdı: {pw:?}");
                assert!(scan_semicolons(st.sql()).is_clean(), "{pw:?}");
                assert!(!st.sql().contains('\\') && st.sql().matches('\'').count() == 2 + q.matches('\'').count());
                // Debug/format! maskeli.
                let dbg = format!("{st:?} {:?}", Some(&st));
                assert_eq!(dbg, "PasswordStatement(********) Some(PasswordStatement(********))");
            }
        }
        // İki çağrı farklı tuz (ve dolayısıyla farklı anahtarlar) verir.
        let a = materialize_password("app", &Secret::new("aynı")).unwrap();
        let b = materialize_password("app", &Secret::new("aynı")).unwrap();
        let salt = |s: &PasswordStatement| verifier_of(s.sql(), "app").split('$').nth(1).unwrap().split(':').nth(1).unwrap().to_string();
        assert_ne!(salt(&a), salt(&b));
        assert_ne!(a.sql(), b.sql());
        // Hatalar: sabit ileti, değer/uzunluk yankılanmaz.
        let over = "p".repeat(MAX_SECRET_BYTES + 1);
        for (bad, want) in [("", SECRET_EMPTY_MSG), ("Gizli\0Parola", SECRET_NUL_MSG), (over.as_str(), SECRET_TOO_LONG_MSG)] {
            let e = materialize_password("app", &Secret::new(bad)).unwrap_err();
            assert_eq!(e, want);
            assert!(!e.contains("Gizli") && !e.contains("ppp"));
        }
        assert!(SECRET_TOO_LONG_MSG.contains(&MAX_SECRET_BYTES.to_string()));
        // Uyarı artık istemci tarafı özeti söyler.
        assert!(PASSWORD_WARNING.contains("SCRAM-SHA-256") && PASSWORD_WARNING.contains("düz metin sunucuya gönderilmez"));
    }

    #[test]
    fn password_value_never_leaks() {
        let marker = "GizliParola-7f3a'\\";
        let secret = Secret::new(marker);
        let mut r = role("app");
        r.password = PasswordChange::Set;
        let renamed = RoleDef { name: "app2".into(), ..r.clone() };
        let plans = [create(&r, &ctx()), alter_ok(&role("app"), &renamed)];
        for p in &plans {
            let red: Vec<_> = p.statements.iter().filter(|s| s.redacted).collect();
            assert_eq!(red.len(), 1, "{p:?}");
            assert!(red[0].sql.ends_with(" PASSWORD ********"));
            assert_eq!((red[0].risk, red[0].transactional), (Risk::Safe, true));
            // Hiçbir ifade/not/uyarı/Debug/JSON değeri taşımaz; hash değerden bağımsız (girdi değildir).
            let dump = format!("{p:?}{}", serde_json::to_string(&p.statements).unwrap());
            assert!(!dump.contains("GizliParola"), "{dump}");
            assert!(p.warnings.iter().all(|w| !w.contains("GizliParola")));
            // Gizlenmemiş ifadeler `PASSWORD` taşımaz.
            assert!(p.statements.iter().filter(|s| !s.redacted).all(|s| !s.sql.contains("PASSWORD")));
        }
        // Tanım (Debug/JSON) değeri taşıyamaz: yalnızca niyet.
        assert_eq!(serde_json::to_value(&r).unwrap()["password"], "set");
        // Secret Debug/Serialize gizli; kabul kuralı iletileri değeri ya da uzunluğu taşımaz.
        assert!(!format!("{secret:?}").contains("GizliParola"));
        assert_eq!(serde_json::to_string(&secret).unwrap(), "\"********\"");
        assert_eq!(check_secret(&secret), Ok(()));
        for bad in ["", "Gizli\0Parola"] {
            let m = check_secret(&Secret::new(bad)).unwrap_err();
            assert!(!m.contains("Gizli") && !m.chars().any(|c| c.is_ascii_digit()), "{m}");
        }
        assert_eq!(check_secret(&Secret::new("")), Err(SECRET_EMPTY_MSG));
        assert_eq!(check_secret(&Secret::new("a\0b")), Err(SECRET_NUL_MSG));
        // Somutlaştırılmış metin yalnızca çalıştırma için; gizlenmiş ifadeyle aynı hedef, düz metin yok.
        let real = materialize_password("app2", &secret).unwrap();
        assert!(is_scram_verifier(verifier_of(real.sql(), "app2")), "{}", real.sql());
        assert!(!real.sql().contains("GizliParola") && !format!("{real:?}").contains("SCRAM"));
        assert!(scan_semicolons(real.sql()).is_clean());
        // Gizlenmiş ifadenin hash'i parolanın içeriğinden bağımsız iki istekte aynıdır.
        assert_eq!(plan_hash(&plans[0].statements), plan_hash(&create(&r, &ctx()).statements));
        // Gizlenmiş ifade bayrağı hash'e girer: aynı metin bayraksız farklı özet verir.
        let mut unflagged = plans[0].statements.clone();
        for st in &mut unflagged {
            st.redacted = false;
        }
        assert_ne!(plan_hash(&plans[0].statements), plan_hash(&unflagged));
    }

    #[test]
    fn create_attribute_table() {
        assert_eq!(sqls(&create(&role("r"), &ctx())), [format!("CREATE ROLE r WITH {DEFAULT_ATTRS}")]);
        type Set = fn(&mut RoleDef, bool);
        let cases: [(Set, &str, &str); 7] = [
            (|r, b| r.login = b, "LOGIN", "NOLOGIN"),
            (|r, b| r.superuser = b, "SUPERUSER", "NOSUPERUSER"),
            (|r, b| r.createdb = b, "CREATEDB", "NOCREATEDB"),
            (|r, b| r.createrole = b, "CREATEROLE", "NOCREATEROLE"),
            (|r, b| r.inherit = b, "INHERIT", "NOINHERIT"),
            (|r, b| r.replication = b, "REPLICATION", "NOREPLICATION"),
            (|r, b| r.bypassrls = b, "BYPASSRLS", "NOBYPASSRLS"),
        ];
        let words = |p: &Plan| p.statements[0].sql.split(' ').map(str::to_string).collect::<Vec<_>>();
        for (i, (set, yes, no)) in cases.iter().enumerate() {
            for b in [true, false] {
                let mut r = role("r");
                set(&mut r, b);
                let p = create(&r, &ctx());
                let w = words(&p);
                // "CREATE ROLE r WITH" + 7 öznitelik konumu sabit.
                assert_eq!(w[4 + i], if b { *yes } else { *no }, "{yes} {b}");
                assert_eq!(p.statements.len(), 1);
                assert_eq!(p.statements[0].risk, Risk::Safe);
                assert!(p.statements[0].transactional && !p.statements[0].redacted);
            }
        }
        // Tümü açık: dört yetkili öznitelik uyarısı (öznitelik sırasıyla).
        let all = RoleDef {
            login: true,
            superuser: true,
            createdb: true,
            createrole: true,
            inherit: false,
            replication: true,
            bypassrls: true,
            connection_limit: 5,
            valid_until: Some("2030-01-01T10:00:00Z".into()),
            ..role("Admin")
        };
        let p = create(&all, &ctx());
        assert_eq!(
            sqls(&p),
            ["CREATE ROLE \"Admin\" WITH LOGIN SUPERUSER CREATEDB CREATEROLE NOINHERIT REPLICATION BYPASSRLS \
              CONNECTION LIMIT 5 VALID UNTIL '2030-01-01T10:00:00Z'"]
        );
        assert_eq!(p.warnings, [SUPERUSER_WARNING, CREATEROLE_WARNING, REPLICATION_WARNING, BYPASSRLS_WARNING]);
        // CREATEDB/LOGIN uyarı vermez.
        assert!(create(&RoleDef { login: true, createdb: true, ..role("r") }, &ctx()).warnings.is_empty());
    }

    #[test]
    fn create_variants() {
        let mut out = String::new();
        let full = RoleDef {
            login: true,
            password: PasswordChange::Set,
            valid_until: Some("infinity".into()),
            member_of: vec![m("zeta", false), m("pg_read_all_data", false), m("Alpha", true)],
            settings: vec![s("work_mem", "64MB"), s("search_path", "\"$user\", public")],
            comment: Some("it's".into()),
            ..role("app")
        };
        let p = create(&full, &ctx());
        assert_eq!(
            sqls(&p),
            [
                "CREATE ROLE app WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS \
                 CONNECTION LIMIT -1 VALID UNTIL 'infinity'",
                "ALTER ROLE app PASSWORD ********",
                "GRANT \"Alpha\" TO app WITH ADMIN OPTION",
                "GRANT pg_read_all_data TO app",
                "GRANT zeta TO app",
                "ALTER ROLE app SET search_path = '$user', 'public'",
                "ALTER ROLE app SET work_mem = '64MB'",
                "COMMENT ON ROLE app IS 'it''s'",
            ]
        );
        assert!(p.statements.iter().all(|s| s.risk == Risk::Safe && s.transactional));
        assert_eq!(p.statements.iter().filter(|s| s.redacted).count(), 1);
        assert_eq!(p.statements[1].note.as_deref(), Some(PASSWORD_NOTE));
        assert_eq!(p.warnings, [PASSWORD_WARNING]);
        out.push_str(&render(&p));
        out.push('\n');
        out.push_str(&render(&create(&role("r"), &ctx())));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn password_clear_on_create_is_error() {
        let e = create_err(&RoleDef { password: PasswordChange::Clear, ..role("r") });
        assert_eq!(paths(e.clone()), ["password"]);
        assert_eq!(e[0].message, CLEAR_ON_CREATE_MSG);
    }

    #[test]
    fn alter_password_set_and_clear() {
        let base = role("r");
        let p = alter_ok(&base, &RoleDef { password: PasswordChange::Clear, ..base.clone() });
        assert_eq!(sqls(&p), ["ALTER ROLE r PASSWORD NULL"]);
        assert!(!p.statements[0].redacted);
        assert_eq!(p.statements[0].risk, Risk::Safe);
        assert!(p.warnings.is_empty());
        let p = alter_ok(&base, &RoleDef { password: PasswordChange::Set, ..base.clone() });
        assert_eq!(sqls(&p), ["ALTER ROLE r PASSWORD ********"]);
        assert!(p.statements[0].redacted);
        assert_eq!(p.warnings, [PASSWORD_WARNING]);
    }

    #[test]
    fn rename_then_password_uses_new_name() {
        let old = role("old");
        let new = RoleDef { name: "Yeni Ad".into(), password: PasswordChange::Set, ..old.clone() };
        let p = alter_ok(&old, &new);
        assert_eq!(sqls(&p), ["ALTER ROLE \"old\" RENAME TO \"Yeni Ad\"", "ALTER ROLE \"Yeni Ad\" PASSWORD ********"]);
        assert_eq!(p.statements[0].note.as_deref(), Some(MD5_RENAME_WARNING));
        assert!(p.statements[1].redacted && !p.statements[0].redacted);
        assert_eq!(p.warnings, [MD5_RENAME_WARNING, PASSWORD_WARNING]);
    }

    #[test]
    fn connection_limit_bounds() {
        for n in [-2, i32::MIN] {
            let e = create_err(&RoleDef { connection_limit: n, ..role("r") });
            assert_eq!(paths(e.clone()), ["connectionLimit"]);
            assert_eq!(e[0].message, CONNECTION_LIMIT_MSG);
            assert_eq!(paths(alter_err(&role("r"), &RoleDef { connection_limit: n, ..role("r") })), ["connectionLimit"]);
        }
        for n in [-1, 0, 1, i32::MAX] {
            let p = create(&RoleDef { connection_limit: n, ..role("r") }, &ctx());
            assert!(p.statements[0].sql.ends_with(&format!("CONNECTION LIMIT {n}")), "{n}");
            let p = alter_ok(&RoleDef { connection_limit: 7, ..role("r") }, &RoleDef { connection_limit: n, ..role("r") });
            assert_eq!(sqls(&p), [format!("ALTER ROLE r WITH CONNECTION LIMIT {n}")]);
        }
        // Yüklü kurala aykırı değer değişmedikçe engellemez.
        let odd = RoleDef { connection_limit: -5, ..role("r") };
        assert!(alter_ok(&odd, &odd).statements.is_empty());
    }

    #[test]
    fn valid_until_grammar() {
        for ok in [
            "infinity",
            "2030-01-01",
            "2030-01-01T10:00:00Z",
            "2030-01-01 10:00:00+03",
            "2030-01-01T10:00:00.123456Z",
            "2030-01-01 10:00",
            "2030-01-01T10:00-05:30",
            "2028-02-29",
            "2000-02-29 23:59:59.1",
            "9999-12-31T23:59:59+15:59",
        ] {
            assert!(valid_until_ok(ok), "{ok:?}");
            let p = create(&RoleDef { valid_until: Some(ok.into()), ..role("r") }, &ctx());
            assert!(p.statements[0].sql.ends_with(&format!(" VALID UNTIL '{ok}'")), "{ok:?}");
        }
        for bad in [
            "2030-13-01",
            "tomorrow",
            "2030-01-01'; --",
            "now()",
            "",
            "Infinity",
            "-infinity",
            "2030-00-10",
            "2030-01-32",
            "2029-02-29",
            "1900-02-29",
            "0000-01-01",
            "30-01-01",
            "2030/01/01",
            "2030-1-01",
            "2030-01-01T",
            "2030-01-01T24:00",
            "2030-01-01T10:60",
            "2030-01-01T10:00:60",
            "2030-01-01T10:00:00.",
            "2030-01-01T10:00:00.1234567",
            "2030-01-01T10:00:00+3",
            "2030-01-01T10:00:00+16",
            "2030-01-01T10:00:00+03:60",
            "2030-01-01T10:00:00ZZ",
            "2030-01-01Z",
            "2030-01-01T10:00:00 UTC",
            "２０３０-01-01",
            "2030-01-01\n",
            "2030-01-01T10:00:00\\",
        ] {
            assert!(!valid_until_ok(bad), "{bad:?}");
            let e = create_err(&RoleDef { valid_until: Some(bad.into()), ..role("r") });
            assert_eq!(paths(e.clone()), ["validUntil"], "{bad:?}");
            // Sabit ileti: değer yankılanmaz.
            assert_eq!(e[0].message, VALID_UNTIL_MSG);
        }
    }

    #[test]
    fn valid_until_cannot_be_cleared_and_changes_are_written() {
        let old = RoleDef { valid_until: Some("2030-01-01T00:00:00Z".into()), ..role("r") };
        let e = alter_err(&old, &role("r"));
        assert_eq!(paths(e.clone()), ["validUntil"]);
        assert_eq!(e[0].message, VALID_UNTIL_CLEAR_MSG);
        let p = alter_ok(&old, &RoleDef { valid_until: Some("infinity".into()), ..role("r") });
        assert_eq!(sqls(&p), ["ALTER ROLE r WITH VALID UNTIL 'infinity'"]);
        let p = alter_ok(&role("r"), &RoleDef { valid_until: Some("2031-06-01".into()), ..role("r") });
        assert_eq!(sqls(&p), ["ALTER ROLE r WITH VALID UNTIL '2031-06-01'"]);
        assert_eq!(paths(alter_err(&old, &RoleDef { valid_until: Some("now()".into()), ..role("r") })), ["validUntil"]);
        // Değişmeyen yüklü değer (dilbilgisi dışı) yazılmaz ve doğrulanmaz.
        let odd = RoleDef { valid_until: Some("2030-01-01 00:00:00+00:00:00".into()), ..role("r") };
        assert!(alter_ok(&odd, &odd).statements.is_empty());
    }

    #[test]
    fn alter_attributes_only_changed_with_notes_and_warnings() {
        let base = RoleDef { login: true, ..role("r") };
        let p = alter_ok(&base, &RoleDef { login: false, ..base.clone() });
        assert_eq!(sqls(&p), ["ALTER ROLE r WITH NOLOGIN"]);
        assert_eq!(p.statements[0].note.as_deref(), Some(NOLOGIN_NOTE));
        let p = alter_ok(&role("r"), &RoleDef { login: true, ..role("r") });
        assert_eq!(sqls(&p), ["ALTER ROLE r WITH LOGIN"]);
        assert_eq!(p.statements[0].note, None);
        let new = RoleDef {
            login: false,
            superuser: true,
            createrole: true,
            inherit: false,
            replication: true,
            bypassrls: true,
            connection_limit: 3,
            valid_until: Some("infinity".into()),
            ..base.clone()
        };
        let p = alter_ok(&base, &new);
        assert_eq!(
            sqls(&p),
            ["ALTER ROLE r WITH NOLOGIN SUPERUSER CREATEROLE NOINHERIT REPLICATION BYPASSRLS CONNECTION LIMIT 3 \
              VALID UNTIL 'infinity'"]
        );
        assert_eq!(p.statements[0].note.as_deref(), Some(NOLOGIN_NOTE));
        assert_eq!(p.warnings, [SUPERUSER_WARNING, CREATEROLE_WARNING, REPLICATION_WARNING, BYPASSRLS_WARNING]);
        assert_eq!(p.risk(), Risk::Safe);
        // Geri alma (true → false) uyarısız; zaten açık olan tekrar uyarı vermez.
        let p = alter_ok(&new, &RoleDef { valid_until: new.valid_until.clone(), ..base.clone() });
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        let p = alter_ok(&new, &RoleDef { createdb: true, ..new.clone() });
        assert_eq!(sqls(&p), ["ALTER ROLE r WITH CREATEDB"]);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn memberships_add_remove_and_admin_both_ways() {
        let old = RoleDef { member_of: vec![m("a", false), m("b", true), m("c", false), m("d", true)], ..role("r") };
        let new = RoleDef { member_of: vec![m("b", false), m("c", true), m("d", true), m("e", false), m("f", true)], ..role("r") };
        let p = alter_ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "REVOKE a FROM r",
                "REVOKE ADMIN OPTION FOR b FROM r",
                "GRANT c TO r WITH ADMIN OPTION",
                "GRANT e TO r",
                "GRANT f TO r WITH ADMIN OPTION",
            ]
        );
        let risks: Vec<_> = p.statements.iter().map(|s| s.risk).collect();
        assert_eq!(risks, [Risk::Destructive, Risk::Destructive, Risk::Safe, Risk::Safe, Risk::Safe]);
        assert_eq!(p.statements[0].note.as_deref(), Some(REVOKE_NOTE));
        assert!(p.statements.iter().all(|s| s.transactional && !s.redacted));
        assert_eq!(p.risk(), Risk::Destructive);
        // Yalnız ekleme Safe.
        let p = alter_ok(&role("r"), &RoleDef { member_of: vec![m("a", false)], ..role("r") });
        assert_eq!((sqls(&p), p.risk()), (vec!["GRANT a TO r"], Risk::Safe));
    }

    #[test]
    fn membership_order_is_by_role_name() {
        let r = RoleDef { member_of: vec![m("z", false), m("B", false), m("a", false), m("_x", false)], ..role("r") };
        assert_eq!(
            &sqls(&create(&r, &ctx()))[1..],
            ["GRANT \"B\" TO r", "GRANT _x TO r", "GRANT a TO r", "GRANT z TO r"]
        );
        // Kaldırılanlar da ad sırasıyla (girdi sırasından bağımsız).
        let p = alter_ok(&r, &role("r"));
        assert_eq!(sqls(&p), ["REVOKE \"B\" FROM r", "REVOKE _x FROM r", "REVOKE a FROM r", "REVOKE z FROM r"]);
    }

    #[test]
    fn membership_duplicate_self_and_name_errors() {
        let e = create_err(&RoleDef { member_of: vec![m("a", false), m("a", true)], ..role("r") });
        assert_eq!(paths(e.clone()), ["memberOf[1].role"]);
        assert_eq!(e[0].message, DUPLICATE_MEMBER_MSG);
        let e = create_err(&RoleDef { member_of: vec![m("r", false)], ..role("r") });
        assert_eq!(paths(e.clone()), ["memberOf[0].role"]);
        assert_eq!(e[0].message, SELF_MEMBER_MSG);
        for bad in ["", "a\0", "a\nb", "a\u{202e}b", "a\u{85}b"] {
            assert_eq!(paths(create_err(&RoleDef { member_of: vec![m(bad, false)], ..role("r") })), ["memberOf[0].role"]);
        }
        // Büyük/küçük harf farklı roller ayrıdır (tırnaklı adlar).
        assert_eq!(create(&RoleDef { member_of: vec![m("a", false), m("A", false)], ..role("R") }, &ctx()).statements.len(), 3);
        // Alter: kendine üyelik yeni adla da denetlenir; yinelenen her zaman.
        let old = RoleDef { member_of: vec![m("x", false)], ..role("r") };
        let e = alter_err(&old, &RoleDef { name: "x".into(), ..old.clone() });
        assert_eq!(paths(e), ["memberOf[0].role"]);
        let e = alter_err(&old, &RoleDef { member_of: vec![m("x", false), m("x", false)], ..old.clone() });
        assert_eq!(paths(e), ["memberOf[1].role"]);
        // Alter: değişmeyen yüklü üyelik (görünmez karakterli ad) doğrulanmaz; değişince katı.
        let zw = RoleDef { member_of: vec![m("a\u{200b}b", false)], ..role("r") };
        assert!(alter_ok(&zw, &zw).statements.is_empty());
        assert_eq!(sqls(&alter_ok(&zw, &role("r"))), ["REVOKE \"a\u{200b}b\" FROM r"]);
        let e = alter_err(&zw, &RoleDef { member_of: vec![m("a\u{200b}b", true)], ..role("r") });
        assert_eq!(paths(e), ["memberOf[0].role"]);
        // Eski taraf üyelik adı gevşek ama boş/NUL değil.
        let bad_old = RoleDef { member_of: vec![m("ok", false), m("", false)], ..role("r") };
        assert_eq!(paths(alter_err(&bad_old, &role("r"))), ["old.memberOf[1].role"]);
    }

    #[test]
    fn predefined_role_membership_accepted() {
        for pre in ["pg_read_all_data", "pg_write_all_data", "pg_monitor", "pg_signal_backend"] {
            let p = create(&RoleDef { member_of: vec![m(pre, false)], ..role("r") }, &ctx());
            assert_eq!(sqls(&p)[1], format!("GRANT {pre} TO r"));
            let p = alter_ok(&role("r"), &RoleDef { member_of: vec![m(pre, true)], ..role("r") });
            assert_eq!(sqls(&p), [format!("GRANT {pre} TO r WITH ADMIN OPTION")]);
        }
    }

    #[test]
    fn pg16_membership_options_are_never_written() {
        let pg16 = |admin: bool, inh: Option<bool>, set: Option<bool>| RoleMembership {
            role: "grp".into(),
            admin,
            inherit_option: inh,
            set_option: set,
        };
        let old = RoleDef { member_of: vec![pg16(false, Some(false), Some(true))], ..role("r") };
        // Admin değişimi: yalnızca admin ifadesi; seçenekler (değişmiş olsalar bile) yazılmaz.
        let new = RoleDef { member_of: vec![pg16(true, Some(true), Some(false))], ..role("r") };
        let p = alter_ok(&old, &new);
        assert_eq!(sqls(&p), ["GRANT grp TO r WITH ADMIN OPTION"]);
        let p = alter_ok(&new, &old);
        assert_eq!(sqls(&p), ["REVOKE ADMIN OPTION FOR grp FROM r"]);
        for st in p.statements.iter().chain(alter_ok(&old, &new).statements.iter()) {
            assert!(!st.sql.contains("INHERIT") && !st.sql.contains("SET"), "{}", st.sql);
        }
        // Yalnızca seçenek farkı → boş plan.
        let only_opts = RoleDef { member_of: vec![pg16(false, Some(true), None)], ..role("r") };
        assert!(alter_ok(&old, &only_opts).statements.is_empty());
        // Yeni üyelikte de yazılmaz (sunucu varsayılanı).
        let p = create(&RoleDef { member_of: vec![pg16(false, Some(false), Some(false))], ..role("r") }, &ctx());
        assert_eq!(sqls(&p)[1], "GRANT grp TO r");
    }

    #[test]
    fn public_and_none_are_not_membership_targets() {
        for bad in ["public", "none"] {
            let e = create_err(&RoleDef { member_of: vec![m(bad, false)], ..role("r") });
            assert_eq!(paths(e.clone()), ["memberOf[0].role"], "{bad}");
            assert_eq!(e[0].message, RESERVED_MEMBER_MSG);
            let e = alter_err(&role("r"), &RoleDef { member_of: vec![m(bad, true)], ..role("r") });
            assert_eq!(paths(e), ["memberOf[0].role"], "{bad}");
        }
        // Büyük harfli (tırnaklı) adlar ve pg_* roller meşru.
        for ok in ["Public", "NONE", "pg_monitor"] {
            assert_eq!(create(&RoleDef { member_of: vec![m(ok, false)], ..role("r") }, &ctx()).statements.len(), 2);
        }
    }

    #[test]
    fn loaded_predefined_roles_friendly_errors() {
        let pre = RoleDef { member_of: vec![m("a", false)], settings: vec![s("work_mem", "1MB")], ..role("pg_monitor") };
        let changes = [
            RoleDef { name: "monitor2".into(), ..pre.clone() },
            RoleDef { login: true, ..pre.clone() },
            RoleDef { connection_limit: 3, ..pre.clone() },
            RoleDef { valid_until: Some("infinity".into()), ..pre.clone() },
            RoleDef { password: PasswordChange::Set, ..pre.clone() },
            RoleDef { password: PasswordChange::Clear, ..pre.clone() },
            RoleDef { settings: vec![], ..pre.clone() },
            RoleDef { settings: vec![s("work_mem", "2MB")], ..pre.clone() },
        ];
        for new in changes {
            let e = alter_err(&pre, &new);
            assert_eq!(paths(e.clone()), ["name"], "{new:?}");
            assert_eq!(e[0].message, PREDEFINED_ALTER_MSG);
        }
        // Yorum ve üyelikler serbest; ad gevşek (kimlik doğrulaması boş).
        let p = alter_ok(&pre, &RoleDef { comment: Some("c".into()), member_of: vec![m("b", true)], ..pre.clone() });
        assert_eq!(sqls(&p), ["REVOKE a FROM pg_monitor", "GRANT b TO pg_monitor WITH ADMIN OPTION", "COMMENT ON ROLE pg_monitor IS 'c'"]);
        assert!(alter_ok(&pre, &pre).statements.is_empty());
        assert!(validate_identity(&o(&pre)).is_empty());
        // Silme: açık ileti.
        let e = drop_with(&o(&pre), DropOptions::default(), &ctx()).unwrap_err();
        assert_eq!(paths(e.clone()), ["name"]);
        assert_eq!(e[0].message, PREDEFINED_DROP_MSG);
        // Önek büyük/küçük harfe duyarlı (PG IsReservedName gibi).
        assert!(drop_with(&o(&role("PG_x")), DropOptions::default(), &ctx()).is_ok());
        assert_eq!(sqls(&alter_ok(&role("Pg_y"), &role("pgy"))), ["ALTER ROLE \"Pg_y\" RENAME TO pgy"]);
    }

    #[test]
    fn reserved_names() {
        for bad in ["public", "none", "pg_x", "pg_", "pg_read_all_data"] {
            let e = create_err(&role(bad));
            assert_eq!(paths(e.clone()), ["name"], "{bad}");
            assert_eq!(e[0].message, RESERVED_MSG);
            assert!(e[0].message.starts_with("Bu rol adı ayrılmıştır"));
            // Rename hedefi de.
            let e = alter_err(&role("r"), &role(bad));
            assert_eq!(paths(e.clone()), ["name"], "{bad}");
            assert_eq!(e[0].message, RESERVED_MSG);
        }
        for ok in ["Public", "NONE", "pgx", "PG_x", "Pg_y", "my_pg_role", "publicx", "nonex"] {
            assert_eq!(create(&role(ok), &ctx()).statements.len(), 1, "{ok}");
            assert_eq!(alter_ok(&role("r"), &role(ok)).statements.len(), 1, "{ok}");
        }
        // Yüklü önceden tanımlı rol (eski taraf) gevşek: adı değişmedikçe düzenlenebilir; silme denemesi sunucuya kalır.
        let pre = role("pg_monitor");
        assert_eq!(sqls(&alter_ok(&pre, &RoleDef { comment: Some("c".into()), ..pre.clone() })), ["COMMENT ON ROLE pg_monitor IS 'c'"]);
        assert!(validate_identity(&o(&pre)).is_empty());
        // Yeni adlarda bidi/C1/denetim karakteri ve uzunluk kuralı.
        let long64 = "a".repeat(64);
        for bad in ["a\u{2067}b", "a\u{85}b", "a\nb", "", long64.as_str()] {
            assert_eq!(paths(create_err(&role(bad))), ["name"], "{bad:?}");
            assert_eq!(paths(alter_err(&role("r"), &role(bad))), ["name"], "{bad:?}");
        }
    }

    #[test]
    fn old_name_lenient() {
        let zw = role("r\u{200b}x");
        assert!(validate_identity(&o(&zw)).is_empty());
        assert_eq!(sqls(&alter_ok(&zw, &role("rx"))), ["ALTER ROLE \"r\u{200b}x\" RENAME TO rx"]);
        assert_eq!(paths(alter_err(&role(""), &role("r"))), ["old.name"]);
        assert_eq!(paths(alter_err(&role("a\0"), &role("r"))), ["old.name"]);
        // Eski taraf ayar adı: yalnızca boş/NUL/denetim karakteri, `old.` önekiyle; kanonik ad RESET'te tırnaklı.
        let old = RoleDef { settings: vec![s("TimeZone", "UTC")], ..role("r") };
        assert_eq!(sqls(&alter_ok(&old, &role("r"))), ["ALTER ROLE r RESET \"TimeZone\""]);
        let bad_old = RoleDef { settings: vec![s("ok", "1"), s("a\nb", "1")], ..role("r") };
        assert_eq!(paths(alter_err(&bad_old, &role("r"))), ["old.settings[1].name"]);
    }

    #[test]
    fn alter_validates_only_written_settings_and_comment() {
        let old = RoleDef { settings: vec![s("TimeZone", "UTC"), s("work_mem", "a\nb")], comment: Some("a\0".into()), ..role("r") };
        assert!(alter_ok(&old, &old).statements.is_empty());
        let p = alter_ok(&old, &RoleDef { connection_limit: 2, ..old.clone() });
        assert_eq!(sqls(&p), ["ALTER ROLE r WITH CONNECTION LIMIT 2"]);
        let e = alter_err(&old, &RoleDef { settings: vec![s("DateStyle", "ISO")], ..old.clone() });
        assert_eq!(paths(e), ["settings[0].name"]);
        // Task 4 M-1: yinelenen girdi yüklü değerle aynı olsa da hata.
        let a1 = RoleDef { settings: vec![s("a", "1")], ..role("r") };
        for list in [vec![s("a", "2"), s("a", "1")], vec![s("a", "1"), s("a", "2")], vec![s("a", "1"), s("a", "1")]] {
            assert_eq!(paths(alter_err(&a1, &RoleDef { settings: list.clone(), ..role("r") })), ["settings[1].name"], "{list:?}");
        }
        assert_eq!(paths(alter_err(&old, &RoleDef { comment: Some("b\0".into()), ..old.clone() })), ["comment"]);
        // Ayar sırası: RESET (ad sırası) sonra SET (ad sırası).
        let old = RoleDef { settings: vec![s("work_mem", "1MB"), s("b", "1")], ..role("r") };
        let p = alter_ok(&old, &RoleDef { settings: vec![s("zz.x", "1"), s("aa", "2"), s("b", "1")], ..role("r") });
        assert_eq!(sqls(&p), ["ALTER ROLE r RESET work_mem", "ALTER ROLE r SET aa = '2'", "ALTER ROLE r SET zz.x = '1'"]);
        assert!(p.statements.iter().all(|s| s.risk == Risk::Safe));
    }

    #[test]
    fn alter_combined_order_uses_new_name() {
        let old = RoleDef {
            login: true,
            member_of: vec![m("old_grp", false), m("adm", true), m("keep", false)],
            settings: vec![s("work_mem", "1MB"), s("timezone", "UTC")],
            comment: Some("c".into()),
            valid_until: Some("2030-01-01T00:00:00Z".into()),
            ..role("app")
        };
        let new = RoleDef {
            name: "Select".into(),
            login: false,
            createdb: true,
            connection_limit: 10,
            valid_until: Some("infinity".into()),
            password: PasswordChange::Set,
            member_of: vec![m("keep", true), m("adm", false), m("new_grp", false)],
            settings: vec![s("work_mem", "2MB"), s("app.user", "x")],
            comment: Some("yeni".into()),
            ..old.clone()
        };
        let p = alter_ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "ALTER ROLE app RENAME TO \"Select\"",
                "ALTER ROLE \"Select\" WITH NOLOGIN CREATEDB CONNECTION LIMIT 10 VALID UNTIL 'infinity'",
                "ALTER ROLE \"Select\" PASSWORD ********",
                "REVOKE ADMIN OPTION FOR adm FROM \"Select\"",
                "REVOKE old_grp FROM \"Select\"",
                "GRANT \"keep\" TO \"Select\" WITH ADMIN OPTION",
                "GRANT new_grp TO \"Select\"",
                "ALTER ROLE \"Select\" RESET timezone",
                "ALTER ROLE \"Select\" SET app.\"user\" = 'x'",
                "ALTER ROLE \"Select\" SET work_mem = '2MB'",
                "COMMENT ON ROLE \"Select\" IS 'yeni'",
            ]
        );
        assert!(p.statements.iter().all(|s| s.transactional));
        assert_eq!(p.statements.iter().filter(|s| s.redacted).count(), 1);
        assert!(p.statements[2].redacted);
        assert_eq!(p.risk(), Risk::Destructive);
        assert_eq!(p.warnings, [MD5_RENAME_WARNING, PASSWORD_WARNING]);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn alter_noop_is_empty() {
        let x = RoleDef {
            login: true,
            superuser: true,
            createrole: true,
            replication: true,
            bypassrls: true,
            connection_limit: 4,
            valid_until: Some("2030-01-01T10:00:00.5Z".into()),
            member_of: vec![
                m("b", true),
                RoleMembership { role: "a".into(), admin: false, inherit_option: Some(true), set_option: Some(false) },
            ],
            settings: vec![s("work_mem", "1MB"), s("search_path", "\"$user\", public")],
            comment: Some("c".into()),
            unsupported: vec!["IN DATABASE d: work_mem=1MB".into()],
            ..role("app")
        };
        for x in [x.clone(), role("r")] {
            let p = alter_ok(&x, &x);
            assert!(p.statements.is_empty() && p.warnings.is_empty(), "{x:?}");
        }
    }

    #[test]
    fn drop_variants() {
        let p = drop_with(&o(&role("Old Role")), DropOptions::default(), &ctx()).unwrap();
        assert_single_statements(&p);
        assert_eq!(sqls(&p), ["DROP ROLE \"Old Role\""]);
        assert_eq!(p.statements[0].risk, Risk::Destructive);
        assert_eq!(p.statements[0].note.as_deref(), Some(DROP_NOTE));
        assert!(p.statements[0].transactional && !p.statements[0].redacted);
        assert!(p.warnings.is_empty());
        // Hiçbir yolla REASSIGN/DROP OWNED üretilmez.
        assert!(!p.statements.iter().any(|s| s.sql.contains("OWNED")));
        // CASCADE ve FORCE ortak kuralla reddedilir.
        let e = drop_with(&o(&role("r")), DropOptions { cascade: true, force: false }, &ctx()).unwrap_err();
        assert_eq!(paths(e), ["cascade"]);
        let e = drop_with(&o(&role("r")), DropOptions { cascade: false, force: true }, &ctx()).unwrap_err();
        assert_eq!(paths(e), ["force"]);
        // Yüklü ad gevşek (yalnız boş/NUL red); diğer alanlar ne olursa olsun silinebilir.
        let odd = RoleDef { name: "a\u{200b}\nb".into(), connection_limit: -9, valid_until: Some("x".into()), ..role("r") };
        let p = drop_with(&o(&odd), DropOptions::default(), &ctx()).unwrap();
        assert_eq!(sqls(&p), ["DROP ROLE \"a\u{200b}\nb\""]);
        let e = drop_with(&o(&role("a\0")), DropOptions::default(), &ctx()).unwrap_err();
        assert_eq!(paths(e), ["name"]);
        insta::assert_snapshot!(render(&drop_with(&o(&role("r")), DropOptions::default(), &ctx()).unwrap()));
    }

    #[test]
    fn injection_matrix() {
        let long63 = "a".repeat(63);
        for n in [
            "r'; DROP ROLE x; --",
            "r\"; DROP ROLE x; --",
            "select",
            "user",
            "Rol ş",
            "日本",
            long63.as_str(),
            "'",
            "\\",
            "/*",
            "$$",
            "--",
        ] {
            let short: String = n.chars().take(20).collect();
            let x = RoleDef {
                name: n.into(),
                password: PasswordChange::Set,
                member_of: vec![m(&format!("g{short}"), true)],
                settings: vec![s("app.x", n), s("search_path", &format!("\"{}\"", n.replace('"', "\"\"")))],
                comment: Some(n.into()),
                valid_until: Some("infinity".into()),
                ..role("r")
            };
            let renamed = RoleDef {
                name: format!("{short}x"),
                member_of: vec![m(&format!("h{short}"), false)],
                settings: vec![s("app.y", n)],
                comment: Some(format!("{n}!")),
                ..x.clone()
            };
            for p in [create(&x, &ctx()), alter_ok(&x, &renamed), drop_with(&o(&x), DropOptions::default(), &ctx()).unwrap()] {
                assert_single_statements(&p);
                assert!(!p.statements.is_empty(), "{n:?}");
                assert!(p.statements.iter().all(|s| s.transactional));
            }
            // Parola somutlaştırma hostile değerle de tek ifade.
            assert!(scan_semicolons(materialize_password(n, &Secret::new(n)).unwrap().sql()).is_clean(), "{n:?}");
        }
        let p = create(&RoleDef { member_of: vec![m("g\"; DROP ROLE x; --", false)], ..role("r\"; --") }, &ctx());
        assert_eq!(
            sqls(&p),
            [format!("CREATE ROLE \"r\"\"; --\" WITH {DEFAULT_ATTRS}"), "GRANT \"g\"\"; DROP ROLE x; --\" TO \"r\"\"; --\"".into()]
        );
        // Alan hataları.
        let long64 = "a".repeat(64);
        let cases: Vec<(RoleDef, &str)> = vec![
            (role(&long64), "name"),
            (role("a\0"), "name"),
            (RoleDef { member_of: vec![m(&long64, false)], ..role("r") }, "memberOf[0].role"),
            (RoleDef { member_of: vec![m("x\n)) --", false)], ..role("r") }, "memberOf[0].role"),
            (RoleDef { valid_until: Some("2030-01-01'; DROP ROLE x; --".into()), ..role("r") }, "validUntil"),
            (RoleDef { valid_until: Some("2030-01-01\0".into()), ..role("r") }, "validUntil"),
            (RoleDef { settings: vec![s("a; DROP ROLE x", "1")], ..role("r") }, "settings[0].name"),
            (RoleDef { settings: vec![s("a\0", "1")], ..role("r") }, "settings[0].name"),
            (RoleDef { settings: vec![s("work_mem", "1\0")], ..role("r") }, "settings[0].value"),
            (RoleDef { settings: vec![s("work_mem", "a\nb")], ..role("r") }, "settings[0].value"),
            (RoleDef { settings: vec![s("search_path", "\"x")], ..role("r") }, "settings[0].value"),
            (RoleDef { comment: Some("a\0b".into()), ..role("r") }, "comment"),
        ];
        for (x, want) in cases {
            assert_eq!(paths(create_err(&x)), [want], "{x:?}");
            // Alter: aynı alan değişince aynı kapı (ad değişimi yalnızca hedef ad).
            assert_eq!(paths(alter_err(&role("r"), &x)), [want], "{x:?}");
        }
    }
}
