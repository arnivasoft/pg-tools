//! Partition (bölüm) üreticisi: `CREATE TABLE … PARTITION OF`, `ATTACH PARTITION`, sınır değiştirme (DETACH + ATTACH),
//! ayırma (`DETACH PARTITION`, `CONCURRENTLY`/`FINALIZE`) ve `pg_get_expr(relpartbound)` ayrıştırıcısı. Plan 36 Karar 4–7.
//!
//! - **Kapsam:** veritabanı kapsamlı tür; `parent_*` bağlamdan gelir, `strategy`/`key_count`/`detach_pending` üst
//!   tablodan gelen salt okunur bilgilerdir (üretici yalnız doğrulamada okur).
//! - **Sınır değerleri** serbest SQL'dir: `check_free_sql` + `wrap` + nihai sınır metninde `composed_is_clean`
//!   (`wrap` kapanıştan önce `\n` ekler; bkz. `bound_sql`/`parse_bound` notu).
//! - **Riskler:** create (+ sahip/yorum) Safe; attach, sınır değiştirme ve ayırma Destructive (`confirmation_required`).
//! - **Transaction:** `DETACH … CONCURRENTLY` ve `DETACH … FINALIZE` transaction dışıdır (`stmt_nt`).
//! - **Alter:** yalnız sınır değişimi anlamlıdır; kimlik ve tablo özellikleri (sahip/yorum/tablespace/alt bölümleme)
//!   Tablo diyaloğundadır → alan hatası. Sınır aynıysa boş plan.

use super::freesql::{check_free_sql, composed_is_clean, wrap};
use super::sqlscan::{paren_group, split_top_level_commas};
use super::table::{partition_by_sql, version_err};
use super::{BuildCtx, check_ident, check_old_ident, check_old_schema_ident, check_schema_ident, check_text, err, stmt, stmt_nt};
use crate::model::{BoundValue, FieldError, PartitionBound, PartitionByDef, PartitionDef, PartitionKey, PartitionMode, PartitionStrategy};
use crate::plan::{Plan, Risk};
use crate::quote::{qualified, quote_ident, quote_literal};

pub const READ_ONLY_MSG: &str = "Bu bölüm salt okunurdur; değiştirilemez ve ayrılamaz.";
pub const IDENTITY_MSG: &str = "Bölümün adı, şeması ve üst tablosu değiştirilemez; yeniden adlandırma Tablo diyaloğundadır.";
pub const PROPS_MSG: &str = "Bölüm tablosunun özellikleri Tablo diyaloğunda düzenlenir.";
pub const PENDING_MSG: &str = "Yarım kalmış ayırma var; önce ayırmayı tamamlayın (FINALIZE).";
pub const REATTACH_NOTE: &str = "Bölüm aynı transaction içinde yeniden bağlanır; tablo yeniden taranır.";
pub const DETACH_NOTE: &str = "Satırlar üst tablodan görünmez olur; tablo silinmez.";
pub const ATTACH_NOTE: &str = "Tablo taranır ve sınır doğrulanır; üst tabloda SHARE UPDATE EXCLUSIVE kilidi alınır.";
pub const ATTACH_NOTE_OLD: &str = "Tablo taranır ve sınır doğrulanır; üst tabloda ACCESS EXCLUSIVE kilidi alınır.";
const COMPOSED_BROKEN: &str = "İfade, yerleştirildiği ifadeyi bozuyor";

fn parent_sql(d: &PartitionDef) -> String {
    qualified(&d.parent_schema, &d.parent_table)
}

fn child_sql(d: &PartitionDef) -> String {
    qualified(&d.schema, &d.name)
}

/// Serbest SQL değerin yerleşimi: PG 12+ `wrap()` (parantezli ifade); PG 10–11 sınır dilbilgisi yalnız sabit kabul
/// eder (parantezli ifade yok) → değer kırpılmış ham yazılır (`validate_bound` tek sabit olmasını ve `--` ile
/// bitmemesini doğrular).
fn place(ver: u32, sql: &str) -> String {
    if ver >= 120000 { wrap(sql) } else { sql.trim().to_string() }
}

fn value_sql(ver: u32, v: &BoundValue) -> String {
    match v {
        BoundValue::Value { sql } => place(ver, sql),
        BoundValue::Minvalue => "MINVALUE".into(),
        BoundValue::Maxvalue => "MAXVALUE".into(),
    }
}

fn join_values(ver: u32, v: &[BoundValue]) -> String {
    v.iter().map(|x| value_sql(ver, x)).collect::<Vec<_>>().join(", ")
}

/// Sınır metni: `FOR VALUES FROM (…) TO (…)`, `FOR VALUES IN (…)`, `FOR VALUES WITH (MODULUS m, REMAINDER r)`, `DEFAULT`.
/// Serbest SQL değerler PG 12+ `wrap()` ile yerleşir (`(<sql>\n)`), bu yüzden `parse_bound(bound_sql(ver, b))` değerleri
/// `wrap`'lı metin olarak geri verir; PG 10–11'de ham yazılır ve düz değerlerde aynen döner. Gerçek sabit nokta
/// yükleyicinin ürettiği metinle sınanır (Task 3).
pub(crate) fn bound_sql(ver: u32, b: &PartitionBound) -> String {
    match b {
        PartitionBound::Range { from, to } => format!("FOR VALUES FROM ({}) TO ({})", join_values(ver, from), join_values(ver, to)),
        PartitionBound::List { values } => {
            format!("FOR VALUES IN ({})", values.iter().map(|v| place(ver, v)).collect::<Vec<_>>().join(", "))
        }
        PartitionBound::Hash { modulus, remainder } => format!("FOR VALUES WITH (MODULUS {modulus}, REMAINDER {remainder})"),
        PartitionBound::Default => "DEFAULT".into(),
    }
}

fn attach_sql(ver: u32, d: &PartitionDef, b: &PartitionBound) -> String {
    format!("ALTER TABLE {} ATTACH PARTITION {} {}", parent_sql(d), child_sql(d), bound_sql(ver, b))
}

fn detach_sql(d: &PartitionDef) -> String {
    format!("ALTER TABLE {} DETACH PARTITION {}", parent_sql(d), child_sql(d))
}

fn attach_stmt(ctx: &BuildCtx, d: &PartitionDef, b: &PartitionBound) -> crate::plan::Statement {
    let note = if ctx.version_num >= 120000 { ATTACH_NOTE } else { ATTACH_NOTE_OLD };
    stmt(attach_sql(ctx.version_num, d, b), Risk::Destructive, Some(note))
}

/// Yorum normalleştirme: `Some("")` yorumsuz sayılır (Policy/Table ile tutarlı).
fn norm_comment(c: &Option<String>) -> Option<&str> {
    c.as_deref().filter(|s| !s.is_empty())
}

