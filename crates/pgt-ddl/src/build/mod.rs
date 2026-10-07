//! Saf DDL üreticileri: tanım → `Plan` (veritabanı/IO yok). Tüm ad/değer yerleştirmeleri `quote` yardımcılarından geçer.
//!
//! Sözleşme: her `Statement.sql` **tek** ifadedir ve sonda `;` taşımaz (çalıştırıcı gerekirse ekler);
//! çıktı deterministiktir (aynı girdi → aynı metin ve sıra).
//!
//! Serbest SQL alanları (Task 4): her alan `scan_semicolons` ile taranır ve `!is_clean()` ise (dış `;` ya da
//! kapanmamış tırnak/yorum) REDDEDİLİR; ayrıca alanlar birleştirildikten sonra **nihai ifade** de taranmalıdır
//! (alan sınırlarında kapanma/açılma kaymasını yakalamak için). Tarayıcı `standard_conforming_strings = on` varsayar.

pub mod extension;
pub mod freesql;
pub mod index;
pub mod matview;
pub mod schema;
pub mod sequence;
pub mod settings;
mod sqlscan;
pub mod table;
pub mod table_alter;
pub mod table_constraint;
pub mod type_canon;
pub mod type_syntax;
pub mod view;

pub use sqlscan::{ScanResult, scan_semicolons};

use crate::model::{FieldError, ObjectDef, ObjectKind};
use crate::plan::{Plan, Risk, Statement};

/// PG tanımlayıcı üst sınırı (BAYT, UTF-8).
pub const MAX_IDENT_BYTES: usize = 63;

/// Üreticilerin bağlamı.
#[derive(Debug, Clone, Default)]
pub struct BuildCtx {
    /// `server_version_num` (ör. 160002). İndeks üreticisi INCLUDE kapısı (PG 11+) için kullanır.
    pub version_num: u32,
    /// `CREATE INDEX CONCURRENTLY` istenir mi (indeks üreticisi: CREATE/DROP INDEX CONCURRENTLY).
    pub concurrently: bool,
}

pub(crate) fn err(path: &str, message: impl Into<String>) -> FieldError {
    FieldError {
        path: path.to_string(),
        message: message.into(),
    }
}

/// Ad kuralları: boş değil, NUL/denetim karakteri (Unicode Cc: ASCII satır sonu, sekme, 0x7f ve C1 `U+0080–U+009F`)
/// yok, görünmez biçim/yön denetimi karakteri (tip dilbilgisindeki küme: `U+061C`, `U+200B–U+200F`, `U+2028–U+202E`,
/// `U+2060–U+2064`, `U+2066–U+2069`, `U+FEFF`) yok, en çok 63 bayt.
pub(crate) fn check_ident(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    if value.is_empty() {
        errs.push(err(path, "Ad boş olamaz."));
    } else if value.contains('\0') {
        errs.push(err(path, "Ad NUL karakteri içeremez."));
    } else if value.chars().any(char::is_control) {
        // Satır sonu/sekme vb.: tırnaklı ad içinde ham kalır; `--` yorumunu bitirip sonrasını SQL yapabilir. C1 denetim
        // karakterleri (U+0080–U+009F, ör. NEL) de görünmezdir ve bazı araçlarda satır sonu sayılır.
        errs.push(err(
            path,
            "Ad denetim karakteri (satır sonu, sekme vb.) içeremez.",
        ));
    } else if let Some(c) = value.chars().find(|&c| type_syntax::is_spoofing_format(c)) {
        // Tırnaklı adda zararsızdır ama görüntüyü aldatır (ör. sağdan sola çevirme ile "gizli" ad).
        errs.push(err(
            path,
            format!(
                "Ad görünmez biçim ya da yön denetimi karakteri içeremez (U+{:04X}).",
                c as u32
            ),
        ));
    } else if value.len() > MAX_IDENT_BYTES {
        errs.push(err(
            path,
            format!(
                "Ad en çok {MAX_IDENT_BYTES} bayt olabilir (şu an {} bayt).",
                value.len()
            ),
        ));
    }
}

