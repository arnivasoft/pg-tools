//! View üreticisi: CREATE, ALTER (rename, şema, `CREATE OR REPLACE VIEW`, seçenek SET/RESET, sahip, yorum, açık
//! yeniden oluşturma), DROP.
//!
//! Sorgu tek serbest SQL alanıdır: `check_free_sql` + `wrap` + nihai ifadede `composed_is_clean`. Sorgu değişince
//! üretici **her zaman** `CREATE OR REPLACE VIEW` üretir (kolon uyumluluğu yalnızca sunucuda anlaşılır, Karar 1);
//! kullanıcı açıkça `recreate` seçerse `DROP VIEW` + `CREATE VIEW` (DataLoss). Sorguya dokunmayan düzenlemeler
//! sorguyu doğrulamaz (yüklü `pg_get_viewdef` metni kapıya takılsa bile görünüm yeniden adlandırılabilir).

use super::freesql::{check_free_sql, composed_is_clean, wrap};
use super::{BuildCtx, check_ident, check_old_ident, check_old_schema_ident, check_schema_ident, check_text, err, stmt};
use crate::model::{CheckOption, FieldError, ViewDef};
use crate::plan::{Plan, Risk};
use crate::quote::{qualified, quote_ident, quote_literal};

pub const REPLACE_NOTE: &str = "Mevcut kolonlar aynı ad, tip ve sırada kalmalı; yeni kolonlar yalnız sona eklenebilir. \
     Uyumsuzsa uygulama geri alınır; 'Yeniden oluştur' seçeneğini kullanın.";
pub const RECREATE_DROP_NOTE: &str = "Görünüm silinip yeniden oluşturulur: GRANT yetkileri, tetikleyiciler/kurallar ve \
     kolon varsayılanları/yorumları kaybolur; bağımlı nesne varsa uygulama başarısız olur";
pub const RECREATE_GRANT_WARNING: &str = "Yeniden oluşturma: yetkiler (GRANT) yeniden verilmelidir";
pub const RECREATE_IGNORED_WARNING: &str = "Sorgu değişmedi; yeniden oluşturma yok sayıldı";
pub const CASCADE_WARNING: &str = "Bağımlı görünümler de silinir";
const INVOKER_VERSION: &str = "security_invoker PG 15+ gerektirir";
const COMPOSED_BROKEN: &str = "Sorgu, yerleştirildiği ifadeyi bozuyor";

fn check_option_sql(c: CheckOption) -> &'static str {
    match c {
        CheckOption::Local => "local",
        CheckOption::Cascaded => "cascaded",
    }
}

/// Seçenek listesi (deterministik sıra); yalnızca `Some`/`true` olanlar.
fn options(d: &ViewDef) -> Vec<String> {
    let mut o = vec![];
    if let Some(c) = d.check_option {
        o.push(format!("check_option = {}", check_option_sql(c)));
    }
    if d.security_barrier {
        o.push("security_barrier = true".into());
    }
    if d.security_invoker {
        o.push("security_invoker = true".into());
    }
    o
}

/// `<verb> <q> [WITH (<seçenekler>)] AS (<sorgu>\n)`.
fn define_sql(verb: &str, d: &ViewDef) -> String {
    let opts = options(d);
    let with = if opts.is_empty() { String::new() } else { format!(" WITH ({})", opts.join(", ")) };
    format!("{verb} {}{with} AS {}", qualified(&d.schema, &d.name), wrap(&d.query))
}

fn comment_sql(q: &str, comment: &Option<String>) -> String {
    match comment {
        Some(c) => format!("COMMENT ON VIEW {q} IS {}", quote_literal(c)),
        None => format!("COMMENT ON VIEW {q} IS NULL"),
    }
}

fn check_invoker(d: &ViewDef, ctx: &BuildCtx, e: &mut Vec<FieldError>) {
    if d.security_invoker && ctx.version_num < 150000 {
        e.push(err("securityInvoker", INVOKER_VERSION));
    }
}

/// Oluşturma doğrulaması (tüm alanlar).
pub fn validate(d: &ViewDef, ctx: &BuildCtx) -> Vec<FieldError> {
    let mut e = vec![];
    check_schema_ident("schema", &d.schema, &mut e);
    check_ident("name", &d.name, &mut e);
    if let Some(o) = &d.owner {
        check_ident("owner", o, &mut e);
    }
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    let before = e.len();
    check_free_sql("query", &d.query, &mut e);
    let query_ok = e.len() == before;
    check_invoker(d, ctx, &mut e);
    // Nihai ifade yalnızca adlar ve sorgu geçerliyse anlamlıdır (geçersiz ad zaten raporlandı).
    if query_ok && e.is_empty() && !composed_is_clean(&define_sql("CREATE VIEW", d)) {
        e.push(err("query", COMPOSED_BROKEN));
    }
    e
}

