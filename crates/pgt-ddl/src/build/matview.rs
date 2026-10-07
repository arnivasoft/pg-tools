//! Materialized View üreticisi: CREATE, ALTER (rename, şema, tablespace, fillfactor, sahip, yorum, REFRESH; sorgu
//! değişince yeniden kurma), DROP.
//!
//! Sorgu tek serbest SQL alanıdır: `check_free_sql` + `wrap` + nihai ifadede `composed_is_clean`; yalnızca oluşturmada
//! ve sorgu değiştiğinde doğrulanır (yüklü `pg_get_viewdef` metni kapıya takılsa bile sorguya dokunmayan düzenlemeler
//! yapılabilsin). PG'de MV tanımı değiştirilemez: sorgu değişimi `DROP` + `CREATE` ile yeniden kurmadır (Karar 2:
//! veri, indeksler ve yetkiler kaybolur → DataLoss). `REFRESH` (her biçim) Destructive'dir (Karar 10).
//!
//! `REFRESH … CONCURRENTLY` transaction bloğunda çalışabilir (`CREATE INDEX CONCURRENTLY`'den farklı olarak
//! `PreventInTransactionBlock` çağırmaz); bu yüzden tüm ifadeler transactional'dır.

use super::freesql::{check_free_sql, composed_is_clean, wrap};
use super::table_alter::TABLESPACE_CLEAR_MSG;
use super::view::CASCADE_WARNING;
use super::{BuildCtx, check_ident, check_old_ident, check_old_schema_ident, check_schema_ident, check_text, err, stmt};
use crate::model::{FieldError, MatViewDef, MatViewRefresh};
use crate::plan::{Plan, Risk, Statement};
use crate::quote::{qualified, quote_ident, quote_literal};

pub const WITH_DATA_NOTE: &str = "Sorgu çalıştırılır; uzun sürebilir";
pub const TABLESPACE_NOTE: &str = "Veri yeniden yazılır; ACCESS EXCLUSIVE kilidi alır";
pub const REBUILD_DROP_NOTE: &str = "Veri, indeksler ve yetkiler silinir; bağımlı görünüm varsa başarısız olur";
pub const REFRESH_NOTE: &str = "ACCESS EXCLUSIVE kilit: yenileme süresince okunamaz";
pub const REFRESH_CONCURRENTLY_NOTE: &str = "Benzersiz (UNIQUE) bir indeks gerekir; okumalar sürer";
pub const REFRESH_NO_DATA_NOTE: &str = "Veri boşaltılır; sorgulanamaz hale gelir";
pub const GRANT_WARNING: &str = "Yetkiler (GRANT) yeniden verilmelidir";
pub const REFRESH_IGNORED_WARNING: &str = "Tanım değişti, görünüm yeniden kuruluyor; yenileme seçimi yok sayıldı";
const REFRESH_NO_DATA_CONCURRENTLY: &str = "Veri yokken CONCURRENTLY kullanılamaz.";
const REFRESH_WHILE_EMPTYING: &str = "Veriyi boşaltırken yenileme seçilemez.";
const REFRESH_NEEDS_DATA: &str = "Önce 'Veriyle doldur'u seçin.";
const REFRESH_ON_CREATE: &str = "Yenileme yalnızca var olan görünümün düzenlenmesinde seçilebilir.";
const FILLFACTOR_RANGE: &str = "Doldurma oranı 10 ile 100 arasında olmalı.";
const COMPOSED_BROKEN: &str = "Sorgu, yerleştirildiği ifadeyi bozuyor";

/// `CREATE MATERIALIZED VIEW <q> [WITH (fillfactor = n)] [TABLESPACE t] AS (<sorgu>\n) WITH [NO] DATA`.
fn create_sql(d: &MatViewDef) -> String {
    let mut sql = format!("CREATE MATERIALIZED VIEW {}", qualified(&d.schema, &d.name));
    if let Some(ff) = d.fillfactor {
        sql.push_str(&format!(" WITH (fillfactor = {ff})"));
    }
    if let Some(ts) = &d.tablespace {
        sql.push_str(&format!(" TABLESPACE {}", quote_ident(ts)));
    }
    sql.push_str(&format!(" AS {}", wrap(&d.query)));
    sql.push_str(if d.with_data { " WITH DATA" } else { " WITH NO DATA" });
    sql
}

fn create_stmt(d: &MatViewDef) -> Statement {
    stmt(create_sql(d), Risk::Safe, d.with_data.then_some(WITH_DATA_NOTE))
}

fn owner_stmt(q: &str, owner: &str) -> Statement {
    stmt(format!("ALTER MATERIALIZED VIEW {q} OWNER TO {}", quote_ident(owner)), Risk::Safe, None)
}

fn comment_stmt(q: &str, comment: &Option<String>) -> Statement {
    let value = comment.as_deref().map_or_else(|| "NULL".to_string(), quote_literal);
    stmt(format!("COMMENT ON MATERIALIZED VIEW {q} IS {value}"), Risk::Safe, None)
}

fn check_fillfactor(d: &MatViewDef, e: &mut Vec<FieldError>) {
    if let Some(ff) = d.fillfactor
        && !(10..=100).contains(&ff)
    {
        e.push(err("fillfactor", FILLFACTOR_RANGE));
    }
}

/// Sorgu + nihai ifade taraması (yalnızca adlar ve sorgu geçerliyse; geçersiz ad zaten raporlandı).
fn check_query(d: &MatViewDef, e: &mut Vec<FieldError>) {
    let before = e.len();
    check_free_sql("query", &d.query, e);
    if e.len() == before && e.is_empty() && !composed_is_clean(&create_sql(d)) {
        e.push(err("query", COMPOSED_BROKEN));
    }
}

