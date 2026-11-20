//! Policy (satır güvenliği politikası) üreticisi: CREATE POLICY, ALTER (RENAME, ALTER POLICY ya da yeniden kurma,
//! COMMENT), DROP. Plan 34 Karar 2–3.
//!
//! - **Kapsam:** tablo kapsamlı tür (`schema`/`table` bağlamdan gelir, alter'da değiştirilemez; PG `ALTER POLICY` ile
//!   tablo değiştirmez). `AS PERMISSIVE|RESTRICTIVE` her zaman yazılır (PG 10+ taban).
//! - **Kurallar (alan hatası):** `INSERT` → `USING` yasak; `SELECT`/`DELETE` → `WITH CHECK` yasak; roller `check_ident`,
//!   yinelenen yok, `public` öğe olarak yok (PUBLIC için liste boş bırakılır). İfadesiz politika geçerlidir (uyarı).
//! - **Serbest SQL:** `using`, `with_check` → `check_free_sql` + `wrap`; nihai ifade `composed_is_clean`.
//! - **Alter sırası:** (1) `ALTER POLICY … RENAME TO`; (2) `command`/`permissive` değişti ya da bir ifade `Some → None`
//!   oldu → `ALTER POLICY` bunları yapamaz (PG ifadeyi sessizce korur) → aynı transaction'da `DROP` (Destructive +
//!   not) + `CREATE` (yeni adla, yorum yeniden yazılır); aksi hâlde yalnız değişen parçalarla tek `ALTER POLICY`;
//!   (3) `COMMENT ON POLICY`.
//! - **Sabit nokta:** `roles` küme olarak karşılaştırılır (yükleyici ada göre sıralı yazar; sıra farkı değişiklik
//!   değildir). Değişmeyen yüklü ifade/rol yazılmadığından doğrulanmaz; `alter(x, x)` boş plandır.

use super::freesql::{check_free_sql, composed_is_clean, wrap};
use super::{BuildCtx, check_ident, check_old_ident, check_old_ref, check_old_schema_ident, check_schema_ident, check_text, err, stmt};
use crate::model::{FieldError, PolicyCommand, PolicyDef};
use crate::plan::{Plan, Risk};
use crate::quote::{qualified, quote_ident, quote_literal};

pub const REBUILD_NOTE: &str = "Politika aynı transaction içinde yeniden kurulur.";
pub const READ_ONLY_MSG: &str = "Bu politika salt okunurdur; değiştirilemez ve silinemez.";
pub const TABLE_CHANGE_MSG: &str = "Politikanın tablosu değiştirilemez.";
pub const NO_EXPR_WARNING: &str = "İfadesiz politika: PERMISSIVE tüm satırlara izin verir, RESTRICTIVE etkisizdir.";
pub const RLS_OFF_WARNING: &str = "Tabloda satır güvenliği kapalı; politika etkili olmaz.";
const COMPOSED_BROKEN: &str = "İfade, yerleştirildiği ifadeyi bozuyor";

fn table_sql(d: &PolicyDef) -> String {
    qualified(&d.schema, &d.table)
}

fn command_sql(c: PolicyCommand) -> &'static str {
    match c {
        PolicyCommand::All => "ALL",
        PolicyCommand::Select => "SELECT",
        PolicyCommand::Insert => "INSERT",
        PolicyCommand::Update => "UPDATE",
        PolicyCommand::Delete => "DELETE",
    }
}

/// `PUBLIC` ya da `<q rol>, …` (boş liste = PUBLIC).
fn roles_sql(roles: &[String]) -> String {
    if roles.is_empty() {
        "PUBLIC".into()
    } else {
        roles.iter().map(|r| quote_ident(r)).collect::<Vec<_>>().join(", ")
    }
}

/// Roller küme olarak eşit mi (sıra bağımsız).
fn same_roles(a: &[String], b: &[String]) -> bool {
    let mut x: Vec<&String> = a.iter().collect();
    let mut y: Vec<&String> = b.iter().collect();
    x.sort();
    y.sort();
    x == y
}