/// Doğrulanmış tanım varsayılır (`build::create` önce `validate` çağırır).
pub fn create(d: &ViewDef) -> Plan {
    let q = qualified(&d.schema, &d.name);
    let mut statements = vec![stmt(define_sql("CREATE VIEW", d), Risk::Safe, None)];
    if let Some(o) = &d.owner {
        statements.push(stmt(format!("ALTER VIEW {q} OWNER TO {}", quote_ident(o)), Risk::Safe, None));
    }
    if d.comment.is_some() {
        statements.push(stmt(comment_sql(&q, &d.comment), Risk::Safe, None));
    }
    Plan { statements, warnings: vec![] }
}

/// Alter doğrulaması: eskinin yalnızca kimliği, yeninin yalnızca değişen ya da ifadeye yazılan alanları.
fn validate_alter(old: &ViewDef, new: &ViewDef, recreate: bool, ctx: &BuildCtx) -> Vec<FieldError> {
    let mut e = vec![];
    // Eski (yüklü) kimlik gevşek: başka araçla oluşturulmuş yanıltıcı ad (ör. U+200B) yeniden adlandırılabilsin;
    // yalnızca `quote_ident` ile yerleşir. Sistem şeması yine reddedilir. YENİ adlar katı kurala tabidir.
    check_old_schema_ident("old.schema", &old.schema, &mut e);
    check_old_ident("old.name", &old.name, &mut e);
    if !e.is_empty() {
        return e;
    }
    let query_changed = old.query != new.query;
    if new.schema != old.schema {
        check_schema_ident("schema", &new.schema, &mut e);
    }
    if new.name != old.name {
        check_ident("name", &new.name, &mut e);
    }
    // Yeniden oluşturmada sahip her zaman yazılır (yeni nesne bağlı rolün olur).
    let owner = if recreate { new.owner.as_ref().or(old.owner.as_ref()) } else { new.owner.as_ref().filter(|_| new.owner != old.owner) };
    if let Some(o) = owner {
        check_ident("owner", o, &mut e);
    }
    if let Some(c) = &new.comment
        && (recreate || new.comment != old.comment)
    {
        check_text("comment", c, &mut e);
    }
    let before = e.len();
    if query_changed {
        check_free_sql("query", &new.query, &mut e);
    }
    let query_ok = e.len() == before;
    // C-O-R yeni seçeneklerin tamamını yazar; aksi halde yalnızca değişen seçenek yazılır.
    if query_changed || new.security_invoker != old.security_invoker {
        check_invoker(new, ctx, &mut e);
    }
    if query_changed && query_ok && e.is_empty() {
        let verb = if recreate { "CREATE VIEW" } else { "CREATE OR REPLACE VIEW" };
        if !composed_is_clean(&define_sql(verb, new)) {
            e.push(err("query", COMPOSED_BROKEN));
        }
    }
    e
}