/// Oluşturma doğrulaması (tüm alanlar).
pub fn validate(d: &MatViewDef, _ctx: &BuildCtx) -> Vec<FieldError> {
    let mut e = vec![];
    check_schema_ident("schema", &d.schema, &mut e);
    check_ident("name", &d.name, &mut e);
    if let Some(o) = &d.owner {
        check_ident("owner", o, &mut e);
    }
    if let Some(ts) = &d.tablespace {
        // `pg_default` geçerli bir addır.
        check_ident("tablespace", ts, &mut e);
    }
    check_fillfactor(d, &mut e);
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    if d.refresh.is_some() {
        e.push(err("refresh", REFRESH_ON_CREATE));
    }
    check_query(d, &mut e);
    e
}

/// Doğrulanmış tanım varsayılır (`build::create` önce `validate` çağırır).
pub fn create(d: &MatViewDef) -> Plan {
    let q = qualified(&d.schema, &d.name);
    let mut statements = vec![create_stmt(d)];
    if let Some(o) = &d.owner {
        statements.push(owner_stmt(&q, o));
    }
    if d.comment.is_some() {
        statements.push(comment_stmt(&q, &d.comment));
    }
    Plan { statements, warnings: vec![] }
}

/// REFRESH kuralları tablosu (yeniden kurma dışı): `(ad öncesi, ad sonrası, not)` ya da alan hatası; `None` = REFRESH yok.
type RefreshParts = (&'static str, &'static str, &'static str);
fn refresh_rule(old: &MatViewDef, new: &MatViewDef) -> Result<Option<RefreshParts>, &'static str> {
    use MatViewRefresh::{Concurrently, Normal};
    match (old.with_data, new.with_data, new.refresh) {
        (false, true, None | Some(Normal)) => Ok(Some(("", "", REFRESH_NOTE))),
        (false, true, Some(Concurrently)) => Err(REFRESH_NO_DATA_CONCURRENTLY),
        (true, false, None) => Ok(Some(("", " WITH NO DATA", REFRESH_NO_DATA_NOTE))),
        (true, false, Some(_)) => Err(REFRESH_WHILE_EMPTYING),
        (true, true, Some(Normal)) => Ok(Some(("", "", REFRESH_NOTE))),
        (true, true, Some(Concurrently)) => Ok(Some(("CONCURRENTLY ", "", REFRESH_CONCURRENTLY_NOTE))),
        (true, true, None) | (false, false, None) => Ok(None),
        (false, false, Some(_)) => Err(REFRESH_NEEDS_DATA),
    }
}

/// Alter doğrulaması: eskinin yalnızca kimliği (gevşek), yeninin yalnızca değişen ya da ifadeye yazılan alanları.
fn validate_alter(old: &MatViewDef, new: &MatViewDef, rebuild: bool) -> Vec<FieldError> {
    let mut e = vec![];
    // Eski (yüklü) kimlik gevşek: başka araçla oluşturulmuş yanıltıcı ad yeniden adlandırılabilsin/silinebilsin;
    // yalnızca `quote_ident` ile yerleşir. Sistem şeması yine reddedilir. YENİ adlar katı kurala tabidir.
    check_old_schema_ident("old.schema", &old.schema, &mut e);
    check_old_ident("old.name", &old.name, &mut e);
    if !e.is_empty() {
        return e;
    }
    // Yeniden kurmada CREATE yeni adı/şemayı doğrudan yazar; adlar değişmese de yeni (katı) kurala tabidir.
    if rebuild || new.schema != old.schema {
        check_schema_ident("schema", &new.schema, &mut e);
    }
    if rebuild || new.name != old.name {
        check_ident("name", &new.name, &mut e);
    }
    let owner = if rebuild {
        new.owner.as_ref().or(old.owner.as_ref())
    } else {
        new.owner.as_ref().filter(|_| new.owner != old.owner)
    };
    if let Some(o) = owner {
        check_ident("owner", o, &mut e);
    }
    if rebuild || new.tablespace != old.tablespace {
        match &new.tablespace {
            Some(ts) => check_ident("tablespace", ts, &mut e),
            // ALTER ile tablespace "kaldırılamaz"; yeniden kurmada CREATE varsayılana koyar (sorun değil).
            None if !rebuild => e.push(err("tablespace", TABLESPACE_CLEAR_MSG)),
            None => {}
        }
    }
    if rebuild || new.fillfactor != old.fillfactor {
        check_fillfactor(new, &mut e);
    }
    if let Some(c) = &new.comment
        && (rebuild || new.comment != old.comment)
    {
        check_text("comment", c, &mut e);
    }
    if rebuild {
        check_query(new, &mut e);
    } else if let Err(m) = refresh_rule(old, new) {
        e.push(err("refresh", m));
    }
    e
}