fn create_sql(d: &PolicyDef) -> String {
    let mut s = format!(
        "CREATE POLICY {} ON {} AS {} FOR {} TO {}",
        quote_ident(&d.name),
        table_sql(d),
        if d.permissive { "PERMISSIVE" } else { "RESTRICTIVE" },
        command_sql(d.command),
        roles_sql(&d.roles)
    );
    if let Some(u) = &d.using {
        s.push_str(&format!(" USING {}", wrap(u)));
    }
    if let Some(w) = &d.with_check {
        s.push_str(&format!(" WITH CHECK {}", wrap(w)));
    }
    s
}

/// Yorum normalleştirme: `Some("")` yorumsuz sayılır (Tablespace/Table ile tutarlı; yükleyici boş yorumu `None` döndürür).
fn norm_comment(c: &Option<String>) -> Option<&str> {
    c.as_deref().filter(|s| !s.is_empty())
}

fn comment_sql(d: &PolicyDef) -> String {
    let value = norm_comment(&d.comment).map(quote_literal).unwrap_or_else(|| "NULL".into());
    format!("COMMENT ON POLICY {} ON {} IS {value}", quote_ident(&d.name), table_sql(d))
}

/// Rol öğeleri: `check_ident` (alter'da eski listede de bulunan ad gevşek), `public` ve yineleme yok.
fn check_roles(d: &PolicyDef, old: Option<&PolicyDef>, e: &mut Vec<FieldError>) {
    for (i, r) in d.roles.iter().enumerate() {
        let path = format!("roles[{i}]");
        let before = e.len();
        if old.is_some_and(|o| o.roles.contains(r)) {
            check_old_ref(&path, r, e);
        } else {
            check_ident(&path, r, e);
        }
        if e.len() != before {
            continue;
        }
        if r.eq_ignore_ascii_case("public") {
            e.push(err(&path, "PUBLIC için listeyi boş bırakın."));
        } else if d.roles[..i].contains(r) {
            e.push(err(&path, "Rol birden çok kez belirtilmiş."));
        }
    }
}

/// Komut/ifade uyumu (PG `CREATE POLICY` kuralları).
fn check_rules(d: &PolicyDef, e: &mut Vec<FieldError>) {
    if d.command == PolicyCommand::Insert && d.using.is_some() {
        e.push(err("using", "INSERT politikasında USING kullanılamaz; yalnızca WITH CHECK."));
    }
    if matches!(d.command, PolicyCommand::Select | PolicyCommand::Delete) && d.with_check.is_some() {
        e.push(err("withCheck", "SELECT ve DELETE politikasında WITH CHECK kullanılamaz; yalnızca USING."));
    }
}

/// Nihai ifade taraması (yalnız diğer her şey geçerliyse).
fn check_composed(sql: impl FnOnce() -> String, d: &PolicyDef, e: &mut Vec<FieldError>) {
    if e.is_empty() && !composed_is_clean(&sql()) {
        e.push(err(if d.using.is_some() { "using" } else { "withCheck" }, COMPOSED_BROKEN));
    }
}

/// Planın uyarıları: ifadesiz politika, tabloda RLS kapalı (`table_rls` yalnız yükleme/varsayılan bilgisidir).
fn warnings(d: &PolicyDef) -> Vec<String> {
    let mut w = vec![];
    if d.using.is_none() && d.with_check.is_none() {
        w.push(NO_EXPR_WARNING.to_string());
    }
    if d.table_rls.is_some_and(|r| !r.enabled) {
        w.push(RLS_OFF_WARNING.to_string());
    }
    w
}