/// DEĞİŞMEZ (yükleyici, Task 6): yüklenen görünümde `owner` hiçbir zaman `None` değildir (`pg_class.relowner` her zaman
/// vardır). `recreate` sahibi `new.owner ?? old.owner` ile "her zaman" yazar; ikisi de `None` olursa yeni görünüm bağlı
/// rolün olarak kalır ve OWNER ifadesi üretilmez.
///
/// Sıra (sonrakiler YENİ adı kullanır): rename, set schema, sorgu (`CREATE OR REPLACE VIEW`, tüm yeni seçeneklerle) ya da
/// seçenek farkı (RESET sonra SET), owner, comment. `recreate` ve sorgu değişmişse: DROP eski + CREATE yeni + owner
/// (her zaman) + comment (doluysa). Sahip `None`'a çekilemez (yok sayılır); yorum `None` ise `IS NULL`.
pub fn alter(old: &ViewDef, new: &ViewDef, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    let query_changed = old.query != new.query;
    let recreate = new.recreate && query_changed;
    let errs = validate_alter(old, new, recreate, ctx);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut statements = vec![];
    let mut warnings = vec![];
    if new.recreate && !query_changed {
        warnings.push(RECREATE_IGNORED_WARNING.to_string());
    }
    let q = qualified(&new.schema, &new.name);

    if recreate {
        statements.push(stmt(
            format!("DROP VIEW {}", qualified(&old.schema, &old.name)),
            Risk::DataLoss,
            Some(RECREATE_DROP_NOTE),
        ));
        statements.push(stmt(define_sql("CREATE VIEW", new), Risk::Safe, None));
        if let Some(o) = new.owner.as_ref().or(old.owner.as_ref()) {
            statements.push(stmt(format!("ALTER VIEW {q} OWNER TO {}", quote_ident(o)), Risk::Safe, None));
        }
        if new.comment.is_some() {
            statements.push(stmt(comment_sql(&q, &new.comment), Risk::Safe, None));
        }
        warnings.push(RECREATE_GRANT_WARNING.to_string());
        if !old.unsupported.is_empty() {
            warnings.push(format!("Kaybolacaklar: {}", old.unsupported.join(", ")));
        }
        return Ok(Plan { statements, warnings });
    }

    if old.name != new.name {
        statements.push(stmt(
            format!("ALTER VIEW {} RENAME TO {}", qualified(&old.schema, &old.name), quote_ident(&new.name)),
            Risk::Safe,
            None,
        ));
    }
    if old.schema != new.schema {
        statements.push(stmt(
            format!("ALTER VIEW {} SET SCHEMA {}", qualified(&old.schema, &new.name), quote_ident(&new.schema)),
            Risk::Safe,
            None,
        ));
    }
    if query_changed {
        // PG seçenek listesini tümüyle değiştirir (boş liste dahil): ayrı SET/RESET gerekmez.
        statements.push(stmt(define_sql("CREATE OR REPLACE VIEW", new), Risk::Safe, Some(REPLACE_NOTE)));
    } else {
        let mut set = vec![];
        let mut reset = vec![];
        if old.check_option != new.check_option {
            match new.check_option {
                Some(c) => set.push(format!("check_option = {}", check_option_sql(c))),
                None => reset.push("check_option"),
            }
        }
        if old.security_barrier != new.security_barrier {
            if new.security_barrier {
                set.push("security_barrier = true".into());
            } else {
                reset.push("security_barrier");
            }
        }
        if old.security_invoker != new.security_invoker {
            if new.security_invoker {
                set.push("security_invoker = true".into());
            } else {
                reset.push("security_invoker");
            }
        }
        if !reset.is_empty() {
            statements.push(stmt(format!("ALTER VIEW {q} RESET ({})", reset.join(", ")), Risk::Safe, None));
        }
        if !set.is_empty() {
            statements.push(stmt(format!("ALTER VIEW {q} SET ({})", set.join(", ")), Risk::Safe, None));
        }
    }
    if let Some(o) = &new.owner
        && new.owner != old.owner
    {
        statements.push(stmt(format!("ALTER VIEW {q} OWNER TO {}", quote_ident(o)), Risk::Safe, None));
    }
    if old.comment != new.comment {
        statements.push(stmt(comment_sql(&q, &new.comment), Risk::Safe, None));
    }
    Ok(Plan { statements, warnings })
}