/// YÜKLENMİŞ (eski taraf) ad: yalnızca `quote_ident` ile yerleşir; sunucudan geldiği için denetim/biçim karakteri ya da
/// uzunluk kuralı uygulanmaz (bunlar var olan bir tablonun yeniden adlandırılmasını/düzenlenmesini engellemesin).
/// NUL ve boş ad yine reddedilir (`quote_ident` yalnızca debug-assert eder).
pub(crate) fn check_old_ident(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    if value.is_empty() {
        errs.push(err(path, "Ad boş olamaz."));
    } else if value.contains('\0') {
        errs.push(err(path, "Ad NUL karakteri içeremez."));
    }
}

/// YÜKLENMİŞ (eski taraf) şema: `check_old_ident` gevşekliği + sistem şeması yasağı (sistem nesnesine dokunulmaz).
pub(crate) fn check_old_schema_ident(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    let before = errs.len();
    check_old_ident(path, value, errs);
    if errs.len() == before && is_system_schema(value) {
        errs.push(err(
            path,
            "Sistem şemaları (pg_*, information_schema) kullanılamaz.",
        ));
    }
}

/// Şema olarak kullanılacak ad: ad kuralları + sistem şemaları yasak (`pg_*`, `information_schema`).
pub(crate) fn check_schema_ident(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    let before = errs.len();
    check_ident(path, value, errs);
    if errs.len() == before && is_system_schema(value) {
        errs.push(err(
            path,
            "Sistem şemaları (pg_*, information_schema) kullanılamaz.",
        ));
    }
}

pub(crate) fn is_system_schema(name: &str) -> bool {
    // PG'nin ayrılmış ad kuralı büyük/küçük harfe duyarlıdır ("PG_Data" geçerli kullanıcı şemasıdır).
    name.starts_with("pg_") || name == "information_schema"
}

/// Serbest metin değeri (yorum, sürüm): NUL yasak.
pub(crate) fn check_text(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    if value.contains('\0') {
        errs.push(err(path, "Değer NUL karakteri içeremez."));
    }
}

/// Transaction içinde çalışabilir ifade.
pub(crate) fn stmt(sql: String, risk: Risk, note: Option<&str>) -> Statement {
    Statement {
        sql,
        risk,
        transactional: true,
        note: note.map(str::to_string),
        redacted: false,
    }
}

/// Transaction dışında çalışması gereken ifade (`stmt` gibi, `transactional: false`).
// Task 3 (Database üreticisi) tüketir.
#[allow(dead_code)]
pub(crate) fn stmt_nt(sql: String, risk: Risk, note: Option<&str>) -> Statement {
    Statement {
        transactional: false,
        ..stmt(sql, risk, note)
    }
}

/// Silme seçenekleri: `force` yalnızca Database (`WITH (FORCE)`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DropOptions {
    pub cascade: bool,
    pub force: bool,
}

/// Task 4–5'in üreticileri gelene kadar yeni türlerin tek yer tutucu hatası.
fn unsupported_kind() -> Vec<FieldError> {
    vec![err("kind", "Bu tür henüz desteklenmiyor.")]
}

fn mismatch(old: &ObjectDef, new: &ObjectDef) -> Vec<FieldError> {
    vec![err(
        "kind",
        format!("Tür uyuşmazlığı: {:?} → {:?}.", old.kind(), new.kind()),
    )]
}

/// Alan düzeyi doğrulama; boş liste = geçerli.
pub fn validate(def: &ObjectDef, ctx: &BuildCtx) -> Vec<FieldError> {
    match def {
        ObjectDef::Schema(d) => schema::validate(d),
        ObjectDef::Sequence(d) => sequence::validate(d),
        ObjectDef::Extension(d) => extension::validate(d),
        ObjectDef::Index(d) => index::validate(d, ctx),
        ObjectDef::Table(d) => table::validate_table(d, ctx),
        ObjectDef::View(d) => view::validate(d, ctx),
        ObjectDef::MaterializedView(d) => matview::validate(d, ctx),
        ObjectDef::Database(_) | ObjectDef::Role(_) => unsupported_kind(),
    }
}

