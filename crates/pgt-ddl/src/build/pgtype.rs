//! Type üreticisi (enum / composite / range): CREATE TYPE, ALTER (RENAME, SET SCHEMA, gövde, COMMENT, OWNER), DROP.
//!
//! - **Enum** (Karar 6): değerler `original` ile eşlenir (`Some` = yüklü etiket, `None` = yeni). Yüklü değer silinemez
//!   ve sırası değişemez (alan hatası); `original ≠ label` → `RENAME VALUE` (PG 10+, taban); yeni değer → `ADD VALUE`
//!   konumu taslak sırasından: öncesinde değer varsa `AFTER '<önceki etiket>'`, ilk konumdaysa `BEFORE '<ilk yüklü>'`
//!   (yeniden adlandırmalardan SONRAKİ etiketlerle; yüklü değer yoksa konumsuz). PG < 12'de her `ADD VALUE` transaction
//!   dışıdır (`stmt_nt`). Önce yeniden adlandırmalar, sonra eklemeler.
//! - **Composite** (Karar 7): öznitelikler `original` ile eşlenir. `RENAME ATTRIBUTE` (Safe), `DROP ATTRIBUTE`
//!   (DataLoss), `ALTER ATTRIBUTE … TYPE [COLLATE]` (DataLoss; kanonik tip ya da collation değiştiyse), `ADD ATTRIBUTE`
//!   (Safe); `CASCADE` yazılmaz. PG öznitelikleri yeniden sıralayamaz ve yeni özniteliği sona ekler: yüklü özniteliklerin
//!   sırası korunmalı, yeni öznitelikler sonda olmalı (aksi hâlde `alter → load` taslağa eşit olmazdı).
//! - **Range** (Karar 8): yalnızca create; alter'da gövde alanı değişimi alan hatası. `MULTIRANGE_TYPE_NAME` PG 14+,
//!   range'in şemasıyla nitelenir (niteliksiz ad arama yolunun ilk şemasına kurulur; PG 14–18'de gözlendi).
//!   `CANONICAL`/`SUBTYPE_DIFF` metni `ad ('.' ad)?` dilbilgisiyle ayrıştırılır (tırnaksız ad küçük harfe katlanır,
//!   tırnaklı adda `""` kaçışı; `regproc::text` biçimi) ve parçalar `quote_ident`/`qualified` ile yeniden yazılır.
//! - **Adlar:** öznitelik, collation, opclass, multirange adı tek tanımlayıcı (`check_ident` + `quote_ident`); etiketler
//!   ≤ 63 bayt, boş değil, NUL/denetim karakteri yok, `quote_literal`. Tipler `validate_type`; alter "değişti mi"
//!   `canonical_type` ile. Serbest SQL alanı yoktur.
//! - **Alter sırası:** RENAME → SET SCHEMA → gövde (enum RENAME VALUE'lar, ADD VALUE'lar / composite RENAME, DROP,
//!   ALTER TYPE, ADD) → COMMENT → OWNER TO (en sonda). Yeniden adlandırma hedefi başka bir yüklü ad olamaz (değiş
//!   tokuş ve silinecek adın devralınması PG'de sırayla başarısız olur).

use super::type_canon::canonical_type;
use super::type_syntax::validate_type;
use super::{
    BuildCtx, MAX_IDENT_BYTES, check_ident, check_old_ident, check_old_ref, check_old_schema_ident, check_schema_ident,
    check_text, err, stmt, stmt_nt,
};
use crate::model::{CompositeAttr, EnumValue, FieldError, TypeBody, TypeDef};
use crate::plan::{Plan, Risk, Statement};
use crate::quote::{qualified, quote_ident, quote_literal};

pub const ENUM_ORDER_MSG: &str = "PostgreSQL enum değerlerini silemez ya da yeniden sıralayamaz.";
pub const ATTR_ORDER_MSG: &str = "PostgreSQL composite özniteliklerini yeniden sıralayamaz.";
pub const ATTR_APPEND_MSG: &str = "Yeni öznitelikler yalnızca sona eklenebilir (PostgreSQL öznitelikleri sona ekler).";
pub const RENAME_CLASH_MSG: &str =
    "Yeni ad başka bir yüklü değerle çakışıyor; önce onu ayrı bir değişiklikte yeniden adlandırın.";
pub const KIND_CHANGE_MSG: &str = "Tipin türü (enum / composite / range) değiştirilemez.";
pub const RANGE_FIXED_MSG: &str = "Range tipinin bu alanı oluşturulduktan sonra değiştirilemez.";
pub const MULTIRANGE_VERSION: &str = "MULTIRANGE_TYPE_NAME PG 14+ gerektirir.";
pub const ATTR_DATA_LOSS_NOTE: &str = "tipi kullanan tablolardaki veri kaybolur";
pub const ATTR_RETYPE_NOTE: &str = "tipi kullanan tablo kolonu varsa PostgreSQL reddeder; typed table CASCADE ister (desteklenmez)";
pub const ADD_VALUE_NT_NOTE: &str = "PG 12 öncesinde transaction dışında çalışır";
pub const CASCADE_WARNING: &str = "Bağımlı nesneler de silinir";
pub const READ_ONLY_MSG: &str =
    "Bu tip salt okunurdur (base type, multirange, eklentiye ait ya da sistem tipi); değiştirilemez ve silinemez.";

fn type_sql(schema: &str, name: &str) -> String {
    qualified(schema, name)
}

fn comment_sql(ty: &str, comment: &Option<String>) -> String {
    let value = comment.as_deref().map(quote_literal).unwrap_or_else(|| "NULL".into());
    format!("COMMENT ON TYPE {ty} IS {value}")
}

fn owner_sql(ty: &str, owner: &str) -> String {
    format!("ALTER TYPE {ty} OWNER TO {}", quote_ident(owner))
}

/// Yeni ya da değişen enum etiketi: boş değil, NUL/denetim karakteri yok, en çok 63 bayt (PG `NAMEDATALEN - 1`).
fn check_label(path: &str, v: &str, e: &mut Vec<FieldError>) {
    if v.is_empty() {
        e.push(err(path, "Etiket boş olamaz."));
    } else if v.contains('\0') {
        e.push(err(path, "Etiket NUL karakteri içeremez."));
    } else if v.chars().any(char::is_control) {
        e.push(err(path, "Etiket denetim karakteri (satır sonu, sekme vb.) içeremez."));
    } else if v.len() > MAX_IDENT_BYTES {
        e.push(err(path, format!("Etiket en çok {MAX_IDENT_BYTES} bayt olabilir (şu an {} bayt).", v.len())));
    }
}

/// Yüklü (değişmeyen) etiket: yalnızca `quote_literal` ile yerleşir; NUL reddedilir.
fn check_old_label(path: &str, v: &str, e: &mut Vec<FieldError>) {
    if v.contains('\0') {
        e.push(err(path, "Etiket NUL karakteri içeremez."));
    }
}

