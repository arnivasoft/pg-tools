//! Domain üreticisi: CREATE DOMAIN, ALTER (RENAME, SET SCHEMA, DEFAULT, NOT NULL, CHECK kısıtları, COMMENT, OWNER), DROP.
//!
//! - **Create:** `CREATE DOMAIN <q> AS <tip> [COLLATE <q>] [DEFAULT (<ifade>\n)] [NOT NULL] [CONSTRAINT <q>] CHECK
//!   (<ifade>\n) …`; adsız kısıt create'te serbest (adı sunucu verir), `NOT VALID` create'te alan hatası (PG
//!   desteklemez). Ardından yorum ve sahip (en sonda).
//! - **Serbest SQL:** `default` ve kısıt `expression` → `check_free_sql` + `wrap`; nihai ifade `composed_is_clean`.
//!   Temel tip `validate_type`; alter "değişti mi" `canonical_type` ile (yüklü `format_type` çıktısına karşı).
//! - **Alter (Karar 9):** temel tip ve collation create-only (değişim alan hatası). Kısıtlar `original` (yüklü ad) ile
//!   eşlenir: ad değişimi `RENAME CONSTRAINT`; ifade değişimi (metin eşitliği; yüklü `pg_get_constraintdef` biçimine
//!   karşı) eski adla `DROP CONSTRAINT` (Destructive) + yeni adla `ADD CONSTRAINT`; eşlenmeyen yüklü kısıt `DROP`
//!   (Destructive); yeni kısıt `ADD CONSTRAINT … [NOT VALID]` (Safe; NOT VALID'siz ise "kullanan kolonları tarar"
//!   notu). Alter'da eklenen kısıtın adı **zorunludur** (sonraki düzenlemede eşlenebilmesi için; adsız ADD sunucu adı
//!   üretirdi ve taslak yüklemeyle eşit olmazdı). `NOT VALID` yalnız eklenen (yeni ya da ifadesi değişen) kısıtta;
//!   `validate` yalnız yüklü, doğrulanmamış ve ifadesi değişmeyen kısıtta (`VALIDATE CONSTRAINT`, yeni adla).
//!   Yeniden adlandırma hedefi başka bir yüklü ad olamaz (RENAME, DROP'tan önce çalışır); taslakta adlar tekil.
//! - **Sıra:** RENAME → SET SCHEMA → SET/DROP DEFAULT → DROP NOT NULL → kısıt RENAME, DROP, ADD, VALIDATE → SET NOT
//!   NULL (Destructive; kısıtlar oturduktan sonra) → COMMENT → OWNER TO (en sonda).

use super::freesql::{check_free_sql, composed_is_clean, wrap};
use super::type_canon::canonical_type;
use super::type_syntax::validate_type;
use super::{BuildCtx, check_ident, check_old_ident, check_old_schema_ident, check_schema_ident, check_text, err, stmt};
use crate::model::{DomainCheck, DomainDef, FieldError};
use crate::plan::{Plan, Risk, Statement};
use crate::quote::{qualified, quote_ident, quote_literal};

pub const ADD_CHECK_NOTE: &str = "kullanan kolonları tarar";
pub const SET_NOT_NULL_NOTE: &str = "kullanan kolonları tarar; NULL değer varsa başarısız olur";
pub const EXPR_CHANGE_NOTE: &str = "ifade değişti: kısıt kaldırılıp yeniden eklenir";
pub const READ_ONLY_MSG: &str = "Bu domain salt okunurdur (eklentiye ait ya da sistem domain'i); değiştirilemez ve silinemez.";
pub const CREATE_ONLY_MSG: &str = "Domain'in temel tipi ve collation'ı oluşturulduktan sonra değiştirilemez.";
pub const RENAME_CLASH_MSG: &str =
    "Yeni ad başka bir yüklü kısıtla çakışıyor; önce onu ayrı bir değişiklikte yeniden adlandırın.";
pub const NAME_REQUIRED_MSG: &str = "Düzenlemede kısıt adı zorunludur (sonradan eşlenebilmesi için).";
pub const NOT_VALID_CREATE_MSG: &str = "NOT VALID yalnızca var olan domain'e kısıt eklerken kullanılabilir.";
pub const NOT_VALID_LOADED_MSG: &str = "NOT VALID yalnızca eklenen (yeni ya da ifadesi değişen) kısıtta kullanılabilir.";
pub const VALIDATE_MSG: &str = "Doğrulama yalnızca yüklü, doğrulanmamış ve ifadesi değişmeyen kısıtta kullanılabilir.";
pub const CASCADE_WARNING: &str = "Bağımlı nesneler de silinir";
const DUP_NAME_MSG: &str = "Kısıt adı birden çok kez kullanılmış.";
const COMPOSED_BROKEN: &str = "İfade, yerleştirildiği ifadeyi bozuyor";

fn domain_sql(schema: &str, name: &str) -> String {
    qualified(schema, name)
}

fn comment_sql(dom: &str, comment: &Option<String>) -> String {
    let value = comment.as_deref().map(quote_literal).unwrap_or_else(|| "NULL".into());
    format!("COMMENT ON DOMAIN {dom} IS {value}")
}

fn owner_sql(dom: &str, owner: &str) -> String {
    format!("ALTER DOMAIN {dom} OWNER TO {}", quote_ident(owner))
}

/// `[CONSTRAINT <q>] CHECK (<ifade>\n) [NOT VALID]`.
fn check_sql(c: &DomainCheck) -> String {
    let mut s = match &c.name {
        Some(n) => format!("CONSTRAINT {} CHECK {}", quote_ident(n), wrap(&c.expression)),
        None => format!("CHECK {}", wrap(&c.expression)),
    };
    if c.not_valid {
        s.push_str(" NOT VALID");
    }
    s
}

fn define_sql(d: &DomainDef) -> String {
    let mut s = format!("CREATE DOMAIN {} AS {}", domain_sql(&d.schema, &d.name), d.base_type);
    if let Some(c) = &d.collation {
        s.push_str(&format!(" COLLATE {}", quote_ident(c)));
    }
    if let Some(x) = &d.default {
        s.push_str(&format!(" DEFAULT {}", wrap(x)));
    }
    if d.not_null {
        s.push_str(" NOT NULL");
    }
    for c in &d.constraints {
        s.push(' ');
        s.push_str(&check_sql(c));
    }
    s
}

fn check_type(path: &str, v: &str, e: &mut Vec<FieldError>) {
    if let Err(m) = validate_type(v) {
        e.push(err(path, m));
    }
}

/// Tek serbest parçanın kendi bağlamında temizliği (nihai ifade bozulursa hatanın yerini bulmak için).
fn fragment_clean(f: &str) -> bool {
    composed_is_clean(&format!("CHECK {}", wrap(f)))
}