/// Yalnızca KİMLİK alanları (şema/tablo/ad): `drop` ve `alter`'ın eski tarafı yalnızca bunları SQL'e koyar. Yüklenmiş
/// bir nesne (özel erişim yöntemi, eski ifade vb.) tam oluşturma doğrulamasına takılsa bile silinebilir/yeniden
/// adlandırılabilir kalmalı; tam doğrulama yalnızca oluşturma ve alter'ın yeni tarafındadır.
pub(crate) fn validate_identity(def: &ObjectDef) -> Vec<FieldError> {
    let mut e = vec![];
    match def {
        ObjectDef::Schema(d) => check_schema_ident("name", &d.name, &mut e),
        ObjectDef::Sequence(d) => {
            check_schema_ident("schema", &d.schema, &mut e);
            check_ident("name", &d.name, &mut e);
        }
        ObjectDef::Extension(d) => check_ident("name", &d.name, &mut e),
        ObjectDef::Index(d) => {
            check_schema_ident("schema", &d.schema, &mut e);
            check_ident("table", &d.table, &mut e);
            check_ident("name", &d.name, &mut e);
        }
        ObjectDef::Table(d) => {
            check_schema_ident("schema", &d.schema, &mut e);
            check_ident("name", &d.name, &mut e);
        }
        // Görünüm: yüklü ad başka araçla oluşturulmuş olabilir (ör. sıfır genişlikli boşluk); yalnızca `quote_ident` ile
        // yerleştiği için gevşek kural yeterli ve yanıltıcı adı düzeltmenin yolu tam da yeniden adlandırma/silmedir.
        ObjectDef::View(d) => {
            check_old_schema_ident("schema", &d.schema, &mut e);
            check_old_ident("name", &d.name, &mut e);
        }
        // Materialized View: görünümle aynı gerekçe (gevşek eski taraf; sistem şeması yine yasak).
        ObjectDef::MaterializedView(d) => {
            check_old_schema_ident("schema", &d.schema, &mut e);
            check_old_ident("name", &d.name, &mut e);
        }
        ObjectDef::Database(d) => check_ident("name", &d.name, &mut e),
        ObjectDef::Role(d) => check_ident("name", &d.name, &mut e),
    }
    e
}