/// `ad ('.' ad)?`; ad := `[A-Za-z_][A-Za-z0-9_$]*` (küçük harfe katlanır) | `"…"` (`""` kaçışı). Parçalar ayrıca
/// `check_ident` kurallarından geçer. Dönüş: (şema?, ad).
pub(crate) fn parse_func_name(s: &str) -> Result<(Option<String>, String), String> {
    const BAD: &str = "Fonksiyon adı `ad` ya da `şema.ad` biçiminde olmalı (gerekirse çift tırnak).";
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut parts: Vec<String> = vec![];
    loop {
        let mut part = String::new();
        match c.get(i) {
            Some('"') => {
                i += 1;
                loop {
                    match c.get(i) {
                        None => return Err(BAD.into()),
                        Some('"') if c.get(i + 1) == Some(&'"') => {
                            part.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            part.push(ch);
                            i += 1;
                        }
                    }
                }
                if part.is_empty() {
                    return Err(BAD.into());
                }
            }
            Some(&ch) if ch.is_ascii_alphabetic() || ch == '_' => {
                while let Some(&ch) = c.get(i) {
                    if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
                        part.push(ch.to_ascii_lowercase());
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => return Err(BAD.into()),
        }
        parts.push(part);
        match c.get(i) {
            None => break,
            Some('.') if parts.len() == 1 => i += 1,
            _ => return Err(BAD.into()),
        }
    }
    let mut e = vec![];
    for p in &parts {
        check_ident("", p, &mut e);
    }
    if let Some(x) = e.into_iter().next() {
        return Err(x.message);
    }
    let name = parts.pop().unwrap_or_default();
    Ok((parts.pop(), name))
}

fn func_sql(s: &str) -> String {
    match parse_func_name(s) {
        Ok((Some(schema), name)) => qualified(&schema, &name),
        Ok((None, name)) => quote_ident(&name),
        // Yalnızca doğrulanmış metinle çağrılır.
        Err(_) => quote_ident(s),
    }
}

fn attr_sql(a: &CompositeAttr) -> String {
    let mut s = format!("{} {}", quote_ident(&a.name), a.type_name);
    if let Some(c) = &a.collation {
        s.push_str(&format!(" COLLATE {}", quote_ident(c)));
    }
    s
}

/// `CREATE TYPE … AS ENUM (…) | AS (…) | AS RANGE (…)` (çağıran doğrulamış olmalı).
fn define_sql(d: &TypeDef) -> String {
    let ty = type_sql(&d.schema, &d.name);
    match &d.body {
        TypeBody::Enum { values } => {
            let labels: Vec<String> = values.iter().map(|v| quote_literal(&v.label)).collect();
            format!("CREATE TYPE {ty} AS ENUM ({})", labels.join(", "))
        }
        TypeBody::Composite { attributes } => {
            let attrs: Vec<String> = attributes.iter().map(attr_sql).collect();
            format!("CREATE TYPE {ty} AS ({})", attrs.join(", "))
        }
        TypeBody::Range { subtype, subtype_opclass, collation, canonical, subtype_diff, multirange_type_name } => {
            let mut opts = vec![format!("SUBTYPE = {subtype}")];
            if let Some(x) = subtype_opclass {
                opts.push(format!("SUBTYPE_OPCLASS = {}", quote_ident(x)));
            }
            if let Some(x) = collation {
                opts.push(format!("COLLATION = {}", quote_ident(x)));
            }
            if let Some(x) = canonical {
                opts.push(format!("CANONICAL = {}", func_sql(x)));
            }
            if let Some(x) = subtype_diff {
                opts.push(format!("SUBTYPE_DIFF = {}", func_sql(x)));
            }
            // Range'in şemasında (otomatik ad gibi); niteliksiz ad arama yolunun ilk şemasına kurulurdu.
            if let Some(x) = multirange_type_name {
                opts.push(format!("MULTIRANGE_TYPE_NAME = {}", qualified(&d.schema, x)));
            }
            format!("CREATE TYPE {ty} AS RANGE ({})", opts.join(", "))
        }
    }
}

fn check_type(path: &str, v: &str, e: &mut Vec<FieldError>) {
    if let Err(m) = validate_type(v) {
        e.push(err(path, m));
    }
}

/// Oluşturma gövdesi doğrulaması (tüm alanlar katı).
fn validate_body(ctx: &BuildCtx, body: &TypeBody, e: &mut Vec<FieldError>) {
    match body {
        TypeBody::Enum { values } => {
            for (i, v) in values.iter().enumerate() {
                let path = format!("body.values[{i}].label");
                let before = e.len();
                check_label(&path, &v.label, e);
                if e.len() == before && values[..i].iter().any(|p| p.label == v.label) {
                    e.push(err(&path, "Etiket birden çok kez kullanılmış."));
                }
            }
        }
        TypeBody::Composite { attributes } => {
            for (i, a) in attributes.iter().enumerate() {
                let p = |f: &str| format!("body.attributes[{i}].{f}");
                let before = e.len();
                check_ident(&p("name"), &a.name, e);
                if e.len() == before && attributes[..i].iter().any(|x| x.name == a.name) {
                    e.push(err(&p("name"), "Öznitelik adı birden çok kez kullanılmış."));
                }
                check_type(&p("typeName"), &a.type_name, e);
                if let Some(c) = &a.collation {
                    check_ident(&p("collation"), c, e);
                }
            }
        }
        TypeBody::Range { subtype, subtype_opclass, collation, canonical, subtype_diff, multirange_type_name } => {
            check_type("body.subtype", subtype, e);
            if let Some(x) = subtype_opclass {
                check_ident("body.subtypeOpclass", x, e);
            }
            if let Some(x) = collation {
                check_ident("body.collation", x, e);
            }
            for (path, v) in [("body.canonical", canonical), ("body.subtypeDiff", subtype_diff)] {
                if let Some(x) = v
                    && let Err(m) = parse_func_name(x)
                {
                    e.push(err(path, m));
                }
            }
            if let Some(x) = multirange_type_name {
                if ctx.version_num < 140000 {
                    e.push(err("body.multirangeTypeName", MULTIRANGE_VERSION));
                } else {
                    check_ident("body.multirangeTypeName", x, e);
                }
            }
        }
    }
}

/// Oluşturma doğrulaması (tüm alanlar katı).
pub fn validate(ctx: &BuildCtx, d: &TypeDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_schema_ident("schema", &d.schema, &mut e);
    check_ident("name", &d.name, &mut e);
    if let Some(o) = &d.owner {
        check_ident("owner", o, &mut e);
    }
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    validate_body(ctx, &d.body, &mut e);
    e
}

/// Sıra: CREATE, COMMENT (doluysa), OWNER (doluysa, **en sonda**).
pub fn create(ctx: &BuildCtx, d: &TypeDef) -> Result<Plan, Vec<FieldError>> {
    let errs = validate(ctx, d);
    if !errs.is_empty() {
        return Err(errs);
    }
    let ty = type_sql(&d.schema, &d.name);
    let mut statements = vec![stmt(define_sql(d), Risk::Safe, None)];
    if d.comment.is_some() {
        statements.push(stmt(comment_sql(&ty, &d.comment), Risk::Safe, None));
    }
    if let Some(o) = &d.owner {
        statements.push(stmt(owner_sql(&ty, o), Risk::Safe, None));
    }
    Ok(Plan { statements, warnings: vec![] })
}

/// Enum farkı: önce RENAME VALUE'lar, sonra ADD VALUE'lar (taslak sırasıyla).
fn enum_changes(ctx: &BuildCtx, ty: &str, old: &[EnumValue], new: &[EnumValue], e: &mut Vec<FieldError>) -> Vec<Statement> {
    for (i, v) in old.iter().enumerate() {
        check_old_label(&format!("old.body.values[{i}].label"), &v.label, e);
    }
    if !e.is_empty() {
        return vec![];
    }
    let loaded: Vec<&str> = old.iter().map(|v| v.label.as_str()).collect();
    let mut seq: Vec<&str> = vec![];
    let mut originals_ok = true;
    for (i, v) in new.iter().enumerate() {
        let lp = format!("body.values[{i}].label");
        let before = e.len();
        match v.original.as_deref() {
            Some(o) if !loaded.contains(&o) => {
                originals_ok = false;
                e.push(err(&format!("body.values[{i}].original"), "Yüklü enum değeri bulunamadı."));
            }
            Some(o) if seq.contains(&o) => {
                originals_ok = false;
                e.push(err(&format!("body.values[{i}].original"), "Yüklü enum değeri birden çok kez eşlenmiş."));
            }
            Some(o) => {
                seq.push(o);
                if o == v.label {
                    check_old_label(&lp, &v.label, e);
                } else {
                    check_label(&lp, &v.label, e);
                    if e.len() == before && loaded.contains(&v.label.as_str()) {
                        e.push(err(&lp, RENAME_CLASH_MSG));
                    }
                }
            }
            None => check_label(&lp, &v.label, e),
        }
        if e.len() == before && new[..i].iter().any(|p| p.label == v.label) {
            e.push(err(&lp, "Etiket birden çok kez kullanılmış."));
        }
    }
    if originals_ok && seq != loaded {
        e.push(err("body.values", ENUM_ORDER_MSG));
    }
    if !e.is_empty() {
        return vec![];
    }

    let mut out = vec![];
    for v in new {
        if let Some(o) = &v.original
            && *o != v.label
        {
            out.push(stmt(
                format!("ALTER TYPE {ty} RENAME VALUE {} TO {}", quote_literal(o), quote_literal(&v.label)),
                Risk::Safe,
                None,
            ));
        }
    }
    let first_loaded = new.iter().find(|v| v.original.is_some()).map(|v| v.label.as_str());
    for (i, v) in new.iter().enumerate() {
        if v.original.is_some() {
            continue;
        }
        let pos = match (i.checked_sub(1).map(|p| new[p].label.as_str()), first_loaded) {
            (Some(prev), _) => format!(" AFTER {}", quote_literal(prev)),
            (None, Some(first)) => format!(" BEFORE {}", quote_literal(first)),
            (None, None) => String::new(),
        };
        let sql = format!("ALTER TYPE {ty} ADD VALUE {}{pos}", quote_literal(&v.label));
        out.push(if ctx.version_num < 120000 {
            stmt_nt(sql, Risk::Safe, Some(ADD_VALUE_NT_NOTE))
        } else {
            stmt(sql, Risk::Safe, None)
        });
    }
    out
}

/// Composite farkı: RENAME ATTRIBUTE, DROP ATTRIBUTE, ALTER ATTRIBUTE … TYPE, ADD ATTRIBUTE (bu sırayla).
fn composite_changes(ty: &str, old: &[CompositeAttr], new: &[CompositeAttr], e: &mut Vec<FieldError>) -> Vec<Statement> {
    for (i, a) in old.iter().enumerate() {
        check_old_ident(&format!("old.body.attributes[{i}].name"), &a.name, e);
    }
    if !e.is_empty() {
        return vec![];
    }
    let loaded: Vec<&str> = old.iter().map(|a| a.name.as_str()).collect();
    // (yeni indeks, eski indeks)
    let mut matched: Vec<(usize, usize)> = vec![];
    let mut originals_ok = true;
    let mut retyped: Vec<usize> = vec![];
    for (i, a) in new.iter().enumerate() {
        let p = |f: &str| format!("body.attributes[{i}].{f}");
        let before = e.len();
        match a.original.as_deref() {
            Some(o) => match loaded.iter().position(|l| *l == o) {
                None => {
                    originals_ok = false;
                    e.push(err(&p("original"), "Yüklü öznitelik bulunamadı."));
                }
                Some(oi) if matched.iter().any(|(_, m)| *m == oi) => {
                    originals_ok = false;
                    e.push(err(&p("original"), "Yüklü öznitelik birden çok kez eşlenmiş."));
                }
                Some(oi) => {
                    matched.push((i, oi));
                    let oa = &old[oi];
                    if oa.name != a.name {
                        check_ident(&p("name"), &a.name, e);
                        if e.len() == before && loaded.contains(&a.name.as_str()) {
                            e.push(err(&p("name"), RENAME_CLASH_MSG));
                        }
                    }
                    let coll_changed = oa.collation != a.collation;
                    if coll_changed || canonical_type(&oa.type_name) != canonical_type(&a.type_name) {
                        retyped.push(i);
                        check_type(&p("typeName"), &a.type_name, e);
                        if let Some(c) = &a.collation {
                            if coll_changed {
                                check_ident(&p("collation"), c, e);
                            } else {
                                check_old_ref(&p("collation"), c, e);
                            }
                        }
                    }
                }
            },
            None => {
                check_ident(&p("name"), &a.name, e);
                check_type(&p("typeName"), &a.type_name, e);
                if let Some(c) = &a.collation {
                    check_ident(&p("collation"), c, e);
                }
            }
        }
        if e.len() == before && new[..i].iter().any(|x| x.name == a.name) {
            e.push(err(&p("name"), "Öznitelik adı birden çok kez kullanılmış."));
        }
    }
    if originals_ok {
        if matched.windows(2).any(|w| w[0].1 > w[1].1) {
            e.push(err("body.attributes", ATTR_ORDER_MSG));
        } else if matched.last().is_some_and(|(ni, _)| new[..*ni].iter().any(|a| a.original.is_none())) {
            e.push(err("body.attributes", ATTR_APPEND_MSG));
        }
    }
    if !e.is_empty() {
        return vec![];
    }

    let mut out = vec![];
    for &(ni, oi) in &matched {
        if old[oi].name != new[ni].name {
            out.push(stmt(
                format!("ALTER TYPE {ty} RENAME ATTRIBUTE {} TO {}", quote_ident(&old[oi].name), quote_ident(&new[ni].name)),
                Risk::Safe,
                None,
            ));
        }
    }
    for (oi, oa) in old.iter().enumerate() {
        if !matched.iter().any(|(_, m)| *m == oi) {
            out.push(stmt(
                format!("ALTER TYPE {ty} DROP ATTRIBUTE {}", quote_ident(&oa.name)),
                Risk::DataLoss,
                Some(ATTR_DATA_LOSS_NOTE),
            ));
        }
    }
    for &ni in &retyped {
        let a = &new[ni];
        let mut sql = format!("ALTER TYPE {ty} ALTER ATTRIBUTE {} TYPE {}", quote_ident(&a.name), a.type_name);
        if let Some(c) = &a.collation {
            sql.push_str(&format!(" COLLATE {}", quote_ident(c)));
        }
        out.push(stmt(sql, Risk::DataLoss, Some(ATTR_RETYPE_NOTE)));
    }
    for a in new.iter().filter(|a| a.original.is_none()) {
        out.push(stmt(format!("ALTER TYPE {ty} ADD ATTRIBUTE {}", attr_sql(a)), Risk::Safe, None));
    }
    out
}

/// Range: gövde alanları değiştirilemez (Karar 8); değişen her alan ayrı hata.
fn range_changes(old: &TypeBody, new: &TypeBody, e: &mut Vec<FieldError>) {
    let (
        TypeBody::Range { subtype: os, subtype_opclass: oo, collation: oc, canonical: ocn, subtype_diff: od, multirange_type_name: om },
        TypeBody::Range { subtype: ns, subtype_opclass: no, collation: nc, canonical: ncn, subtype_diff: nd, multirange_type_name: nm },
    ) = (old, new)
    else {
        return;
    };
    if canonical_type(os) != canonical_type(ns) {
        e.push(err("body.subtype", RANGE_FIXED_MSG));
    }
    for (path, changed) in [
        ("body.subtypeOpclass", oo != no),
        ("body.collation", oc != nc),
        ("body.canonical", ocn != ncn),
        ("body.subtypeDiff", od != nd),
        ("body.multirangeTypeName", om != nm),
    ] {
        if changed {
            e.push(err(path, RANGE_FIXED_MSG));
        }
    }
}

fn same_variant(a: &TypeBody, b: &TypeBody) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

/// Sıra (Global Constraints): RENAME → SET SCHEMA → gövde → COMMENT → OWNER TO (en sonda). Yalnızca değişen ya da
/// ifadeye yazılan alanlar doğrulanır. `editable = false` eski tanımda her değişiklik `object` hatasıdır.
pub fn alter(ctx: &BuildCtx, old: &TypeDef, new: &TypeDef) -> Result<Plan, Vec<FieldError>> {
    let mut e = vec![];
    check_old_schema_ident("old.schema", &old.schema, &mut e);
    check_old_ident("old.name", &old.name, &mut e);
    if !e.is_empty() {
        return Err(e);
    }
    let name_changed = old.name != new.name;
    let schema_changed = old.schema != new.schema;
    let owner_changed = new.owner.is_some() && new.owner != old.owner;
    let comment_changed = old.comment != new.comment;
    let body_changed = old.body != new.body;
    if !old.editable && (name_changed || schema_changed || owner_changed || comment_changed || body_changed) {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    if !same_variant(&old.body, &new.body) {
        return Err(vec![err("body", KIND_CHANGE_MSG)]);
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
    // Gövde ifadeleri yeni ad ve şemayı hedefler (RENAME ve SET SCHEMA önce çalışır).
    let ty = type_sql(&new.schema, &new.name);
    let body = match (&old.body, &new.body) {
        (TypeBody::Enum { values: o }, TypeBody::Enum { values: n }) => enum_changes(ctx, &ty, o, n, &mut e),
        (TypeBody::Composite { attributes: o }, TypeBody::Composite { attributes: n }) => composite_changes(&ty, o, n, &mut e),
        (o, n) => {
            range_changes(o, n, &mut e);
            vec![]
        }
    };
    if !e.is_empty() {
        return Err(e);
    }

    let mut statements = vec![];
    if name_changed {
        statements.push(stmt(
            format!("ALTER TYPE {} RENAME TO {}", type_sql(&old.schema, &old.name), quote_ident(&new.name)),
            Risk::Safe,
            None,
        ));
    }
    if schema_changed {
        statements.push(stmt(
            format!("ALTER TYPE {} SET SCHEMA {}", type_sql(&old.schema, &new.name), quote_ident(&new.schema)),
            Risk::Safe,
            None,
        ));
    }
    statements.extend(body);
    if comment_changed {
        statements.push(stmt(comment_sql(&ty, &new.comment), Risk::Safe, None));
    }
    // OWNER TO en sonda: kendisi eski sahibin haklarını ister (View kuralı).
    if let Some(o) = &new.owner
        && owner_changed
    {
        statements.push(stmt(owner_sql(&ty, o), Risk::Safe, None));
    }
    Ok(Plan { statements, warnings: vec![] })
}

/// `DROP TYPE <q> [CASCADE]`; kimlik dağıtıcıda (`validate_identity`) doğrulanır. Salt okunur silinemez.
pub fn drop(d: &TypeDef, cascade: bool) -> Result<Plan, Vec<FieldError>> {
    if !d.editable {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    let sql = format!("DROP TYPE {}", type_sql(&d.schema, &d.name));
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
    use crate::build::{alter as balter, assert_single_statements, create as bcreate, drop as bdrop, render};
    use crate::model::ObjectDef;

    fn ctx_v(version_num: u32) -> BuildCtx {
        BuildCtx { version_num, ..Default::default() }
    }
    fn ctx() -> BuildCtx {
        ctx_v(160000)
    }
    fn o(d: &TypeDef) -> ObjectDef {
        ObjectDef::Type(d.clone())
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn paths(errs: Vec<FieldError>) -> Vec<String> {
        errs.into_iter().map(|e| e.path).collect()
    }
    fn base(body: TypeBody) -> TypeDef {
        TypeDef { schema: "public".into(), name: "ty".into(), owner: None, comment: None, body, editable: true, unsupported: vec![] }
    }
    /// Yeni değer (`original = None`).
    fn nv(l: &str) -> EnumValue {
        EnumValue { label: l.into(), original: None }
    }
    /// Yüklü değer (`original = label`).
    fn lv(l: &str) -> EnumValue {
        EnumValue { label: l.into(), original: Some(l.into()) }
    }
    /// Yeniden adlandırılan yüklü değer.
    fn rv(from: &str, to: &str) -> EnumValue {
        EnumValue { label: to.into(), original: Some(from.into()) }
    }
    fn en(values: Vec<EnumValue>) -> TypeDef {
        base(TypeBody::Enum { values })
    }
    fn loaded_enum(labels: &[&str]) -> TypeDef {
        en(labels.iter().map(|l| lv(l)).collect())
    }
    fn na(n: &str, t: &str) -> CompositeAttr {
        CompositeAttr { name: n.into(), type_name: t.into(), collation: None, original: None }
    }
    fn la(n: &str, t: &str) -> CompositeAttr {
        CompositeAttr { original: Some(n.into()), ..na(n, t) }
    }
    fn comp(attributes: Vec<CompositeAttr>) -> TypeDef {
        base(TypeBody::Composite { attributes })
    }
    fn range(subtype: &str) -> TypeBody {
        TypeBody::Range {
            subtype: subtype.into(),
            subtype_opclass: None,
            collation: None,
            canonical: None,
            subtype_diff: None,
            multirange_type_name: None,
        }
    }
    fn create_ok_v(d: &TypeDef, v: u32) -> Plan {
        let p = bcreate(&o(d), &ctx_v(v)).unwrap();
        assert_single_statements(&p);
        p
    }
    fn create_ok(d: &TypeDef) -> Plan {
        create_ok_v(d, 160000)
    }
    fn alter_ok_v(old: &TypeDef, new: &TypeDef, v: u32) -> Plan {
        let p = balter(&o(old), &o(new), &ctx_v(v)).unwrap();
        assert_single_statements(&p);
        p
    }
    fn alter_ok(old: &TypeDef, new: &TypeDef) -> Plan {
        alter_ok_v(old, new, 160000)
    }
    fn alter_err(old: &TypeDef, new: &TypeDef) -> Vec<String> {
        paths(balter(&o(old), &o(new), &ctx()).unwrap_err())
    }

    #[test]
    fn create_variants() {
        let full_range = TypeBody::Range {
            subtype: "text".into(),
            subtype_opclass: Some("text_ops".into()),
            collation: Some("C".into()),
            canonical: Some("public.canon".into()),
            subtype_diff: Some("\"My S\".\"diff.fn\"".into()),
            multirange_type_name: Some("textmultirange".into()),
        };
        let cases: Vec<(&str, TypeDef)> = vec![
            ("enum", en(vec![nv("a"), nv("b c"), nv("it's")])),
            ("enum boş", en(vec![])),
            ("enum yorum + sahip", TypeDef { comment: Some("c".into()), owner: Some("Sahip".into()), ..en(vec![nv("x")]) }),
            (
                "composite",
                comp(vec![na("a", "integer"), CompositeAttr { collation: Some("C".into()), ..na("Ad Soyad", "text") }, na("t", "numeric(10,2)[]")]),
            ),
            ("composite boş", comp(vec![])),
            ("range yalın", base(range("float8"))),
            ("range tam", base(full_range)),
            (
                "range niteliksiz fonksiyon",
                with_range(|b| if let TypeBody::Range { canonical, .. } = b { *canonical = Some("Canon".into()) }),
            ),
        ];
        let mut out = String::new();
        for (label, d) in cases {
            let p = create_ok(&d);
            assert!(p.statements.iter().all(|s| s.risk == Risk::Safe && s.transactional), "{label}");
            assert!(p.warnings.is_empty(), "{label}");
            out.push_str(&format!("# {label}\n{}\n", render(&p)));
        }
        insta::assert_snapshot!(out);
        assert_eq!(sqls(&create_ok(&en(vec![nv("a"), nv("b")]))), ["CREATE TYPE public.ty AS ENUM ('a', 'b')"]);
        assert_eq!(
            sqls(&create_ok(&TypeDef { comment: Some("c".into()), owner: Some("r".into()), ..comp(vec![na("a", "int")]) })),
            ["CREATE TYPE public.ty AS (a int)", "COMMENT ON TYPE public.ty IS 'c'", "ALTER TYPE public.ty OWNER TO r"]
        );
        assert_eq!(
            sqls(&create_ok(&with_range(|b| if let TypeBody::Range { subtype_diff, .. } = b { *subtype_diff = Some("float8mi".into()) }))),
            ["CREATE TYPE public.ty AS RANGE (SUBTYPE = int4, SUBTYPE_DIFF = float8mi)"]
        );
    }

    #[test]
    fn multirange_name_is_pg14_plus() {
        let d = with_range(|b| if let TypeBody::Range { multirange_type_name, .. } = b { *multirange_type_name = Some("mr".into()) });
        assert_eq!(paths(bcreate(&o(&d), &ctx_v(130000)).unwrap_err()), ["body.multirangeTypeName"]);
        assert_eq!(paths(bcreate(&o(&d), &ctx_v(139999)).unwrap_err()), ["body.multirangeTypeName"]);
        // Range'in şemasıyla nitelenir: niteliksiz ad arama yolunun ilk şemasına kurulurdu (PG 14–18'de gözlendi);
        // otomatik ad gibi range'in şemasında olmalı.
        assert_eq!(sqls(&create_ok_v(&d, 140000)), ["CREATE TYPE public.ty AS RANGE (SUBTYPE = int4, MULTIRANGE_TYPE_NAME = public.mr)"]);
        // Multirange adsız range PG 13'te serbest.
        assert!(bcreate(&o(&base(range("int4"))), &ctx_v(130000)).is_ok());
    }

    fn with_range(f: impl FnOnce(&mut TypeBody)) -> TypeDef {
        let mut b = range("int4");
        f(&mut b);
        base(b)
    }

    #[test]
    fn create_rule_violations() {
        let set = |field: &'static str, v: &str| {
            let v = v.to_string();
            with_range(move |b| {
                if let TypeBody::Range { subtype_opclass, collation, canonical, subtype_diff, multirange_type_name, .. } = b {
                    let slot = match field {
                        "opclass" => subtype_opclass,
                        "collation" => collation,
                        "canonical" => canonical,
                        "diff" => subtype_diff,
                        _ => multirange_type_name,
                    };
                    *slot = Some(v);
                }
            })
        };
        let cases: Vec<(&str, TypeDef, &str)> = vec![
            ("etiket boş", en(vec![nv("a"), nv("")]), "body.values[1].label"),
            ("etiket NUL", en(vec![nv("a\0")]), "body.values[0].label"),
            ("etiket satır sonu", en(vec![nv("a\nb")]), "body.values[0].label"),
            ("etiket C1", en(vec![nv("a\u{85}")]), "body.values[0].label"),
            ("etiket 64 bayt", en(vec![nv(&"a".repeat(64))]), "body.values[0].label"),
            ("etiket 64 bayt çok baytlı", en(vec![nv(&"ş".repeat(32))]), "body.values[0].label"),
            ("etiket tekrar", en(vec![nv("a"), nv("b"), nv("a")]), "body.values[2].label"),
            ("öznitelik adı boş", comp(vec![na("", "int")]), "body.attributes[0].name"),
            ("öznitelik adı tekrar", comp(vec![na("a", "int"), na("a", "text")]), "body.attributes[1].name"),
            ("öznitelik tipi enjeksiyon", comp(vec![na("a", "int) ; DROP TABLE x; --")]), "body.attributes[0].typeName"),
            ("öznitelik tipi NOT NULL", comp(vec![na("a", "int not null")]), "body.attributes[0].typeName"),
            ("öznitelik tipi boş", comp(vec![na("a", "")]), "body.attributes[0].typeName"),
            (
                "öznitelik collation boş",
                comp(vec![CompositeAttr { collation: Some(String::new()), ..na("a", "text") }]),
                "body.attributes[0].collation",
            ),
            ("range alt tip boş", base(range("")), "body.subtype"),
            ("range alt tip enjeksiyon", base(range("int4, CANONICAL = x")), "body.subtype"),
            ("range opclass satır sonu", set("opclass", "o\n"), "body.subtypeOpclass"),
            ("range collation boş", set("collation", ""), "body.collation"),
            ("range canonical üç parça", set("canonical", "a.b.c"), "body.canonical"),
            ("range canonical parantez", set("canonical", "f()"), "body.canonical"),
            ("range subtype_diff boş", set("diff", ""), "body.subtypeDiff"),
            ("range subtype_diff enjeksiyon", set("diff", "f); DROP TABLE x; --"), "body.subtypeDiff"),
            ("range multirange boş", set("multirange", ""), "body.multirangeTypeName"),
            ("ad boş", TypeDef { name: String::new(), ..en(vec![]) }, "name"),
            ("ad 64 bayt", TypeDef { name: "a".repeat(64), ..en(vec![]) }, "name"),
            ("şema sistem", TypeDef { schema: "pg_catalog".into(), ..en(vec![]) }, "schema"),
            ("sahip boş", TypeDef { owner: Some(String::new()), ..en(vec![]) }, "owner"),
            ("yorum NUL", TypeDef { comment: Some("a\0".into()), ..en(vec![]) }, "comment"),
        ];
        let mut out = String::new();
        for (label, d, want) in cases {
            assert_eq!(paths(bcreate(&o(&d), &ctx()).unwrap_err()), [want], "{label}");
            assert_eq!(paths(validate(&ctx(), &d)), [want], "{label}");
            out.push_str(&format!("{label}: {want}\n"));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn func_name_grammar() {
        let ok = |s: &str| parse_func_name(s).unwrap();
        assert_eq!(ok("f"), (None, "f".into()));
        assert_eq!(ok("Public.F_1$"), (Some("public".into()), "f_1$".into()));
        assert_eq!(ok("\"My S\".\"f.g\""), (Some("My S".into()), "f.g".into()));
        assert_eq!(ok("\"a\"\"b\""), (None, "a\"b".into()));
        let long = format!("\"{}\"", "a".repeat(64));
        for bad in ["", ".f", "f.", "a.b.c", "f()", "f g", "\"\"", "\"x", "1f", "f;", "\"a\nb\"", "s.\"\u{202e}x\"", long.as_str()] {
            assert!(parse_func_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(func_sql("\"My S\".\"f.g\""), "\"My S\".\"f.g\"");
        assert_eq!(func_sql("public.\"select\""), "public.\"select\"");
        assert_eq!(func_sql("FN"), "fn");
    }

    #[test]
    fn enum_add_positions() {
        let old = loaded_enum(&["a", "b", "c"]);
        let cases: Vec<(&str, Vec<EnumValue>, Vec<&str>)> = vec![
            ("başa", vec![nv("x"), lv("a"), lv("b"), lv("c")], vec!["ALTER TYPE public.ty ADD VALUE 'x' BEFORE 'a'"]),
            ("ortaya", vec![lv("a"), nv("x"), lv("b"), lv("c")], vec!["ALTER TYPE public.ty ADD VALUE 'x' AFTER 'a'"]),
            ("sona", vec![lv("a"), lv("b"), lv("c"), nv("x")], vec!["ALTER TYPE public.ty ADD VALUE 'x' AFTER 'c'"]),
            (
                "başa iki",
                vec![nv("x"), nv("y"), lv("a"), lv("b"), lv("c")],
                vec!["ALTER TYPE public.ty ADD VALUE 'x' BEFORE 'a'", "ALTER TYPE public.ty ADD VALUE 'y' AFTER 'x'"],
            ),
            (
                "dağınık",
                vec![lv("a"), nv("x"), lv("b"), nv("y"), nv("z"), lv("c"), nv("w")],
                vec![
                    "ALTER TYPE public.ty ADD VALUE 'x' AFTER 'a'",
                    "ALTER TYPE public.ty ADD VALUE 'y' AFTER 'b'",
                    "ALTER TYPE public.ty ADD VALUE 'z' AFTER 'y'",
                    "ALTER TYPE public.ty ADD VALUE 'w' AFTER 'c'",
                ],
            ),
        ];
        let mut out = String::new();
        for (label, values, want) in cases {
            let pl = alter_ok(&old, &en(values));
            assert_eq!(sqls(&pl), want, "{label}");
            assert!(pl.statements.iter().all(|s| s.risk == Risk::Safe && s.transactional), "{label}");
            out.push_str(&format!("# {label}\n{}\n", render(&pl)));
        }
        insta::assert_snapshot!(out);
        // Boş enum'a ilk değer: konumsuz; ikincisi öncekinin ardına.
        assert_eq!(
            sqls(&alter_ok(&en(vec![]), &en(vec![nv("x"), nv("y")]))),
            ["ALTER TYPE public.ty ADD VALUE 'x'", "ALTER TYPE public.ty ADD VALUE 'y' AFTER 'x'"]
        );
    }

    #[test]
    fn enum_rename_then_add_uses_new_labels() {
        let old = loaded_enum(&["a", "b"]);
        let new = en(vec![nv("x"), rv("a", "A1"), nv("y"), rv("b", "it's"), nv("z")]);
        let pl = alter_ok(&old, &new);
        assert_eq!(
            sqls(&pl),
            [
                "ALTER TYPE public.ty RENAME VALUE 'a' TO 'A1'",
                "ALTER TYPE public.ty RENAME VALUE 'b' TO 'it''s'",
                "ALTER TYPE public.ty ADD VALUE 'x' BEFORE 'A1'",
                "ALTER TYPE public.ty ADD VALUE 'y' AFTER 'A1'",
                "ALTER TYPE public.ty ADD VALUE 'z' AFTER 'it''s'",
            ]
        );
        assert!(pl.statements.iter().all(|s| s.risk == Risk::Safe));
        insta::assert_snapshot!(render(&pl));
        // Yalnız yeniden adlandırma.
        assert_eq!(sqls(&alter_ok(&old, &en(vec![lv("a"), rv("b", "c")]))), ["ALTER TYPE public.ty RENAME VALUE 'b' TO 'c'"]);
    }

    #[test]
    fn enum_add_value_transaction_by_version() {
        let old = loaded_enum(&["a"]);
        let new = en(vec![rv("a", "b"), nv("x"), nv("y")]);
        for (v, tx) in [(100000, false), (110000, false), (119999, false), (120000, true), (160000, true)] {
            let pl = alter_ok_v(&old, &new, v);
            assert_eq!(sqls(&pl).len(), 3, "{v}");
            // RENAME VALUE her sürümde transaction içinde; ADD VALUE PG < 12'de dışında.
            assert!(pl.statements[0].transactional, "{v}");
            for s in &pl.statements[1..] {
                assert_eq!(s.transactional, tx, "{v}");
                assert_eq!(s.note.as_deref(), if tx { None } else { Some(ADD_VALUE_NT_NOTE) }, "{v}");
            }
        }
        insta::assert_snapshot!("enum_add_value_pg11", render(&alter_ok_v(&old, &new, 110000)));
        insta::assert_snapshot!("enum_add_value_pg12", render(&alter_ok_v(&old, &new, 120000)));
    }

    #[test]
    fn enum_remove_reorder_and_mapping_errors() {
        let old = loaded_enum(&["a", "b", "c"]);
        let long = "x".repeat(64);
        let cases: Vec<(&str, Vec<EnumValue>, &str)> = vec![
            ("silme", vec![lv("a"), lv("c")], "body.values"),
            ("hepsini silme", vec![], "body.values"),
            ("yeniden sıralama", vec![lv("b"), lv("a"), lv("c")], "body.values"),
            ("silip aynı adla ekleme", vec![lv("a"), lv("b"), nv("c")], "body.values"),
            ("bilinmeyen original", vec![lv("a"), lv("b"), lv("c"), rv("zz", "d")], "body.values[3].original"),
            ("original tekrar", vec![lv("a"), lv("b"), lv("c"), rv("a", "d")], "body.values[3].original"),
            ("değiş tokuş", vec![rv("a", "b"), rv("b", "a"), lv("c")], "body.values[0].label"),
            ("yeni etiket yüklüyle aynı", vec![lv("a"), lv("b"), lv("c"), nv("a")], "body.values[3].label"),
            ("yeni etiket tekrar", vec![lv("a"), lv("b"), lv("c"), nv("x"), nv("x")], "body.values[4].label"),
            ("yeniden adlandırma 64 bayt", vec![rv("a", &long), lv("b"), lv("c")], "body.values[0].label"),
            ("yeniden adlandırma boş", vec![rv("a", ""), lv("b"), lv("c")], "body.values[0].label"),
        ];
        let mut out = String::new();
        for (label, values, want) in cases {
            let e = balter(&o(&old), &o(&en(values)), &ctx()).unwrap_err();
            assert!(paths(e.clone()).contains(&want.to_string()), "{label}: {e:?}");
            if want == "body.values" {
                assert_eq!(e.iter().find(|x| x.path == want).unwrap().message, ENUM_ORDER_MSG, "{label}");
            }
            let all: Vec<String> = e.iter().map(|x| format!("{} ({})", x.path, x.message)).collect();
            out.push_str(&format!("{label}: {}\n", all.join("; ")));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn composite_alter_single_changes() {
        let old = comp(vec![la("a", "integer"), la("b", "text")]);
        let c_coll = |a: CompositeAttr| CompositeAttr { collation: Some("C".into()), ..a };
        let renamed_b = CompositeAttr { name: "Yeni B".into(), ..la("b", "text") };
        let cases: Vec<(&str, Vec<CompositeAttr>, Vec<&str>, Risk)> = vec![
            (
                "ekle",
                vec![la("a", "integer"), la("b", "text"), na("c", "numeric(10,2)")],
                vec!["ALTER TYPE public.ty ADD ATTRIBUTE c numeric(10,2)"],
                Risk::Safe,
            ),
            (
                "ekle collation",
                vec![la("a", "integer"), la("b", "text"), c_coll(na("c", "text"))],
                vec!["ALTER TYPE public.ty ADD ATTRIBUTE c text COLLATE \"C\""],
                Risk::Safe,
            ),
            ("yeniden adlandır", vec![la("a", "integer"), renamed_b], vec!["ALTER TYPE public.ty RENAME ATTRIBUTE b TO \"Yeni B\""], Risk::Safe),
            ("tip değiştir", vec![la("a", "bigint"), la("b", "text")], vec!["ALTER TYPE public.ty ALTER ATTRIBUTE a TYPE bigint"], Risk::DataLoss),
            (
                "collation değiştir",
                vec![la("a", "integer"), c_coll(la("b", "text"))],
                vec!["ALTER TYPE public.ty ALTER ATTRIBUTE b TYPE text COLLATE \"C\""],
                Risk::DataLoss,
            ),
            ("sil", vec![la("a", "integer")], vec!["ALTER TYPE public.ty DROP ATTRIBUTE b"], Risk::DataLoss),
            (
                "aynı adla silip ekle",
                vec![la("a", "integer"), na("b", "jsonb")],
                vec!["ALTER TYPE public.ty DROP ATTRIBUTE b", "ALTER TYPE public.ty ADD ATTRIBUTE b jsonb"],
                Risk::DataLoss,
            ),
            // Kanonik olarak eşit tip metni (format_type biçimine karşı) ifade üretmez.
            ("kanonik eşit tip", vec![la("a", "pg_catalog.int4"), la("b", "text")], vec![], Risk::Safe),
        ];
        let mut out = String::new();
        for (label, attrs, want, risk) in cases {
            let pl = alter_ok(&old, &comp(attrs));
            assert_eq!(sqls(&pl), want, "{label}");
            assert_eq!(pl.risk(), risk, "{label}");
            for s in &pl.statements {
                assert!(s.transactional);
                let want_note = match s.risk {
                    Risk::DataLoss if s.sql.contains(" DROP ATTRIBUTE ") => Some(ATTR_DATA_LOSS_NOTE),
                    Risk::DataLoss => Some(ATTR_RETYPE_NOTE),
                    _ => None,
                };
                assert_eq!(s.note.as_deref(), want_note, "{label}");
                assert!(!s.sql.contains("CASCADE"), "{label}");
            }
            out.push_str(&format!("# {label}\n{}\n", render(&pl)));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn composite_alter_combined_order() {
        let old = comp(vec![la("a", "integer"), la("b", "text"), la("c", "date"), la("d", "int")]);
        let new = TypeDef {
            name: "ty2".into(),
            schema: "app".into(),
            comment: Some("k".into()),
            owner: Some("r".into()),
            ..comp(vec![
                CompositeAttr { name: "a2".into(), type_name: "bigint".into(), ..la("a", "integer") },
                la("c", "date"),
                CompositeAttr { name: "e".into(), ..la("d", "int") },
                na("b", "text"),
                na("f", "uuid"),
            ])
        };
        let pl = alter_ok(&old, &new);
        assert_eq!(
            sqls(&pl),
            [
                "ALTER TYPE public.ty RENAME TO ty2",
                "ALTER TYPE public.ty2 SET SCHEMA app",
                "ALTER TYPE app.ty2 RENAME ATTRIBUTE a TO a2",
                "ALTER TYPE app.ty2 RENAME ATTRIBUTE d TO e",
                "ALTER TYPE app.ty2 DROP ATTRIBUTE b",
                "ALTER TYPE app.ty2 ALTER ATTRIBUTE a2 TYPE bigint",
                "ALTER TYPE app.ty2 ADD ATTRIBUTE b text",
                "ALTER TYPE app.ty2 ADD ATTRIBUTE f uuid",
                "COMMENT ON TYPE app.ty2 IS 'k'",
                "ALTER TYPE app.ty2 OWNER TO r",
            ]
        );
        assert_eq!(pl.risk(), Risk::DataLoss);
        insta::assert_snapshot!(render(&pl));
    }

    #[test]
    fn enum_alter_combined_order() {
        let old = TypeDef { comment: Some("c".into()), ..loaded_enum(&["a", "b"]) };
        let new = TypeDef {
            name: "Ty".into(),
            schema: "app".into(),
            comment: None,
            owner: Some("r".into()),
            ..en(vec![lv("a"), rv("b", "B"), nv("c")])
        };
        let pl = alter_ok_v(&old, &new, 110000);
        assert_eq!(
            sqls(&pl),
            [
                "ALTER TYPE public.ty RENAME TO \"Ty\"",
                "ALTER TYPE public.\"Ty\" SET SCHEMA app",
                "ALTER TYPE app.\"Ty\" RENAME VALUE 'b' TO 'B'",
                "ALTER TYPE app.\"Ty\" ADD VALUE 'c' AFTER 'B'",
                "COMMENT ON TYPE app.\"Ty\" IS NULL",
                "ALTER TYPE app.\"Ty\" OWNER TO r",
            ]
        );
        insta::assert_snapshot!(render(&pl));
    }

    #[test]
    fn composite_mapping_errors() {
        let old = comp(vec![la("a", "integer"), la("b", "text")]);
        let ren = |a: CompositeAttr, n: &str| CompositeAttr { name: n.into(), ..a };
        let cases: Vec<(&str, Vec<CompositeAttr>, &str)> = vec![
            ("yeniden sıralama", vec![la("b", "text"), la("a", "integer")], "body.attributes"),
            ("yeni ortada", vec![la("a", "integer"), na("x", "int"), la("b", "text")], "body.attributes"),
            ("yeni başta", vec![na("x", "int"), la("a", "integer"), la("b", "text")], "body.attributes"),
            (
                "bilinmeyen original",
                vec![la("a", "integer"), la("b", "text"), CompositeAttr { original: Some("zz".into()), ..na("c", "int") }],
                "body.attributes[2].original",
            ),
            ("original tekrar", vec![la("a", "integer"), ren(la("a", "integer"), "c")], "body.attributes[1].original"),
            ("değiş tokuş", vec![ren(la("a", "integer"), "b"), ren(la("b", "text"), "a")], "body.attributes[0].name"),
            ("silinen adı devralma", vec![ren(la("a", "integer"), "b")], "body.attributes[0].name"),
            ("yeni ad tekrar", vec![la("a", "integer"), la("b", "text"), na("a", "int")], "body.attributes[2].name"),
            ("yeni ad boş", vec![la("a", "integer"), la("b", "text"), na("", "int")], "body.attributes[2].name"),
            ("yeni tip geçersiz", vec![la("a", "integer"), la("b", "text"), na("c", "int;")], "body.attributes[2].typeName"),
            ("değişen tip geçersiz", vec![la("a", "integer primary key"), la("b", "text")], "body.attributes[0].typeName"),
            (
                "değişen collation geçersiz",
                vec![la("a", "integer"), CompositeAttr { collation: Some("C\n".into()), ..la("b", "text") }],
                "body.attributes[1].collation",
            ),
            ("yeniden adlandırma 64 bayt", vec![ren(la("a", "integer"), &"x".repeat(64)), la("b", "text")], "body.attributes[0].name"),
        ];
        let mut out = String::new();
        for (label, attrs, want) in cases {
            let e = balter(&o(&old), &o(&comp(attrs)), &ctx()).unwrap_err();
            // İlk hata beklenen yolda; tümü (değiş tokuşta iki taraf) snapshot'ta.
            assert_eq!(e[0].path, want, "{label}: {e:?}");
            let all: Vec<String> = e.iter().map(|x| format!("{} ({})", x.path, x.message)).collect();
            out.push_str(&format!("{label}: {}\n", all.join("; ")));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn range_alter_identity_only() {
        let full = TypeBody::Range {
            subtype: "text".into(),
            subtype_opclass: Some("text_ops".into()),
            collation: Some("C".into()),
            canonical: Some("public.c".into()),
            subtype_diff: Some("public.d".into()),
            multirange_type_name: Some("mr".into()),
        };
        let old = base(full.clone());
        let new = TypeDef { name: "r2".into(), schema: "app".into(), comment: Some("k".into()), owner: Some("r".into()), ..old.clone() };
        assert_eq!(
            sqls(&alter_ok(&old, &new)),
            [
                "ALTER TYPE public.ty RENAME TO r2",
                "ALTER TYPE public.r2 SET SCHEMA app",
                "COMMENT ON TYPE app.r2 IS 'k'",
                "ALTER TYPE app.r2 OWNER TO r",
            ]
        );
        // Kanonik eşit alt tip değişim sayılmaz.
        assert!(alter_ok(&base(range("int4")), &base(range("integer"))).statements.is_empty());
        let edit = |f: fn(&mut TypeBody)| {
            let mut b = full.clone();
            f(&mut b);
            TypeDef { body: b, ..old.clone() }
        };
        let cases: Vec<(TypeDef, Vec<&str>)> = vec![
            (edit(|b| if let TypeBody::Range { subtype, .. } = b { *subtype = "bigint".into() }), vec!["body.subtype"]),
            (edit(|b| if let TypeBody::Range { subtype_opclass, .. } = b { *subtype_opclass = None }), vec!["body.subtypeOpclass"]),
            (edit(|b| if let TypeBody::Range { collation, .. } = b { *collation = Some("POSIX".into()) }), vec!["body.collation"]),
            (edit(|b| if let TypeBody::Range { canonical, .. } = b { *canonical = None }), vec!["body.canonical"]),
            (edit(|b| if let TypeBody::Range { subtype_diff, .. } = b { *subtype_diff = Some("x".into()) }), vec!["body.subtypeDiff"]),
            (
                edit(|b| if let TypeBody::Range { multirange_type_name, .. } = b { *multirange_type_name = Some("mr2".into()) }),
                vec!["body.multirangeTypeName"],
            ),
            // Birden çok alan: her biri ayrı hata.
            (
                edit(|b| {
                    if let TypeBody::Range { subtype, subtype_opclass, .. } = b {
                        *subtype = "bigint".into();
                        *subtype_opclass = None;
                    }
                }),
                vec!["body.subtype", "body.subtypeOpclass"],
            ),
        ];
        for (new, want) in cases {
            let e = balter(&o(&old), &o(&new), &ctx()).unwrap_err();
            assert_eq!(paths(e.clone()), want);
            assert!(e.iter().all(|x| x.message == RANGE_FIXED_MSG));
        }
    }

    #[test]
    fn kind_change_is_rejected() {
        let e = loaded_enum(&["a"]);
        let c = comp(vec![la("a", "int")]);
        let r = base(range("int4"));
        for (a, b) in [(&e, &c), (&c, &e), (&e, &r), (&r, &c)] {
            let errs = balter(&o(a), &o(b), &ctx()).unwrap_err();
            assert_eq!(paths(errs.clone()), ["body"]);
            assert_eq!(errs[0].message, KIND_CHANGE_MSG);
        }
    }

    #[test]
    fn read_only_rejects_changes_and_drop() {
        let ro = TypeDef { editable: false, unsupported: vec!["base type".into()], ..loaded_enum(&["a"]) };
        assert!(alter_ok(&ro, &ro).statements.is_empty());
        for new in [
            TypeDef { name: "x".into(), ..ro.clone() },
            TypeDef { schema: "app".into(), ..ro.clone() },
            TypeDef { owner: Some("r".into()), ..ro.clone() },
            TypeDef { comment: Some("k".into()), ..ro.clone() },
            TypeDef { body: TypeBody::Enum { values: vec![lv("a"), nv("b")] }, ..ro.clone() },
            TypeDef { body: range("int4"), ..ro.clone() },
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
            (en(vec![]), false, "DROP TYPE public.ty", Risk::Destructive),
            (en(vec![]), true, "DROP TYPE public.ty CASCADE", Risk::DataLoss),
            (TypeDef { schema: "Ş".into(), name: "My \"T\"".into(), ..comp(vec![]) }, false, "DROP TYPE \"Ş\".\"My \"\"T\"\"\"", Risk::Destructive),
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
        assert!(bdrop(&o(&en(vec![nv(""), nv("")])), false, &ctx()).is_ok());
        assert_eq!(paths(bdrop(&o(&TypeDef { schema: "pg_catalog".into(), ..en(vec![]) }), false, &ctx()).unwrap_err()), ["schema"]);
    }

    #[test]
    fn hostile_labels_stay_single_statement() {
        let l63 = "a".repeat(63);
        let u63 = format!("{}a", "ş".repeat(31));
        for l in ["'", "it's", "\\", "a\\'b", "'); DROP TYPE x; --", "$$", "$pgt$", "/*", "--", "\"", "ş 日本 €", "\u{202e}x", l63.as_str(), u63.as_str()] {
            let pl = create_ok(&en(vec![nv(l), nv("ok")]));
            assert_eq!(pl.statements[0].sql, format!("CREATE TYPE public.ty AS ENUM ({}, 'ok')", quote_literal(l)), "{l:?}");
            for v in [110000, 160000] {
                let pl = alter_ok_v(&loaded_enum(&["ok"]), &en(vec![nv(l), lv("ok")]), v);
                assert_eq!(sqls(&pl), [format!("ALTER TYPE public.ty ADD VALUE {} BEFORE 'ok'", quote_literal(l))], "{l:?}");
                let pl = alter_ok_v(&loaded_enum(&["ok"]), &en(vec![rv("ok", l)]), v);
                assert_eq!(sqls(&pl), [format!("ALTER TYPE public.ty RENAME VALUE 'ok' TO {}", quote_literal(l))], "{l:?}");
                // Yüklü düşmanca etiket çapa olarak yazılır.
                let pl = alter_ok_v(&loaded_enum(&[l]), &en(vec![lv(l), nv("z")]), v);
                assert_eq!(sqls(&pl), [format!("ALTER TYPE public.ty ADD VALUE 'z' AFTER {}", quote_literal(l))], "{l:?}");
            }
        }
        // `\` → E'…' biçimi.
        assert_eq!(create_ok(&en(vec![nv("a\\b")])).statements[0].sql, "CREATE TYPE public.ty AS ENUM (E'a\\\\b')");
        // 63 bayt sınırı (çok baytlı dahil): 64 bayt reddedilir.
        assert_eq!(paths(bcreate(&o(&en(vec![nv(&"a".repeat(64))])), &ctx()).unwrap_err()), ["body.values[0].label"]);
        assert_eq!(paths(bcreate(&o(&en(vec![nv(&format!("{}a", "€".repeat(21)))])), &ctx()).unwrap_err()), ["body.values[0].label"]);
        assert!(bcreate(&o(&en(vec![nv(&"€".repeat(21))])), &ctx()).is_ok());
    }

    #[test]
    fn hostile_names_stay_single_statement() {
        let long63 = "a".repeat(63);
        for n in ["t\"; DROP TABLE x; --", "select", "Tip ş", "日本", long63.as_str(), "'", "\\", "/*", "$$", "$pgt$"] {
            let short = &n[..n.len().min(60)];
            let attr = |name: &str| CompositeAttr { collation: Some(n.into()), original: Some(name.into()), ..na(name, "text") };
            let d = TypeDef { schema: n.into(), name: n.into(), owner: Some(n.into()), comment: Some(n.into()), ..comp(vec![attr(n)]) };
            let new = TypeDef {
                name: format!("{short}x"),
                owner: Some(format!("{short}2")),
                comment: Some(format!("{n}!")),
                body: TypeBody::Composite {
                    attributes: vec![
                        CompositeAttr { name: format!("{short}x"), type_name: "bigint".into(), ..attr(n) },
                        CompositeAttr { original: None, ..attr("yeni") },
                    ],
                },
                ..d.clone()
            };
            let q = n.replace('"', "\"\"");
            let rng = TypeDef {
                body: TypeBody::Range {
                    subtype: "text".into(),
                    subtype_opclass: Some(n.into()),
                    collation: Some(n.into()),
                    canonical: Some(format!("\"{q}\".\"{q}\"")),
                    subtype_diff: Some(format!("\"{q}\"")),
                    multirange_type_name: Some(n.into()),
                },
                ..d.clone()
            };
            let enm = TypeDef { body: TypeBody::Enum { values: vec![nv(n)] }, ..d.clone() };
            for pl in [create_ok(&d), create_ok(&rng), create_ok(&enm), alter_ok(&d, &new), bdrop(&o(&d), true, &ctx()).unwrap(), bdrop(&o(&d), false, &ctx()).unwrap()] {
                assert!(!pl.statements.is_empty(), "{n:?}");
                assert_single_statements(&pl);
            }
            // Range fonksiyon adları ayrıştırılıp yeniden tırnaklanır.
            let s = &create_ok(&rng).statements[0].sql;
            assert!(s.contains(&format!("CANONICAL = {}", qualified(n, n))), "{s}");
            assert!(s.contains(&format!("SUBTYPE_DIFF = {}", quote_ident(n))), "{s}");
        }
        let pl = create_ok(&TypeDef { name: "t\"; DROP TABLE x; --".into(), ..en(vec![]) });
        assert_eq!(pl.statements[0].sql, "CREATE TYPE public.\"t\"\"; DROP TABLE x; --\" AS ENUM ()");
    }

    #[test]
    fn alter_validates_only_changed_or_written_fields() {
        // Eski taraf kimliği gevşek: sıfır genişlikli ad yeniden adlandırılabilir; yeni ad katı.
        let zw = TypeDef { name: "t\u{200b}y".into(), ..loaded_enum(&["a"]) };
        assert_eq!(sqls(&alter_ok(&zw, &TypeDef { name: "ty".into(), ..zw.clone() })), ["ALTER TYPE public.\"t\u{200b}y\" RENAME TO ty"]);
        assert_eq!(alter_err(&loaded_enum(&["a"]), &TypeDef { name: "t\u{200b}y".into(), ..loaded_enum(&["a"]) }), ["name"]);
        assert_eq!(alter_err(&TypeDef { name: String::new(), ..zw.clone() }, &zw), ["old.name"]);
        assert_eq!(alter_err(&TypeDef { schema: "pg_catalog".into(), ..zw.clone() }, &zw), ["old.schema"]);
        assert_eq!(alter_err(&zw, &TypeDef { schema: "pg_toast".into(), ..zw.clone() }), ["schema"]);
        // Yüklü değişmeyen etiketler gevşek (64 bayt, satır sonu); yalnız NUL reddedilir.
        let long = "z".repeat(70);
        let odd = loaded_enum(&["x\ny", &long]);
        assert_eq!(sqls(&alter_ok(&odd, &TypeDef { comment: Some("k".into()), ..odd.clone() })), ["COMMENT ON TYPE public.ty IS 'k'"]);
        let added = en(vec![lv("x\ny"), lv(&long), nv("w")]);
        assert_eq!(sqls(&alter_ok(&odd, &added)), [format!("ALTER TYPE public.ty ADD VALUE 'w' AFTER '{long}'")]);
        assert_eq!(alter_err(&loaded_enum(&["a\0"]), &loaded_enum(&["a\0"])), ["old.body.values[0].label"]);
        // Tipi değişen özniteliğin değişmeyen yüklü adı ve collation'ı gevşek; değişen collation katı.
        let a0 = CompositeAttr { collation: Some("c\u{200b}".into()), ..la("a\u{200b}", "integer") };
        let oddc = comp(vec![a0.clone()]);
        let retyped = comp(vec![CompositeAttr { type_name: "bigint".into(), ..a0.clone() }]);
        assert_eq!(sqls(&alter_ok(&oddc, &retyped)), ["ALTER TYPE public.ty ALTER ATTRIBUTE \"a\u{200b}\" TYPE bigint COLLATE \"c\u{200b}\""]);
        let recoll = comp(vec![CompositeAttr { collation: Some("d\u{200b}".into()), ..a0 }]);
        assert_eq!(alter_err(&oddc, &recoll), ["body.attributes[0].collation"]);
        // Değişmeyen öznitelik tipi doğrulanmaz: yüklü biçim katı dilbilgisine takılsa bile.
        let weird = comp(vec![la("a", "geometry(Point,4326)")]);
        assert_eq!(
            sqls(&alter_ok(&weird, &comp(vec![la("a", "geometry(Point,4326)"), na("b", "int")]))),
            ["ALTER TYPE public.ty ADD ATTRIBUTE b int"]
        );
        // Değişmeyen NUL'lu yorum yazılmaz; değişen yorum/sahip katı.
        let nul = TypeDef { comment: Some("a\0".into()), ..loaded_enum(&["a"]) };
        assert!(balter(&o(&nul), &o(&TypeDef { name: "x".into(), ..nul.clone() }), &ctx()).is_ok());
        assert_eq!(alter_err(&loaded_enum(&["a"]), &TypeDef { comment: Some("b\0".into()), ..loaded_enum(&["a"]) }), ["comment"]);
        assert_eq!(alter_err(&loaded_enum(&["a"]), &TypeDef { owner: Some("r\n".into()), ..loaded_enum(&["a"]) }), ["owner"]);
        // Sahip None → değişiklik yok (boş taslak sahibi düşürmez).
        let owned = TypeDef { owner: Some("r".into()), ..loaded_enum(&["a"]) };
        assert!(alter_ok(&owned, &TypeDef { owner: None, ..owned.clone() }).statements.is_empty());
    }

    #[test]
    fn alter_noop_is_empty() {
        let full_range = base(TypeBody::Range {
            subtype: "text".into(),
            subtype_opclass: Some("text_ops".into()),
            collation: Some("C".into()),
            canonical: Some("public.c".into()),
            subtype_diff: Some("public.d".into()),
            multirange_type_name: Some("mr".into()),
        });
        for d in [
            loaded_enum(&[]),
            loaded_enum(&["a", "b", "it's"]),
            TypeDef {
                comment: Some("c".into()),
                owner: Some("r".into()),
                ..comp(vec![la("a", "integer"), CompositeAttr { collation: Some("C".into()), ..la("b", "text") }])
            },
            comp(vec![]),
            base(range("int4")),
            full_range,
        ] {
            for v in [100000, 110000, 130000, 160000] {
                let pl = alter_ok_v(&d, &d, v);
                assert!(pl.statements.is_empty() && pl.warnings.is_empty(), "{d:?}");
            }
        }
    }
}