/// Oluşturma doğrulaması (tüm alanlar katı; diğer her şey geçerliyse nihai ifade de taranır).
pub fn validate(_ctx: &BuildCtx, d: &DomainDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_schema_ident("schema", &d.schema, &mut e);
    check_ident("name", &d.name, &mut e);
    if let Some(o) = &d.owner {
        check_ident("owner", o, &mut e);
    }
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    check_type("baseType", &d.base_type, &mut e);
    if let Some(c) = &d.collation {
        check_ident("collation", c, &mut e);
    }
    if let Some(x) = &d.default {
        check_free_sql("default", x, &mut e);
    }
    for (i, c) in d.constraints.iter().enumerate() {
        let p = |f: &str| format!("constraints[{i}].{f}");
        if let Some(n) = &c.name {
            let before = e.len();
            check_ident(&p("name"), n, &mut e);
            if e.len() == before && d.constraints[..i].iter().any(|x| x.name.as_ref() == Some(n)) {
                e.push(err(&p("name"), DUP_NAME_MSG));
            }
        }
        check_free_sql(&p("expression"), &c.expression, &mut e);
        if c.not_valid {
            e.push(err(&p("notValid"), NOT_VALID_CREATE_MSG));
        }
    }
    if e.is_empty() && !composed_is_clean(&define_sql(d)) {
        // Olmaması gerekir (kabul edilen her parça sarıldığında temizdir); yine de hatayı parçaya bağla.
        let mut located = false;
        if let Some(x) = &d.default
            && !fragment_clean(x)
        {
            located = true;
            e.push(err("default", COMPOSED_BROKEN));
        }
        for (i, c) in d.constraints.iter().enumerate() {
            if !fragment_clean(&c.expression) {
                located = true;
                e.push(err(&format!("constraints[{i}].expression"), COMPOSED_BROKEN));
            }
        }
        if !located {
            e.push(err(if d.default.is_some() { "default" } else { "constraints" }, COMPOSED_BROKEN));
        }
    }
    e
}

/// Sıra: CREATE, COMMENT (doluysa), OWNER (doluysa, **en sonda**).
pub fn create(ctx: &BuildCtx, d: &DomainDef) -> Result<Plan, Vec<FieldError>> {
    let errs = validate(ctx, d);
    if !errs.is_empty() {
        return Err(errs);
    }
    let dom = domain_sql(&d.schema, &d.name);
    let mut statements = vec![stmt(define_sql(d), Risk::Safe, None)];
    if d.comment.is_some() {
        statements.push(stmt(comment_sql(&dom, &d.comment), Risk::Safe, None));
    }
    if let Some(o) = &d.owner {
        statements.push(stmt(owner_sql(&dom, o), Risk::Safe, None));
    }
    Ok(Plan { statements, warnings: vec![] })
}

/// Eşlenmiş yüklü kısıt: (yeni indeks, eski indeks, ifade değişti mi).
struct Matched {
    ni: usize,
    oi: usize,
    expr_changed: bool,
}

/// Kısıt farkı doğrulaması ve eşlemesi. Eski taraf `alter` başında denetlenmiştir (adlar dolu ve tekil).
fn match_constraints(old: &[DomainCheck], new: &[DomainCheck], e: &mut Vec<FieldError>) -> Vec<Matched> {
    let loaded: Vec<&str> = old.iter().map(|c| c.name.as_deref().unwrap_or_default()).collect();
    let mut matched: Vec<Matched> = vec![];
    for (i, c) in new.iter().enumerate() {
        let p = |f: &str| format!("constraints[{i}].{f}");
        let before = e.len();
        match c.original.as_deref() {
            Some(o) => match loaded.iter().position(|l| *l == o) {
                None => e.push(err(&p("original"), "Yüklü kısıt bulunamadı.")),
                Some(oi) if matched.iter().any(|m| m.oi == oi) => {
                    e.push(err(&p("original"), "Yüklü kısıt birden çok kez eşlenmiş."))
                }
                Some(oi) => {
                    let oc = &old[oi];
                    match c.name.as_deref() {
                        None => e.push(err(&p("name"), NAME_REQUIRED_MSG)),
                        Some(n) if n != o => {
                            let b = e.len();
                            check_ident(&p("name"), n, e);
                            if e.len() == b && loaded.contains(&n) {
                                e.push(err(&p("name"), RENAME_CLASH_MSG));
                            }
                        }
                        Some(_) => {}
                    }
                    let expr_changed = oc.expression != c.expression;
                    if expr_changed {
                        check_free_sql(&p("expression"), &c.expression, e);
                    } else if c.not_valid != oc.not_valid {
                        e.push(err(&p("notValid"), NOT_VALID_LOADED_MSG));
                    }
                    if c.validate && (oc.validated || expr_changed) {
                        e.push(err(&p("validate"), VALIDATE_MSG));
                    }
                    matched.push(Matched { ni: i, oi, expr_changed });
                }
            },
            None => {
                match &c.name {
                    None => e.push(err(&p("name"), NAME_REQUIRED_MSG)),
                    Some(n) => check_ident(&p("name"), n, e),
                }
                check_free_sql(&p("expression"), &c.expression, e);
                if c.validate {
                    e.push(err(&p("validate"), VALIDATE_MSG));
                }
            }
        }
        if e.len() == before
            && let Some(n) = &c.name
            && new[..i].iter().any(|x| x.name.as_ref() == Some(n))
        {
            e.push(err(&p("name"), DUP_NAME_MSG));
        }
    }
    matched
}