fn create_sql(ver: u32, d: &PartitionDef) -> String {
    let mut s = format!("CREATE TABLE {} PARTITION OF {} {}", child_sql(d), parent_sql(d), bound_sql(ver, &d.bound));
    if let Some(p) = &d.partition_by {
        s.push_str(&partition_by_sql(p));
    }
    if let Some(ts) = &d.tablespace {
        s.push_str(&format!(" TABLESPACE {}", quote_ident(ts)));
    }
    s
}

/// PG 10–11: sınır değeri tek sabit olmalı (ham yerleşir); üst düzey virgül listeyi bozar.
fn check_pre12_literal(ctx: &BuildCtx, path: &str, sql: &str, e: &mut Vec<FieldError>) {
    if ctx.version_num >= 120000 || e.iter().any(|x| x.path == path) {
        return;
    }
    if split_top_level_commas(sql).is_none_or(|p| p.len() > 1) {
        e.push(err(path, "PG 12 öncesinde sınır değeri tek bir sabit olmalı."));
    }
}

/// Sınır doğrulaması: strateji uyumu, sayılar, MINVALUE/MAXVALUE kuralı, serbest SQL kapısı, sürüm kapıları.
fn validate_bound(ctx: &BuildCtx, d: &PartitionDef, b: &PartitionBound, e: &mut Vec<FieldError>) {
    let before = e.len();
    match (d.strategy, b) {
        (PartitionStrategy::Range, PartitionBound::Range { from, to }) => {
            for (side, vals) in [("from", from), ("to", to)] {
                if vals.len() != d.key_count as usize {
                    e.push(err(
                        &format!("bound.{side}"),
                        format!("{} listesi bölüm anahtarı sayısı kadar ({}) değer içermeli.", side.to_uppercase(), d.key_count),
                    ));
                }
                let mut keyword: Option<&BoundValue> = None;
                for (i, v) in vals.iter().enumerate() {
                    let path = format!("bound.{side}[{i}]");
                    match v {
                        BoundValue::Value { sql } => {
                            if keyword.is_some() {
                                e.push(err(&path, "MINVALUE/MAXVALUE'dan sonraki değerler de aynı anahtar sözcük olmalı."));
                            } else {
                                check_free_sql(&path, sql, e);
                                check_pre12_literal(ctx, &path, sql, e);
                            }
                        }
                        kw => match keyword {
                            Some(k) if k != kw => {
                                e.push(err(&path, "MINVALUE/MAXVALUE'dan sonraki değerler de aynı anahtar sözcük olmalı."));
                            }
                            Some(_) => {}
                            None => keyword = Some(kw),
                        },
                    }
                }
            }
        }
        (PartitionStrategy::List, PartitionBound::List { values }) => {
            if values.is_empty() {
                e.push(err("bound.values", "En az bir değer gerekli."));
            }
            for (i, v) in values.iter().enumerate() {
                let path = format!("bound.values[{i}]");
                check_free_sql(&path, v, e);
                check_pre12_literal(ctx, &path, v, e);
            }
        }
        (PartitionStrategy::Hash, PartitionBound::Hash { modulus, remainder }) => {
            if ctx.version_num < 110000 {
                e.push(version_err("bound", "Hash bölümleme", 11));
                return;
            }
            if *modulus < 1 {
                e.push(err("bound.modulus", "Modulus en az 1 olmalı."));
            } else if remainder >= modulus {
                e.push(err("bound.remainder", "Remainder, modulus'tan küçük olmalı."));
            }
        }
        (PartitionStrategy::Range | PartitionStrategy::List, PartitionBound::Default) => {
            if ctx.version_num < 110000 {
                e.push(version_err("bound", "DEFAULT bölüm", 11));
            }
        }
        (s, _) => {
            let msg = match s {
                PartitionStrategy::Range => "Üst tablo RANGE ile bölümlenmiş; sınır FROM … TO … olmalı.",
                PartitionStrategy::List => "Üst tablo LIST ile bölümlenmiş; sınır IN (…) ya da DEFAULT olmalı.",
                PartitionStrategy::Hash => "Üst tablo HASH ile bölümlenmiş; sınır WITH (MODULUS …, REMAINDER …) olmalı.",
            };
            e.push(err("bound", msg));
        }
    }
    // Nihai sınır metni (alan sınırlarında bölge/parantez kayması).
    if e.len() == before && !composed_is_clean(&bound_sql(ctx.version_num, b)) {
        e.push(err("bound", COMPOSED_BROKEN));
    }
}

/// Alt bölümleme (`PARTITION BY`) doğrulaması; kolon varlığını PG doğrular (üst tablonun kolonları burada bilinmez).
fn validate_partition_by(ctx: &BuildCtx, p: &PartitionByDef, e: &mut Vec<FieldError>) {
    let before = e.len();
    if p.strategy == PartitionStrategy::Hash && ctx.version_num < 110000 {
        e.push(version_err("partitionBy.strategy", "PARTITION BY HASH", 11));
    }
    if p.keys.is_empty() {
        e.push(err("partitionBy.keys", "En az bir bölüm anahtarı gerekli."));
    }
    for (j, k) in p.keys.iter().enumerate() {
        let path = format!("partitionBy.keys[{j}]");
        match k {
            PartitionKey::Column { name } => check_ident(&path, name, e),
            PartitionKey::Expression { sql } => check_free_sql(&path, sql, e),
        }
    }
    if e.len() == before && !composed_is_clean(&partition_by_sql(p)) {
        e.push(err("partitionBy.keys", COMPOSED_BROKEN));
    }
}

/// Oluşturma doğrulaması. `partition_by`/`tablespace`/`owner`/`comment` yalnız `create` modunda yazıldığından yalnız
/// orada doğrulanır; `attach`'te yok sayılır.
pub fn validate(ctx: &BuildCtx, d: &PartitionDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_old_schema_ident("parentSchema", &d.parent_schema, &mut e);
    check_old_ident("parentTable", &d.parent_table, &mut e);
    check_schema_ident("schema", &d.schema, &mut e);
    check_ident("name", &d.name, &mut e);
    if ctx.version_num < 100000 {
        e.push(version_err("object", "Bölümleme", 10));
        return e;
    }
    validate_bound(ctx, d, &d.bound, &mut e);
    if d.mode == PartitionMode::Create {
        if let Some(p) = &d.partition_by {
            validate_partition_by(ctx, p, &mut e);
        }
        if let Some(ts) = &d.tablespace {
            check_ident("tablespace", ts, &mut e);
        }
        if let Some(o) = &d.owner {
            check_ident("owner", o, &mut e);
        }
        if let Some(c) = &d.comment {
            check_text("comment", c, &mut e);
        }
    }
    e
}