/// Oluşturma doğrulaması (tüm alanlar katı).
pub fn validate(_ctx: &BuildCtx, d: &PolicyDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_schema_ident("schema", &d.schema, &mut e);
    check_ident("table", &d.table, &mut e);
    check_ident("name", &d.name, &mut e);
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    check_rules(d, &mut e);
    check_roles(d, None, &mut e);
    if let Some(u) = &d.using {
        check_free_sql("using", u, &mut e);
    }
    if let Some(w) = &d.with_check {
        check_free_sql("withCheck", w, &mut e);
    }
    check_composed(|| create_sql(d), d, &mut e);
    e
}

/// Sıra: CREATE, COMMENT (doluysa).
pub fn create(ctx: &BuildCtx, d: &PolicyDef) -> Result<Plan, Vec<FieldError>> {
    let errs = validate(ctx, d);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut statements = vec![stmt(create_sql(d), Risk::Safe, None)];
    if norm_comment(&d.comment).is_some() {
        statements.push(stmt(comment_sql(d), Risk::Safe, None));
    }
    Ok(Plan { statements, warnings: warnings(d) })
}

/// `ALTER POLICY` ile yapılamayan değişiklik var mı (Karar 3): komut/permissive değişimi ya da ifade kaldırma.
pub(crate) fn needs_rebuild(old: &PolicyDef, new: &PolicyDef) -> bool {
    old.command != new.command
        || old.permissive != new.permissive
        || (old.using.is_some() && new.using.is_none())
        || (old.with_check.is_some() && new.with_check.is_none())
}