fn checked(def: &ObjectDef, ctx: &BuildCtx) -> Result<(), Vec<FieldError>> {
    let errs = validate(def, ctx);
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

pub fn create(def: &ObjectDef, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    // Tablo: paylaşılan doğrulamaya ek olarak yalnızca oluşturmaya özgü kurallar ve nihai ifade taraması.
    let errs = match def {
        ObjectDef::Table(d) => table::validate_create(d, ctx),
        _ => validate(def, ctx),
    };
    if !errs.is_empty() {
        return Err(errs);
    }
    match def {
        ObjectDef::Schema(d) => Ok(schema::create(d)),
        ObjectDef::Sequence(d) => Ok(sequence::create(d)),
        ObjectDef::Extension(d) => Ok(extension::create(d)),
        ObjectDef::Index(d) => Ok(index::create(d, ctx)),
        ObjectDef::Table(d) => Ok(table::create(d)),
        ObjectDef::View(d) => Ok(view::create(d)),
        ObjectDef::MaterializedView(d) => Ok(matview::create(d)),
        ObjectDef::Database(_) | ObjectDef::Role(_) => Err(unsupported_kind()),
    }
}

/// Aynı türde iki tanım arasındaki fark; ad değişimi `RENAME`. Fark yoksa boş plan.
/// Yeni tanım tam doğrulanır; eski tanımın yalnızca kimlik alanları (bkz. `validate_identity`).
pub fn alter(old: &ObjectDef, new: &ObjectDef, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    if old.kind() != new.kind() {
        return Err(mismatch(old, new));
    }
    if let (ObjectDef::Table(o), ObjectDef::Table(n)) = (old, new) {
        // Tablo kendi doğrulamasını yapar: eski tarafta kimlik alanlarına kolon adları/attnum'lar da girer, yeni
        // tarafta alter'a özgü kurallar (eşleme, bölüm/kalıtım koruması, üretilmiş kolon geçişleri) vardır.
        return table_alter::alter(o, n, ctx);
    }
    if let (ObjectDef::View(o), ObjectDef::View(n)) = (old, new) {
        // Görünüm kendi doğrulamasını yapar: sorgu yalnızca değiştiyse doğrulanır (yüklü `pg_get_viewdef` metni serbest
        // SQL kapısına takılsa bile sorguya dokunmayan düzenlemeler yapılabilsin).
        return view::alter(o, n, ctx);
    }
    if let (ObjectDef::MaterializedView(o), ObjectDef::MaterializedView(n)) = (old, new) {
        // MV de kendi doğrulamasını yapar (sorgu yalnızca değiştiyse; yerinde düzenleme yalnızca değişen alanları).
        return matview::alter(o, n, ctx);
    }
    checked(new, ctx)?;
    // Eski tanımdaki ad/şema da SQL'e girer (RENAME/ALTER hedefi); yalnızca kimlik alanları doğrulanır.
    let mut old_errs = validate_identity(old);
    if !old_errs.is_empty() {
        for e in &mut old_errs {
            e.path = format!("old.{}", e.path);
        }
        return Err(old_errs);
    }
    match (old, new) {
        (ObjectDef::Schema(o), ObjectDef::Schema(n)) => Ok(schema::alter(o, n)),
        (ObjectDef::Sequence(o), ObjectDef::Sequence(n)) => Ok(sequence::alter(o, n)),
        (ObjectDef::Extension(o), ObjectDef::Extension(n)) => extension::alter(o, n),
        (ObjectDef::Index(o), ObjectDef::Index(n)) => index::alter(o, n),
        (ObjectDef::Database(_), ObjectDef::Database(_))
        | (ObjectDef::Role(_), ObjectDef::Role(_)) => Err(unsupported_kind()),
        _ => Err(mismatch(old, new)),
    }
}

pub fn drop(def: &ObjectDef, cascade: bool, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    drop_with(def, DropOptions { cascade, force: false }, ctx)
}

/// Silme; ortak kurallar: `force` yalnızca Database, `cascade` Database/Role'de yok.
pub fn drop_with(def: &ObjectDef, o: DropOptions, _ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    let mut errs = validate_identity(def);
    let kind = def.kind();
    if o.force && kind != ObjectKind::Database {
        errs.push(err("force", "WITH (FORCE) yalnızca veritabanı silmede kullanılır."));
    }
    if o.cascade && matches!(kind, ObjectKind::Database | ObjectKind::Role) {
        errs.push(err("cascade", "Bu tür CASCADE ile silinemez."));
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    let cascade = o.cascade;
    match def {
        ObjectDef::Schema(d) => Ok(schema::drop(d, cascade)),
        ObjectDef::Sequence(d) => Ok(sequence::drop(d, cascade)),
        ObjectDef::Extension(d) => Ok(extension::drop(d, cascade)),
        ObjectDef::Index(d) => index::drop(d, cascade),
        ObjectDef::Table(d) => Ok(table::drop(d, cascade)),
        ObjectDef::View(d) => Ok(view::drop(d, cascade)),
        ObjectDef::MaterializedView(d) => Ok(matview::drop(d, cascade)),
        ObjectDef::Database(_) | ObjectDef::Role(_) => Err(unsupported_kind()),
    }
}

/// Testler için okunabilir döküm: `[risk] sql  -- not`, ardından uyarılar.
#[cfg(test)]
pub(crate) fn render(plan: &Plan) -> String {
    let mut out = String::new();
    for s in &plan.statements {
        out.push_str(&format!(
            "[{:?}{}] {}",
            s.risk,
            if s.transactional { "" } else { " non-tx" },
            s.sql
        ));
        if let Some(n) = &s.note {
            out.push_str(&format!("  -- {n}"));
        }
        out.push('\n');
    }
    for w in &plan.warnings {
        out.push_str(&format!("WARN: {w}\n"));
    }
    out
}

/// Her ifade tek ifade ve sonda `;` yok (enjeksiyon denetimi).
#[cfg(test)]
pub(crate) fn assert_single_statements(plan: &Plan) {
    for s in &plan.statements {
        assert!(
            scan_semicolons(&s.sql).is_clean(),
            "dış ';' ya da kapanmamış bölge var: {}",
            s.sql
        );
        assert!(!s.sql.trim_end().ends_with(';'), "sonda ';' var: {}", s.sql);
        assert!(!s.sql.contains('\0'));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn ctx() -> BuildCtx {
        BuildCtx {
            version_num: 160000,
            ..Default::default()
        }
    }

    #[test]
    fn default_ctx_is_not_concurrent() {
        assert!(!BuildCtx::default().concurrently);
    }

    #[test]
    fn alter_rejects_kind_mismatch() {
        let s = ObjectDef::Schema(SchemaDef {
            name: "a".into(),
            owner: None,
            comment: None,
        });
        let e = ObjectDef::Extension(ExtensionDef {
            name: "a".into(),
            schema: None,
            version: None,
            cascade: false,
        });
        let errs = alter(&s, &e, &ctx()).unwrap_err();
        assert_eq!(errs[0].path, "kind");
        assert!(errs[0].message.contains("uyuşmazlığı"));
    }

    fn table() -> ObjectDef {
        ObjectDef::Table(TableDef {
            schema: "public".into(),
            name: "t".into(),
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
    fn table_is_wired() {
        let t = table();
        // create/drop/validate/alter tablo üreticisine bağlı. Kolonsuz tablo: create reddeder ("columns"); PG kolonsuz
        // tabloya izin verdiğinden paylaşılan doğrulama ve alter kabul eder (boş değişiklik → boş plan).
        assert!(validate(&t, &ctx()).is_empty());
        assert_eq!(create(&t, &ctx()).unwrap_err()[0].path, "columns");
        assert!(alter(&t, &t, &ctx()).unwrap().statements.is_empty());
        let ObjectDef::Table(mut named) = t.clone() else { unreachable!() };
        named.name = String::new();
        assert_eq!(alter(&t, &ObjectDef::Table(named), &ctx()).unwrap_err()[0].path, "name");
        assert!(drop(&t, false, &ctx()).is_ok());
        // Kimlik doğrulaması tablo için de çalışır (eski taraf/drop).
        assert!(validate_identity(&t).is_empty());
        let ObjectDef::Table(mut bad) = t.clone() else { unreachable!() };
        bad.schema = "pg_catalog".into();
        bad.name = "x\n".into();
        let paths: Vec<String> = validate_identity(&ObjectDef::Table(bad.clone())).into_iter().map(|e| e.path).collect();
        assert_eq!(paths, ["schema", "name"]);
        assert_eq!(drop(&ObjectDef::Table(bad), false, &ctx()).unwrap_err()[0].path, "schema");
        // Tür uyuşmazlığı tablo doğrulamasından önce yakalanır.
        let s = ObjectDef::Schema(SchemaDef { name: "a".into(), owner: None, comment: None });
        assert_eq!(alter(&s, &t, &ctx()).unwrap_err()[0].message, mismatch(&s, &t)[0].message);
    }

    fn view(schema: &str) -> ObjectDef {
        ObjectDef::View(ViewDef {
            schema: schema.into(),
            name: "v".into(),
            owner: None,
            query: "SELECT 1".into(),
            check_option: None,
            security_barrier: false,
            security_invoker: false,
            comment: None,
            recreate: false,
            columns: vec![],
            unsupported: vec![],
        })
    }

    fn matview() -> ObjectDef {
        ObjectDef::MaterializedView(MatViewDef {
            schema: "public".into(),
            name: "mv".into(),
            owner: None,
            query: "SELECT 1".into(),
            tablespace: None,
            fillfactor: None,
            with_data: true,
            comment: None,
            refresh: None,
            columns: vec![],
            unsupported: vec![],
        })
    }

    fn database() -> ObjectDef {
        ObjectDef::Database(DatabaseDef {
            name: "d".into(),
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
        })
    }

    fn role(name: &str) -> ObjectDef {
        ObjectDef::Role(RoleDef {
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
        })
    }

    #[test]
    fn drop_with_common_rules() {
        let s = ObjectDef::Schema(SchemaDef { name: "a".into(), owner: None, comment: None });
        let force = DropOptions { cascade: false, force: true };
        assert_eq!(drop_with(&s, force, &ctx()).unwrap_err()[0].path, "force");
        let cascade = DropOptions { cascade: true, force: false };
        for d in [database(), role("r")] {
            let paths: Vec<String> = drop_with(&d, cascade, &ctx()).unwrap_err().into_iter().map(|e| e.path).collect();
            assert_eq!(paths, ["cascade"], "{d:?}");
        }
        // Role'de force da yasak; iki kural birlikte raporlanır.
        let both = DropOptions { cascade: true, force: true };
        let paths: Vec<String> = drop_with(&role("r"), both, &ctx()).unwrap_err().into_iter().map(|e| e.path).collect();
        assert_eq!(paths, ["force", "cascade"]);
        // `drop` = force'suz `drop_with`; mevcut türlerde CASCADE serbest.
        assert_eq!(drop(&s, true, &ctx()), drop_with(&s, cascade, &ctx()));
        assert!(drop(&s, true, &ctx()).is_ok());
        assert_eq!(DropOptions::default(), DropOptions { cascade: false, force: false });
    }

    #[test]
    fn new_kinds_are_placeholders_for_now() {
        for d in [database(), role("r")] {
            let kind = |r: Result<Plan, Vec<FieldError>>| r.unwrap_err()[0].path.clone();
            assert_eq!(validate(&d, &ctx())[0].path, "kind", "{d:?}");
            assert_eq!(kind(create(&d, &ctx())), "kind");
            assert_eq!(kind(alter(&d, &d, &ctx())), "kind");
            assert_eq!(kind(drop(&d, false, &ctx())), "kind");
            assert!(validate_identity(&d).is_empty(), "{d:?}");
        }
        // FORCE + Database: ortak kuraldan geçer, yer tutucuya ulaşır.
        assert_eq!(drop_with(&database(), DropOptions { cascade: false, force: true }, &ctx()).unwrap_err()[0].path, "kind");
        // Yeni tür ile mevcut tür arasındaki alter yine tür uyuşmazlığıdır.
        let e = alter(&view("public"), &matview(), &ctx()).unwrap_err();
        assert!(e[0].message.contains("uyuşmazlığı"), "{e:?}");
    }

    #[test]
    fn matview_is_wired() {
        let mv = matview();
        assert!(validate(&mv, &ctx()).is_empty());
        assert_eq!(create(&mv, &ctx()).unwrap().statements[0].sql, "CREATE MATERIALIZED VIEW public.mv AS (SELECT 1
) WITH DATA");
        assert!(alter(&mv, &mv, &ctx()).unwrap().statements.is_empty());
        assert_eq!(drop(&mv, false, &ctx()).unwrap().statements[0].sql, "DROP MATERIALIZED VIEW public.mv");
        let ObjectDef::MaterializedView(mut sys) = mv.clone() else { unreachable!() };
        sys.schema = "pg_catalog".into();
        let sys = ObjectDef::MaterializedView(sys);
        assert_eq!(alter(&sys, &mv, &ctx()).unwrap_err()[0].path, "old.schema");
        assert_eq!(drop(&sys, false, &ctx()).unwrap_err()[0].path, "schema");
        assert_eq!(drop_with(&mv, DropOptions { cascade: false, force: true }, &ctx()).unwrap_err()[0].path, "force");
        assert_eq!(alter(&mv, &view("public"), &ctx()).unwrap_err()[0].path, "kind");
    }

    #[test]
    fn view_is_wired() {
        let v = view("public");
        assert!(validate(&v, &ctx()).is_empty());
        assert_eq!(create(&v, &ctx()).unwrap().statements[0].sql, "CREATE VIEW public.v AS (SELECT 1
)");
        assert!(alter(&v, &v, &ctx()).unwrap().statements.is_empty());
        assert_eq!(drop(&v, false, &ctx()).unwrap().statements[0].sql, "DROP VIEW public.v");
        // Eski taraf kimliği `old.` önekiyle; drop'ta sistem şeması reddedilir.
        assert_eq!(alter(&view("pg_catalog"), &v, &ctx()).unwrap_err()[0].path, "old.schema");
        assert_eq!(drop(&view("pg_catalog"), false, &ctx()).unwrap_err()[0].path, "schema");
        assert_eq!(drop_with(&v, DropOptions { cascade: false, force: true }, &ctx()).unwrap_err()[0].path, "force");
    }

    #[test]
    fn new_kinds_validate_identity() {
        let paths = |d: &ObjectDef| validate_identity(d).into_iter().map(|e| e.path).collect::<Vec<_>>();
        assert_eq!(paths(&view("pg_catalog")), ["schema"]);
        let ObjectDef::MaterializedView(mut m) = matview() else { unreachable!() };
        m.schema = "information_schema".into();
        m.name = String::new();
        assert_eq!(paths(&ObjectDef::MaterializedView(m)), ["schema", "name"]);
        assert_eq!(paths(&role("x\n")), ["name"]);
        let ObjectDef::Database(mut d) = database() else { unreachable!() };
        d.name = "a".repeat(64);
        assert_eq!(paths(&ObjectDef::Database(d)), ["name"]);
        // Kimlik hatası drop'ta yer tutucudan önce döner.
        assert_eq!(drop(&role("x\n"), false, &ctx()).unwrap_err()[0].path, "name");
    }

    #[test]
    fn stmt_nt_is_non_transactional() {
        let s = stmt_nt("CREATE DATABASE d".into(), Risk::Safe, Some("n"));
        assert_eq!((s.transactional, s.redacted, s.note.as_deref()), (false, false, Some("n")));
        assert!(stmt("x".into(), Risk::Safe, None).transactional);
    }

    #[test]
    fn names_reject_bidi_isolates_alm_and_c1_for_every_kind() {
        let ObjectDef::Table(base) = table() else { unreachable!() };
        let col = |name: &str| ColumnDef {
            attnum: None,
            name: name.into(),
            data_type: "integer".into(),
            not_null: false,
            default: None,
            identity: None,
            generated: None,
            collation: None,
            comment: None,
            using: None,
        };
        let paths = |d: &ObjectDef| validate_identity(d).into_iter().map(|e| e.path).collect::<Vec<_>>();
        // (ad, reddedilmeli mi): yön yalıtımıyla "ters" görünen ad, ALM, C1 (NEL); meşru Türkçe harfler.
        for (h, bad) in [
            ("admin\u{2067}nimda\u{2069}", true),
            ("a\u{061c}b", true),
            ("a\u{85}b", true),
            ("İğş", false),
            ("Şehir_ğüçö", false),
        ] {
            let want: &[&str] = if bad { &["name"] } else { &[] };
            let s = ObjectDef::Schema(SchemaDef { name: h.into(), owner: None, comment: None });
            assert_eq!(paths(&s), want, "schema {h:?}");
            assert_eq!(validate(&s, &ctx()).is_empty(), !bad, "schema {h:?}");
            let mut t = base.clone();
            t.name = h.into();
            assert_eq!(paths(&ObjectDef::Table(t)), want, "table {h:?}");
            let mut t = base.clone();
            t.columns = vec![col(h)];
            let ce = validate(&ObjectDef::Table(t), &ctx());
            assert_eq!(ce.iter().any(|e| e.path.starts_with("columns[0]")), bad, "column {h:?}: {ce:?}");
            assert_eq!(paths(&role(h)), want, "role {h:?}");
            let ObjectDef::Database(mut d) = database() else { unreachable!() };
            d.name = h.into();
            assert_eq!(paths(&ObjectDef::Database(d)), want, "database {h:?}");
        }
    }

    fn errs_for(v: &str) -> usize {
        let mut e = vec![];
        check_ident("n", v, &mut e);
        e.len()
    }

    #[test]
    fn ident_byte_boundary() {
        assert_eq!(errs_for(&"a".repeat(63)), 0);
        assert_eq!(errs_for(&"a".repeat(64)), 1);
        // 31 × 'ş' (2 bayt) = 62 bayt; + 'a' = 63 tamam; 32 × 'ş' = 64 bayt red.
        assert_eq!(errs_for(&"ş".repeat(31)), 0);
        assert_eq!(errs_for(&format!("{}a", "ş".repeat(31))), 0);
        assert_eq!(errs_for(&"ş".repeat(32)), 1);
        // 21 × '€' (3 bayt) = 63 tamam; sonuna 'a' → 64 red.
        assert_eq!(errs_for(&"€".repeat(21)), 0);
        assert_eq!(errs_for(&format!("{}a", "€".repeat(21))), 1);
        // 63 karakter ama 126 bayt: bayt sınırı karakter sınırından önce çalışır.
        assert_eq!(errs_for(&"ş".repeat(63)), 1);
        // Sınırı çok baytlı karakter ortasında aşan ad (62 + 'ş' = 64 bayt).
        assert_eq!(errs_for(&format!("{}ş", "a".repeat(62))), 1);
        assert_eq!(errs_for(&format!("{}ş", "a".repeat(61))), 0);
    }

    #[test]
    fn ident_rejects_empty_and_nul() {
        assert_eq!(errs_for(""), 1);
        assert_eq!(errs_for("a\0b"), 1);
    }

    #[test]
    fn ident_rejects_control_characters() {
        for v in ["a\nb", "a\rb", "a\tb", "a\x7fb", "\n", "a\x1bb", "x\n)) --"] {
            assert_eq!(errs_for(v), 1, "{v:?}");
        }
        assert_eq!(errs_for("a b"), 0);
        // C1 denetim karakterleri (U+0080–U+009F) de yasak.
        for v in ["ş\u{85}", "a\u{80}b", "a\u{9f}b", "\u{9b}x"] {
            assert_eq!(errs_for(v), 1, "{v:?}");
        }
        // Hemen dışı (NBSP U+00A0) ve Türkçe harfler serbest.
        for v in ["a\u{a0}b", "İğşçöüı", "ĞÜŞİÖÇ"] {
            assert_eq!(errs_for(v), 0, "{v:?}");
        }
    }

    #[test]
    fn ident_rejects_spoofing_format_chars() {
        for c in [
            '\u{061c}', '\u{200b}', '\u{200d}', '\u{200f}', '\u{2028}', '\u{202a}', '\u{202e}', '\u{2060}', '\u{2064}', '\u{2066}',
            '\u{2067}', '\u{2068}', '\u{2069}', '\u{feff}',
        ] {
            let mut e = vec![];
            check_ident("n", &format!("a{c}b"), &mut e);
            assert_eq!(e.len(), 1, "{c:?}");
            assert!(e[0].message.contains(&format!("U+{:04X}", c as u32)), "{e:?}");
        }
        // Kümenin hemen dışındakiler ve sıradan Unicode serbest.
        for v in ["a\u{2027}b", "a\u{202f}b", "a\u{2065}b", "a\u{206a}b", "a\u{061b}b", "a\u{061d}b", "a\u{200a}b", "ünlü ş", "日本"] {
            assert_eq!(errs_for(v), 0, "{v:?}");
        }
        // Şema doğrulayıcısı da aynı kuralı uygular.
        let mut e = vec![];
        check_schema_ident("s", "pub\u{202e}lic", &mut e);
        assert_eq!(e.len(), 1);
    }

    #[test]
    fn system_schema_detection() {
        for s in ["pg_catalog", "pg_toast", "pg_temp_3", "information_schema"] {
            assert!(is_system_schema(s), "{s}");
        }
        for s in [
            "public",
            "pgx",
            "pg",
            "my_pg_schema",
            "information_schemas",
            "PG_Data",
            "Information_Schema",
        ] {
            assert!(!is_system_schema(s), "{s}");
        }
    }
}