/// `create`: CREATE, COMMENT, OWNER TO (OWNER en sonda); `attach`: tek ATTACH (Destructive, kilit notu).
pub fn create(ctx: &BuildCtx, d: &PartitionDef) -> Result<Plan, Vec<FieldError>> {
    let errs = validate(ctx, d);
    if !errs.is_empty() {
        return Err(errs);
    }
    let statements = match d.mode {
        PartitionMode::Attach => vec![attach_stmt(ctx, d, &d.bound)],
        PartitionMode::Create => {
            let mut v = vec![stmt(create_sql(ctx.version_num, d), Risk::Safe, None)];
            if let Some(c) = norm_comment(&d.comment) {
                v.push(stmt(format!("COMMENT ON TABLE {} IS {}", child_sql(d), quote_literal(c)), Risk::Safe, None));
            }
            if let Some(o) = &d.owner {
                v.push(stmt(format!("ALTER TABLE {} OWNER TO {}", child_sql(d), quote_ident(o)), Risk::Safe, None));
            }
            v
        }
    };
    Ok(Plan { statements, warnings: vec![] })
}

/// Yalnız sınır değişimi: kimlik/özellik değişimi alan hatası; sınır aynıysa boş plan; farklıysa aynı transaction'da
/// DETACH + ATTACH (ikisi de Destructive). Salt okunur eski tanım ve yarım kalmış ayırma `object` hatasıdır.
pub fn alter(ctx: &BuildCtx, old: &PartitionDef, new: &PartitionDef) -> Result<Plan, Vec<FieldError>> {
    let mut e = vec![];
    check_old_schema_ident("old.parentSchema", &old.parent_schema, &mut e);
    check_old_ident("old.parentTable", &old.parent_table, &mut e);
    check_old_schema_ident("old.schema", &old.schema, &mut e);
    check_old_ident("old.name", &old.name, &mut e);
    if !e.is_empty() {
        return Err(e);
    }
    for (path, changed) in [
        ("parentSchema", old.parent_schema != new.parent_schema),
        ("parentTable", old.parent_table != new.parent_table),
        ("schema", old.schema != new.schema),
        ("name", old.name != new.name),
    ] {
        if changed {
            e.push(err(path, IDENTITY_MSG));
        }
    }
    // Tablo özellikleri bölüm diyaloğunda yazılmaz: dolu değer ya da değişim hata (yükleyici dördünü de `None` yazar).
    for (path, set_or_changed) in [
        ("owner", new.owner.is_some() || old.owner != new.owner),
        ("comment", norm_comment(&new.comment).is_some() || norm_comment(&old.comment) != norm_comment(&new.comment)),
        ("tablespace", new.tablespace.is_some() || old.tablespace != new.tablespace),
        ("partitionBy", new.partition_by.is_some() || old.partition_by != new.partition_by),
    ] {
        if set_or_changed {
            e.push(err(path, PROPS_MSG));
        }
    }
    if !e.is_empty() {
        return Err(e);
    }
    // Sınır aynıysa hiçbir şey yazılmaz: salt okunur / yarım kalmış ayırmalı eski tanımda da boş plan (sabit nokta).
    if old.bound == new.bound {
        return Ok(Plan::default());
    }
    if !old.editable {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    if old.detach_pending {
        return Err(vec![err("object", PENDING_MSG)]);
    }
    // Strateji/anahtar sayısı yüklü (eski) tanımdan okunur: istemcinin gönderdiği değerler yetkili değildir.
    validate_bound(ctx, old, &new.bound, &mut e);
    if !e.is_empty() {
        return Err(e);
    }
    let statements = vec![
        stmt(detach_sql(new), Risk::Destructive, Some(REATTACH_NOTE)),
        attach_stmt(ctx, new, &new.bound),
    ];
    Ok(Plan { statements, warnings: vec![] })
}

/// Ayırma (`drop` karşılığı; tablo silinmez): normal → transaction içi; `concurrently` → `stmt_nt` (PG 14+);
/// `detach_pending` → yalnız `FINALIZE` (`stmt_nt`; `concurrently` yok sayılır). `cascade` reddi dağıtıcıdadır, kimlik de.
pub fn detach(ctx: &BuildCtx, d: &PartitionDef, concurrently: bool) -> Result<Plan, Vec<FieldError>> {
    if !d.editable {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    let base = detach_sql(d);
    let statement = if d.detach_pending {
        if ctx.version_num < 140000 {
            return Err(vec![version_err("object", "DETACH PARTITION … FINALIZE", 14)]);
        }
        stmt_nt(format!("{base} FINALIZE"), Risk::Destructive, Some(DETACH_NOTE))
    } else if concurrently {
        if ctx.version_num < 140000 {
            return Err(vec![version_err("concurrently", "DETACH PARTITION … CONCURRENTLY", 14)]);
        }
        stmt_nt(format!("{base} CONCURRENTLY"), Risk::Destructive, Some(DETACH_NOTE))
    } else {
        stmt(base, Risk::Destructive, Some(DETACH_NOTE))
    };
    Ok(Plan { statements: vec![statement], warnings: vec![] })
}

// ---------------------------------------------------------------------------------------------------------------------
// Sınır ayrıştırıcısı (Karar 7)
// ---------------------------------------------------------------------------------------------------------------------

/// Baştaki boşlukları atıp büyük/küçük harf duyarsız `kw` sözcüğünü soyar (ardından tanımlayıcı karakteri gelemez).
fn strip_kw<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let s = s.trim_start();
    let head = s.get(..kw.len())?;
    if !head.eq_ignore_ascii_case(kw) {
        return None;
    }
    let rest = &s[kw.len()..];
    match rest.chars().next() {
        Some(c) if c.is_alphanumeric() || c == '_' => None,
        _ => Some(rest),
    }
}

/// Parantez içi öğeler: üst düzey virgüllerden bölünür, kırpılır, boş öğe reddedilir.
fn items(inner: &str) -> Option<Vec<String>> {
    let parts = split_top_level_commas(inner)?;
    let parts: Vec<String> = parts.iter().map(|p| p.trim().to_string()).collect();
    parts.iter().all(|p| !p.is_empty()).then_some(parts)
}

fn bound_values(inner: &str) -> Option<Vec<BoundValue>> {
    Some(
        items(inner)?
            .into_iter()
            .map(|p| {
                if p.eq_ignore_ascii_case("MINVALUE") {
                    BoundValue::Minvalue
                } else if p.eq_ignore_ascii_case("MAXVALUE") {
                    BoundValue::Maxvalue
                } else {
                    BoundValue::Value { sql: p }
                }
            })
            .collect(),
    )
}

/// `modulus <n>` / `remainder <n>` öğesinin sayısı (yalnız ondalık rakamlar).
fn hash_number(item: &str, kw: &str) -> Option<u32> {
    let n = strip_kw(item, kw)?.trim();
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

/// `pg_get_expr(relpartbound, oid)` metnini sınıra çevirir; tanınmayan/bozuk metin → `None`.
pub(crate) fn parse_bound(text: &str) -> Option<PartitionBound> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("DEFAULT") {
        return Some(PartitionBound::Default);
    }
    let rest = strip_kw(t, "FOR")?;
    let rest = strip_kw(rest, "VALUES")?;
    if let Some(rest) = strip_kw(rest, "FROM") {
        let (from, rest) = paren_group(rest.trim_start())?;
        let rest = strip_kw(rest, "TO")?;
        let (to, rest) = paren_group(rest.trim_start())?;
        if !rest.trim().is_empty() {
            return None;
        }
        return Some(PartitionBound::Range { from: bound_values(from)?, to: bound_values(to)? });
    }
    if let Some(rest) = strip_kw(rest, "IN") {
        let (inner, rest) = paren_group(rest.trim_start())?;
        if !rest.trim().is_empty() {
            return None;
        }
        return Some(PartitionBound::List { values: items(inner)? });
    }
    let rest = strip_kw(rest, "WITH")?;
    let (inner, rest) = paren_group(rest.trim_start())?;
    if !rest.trim().is_empty() {
        return None;
    }
    let parts = items(inner)?;
    let [a, b] = parts.as_slice() else { return None };
    let (modulus, remainder) = match (hash_number(a, "modulus"), hash_number(b, "remainder")) {
        (Some(m), Some(r)) => (m, r),
        _ => (hash_number(b, "modulus")?, hash_number(a, "remainder")?),
    };
    Some(PartitionBound::Hash { modulus, remainder })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{alter as balter, assert_single_statements, create as bcreate, drop_with, render, DropOptions};
    use crate::model::ObjectDef;

    fn val(s: &str) -> BoundValue {
        BoundValue::Value { sql: s.into() }
    }
    fn range1(a: &str, b: &str) -> PartitionBound {
        PartitionBound::Range { from: vec![val(a)], to: vec![val(b)] }
    }
    fn pd() -> PartitionDef {
        PartitionDef {
            parent_schema: "public".into(),
            parent_table: "measurement".into(),
            strategy: PartitionStrategy::Range,
            key_count: 1,
            mode: PartitionMode::Create,
            schema: "public".into(),
            name: "m_2024".into(),
            bound: range1("'2024-01-01'", "'2025-01-01'"),
            partition_by: None,
            tablespace: None,
            owner: None,
            comment: None,
            detach_pending: false,
            editable: true,
            unsupported: vec![],
        }
    }
    fn list_pd(values: &[&str]) -> PartitionDef {
        PartitionDef {
            strategy: PartitionStrategy::List,
            bound: PartitionBound::List { values: values.iter().map(|v| v.to_string()).collect() },
            ..pd()
        }
    }
    fn hash_pd(m: u32, r: u32) -> PartitionDef {
        PartitionDef { strategy: PartitionStrategy::Hash, bound: PartitionBound::Hash { modulus: m, remainder: r }, ..pd() }
    }
    fn ctx(v: u32) -> BuildCtx {
        BuildCtx { version_num: v, ..Default::default() }
    }
    fn o(d: &PartitionDef) -> ObjectDef {
        ObjectDef::Partition(d.clone())
    }
    fn s(x: &str) -> Option<String> {
        Some(x.into())
    }
    fn paths(errs: Vec<FieldError>) -> Vec<String> {
        errs.into_iter().map(|e| e.path).collect()
    }
    fn create_ok(d: &PartitionDef, v: u32) -> Plan {
        let p = bcreate(&o(d), &ctx(v)).unwrap();
        assert_single_statements(&p);
        p
    }
    fn create_err(d: &PartitionDef, v: u32) -> Vec<String> {
        paths(bcreate(&o(d), &ctx(v)).unwrap_err())
    }
    fn alter_ok(old: &PartitionDef, new: &PartitionDef, v: u32) -> Plan {
        let p = balter(&o(old), &o(new), &ctx(v)).unwrap();
        assert_single_statements(&p);
        p
    }
    fn alter_err(old: &PartitionDef, new: &PartitionDef, v: u32) -> Vec<String> {
        paths(balter(&o(old), &o(new), &ctx(v)).unwrap_err())
    }
    fn detach_ok(d: &PartitionDef, concurrently: bool, v: u32) -> Plan {
        let p = drop_with(&o(d), DropOptions { concurrently, ..Default::default() }, &ctx(v)).unwrap();
        assert_single_statements(&p);
        p
    }
    fn detach_err(d: &PartitionDef, o_: DropOptions, v: u32) -> Vec<String> {
        paths(drop_with(&o(d), o_, &ctx(v)).unwrap_err())
    }

    #[test]
    fn create_variants() {
        let multi = PartitionDef {
            key_count: 2,
            bound: PartitionBound::Range {
                from: vec![val("1"), BoundValue::Minvalue],
                to: vec![val("10"), BoundValue::Maxvalue],
            },
            ..pd()
        };
        let cases: Vec<(&str, PartitionDef)> = vec![
            ("range tek anahtar", pd()),
            ("range çok anahtar MINVALUE/MAXVALUE", multi),
            ("list", list_pd(&["1", "2", "NULL"])),
            ("hash", hash_pd(4, 3)),
            ("default", PartitionDef { bound: PartitionBound::Default, ..pd() }),
            (
                "alt bölümleme",
                PartitionDef {
                    strategy: PartitionStrategy::List,
                    bound: PartitionBound::List { values: vec!["'a'".into()] },
                    partition_by: Some(PartitionByDef {
                        strategy: PartitionStrategy::List,
                        keys: vec![PartitionKey::Column { name: "region".into() }],
                    }),
                    ..pd()
                },
            ),
            (
                "tablespace + sahip + yorum",
                PartitionDef { tablespace: s("fast ts"), owner: s("Bob"), comment: s("it's"), ..pd() },
            ),
            ("farklı şemalar", PartitionDef { schema: "arch".into(), name: "Old Part".into(), ..pd() }),
        ];
        let mut out = String::new();
        for (label, d) in cases {
            let pl = create_ok(&d, 160000);
            assert!(pl.statements.iter().all(|x| x.risk == Risk::Safe && x.transactional), "{label}");
            out.push_str(&format!("# {label}\n{}\n", render(&pl)));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn create_statement_order_and_shape() {
        let d = PartitionDef { tablespace: s("ts"), owner: s("bob"), comment: s("c"), ..pd() };
        let p = create_ok(&d, 160000);
        let sqls: Vec<&str> = p.statements.iter().map(|x| x.sql.as_str()).collect();
        assert!(sqls[0].starts_with("CREATE TABLE public.m_2024 PARTITION OF public.measurement FOR VALUES FROM (("));
        assert!(sqls[0].ends_with(" TABLESPACE ts"));
        assert_eq!(sqls[1], "COMMENT ON TABLE public.m_2024 IS 'c'");
        assert_eq!(sqls[2], "ALTER TABLE public.m_2024 OWNER TO bob");
        // Boş yorum yorumsuz sayılır.
        assert_eq!(create_ok(&PartitionDef { comment: s(""), ..pd() }, 160000).statements.len(), 1);
    }

    #[test]
    fn create_version_gates_and_rule_violations() {
        // PG 10: default/hash sürüm hatası; range/list serbest.
        assert_eq!(create_err(&PartitionDef { bound: PartitionBound::Default, ..pd() }, 100000), ["bound"]);
        assert_eq!(create_err(&hash_pd(4, 0), 100000), ["bound"]);
        assert!(bcreate(&o(&pd()), &ctx(100000)).is_ok());
        assert!(bcreate(&o(&list_pd(&["1"])), &ctx(100000)).is_ok());
        assert_eq!(create_err(&pd(), 90600), ["object"]);
        // Alt bölümleme HASH PG 10'da kapalı.
        let sub = PartitionDef {
            partition_by: Some(PartitionByDef {
                strategy: PartitionStrategy::Hash,
                keys: vec![PartitionKey::Column { name: "id".into() }],
            }),
            ..pd()
        };
        assert_eq!(create_err(&sub, 100000), ["partitionBy.strategy"]);
        // Strateji uyuşmazlığı ve DEFAULT + HASH.
        assert_eq!(create_err(&PartitionDef { bound: PartitionBound::List { values: vec!["1".into()] }, ..pd() }, 160000), ["bound"]);
        assert_eq!(create_err(&PartitionDef { bound: range1("1", "2"), ..list_pd(&["1"]) }, 160000), ["bound"]);
        assert_eq!(create_err(&PartitionDef { bound: PartitionBound::Default, ..hash_pd(2, 0) }, 160000), ["bound"]);
        // key_count uyuşmazlığı.
        assert_eq!(create_err(&PartitionDef { key_count: 2, ..pd() }, 160000), ["bound.from", "bound.to"]);
        // MINVALUE/MAXVALUE sonrası değer ve karışık anahtar sözcük.
        let after_kw = PartitionDef {
            key_count: 2,
            bound: PartitionBound::Range {
                from: vec![BoundValue::Minvalue, val("1")],
                to: vec![BoundValue::Maxvalue, BoundValue::Minvalue],
            },
            ..pd()
        };
        assert_eq!(create_err(&after_kw, 160000), ["bound.from[1]", "bound.to[1]"]);
        // Boş list, boş değer, hash sayıları.
        assert_eq!(create_err(&list_pd(&[]), 160000), ["bound.values"]);
        assert_eq!(create_err(&list_pd(&["1", " "]), 160000), ["bound.values[1]"]);
        assert_eq!(create_err(&list_pd(&[""]), 160000), ["bound.values[0]"]);
        assert_eq!(create_err(&hash_pd(2, 2), 160000), ["bound.remainder"]);
        assert_eq!(create_err(&hash_pd(0, 0), 160000), ["bound.modulus"]);
        assert!(bcreate(&o(&hash_pd(2, 1)), &ctx(160000)).is_ok());
        // Ad/şema kuralları ve yalnız create modunda yazılan alanlar.
        assert_eq!(create_err(&PartitionDef { name: String::new(), ..pd() }, 160000), ["name"]);
        assert_eq!(create_err(&PartitionDef { schema: "pg_catalog".into(), ..pd() }, 160000), ["schema"]);
        assert_eq!(create_err(&PartitionDef { parent_table: String::new(), ..pd() }, 160000), ["parentTable"]);
        assert_eq!(create_err(&PartitionDef { owner: s("a\nb"), ..pd() }, 160000), ["owner"]);
        assert_eq!(create_err(&PartitionDef { tablespace: s(""), ..pd() }, 160000), ["tablespace"]);
        assert_eq!(create_err(&PartitionDef { comment: s("a\0b"), ..pd() }, 160000), ["comment"]);
        let bad_sub = PartitionDef {
            partition_by: Some(PartitionByDef {
                strategy: PartitionStrategy::List,
                keys: vec![PartitionKey::Column { name: String::new() }, PartitionKey::Expression { sql: "x; y".into() }],
            }),
            ..pd()
        };
        assert_eq!(create_err(&bad_sub, 160000), ["partitionBy.keys[0]", "partitionBy.keys[1]"]);
        let no_keys = PartitionDef { partition_by: Some(PartitionByDef { strategy: PartitionStrategy::List, keys: vec![] }), ..pd() };
        assert_eq!(create_err(&no_keys, 160000), ["partitionBy.keys"]);
    }

    #[test]
    fn attach_variants() {
        let attach = PartitionDef {
            mode: PartitionMode::Attach,
            // Create modunda yazılacak alanlar attach'te ifadeye girmez ve doğrulanmaz.
            owner: s("a\nb"),
            comment: s("c"),
            tablespace: s("ts"),
            partition_by: Some(PartitionByDef { strategy: PartitionStrategy::List, keys: vec![] }),
            ..pd()
        };
        let mut out = String::new();
        for v in [110000, 160000] {
            let pl = create_ok(&attach, v);
            assert_eq!(pl.statements.len(), 1);
            assert!(pl.statements[0].transactional && pl.statements[0].risk == Risk::Destructive);
            out.push_str(&format!("# PG {v}\n{}\n", render(&pl)));
        }
        let def = PartitionDef { mode: PartitionMode::Attach, bound: PartitionBound::Default, ..pd() };
        out.push_str(&format!("# default\n{}\n", render(&create_ok(&def, 160000))));
        insta::assert_snapshot!(out);
        let p11 = create_ok(&attach, 110000);
        let p16 = create_ok(&attach, 160000);
        assert_eq!(p11.statements[0].note.as_deref(), Some(ATTACH_NOTE_OLD));
        assert_eq!(p16.statements[0].note.as_deref(), Some(ATTACH_NOTE));
        // attach'te de sınır doğrulanır.
        assert_eq!(create_err(&PartitionDef { mode: PartitionMode::Attach, key_count: 3, ..pd() }, 160000), ["bound.from", "bound.to"]);
    }

    #[test]
    fn alter_cases() {
        let old = pd();
        let new = PartitionDef { bound: range1("'2025-01-01'", "'2026-01-01'"), ..pd() };
        // Aynı sınır → boş plan (mode/strategy/key_count farkları yok sayılır).
        assert!(alter_ok(&old, &old, 160000).statements.is_empty());
        let ignored = PartitionDef { mode: PartitionMode::Attach, key_count: 5, ..pd() };
        assert!(alter_ok(&old, &ignored, 160000).statements.is_empty());
        // Sınır değişimi: DETACH + ATTACH.
        let p = alter_ok(&old, &new, 160000);
        assert!(p.statements.iter().all(|x| x.risk == Risk::Destructive && x.transactional));
        assert_eq!(p.statements.len(), 2);
        let default_new = PartitionDef { bound: PartitionBound::Default, ..pd() };
        let mut out = format!("# range → range\n{}\n", render(&p));
        out.push_str(&format!("# range → default\n{}\n", render(&alter_ok(&old, &default_new, 160000))));
        insta::assert_snapshot!(out);
        // Kimlik değişimi.
        assert_eq!(alter_err(&old, &PartitionDef { schema: "x".into(), ..new.clone() }, 160000), ["schema"]);
        assert_eq!(alter_err(&old, &PartitionDef { name: "x".into(), ..new.clone() }, 160000), ["name"]);
        assert_eq!(alter_err(&old, &PartitionDef { parent_table: "x".into(), ..new.clone() }, 160000), ["parentTable"]);
        assert_eq!(alter_err(&old, &PartitionDef { parent_schema: "x".into(), ..new.clone() }, 160000), ["parentSchema"]);
        // Özellikler.
        assert_eq!(alter_err(&old, &PartitionDef { owner: s("bob"), ..new.clone() }, 160000), ["owner"]);
        assert_eq!(alter_err(&old, &PartitionDef { comment: s("c"), ..new.clone() }, 160000), ["comment"]);
        assert_eq!(alter_err(&old, &PartitionDef { tablespace: s("t"), ..old.clone() }, 160000), ["tablespace"]);
        let sub = Some(PartitionByDef { strategy: PartitionStrategy::List, keys: vec![] });
        assert_eq!(alter_err(&old, &PartitionDef { partition_by: sub, ..old.clone() }, 160000), ["partitionBy"]);
        // Yarım kalmış ayırma ve salt okunur eski tanım.
        let pending = PartitionDef { detach_pending: true, ..pd() };
        let e = balter(&o(&pending), &o(&new), &ctx(160000)).unwrap_err();
        assert_eq!((e[0].path.as_str(), e[0].message.as_str()), ("object", PENDING_MSG));
        let ro = PartitionDef { editable: false, unsupported: vec!["sınır ifadesi ayrıştırılamadı".into()], ..pd() };
        assert_eq!(alter_err(&ro, &new, 160000), ["object"]);
        assert!(alter_ok(&ro, &ro, 160000).statements.is_empty(), "değişiklik yoksa boş plan");
        // Yeni sınır doğrulanır; hatalı eski kimlik `old.` önekiyle.
        assert_eq!(alter_err(&old, &PartitionDef { bound: range1("1; DROP TABLE x", "2"), ..pd() }, 160000), ["bound.from[0]"]);
        assert_eq!(alter_err(&PartitionDef { name: String::new(), ..pd() }, &new, 160000), ["old.name"]);
        // Değişmeyen (yüklenmiş, düz `\` içeren) sınır yazılmadığından doğrulanmaz.
        let loaded = PartitionDef { bound: range1(r"'a\b'", "'z'"), ..pd() };
        assert!(alter_ok(&loaded, &loaded, 160000).statements.is_empty());
        assert_eq!(alter_err(&loaded, &PartitionDef { bound: range1(r"'a\b'", "'zz'"), ..pd() }, 160000), ["bound.from[0]"]);
    }

    #[test]
    fn detach_cases() {
        let d = pd();
        let normal = detach_ok(&d, false, 160000);
        let conc = detach_ok(&d, true, 160000);
        let pending = PartitionDef { detach_pending: true, ..pd() };
        // `detach_pending` iken `concurrently` yok sayılır.
        let fin = detach_ok(&pending, true, 160000);
        assert_eq!(render(&fin), render(&detach_ok(&pending, false, 160000)));
        let out = format!("# normal\n{}\n# concurrently\n{}\n# finalize\n{}\n", render(&normal), render(&conc), render(&fin));
        insta::assert_snapshot!(out);
        assert!(normal.statements[0].transactional && !conc.statements[0].transactional && !fin.statements[0].transactional);
        assert!(normal.statements.iter().chain(&conc.statements).chain(&fin.statements).all(|x| x.risk == Risk::Destructive));
        assert_eq!(normal.statements[0].note.as_deref(), Some(DETACH_NOTE));
        // Sürüm kapısı.
        assert_eq!(detach_err(&d, DropOptions { concurrently: true, ..Default::default() }, 130000), ["concurrently"]);
        assert_eq!(detach_err(&pending, DropOptions::default(), 130000), ["object"]);
        assert!(drop_with(&o(&d), DropOptions::default(), &ctx(100000)).is_ok());
        // cascade ve force reddi.
        assert_eq!(detach_err(&d, DropOptions { cascade: true, ..Default::default() }, 160000), ["cascade"]);
        assert_eq!(detach_err(&d, DropOptions { force: true, ..Default::default() }, 160000), ["force"]);
        // Salt okunur.
        assert_eq!(detach_err(&PartitionDef { editable: false, ..pd() }, DropOptions::default(), 160000), ["object"]);
        // Geçersiz UTF-8 tek gerekçeyse ayrılabilir; başka gerekçe de varsa kilitli.
        let bad = PartitionDef {
            editable: false,
            unsupported: vec![crate::load::INVALID_UTF8_REASON.into()],
            ..pd()
        };
        assert_single_statements(&drop_with(&o(&bad), DropOptions::default(), &ctx(160000)).unwrap());
        let two = PartitionDef { unsupported: vec![crate::load::INVALID_UTF8_REASON.into(), "x".into()], ..bad.clone() };
        assert_eq!(detach_err(&two, DropOptions::default(), 160000), ["object"]);
        // Create/alter geçersiz UTF-8'de kilitli.
        assert_eq!(create_err(&bad, 160000), ["object"]);
        // Diğer türlerde `concurrently` reddedilir (Task 1 ortak kural), bölümde kabul.
        assert!(drop_with(&o(&d), DropOptions { concurrently: true, ..Default::default() }, &ctx(160000)).is_ok());
    }

    #[test]
    fn identity_validation_in_drop() {
        let e = |d: PartitionDef| paths(drop_with(&o(&d), DropOptions::default(), &ctx(160000)).unwrap_err());
        assert_eq!(e(PartitionDef { parent_schema: "pg_catalog".into(), ..pd() }), ["parentSchema"]);
        assert_eq!(e(PartitionDef { parent_table: String::new(), ..pd() }), ["parentTable"]);
        assert_eq!(e(PartitionDef { schema: "information_schema".into(), ..pd() }), ["schema"]);
        assert_eq!(e(PartitionDef { name: "a\0b".into(), ..pd() }), ["name"]);
    }

    #[test]
    fn hostile_input() {
        // Adlar: `"` ve `;` tırnaklı yazılır (tek ifade); satır sonu yeni adda reddedilir.
        let d = PartitionDef { name: "a\";b".into(), ..pd() };
        let p = create_ok(&d, 160000);
        assert!(p.statements[0].sql.contains(r#"public."a"";b""#));
        assert_eq!(create_err(&PartitionDef { name: "a\nb".into(), ..pd() }, 160000), ["name"]);
        assert_eq!(create_err(&PartitionDef { schema: "a\nb".into(), ..pd() }, 160000), ["schema"]);
        // Yüklü üst ad satır sonu içerebilir (gevşek); yine tırnaklı tek ifadedir.
        create_ok(&PartitionDef { parent_table: "a\nb;--".into(), ..pd() }, 160000);
        // Değerler.
        for bad in ["'); DROP TABLE x; --", "'abc", r"'a\b'", "1); DROP TABLE x; (2", "x -- ", "x \0"] {
            assert_eq!(create_err(&PartitionDef { bound: range1(bad, "'z'"), ..pd() }, 160000), ["bound.from[0]"], "{bad}");
            assert_eq!(create_err(&PartitionDef { bound: range1("'a'", bad), ..pd() }, 160000), ["bound.to[0]"], "{bad}");
            assert_eq!(create_err(&list_pd(&["1", bad]), 160000), ["bound.values[1]"], "{bad}");
            assert_eq!(
                create_err(&PartitionDef { mode: PartitionMode::Attach, ..list_pd(&[bad]) }, 160000),
                ["bound.values[0]"],
                "{bad}"
            );
        }
        // E'…' kaçışı kabul edilir ve tek ifade kalır.
        let esc = create_ok(&list_pd(&[r"E'\x'", r"E'a\'b'"]), 160000);
        assert!(esc.statements[0].sql.contains(r"E'\x'"));
        // Alt bölümleme ifadesi ve sahip/tablespace adı.
        let sub = PartitionDef {
            partition_by: Some(PartitionByDef {
                strategy: PartitionStrategy::List,
                keys: vec![PartitionKey::Expression { sql: "x') ; DROP TABLE t; --".into() }],
            }),
            ..pd()
        };
        assert_eq!(create_err(&sub, 160000), ["partitionBy.keys[0]"]);
        let q = create_ok(&PartitionDef { owner: s("o\";x"), tablespace: s("t\"s"), comment: s("c'; DROP"), ..pd() }, 160000);
        assert_eq!(q.statements.len(), 3);
    }

    // --- parse_bound ---------------------------------------------------------------------------------------------

    fn vs(sql: &str) -> BoundValue {
        val(sql)
    }

    #[test]
    fn parse_bound_table() {
        let cases: Vec<(&str, PartitionBound)> = vec![
            ("FOR VALUES FROM ('2024-01-01') TO ('2025-01-01')", range1("'2024-01-01'", "'2025-01-01'")),
            (
                "FOR VALUES FROM (MINVALUE, MINVALUE) TO (10, MAXVALUE)",
                PartitionBound::Range {
                    from: vec![BoundValue::Minvalue, BoundValue::Minvalue],
                    to: vec![vs("10"), BoundValue::Maxvalue],
                },
            ),
            (
                "FOR VALUES IN ('a,b', 'it''s', NULL)",
                PartitionBound::List { values: vec!["'a,b'".into(), "'it''s'".into(), "NULL".into()] },
            ),
            ("FOR VALUES IN ('(x)')", PartitionBound::List { values: vec!["'(x)'".into()] }),
            (
                "FOR VALUES FROM ('2024-01-01 00:00:00+00') TO ('2024-02-01 00:00:00+00')",
                range1("'2024-01-01 00:00:00+00'", "'2024-02-01 00:00:00+00'"),
            ),
            ("FOR VALUES WITH (modulus 4, remainder 3)", PartitionBound::Hash { modulus: 4, remainder: 3 }),
            ("DEFAULT", PartitionBound::Default),
            // büyük/küçük harf ve fazladan boşluk
            ("  default  ", PartitionBound::Default),
            ("for  values   from ( minvalue )  to  ( Maxvalue )", PartitionBound::Range { from: vec![BoundValue::Minvalue], to: vec![BoundValue::Maxvalue] }),
            ("FOR VALUES\nIN (  1 ,2 )", PartitionBound::List { values: vec!["1".into(), "2".into()] }),
            ("FOR VALUES WITH (MODULUS 8, REMAINDER 0)", PartitionBound::Hash { modulus: 8, remainder: 0 }),
            ("FOR VALUES WITH (remainder 1, modulus 2)", PartitionBound::Hash { modulus: 2, remainder: 1 }),
            // MINVALUE bir dize/ifade içindeyse anahtar sözcük değildir.
            ("FOR VALUES FROM ('MINVALUE') TO (f(MAXVALUE))", range1("'MINVALUE'", "f(MAXVALUE)")),
            ("FOR VALUES IN (ARRAY[1,2])", PartitionBound::List { values: vec!["ARRAY[1,2]".into()] }),
            ("FOR VALUES IN (E'a\\'b,c')", PartitionBound::List { values: vec!["E'a\\'b,c'".into()] }),
            ("FOR VALUES IN ('x)', $$y,)$$)", PartitionBound::List { values: vec!["'x)'".into(), "$$y,)$$".into()] }),
        ];
        for (text, want) in cases {
            assert_eq!(parse_bound(text), Some(want), "{text}");
        }
        for bad in [
            "",
            "   ",
            "FOR VALUES FROM (1",
            "FOR VALUES FROM (1) TO",
            "FOR VALUES FROM (1) TO (2) extra",
            "FOR VALUES FROM (1)",
            "FOR VALUES FROM ('a) TO ('b')",
            "FOR VALUES IN ('a)",
            "FOR VALUES IN ()",
            "FOR VALUES IN (1,,2)",
            "FOR VALUES IN (1) x",
            "FOR VALUES FROM (1,) TO (2)",
            "FOR VALUES SOMETHING (1)",
            "FOR VALUESIN (1)",
            "FOR VALUES WITH (modulus x, remainder 1)",
            "FOR VALUES WITH (modulus 4)",
            "FOR VALUES WITH (modulus 4, remainder -1)",
            "FOR VALUES WITH (modulus 4, remainder 1, extra 2)",
            "FOR VALUES WITH (modulus 99999999999, remainder 1)",
            "DEFAULTS",
            "BOGUS",
        ] {
            assert_eq!(parse_bound(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn bound_sql_shapes() {
        assert_eq!(bound_sql(160000, &PartitionBound::Default), "DEFAULT");
        assert_eq!(bound_sql(160000, &PartitionBound::Hash { modulus: 4, remainder: 3 }), "FOR VALUES WITH (MODULUS 4, REMAINDER 3)");
        let r = PartitionBound::Range { from: vec![BoundValue::Minvalue], to: vec![val(" 1 ")] };
        assert_eq!(bound_sql(160000, &r), "FOR VALUES FROM (MINVALUE) TO (( 1 \n))");
        assert_eq!(bound_sql(110000, &r), "FOR VALUES FROM (MINVALUE) TO (1)");
        let l = PartitionBound::List { values: vec!["1".into(), "NULL".into()] };
        assert_eq!(bound_sql(160000, &l), "FOR VALUES IN ((1\n), (NULL\n))");
        assert_eq!(bound_sql(110000, &l), "FOR VALUES IN (1, NULL)");
        assert_eq!(bound_sql(119999, &l), bound_sql(110000, &l));
        assert_eq!(bound_sql(120000, &l), bound_sql(160000, &l));
    }

    /// Sabit nokta: anahtar sözcük/sayı içeren sınırlar `bound_sql` → `parse_bound` ile aynen döner. Serbest SQL değerler
    /// `wrap()` ile `(<sql>\n)` yazıldığından geri okunan değer `wrap`'lı metindir (davranış belgelenir; gerçek sabit
    /// nokta yükleyici metniyle Task 3'te sınanır).
    #[test]
    fn parse_bound_round_trip_and_wrap_effect() {
        for b in [
            PartitionBound::Default,
            PartitionBound::Hash { modulus: 4, remainder: 3 },
            PartitionBound::Range {
                from: vec![BoundValue::Minvalue, BoundValue::Minvalue],
                to: vec![BoundValue::Maxvalue, BoundValue::Maxvalue],
            },
        ] {
            assert_eq!(parse_bound(&bound_sql(160000, &b)), Some(b.clone()), "{b:?}");
        }
        // `wrap` etkisi: değerler parantezli + satır sonlu döner (sunucu çıktısında olmayan biçim).
        let b = range1("1", "2");
        assert_eq!(parse_bound(&bound_sql(160000, &b)), Some(range1("(1\n)", "(2\n)")));
        // PG 10–11: ham yazım → düz değerlerde tam sabit nokta.
        assert_eq!(parse_bound(&bound_sql(110000, &b)), Some(b.clone()));
        let l = PartitionBound::List { values: vec!["'a,b'".into(), "NULL".into()] };
        assert_eq!(parse_bound(&bound_sql(160000, &l)), Some(PartitionBound::List { values: vec!["('a,b'\n)".into(), "(NULL\n)".into()] }));
        assert_eq!(parse_bound(&bound_sql(110000, &l)), Some(l.clone()));
        // Yükleyici biçimi (parantezsiz) bir kez yazılıp okununca değer sayısı ve sıra korunur.
        let loaded = parse_bound("FOR VALUES IN ('a,b', NULL)").unwrap();
        let PartitionBound::List { values } = &loaded else { panic!() };
        assert_eq!(values.len(), 2);
        assert_eq!(parse_bound(&bound_sql(160000, &loaded)).map(|p| match p {
            PartitionBound::List { values } => values.len(),
            _ => 0,
        }), Some(2));
    }

    #[test]
    fn pre12_bounds_are_raw_and_checked() {
        let range = PartitionDef { bound: range1(" '2024-01-01' ", "'2025-01-01'"), ..pd() };
        let list = list_pd(&["1", "NULL"]);
        let mut out = String::new();
        for v in [110000, 160000] {
            out.push_str(&format!("# PG {v} range\n{}\n# PG {v} list\n{}\n", render(&create_ok(&range, v)), render(&create_ok(&list, v))));
        }
        insta::assert_snapshot!(out);
        // PG 11: ham sabit; PG 16: wrap.
        assert_eq!(
            create_ok(&list, 110000).statements[0].sql,
            "CREATE TABLE public.m_2024 PARTITION OF public.measurement FOR VALUES IN (1, NULL)"
        );
        // Üst düzey virgül listeyi bozar (PG 12 öncesi); PG 12+ ifade olarak kabul.
        assert_eq!(create_err(&list_pd(&["1, 2"]), 110000), ["bound.values[0]"]);
        assert_eq!(create_err(&PartitionDef { bound: range1("1, 2", "3"), ..pd() }, 110000), ["bound.from[0]"]);
        assert!(bcreate(&o(&list_pd(&["1, 2"])), &ctx(160000)).is_ok());
        // Virgül tırnak/parantez içindeyse tek sabittir.
        assert!(bcreate(&o(&list_pd(&["'a,b'", "ARRAY[1,2]"])), &ctx(110000)).is_ok());
        // Satır yorumuyla biten değer her sürümde tek hata verir; birleşik ifade temiz kalır.
        for v in [110000, 160000] {
            for bad in ["1 -- ", "'a' --"] {
                assert_eq!(create_err(&list_pd(&[bad]), v), ["bound.values[0]"], "{bad} PG {v}");
                assert_eq!(create_err(&PartitionDef { bound: range1(bad, "'z'"), ..pd() }, v), ["bound.from[0]"], "{bad} PG {v}");
            }
            let ok = create_ok(&list_pd(&["1 /* c */", "'x'"]), v);
            assert!(composed_is_clean(&ok.statements[0].sql));
        }
    }

    #[test]
    fn alter_uses_loaded_strategy_and_notes() {
        // Strateji/anahtar sayısı yüklü (eski) tanımdan okunur; istemci değerleri yetkili değil.
        let old = pd();
        let new = PartitionDef { strategy: PartitionStrategy::List, key_count: 2, bound: range1("1", "2"), ..pd() };
        assert!(alter_ok(&old, &new, 160000).statements.len() == 2);
        let bad = PartitionDef { bound: PartitionBound::List { values: vec!["1".into()] }, ..pd() };
        assert_eq!(alter_err(&old, &bad, 160000), ["bound"]);
        // Notlar: DETACH yeniden bağlama notu, ATTACH sürüme göre kilit notu.
        let new = PartitionDef { bound: range1("1", "2"), ..pd() };
        let p16 = alter_ok(&old, &new, 160000);
        assert_eq!(p16.statements[0].note.as_deref(), Some(REATTACH_NOTE));
        assert_eq!(p16.statements[1].note.as_deref(), Some(ATTACH_NOTE));
        let p11 = alter_ok(&old, &new, 110000);
        assert_eq!(p11.statements[1].note.as_deref(), Some(ATTACH_NOTE_OLD));
        // Sınır aynıysa salt okunur / yarım kalmış eski tanımda da boş plan.
        let pending = PartitionDef { detach_pending: true, editable: false, ..pd() };
        assert!(alter_ok(&pending, &pending, 160000).statements.is_empty());
    }

    #[test]
    fn multibyte_and_crlf_values() {
        assert_eq!(
            split_top_level_commas("'ç,ö', 'ş'"),
            Some(vec!["'ç,ö'".to_string(), " 'ş'".to_string()])
        );
        assert_eq!(
            parse_bound("FOR VALUES IN ('ç,ö', 'ş')"),
            Some(PartitionBound::List { values: vec!["'ç,ö'".into(), "'ş'".into()] })
        );
        assert_eq!(
            parse_bound("FOR VALUES IN ('a\r\nb,c', 'd')"),
            Some(PartitionBound::List { values: vec!["'a\r\nb,c'".into(), "'d'".into()] })
        );
        assert_eq!(
            parse_bound("FOR VALUES FROM ('ğ\r\n1') TO ('ü\n2')"),
            Some(range1("'ğ\r\n1'", "'ü\n2'"))
        );
        // Çok baytlı değerler üretici yoluyla da geçer (PG 11 ham, PG 16 wrap).
        for v in [110000, 160000] {
            create_ok(&list_pd(&["'ç,ö'", "'a\r\nb'"]), v);
        }
        // Anahtar sözcük karşılaştırması çok baytlı metinde panik atmaz.
        assert_eq!(parse_bound("FOR VALUES İN (1)"), None);
        assert_eq!(parse_bound("ÇÇ"), None);
    }
}