/// DEĞİŞMEZ (yükleyici, Task 6): yüklenen MV'de `owner` hiçbir zaman `None` değildir; `refresh` her zaman `None`'dır.
///
/// Sorgu değiştiyse yeniden kurma: DROP eski (DataLoss) + CREATE yeni (yeni ad/şema/tablespace/fillfactor/withData) +
/// sahip (`new ?? old`, her zaman) + yorum (doluysa); `refresh` yok sayılır (uyarı). Aksi halde sıra (sonrakiler YENİ
/// adı kullanır): rename, set schema, set tablespace, fillfactor SET/RESET, owner, comment, REFRESH (kurallar tablosu).
pub fn alter(old: &MatViewDef, new: &MatViewDef, _ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    let rebuild = old.query != new.query;
    let errs = validate_alter(old, new, rebuild);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut statements = vec![];
    let mut warnings = vec![];
    let q = qualified(&new.schema, &new.name);

    if rebuild {
        statements.push(stmt(
            format!("DROP MATERIALIZED VIEW {}", qualified(&old.schema, &old.name)),
            Risk::DataLoss,
            Some(REBUILD_DROP_NOTE),
        ));
        statements.push(create_stmt(new));
        if let Some(o) = new.owner.as_ref().or(old.owner.as_ref()) {
            statements.push(owner_stmt(&q, o));
        }
        if new.comment.is_some() {
            statements.push(comment_stmt(&q, &new.comment));
        }
        let (indexes, others): (Vec<&String>, Vec<&String>) =
            old.unsupported.iter().partition(|u| u.starts_with("INDEX "));
        if !indexes.is_empty() {
            let names: Vec<&str> = indexes.iter().map(|u| &u["INDEX ".len()..]).collect();
            warnings.push(format!("İndeksler yeniden oluşturulmaz: {}", names.join(", ")));
        }
        warnings.push(GRANT_WARNING.to_string());
        if !others.is_empty() {
            let others: Vec<&str> = others.iter().map(|s| s.as_str()).collect();
            warnings.push(format!("Kaybolacaklar: {}", others.join(", ")));
        }
        if new.refresh.is_some() {
            warnings.push(REFRESH_IGNORED_WARNING.to_string());
        }
        return Ok(Plan { statements, warnings });
    }

    if old.name != new.name {
        statements.push(stmt(
            format!(
                "ALTER MATERIALIZED VIEW {} RENAME TO {}",
                qualified(&old.schema, &old.name),
                quote_ident(&new.name)
            ),
            Risk::Safe,
            None,
        ));
    }
    if old.schema != new.schema {
        statements.push(stmt(
            format!(
                "ALTER MATERIALIZED VIEW {} SET SCHEMA {}",
                qualified(&old.schema, &new.name),
                quote_ident(&new.schema)
            ),
            Risk::Safe,
            None,
        ));
    }
    if let Some(ts) = &new.tablespace
        && new.tablespace != old.tablespace
    {
        statements.push(stmt(
            format!("ALTER MATERIALIZED VIEW {q} SET TABLESPACE {}", quote_ident(ts)),
            Risk::Safe,
            Some(TABLESPACE_NOTE),
        ));
    }
    if old.fillfactor != new.fillfactor {
        let sql = match new.fillfactor {
            Some(n) => format!("ALTER MATERIALIZED VIEW {q} SET (fillfactor = {n})"),
            None => format!("ALTER MATERIALIZED VIEW {q} RESET (fillfactor)"),
        };
        statements.push(stmt(sql, Risk::Safe, None));
    }
    if let Some(o) = &new.owner
        && new.owner != old.owner
    {
        statements.push(owner_stmt(&q, o));
    }
    if old.comment != new.comment {
        statements.push(comment_stmt(&q, &new.comment));
    }
    // Doğrulama hata yollarını eledi.
    if let Ok(Some((prefix, suffix, note))) = refresh_rule(old, new) {
        statements.push(stmt(format!("REFRESH MATERIALIZED VIEW {prefix}{q}{suffix}"), Risk::Destructive, Some(note)));
    }
    Ok(Plan { statements, warnings })
}