/// Sıra (Global Constraints): RENAME → SET SCHEMA → SET/DROP DEFAULT → DROP NOT NULL → kısıt RENAME, DROP, ADD,
/// VALIDATE → SET NOT NULL → COMMENT → OWNER TO (en sonda). Yalnızca değişen ya da ifadeye yazılan alanlar doğrulanır.
pub fn alter(_ctx: &BuildCtx, old: &DomainDef, new: &DomainDef) -> Result<Plan, Vec<FieldError>> {
    let mut e = vec![];
    check_old_schema_ident("old.schema", &old.schema, &mut e);
    check_old_ident("old.name", &old.name, &mut e);
    for (i, c) in old.constraints.iter().enumerate() {
        let path = format!("old.constraints[{i}].name");
        match &c.name {
            // Yükleyici adı her zaman doldurur (`pg_constraint.conname`).
            None => e.push(err(&path, "Yüklü kısıtın adı yok.")),
            Some(n) => {
                let before = e.len();
                check_old_ident(&path, n, &mut e);
                if e.len() == before && old.constraints[..i].iter().any(|x| x.name.as_ref() == Some(n)) {
                    e.push(err(&path, DUP_NAME_MSG));
                }
            }
        }
    }
    if !e.is_empty() {
        return Err(e);
    }
    let name_changed = old.name != new.name;
    let schema_changed = old.schema != new.schema;
    let owner_changed = new.owner.is_some() && new.owner != old.owner;
    let comment_changed = old.comment != new.comment;
    let type_changed = canonical_type(&old.base_type) != canonical_type(&new.base_type);
    let collation_changed = old.collation != new.collation;
    let default_changed = old.default != new.default;
    let not_null_changed = old.not_null != new.not_null;
    let constraints_changed = old.constraints != new.constraints;
    if !old.editable
        && (name_changed
            || schema_changed
            || owner_changed
            || comment_changed
            || old.base_type != new.base_type
            || collation_changed
            || default_changed
            || not_null_changed
            || constraints_changed)
    {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    if type_changed {
        e.push(err("baseType", CREATE_ONLY_MSG));
    }
    if collation_changed {
        e.push(err("collation", CREATE_ONLY_MSG));
    }
    if name_changed {
        check_ident("name", &new.name, &mut e);
    }
    if schema_changed {
        check_schema_ident("schema", &new.schema, &mut e);
    }
    if let Some(o) = &new.owner
        && owner_changed
    {
        check_ident("owner", o, &mut e);
    }
    if let Some(c) = &new.comment
        && comment_changed
    {
        check_text("comment", c, &mut e);
    }
    if let Some(x) = &new.default
        && default_changed
    {
        check_free_sql("default", x, &mut e);
    }
    let matched = match_constraints(&old.constraints, &new.constraints, &mut e);
    if !e.is_empty() {
        return Err(e);
    }

    // Gövde ifadeleri yeni ad ve şemayı hedefler (RENAME ve SET SCHEMA önce çalışır).
    let dom = domain_sql(&new.schema, &new.name);
    // (serbest SQL alan yolu, ifade): nihai ifade taraması için.
    let mut out: Vec<(Option<String>, Statement)> = vec![];
    if name_changed {
        let sql = format!("ALTER DOMAIN {} RENAME TO {}", domain_sql(&old.schema, &old.name), quote_ident(&new.name));
        out.push((None, stmt(sql, Risk::Safe, None)));
    }
    if schema_changed {
        let sql = format!("ALTER DOMAIN {} SET SCHEMA {}", domain_sql(&old.schema, &new.name), quote_ident(&new.schema));
        out.push((None, stmt(sql, Risk::Safe, None)));
    }
    if default_changed {
        match &new.default {
            Some(x) => out.push((
                Some("default".into()),
                stmt(format!("ALTER DOMAIN {dom} SET DEFAULT {}", wrap(x)), Risk::Safe, None),
            )),
            None => out.push((None, stmt(format!("ALTER DOMAIN {dom} DROP DEFAULT"), Risk::Safe, None))),
        }
    }
    if not_null_changed && !new.not_null {
        out.push((None, stmt(format!("ALTER DOMAIN {dom} DROP NOT NULL"), Risk::Safe, None)));
    }
    let old_name = |oi: usize| old.constraints[oi].name.as_deref().unwrap_or_default();
    let new_name = |ni: usize| new.constraints[ni].name.as_deref().unwrap_or_default();
    for m in matched.iter().filter(|m| !m.expr_changed && old_name(m.oi) != new_name(m.ni)) {
        let sql = format!("ALTER DOMAIN {dom} RENAME CONSTRAINT {} TO {}", quote_ident(old_name(m.oi)), quote_ident(new_name(m.ni)));
        out.push((None, stmt(sql, Risk::Safe, None)));
    }
    for oi in 0..old.constraints.len() {
        let note = match matched.iter().find(|m| m.oi == oi) {
            None => None,
            Some(m) if m.expr_changed => Some(EXPR_CHANGE_NOTE),
            Some(_) => continue,
        };
        let sql = format!("ALTER DOMAIN {dom} DROP CONSTRAINT {}", quote_ident(old_name(oi)));
        out.push((None, stmt(sql, Risk::Destructive, note)));
    }
    for (ni, c) in new.constraints.iter().enumerate() {
        let added = match matched.iter().find(|m| m.ni == ni) {
            None => true,
            Some(m) => m.expr_changed,
        };
        if added {
            let note = if c.not_valid { None } else { Some(ADD_CHECK_NOTE) };
            out.push((
                Some(format!("constraints[{ni}].expression")),
                stmt(format!("ALTER DOMAIN {dom} ADD {}", check_sql(c)), Risk::Safe, note),
            ));
        }
    }
    for m in matched.iter().filter(|m| new.constraints[m.ni].validate) {
        let sql = format!("ALTER DOMAIN {dom} VALIDATE CONSTRAINT {}", quote_ident(new_name(m.ni)));
        out.push((None, stmt(sql, Risk::Safe, None)));
    }
    if not_null_changed && new.not_null {
        out.push((None, stmt(format!("ALTER DOMAIN {dom} SET NOT NULL"), Risk::Destructive, Some(SET_NOT_NULL_NOTE))));
    }
    if comment_changed {
        out.push((None, stmt(comment_sql(&dom, &new.comment), Risk::Safe, None)));
    }
    // OWNER TO en sonda: kendisi eski sahibin haklarını ister (View kuralı).
    if let Some(o) = &new.owner
        && owner_changed
    {
        out.push((None, stmt(owner_sql(&dom, o), Risk::Safe, None)));
    }

    for (path, s) in &out {
        if let Some(p) = path
            && !composed_is_clean(&s.sql)
        {
            e.push(err(p, COMPOSED_BROKEN));
        }
    }
    if !e.is_empty() {
        return Err(e);
    }
    Ok(Plan { statements: out.into_iter().map(|(_, s)| s).collect(), warnings: vec![] })
}

/// `DROP DOMAIN <q> [CASCADE]`; kimlik dağıtıcıda (`validate_identity`) doğrulanır. Salt okunur silinemez.
pub fn drop(d: &DomainDef, cascade: bool) -> Result<Plan, Vec<FieldError>> {
    if !d.editable {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    let sql = format!("DROP DOMAIN {}", domain_sql(&d.schema, &d.name));
    Ok(if cascade {
        Plan {
            statements: vec![stmt(format!("{sql} CASCADE"), Risk::DataLoss, Some(CASCADE_WARNING))],
            warnings: vec![CASCADE_WARNING.into()],
        }
    } else {
        Plan { statements: vec![stmt(sql, Risk::Destructive, None)], warnings: vec![] }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::freesql::composed_is_clean;
    use crate::build::{alter as balter, assert_single_statements, create as bcreate, drop as bdrop, render};
    use crate::model::{DomainCheck, ObjectDef};
    use crate::plan::Risk;
    use crate::quote::{qualified, quote_ident};

    fn ctx() -> BuildCtx {
        BuildCtx { version_num: 160000, ..Default::default() }
    }
    fn o(d: &DomainDef) -> ObjectDef {
        ObjectDef::Domain(d.clone())
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn paths(errs: Vec<FieldError>) -> Vec<String> {
        errs.into_iter().map(|e| e.path).collect()
    }
    fn dom() -> DomainDef {
        DomainDef {
            schema: "public".into(),
            name: "d".into(),
            owner: None,
            comment: None,
            base_type: "integer".into(),
            collation: None,
            default: None,
            not_null: false,
            constraints: vec![],
            editable: true,
            unsupported: vec![],
        }
    }
    fn with(constraints: Vec<DomainCheck>) -> DomainDef {
        DomainDef { constraints, ..dom() }
    }
    /// Yeni adlı kısıt.
    fn nc(name: &str, expr: &str) -> DomainCheck {
        DomainCheck { name: Some(name.into()), expression: expr.into(), not_valid: false, validated: true, original: None, validate: false }
    }
    /// Yeni adsız kısıt.
    fn uc(expr: &str) -> DomainCheck {
        DomainCheck { name: None, ..nc("", expr) }
    }
    /// Yüklü kısıt (`original = name`).
    fn lc(name: &str, expr: &str) -> DomainCheck {
        DomainCheck { original: Some(name.into()), ..nc(name, expr) }
    }
    /// Yüklü, doğrulanmamış kısıt.
    fn lcu(name: &str, expr: &str) -> DomainCheck {
        DomainCheck { validated: false, ..lc(name, expr) }
    }
    fn create_ok(d: &DomainDef) -> Plan {
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert_single_statements(&p);
        assert!(p.statements.iter().all(|s| composed_is_clean(&s.sql)));
        p
    }
    fn alter_ok(old: &DomainDef, new: &DomainDef) -> Plan {
        let p = balter(&o(old), &o(new), &ctx()).unwrap();
        assert_single_statements(&p);
        assert!(p.statements.iter().all(|s| composed_is_clean(&s.sql)));
        p
    }
    fn alter_err(old: &DomainDef, new: &DomainDef) -> Vec<String> {
        paths(balter(&o(old), &o(new), &ctx()).unwrap_err())
    }

    #[test]
    fn create_variants() {
        let full = DomainDef {
            base_type: "character varying(20)".into(),
            collation: Some("C".into()),
            default: Some("'x'".into()),
            not_null: true,
            constraints: vec![nc("uzun", "length(VALUE) > 1"), uc("VALUE <> 'y'"), nc("Büyük Ad", "VALUE ~ '^[a-z]' -- yorum\n")],
            ..dom()
        };
        let cases: Vec<(&str, DomainDef)> = vec![
            ("yalın", dom()),
            ("tam", full.clone()),
            ("yorum + sahip", DomainDef { comment: Some("it's".into()), owner: Some("Sahip".into()), ..dom() }),
            ("dizi tipi + yalnız NOT NULL", DomainDef { base_type: "numeric(10,2)[]".into(), not_null: true, ..dom() }),
            ("yalnız adsız kısıt", with(vec![uc("VALUE > 0")])),
        ];
        let mut out = String::new();
        for (label, d) in cases {
            let p = create_ok(&d);
            assert!(p.statements.iter().all(|s| s.risk == Risk::Safe && s.transactional), "{label}");
            assert!(p.warnings.is_empty(), "{label}");
            out.push_str(&format!("# {label}\n{}\n", render(&p)));
        }
        insta::assert_snapshot!(out);
        assert_eq!(sqls(&create_ok(&dom())), ["CREATE DOMAIN public.d AS integer"]);
        assert_eq!(
            sqls(&create_ok(&full)),
            ["CREATE DOMAIN public.d AS character varying(20) COLLATE \"C\" DEFAULT ('x'\n) NOT NULL \
              CONSTRAINT uzun CHECK (length(VALUE) > 1\n) CHECK (VALUE <> 'y'\n) \
              CONSTRAINT \"Büyük Ad\" CHECK (VALUE ~ '^[a-z]' -- yorum\n\n)"]
        );
        assert_eq!(
            sqls(&create_ok(&DomainDef { comment: Some("c".into()), owner: Some("r".into()), ..dom() })),
            ["CREATE DOMAIN public.d AS integer", "COMMENT ON DOMAIN public.d IS 'c'", "ALTER DOMAIN public.d OWNER TO r"]
        );
    }

    #[test]
    fn create_rule_violations() {
        let cases: Vec<(&str, DomainDef, &str)> = vec![
            ("tip boş", DomainDef { base_type: String::new(), ..dom() }, "baseType"),
            ("tip enjeksiyon", DomainDef { base_type: "int; DROP TABLE x".into(), ..dom() }, "baseType"),
            ("tip NOT NULL", DomainDef { base_type: "int not null".into(), ..dom() }, "baseType"),
            ("collation boş", DomainDef { collation: Some(String::new()), ..dom() }, "collation"),
            ("collation satır sonu", DomainDef { collation: Some("C\n".into()), ..dom() }, "collation"),
            ("default boş", DomainDef { default: Some(" ".into()), ..dom() }, "default"),
            ("default ;", DomainDef { default: Some("1; DROP TABLE x".into()), ..dom() }, "default"),
            ("default )", DomainDef { default: Some("1) NOT NULL CHECK (true".into()), ..dom() }, "default"),
            ("default --", DomainDef { default: Some("1 --".into()), ..dom() }, "default"),
            ("default ters bölü", DomainDef { default: Some(r"'a\b'".into()), ..dom() }, "default"),
            ("default NUL", DomainDef { default: Some("1\0".into()), ..dom() }, "default"),
            ("ifade boş", with(vec![nc("c", "")]), "constraints[0].expression"),
            ("ifade ;", with(vec![nc("a", "true"), nc("c", "VALUE > 0; DROP TABLE x")]), "constraints[1].expression"),
            ("ifade )", with(vec![uc("VALUE > 0) NOT NULL CHECK (true")]), "constraints[0].expression"),
            ("ifade --", with(vec![uc("VALUE > 0 -- x")]), "constraints[0].expression"),
            ("ifade kapanmamış", with(vec![uc("VALUE <> 'x")]), "constraints[0].expression"),
            ("NOT VALID create'te", with(vec![DomainCheck { not_valid: true, ..nc("c", "true") }]), "constraints[0].notValid"),
            ("kısıt adı boş", with(vec![nc("", "true")]), "constraints[0].name"),
            ("kısıt adı 64 bayt", with(vec![nc(&"a".repeat(64), "true")]), "constraints[0].name"),
            ("kısıt adı tekrar", with(vec![nc("c", "true"), uc("true"), nc("c", "false")]), "constraints[2].name"),
            ("ad boş", DomainDef { name: String::new(), ..dom() }, "name"),
            ("şema sistem", DomainDef { schema: "pg_catalog".into(), ..dom() }, "schema"),
            ("sahip boş", DomainDef { owner: Some(String::new()), ..dom() }, "owner"),
            ("yorum NUL", DomainDef { comment: Some("a\0".into()), ..dom() }, "comment"),
        ];
        let mut out = String::new();
        for (label, d, want) in cases {
            assert_eq!(paths(bcreate(&o(&d), &ctx()).unwrap_err()), [want], "{label}");
            out.push_str(&format!("{label}: {want}\n"));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn alter_single_changes() {
        let old = DomainDef { default: Some("0".into()), constraints: vec![lc("pos", "VALUE > 0"), lcu("kucuk", "VALUE < 100")], ..dom() };
        let c = |cs: Vec<DomainCheck>| DomainDef { constraints: cs, ..old.clone() };
        let keep = || vec![lc("pos", "VALUE > 0"), lcu("kucuk", "VALUE < 100")];
        let cases: Vec<(&str, DomainDef, Vec<&str>, Risk)> = vec![
            ("default değiştir", DomainDef { default: Some("1 + 1".into()), ..old.clone() }, vec!["ALTER DOMAIN public.d SET DEFAULT (1 + 1\n)"], Risk::Safe),
            ("default kaldır", DomainDef { default: None, ..old.clone() }, vec!["ALTER DOMAIN public.d DROP DEFAULT"], Risk::Safe),
            ("not null aç", DomainDef { not_null: true, ..old.clone() }, vec!["ALTER DOMAIN public.d SET NOT NULL"], Risk::Destructive),
            (
                "kısıt ekle",
                c([keep(), vec![nc("yeni", "VALUE <> 5")]].concat()),
                vec!["ALTER DOMAIN public.d ADD CONSTRAINT yeni CHECK (VALUE <> 5\n)"],
                Risk::Safe,
            ),
            (
                "kısıt ekle NOT VALID",
                c([keep(), vec![DomainCheck { not_valid: true, ..nc("yeni", "VALUE <> 5") }]].concat()),
                vec!["ALTER DOMAIN public.d ADD CONSTRAINT yeni CHECK (VALUE <> 5\n) NOT VALID"],
                Risk::Safe,
            ),
            (
                "kısıt yeniden adlandır",
                c(vec![DomainCheck { name: Some("Pozitif".into()), ..lc("pos", "VALUE > 0") }, lcu("kucuk", "VALUE < 100")]),
                vec!["ALTER DOMAIN public.d RENAME CONSTRAINT pos TO \"Pozitif\""],
                Risk::Safe,
            ),
            (
                "ifade değiştir",
                c(vec![lc("pos", "VALUE >= 0"), lcu("kucuk", "VALUE < 100")]),
                vec!["ALTER DOMAIN public.d DROP CONSTRAINT pos", "ALTER DOMAIN public.d ADD CONSTRAINT pos CHECK (VALUE >= 0\n)"],
                Risk::Destructive,
            ),
            ("kısıt sil", c(vec![lcu("kucuk", "VALUE < 100")]), vec!["ALTER DOMAIN public.d DROP CONSTRAINT pos"], Risk::Destructive),
            (
                "doğrula",
                c(vec![lc("pos", "VALUE > 0"), DomainCheck { validate: true, ..lcu("kucuk", "VALUE < 100") }]),
                vec!["ALTER DOMAIN public.d VALIDATE CONSTRAINT kucuk"],
                Risk::Safe,
            ),
            // Kanonik olarak eşit temel tip değişim sayılmaz.
            ("kanonik eşit tip", DomainDef { base_type: "int4".into(), ..old.clone() }, vec![], Risk::Safe),
        ];
        let mut out = String::new();
        for (label, new, want, risk) in cases {
            let pl = alter_ok(&old, &new);
            assert_eq!(sqls(&pl), want, "{label}");
            assert_eq!(pl.risk(), risk, "{label}");
            assert!(pl.statements.iter().all(|s| s.transactional), "{label}");
            out.push_str(&format!("# {label}\n{}\n", render(&pl)));
        }
        insta::assert_snapshot!(out);
        // NOT NULL kapat: Safe.
        let nn = DomainDef { not_null: true, ..dom() };
        let pl = alter_ok(&nn, &dom());
        assert_eq!(sqls(&pl), ["ALTER DOMAIN public.d DROP NOT NULL"]);
        assert_eq!(pl.risk(), Risk::Safe);
        // Varsayılansızdan varsayılana.
        assert_eq!(sqls(&alter_ok(&dom(), &DomainDef { default: Some("now()".into()), ..dom() })), ["ALTER DOMAIN public.d SET DEFAULT (now()\n)"]);
        // Notlar: ADD (NOT VALID'siz) kullanan kolonları tarar; NOT VALID notsuz; SET NOT NULL notlu; ifade değişimi DROP notlu.
        let add = alter_ok(&dom(), &with(vec![nc("a", "true")]));
        assert_eq!(add.statements[0].note.as_deref(), Some(ADD_CHECK_NOTE));
        let add_nv = alter_ok(&dom(), &with(vec![DomainCheck { not_valid: true, ..nc("a", "true") }]));
        assert_eq!(add_nv.statements[0].note, None);
        assert_eq!(alter_ok(&dom(), &nn).statements[0].note.as_deref(), Some(SET_NOT_NULL_NOTE));
        let ch = alter_ok(&with(vec![lc("a", "true")]), &with(vec![lc("a", "false")]));
        assert_eq!(ch.statements[0].note.as_deref(), Some(EXPR_CHANGE_NOTE));
        assert_eq!(ch.statements[1].note.as_deref(), Some(ADD_CHECK_NOTE));
        // İfade değişimi + yeniden adlandırma: RENAME yok; eski adla DROP, yeni adla ADD.
        let pl = alter_ok(&with(vec![lc("a", "true")]), &with(vec![DomainCheck { name: Some("b".into()), ..lc("a", "false") }]));
        assert_eq!(sqls(&pl), ["ALTER DOMAIN public.d DROP CONSTRAINT a", "ALTER DOMAIN public.d ADD CONSTRAINT b CHECK (false\n)"]);
        // İfadesi değişen kısıt NOT VALID ile yeniden eklenebilir (yeni ADD).
        let pl = alter_ok(&with(vec![lc("a", "true")]), &with(vec![DomainCheck { not_valid: true, ..lc("a", "false") }]));
        assert_eq!(pl.statements[1].sql, "ALTER DOMAIN public.d ADD CONSTRAINT a CHECK (false\n) NOT VALID");
        // Silinen adı yeni kısıt devralabilir (DROP, ADD'den önce çalışır).
        let pl = alter_ok(&with(vec![lc("a", "true")]), &with(vec![nc("a", "false")]));
        assert_eq!(sqls(&pl), ["ALTER DOMAIN public.d DROP CONSTRAINT a", "ALTER DOMAIN public.d ADD CONSTRAINT a CHECK (false\n)"]);
        // Yeniden adlandırılan kısıt doğrulanır: VALIDATE yeni adla.
        let pl = alter_ok(&with(vec![lcu("a", "true")]), &with(vec![DomainCheck { name: Some("b".into()), validate: true, ..lcu("a", "true") }]));
        assert_eq!(sqls(&pl), ["ALTER DOMAIN public.d RENAME CONSTRAINT a TO b", "ALTER DOMAIN public.d VALIDATE CONSTRAINT b"]);
    }

    #[test]
    fn alter_combined_order() {
        let old = DomainDef {
            not_null: true,
            default: Some("0".into()),
            comment: Some("c".into()),
            constraints: vec![lc("a", "VALUE > 0"), lc("b", "VALUE < 10"), lcu("c", "VALUE <> 5"), lc("d", "true")],
            ..dom()
        };
        let mid = DomainDef { not_null: false, ..old.clone() };
        let new = DomainDef {
            name: "d2".into(),
            schema: "app".into(),
            default: None,
            comment: None,
            owner: Some("r".into()),
            constraints: vec![
                DomainCheck { name: Some("a2".into()), ..lc("a", "VALUE > 0") },
                lc("b", "VALUE < 20"),
                DomainCheck { validate: true, ..lcu("c", "VALUE <> 5") },
                nc("e", "VALUE <> 7"),
            ],
            ..mid.clone()
        };
        let pl = alter_ok(&old, &new);
        assert_eq!(
            sqls(&pl),
            [
                "ALTER DOMAIN public.d RENAME TO d2",
                "ALTER DOMAIN public.d2 SET SCHEMA app",
                "ALTER DOMAIN app.d2 DROP DEFAULT",
                "ALTER DOMAIN app.d2 DROP NOT NULL",
                "ALTER DOMAIN app.d2 RENAME CONSTRAINT a TO a2",
                "ALTER DOMAIN app.d2 DROP CONSTRAINT b",
                "ALTER DOMAIN app.d2 DROP CONSTRAINT d",
                "ALTER DOMAIN app.d2 ADD CONSTRAINT b CHECK (VALUE < 20\n)",
                "ALTER DOMAIN app.d2 ADD CONSTRAINT e CHECK (VALUE <> 7\n)",
                "ALTER DOMAIN app.d2 VALIDATE CONSTRAINT c",
                "COMMENT ON DOMAIN app.d2 IS NULL",
                "ALTER DOMAIN app.d2 OWNER TO r",
            ]
        );
        assert_eq!(pl.risk(), Risk::Destructive);
        insta::assert_snapshot!(render(&pl));
        // SET NOT NULL kısıtlardan sonra, COMMENT'ten önce.
        let pl = alter_ok(&dom(), &DomainDef { not_null: true, comment: Some("k".into()), default: Some("1".into()), ..with(vec![nc("a", "true")]) });
        assert_eq!(
            sqls(&pl),
            [
                "ALTER DOMAIN public.d SET DEFAULT (1\n)",
                "ALTER DOMAIN public.d ADD CONSTRAINT a CHECK (true\n)",
                "ALTER DOMAIN public.d SET NOT NULL",
                "COMMENT ON DOMAIN public.d IS 'k'",
            ]
        );
    }

    #[test]
    fn alter_errors() {
        let old = DomainDef {
            base_type: "text".into(),
            collation: Some("C".into()),
            constraints: vec![lc("a", "true"), lcu("b", "VALUE <> ''")],
            ..dom()
        };
        let c = |cs: Vec<DomainCheck>| DomainDef { constraints: cs, ..old.clone() };
        let cases: Vec<(&str, DomainDef, &str)> = vec![
            ("temel tip değişimi", DomainDef { base_type: "varchar(10)".into(), ..old.clone() }, "baseType"),
            ("collation değişimi", DomainDef { collation: Some("POSIX".into()), ..old.clone() }, "collation"),
            ("collation kaldırma", DomainDef { collation: None, ..old.clone() }, "collation"),
            ("eklenen kısıt adsız", c(vec![lc("a", "true"), lcu("b", "VALUE <> ''"), uc("true")]), "constraints[2].name"),
            ("yüklü kısıt adı boşaltılmış", c(vec![DomainCheck { name: None, ..lc("a", "true") }, lcu("b", "VALUE <> ''")]), "constraints[0].name"),
            ("bilinmeyen original", c(vec![lc("a", "true"), lcu("b", "VALUE <> ''"), lc("zz", "true")]), "constraints[2].original"),
            ("original tekrar", c(vec![lc("a", "true"), DomainCheck { name: Some("x".into()), ..lc("a", "true") }, lcu("b", "VALUE <> ''")]), "constraints[1].original"),
            ("yeniden adlandırma çakışması", c(vec![DomainCheck { name: Some("b".into()), ..lc("a", "true") }]), "constraints[0].name"),
            ("ad tekrar", c(vec![lc("a", "true"), lcu("b", "VALUE <> ''"), nc("a", "false")]), "constraints[2].name"),
            ("yeniden adlandırma geçersiz", c(vec![DomainCheck { name: Some("x\n".into()), ..lc("a", "true") }, lcu("b", "VALUE <> ''")]), "constraints[0].name"),
            ("yeni ad geçersiz", c(vec![lc("a", "true"), lcu("b", "VALUE <> ''"), nc("", "true")]), "constraints[2].name"),
            ("NOT VALID yüklü kısıtta", c(vec![DomainCheck { not_valid: true, ..lc("a", "true") }, lcu("b", "VALUE <> ''")]), "constraints[0].notValid"),
            ("doğrula yeni kısıtta", c(vec![lc("a", "true"), lcu("b", "VALUE <> ''"), DomainCheck { validate: true, ..nc("x", "true") }]), "constraints[2].validate"),
            ("doğrula doğrulanmış kısıtta", c(vec![DomainCheck { validate: true, ..lc("a", "true") }, lcu("b", "VALUE <> ''")]), "constraints[0].validate"),
            ("doğrula ifadesi değişen kısıtta", c(vec![lc("a", "true"), DomainCheck { validate: true, ..lcu("b", "VALUE <> 'x'") }]), "constraints[1].validate"),
            ("default ;", DomainDef { default: Some("1; DROP TABLE x".into()), ..old.clone() }, "default"),
            ("default )", DomainDef { default: Some("1) x (".into()), ..old.clone() }, "default"),
            ("default --", DomainDef { default: Some("1 -- x".into()), ..old.clone() }, "default"),
            ("yeni ifade ;", c(vec![lc("a", "true"), lcu("b", "VALUE <> ''"), nc("x", "true; DROP TABLE t")]), "constraints[2].expression"),
            ("değişen ifade )", c(vec![lc("a", "true) NOT VALID"), lcu("b", "VALUE <> ''")]), "constraints[0].expression"),
            ("değişen ifade --", c(vec![lc("a", "true --"), lcu("b", "VALUE <> ''")]), "constraints[0].expression"),
        ];
        let mut out = String::new();
        for (label, new, want) in cases {
            let e = balter(&o(&old), &o(&new), &ctx()).unwrap_err();
            assert_eq!(e[0].path, want, "{label}: {e:?}");
            let all: Vec<String> = e.iter().map(|x| format!("{} ({})", x.path, x.message)).collect();
            out.push_str(&format!("{label}: {}\n", all.join("; ")));
        }
        insta::assert_snapshot!(out);
        assert_eq!(balter(&o(&old), &o(&DomainDef { base_type: "int".into(), ..old.clone() }), &ctx()).unwrap_err()[0].message, CREATE_ONLY_MSG);
        // Yüklü kısıtın adı yoksa (yükleyici doldurmalı) eski taraf hatası.
        let unnamed = with(vec![DomainCheck { name: None, ..lc("a", "true") }]);
        assert_eq!(alter_err(&unnamed, &unnamed), ["old.constraints[0].name"]);
        assert_eq!(alter_err(&with(vec![lc("a\0", "true")]), &with(vec![lc("a\0", "true")])), ["old.constraints[0].name"]);
        // Yüklü kısıt adları tekil olmalı (eski taraf).
        assert_eq!(alter_err(&with(vec![lc("a", "true"), lc("a", "false")]), &with(vec![lc("a", "true")])), ["old.constraints[1].name"]);
    }

    #[test]
    fn read_only_rejects_changes_and_drop() {
        let ro = DomainDef { editable: false, unsupported: vec!["eklenti".into()], ..with(vec![lc("a", "true")]) };
        assert!(alter_ok(&ro, &ro).statements.is_empty());
        for new in [
            DomainDef { name: "x".into(), ..ro.clone() },
            DomainDef { schema: "app".into(), ..ro.clone() },
            DomainDef { owner: Some("r".into()), ..ro.clone() },
            DomainDef { comment: Some("k".into()), ..ro.clone() },
            DomainDef { default: Some("1".into()), ..ro.clone() },
            DomainDef { not_null: true, ..ro.clone() },
            DomainDef { constraints: vec![], ..ro.clone() },
            DomainDef { base_type: "bigint".into(), ..ro.clone() },
        ] {
            let e = balter(&o(&ro), &o(&new), &ctx()).unwrap_err();
            assert_eq!(paths(e.clone()), ["object"]);
            assert_eq!(e[0].message, READ_ONLY_MSG);
        }
        assert_eq!(paths(bdrop(&o(&ro), false, &ctx()).unwrap_err()), ["object"]);
        assert_eq!(paths(bdrop(&o(&ro), true, &ctx()).unwrap_err()), ["object"]);
    }

    #[test]
    fn drop_matrix() {
        let mut out = String::new();
        for (d, cascade, want, risk) in [
            (dom(), false, "DROP DOMAIN public.d", Risk::Destructive),
            (dom(), true, "DROP DOMAIN public.d CASCADE", Risk::DataLoss),
            (DomainDef { schema: "Ş".into(), name: "My \"D\"".into(), ..dom() }, false, "DROP DOMAIN \"Ş\".\"My \"\"D\"\"\"", Risk::Destructive),
        ] {
            let pl = bdrop(&o(&d), cascade, &ctx()).unwrap();
            assert_single_statements(&pl);
            assert_eq!(sqls(&pl), [want]);
            assert_eq!(pl.statements[0].risk, risk);
            assert_eq!(pl.warnings.is_empty(), !cascade);
            out.push_str(&render(&pl));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
        // Gövde doğrulanmaz (yalnız kimlik); sistem şeması reddedilir.
        assert!(bdrop(&o(&DomainDef { base_type: String::new(), default: Some(";".into()), ..dom() }), false, &ctx()).is_ok());
        assert_eq!(paths(bdrop(&o(&DomainDef { schema: "pg_catalog".into(), ..dom() }), false, &ctx()).unwrap_err()), ["schema"]);
    }

    #[test]
    fn hostile_expressions_stay_single_statement() {
        // Kapıdan geçen düşmanca ama meşru ifadeler: tek ifade kalır, alan baytı baytına korunur.
        for x in [
            "VALUE = ';'",
            "VALUE <> 'it''s; DROP TABLE x; --'",
            "VALUE = $$;)$$",
            "VALUE = E'\\'; x'",
            "VALUE /* ; ) */ > 1",
            "VALUE > 0 -- yorum\n",
            "VALUE ~ 'a' -- ) ; DROP\nAND VALUE <> ''",
            "f(g(VALUE), (1))",
            "VALUE = \"we;ird\"",
        ] {
            let d = DomainDef { default: Some(x.into()), ..with(vec![nc("c", x), uc(x)]) };
            let pl = create_ok(&d);
            assert!(pl.statements[0].sql.contains(&format!("CHECK ({x}\n)")), "{x:?}");
            assert!(pl.statements[0].sql.contains(&format!("DEFAULT ({x}\n)")), "{x:?}");
            let pl = alter_ok(&dom(), &d.clone().tap_named());
            assert!(pl.statements.iter().any(|s| s.sql.ends_with(&format!("CHECK ({x}\n)"))), "{x:?}");
            let pl = alter_ok(&with(vec![lc("c", "true")]), &DomainDef { default: Some(x.into()), ..with(vec![lc("c", x)]) });
            assert_eq!(pl.statements.len(), 3, "{x:?}");
        }
        // Reddedilenler: `;`, `)`, `--` ile biten, kapanmamış tırnak/yorum, rakam bitişik tırnak.
        for x in ["1; DROP TABLE t", "1) CHECK (true", "1 --", "'x", "/* x", "1'x'", "x)); DROP DOMAIN d; --"] {
            for d in [
                DomainDef { default: Some(x.into()), ..dom() },
                with(vec![nc("c", x)]),
                with(vec![uc(x)]),
            ] {
                assert!(bcreate(&o(&d), &ctx()).is_err(), "{x:?}");
            }
            assert_eq!(alter_err(&dom(), &DomainDef { default: Some(x.into()), ..dom() }), ["default"], "{x:?}");
            assert_eq!(alter_err(&dom(), &with(vec![nc("c", x)])), ["constraints[0].expression"], "{x:?}");
            assert_eq!(alter_err(&with(vec![lc("c", "true")]), &with(vec![lc("c", x)])), ["constraints[0].expression"], "{x:?}");
        }
    }

    trait TapNamed {
        /// Adsız kısıtlara ad verir (alter'da eklenen kısıt adı zorunludur).
        fn tap_named(self) -> Self;
    }
    impl TapNamed for DomainDef {
        fn tap_named(mut self) -> Self {
            for (i, c) in self.constraints.iter_mut().enumerate() {
                if c.name.is_none() {
                    c.name = Some(format!("k{i}"));
                }
            }
            self
        }
    }

    #[test]
    fn hostile_names_stay_single_statement() {
        let long63 = "a".repeat(63);
        for n in ["t\"; DROP TABLE x; --", "select", "Alan ş", "日本", long63.as_str(), "'", "\\", "/*", "$$", "$pgt$"] {
            let short = &n[..n.len().min(60)];
            let d = DomainDef {
                schema: n.into(),
                name: n.into(),
                owner: Some(n.into()),
                comment: Some(n.into()),
                base_type: "text".into(),
                collation: Some(n.into()),
                constraints: vec![lcu(n, "true")],
                ..dom()
            };
            let new = DomainDef {
                name: format!("{short}x"),
                owner: Some(format!("{short}2")),
                comment: Some(format!("{n}!")),
                constraints: vec![
                    DomainCheck { name: Some(format!("{short}y")), validate: true, ..lcu(n, "true") },
                    nc(&format!("{short}z"), "VALUE <> ''"),
                ],
                ..d.clone()
            };
            for pl in [create_ok(&d), alter_ok(&d, &new), bdrop(&o(&d), true, &ctx()).unwrap(), bdrop(&o(&d), false, &ctx()).unwrap()] {
                assert!(!pl.statements.is_empty(), "{n:?}");
                assert_single_statements(&pl);
            }
            let s = &create_ok(&d).statements[0].sql;
            assert!(s.starts_with(&format!("CREATE DOMAIN {} AS text COLLATE {}", qualified(n, n), quote_ident(n))), "{s}");
            // Yüklü kısıt silinir: eski ad tırnaklı.
            let pl = alter_ok(&d, &DomainDef { constraints: vec![], ..d.clone() });
            assert_eq!(sqls(&pl), [format!("ALTER DOMAIN {} DROP CONSTRAINT {}", qualified(n, n), quote_ident(n))]);
        }
        let pl = create_ok(&DomainDef { name: "t\"; DROP TABLE x; --".into(), ..dom() });
        assert_eq!(pl.statements[0].sql, "CREATE DOMAIN public.\"t\"\"; DROP TABLE x; --\" AS integer");
    }

    #[test]
    fn alter_validates_only_changed_or_written_fields() {
        // Eski taraf kimliği gevşek: sıfır genişlikli ad yeniden adlandırılabilir; yeni ad katı.
        let zw = DomainDef { name: "d\u{200b}".into(), ..dom() };
        assert_eq!(sqls(&alter_ok(&zw, &dom())), ["ALTER DOMAIN public.\"d\u{200b}\" RENAME TO d"]);
        assert_eq!(alter_err(&dom(), &zw), ["name"]);
        assert_eq!(alter_err(&DomainDef { name: String::new(), ..dom() }, &dom()), ["old.name"]);
        assert_eq!(alter_err(&DomainDef { schema: "pg_catalog".into(), ..dom() }, &dom()), ["old.schema"]);
        assert_eq!(alter_err(&dom(), &DomainDef { schema: "pg_toast".into(), ..dom() }), ["schema"]);
        // Değişmeyen yüklü ifade/varsayılan/temel tip doğrulanmaz (ör. `\` içeren `pg_get_constraintdef` çıktısı).
        let odd = DomainDef {
            base_type: "geometry(Point,4326)".into(),
            default: Some(r"'a\b'".into()),
            constraints: vec![lc("c\u{200b}", r"VALUE ~ '\d'"), lc("k", "VALUE <> ''")],
            ..dom()
        };
        assert_eq!(sqls(&alter_ok(&odd, &DomainDef { comment: Some("k".into()), ..odd.clone() })), ["COMMENT ON DOMAIN public.d IS 'k'"]);
        // Değişmeyen gevşek yüklü ad yeniden adlandırılabilir ve silinebilir.
        let ren = DomainDef { constraints: vec![DomainCheck { name: Some("c".into()), ..lc("c\u{200b}", r"VALUE ~ '\d'") }, lc("k", "VALUE <> ''")], ..odd.clone() };
        assert_eq!(sqls(&alter_ok(&odd, &ren)), ["ALTER DOMAIN public.d RENAME CONSTRAINT \"c\u{200b}\" TO c"]);
        let dropped = DomainDef { constraints: vec![lc("k", "VALUE <> ''")], ..odd.clone() };
        assert_eq!(sqls(&alter_ok(&odd, &dropped)), ["ALTER DOMAIN public.d DROP CONSTRAINT \"c\u{200b}\""]);
        // Değişen ifade katı.
        let changed = DomainDef { constraints: vec![lc("c\u{200b}", r"VALUE ~ '\w'"), lc("k", "VALUE <> ''")], ..odd.clone() };
        assert_eq!(alter_err(&odd, &changed), ["constraints[0].expression"]);
        // Değişmeyen NUL'lu yorum yazılmaz; değişen yorum/sahip katı.
        let nul = DomainDef { comment: Some("a\0".into()), ..dom() };
        assert!(balter(&o(&nul), &o(&DomainDef { name: "x".into(), ..nul.clone() }), &ctx()).is_ok());
        assert_eq!(alter_err(&dom(), &DomainDef { comment: Some("b\0".into()), ..dom() }), ["comment"]);
        assert_eq!(alter_err(&dom(), &DomainDef { owner: Some("r\n".into()), ..dom() }), ["owner"]);
        // Sahip None → değişiklik yok.
        let owned = DomainDef { owner: Some("r".into()), ..dom() };
        assert!(alter_ok(&owned, &dom()).statements.is_empty());
    }

    #[test]
    fn alter_noop_is_empty() {
        for d in [
            dom(),
            DomainDef {
                comment: Some("c".into()),
                owner: Some("r".into()),
                base_type: "character varying(20)".into(),
                collation: Some("C".into()),
                default: Some("'x'::character varying".into()),
                not_null: true,
                constraints: vec![lc("a", "(VALUE)::text <> ''::text"), lcu("b", "length((VALUE)::text) > 1")],
                ..dom()
            },
        ] {
            let pl = alter_ok(&d, &d);
            assert!(pl.statements.is_empty() && pl.warnings.is_empty(), "{d:?}");
        }
    }
}