/// Sıra: RENAME → (yeniden kurma | ALTER POLICY) → COMMENT. `editable = false` eski tanımda her değişiklik `object`
/// hatasıdır (değişiklik yoksa boş plan).
pub fn alter(_ctx: &BuildCtx, old: &PolicyDef, new: &PolicyDef) -> Result<Plan, Vec<FieldError>> {
    let mut e = vec![];
    check_old_schema_ident("old.schema", &old.schema, &mut e);
    check_old_ident("old.table", &old.table, &mut e);
    check_old_ident("old.name", &old.name, &mut e);
    if !e.is_empty() {
        return Err(e);
    }
    if old.schema != new.schema {
        e.push(err("schema", TABLE_CHANGE_MSG));
    }
    if old.table != new.table {
        e.push(err("table", TABLE_CHANGE_MSG));
    }
    if !e.is_empty() {
        return Err(e);
    }
    let rebuild = needs_rebuild(old, new);
    let name_changed = old.name != new.name;
    let roles_changed = !same_roles(&old.roles, &new.roles);
    let using_changed = old.using != new.using;
    let check_changed = old.with_check != new.with_check;
    let comment_changed = norm_comment(&old.comment) != norm_comment(&new.comment);
    if !old.editable && (rebuild || name_changed || roles_changed || using_changed || check_changed || comment_changed) {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    // DROP + CREATE yorumu zaten siler: yeniden kurmada yalnız (varsa) yeni yorum yazılır.
    let write_comment = if rebuild { norm_comment(&new.comment).is_some() } else { comment_changed };

    // Ad aynıysa yüklü ad `old.name` olarak zaten (gevşek) denetlendi.
    if name_changed {
        check_ident("name", &new.name, &mut e);
    }
    if write_comment && let Some(c) = &new.comment {
        check_text("comment", c, &mut e);
    }
    // Komut/ifade uyumu her zaman yeni tanımda (yüklü tanım zaten uyumludur; komut değişimi ihlal doğurabilir).
    check_rules(new, &mut e);
    // Yazılan roller/ifadeler: yeniden kurmada hepsi, aksi hâlde yalnız değişenler.
    if rebuild || roles_changed {
        check_roles(new, Some(old), &mut e);
    }
    if let Some(u) = &new.using
        && (rebuild || using_changed)
    {
        check_free_sql("using", u, &mut e);
    }
    if let Some(w) = &new.with_check
        && (rebuild || check_changed)
    {
        check_free_sql("withCheck", w, &mut e);
    }

    let mut main = None;
    if rebuild {
        check_composed(|| create_sql(new), new, &mut e);
        main = e.is_empty().then(|| create_sql(new));
    } else if roles_changed || using_changed || check_changed {
        let mut s = format!("ALTER POLICY {} ON {}", quote_ident(&new.name), table_sql(new));
        if roles_changed {
            s.push_str(&format!(" TO {}", roles_sql(&new.roles)));
        }
        if using_changed && let Some(u) = &new.using {
            s.push_str(&format!(" USING {}", wrap(u)));
        }
        if check_changed && let Some(w) = &new.with_check {
            s.push_str(&format!(" WITH CHECK {}", wrap(w)));
        }
        check_composed(|| s.clone(), new, &mut e);
        main = Some(s);
    }
    if !e.is_empty() {
        return Err(e);
    }

    let mut statements = vec![];
    if name_changed {
        statements.push(stmt(
            format!("ALTER POLICY {} ON {} RENAME TO {}", quote_ident(&old.name), table_sql(old), quote_ident(&new.name)),
            Risk::Safe,
            None,
        ));
    }
    if let Some(sql) = main {
        if rebuild {
            statements.push(stmt(
                format!("DROP POLICY {} ON {}", quote_ident(&new.name), table_sql(new)),
                Risk::Destructive,
                Some(REBUILD_NOTE),
            ));
        }
        statements.push(stmt(sql, Risk::Safe, None));
    }
    if write_comment {
        statements.push(stmt(comment_sql(new), Risk::Safe, None));
    }
    let warnings = if statements.is_empty() { vec![] } else { warnings(new) };
    Ok(Plan { statements, warnings })
}

/// `DROP POLICY <q> ON <tablo>` (CASCADE yok; dağıtıcının `cascade` bayrağı yok sayılır); kimlik dağıtıcıda
/// (`validate_identity`) doğrulanır. Salt okunur silinemez.
pub fn drop(d: &PolicyDef) -> Result<Plan, Vec<FieldError>> {
    if !d.editable {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    let sql = format!("DROP POLICY {} ON {}", quote_ident(&d.name), table_sql(d));
    Ok(Plan { statements: vec![stmt(sql, Risk::Destructive, None)], warnings: vec![] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{alter as balter, assert_single_statements, create as bcreate, drop as bdrop, render};
    use crate::model::{ObjectDef, RowSecurity};

    fn p() -> PolicyDef {
        PolicyDef {
            schema: "public".into(),
            table: "t".into(),
            name: "pol".into(),
            command: PolicyCommand::All,
            permissive: true,
            roles: vec![],
            using: None,
            with_check: None,
            comment: None,
            editable: true,
            unsupported: vec![],
            table_rls: None,
        }
    }
    fn ctx() -> BuildCtx {
        BuildCtx { version_num: 160000, ..Default::default() }
    }
    fn o(d: &PolicyDef) -> ObjectDef {
        ObjectDef::Policy(d.clone())
    }
    fn s(x: &str) -> Option<String> {
        Some(x.into())
    }
    fn roles(r: &[&str]) -> Vec<String> {
        r.iter().map(|x| (*x).to_string()).collect()
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|x| x.sql.as_str()).collect()
    }
    fn paths(errs: Vec<FieldError>) -> Vec<String> {
        errs.into_iter().map(|e| e.path).collect()
    }
    fn create_ok(d: &PolicyDef) -> Plan {
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert_single_statements(&p);
        p
    }
    fn create_err(d: &PolicyDef) -> Vec<String> {
        paths(bcreate(&o(d), &ctx()).unwrap_err())
    }
    fn alter_ok(old: &PolicyDef, new: &PolicyDef) -> Plan {
        let p = balter(&o(old), &o(new), &ctx()).unwrap();
        assert_single_statements(&p);
        p
    }
    fn alter_err(old: &PolicyDef, new: &PolicyDef) -> Vec<String> {
        paths(balter(&o(old), &o(new), &ctx()).unwrap_err())
    }

    #[test]
    fn create_variants() {
        let cases: Vec<(&str, PolicyDef)> = vec![
            ("all permissive public ifadesiz", p()),
            (
                "select restrictive iki rol using",
                PolicyDef {
                    command: PolicyCommand::Select,
                    permissive: false,
                    roles: roles(&["a", "Rol B"]),
                    using: s("owner = current_user"),
                    ..p()
                },
            ),
            ("insert with check", PolicyDef { command: PolicyCommand::Insert, with_check: s("x > 0"), ..p() }),
            (
                "update her ikisi + yorum",
                PolicyDef {
                    command: PolicyCommand::Update,
                    using: s("x > 0 -- yorum\n"),
                    with_check: s("x < 9"),
                    comment: s("it's"),
                    ..p()
                },
            ),
            ("delete using", PolicyDef { command: PolicyCommand::Delete, using: s("true"), ..p() }),
            (
                "tabloda rls kapalı",
                PolicyDef { using: s("true"), table_rls: Some(RowSecurity { enabled: false, forced: false }), ..p() },
            ),
            (
                "tabloda rls açık",
                PolicyDef { using: s("true"), table_rls: Some(RowSecurity { enabled: true, forced: false }), ..p() },
            ),
        ];
        let mut out = String::new();
        for (label, d) in cases {
            let pl = create_ok(&d);
            assert!(pl.statements.iter().all(|x| x.risk == Risk::Safe && x.transactional), "{label}");
            out.push_str(&format!("# {label}\n{}\n", render(&pl)));
        }
        insta::assert_snapshot!(out);
        assert_eq!(sqls(&create_ok(&p())), ["CREATE POLICY pol ON public.t AS PERMISSIVE FOR ALL TO PUBLIC"]);
        assert_eq!(create_ok(&p()).warnings, [NO_EXPR_WARNING]);
        let rls_off = PolicyDef { using: s("true"), table_rls: Some(RowSecurity::default()), ..p() };
        assert_eq!(create_ok(&rls_off).warnings, [RLS_OFF_WARNING]);
        assert!(create_ok(&PolicyDef { using: s("true"), ..p() }).warnings.is_empty());
    }

    #[test]
    fn create_rule_violations() {
        let ins_using = PolicyDef { command: PolicyCommand::Insert, using: s("true"), ..p() };
        assert_eq!(create_err(&ins_using), ["using"]);
        for c in [PolicyCommand::Select, PolicyCommand::Delete] {
            assert_eq!(create_err(&PolicyDef { command: c, with_check: s("true"), ..p() }), ["withCheck"]);
        }
        assert_eq!(create_err(&PolicyDef { roles: roles(&["public"]), ..p() }), ["roles[0]"]);
        assert_eq!(create_err(&PolicyDef { roles: roles(&["a", "PUBLIC"]), ..p() }), ["roles[1]"]);
        assert_eq!(create_err(&PolicyDef { roles: roles(&["a", "b", "a"]), ..p() }), ["roles[2]"]);
        assert_eq!(create_err(&PolicyDef { roles: roles(&[""]), ..p() }), ["roles[0]"]);
        assert_eq!(create_err(&PolicyDef { name: String::new(), ..p() }), ["name"]);
        assert_eq!(create_err(&PolicyDef { schema: "pg_catalog".into(), ..p() }), ["schema"]);
        assert_eq!(create_err(&PolicyDef { table: "a\nb".into(), ..p() }), ["table"]);
        assert_eq!(create_err(&PolicyDef { comment: s("a\0b"), ..p() }), ["comment"]);
        assert_eq!(create_err(&PolicyDef { using: s("  "), ..p() }), ["using"]);
    }

    #[test]
    fn hostile_input_rejected() {
        for bad in ["x; DROP TABLE t", "x) OR (1=1", "x -- ", "'abc", "'a\\b'", "x \0"] {
            assert_eq!(create_err(&PolicyDef { using: s(bad), ..p() }), ["using"], "{bad}");
            assert_eq!(
                create_err(&PolicyDef { command: PolicyCommand::Insert, with_check: s(bad), ..p() }),
                ["withCheck"],
                "{bad}"
            );
        }
        // Ad/rol: tırnaklanır; denetim karakteri reddedilir.
        for bad in ["a\nb", "a\0b", "a\u{202e}b"] {
            assert_eq!(create_err(&PolicyDef { name: bad.into(), ..p() }), ["name"]);
            assert_eq!(create_err(&PolicyDef { roles: roles(&[bad]), ..p() }), ["roles[0]"]);
        }
        let d = PolicyDef { name: "a\";drop".into(), roles: roles(&["r'; --"]), using: s("x = 'a;b'"), ..p() };
        let pl = create_ok(&d);
        assert_eq!(
            pl.statements[0].sql,
            "CREATE POLICY \"a\"\";drop\" ON public.t AS PERMISSIVE FOR ALL TO \"r'; --\" USING (x = 'a;b'\n)"
        );
        // Alter'da yazılan ifade de denetlenir.
        let new = PolicyDef { using: s("x; y"), ..p() };
        assert_eq!(alter_err(&p(), &new), ["using"]);
        let new = PolicyDef { using: s("x -- "), with_check: s("1"), ..p() };
        assert_eq!(alter_err(&p(), &new), ["using"]);
    }

    #[test]
    fn alter_cases() {
        let base = PolicyDef { using: s("a > 0"), ..p() };
        let cases: Vec<(&str, PolicyDef, PolicyDef)> = vec![
            ("yalnız ad", base.clone(), PolicyDef { name: "yeni".into(), ..base.clone() }),
            ("roller", base.clone(), PolicyDef { roles: roles(&["a", "b"]), ..base.clone() }),
            ("roller PUBLIC'e", PolicyDef { roles: roles(&["a"]), ..base.clone() }, base.clone()),
            ("using değişimi", base.clone(), PolicyDef { using: s("a > 1"), ..base.clone() }),
            ("ad + using", base.clone(), PolicyDef { name: "yeni".into(), using: s("a > 1"), ..base.clone() }),
            ("komut değişimi", base.clone(), PolicyDef { command: PolicyCommand::Select, comment: s("c"), ..base.clone() }),
            ("permissive değişimi", base.clone(), PolicyDef { permissive: false, ..base.clone() }),
            ("using kaldırma", base.clone(), PolicyDef { using: None, ..base.clone() }),
            ("with check ekleme", base.clone(), PolicyDef { with_check: s("b > 0"), ..base.clone() }),
            ("using ekleme", p(), base.clone()),
            ("yorum", base.clone(), PolicyDef { comment: s("it's"), ..base.clone() }),
            ("yorum kaldırma", PolicyDef { comment: s("x"), ..base.clone() }, base.clone()),
            (
                "hepsi birden",
                base.clone(),
                PolicyDef {
                    name: "yeni".into(),
                    roles: roles(&["z"]),
                    using: s("a > 2"),
                    with_check: s("b > 2"),
                    comment: s("c"),
                    ..base.clone()
                },
            ),
        ];
        let mut out = String::new();
        for (label, old, new) in &cases {
            out.push_str(&format!("# {label}\n{}\n", render(&alter_ok(old, new))));
        }
        insta::assert_snapshot!(out);

        let rn = alter_ok(&base, &PolicyDef { name: "yeni".into(), ..base.clone() });
        assert_eq!(sqls(&rn), ["ALTER POLICY pol ON public.t RENAME TO yeni"]);
        let rl = alter_ok(&base, &PolicyDef { roles: roles(&["a"]), ..base.clone() });
        assert_eq!(sqls(&rl), ["ALTER POLICY pol ON public.t TO a"]);
        let both = alter_ok(&base, &PolicyDef { name: "yeni".into(), using: s("a > 1"), ..base.clone() });
        assert_eq!(
            sqls(&both),
            ["ALTER POLICY pol ON public.t RENAME TO yeni", "ALTER POLICY yeni ON public.t USING (a > 1\n)"]
        );
        // Yeniden kurma: DROP (Destructive + not) → CREATE (Safe), yeni adla, yorumla.
        let re = alter_ok(&base, &PolicyDef { name: "yeni".into(), command: PolicyCommand::Select, comment: s("c"), ..base.clone() });
        assert_eq!(re.statements.len(), 4);
        assert_eq!((re.statements[1].risk, re.statements[1].note.as_deref()), (Risk::Destructive, Some(REBUILD_NOTE)));
        assert_eq!(re.statements[1].sql, "DROP POLICY yeni ON public.t");
        assert!(re.statements[2].sql.starts_with("CREATE POLICY yeni ON public.t AS PERMISSIVE FOR SELECT"));
        assert_eq!(re.statements[3].sql, "COMMENT ON POLICY yeni ON public.t IS 'c'");
        assert_eq!(re.risk(), Risk::Destructive);
        // using kaldırma: ALTER değil yeniden kurma.
        let rm = alter_ok(&base, &PolicyDef { using: None, ..base.clone() });
        assert_eq!(rm.statements[0].sql, "DROP POLICY pol ON public.t");
        // with check ekleme: Safe ALTER.
        let wc = alter_ok(&base, &PolicyDef { with_check: s("b > 0"), ..base.clone() });
        assert_eq!((wc.statements.len(), wc.risk()), (1, Risk::Safe));
    }

    #[test]
    fn needs_rebuild_rules() {
        let base = PolicyDef { using: s("a"), with_check: s("b"), ..p() };
        assert!(!needs_rebuild(&base, &base));
        assert!(!needs_rebuild(&p(), &base));
        assert!(needs_rebuild(&base, &PolicyDef { command: PolicyCommand::Update, ..base.clone() }));
        assert!(needs_rebuild(&base, &PolicyDef { permissive: false, ..base.clone() }));
        assert!(needs_rebuild(&base, &PolicyDef { using: None, ..base.clone() }));
        assert!(needs_rebuild(&base, &PolicyDef { with_check: None, ..base.clone() }));
        assert!(!needs_rebuild(&base, &PolicyDef { using: s("z"), roles: roles(&["x"]), ..base.clone() }));
    }

    #[test]
    fn alter_fixed_point_and_role_order() {
        let loaded = PolicyDef {
            roles: roles(&["a", "b"]),
            using: s("a = 'x\\y'"),
            with_check: s("b"),
            comment: s("c"),
            table_rls: Some(RowSecurity::default()),
            ..p()
        };
        let pl = alter_ok(&loaded, &loaded);
        assert!(pl.statements.is_empty() && pl.warnings.is_empty());
        // Yalnız sıra farkı: boş plan.
        let swapped = PolicyDef { roles: roles(&["b", "a"]), ..loaded.clone() };
        assert!(alter_ok(&loaded, &swapped).statements.is_empty());
        // Salt okunur: değişiklik yoksa boş plan, varsa object hatası.
        let ro = PolicyDef { editable: false, ..loaded.clone() };
        assert!(alter_ok(&ro, &ro).statements.is_empty());
        assert_eq!(alter_err(&ro, &PolicyDef { comment: None, ..ro.clone() }), ["object"]);
        // Yüklü ifade düz `\` içeriyorsa yalnız o ifadeyi yazan düzenleme reddedilir.
        assert_eq!(alter_err(&loaded, &PolicyDef { using: s("a = 'z\\y'"), ..loaded.clone() }), ["using"]);
        assert!(!alter_ok(&loaded, &PolicyDef { comment: s("d"), ..loaded.clone() }).statements.is_empty());
    }

    #[test]
    fn empty_comment_is_no_comment() {
        // None → Some("") boş plan (yükleyici boş yorumu None döndürür: sabit nokta).
        let none = PolicyDef { comment: None, ..p() };
        assert!(alter_ok(&none, &PolicyDef { comment: s(""), ..none.clone() }).statements.is_empty());
        // Create'te boş yorum COMMENT ifadesi üretmez.
        assert_eq!(create_ok(&PolicyDef { comment: s(""), ..p() }).statements.len(), 1);
        // Some("x") → Some("") yorumu siler (IS NULL).
        let had = PolicyDef { comment: s("x"), ..p() };
        let p2 = alter_ok(&had, &PolicyDef { comment: s(""), ..had.clone() });
        assert_eq!(p2.statements.len(), 1);
        assert!(p2.statements[0].sql.ends_with("IS NULL"));
    }

    #[test]
    fn alter_rejects_table_change_and_bad_values() {
        assert_eq!(alter_err(&p(), &PolicyDef { table: "u".into(), ..p() }), ["table"]);
        assert_eq!(alter_err(&p(), &PolicyDef { schema: "s".into(), ..p() }), ["schema"]);
        assert_eq!(alter_err(&p(), &PolicyDef { name: "a\nb".into(), ..p() }), ["name"]);
        assert_eq!(alter_err(&p(), &PolicyDef { roles: roles(&["public"]), ..p() }), ["roles[0]"]);
        assert_eq!(alter_err(&p(), &PolicyDef { roles: roles(&["a", "a"]), ..p() }), ["roles[1]"]);
        // Komut INSERT'e çevrilirken USING duruyorsa alan hatası.
        let base = PolicyDef { using: s("a"), ..p() };
        assert_eq!(alter_err(&base, &PolicyDef { command: PolicyCommand::Insert, ..base.clone() }), ["using"]);
        // Eski taraf kimlik hatası.
        let bad_old = PolicyDef { name: String::new(), ..p() };
        assert_eq!(alter_err(&bad_old, &p()), ["old.name"]);
        // Değişmeyen yüklü rol (görünmez biçim karakteri) yeniden kurmada gevşek, yeni rol katı.
        let odd = PolicyDef { roles: roles(&["a\u{200b}b"]), using: s("x"), ..p() };
        assert_eq!(alter_ok(&odd, &PolicyDef { permissive: false, ..odd.clone() }).statements.len(), 2);
        assert_eq!(alter_err(&odd, &PolicyDef { roles: roles(&["c\u{200b}d"]), ..odd.clone() }), ["roles[0]"]);
    }

    #[test]
    fn alter_warnings() {
        let old = PolicyDef { using: s("a"), table_rls: Some(RowSecurity::default()), ..p() };
        let pl = alter_ok(&old, &PolicyDef { using: s("b"), ..old.clone() });
        assert_eq!(pl.warnings, [RLS_OFF_WARNING]);
        let pl = alter_ok(&old, &PolicyDef { using: None, ..old.clone() });
        assert_eq!(pl.warnings, [NO_EXPR_WARNING, RLS_OFF_WARNING]);
    }

    #[test]
    fn drop_is_destructive_and_ignores_cascade() {
        let d = PolicyDef { name: "Pol 1".into(), ..p() };
        for cascade in [false, true] {
            let pl = bdrop(&o(&d), cascade, &ctx()).unwrap();
            assert_single_statements(&pl);
            assert_eq!(sqls(&pl), ["DROP POLICY \"Pol 1\" ON public.t"]);
            assert_eq!(pl.risk(), Risk::Destructive);
            assert!(pl.warnings.is_empty());
        }
        assert_eq!(paths(bdrop(&o(&PolicyDef { editable: false, ..d.clone() }), false, &ctx()).unwrap_err()), ["object"]);
        assert_eq!(paths(bdrop(&o(&PolicyDef { name: String::new(), ..d }), false, &ctx()).unwrap_err()), ["name"]);
    }
}