pub fn drop(d: &MatViewDef, cascade: bool) -> Plan {
    let q = qualified(&d.schema, &d.name);
    if cascade {
        Plan {
            statements: vec![stmt(
                format!("DROP MATERIALIZED VIEW {q} CASCADE"),
                Risk::DataLoss,
                Some(CASCADE_WARNING),
            )],
            warnings: vec![CASCADE_WARNING.into()],
        }
    } else {
        Plan {
            statements: vec![stmt(format!("DROP MATERIALIZED VIEW {q}"), Risk::Destructive, None)],
            warnings: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{
        alter as balter, assert_single_statements, create as bcreate, drop as bdrop, render, validate_identity,
    };
    use crate::model::{ObjectDef, ViewColumn};

    fn m() -> MatViewDef {
        MatViewDef {
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
        }
    }
    fn o(d: MatViewDef) -> ObjectDef {
        ObjectDef::MaterializedView(d)
    }
    fn ctx() -> BuildCtx {
        BuildCtx { version_num: 160000, ..Default::default() }
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn paths(errs: Vec<FieldError>) -> Vec<String> {
        errs.into_iter().map(|e| e.path).collect()
    }
    fn alter_ok(old: &MatViewDef, new: &MatViewDef) -> Plan {
        let p = balter(&o(old.clone()), &o(new.clone()), &ctx()).unwrap();
        assert_single_statements(&p);
        assert!(p.statements.iter().all(|s| s.transactional && !s.redacted));
        p
    }
    fn alter_err(old: &MatViewDef, new: &MatViewDef) -> Vec<FieldError> {
        balter(&o(old.clone()), &o(new.clone()), &ctx()).unwrap_err()
    }

    #[test]
    fn create_variants() {
        let cases = [
            (m(), vec!["CREATE MATERIALIZED VIEW public.mv AS (SELECT 1\n) WITH DATA"], Some(WITH_DATA_NOTE)),
            (
                MatViewDef { with_data: false, ..m() },
                vec!["CREATE MATERIALIZED VIEW public.mv AS (SELECT 1\n) WITH NO DATA"],
                None,
            ),
            (
                MatViewDef { fillfactor: Some(70), ..m() },
                vec!["CREATE MATERIALIZED VIEW public.mv WITH (fillfactor = 70) AS (SELECT 1\n) WITH DATA"],
                Some(WITH_DATA_NOTE),
            ),
            (
                MatViewDef { tablespace: Some("Fast Disk".into()), ..m() },
                vec!["CREATE MATERIALIZED VIEW public.mv TABLESPACE \"Fast Disk\" AS (SELECT 1\n) WITH DATA"],
                Some(WITH_DATA_NOTE),
            ),
            (
                MatViewDef {
                    schema: "App".into(),
                    name: "My MV".into(),
                    owner: Some("Bob".into()),
                    comment: Some("it's \\ ok".into()),
                    fillfactor: Some(100),
                    tablespace: Some("pg_default".into()),
                    with_data: false,
                    ..m()
                },
                vec![
                    "CREATE MATERIALIZED VIEW \"App\".\"My MV\" WITH (fillfactor = 100) TABLESPACE pg_default AS (SELECT 1\n) WITH NO DATA",
                    "ALTER MATERIALIZED VIEW \"App\".\"My MV\" OWNER TO \"Bob\"",
                    "COMMENT ON MATERIALIZED VIEW \"App\".\"My MV\" IS E'it''s \\\\ ok'",
                ],
                None,
            ),
        ];
        let mut out = String::new();
        for (d, want, note) in cases {
            let p = bcreate(&o(d), &ctx()).unwrap();
            assert_single_statements(&p);
            assert_eq!(sqls(&p), want);
            assert_eq!(p.statements[0].note.as_deref(), note);
            assert!(p.statements[1..].iter().all(|s| s.note.is_none()));
            assert!(p.statements.iter().all(|s| s.risk == Risk::Safe && s.transactional && !s.redacted));
            assert!(p.warnings.is_empty());
            out.push_str(&render(&p));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn create_rejects_refresh_and_bad_fields() {
        for r in [MatViewRefresh::Normal, MatViewRefresh::Concurrently] {
            let e = bcreate(&o(MatViewDef { refresh: Some(r), ..m() }), &ctx()).unwrap_err();
            assert_eq!(paths(e.clone()), ["refresh"]);
            assert_eq!(e[0].message, REFRESH_ON_CREATE);
        }
        for ff in [0, 9, 101, 1000] {
            let e = validate(&MatViewDef { fillfactor: Some(ff), ..m() }, &ctx());
            assert_eq!(paths(e.clone()), ["fillfactor"], "{ff}");
            assert_eq!(e[0].message, FILLFACTOR_RANGE);
        }
        for ff in [10, 50, 100] {
            assert!(validate(&MatViewDef { fillfactor: Some(ff), ..m() }, &ctx()).is_empty(), "{ff}");
        }
        assert_eq!(paths(validate(&MatViewDef { tablespace: Some("a\nb".into()), ..m() }, &ctx())), ["tablespace"]);
        assert_eq!(paths(validate(&MatViewDef { tablespace: Some(String::new()), ..m() }, &ctx())), ["tablespace"]);
    }

    #[test]
    fn alter_single_changes() {
        let base = MatViewDef {
            owner: Some("a".into()),
            comment: Some("c".into()),
            tablespace: Some("fast".into()),
            fillfactor: Some(70),
            ..m()
        };
        let cases: Vec<(MatViewDef, Vec<&str>, Option<&str>)> = vec![
            (MatViewDef { name: "w".into(), ..base.clone() }, vec!["ALTER MATERIALIZED VIEW public.mv RENAME TO w"], None),
            (
                MatViewDef { schema: "app".into(), ..base.clone() },
                vec!["ALTER MATERIALIZED VIEW public.mv SET SCHEMA app"],
                None,
            ),
            (
                MatViewDef { tablespace: Some("slow".into()), ..base.clone() },
                vec!["ALTER MATERIALIZED VIEW public.mv SET TABLESPACE slow"],
                Some(TABLESPACE_NOTE),
            ),
            (
                MatViewDef { tablespace: Some("pg_default".into()), ..base.clone() },
                vec!["ALTER MATERIALIZED VIEW public.mv SET TABLESPACE pg_default"],
                Some(TABLESPACE_NOTE),
            ),
            (
                MatViewDef { fillfactor: Some(90), ..base.clone() },
                vec!["ALTER MATERIALIZED VIEW public.mv SET (fillfactor = 90)"],
                None,
            ),
            (
                MatViewDef { fillfactor: None, ..base.clone() },
                vec!["ALTER MATERIALIZED VIEW public.mv RESET (fillfactor)"],
                None,
            ),
            (
                MatViewDef { owner: Some("b".into()), ..base.clone() },
                vec!["ALTER MATERIALIZED VIEW public.mv OWNER TO b"],
                None,
            ),
            // Sahip `None`'a çekilemez: yok sayılır.
            (MatViewDef { owner: None, ..base.clone() }, vec![], None),
            (
                MatViewDef { comment: None, ..base.clone() },
                vec!["COMMENT ON MATERIALIZED VIEW public.mv IS NULL"],
                None,
            ),
            (
                MatViewDef { comment: Some("d".into()), ..base.clone() },
                vec!["COMMENT ON MATERIALIZED VIEW public.mv IS 'd'"],
                None,
            ),
            // Yalnızca yükleme alanları üretici tarafından okunmaz.
            (
                MatViewDef {
                    columns: vec![ViewColumn { name: "x".into(), data_type: "int".into() }],
                    unsupported: vec!["INDEX i".into()],
                    ..base.clone()
                },
                vec![],
                None,
            ),
        ];
        let mut out = String::new();
        for (new, want, note) in cases {
            let p = alter_ok(&base, &new);
            assert_eq!(sqls(&p), want, "{new:?}");
            assert!(p.statements.iter().all(|s| s.risk == Risk::Safe));
            if let Some(s) = p.statements.first() {
                assert_eq!(s.note.as_deref(), note, "{new:?}");
            }
            assert!(p.warnings.is_empty());
            out.push_str(&render(&p));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn tablespace_cannot_be_cleared() {
        let old = MatViewDef { tablespace: Some("fast".into()), ..m() };
        let e = alter_err(&old, &m());
        assert_eq!(paths(e.clone()), ["tablespace"]);
        assert_eq!(e[0].message, TABLESPACE_CLEAR_MSG);
        assert!(e[0].message.contains("pg_default"));
        // Yeni tablespace adı da katı kurala tabidir.
        assert_eq!(paths(alter_err(&old, &MatViewDef { tablespace: Some("a\0".into()), ..m() })), ["tablespace"]);
        // Yeniden kurmada CREATE varsayılana koyar: temizleme serbest.
        let p = alter_ok(&old, &MatViewDef { query: "SELECT 2".into(), ..m() });
        assert_eq!(sqls(&p)[1], "CREATE MATERIALIZED VIEW public.mv AS (SELECT 2\n) WITH DATA");
    }

    /// REFRESH kuralları tablosunun her satırı: (eski withData, yeni withData, refresh) → ifade/not ya da hata.
    #[test]
    fn refresh_rules_table() {
        use MatViewRefresh::{Concurrently, Normal};
        type Row = (bool, bool, Option<MatViewRefresh>, Result<Option<(&'static str, &'static str)>, &'static str>);
        let rows: [Row; 12] = [
            (false, true, None, Ok(Some(("REFRESH MATERIALIZED VIEW public.mv", REFRESH_NOTE)))),
            (false, true, Some(Normal), Ok(Some(("REFRESH MATERIALIZED VIEW public.mv", REFRESH_NOTE)))),
            (false, true, Some(Concurrently), Err("Veri yokken CONCURRENTLY kullanılamaz.")),
            (true, false, None, Ok(Some(("REFRESH MATERIALIZED VIEW public.mv WITH NO DATA", REFRESH_NO_DATA_NOTE)))),
            (true, false, Some(Normal), Err("Veriyi boşaltırken yenileme seçilemez.")),
            (true, false, Some(Concurrently), Err("Veriyi boşaltırken yenileme seçilemez.")),
            (true, true, Some(Normal), Ok(Some(("REFRESH MATERIALIZED VIEW public.mv", REFRESH_NOTE)))),
            (
                true,
                true,
                Some(Concurrently),
                Ok(Some(("REFRESH MATERIALIZED VIEW CONCURRENTLY public.mv", REFRESH_CONCURRENTLY_NOTE))),
            ),
            (true, true, None, Ok(None)),
            (false, false, None, Ok(None)),
            (false, false, Some(Normal), Err("Önce 'Veriyle doldur'u seçin.")),
            (false, false, Some(Concurrently), Err("Önce 'Veriyle doldur'u seçin.")),
        ];
        let mut out = String::new();
        for (ow, nw, refresh, want) in rows {
            let old = MatViewDef { with_data: ow, ..m() };
            let new = MatViewDef { with_data: nw, refresh, ..m() };
            let label = format!("{ow} -> {nw} {refresh:?}");
            match want {
                Ok(None) => assert!(alter_ok(&old, &new).statements.is_empty(), "{label}"),
                Ok(Some((sql, note))) => {
                    let p = alter_ok(&old, &new);
                    assert_eq!(sqls(&p), [sql], "{label}");
                    assert_eq!(p.statements[0].risk, Risk::Destructive, "{label}");
                    assert_eq!(p.statements[0].note.as_deref(), Some(note), "{label}");
                    assert!(p.statements[0].transactional);
                    assert_eq!(p.risk(), Risk::Destructive);
                    out.push_str(&format!("{label}\n{}\n", render(&p)));
                }
                Err(msg) => {
                    let e = alter_err(&old, &new);
                    assert_eq!(paths(e.clone()), ["refresh"], "{label}");
                    assert_eq!(e[0].message, msg, "{label}");
                }
            }
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn rebuild_on_query_change() {
        let old = MatViewDef {
            owner: Some("a".into()),
            comment: Some("c".into()),
            tablespace: Some("fast".into()),
            fillfactor: Some(70),
            unsupported: vec!["INDEX mv_a_idx".into(), "INDEX \"Mv Uniq\"".into(), "WITH (autovacuum_enabled=false)".into()],
            ..m()
        };
        let new = MatViewDef {
            schema: "app".into(),
            name: "w".into(),
            query: "SELECT 1, 2".into(),
            tablespace: Some("slow".into()),
            fillfactor: Some(80),
            with_data: false,
            owner: None,
            refresh: Some(MatViewRefresh::Concurrently),
            unsupported: vec![],
            ..old.clone()
        };
        let p = alter_ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "DROP MATERIALIZED VIEW public.mv",
                "CREATE MATERIALIZED VIEW app.w WITH (fillfactor = 80) TABLESPACE slow AS (SELECT 1, 2\n) WITH NO DATA",
                "ALTER MATERIALIZED VIEW app.w OWNER TO a",
                "COMMENT ON MATERIALIZED VIEW app.w IS 'c'",
            ]
        );
        assert_eq!(p.statements[0].risk, Risk::DataLoss);
        assert_eq!(p.statements[0].note.as_deref(), Some(REBUILD_DROP_NOTE));
        assert!(p.statements[1..].iter().all(|s| s.risk == Risk::Safe));
        assert_eq!(p.risk(), Risk::DataLoss);
        assert_eq!(
            p.warnings,
            [
                "İndeksler yeniden oluşturulmaz: mv_a_idx, \"Mv Uniq\"",
                GRANT_WARNING,
                "Kaybolacaklar: WITH (autovacuum_enabled=false)",
                REFRESH_IGNORED_WARNING,
            ]
        );
        insta::assert_snapshot!(render(&p));
        // Yeni sahip öncelikli; yorum yalnızca doluysa; WITH DATA notu; indeks yoksa indeks uyarısı yok.
        let p = alter_ok(
            &MatViewDef { owner: Some("a".into()), ..m() },
            &MatViewDef { query: "SELECT 2".into(), owner: Some("b".into()), ..m() },
        );
        assert_eq!(
            sqls(&p),
            [
                "DROP MATERIALIZED VIEW public.mv",
                "CREATE MATERIALIZED VIEW public.mv AS (SELECT 2\n) WITH DATA",
                "ALTER MATERIALIZED VIEW public.mv OWNER TO b",
            ]
        );
        assert_eq!(p.statements[1].note.as_deref(), Some(WITH_DATA_NOTE));
        assert_eq!(p.warnings, [GRANT_WARNING]);
        // Yeniden kurmada REFRESH tablosu uygulanmaz (false → false + refresh: hata değil, yok sayılır).
        let p = alter_ok(
            &MatViewDef { with_data: false, ..m() },
            &MatViewDef { with_data: false, query: "SELECT 2".into(), refresh: Some(MatViewRefresh::Normal), ..m() },
        );
        assert!(!sqls(&p).iter().any(|s| s.starts_with("REFRESH")));
        assert_eq!(p.warnings.last().map(String::as_str), Some(REFRESH_IGNORED_WARNING));
    }

    #[test]
    fn alter_combined_order_uses_new_name() {
        let old = MatViewDef { owner: Some("a".into()), comment: Some("c".into()), with_data: false, ..m() };
        let new = MatViewDef {
            schema: "App".into(),
            name: "select".into(),
            tablespace: Some("fast".into()),
            fillfactor: Some(50),
            owner: Some("b".into()),
            comment: None,
            with_data: true,
            ..old.clone()
        };
        let p = alter_ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "ALTER MATERIALIZED VIEW public.mv RENAME TO \"select\"",
                "ALTER MATERIALIZED VIEW public.\"select\" SET SCHEMA \"App\"",
                "ALTER MATERIALIZED VIEW \"App\".\"select\" SET TABLESPACE fast",
                "ALTER MATERIALIZED VIEW \"App\".\"select\" SET (fillfactor = 50)",
                "ALTER MATERIALIZED VIEW \"App\".\"select\" OWNER TO b",
                "COMMENT ON MATERIALIZED VIEW \"App\".\"select\" IS NULL",
                "REFRESH MATERIALIZED VIEW \"App\".\"select\"",
            ]
        );
        assert_eq!(p.risk(), Risk::Destructive);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn alter_noop_is_empty() {
        let x = MatViewDef {
            owner: Some("a".into()),
            comment: Some("c".into()),
            tablespace: Some("fast".into()),
            fillfactor: Some(70),
            unsupported: vec!["INDEX i".into()],
            ..m()
        };
        for x in [x.clone(), MatViewDef { with_data: false, ..x }] {
            let p = alter_ok(&x, &x);
            assert!(p.statements.is_empty() && p.warnings.is_empty(), "{x:?}");
        }
    }

    #[test]
    fn drop_matrix() {
        let mut out = String::new();
        for (name, cascade, want, risk) in [
            ("mv", false, "DROP MATERIALIZED VIEW public.mv", Risk::Destructive),
            ("mv", true, "DROP MATERIALIZED VIEW public.mv CASCADE", Risk::DataLoss),
            ("My \"V\"", false, "DROP MATERIALIZED VIEW public.\"My \"\"V\"\"\"", Risk::Destructive),
        ] {
            let p = bdrop(&o(MatViewDef { name: name.into(), ..m() }), cascade, &ctx()).unwrap();
            assert_single_statements(&p);
            assert_eq!(sqls(&p), [want]);
            assert_eq!(p.statements[0].risk, risk);
            assert!(p.statements[0].transactional);
            assert_eq!(p.warnings.is_empty(), !cascade);
            out.push_str(&render(&p));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
        // Yüklü sorgu/alanlar ne olursa olsun silinebilir (yalnızca kimlik doğrulanır).
        let loaded = MatViewDef { query: "SELECT 'a\\b'".into(), fillfactor: Some(1), refresh: Some(MatViewRefresh::Normal), ..m() };
        assert!(bdrop(&o(loaded), false, &ctx()).is_ok());
    }

    #[test]
    fn query_injection_matrix() {
        for q in ["1); DROP TABLE t; --", "x'", "SELECT 1 --", "SELECT 1;", "SELECT '\\'", "(SELECT 1", "SELECT 1)", "$$", "SELECT 1$$x$$", ""] {
            let d = MatViewDef { query: q.into(), ..m() };
            assert_eq!(paths(validate(&d, &ctx())), ["query"], "create {q:?}");
            assert!(bcreate(&o(d.clone()), &ctx()).is_err());
            // Alter: sorgu değişimi (yeniden kurma) aynı kapıdan geçer.
            assert_eq!(paths(alter_err(&m(), &d)), ["query"], "alter {q:?}");
        }
        for q in ["SELECT 1 -- yorum\n", "SELECT ';'", "SELECT E'\\\\'", "WITH a AS (SELECT 1) SELECT * FROM a", "VALUES (1), (2)"] {
            let d = MatViewDef { query: q.into(), ..m() };
            let p = bcreate(&o(d.clone()), &ctx()).unwrap();
            assert_single_statements(&p);
            assert!(p.statements[0].sql.ends_with(&format!("AS ({q}\n) WITH DATA")), "{q:?}");
            assert_single_statements(&alter_ok(&m(), &d));
        }
    }

    #[test]
    fn hostile_names_stay_single_statement_or_fail_on_field() {
        let long63 = "a".repeat(63);
        for n in ["v\"; DROP VIEW x; --", "select", "Görünüm ş", "日本", long63.as_str(), "'", "\\", "/*", "$$"] {
            let d = MatViewDef {
                schema: n.into(),
                name: n.into(),
                owner: Some(n.into()),
                comment: Some(n.into()),
                tablespace: Some(n.into()),
                ..m()
            };
            let new = MatViewDef {
                name: format!("{}x", &n[..n.len().min(60)]),
                tablespace: Some(format!("{}t", &n[..n.len().min(60)])),
                with_data: false,
                ..d.clone()
            };
            for p in [
                bcreate(&o(d.clone()), &ctx()).unwrap(),
                alter_ok(&d, &new),
                alter_ok(&d, &MatViewDef { query: "SELECT 2".into(), ..new.clone() }),
                alter_ok(&m(), &MatViewDef { refresh: Some(MatViewRefresh::Concurrently), ..d.clone() }),
                bdrop(&o(d.clone()), true, &ctx()).unwrap(),
                bdrop(&o(d.clone()), false, &ctx()).unwrap(),
            ] {
                assert!(!p.statements.is_empty(), "{n:?}");
                assert_single_statements(&p);
            }
        }
        let p = bcreate(&o(MatViewDef { name: "v\"; DROP VIEW x; --".into(), ..m() }), &ctx()).unwrap();
        assert_eq!(p.statements[0].sql, "CREATE MATERIALIZED VIEW public.\"v\"\"; DROP VIEW x; --\" AS (SELECT 1\n) WITH DATA");
        // Alan hataları (create ve alter): 64 bayt, satır sonu, NUL, boş, sistem şeması, yön karakteri.
        let long64 = "a".repeat(64);
        for (d, want) in [
            (MatViewDef { name: long64.clone(), ..m() }, "name"),
            (MatViewDef { name: "a\nb".into(), ..m() }, "name"),
            (MatViewDef { name: String::new(), ..m() }, "name"),
            (MatViewDef { name: "a\u{202e}b".into(), ..m() }, "name"),
            (MatViewDef { schema: "pg_catalog".into(), ..m() }, "schema"),
            (MatViewDef { schema: "a\nb".into(), ..m() }, "schema"),
            (MatViewDef { owner: Some("a\nb".into()), ..m() }, "owner"),
            (MatViewDef { owner: Some(long64.clone()), ..m() }, "owner"),
            (MatViewDef { tablespace: Some(long64.clone()), ..m() }, "tablespace"),
            (MatViewDef { tablespace: Some("a\u{85}b".into()), ..m() }, "tablespace"),
            (MatViewDef { fillfactor: Some(5), ..m() }, "fillfactor"),
            (MatViewDef { comment: Some("a\0b".into()), ..m() }, "comment"),
            (MatViewDef { query: "SELECT 1\0".into(), ..m() }, "query"),
        ] {
            assert_eq!(paths(validate(&d, &ctx())), [want], "{d:?}");
            assert_eq!(paths(alter_err(&m(), &d)), [want], "alter {d:?}");
        }
        // Eski taraf: yalnızca kimlik, `old.` önekiyle; boş/NUL/sistem şeması reddedilir.
        for (old, want) in [
            (MatViewDef { name: String::new(), ..m() }, "name"),
            (MatViewDef { name: "a\0".into(), ..m() }, "name"),
            (MatViewDef { schema: "pg_catalog".into(), ..m() }, "schema"),
        ] {
            assert_eq!(paths(alter_err(&old, &m())), [format!("old.{want}")], "{old:?}");
            assert_eq!(paths(bdrop(&o(old), false, &ctx()).unwrap_err()), [want]);
        }
    }

    #[test]
    fn old_side_identity_is_lenient_new_side_strict() {
        // Başka araçla oluşturulmuş sıfır genişlikli boşluklu / satır sonlu ad: yeniden adlandırılabilir, silinebilir.
        let zw = MatViewDef { schema: "s\u{200b}x".into(), name: "v\nx".into(), ..m() };
        let p = alter_ok(&zw, &MatViewDef { schema: "app".into(), name: "vx".into(), ..zw.clone() });
        assert_eq!(
            sqls(&p),
            [
                "ALTER MATERIALIZED VIEW \"s\u{200b}x\".\"v\nx\" RENAME TO vx",
                "ALTER MATERIALIZED VIEW \"s\u{200b}x\".vx SET SCHEMA app",
            ]
        );
        assert_eq!(sqls(&bdrop(&o(zw.clone()), false, &ctx()).unwrap()), ["DROP MATERIALIZED VIEW \"s\u{200b}x\".\"v\nx\""]);
        assert!(validate_identity(&o(zw.clone())).is_empty());
        // Ad değişmeden kalan yerinde düzenlemeler eski adı yeniden yazar ve serbesttir.
        assert_single_statements(&alter_ok(&zw, &MatViewDef { comment: Some("k".into()), ..zw.clone() }));
        assert_single_statements(&alter_ok(&zw, &MatViewDef { refresh: Some(MatViewRefresh::Normal), ..zw.clone() }));
        // Yeniden kurma YENİ adı CREATE'e yazar: aynı (yanıltıcı) ad katı kurala takılır.
        assert_eq!(
            paths(alter_err(&zw, &MatViewDef { query: "SELECT 2".into(), ..zw.clone() })),
            ["schema", "name"]
        );
        // YENİ ad/şema olarak reddedilir (create ve alter).
        assert_eq!(paths(validate(&zw, &ctx())), ["schema", "name"]);
        assert_eq!(paths(alter_err(&m(), &MatViewDef { name: "v\u{200b}x".into(), ..m() })), ["name"]);
    }

    #[test]
    fn alter_validates_only_changed_fields() {
        // Değişmeyen ama kurala aykırı yüklü alanlar (sahip, tablespace, fillfactor, yorum, sorgu) yerinde düzenlemeyi
        // engellemez; yeniden kurmada yazıldıkları için doğrulanır.
        let old = MatViewDef {
            owner: Some("a\nb".into()),
            tablespace: Some("t\u{200b}s".into()),
            fillfactor: Some(5),
            comment: Some("a\0".into()),
            query: "SELECT 'a\\b'".into(),
            ..m()
        };
        let p = alter_ok(&old, &MatViewDef { name: "w".into(), refresh: Some(MatViewRefresh::Normal), ..old.clone() });
        assert_eq!(sqls(&p), ["ALTER MATERIALIZED VIEW public.mv RENAME TO w", "REFRESH MATERIALIZED VIEW public.w"]);
        let r = MatViewDef { query: "SELECT 2".into(), ..old.clone() };
        assert_eq!(paths(alter_err(&old, &r)), ["owner", "tablespace", "fillfactor", "comment"]);
        // Sorgu değişimi aynı `\` ile reddedilir (E'…' ipucu).
        let e = alter_err(&m(), &MatViewDef { query: "SELECT 'a\\b'".into(), ..m() });
        assert!(e[0].message.contains("E'"), "{e:?}");
    }

    /// Her ifade türü → beklenen risk (tek tablo).
    #[test]
    fn risk_classes() {
        let base = MatViewDef { owner: Some("a".into()), tablespace: Some("fast".into()), ..m() };
        let first = |p: Plan| (p.statements[0].sql.clone(), p.statements[0].risk);
        let cases: Vec<(&str, (String, Risk), Risk)> = vec![
            ("CREATE MATERIALIZED VIEW", first(bcreate(&o(m()), &ctx()).unwrap()), Risk::Safe),
            (
                "CREATE MATERIALIZED VIEW",
                first(bcreate(&o(MatViewDef { with_data: false, ..m() }), &ctx()).unwrap()),
                Risk::Safe,
            ),
            ("RENAME TO", first(alter_ok(&base, &MatViewDef { name: "w".into(), ..base.clone() })), Risk::Safe),
            ("SET SCHEMA", first(alter_ok(&base, &MatViewDef { schema: "s".into(), ..base.clone() })), Risk::Safe),
            (
                "SET TABLESPACE",
                first(alter_ok(&base, &MatViewDef { tablespace: Some("t".into()), ..base.clone() })),
                Risk::Safe,
            ),
            ("SET (fillfactor", first(alter_ok(&base, &MatViewDef { fillfactor: Some(50), ..base.clone() })), Risk::Safe),
            (
                "RESET (fillfactor)",
                first(alter_ok(&MatViewDef { fillfactor: Some(50), ..base.clone() }, &base)),
                Risk::Safe,
            ),
            ("OWNER TO", first(alter_ok(&base, &MatViewDef { owner: Some("b".into()), ..base.clone() })), Risk::Safe),
            ("COMMENT ON", first(alter_ok(&base, &MatViewDef { comment: Some("c".into()), ..base.clone() })), Risk::Safe),
            (
                "REFRESH MATERIALIZED VIEW public",
                first(alter_ok(&base, &MatViewDef { refresh: Some(MatViewRefresh::Normal), ..base.clone() })),
                Risk::Destructive,
            ),
            (
                "REFRESH MATERIALIZED VIEW CONCURRENTLY",
                first(alter_ok(&base, &MatViewDef { refresh: Some(MatViewRefresh::Concurrently), ..base.clone() })),
                Risk::Destructive,
            ),
            ("WITH NO DATA", first(alter_ok(&base, &MatViewDef { with_data: false, ..base.clone() })), Risk::Destructive),
            (
                "DROP MATERIALIZED VIEW",
                first(alter_ok(&base, &MatViewDef { query: "SELECT 2".into(), ..base.clone() })),
                Risk::DataLoss,
            ),
            ("DROP MATERIALIZED VIEW", first(bdrop(&o(m()), false, &ctx()).unwrap()), Risk::Destructive),
            ("CASCADE", first(bdrop(&o(m()), true, &ctx()).unwrap()), Risk::DataLoss),
        ];
        for (needle, (sql, risk), want) in cases {
            assert!(sql.contains(needle), "{needle:?} ∉ {sql:?}");
            assert_eq!(risk, want, "{sql}");
        }
        // Yeniden kurmadaki CREATE/OWNER/COMMENT Safe.
        let p = alter_ok(&base, &MatViewDef { query: "SELECT 2".into(), comment: Some("k".into()), ..base.clone() });
        assert!(p.statements[1..].iter().all(|s| s.risk == Risk::Safe));
    }
}