pub fn drop(d: &ViewDef, cascade: bool) -> Plan {
    let q = qualified(&d.schema, &d.name);
    if cascade {
        Plan {
            statements: vec![stmt(format!("DROP VIEW {q} CASCADE"), Risk::DataLoss, Some(CASCADE_WARNING))],
            warnings: vec![CASCADE_WARNING.into()],
        }
    } else {
        Plan { statements: vec![stmt(format!("DROP VIEW {q}"), Risk::Destructive, None)], warnings: vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{alter as balter, assert_single_statements, create as bcreate, drop as bdrop, render};
    use crate::model::{ObjectDef, ViewColumn};

    fn v() -> ViewDef {
        ViewDef {
            schema: "public".into(),
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
        }
    }
    fn o(d: ViewDef) -> ObjectDef {
        ObjectDef::View(d)
    }
    fn ctx_v(version_num: u32) -> BuildCtx {
        BuildCtx { version_num, ..Default::default() }
    }
    fn ctx() -> BuildCtx {
        ctx_v(160000)
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn paths(errs: Vec<FieldError>) -> Vec<String> {
        errs.into_iter().map(|e| e.path).collect()
    }
    fn alter_ok(old: &ViewDef, new: &ViewDef) -> Plan {
        let p = balter(&o(old.clone()), &o(new.clone()), &ctx()).unwrap();
        assert_single_statements(&p);
        p
    }

    #[test]
    fn create_variants() {
        let cases = [
            (v(), vec!["CREATE VIEW public.v AS (SELECT 1\n)"]),
            (
                ViewDef { check_option: Some(CheckOption::Local), ..v() },
                vec!["CREATE VIEW public.v WITH (check_option = local) AS (SELECT 1\n)"],
            ),
            (
                ViewDef { check_option: Some(CheckOption::Cascaded), ..v() },
                vec!["CREATE VIEW public.v WITH (check_option = cascaded) AS (SELECT 1\n)"],
            ),
            (ViewDef { security_barrier: true, ..v() }, vec!["CREATE VIEW public.v WITH (security_barrier = true) AS (SELECT 1\n)"]),
            (
                ViewDef { security_invoker: true, ..v() },
                vec!["CREATE VIEW public.v WITH (security_invoker = true) AS (SELECT 1\n)"],
            ),
            (
                ViewDef {
                    check_option: Some(CheckOption::Local),
                    security_barrier: true,
                    security_invoker: true,
                    owner: Some("Bob".into()),
                    comment: Some("it's \\ ok".into()),
                    name: "My View".into(),
                    ..v()
                },
                vec![
                    "CREATE VIEW public.\"My View\" WITH (check_option = local, security_barrier = true, security_invoker = true) AS (SELECT 1\n)",
                    "ALTER VIEW public.\"My View\" OWNER TO \"Bob\"",
                    "COMMENT ON VIEW public.\"My View\" IS E'it''s \\\\ ok'",
                ],
            ),
        ];
        let mut out = String::new();
        for (d, want) in cases {
            let p = bcreate(&o(d), &ctx()).unwrap();
            assert_single_statements(&p);
            assert_eq!(sqls(&p), want);
            assert!(p.statements.iter().all(|s| s.risk == Risk::Safe && s.transactional && !s.redacted));
            out.push_str(&render(&p));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn security_invoker_is_version_gated() {
        let d = ViewDef { security_invoker: true, ..v() };
        let e = bcreate(&o(d.clone()), &ctx_v(140000)).unwrap_err();
        assert_eq!((e[0].path.as_str(), e[0].message.as_str()), ("securityInvoker", "security_invoker PG 15+ gerektirir"));
        assert!(bcreate(&o(d.clone()), &ctx_v(150000)).is_ok());
        // Alter: seçenek eklemek de, sorgu değişince C-O-R'a yazılması da kapıya tabi.
        assert_eq!(paths(balter(&o(v()), &o(d.clone()), &ctx_v(140000)).unwrap_err()), ["securityInvoker"]);
        let q = ViewDef { query: "SELECT 2".into(), ..d.clone() };
        assert_eq!(paths(balter(&o(d.clone()), &o(q), &ctx_v(140000)).unwrap_err()), ["securityInvoker"]);
        // PG 14'te kapalıyken diğer düzenlemeler serbest.
        let renamed = ViewDef { name: "w".into(), ..v() };
        assert!(balter(&o(v()), &o(renamed), &ctx_v(140000)).is_ok());
        assert!(bcreate(&o(v()), &ctx_v(140000)).is_ok());
    }

    #[test]
    fn alter_single_changes() {
        let base = ViewDef { owner: Some("a".into()), comment: Some("c".into()), ..v() };
        let cases: Vec<(ViewDef, Vec<&str>)> = vec![
            (ViewDef { name: "w".into(), ..base.clone() }, vec!["ALTER VIEW public.v RENAME TO w"]),
            (ViewDef { schema: "app".into(), ..base.clone() }, vec!["ALTER VIEW public.v SET SCHEMA app"]),
            (ViewDef { query: "SELECT 2".into(), ..base.clone() }, vec!["CREATE OR REPLACE VIEW public.v AS (SELECT 2\n)"]),
            (
                ViewDef { check_option: Some(CheckOption::Cascaded), security_barrier: true, ..base.clone() },
                vec!["ALTER VIEW public.v SET (check_option = cascaded, security_barrier = true)"],
            ),
            (ViewDef { owner: Some("b".into()), ..base.clone() }, vec!["ALTER VIEW public.v OWNER TO b"]),
            (ViewDef { owner: None, ..base.clone() }, vec![]),
            (ViewDef { comment: None, ..base.clone() }, vec!["COMMENT ON VIEW public.v IS NULL"]),
            (ViewDef { comment: Some("d".into()), ..base.clone() }, vec!["COMMENT ON VIEW public.v IS 'd'"]),
            // Yalnızca yükleme alanları üretici tarafından okunmaz.
            (
                ViewDef {
                    columns: vec![ViewColumn { name: "x".into(), data_type: "int".into() }],
                    unsupported: vec!["TRIGGER t".into()],
                    ..base.clone()
                },
                vec![],
            ),
        ];
        let mut out = String::new();
        for (new, want) in cases {
            let p = alter_ok(&base, &new);
            assert_eq!(sqls(&p), want, "{new:?}");
            assert!(p.statements.iter().all(|s| s.risk == Risk::Safe));
            assert!(p.warnings.is_empty());
            out.push_str(&render(&p));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
        // C-O-R notu.
        let p = alter_ok(&base, &ViewDef { query: "SELECT 2".into(), ..base.clone() });
        assert_eq!(p.statements[0].note.as_deref(), Some(REPLACE_NOTE));
    }

    #[test]
    fn alter_query_and_options_together_is_single_replace() {
        let old = ViewDef { check_option: Some(CheckOption::Local), security_barrier: true, ..v() };
        // Seçenekler değişti + sorgu değişti → tek C-O-R, WITH yeni seçeneklerin tamamı; SET/RESET yok.
        let new = ViewDef { query: "SELECT 2".into(), check_option: None, security_invoker: true, ..old.clone() };
        let p = alter_ok(&old, &new);
        assert_eq!(sqls(&p), ["CREATE OR REPLACE VIEW public.v WITH (security_barrier = true, security_invoker = true) AS (SELECT 2\n)"]);
        // Tüm seçenekler kalkarsa WITH yazılmaz (PG listeyi boşaltır).
        let new = ViewDef { query: "SELECT 2".into(), ..v() };
        assert_eq!(sqls(&alter_ok(&old, &new)), ["CREATE OR REPLACE VIEW public.v AS (SELECT 2\n)"]);
    }

    #[test]
    fn alter_options_set_reset_order() {
        let all = ViewDef { check_option: Some(CheckOption::Local), security_barrier: true, security_invoker: true, ..v() };
        // Yalnız RESET.
        assert_eq!(sqls(&alter_ok(&all, &v())), ["ALTER VIEW public.v RESET (check_option, security_barrier, security_invoker)"]);
        // Yalnız SET.
        assert_eq!(
            sqls(&alter_ok(&v(), &all)),
            ["ALTER VIEW public.v SET (check_option = local, security_barrier = true, security_invoker = true)"]
        );
        // check_option değeri değişimi SET'tir.
        let casc = ViewDef { check_option: Some(CheckOption::Cascaded), ..all.clone() };
        assert_eq!(sqls(&alter_ok(&all, &casc)), ["ALTER VIEW public.v SET (check_option = cascaded)"]);
        // SET + RESET: önce RESET.
        let mixed = ViewDef { check_option: None, security_barrier: true, ..v() };
        let old = ViewDef { check_option: Some(CheckOption::Local), ..v() };
        assert_eq!(
            sqls(&alter_ok(&old, &mixed)),
            ["ALTER VIEW public.v RESET (check_option)", "ALTER VIEW public.v SET (security_barrier = true)"]
        );
    }

    #[test]
    fn alter_combined_order_uses_new_name_and_schema() {
        let old = ViewDef { owner: Some("a".into()), comment: Some("c".into()), ..v() };
        let new = ViewDef {
            schema: "App".into(),
            name: "select".into(),
            query: "SELECT 2".into(),
            owner: Some("b".into()),
            comment: None,
            security_barrier: true,
            ..old.clone()
        };
        let p = alter_ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "ALTER VIEW public.v RENAME TO \"select\"",
                "ALTER VIEW public.\"select\" SET SCHEMA \"App\"",
                "CREATE OR REPLACE VIEW \"App\".\"select\" WITH (security_barrier = true) AS (SELECT 2\n)",
                "ALTER VIEW \"App\".\"select\" OWNER TO b",
                "COMMENT ON VIEW \"App\".\"select\" IS NULL",
            ]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn recreate_drops_old_and_creates_new() {
        let old = ViewDef {
            owner: Some("a".into()),
            comment: Some("c".into()),
            unsupported: vec!["TRIGGER t".into(), "RULE r".into()],
            ..v()
        };
        let new = ViewDef {
            schema: "app".into(),
            name: "w".into(),
            query: "SELECT 1, 2".into(),
            owner: None,
            recreate: true,
            check_option: Some(CheckOption::Local),
            unsupported: vec![],
            ..old.clone()
        };
        let p = alter_ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "DROP VIEW public.v",
                "CREATE VIEW app.w WITH (check_option = local) AS (SELECT 1, 2\n)",
                "ALTER VIEW app.w OWNER TO a",
                "COMMENT ON VIEW app.w IS 'c'",
            ]
        );
        assert_eq!(p.statements[0].risk, Risk::DataLoss);
        assert_eq!(p.statements[0].note.as_deref(), Some(RECREATE_DROP_NOTE));
        assert!(p.statements[1..].iter().all(|s| s.risk == Risk::Safe));
        assert_eq!(p.risk(), Risk::DataLoss);
        assert_eq!(p.warnings, [RECREATE_GRANT_WARNING, "Kaybolacaklar: TRIGGER t, RULE r"]);
        insta::assert_snapshot!(render(&p));
        // Sahip her zaman yazılır (yeni sahip öncelikli); yorum yalnızca doluysa.
        let same = ViewDef { query: "SELECT 2".into(), recreate: true, owner: Some("b".into()), comment: None, ..v() };
        let p = alter_ok(&ViewDef { owner: Some("a".into()), ..v() }, &same);
        assert_eq!(sqls(&p), ["DROP VIEW public.v", "CREATE VIEW public.v AS (SELECT 2\n)", "ALTER VIEW public.v OWNER TO b"]);
        assert_eq!(p.warnings, [RECREATE_GRANT_WARNING]);
    }

    #[test]
    fn recreate_without_query_change_is_ignored() {
        let x = ViewDef { recreate: true, ..v() };
        let p = alter_ok(&v(), &x);
        assert!(p.statements.is_empty());
        assert_eq!(p.warnings, [RECREATE_IGNORED_WARNING]);
        // Diğer değişiklikler sıradan alter olarak üretilir.
        let p = alter_ok(&v(), &ViewDef { name: "w".into(), ..x });
        assert_eq!(sqls(&p), ["ALTER VIEW public.v RENAME TO w"]);
        assert_eq!(p.warnings, [RECREATE_IGNORED_WARNING]);
    }

    #[test]
    fn alter_noop_is_empty() {
        let x = ViewDef {
            owner: Some("a".into()),
            comment: Some("c".into()),
            check_option: Some(CheckOption::Cascaded),
            security_barrier: true,
            ..v()
        };
        let p = alter_ok(&x, &x);
        assert!(p.statements.is_empty() && p.warnings.is_empty());
    }

    #[test]
    fn drop_matrix() {
        let mut out = String::new();
        for (name, cascade, want, risk) in [
            ("v", false, "DROP VIEW public.v", Risk::Destructive),
            ("v", true, "DROP VIEW public.v CASCADE", Risk::DataLoss),
            ("My \"V\"", false, "DROP VIEW public.\"My \"\"V\"\"\"", Risk::Destructive),
        ] {
            let p = bdrop(&o(ViewDef { name: name.into(), ..v() }), cascade, &ctx()).unwrap();
            assert_single_statements(&p);
            assert_eq!(sqls(&p), [want]);
            assert_eq!(p.statements[0].risk, risk);
            assert_eq!(p.warnings.is_empty(), !cascade);
            out.push_str(&render(&p));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
        // Yüklü sorgu ne olursa olsun silinebilir (sorgu doğrulanmaz).
        assert!(bdrop(&o(ViewDef { query: "SELECT 'a\\b'".into(), ..v() }), false, &ctx()).is_ok());
    }

    #[test]
    fn query_injection_matrix() {
        for q in ["1); DROP TABLE t; --", "x'", "SELECT 1 --", "SELECT 1;", "SELECT '\\'", "(SELECT 1", "SELECT 1)", "$$", "SELECT 1$$x$$"] {
            let d = ViewDef { query: q.into(), ..v() };
            assert_eq!(paths(validate(&d, &ctx())), ["query"], "create {q:?}");
            assert!(bcreate(&o(d.clone()), &ctx()).is_err());
            // Alter: sorgu değişimi (C-O-R ve recreate) de aynı kapıdan geçer.
            assert_eq!(paths(balter(&o(v()), &o(d.clone()), &ctx()).unwrap_err()), ["query"], "alter {q:?}");
            let r = ViewDef { recreate: true, ..d };
            assert_eq!(paths(balter(&o(v()), &o(r), &ctx()).unwrap_err()), ["query"], "recreate {q:?}");
        }
        for q in ["SELECT 1 -- yorum\n", "SELECT ';'", "SELECT E'\\\\'", "WITH a AS (SELECT 1) SELECT * FROM a", "VALUES (1), (2)"] {
            let d = ViewDef { query: q.into(), ..v() };
            let p = bcreate(&o(d.clone()), &ctx()).unwrap();
            assert_single_statements(&p);
            assert!(p.statements[0].sql.ends_with(&format!("AS ({q}\n)")), "{q:?}");
            assert_single_statements(&alter_ok(&v(), &d));
            assert_single_statements(&alter_ok(&v(), &ViewDef { recreate: true, ..d }));
        }
    }

    #[test]
    fn hostile_names_stay_single_statement_or_fail_on_field() {
        let long63 = "a".repeat(63);
        for n in ["v\"; DROP VIEW x; --", "select", "Görünüm ş", "日本", long63.as_str(), "'", "\\", "/*", "$$"] {
            let d = ViewDef {
                schema: n.into(),
                name: n.into(),
                owner: Some(n.into()),
                comment: Some(n.into()),
                check_option: Some(CheckOption::Local),
                ..v()
            };
            let new = ViewDef { name: format!("{}x", &n[..n.len().min(60)]), query: "SELECT 2".into(), ..d.clone() };
            for p in [
                bcreate(&o(d.clone()), &ctx()).unwrap(),
                alter_ok(&d, &new),
                alter_ok(&d, &ViewDef { recreate: true, ..new.clone() }),
                alter_ok(&v(), &d),
                bdrop(&o(d.clone()), true, &ctx()).unwrap(),
                bdrop(&o(d.clone()), false, &ctx()).unwrap(),
            ] {
                assert!(!p.statements.is_empty(), "{n:?}");
                assert_single_statements(&p);
            }
        }
        let p = bcreate(&o(ViewDef { name: "v\"; DROP VIEW x; --".into(), ..v() }), &ctx()).unwrap();
        assert_eq!(p.statements[0].sql, "CREATE VIEW public.\"v\"\"; DROP VIEW x; --\" AS (SELECT 1\n)");
        // Alan hataları: 64 bayt, satır sonu, NUL, boş, sistem şeması.
        let long64 = "a".repeat(64);
        for (d, want) in [
            (ViewDef { name: long64.clone(), ..v() }, "name"),
            (ViewDef { name: "a\nb".into(), ..v() }, "name"),
            (ViewDef { name: String::new(), ..v() }, "name"),
            (ViewDef { schema: "pg_catalog".into(), ..v() }, "schema"),
            (ViewDef { schema: "a\nb".into(), ..v() }, "schema"),
            (ViewDef { owner: Some("a\nb".into()), ..v() }, "owner"),
            (ViewDef { owner: Some(long64.clone()), ..v() }, "owner"),
            (ViewDef { comment: Some("a\0b".into()), ..v() }, "comment"),
            (ViewDef { query: "SELECT 1\0".into(), ..v() }, "query"),
        ] {
            assert_eq!(paths(validate(&d, &ctx())), [want], "{d:?}");
            assert_eq!(paths(balter(&o(v()), &o(d.clone()), &ctx()).unwrap_err()), [want], "alter {d:?}");
        }
        // Eski taraf: yalnızca kimlik, `old.` önekiyle.
        let bad_old = ViewDef { name: "a\nb".into(), ..v() };
        let empty_old = ViewDef { name: String::new(), ..v() };
        assert_eq!(paths(balter(&o(empty_old.clone()), &o(v()), &ctx()).unwrap_err()), ["old.name"]);
        assert_eq!(paths(bdrop(&o(empty_old), false, &ctx()).unwrap_err()), ["name"]);
        let sys_old = ViewDef { schema: "pg_catalog".into(), ..v() };
        assert_eq!(paths(balter(&o(sys_old.clone()), &o(v()), &ctx()).unwrap_err()), ["old.schema"]);
        assert_eq!(paths(bdrop(&o(sys_old), false, &ctx()).unwrap_err()), ["schema"]);
        // Satır sonlu yüklü ad: eski tarafta gevşek (yalnızca `quote_ident` ile yerleşir), yeni adda katı.
        assert_single_statements(&alter_ok(&bad_old, &v()));
        assert!(bdrop(&o(bad_old.clone()), false, &ctx()).is_ok());
        assert_eq!(paths(balter(&o(v()), &o(bad_old), &ctx()).unwrap_err()), ["name"]);
    }

    #[test]
    fn loaded_unclean_query_allows_rename_but_not_query_change() {
        // `pg_get_viewdef` sıradan '…' içinde `\` yazabilir: sorguya dokunmayan düzenlemeler serbest.
        let loaded = ViewDef { query: "SELECT 'a\\b'::text AS x".into(), owner: Some("a".into()), ..v() };
        let renamed = ViewDef { name: "w".into(), schema: "app".into(), owner: Some("b".into()), security_barrier: true, ..loaded.clone() };
        let p = alter_ok(&loaded, &renamed);
        assert_eq!(
            sqls(&p),
            [
                "ALTER VIEW public.v RENAME TO w",
                "ALTER VIEW public.w SET SCHEMA app",
                "ALTER VIEW app.w SET (security_barrier = true)",
                "ALTER VIEW app.w OWNER TO b",
            ]
        );
        // Sorgu değişince (aynı `\` korunursa) kapı reddeder; E'…' kabul edilir.
        let changed = ViewDef { query: "SELECT 'a\\b'::text AS x, 1 AS y".into(), ..loaded.clone() };
        let e = balter(&o(loaded.clone()), &o(changed), &ctx()).unwrap_err();
        assert_eq!(paths(e.clone()), ["query"]);
        assert!(e[0].message.contains("E'"), "{e:?}");
        let fixed = ViewDef { query: "SELECT E'a\\\\b'::text AS x, 1 AS y".into(), ..loaded.clone() };
        assert_eq!(sqls(&alter_ok(&loaded, &fixed)), ["CREATE OR REPLACE VIEW public.v AS (SELECT E'a\\\\b'::text AS x, 1 AS y\n)"]);
        // Yalnızca yorum değişimi de serbest.
        assert_eq!(sqls(&alter_ok(&loaded, &ViewDef { comment: Some("k".into()), ..loaded.clone() })), ["COMMENT ON VIEW public.v IS 'k'"]);
    }

    #[test]
    fn alter_validates_only_changed_fields() {
        // Değişmeyen ama kurala aykırı sahip (yüklü) alter'ı engellemez; yeniden oluşturmada yazıldığı için doğrulanır.
        let old = ViewDef { owner: Some("a\nb".into()), ..v() };
        assert!(balter(&o(old.clone()), &o(ViewDef { name: "w".into(), ..old.clone() }), &ctx()).is_ok());
        let r = ViewDef { query: "SELECT 2".into(), recreate: true, ..old.clone() };
        assert_eq!(paths(balter(&o(old.clone()), &o(r), &ctx()).unwrap_err()), ["owner"]);
        // Değişmeyen NUL'lu yorum: yalnızca recreate'te (yeniden yazılır) hata.
        let old = ViewDef { comment: Some("a\0".into()), ..v() };
        assert!(balter(&o(old.clone()), &o(ViewDef { name: "w".into(), ..old.clone() }), &ctx()).is_ok());
        let r = ViewDef { query: "SELECT 2".into(), recreate: true, ..old.clone() };
        assert_eq!(paths(balter(&o(old.clone()), &o(r), &ctx()).unwrap_err()), ["comment"]);
    }

    #[test]
    fn old_side_identity_is_lenient_new_side_strict() {
        // Başka araçla oluşturulmuş sıfır genişlikli boşluklu ad / şema: yeniden adlandırılabilir, taşınabilir, silinebilir.
        let zw = ViewDef { schema: "s\u{200b}x".into(), name: "v\u{200b}x".into(), ..v() };
        let p = alter_ok(&zw, &ViewDef { schema: "app".into(), name: "vx".into(), ..zw.clone() });
        assert_eq!(sqls(&p), ["ALTER VIEW \"s\u{200b}x\".\"v\u{200b}x\" RENAME TO vx", "ALTER VIEW \"s\u{200b}x\".vx SET SCHEMA app"]);
        let p = bdrop(&o(zw.clone()), false, &ctx()).unwrap();
        assert_eq!(sqls(&p), ["DROP VIEW \"s\u{200b}x\".\"v\u{200b}x\""]);
        assert!(crate::build::validate_identity(&o(zw.clone())).is_empty());
        // Ad değişmeden kalan düzenlemeler (yorum, sorgu, recreate) eski adı yeniden yazar ve serbesttir.
        assert_single_statements(&alter_ok(&zw, &ViewDef { comment: Some("k".into()), ..zw.clone() }));
        assert_single_statements(&alter_ok(&zw, &ViewDef { query: "SELECT 2".into(), recreate: true, ..zw.clone() }));
        // YENİ ad/şema olarak aynı karakter reddedilir (create ve alter).
        assert_eq!(paths(validate(&zw, &ctx())), ["schema", "name"]);
        assert_eq!(paths(balter(&o(v()), &o(ViewDef { name: "v\u{200b}x".into(), ..v() }), &ctx()).unwrap_err()), ["name"]);
        assert_eq!(paths(balter(&o(v()), &o(ViewDef { schema: "s\u{200b}x".into(), ..v() }), &ctx()).unwrap_err()), ["schema"]);
        // NUL eski tarafta da reddedilir.
        let nul = ViewDef { name: "a\0".into(), ..v() };
        assert_eq!(paths(balter(&o(nul.clone()), &o(v()), &ctx()).unwrap_err()), ["old.name"]);
        assert_eq!(paths(bdrop(&o(nul), false, &ctx()).unwrap_err()), ["name"]);
    }
}
