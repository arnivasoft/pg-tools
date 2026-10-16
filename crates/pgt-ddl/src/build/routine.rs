//! Function / Procedure üreticisi: CREATE, ALTER (`CREATE OR REPLACE` eski adla, RENAME, SET SCHEMA, COMMENT, OWNER;
//! açık yeniden oluşturma), DROP.
//!
//! - **Gövde** (Karar 1): içeriği taranmaz; `dollar_tag` gövdede geçmeyen bir etiket (`$pgt$`, `$pgt1$`, …) seçer ve
//!   gövde baytı baytına sarılır. Nihai ifade yine `composed_is_clean` ile taranır (olmaması gereken kayma → alan hatası).
//! - **Kimlik** (Karar 3): fonksiyonda ALTER/DROP/COMMENT `şema.ad(giriş tipleri)` ile yazılır; giriş = IN, INOUT,
//!   VARIADIC. PG 14+ procedure'de OUT argümanları da çağrı imzasına girdiğinden procedure kimliği TÜM argümanları açık
//!   modla yazar (`IN int, OUT text`): modsuz liste aşırı yüklemede belirsiz kalabilir (bkz. `ident_sql`); PG 11–13'te
//!   procedure OUT alamaz, açık mod yine kabul edilir.
//! - **`OR REPLACE` sınırları** (Karar 2): `needs_recreate`; `recreate` yalnızca bu liste doluysa anlamlıdır.
//! - **Serbest SQL**: yalnız argüman `DEFAULT` ifadeleri (`check_free_sql` + `wrap`); tipler `validate_type` ile katı.
//! - Alter "değişti mi" kararı tiplerde `canonical_type`, COST/ROWS'ta sayısal değer ile verilir (yüklü `format_type`
//!   ve `procost` biçimine karşı sabit nokta); argüman varsayılanları ve ayarlar metin olarak karşılaştırılır.

use super::freesql::{check_free_sql, composed_is_clean, wrap};
use super::settings::{guc_assignment_sql, validate_settings, validate_written_settings};
use super::type_canon::canonical_type;
use super::type_syntax::validate_type;
use super::{
    BuildCtx, check_ident, check_old_ident, check_old_ref, check_old_schema_ident, check_schema_ident, check_text, err, stmt,
};
use crate::model::{ArgMode, FieldError, FunctionArg, FunctionDef, FunctionReturn, ObjectKind, Parallel, Volatility};
use crate::plan::{Plan, Risk};
use crate::quote::{qualified, quote_ident, quote_literal};

pub const RECREATE_MSG: &str = "Bu değişiklik fonksiyonu yeniden oluşturmayı gerektirir; 'Yeniden oluştur' seçin.";
pub const RECREATE_DROP_NOTE: &str = "GRANT'lar ve bağımlılar kaybolur; bağımlı nesne varsa PG reddeder";
pub const RECREATE_GRANT_WARNING: &str = "Yeniden oluşturma: yetkiler (GRANT) yeniden verilmelidir";
pub const SECURITY_DEFINER_WARNING: &str = "Fonksiyon sahibinin yetkileriyle çalışır; SET search_path = … önerilir.";
pub const CASCADE_WARNING: &str = "Bağımlı nesneler de silinir";
pub const READ_ONLY_MSG: &str = "Bu nesne desteklenmeyen özellik içerdiği için salt okunurdur; değiştirilemez ve silinemez.";
/// Gövde üst sınırı (bayt).
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// COST/ROWS sayısal metninin üst sınırı (karakter).
const MAX_NUMBER_CHARS: usize = 32;
const PROCEDURE_VERSION: &str = "Procedure PG 11+ gerektirir";
const PROCEDURE_OUT_VERSION: &str = "Procedure OUT argümanı PG 14+ gerektirir";
const NOT_FOR_PROCEDURE: &str = "Procedure için kullanılamaz.";
const COMPOSED_BROKEN: &str = "Gövde ya da varsayılan ifadeler, yerleştirildiği ifadeyi bozuyor";

/// Gövdeyi saran dolar etiketi: `$pgt$`, `$pgt1$`, `$pgt2$` … içinden, `gövde + etiket` metninde İLK geçişi tam gövde
/// sonunda olan ilki. Bu, "gövdede geçmez" koşulundan güçlüdür: gövde etiketin önekiyle bitse (`x $pgt`) kapanış erken
/// eşleşirdi. PG'nin dolar tırnak sözcükleyicisi kapanışı gövdenin ilk eşleşen alt dizisinde bulur (öncesindeki
/// tanımlayıcı karakterlerine bakmaz); tarayıcımız da aynı kuralı izler.
///
/// Doğrusal: gövde TEK geçişte taranır (`$pgt` geçişleri; ardından `$` → n = 0, sıfırsız rakamlar + `$` → n), en küçük
/// kullanılmayan n seçilir. Etiketlerin tek kenarlığı (hem önek hem sonek) `$` olduğundan `gövde + etiket` içindeki
/// erken geçiş yalnızca gövde etiketin son `$` hariç önekiyle (`$pgt`, `$pgt1` …) bittiğinde olur; o aday atlanır
/// (en çok bir aday). Daha katı "gövde etiketin hiçbir önekiyle bitmesin" kuralı `$` ile biten her gövdede sonsuz
/// döngü olurdu.
pub fn dollar_tag(body: &str) -> String {
    let mut used = std::collections::HashSet::new();
    for (i, _) in body.match_indices("$pgt") {
        let rest = &body.as_bytes()[i + 4..];
        let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
        if rest.get(digits) != Some(&b'$') {
            continue;
        }
        if digits == 0 {
            used.insert(0usize);
        } else if rest[0] != b'0'
            && let Ok(n) = std::str::from_utf8(&rest[..digits]).unwrap_or_default().parse::<usize>()
        {
            // Üretilen etiketler sıfırsız yazılır (`$pgt01$` hiçbir adayla çakışmaz); taşan sayı aday olamaz.
            used.insert(n);
        }
    }
    let tag = |n: usize| if n == 0 { "$pgt$".to_string() } else { format!("$pgt{n}$") };
    let mut n = 0;
    loop {
        if !used.contains(&n) {
            let t = tag(n);
            if !body.ends_with(&t[..t.len() - 1]) {
                return t;
            }
        }
        n += 1;
    }
}

/// Dizi eki taşımayan ama VARIADIC olabilen çok biçimli tipler (`anyarray`, `anycompatiblearray`, `"any"`).
fn is_polymorphic_variadic(t: &str) -> bool {
    let c = canonical_type(t);
    let c = c.strip_prefix("pg_catalog.").unwrap_or(&c);
    matches!(c, "anyarray" | "anycompatiblearray" | "\"any\"" | "any")
}

/// Dil adı: sade ASCII ad (`[A-Za-z_][A-Za-z0-9_]*`) tırnaksız yazılmış gibi küçük harfe çevrilir (`PLPGSQL` →
/// `plpgsql`; `pg_language` adları küçük harflidir), diğer her ad olduğu gibi kalır. Yerleştirme yine `quote_ident`.
fn language_name(l: &str) -> String {
    let mut chars = l.chars();
    let plain = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if plain { l.to_ascii_lowercase() } else { l.to_string() }
}

fn is_input(a: &FunctionArg) -> bool {
    a.mode != ArgMode::Out
}

fn is_output(a: &FunctionArg) -> bool {
    matches!(a.mode, ArgMode::Out | ArgMode::Inout)
}

/// `şema.ad(giriş tipi, …)` (Karar 3; OUT hariç). Tipler doğrulanmış varsayılır.
pub fn signature_sql(schema: &str, name: &str, args: &[FunctionArg]) -> String {
    let types: Vec<&str> = args.iter().filter(|a| is_input(a)).map(|a| a.type_name.as_str()).collect();
    format!("{}({})", qualified(schema, name), types.join(", "))
}

/// ALTER/DROP/COMMENT hedefi: fonksiyonda `signature_sql`; procedure'de TÜM argümanlar açık modla (`IN tip`, `INOUT
/// tip`, `VARIADIC tip`, `OUT tip`). PG 14+'da modsuz liste hem giriş hem tüm argüman imzasıyla eşlenir: `p(int, text)`
/// yanında `p(int, OUT text)` varsa "not unique" hatası verir; açık `IN`/`OUT` her zaman tek nesneyi seçer. PG 11–13
/// açık modu da kabul eder (OUT zaten yoktur; Docker'da 11/13/14/18 doğrulandı).
fn ident_sql(kind: ObjectKind, schema: &str, name: &str, args: &[FunctionArg]) -> String {
    if kind == ObjectKind::Procedure {
        let types: Vec<String> = args
            .iter()
            .map(|a| {
                let mode = match a.mode {
                    ArgMode::In => "IN",
                    ArgMode::Out => "OUT",
                    ArgMode::Inout => "INOUT",
                    ArgMode::Variadic => "VARIADIC",
                };
                format!("{mode} {}", a.type_name)
            })
            .collect();
        format!("{}({})", qualified(schema, name), types.join(", "))
    } else {
        signature_sql(schema, name, args)
    }
}

fn keyword(kind: ObjectKind) -> &'static str {
    if kind == ObjectKind::Procedure { "PROCEDURE" } else { "FUNCTION" }
}

fn check_kind(kind: ObjectKind) -> Result<(), Vec<FieldError>> {
    match kind {
        ObjectKind::Function | ObjectKind::Procedure => Ok(()),
        _ => Err(vec![err("kind", "Yalnızca Function ya da Procedure.")]),
    }
}

/// `[OUT|INOUT|VARIADIC ][<q ad> ]<tip>[ DEFAULT (<ifade>\n)]` (`IN` yazılmaz).
fn arg_sql(a: &FunctionArg) -> String {
    let mode = match a.mode {
        ArgMode::In => "",
        ArgMode::Out => "OUT ",
        ArgMode::Inout => "INOUT ",
        ArgMode::Variadic => "VARIADIC ",
    };
    let name = a.name.as_ref().map(|n| format!("{} ", quote_ident(n))).unwrap_or_default();
    let default = a.default.as_ref().map(|d| format!(" DEFAULT {}", wrap(d))).unwrap_or_default();
    format!("{mode}{name}{}{default}", a.type_name)
}

fn returns_sql(r: &FunctionReturn) -> String {
    match r {
        FunctionReturn::Type { type_name, setof } => format!("RETURNS {}{type_name}", if *setof { "SETOF " } else { "" }),
        FunctionReturn::Table { columns } => {
            let cols: Vec<String> = columns.iter().map(|c| format!("{} {}", quote_ident(&c.name), c.type_name)).collect();
            format!("RETURNS TABLE ({})", cols.join(", "))
        }
        FunctionReturn::Trigger => "RETURNS trigger".into(),
        FunctionReturn::Void => "RETURNS void".into(),
    }
}

fn volatility_sql(v: Volatility) -> &'static str {
    match v {
        Volatility::Volatile => "VOLATILE",
        Volatility::Stable => "STABLE",
        Volatility::Immutable => "IMMUTABLE",
    }
}

fn parallel_sql(p: Parallel) -> &'static str {
    match p {
        Parallel::Unsafe => "UNSAFE",
        Parallel::Restricted => "RESTRICTED",
        Parallel::Safe => "SAFE",
    }
}

/// `CREATE [OR REPLACE] FUNCTION|PROCEDURE <şema.ad>(…) …` — ad/şema çağırandan (OR REPLACE eski adla çalışır), diğer
/// her alan `d`'den. `OR REPLACE` belirtilmeyen öznitelikleri varsayılana çevirdiği için hepsi her seferinde yazılır
/// (varsayılan olanlar — CALLED ON NULL INPUT, SECURITY INVOKER, NOT LEAKPROOF — yazılmaz; anlamı aynıdır).
fn define_sql(kind: ObjectKind, replace: bool, schema: &str, name: &str, d: &FunctionDef) -> String {
    let verb = if replace { "CREATE OR REPLACE" } else { "CREATE" };
    let args: Vec<String> = d.args.iter().map(arg_sql).collect();
    let mut s = format!("{verb} {} {}({})", keyword(kind), qualified(schema, name), args.join(", "));
    let function = kind == ObjectKind::Function;
    if function && let Some(r) = &d.returns {
        s.push(' ');
        s.push_str(&returns_sql(r));
    }
    s.push_str(&format!(" LANGUAGE {}", quote_ident(&language_name(&d.language))));
    if function {
        s.push(' ');
        s.push_str(volatility_sql(d.volatility));
        if d.strict {
            s.push_str(" STRICT");
        }
    }
    if d.security_definer {
        s.push_str(" SECURITY DEFINER");
    }
    if function {
        if d.leakproof {
            s.push_str(" LEAKPROOF");
        }
        s.push_str(&format!(" PARALLEL {}", parallel_sql(d.parallel)));
        if let Some(c) = &d.cost {
            s.push_str(&format!(" COST {c}"));
        }
        if let Some(r) = &d.rows {
            s.push_str(&format!(" ROWS {r}"));
        }
    }
    for g in &d.settings {
        s.push_str(&format!(" SET {}", guc_assignment_sql(g)));
    }
    let tag = dollar_tag(&d.body);
    s.push_str(&format!(" AS {tag}{}{tag}", d.body));
    s
}

fn comment_sql(kind: ObjectKind, ident: &str, comment: &Option<String>) -> String {
    match comment {
        Some(c) => format!("COMMENT ON {} {ident} IS {}", keyword(kind), quote_literal(c)),
        None => format!("COMMENT ON {} {ident} IS NULL", keyword(kind)),
    }
}

fn owner_sql(kind: ObjectKind, ident: &str, owner: &str) -> String {
    format!("ALTER {} {ident} OWNER TO {}", keyword(kind), quote_ident(owner))
}

/// Sayısal metin: `rakam+ ('.' rakam+)?`, değeri > 0 (ifadeye tırnaksız yazılır; dilbilgisi güvenlik kapısıdır).
/// Yükleyici `procost`/`prorows` metnini de bununla sınar (geçmeyen değer salt okunur gerekçesidir).
pub(crate) fn parse_positive(v: &str) -> Option<f64> {
    let (int, frac) = match v.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (v, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if v.len() > MAX_NUMBER_CHARS || !digits(int) || frac.is_some_and(|f| !digits(f)) {
        return None;
    }
    v.parse::<f64>().ok().filter(|n| *n > 0.0 && n.is_finite())
}

fn check_positive(path: &str, v: &str, errs: &mut Vec<FieldError>) {
    if parse_positive(v).is_none() {
        errs.push(err(path, "Pozitif bir sayı olmalı (ör. 100 ya da 0.5)."));
    }
}

fn same_number(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x == y || matches!((parse_positive(x), parse_positive(y)), (Some(p), Some(q)) if p == q),
        (None, None) => true,
        _ => false,
    }
}

fn same_type(a: &str, b: &str) -> bool {
    a == b || canonical_type(a) == canonical_type(b)
}

fn same_return(a: &Option<FunctionReturn>, b: &Option<FunctionReturn>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(FunctionReturn::Type { type_name: t1, setof: s1 }), Some(FunctionReturn::Type { type_name: t2, setof: s2 })) => {
            s1 == s2 && same_type(t1, t2)
        }
        (Some(FunctionReturn::Table { columns: c1 }), Some(FunctionReturn::Table { columns: c2 })) => {
            c1.len() == c2.len() && c1.iter().zip(c2).all(|(x, y)| x.name == y.name && same_type(&x.type_name, &y.type_name))
        }
        (Some(FunctionReturn::Trigger), Some(FunctionReturn::Trigger)) | (Some(FunctionReturn::Void), Some(FunctionReturn::Void)) => true,
        _ => false,
    }
}

fn same_arg(a: &FunctionArg, b: &FunctionArg) -> bool {
    a.mode == b.mode && a.name == b.name && a.default == b.default && same_type(&a.type_name, &b.type_name)
}

/// `CREATE [OR REPLACE]` ifadesine yazılan alanlardan biri değişti mi (ad/şema/sahip/yorum hariç)?
fn definition_changed(old: &FunctionDef, new: &FunctionDef) -> bool {
    language_name(&old.language) != language_name(&new.language)
        || old.args.len() != new.args.len()
        || old.args.iter().zip(&new.args).any(|(a, b)| !same_arg(a, b))
        || !same_return(&old.returns, &new.returns)
        || old.body != new.body
        || old.volatility != new.volatility
        || old.strict != new.strict
        || old.security_definer != new.security_definer
        || old.leakproof != new.leakproof
        || old.parallel != new.parallel
        || !same_number(&old.cost, &new.cost)
        || !same_number(&old.rows, &new.rows)
        || old.settings != new.settings
}

/// OR REPLACE ile yapılamayan değişiklikler (Karar 2), alan yollarıyla; boş = OR REPLACE yeterli.
///
/// - giriş argüman sayısı değişimi → `args`; giriş tipi (kanonik) → `args[i].typeName`; var olan giriş adının
///   değişimi/kaldırılması → `args[i].name` (adsız girişe ad vermek serbest; PG kuralı); varsayılan kaldırma →
///   `args[i].default` (ekleme/değiştirme serbest);
/// - çıkış parametreleri (OUT/INOUT; sıra, ad, kanonik tip) değişimi → `args` (dönüş satır tipini değiştirir);
/// - dönüş (tür, SETOF, kanonik tip, TABLE kolonları) değişimi → `returns`.
///
/// Function ↔ Procedure dönüşümü ayrı `ObjectDef` varyantlarıdır: dağıtıcı (`build::alter`) bunu tür uyuşmazlığı
/// olarak reddeder (silip yeniden oluşturma iki ayrı işlemdir).
pub fn needs_recreate(_kind: ObjectKind, old: &FunctionDef, new: &FunctionDef) -> Vec<FieldError> {
    let mut e = vec![];
    let oin: Vec<&FunctionArg> = old.args.iter().filter(|a| is_input(a)).collect();
    let nin: Vec<(usize, &FunctionArg)> = new.args.iter().enumerate().filter(|(_, a)| is_input(a)).collect();
    if oin.len() != nin.len() {
        e.push(err("args", RECREATE_MSG));
    } else {
        for (o, (i, n)) in oin.iter().zip(&nin) {
            if !same_type(&o.type_name, &n.type_name) {
                e.push(err(&format!("args[{i}].typeName"), RECREATE_MSG));
            } else if o.name.is_some() && o.name != n.name {
                e.push(err(&format!("args[{i}].name"), RECREATE_MSG));
            }
            if o.default.is_some() && n.default.is_none() {
                e.push(err(&format!("args[{i}].default"), RECREATE_MSG));
            }
        }
    }
    // Çıkış listesi (OUT/INOUT): sayı, mod, ad, kanonik tip. Her iki tarafta INOUT olan ve adı/tipi giriş olarak zaten
    // `args[i].name|typeName` ile raporlanmış argüman ayrıca `args` üretmez (aynı değişiklik iki kez gösterilmesin).
    let oo: Vec<&FunctionArg> = old.args.iter().filter(|a| is_output(a)).collect();
    let no: Vec<(usize, &FunctionArg)> = new.args.iter().enumerate().filter(|(_, a)| is_output(a)).collect();
    let reported = |i: usize| e.iter().any(|x| x.path == format!("args[{i}].name") || x.path == format!("args[{i}].typeName"));
    let outs_changed = oo.len() != no.len()
        || oo.iter().zip(&no).any(|(o, (i, n))| {
            let same = o.mode == n.mode && o.name == n.name && same_type(&o.type_name, &n.type_name);
            !same && !(o.mode == ArgMode::Inout && n.mode == ArgMode::Inout && reported(*i))
        });
    if outs_changed && !e.iter().any(|x| x.path == "args") {
        e.push(err("args", RECREATE_MSG));
    }
    if !same_return(&old.returns, &new.returns) {
        e.push(err("returns", RECREATE_MSG));
    }
    e
}

/// Tanım (CREATE [OR REPLACE] ifadesi) doğrulaması: dil, argümanlar, dönüş, öznitelikler, gövde, ayarlar. Alter'da
/// `old` verilir: değişmeyen (aynı konumda aynı) argüman/TABLE kolon adları ve dil yüklü kimliktir → gevşek
/// `check_old_ref`; ayarlarda yalnızca YAZILAN (değişen) girdiler katı denetlenir.
fn validate_definition(ctx: &BuildCtx, kind: ObjectKind, d: &FunctionDef, old: Option<&FunctionDef>, e: &mut Vec<FieldError>) {
    let procedure = kind == ObjectKind::Procedure;
    if procedure && ctx.version_num < 110000 {
        e.push(err("kind", PROCEDURE_VERSION));
    }
    // Dil: ad kuralı + C/internal reddi (sunucu varlığı doğrular; lookup kümesine üyelik aranmaz).
    let before = e.len();
    if old.is_some_and(|o| o.language == d.language) {
        check_old_ref("language", &d.language, e);
    } else {
        check_ident("language", &d.language, e);
    }
    if e.len() == before && matches!(d.language.to_ascii_lowercase().as_str(), "c" | "internal") {
        e.push(err("language", "C ve internal dilli fonksiyonlar desteklenmez."));
    }

    // Argümanlar.
    let mut names: Vec<&str> = vec![];
    let mut saw_default = false;
    for (i, a) in d.args.iter().enumerate() {
        let path = |f: &str| format!("args[{i}].{f}");
        if let Some(n) = &a.name {
            let before = e.len();
            if old.and_then(|o| o.args.get(i)).is_some_and(|oa| oa.name.as_ref() == Some(n)) {
                check_old_ref(&path("name"), n, e);
            } else {
                check_ident(&path("name"), n, e);
            }
            if e.len() == before && names.contains(&n.as_str()) {
                e.push(err(&path("name"), "Argüman adı birden çok kez kullanılmış."));
            }
            names.push(n);
        }
        let type_ok = match validate_type(&a.type_name) {
            Ok(()) => true,
            Err(m) => {
                e.push(err(&path("typeName"), m));
                false
            }
        };
        match &a.default {
            Some(_) if !is_input(a) => e.push(err(&path("default"), "Yalnızca giriş argümanlarının varsayılanı olabilir.")),
            Some(x) => {
                check_free_sql(&path("default"), x, e);
                saw_default = true;
            }
            None if is_input(a) && saw_default => {
                e.push(err(&path("default"), "Varsayılanlı argümandan sonraki giriş argümanlarının da varsayılanı olmalı."))
            }
            None => {}
        }
        if a.mode == ArgMode::Variadic {
            if d.args[i + 1..].iter().any(is_input) {
                e.push(err(&path("mode"), "VARIADIC yalnızca son giriş argümanı olabilir."));
            }
            if type_ok && !a.type_name.ends_with(']') && !is_polymorphic_variadic(&a.type_name) {
                e.push(err(&path("typeName"), "VARIADIC argümanın tipi dizi olmalı (ör. integer[])."));
            }
        }
        if procedure && a.mode == ArgMode::Out && ctx.version_num < 140000 {
            e.push(err(&path("mode"), PROCEDURE_OUT_VERSION));
        }
    }

    // Dönüş.
    let mut set_returning = false;
    match (&d.returns, procedure) {
        (Some(_), true) => e.push(err("returns", "Procedure dönüş tipi alamaz.")),
        (None, true) => {}
        (None, false) => e.push(err("returns", "Fonksiyonun dönüş tipi seçilmeli.")),
        (Some(FunctionReturn::Type { type_name, setof }), false) => {
            set_returning = *setof;
            if let Err(m) = validate_type(type_name) {
                e.push(err("returns.typeName", m));
            }
        }
        (Some(FunctionReturn::Table { columns }), false) => {
            set_returning = true;
            if d.args.iter().any(is_output) {
                e.push(err("returns", "RETURNS TABLE ile OUT/INOUT argüman birlikte kullanılamaz."));
            }
            if columns.is_empty() {
                e.push(err("returns.columns", "TABLE dönüşünde en az bir kolon olmalı."));
            }
            let old_cols = match old.map(|o| &o.returns) {
                Some(Some(FunctionReturn::Table { columns })) => columns.as_slice(),
                _ => &[],
            };
            for (i, c) in columns.iter().enumerate() {
                let np = format!("returns.columns[{i}].name");
                let before = e.len();
                if old_cols.get(i).is_some_and(|oc| oc.name == c.name) {
                    check_old_ref(&np, &c.name, e);
                } else {
                    check_ident(&np, &c.name, e);
                }
                if e.len() == before && names.contains(&c.name.as_str()) {
                    e.push(err(&np, "Kolon adı bir argüman ya da başka bir kolonla aynı."));
                }
                names.push(&c.name);
                if let Err(m) = validate_type(&c.type_name) {
                    e.push(err(&format!("returns.columns[{i}].typeName"), m));
                }
            }
        }
        (Some(FunctionReturn::Trigger), false) => {
            if !d.args.is_empty() {
                e.push(err("args", "Trigger fonksiyonu argüman alamaz (argümanlar trigger tanımında verilir)."));
            }
        }
        (Some(FunctionReturn::Void), false) => {}
    }

    // Öznitelikler.
    if procedure {
        let checks = [
            ("volatility", d.volatility != Volatility::Volatile),
            ("strict", d.strict),
            ("leakproof", d.leakproof),
            ("parallel", d.parallel != Parallel::Unsafe),
            ("cost", d.cost.is_some()),
            ("rows", d.rows.is_some()),
        ];
        for (path, bad) in checks {
            if bad {
                e.push(err(path, NOT_FOR_PROCEDURE));
            }
        }
    } else {
        if let Some(c) = &d.cost {
            check_positive("cost", c, e);
        }
        match &d.rows {
            Some(_) if !set_returning => e.push(err("rows", "ROWS yalnızca küme dönen (SETOF/TABLE) fonksiyonda kullanılır.")),
            Some(r) => check_positive("rows", r, e),
            None => {}
        }
    }

    // Gövde (Karar 1: içerik taranmaz).
    if d.body.contains('\0') {
        e.push(err("body", "Gövde NUL karakteri içeremez."));
    } else if d.body.trim().is_empty() {
        e.push(err("body", "Gövde boş olamaz."));
    } else if d.body.len() > MAX_BODY_BYTES {
        e.push(err("body", "Gövde en çok 1 MiB olabilir."));
    }

    // Ayarlar: OR REPLACE tüm listeyi yazar; değişmeyen girdiler yüklü değerdir (yalnız NUL denetlenir).
    match old {
        None => validate_settings(&d.settings, e),
        Some(o) => {
            validate_written_settings(&o.settings, &d.settings, e);
            for (i, s) in d.settings.iter().enumerate() {
                if o.settings.contains(s) && (s.name.contains('\0') || s.value.contains('\0')) {
                    e.push(err(&format!("settings[{i}].value"), "Değer NUL karakteri içeremez."));
                }
            }
        }
    }
}

/// Nihai ifade taraması (yalnızca diğer her şey geçerliyse anlamlıdır). Hata `body`'ye yazılır: varsayılan ifadeleri
/// önceden `check_free_sql` + `wrap` ile tek başına temizdir, adlar/tipler/ayarlar tırnaklıdır ya da katı dilbilgisinden
/// geçmiştir; geriye kalan tek serbest parça gövde ve etiketidir (olmaması gereken durum).
fn check_composed(sql: impl FnOnce() -> String, e: &mut Vec<FieldError>) {
    if e.is_empty() && !composed_is_clean(&sql()) {
        e.push(err("body", COMPOSED_BROKEN));
    }
}

/// Oluşturma doğrulaması (tüm alanlar katı).
pub fn validate(ctx: &BuildCtx, kind: ObjectKind, d: &FunctionDef) -> Vec<FieldError> {
    if let Err(e) = check_kind(kind) {
        return e;
    }
    let mut e = vec![];
    check_schema_ident("schema", &d.schema, &mut e);
    check_ident("name", &d.name, &mut e);
    if let Some(o) = &d.owner {
        check_ident("owner", o, &mut e);
    }
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    validate_definition(ctx, kind, d, None, &mut e);
    check_composed(|| define_sql(kind, false, &d.schema, &d.name, d), &mut e);
    e
}

/// Sıra: CREATE, COMMENT (doluysa), OWNER (doluysa, **en sonda**). `SECURITY DEFINER` plan uyarısı üretir.
pub fn create(ctx: &BuildCtx, kind: ObjectKind, d: &FunctionDef) -> Result<Plan, Vec<FieldError>> {
    let errs = validate(ctx, kind, d);
    if !errs.is_empty() {
        return Err(errs);
    }
    let id = ident_sql(kind, &d.schema, &d.name, &d.args);
    let mut statements = vec![stmt(define_sql(kind, false, &d.schema, &d.name, d), Risk::Safe, None)];
    if d.comment.is_some() {
        statements.push(stmt(comment_sql(kind, &id, &d.comment), Risk::Safe, None));
    }
    if let Some(o) = &d.owner {
        statements.push(stmt(owner_sql(kind, &id, o), Risk::Safe, None));
    }
    let warnings = if d.security_definer { vec![SECURITY_DEFINER_WARNING.to_string()] } else { vec![] };
    Ok(Plan { statements, warnings })
}

/// Eski (yüklü) tarafın ifadeye giren alanları: şema/ad gevşek, imza tipleri katı dilbilgisi (tırnaksız yazılır).
fn validate_old(prefix: &str, d: &FunctionDef, e: &mut Vec<FieldError>) {
    check_old_schema_ident(&format!("{prefix}schema"), &d.schema, e);
    check_old_ident(&format!("{prefix}name"), &d.name, e);
    for (i, a) in d.args.iter().enumerate() {
        if let Err(m) = validate_type(&a.type_name) {
            e.push(err(&format!("{prefix}args[{i}].typeName"), m));
        }
    }
}

/// Sıra (Global Constraints): (1) tanım değiştiyse `CREATE OR REPLACE` **eski ad ve şemayla** (yeni tanımın diğer
/// alanlarıyla) ya da `recreate` ise `DROP (eski imza)` + `CREATE (yeni ad/şema)`; (2) RENAME (eski imza); (3) SET
/// SCHEMA (yeni ad, eski imza tipleri); (4) COMMENT (yeni ad/şema, yeni imza); (5) OWNER (**en sonda**).
///
/// `recreate` yalnızca `needs_recreate` doluysa uygulanır; değilse sessizce yok sayılır. Yeniden kurmada sahip her
/// zaman yazılır (`new.owner ?? old.owner`), yorum doluysa. `editable = false` eski tanımda her değişiklik `object`
/// hatasıdır (değişiklik yoksa boş plan).
pub fn alter(ctx: &BuildCtx, kind: ObjectKind, old: &FunctionDef, new: &FunctionDef) -> Result<Plan, Vec<FieldError>> {
    check_kind(kind)?;
    let mut e = vec![];
    validate_old("old.", old, &mut e);
    if !e.is_empty() {
        return Err(e);
    }
    let def_changed = definition_changed(old, new);
    let name_changed = old.name != new.name;
    let schema_changed = old.schema != new.schema;
    let owner_changed = new.owner.is_some() && new.owner != old.owner;
    let comment_changed = old.comment != new.comment;
    if !old.editable && (def_changed || name_changed || schema_changed || owner_changed || comment_changed) {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    let needed = needs_recreate(kind, old, new);
    let recreate = new.recreate && !needed.is_empty();
    if !recreate {
        e.extend(needed);
    }

    if schema_changed {
        check_schema_ident("schema", &new.schema, &mut e);
    }
    if name_changed {
        check_ident("name", &new.name, &mut e);
    }
    match &new.owner {
        Some(o) if owner_changed => check_ident("owner", o, &mut e),
        _ if recreate => {
            if let Some(o) = &old.owner {
                check_old_ref("owner", o, &mut e);
            }
        }
        _ => {}
    }
    if let Some(c) = &new.comment
        && (recreate || comment_changed)
    {
        check_text("comment", c, &mut e);
    }
    if recreate || def_changed {
        validate_definition(ctx, kind, new, Some(old), &mut e);
    } else {
        // Yeni imza (COMMENT/OWNER hedefi) tanım değişmese de ifadeye yazılır: tipler kanonik olarak eşit olsa bile
        // metinleri katı dilbilgisinden geçmeli.
        for (i, a) in new.args.iter().enumerate() {
            if let Err(m) = validate_type(&a.type_name) {
                e.push(err(&format!("args[{i}].typeName"), m));
            }
        }
    }
    // İfade yalnızca doğrulanmış alanlarla kurulur (`quote_*` NUL'u çağırana bırakır).
    let mut define = None;
    if e.is_empty() {
        define = if recreate {
            Some(define_sql(kind, false, &new.schema, &new.name, new))
        } else if def_changed {
            Some(define_sql(kind, true, &old.schema, &old.name, new))
        } else {
            None
        };
        if let Some(sql) = &define {
            check_composed(|| sql.clone(), &mut e);
        }
    }
    if !e.is_empty() {
        return Err(e);
    }

    let mut statements = vec![];
    let mut warnings = vec![];
    let new_id = ident_sql(kind, &new.schema, &new.name, &new.args);
    if recreate {
        statements.push(stmt(
            format!("DROP {} {}", keyword(kind), ident_sql(kind, &old.schema, &old.name, &old.args)),
            Risk::DataLoss,
            Some(RECREATE_DROP_NOTE),
        ));
        statements.push(stmt(define.unwrap_or_default(), Risk::Safe, None));
        if new.comment.is_some() {
            statements.push(stmt(comment_sql(kind, &new_id, &new.comment), Risk::Safe, None));
        }
        if let Some(o) = new.owner.as_ref().or(old.owner.as_ref()) {
            statements.push(stmt(owner_sql(kind, &new_id, o), Risk::Safe, None));
        }
        warnings.push(RECREATE_GRANT_WARNING.to_string());
    } else {
        if let Some(sql) = define {
            statements.push(stmt(sql, Risk::Safe, None));
        }
        if name_changed {
            statements.push(stmt(
                format!(
                    "ALTER {} {} RENAME TO {}",
                    keyword(kind),
                    ident_sql(kind, &old.schema, &old.name, &old.args),
                    quote_ident(&new.name)
                ),
                Risk::Safe,
                None,
            ));
        }
        if schema_changed {
            statements.push(stmt(
                format!(
                    "ALTER {} {} SET SCHEMA {}",
                    keyword(kind),
                    ident_sql(kind, &old.schema, &new.name, &old.args),
                    quote_ident(&new.schema)
                ),
                Risk::Safe,
                None,
            ));
        }
        if comment_changed {
            statements.push(stmt(comment_sql(kind, &new_id, &new.comment), Risk::Safe, None));
        }
        // OWNER TO en sonda: kendisi eski sahibin haklarını ister; önceki ifadeler bu yüzden yetki bulur (View kuralı).
        if let Some(o) = &new.owner
            && owner_changed
        {
            statements.push(stmt(owner_sql(kind, &new_id, o), Risk::Safe, None));
        }
    }
    if new.security_definer && !old.security_definer {
        warnings.push(SECURITY_DEFINER_WARNING.to_string());
    }
    Ok(Plan { statements, warnings })
}

/// `DROP FUNCTION|PROCEDURE <eski imza> [CASCADE]`; kimlik (şema/ad) dağıtıcıda (`validate_identity`) doğrulanır,
/// imza tipleri burada. Salt okunur tanım silinemez.
pub fn drop(kind: ObjectKind, d: &FunctionDef, cascade: bool) -> Result<Plan, Vec<FieldError>> {
    check_kind(kind)?;
    if !d.editable {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    let mut e = vec![];
    validate_old("", d, &mut e);
    if !e.is_empty() {
        return Err(e);
    }
    let id = ident_sql(kind, &d.schema, &d.name, &d.args);
    Ok(if cascade {
        Plan {
            statements: vec![stmt(format!("DROP {} {id} CASCADE", keyword(kind)), Risk::DataLoss, Some(CASCADE_WARNING))],
            warnings: vec![CASCADE_WARNING.into()],
        }
    } else {
        Plan { statements: vec![stmt(format!("DROP {} {id}", keyword(kind)), Risk::Destructive, None)], warnings: vec![] }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::freesql::composed_is_clean;
    use crate::build::{alter as balter, assert_single_statements, create as bcreate, drop as bdrop, render};
    use crate::model::{ArgMode, ConfigSetting, FunctionReturn, ObjectDef, Parallel, ReturnColumn, Volatility};
    use crate::plan::Risk;

    fn f() -> FunctionDef {
        FunctionDef {
            schema: "public".into(),
            name: "f".into(),
            owner: None,
            comment: None,
            language: "plpgsql".into(),
            args: vec![],
            returns: Some(FunctionReturn::Type { type_name: "integer".into(), setof: false }),
            body: "BEGIN RETURN 1; END".into(),
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
    fn p() -> FunctionDef {
        FunctionDef { name: "p".into(), returns: None, body: "BEGIN NULL; END".into(), ..f() }
    }
    fn arg(mode: ArgMode, name: Option<&str>, t: &str, default: Option<&str>) -> FunctionArg {
        FunctionArg { mode, name: name.map(str::to_string), type_name: t.into(), default: default.map(str::to_string) }
    }
    fn a_in(n: &str, t: &str) -> FunctionArg {
        arg(ArgMode::In, Some(n), t, None)
    }
    fn ty(t: &str, setof: bool) -> Option<FunctionReturn> {
        Some(FunctionReturn::Type { type_name: t.into(), setof })
    }
    fn tbl(cols: &[(&str, &str)]) -> Option<FunctionReturn> {
        Some(FunctionReturn::Table {
            columns: cols.iter().map(|(n, t)| ReturnColumn { name: (*n).into(), type_name: (*t).into() }).collect(),
        })
    }
    fn set(n: &str, v: &str) -> ConfigSetting {
        ConfigSetting { name: n.into(), value: v.into() }
    }
    fn ctx_v(version_num: u32) -> BuildCtx {
        BuildCtx { version_num, ..Default::default() }
    }
    fn ctx() -> BuildCtx {
        ctx_v(160000)
    }
    fn of(d: &FunctionDef) -> ObjectDef {
        ObjectDef::Function(d.clone())
    }
    fn op(d: &FunctionDef) -> ObjectDef {
        ObjectDef::Procedure(d.clone())
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn paths(errs: Vec<FieldError>) -> Vec<String> {
        errs.into_iter().map(|e| e.path).collect()
    }
    fn alter_ok(old: &FunctionDef, new: &FunctionDef) -> Plan {
        let p = balter(&of(old), &of(new), &ctx()).unwrap();
        assert_single_statements(&p);
        p
    }
    fn alter_err(old: &FunctionDef, new: &FunctionDef) -> Vec<String> {
        paths(balter(&of(old), &of(new), &ctx()).unwrap_err())
    }
    fn create_err(d: &FunctionDef) -> Vec<String> {
        paths(bcreate(&of(d), &ctx()).unwrap_err())
    }

    #[test]
    fn dollar_tag_picks_first_tag_absent_from_body() {
        assert_eq!(dollar_tag("SELECT 1"), "$pgt$");
        assert_eq!(dollar_tag("x $$ y $_$ z"), "$pgt$");
        assert_eq!(dollar_tag("a $pgt$ b"), "$pgt1$");
        assert_eq!(dollar_tag("$pgt$ $pgt1$"), "$pgt2$");
        assert_eq!(dollar_tag("$pgt$$pgt1$$pgt2$"), "$pgt3$");
        // Gövde sonu etiketin önekiyse kapanış erken eşleşirdi (`x $pgt` + `$pgt$`): sonraki etiket.
        assert_eq!(dollar_tag("x $pgt"), "$pgt1$");
        assert_eq!(dollar_tag("x $pgt1"), "$pgt$");
        // Büyük harf farklı etikettir (PG etiketleri harfe duyarlı).
        assert_eq!(dollar_tag("$PGT$"), "$pgt$");
        for body in ["", "$", "$pgt", "pgt$", "$pgt$", "$pgt$x$pgt1$ $pgt"] {
            let t = dollar_tag(body);
            let joined = format!("{body}{t}");
            assert_eq!(joined.find(&t), Some(body.len()), "{body:?} → {t}");
        }
    }

    #[test]
    fn dollar_tag_is_linear_on_tag_heavy_bodies() {
        // 200k farklı `$pgtN$` + `$pgt$`: eski aday başına kopyalama O(n²) idi; tek geçiş anında döner.
        let mut body = String::from("$pgt$");
        for i in 1..=200_000 {
            body.push_str(&format!("$pgt{i}$"));
        }
        assert_eq!(dollar_tag(&body), "$pgt200001$");
        // Önde sıfırlı / sayısal olmayan ekler üretilen etiketlerle çakışmaz; gövde `$` ile bitebilir.
        assert_eq!(dollar_tag("$pgt01$ $pgtx$ $pgt"), "$pgt1$");
        assert_eq!(dollar_tag("x $"), "$pgt$");
        assert_eq!(dollar_tag("$pgt$ x $pgt1"), "$pgt2$");
    }

    #[test]
    fn variadic_accepts_polymorphic_any_types() {
        for t in ["anyarray", "anycompatiblearray", "\"any\"", "pg_catalog.anyarray", "ANYARRAY"] {
            let d = FunctionDef { args: vec![arg(ArgMode::Variadic, Some("a"), t, None)], ..f() };
            assert!(bcreate(&of(&d), &ctx()).is_ok(), "{t}");
        }
        for t in ["anyelement", "integer", "\"anyarray\"x"] {
            let d = FunctionDef { args: vec![arg(ArgMode::Variadic, Some("a"), t, None)], ..f() };
            assert_eq!(create_err(&d), ["args[0].typeName"], "{t}");
        }
    }

    #[test]
    fn inout_rename_reports_only_the_name() {
        let old = FunctionDef { args: vec![arg(ArgMode::Inout, Some("a"), "integer", None)], ..f() };
        let new = FunctionDef { args: vec![arg(ArgMode::Inout, Some("b"), "integer", None)], ..f() };
        assert_eq!(paths(needs_recreate(ObjectKind::Function, &old, &new)), ["args[0].name"]);
        let typed = FunctionDef { args: vec![arg(ArgMode::Inout, Some("a"), "bigint", None)], ..f() };
        assert_eq!(paths(needs_recreate(ObjectKind::Function, &old, &typed)), ["args[0].typeName"]);
        // Mod değişimi (INOUT → IN) çıkış listesini değiştirir: `args`.
        let mode = FunctionDef { args: vec![a_in("a", "integer")], ..f() };
        assert_eq!(paths(needs_recreate(ObjectKind::Function, &old, &mode)), ["args"]);
    }

    #[test]
    fn plain_language_names_are_lowercased() {
        let sql = |l: &str| bcreate(&of(&FunctionDef { language: l.into(), ..f() }), &ctx()).unwrap().statements[0].sql.clone();
        assert!(sql("PLPGSQL").contains(" LANGUAGE plpgsql "));
        assert!(sql("PlPgSql").contains(" LANGUAGE plpgsql "));
        assert!(sql("SQL").contains(" LANGUAGE \"sql\" "));
        // Sade olmayan ad olduğu gibi tırnaklanır.
        assert!(sql("My Lang").contains(" LANGUAGE \"My Lang\" "));
        assert!(sql("plpgsql_ş").contains(" LANGUAGE \"plpgsql_ş\" "));
        // Yalnızca harf büyüklüğü farkı değişiklik değildir.
        assert!(alter_ok(&f(), &FunctionDef { language: "PLPGSQL".into(), ..f() }).statements.is_empty());
    }

    #[test]
    fn signature_uses_input_types_only() {
        let args = vec![
            a_in("a", "integer"),
            arg(ArgMode::Out, Some("o"), "text", None),
            arg(ArgMode::Inout, Some("io"), "\"My T\"", None),
            arg(ArgMode::Variadic, None, "numeric[]", None),
        ];
        assert_eq!(signature_sql("public", "f", &args), "public.f(integer, \"My T\", numeric[])");
        assert_eq!(signature_sql("Ş", "select", &[]), "\"Ş\".\"select\"()");
    }

    #[test]
    fn create_variants() {
        let cases: Vec<(&str, ObjectDef)> = vec![
            ("skaler", of(&f())),
            ("setof", of(&FunctionDef { returns: ty("text", true), body: "SELECT 'a'".into(), language: "sql".into(), ..f() })),
            (
                "table",
                of(&FunctionDef {
                    returns: tbl(&[("id", "integer"), ("Ad", "text")]),
                    language: "sql".into(),
                    body: "SELECT 1, 'x'".into(),
                    ..f()
                }),
            ),
            (
                "trigger",
                of(&FunctionDef { name: "tf".into(), returns: Some(FunctionReturn::Trigger), body: "BEGIN RETURN NEW; END".into(), ..f() }),
            ),
            ("void", of(&FunctionDef { returns: Some(FunctionReturn::Void), body: "BEGIN END".into(), ..f() })),
            ("procedure", op(&FunctionDef { args: vec![a_in("x", "integer")], ..p() })),
            (
                "out/inout/variadic",
                of(&FunctionDef {
                    args: vec![
                        a_in("a", "integer"),
                        arg(ArgMode::Out, Some("o"), "text", None),
                        arg(ArgMode::Inout, Some("io"), "bigint", None),
                        arg(ArgMode::Variadic, Some("rest"), "numeric[]", None),
                    ],
                    returns: ty("record", false),
                    ..f()
                }),
            ),
            (
                "default",
                of(&FunctionDef {
                    args: vec![
                        a_in("a", "integer"),
                        arg(ArgMode::In, Some("b"), "text", Some("'x'")),
                        arg(ArgMode::In, None, "integer", Some("1 + 2 -- yorum\n")),
                    ],
                    ..f()
                }),
            ),
            ("set", of(&FunctionDef { settings: vec![set("search_path", "\"$user\", public"), set("work_mem", "64MB")], ..f() })),
            ("security definer", of(&FunctionDef { security_definer: true, ..f() })),
            ("leakproof", of(&FunctionDef { leakproof: true, volatility: Volatility::Immutable, strict: true, ..f() })),
            ("parallel safe", of(&FunctionDef { parallel: Parallel::Safe, volatility: Volatility::Stable, ..f() })),
            ("cost/rows", of(&FunctionDef { returns: ty("integer", true), cost: Some("10".into()), rows: Some("2.5".into()), ..f() })),
            (
                "sahip/yorum",
                of(&FunctionDef { owner: Some("Bob".into()), comment: Some("it's".into()), args: vec![a_in("a", "integer")], ..f() }),
            ),
            (
                "procedure tam",
                op(&FunctionDef {
                    security_definer: true,
                    settings: vec![set("work_mem", "1MB")],
                    owner: Some("o".into()),
                    comment: Some("c".into()),
                    ..p()
                }),
            ),
        ];
        let mut out = String::new();
        for (label, d) in cases {
            let p = bcreate(&d, &ctx()).unwrap();
            assert_single_statements(&p);
            assert!(p.statements.iter().all(|s| s.risk == Risk::Safe && s.transactional), "{label}");
            out.push_str(&format!("# {label}\n{}\n", render(&p)));
        }
        insta::assert_snapshot!(out);
        let pl = bcreate(&of(&f()), &ctx()).unwrap();
        assert_eq!(
            sqls(&pl),
            ["CREATE FUNCTION public.f() RETURNS integer LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE AS $pgt$BEGIN RETURN 1; END$pgt$"]
        );
        assert!(pl.warnings.is_empty());
        let pl = bcreate(&of(&FunctionDef { security_definer: true, ..f() }), &ctx()).unwrap();
        assert_eq!(pl.warnings, [SECURITY_DEFINER_WARNING]);
        let pl = bcreate(&op(&FunctionDef { owner: Some("o".into()), ..p() }), &ctx()).unwrap();
        assert_eq!(
            sqls(&pl),
            ["CREATE PROCEDURE public.p() LANGUAGE plpgsql AS $pgt$BEGIN NULL; END$pgt$", "ALTER PROCEDURE public.p() OWNER TO o"]
        );
    }

    #[test]
    fn create_validation_matrix() {
        let long = "x".repeat(MAX_BODY_BYTES + 1);
        let cases: Vec<(FunctionDef, &str)> = vec![
            (FunctionDef { returns: None, ..f() }, "returns"),
            (FunctionDef { returns: ty("integer; DROP TABLE t", false), ..f() }, "returns.typeName"),
            (FunctionDef { returns: ty("SETOF integer", false), ..f() }, "returns.typeName"),
            (FunctionDef { returns: tbl(&[]), ..f() }, "returns.columns"),
            (FunctionDef { returns: tbl(&[("a\nb", "int")]), ..f() }, "returns.columns[0].name"),
            (FunctionDef { returns: tbl(&[("a", "int x")]), ..f() }, "returns.columns[0].typeName"),
            (FunctionDef { returns: tbl(&[("a", "int"), ("a", "text")]), ..f() }, "returns.columns[1].name"),
            (FunctionDef { args: vec![arg(ArgMode::Out, Some("o"), "int", None)], returns: tbl(&[("a", "int")]), ..f() }, "returns"),
            (FunctionDef { args: vec![a_in("a", "int")], returns: tbl(&[("a", "int")]), ..f() }, "returns.columns[0].name"),
            (FunctionDef { args: vec![a_in("a", "int"), a_in("a", "text")], ..f() }, "args[1].name"),
            (FunctionDef { args: vec![a_in("a\0", "int")], ..f() }, "args[0].name"),
            (FunctionDef { args: vec![a_in("a", "")], ..f() }, "args[0].typeName"),
            (FunctionDef { args: vec![a_in("a", "int DEFAULT 1")], ..f() }, "args[0].typeName"),
            (FunctionDef { args: vec![arg(ArgMode::In, Some("a"), "int", Some("1); DROP TABLE t; --"))], ..f() }, "args[0].default"),
            (FunctionDef { args: vec![arg(ArgMode::In, Some("a"), "int", Some("'a\\b'"))], ..f() }, "args[0].default"),
            (
                FunctionDef { args: vec![arg(ArgMode::Out, Some("a"), "int", Some("1"))], returns: ty("int", false), ..f() },
                "args[0].default",
            ),
            (FunctionDef { args: vec![arg(ArgMode::In, Some("a"), "int", Some("1")), a_in("b", "int")], ..f() }, "args[1].default"),
            (FunctionDef { args: vec![arg(ArgMode::Variadic, Some("a"), "int[]", None), a_in("b", "int")], ..f() }, "args[0].mode"),
            (FunctionDef { args: vec![arg(ArgMode::Variadic, Some("a"), "int", None)], ..f() }, "args[0].typeName"),
            (FunctionDef { rows: Some("10".into()), ..f() }, "rows"),
            (FunctionDef { returns: ty("int", true), rows: Some("0".into()), ..f() }, "rows"),
            (FunctionDef { cost: Some("0".into()), ..f() }, "cost"),
            (FunctionDef { cost: Some("-1".into()), ..f() }, "cost"),
            (FunctionDef { cost: Some("1e3".into()), ..f() }, "cost"),
            (FunctionDef { cost: Some("10 ROWS 5".into()), ..f() }, "cost"),
            (FunctionDef { language: "c".into(), ..f() }, "language"),
            (FunctionDef { language: "C".into(), ..f() }, "language"),
            (FunctionDef { language: "internal".into(), ..f() }, "language"),
            (FunctionDef { language: "plpgsql\n".into(), ..f() }, "language"),
            (FunctionDef { language: String::new(), ..f() }, "language"),
            (FunctionDef { body: String::new(), ..f() }, "body"),
            (FunctionDef { body: "  \n".into(), ..f() }, "body"),
            (FunctionDef { body: "a\0b".into(), ..f() }, "body"),
            (FunctionDef { body: long, ..f() }, "body"),
            (FunctionDef { settings: vec![set("Work_Mem", "1MB")], ..f() }, "settings[0].name"),
            (FunctionDef { settings: vec![set("work_mem", "a\nb")], ..f() }, "settings[0].value"),
            (FunctionDef { name: "a\nb".into(), ..f() }, "name"),
            (FunctionDef { schema: "pg_catalog".into(), ..f() }, "schema"),
            (FunctionDef { owner: Some(String::new()), ..f() }, "owner"),
            (FunctionDef { comment: Some("a\0".into()), ..f() }, "comment"),
            // Trigger fonksiyonu argüman alamaz.
            (FunctionDef { returns: Some(FunctionReturn::Trigger), args: vec![a_in("a", "int")], ..f() }, "args"),
        ];
        for (d, want) in cases {
            assert_eq!(create_err(&d), [want], "{d:?}");
            assert_eq!(paths(validate(&ctx(), ObjectKind::Function, &d)), [want]);
        }
        // Sınırda gövde (tam 1 MiB) kabul edilir.
        let edge = FunctionDef { body: "x".repeat(MAX_BODY_BYTES), ..f() };
        assert!(bcreate(&of(&edge), &ctx()).is_ok());
        // Ondalıklı COST/ROWS kabul edilir.
        assert!(bcreate(&of(&FunctionDef { cost: Some("0.5".into()), ..f() }), &ctx()).is_ok());
        // VARIADIC'ten sonra OUT argümanı serbest; DEFAULT'lu VARIADIC serbest.
        let ok = FunctionDef {
            args: vec![arg(ArgMode::Variadic, Some("a"), "int[]", Some("'{}'")), arg(ArgMode::Out, Some("o"), "int", None)],
            ..f()
        };
        assert!(bcreate(&of(&ok), &ctx()).is_ok());
        // OUT argümanı DEFAULT kuralında sayılmaz; sonraki DEFAULT'suz giriş argümanı yine hata.
        let bad = FunctionDef {
            args: vec![arg(ArgMode::In, Some("a"), "int", Some("1")), arg(ArgMode::Out, Some("o"), "int", None), a_in("b", "int")],
            ..f()
        };
        assert_eq!(create_err(&bad), ["args[2].default"]);
    }

    #[test]
    fn procedure_rules() {
        // PG 10'da Procedure yok.
        assert_eq!(paths(bcreate(&op(&p()), &ctx_v(100000)).unwrap_err()), ["kind"]);
        assert!(bcreate(&op(&p()), &ctx_v(110000)).is_ok());
        // OUT argümanı PG 14+.
        let out = FunctionDef { args: vec![arg(ArgMode::Out, Some("o"), "int", None)], ..p() };
        assert_eq!(paths(bcreate(&op(&out), &ctx_v(130000)).unwrap_err()), ["args[0].mode"]);
        assert!(bcreate(&op(&out), &ctx_v(140000)).is_ok());
        // Fonksiyona özgü öznitelikler Procedure'de alan hatası.
        for (d, want) in [
            (FunctionDef { returns: Some(FunctionReturn::Void), ..p() }, "returns"),
            (FunctionDef { volatility: Volatility::Stable, ..p() }, "volatility"),
            (FunctionDef { strict: true, ..p() }, "strict"),
            (FunctionDef { leakproof: true, ..p() }, "leakproof"),
            (FunctionDef { parallel: Parallel::Safe, ..p() }, "parallel"),
            (FunctionDef { cost: Some("1".into()), ..p() }, "cost"),
            (FunctionDef { rows: Some("1".into()), ..p() }, "rows"),
        ] {
            assert_eq!(paths(bcreate(&op(&d), &ctx()).unwrap_err()), [want], "{d:?}");
        }
        // Procedure kimliği tüm argümanları AÇIK modla yazar (PG 14+ çağrı imzası OUT'ları içerir; modsuz
        // `p(int, text)` yanında `p(int, OUT text)` aşırı yüklemesi varsa PG "not unique" der — Docker'da gözlendi);
        // fonksiyon yalnızca giriş tiplerini yazar.
        let both = FunctionDef { args: vec![a_in("a", "int"), arg(ArgMode::Out, Some("o"), "text", None)], ..p() };
        let d = bdrop(&op(&both), false, &ctx()).unwrap();
        assert_eq!(sqls(&d), ["DROP PROCEDURE public.p(IN int, OUT text)"]);
        let all_in = FunctionDef { args: vec![a_in("a", "int"), a_in("b", "text")], ..p() };
        assert_eq!(sqls(&bdrop(&op(&all_in), false, &ctx()).unwrap()), ["DROP PROCEDURE public.p(IN int, IN text)"]);
        let modes = FunctionDef {
            args: vec![arg(ArgMode::Inout, Some("b"), "text", None), arg(ArgMode::Variadic, Some("c"), "int[]", None)],
            ..p()
        };
        assert_eq!(sqls(&bdrop(&op(&modes), false, &ctx()).unwrap()), ["DROP PROCEDURE public.p(INOUT text, VARIADIC int[])"]);
        let fun = FunctionDef { returns: ty("text", false), ..both.clone() };
        assert_eq!(sqls(&bdrop(&of(&fun), false, &ctx()).unwrap()), ["DROP FUNCTION public.p(int)"]);
    }

    #[test]
    fn alter_single_changes() {
        let base = FunctionDef { owner: Some("a".into()), comment: Some("c".into()), args: vec![a_in("x", "integer")], ..f() };
        let cases: Vec<(&str, FunctionDef, Vec<&str>)> = vec![
            (
                "gövde",
                FunctionDef { body: "BEGIN RETURN 2; END".into(), ..base.clone() },
                vec![
                    "CREATE OR REPLACE FUNCTION public.f(x integer) RETURNS integer LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE AS $pgt$BEGIN RETURN 2; END$pgt$",
                ],
            ),
            ("ad", FunctionDef { name: "g".into(), ..base.clone() }, vec!["ALTER FUNCTION public.f(integer) RENAME TO g"]),
            ("şema", FunctionDef { schema: "app".into(), ..base.clone() }, vec!["ALTER FUNCTION public.f(integer) SET SCHEMA app"]),
            ("sahip", FunctionDef { owner: Some("b".into()), ..base.clone() }, vec!["ALTER FUNCTION public.f(integer) OWNER TO b"]),
            ("sahip None", FunctionDef { owner: None, ..base.clone() }, vec![]),
            ("yorum", FunctionDef { comment: Some("d".into()), ..base.clone() }, vec!["COMMENT ON FUNCTION public.f(integer) IS 'd'"]),
            ("yorum None", FunctionDef { comment: None, ..base.clone() }, vec!["COMMENT ON FUNCTION public.f(integer) IS NULL"]),
            (
                "öznitelik",
                FunctionDef { volatility: Volatility::Stable, strict: true, parallel: Parallel::Safe, cost: Some("5".into()), ..base.clone() },
                vec![
                    "CREATE OR REPLACE FUNCTION public.f(x integer) RETURNS integer LANGUAGE plpgsql STABLE STRICT PARALLEL SAFE COST 5 AS $pgt$BEGIN RETURN 1; END$pgt$",
                ],
            ),
            (
                "ayar",
                FunctionDef { settings: vec![set("work_mem", "1MB"), set("search_path", "public")], ..base.clone() },
                vec![
                    "CREATE OR REPLACE FUNCTION public.f(x integer) RETURNS integer LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE SET work_mem = '1MB' SET search_path = 'public' AS $pgt$BEGIN RETURN 1; END$pgt$",
                ],
            ),
            (
                "default ekleme",
                FunctionDef { args: vec![arg(ArgMode::In, Some("x"), "integer", Some("1"))], ..base.clone() },
                vec![
                    "CREATE OR REPLACE FUNCTION public.f(x integer DEFAULT (1\n)) RETURNS integer LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE AS $pgt$BEGIN RETURN 1; END$pgt$",
                ],
            ),
            // Yalnızca yükleme alanları üretici tarafından okunmaz; takma ad tip (int4 = integer) değişiklik değildir.
            ("değişmez", FunctionDef { unsupported: vec!["x".into()], args: vec![a_in("x", "int4")], ..base.clone() }, vec![]),
        ];
        let mut out = String::new();
        for (label, new, want) in cases {
            let pl = alter_ok(&base, &new);
            assert_eq!(sqls(&pl), want, "{label}");
            assert!(pl.statements.iter().all(|s| s.risk == Risk::Safe), "{label}");
            assert!(pl.warnings.is_empty(), "{label}");
            out.push_str(&format!("# {label}\n{}\n", render(&pl)));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn alter_combined_order() {
        let old = FunctionDef { owner: Some("a".into()), comment: Some("c".into()), args: vec![a_in("x", "integer")], ..f() };
        let new = FunctionDef {
            schema: "App".into(),
            name: "select".into(),
            body: "BEGIN RETURN x; END".into(),
            owner: Some("b".into()),
            comment: None,
            args: vec![a_in("x", "int")],
            ..old.clone()
        };
        let pl = alter_ok(&old, &new);
        assert_eq!(
            sqls(&pl),
            [
                "CREATE OR REPLACE FUNCTION public.f(x int) RETURNS integer LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE AS $pgt$BEGIN RETURN x; END$pgt$",
                "ALTER FUNCTION public.f(integer) RENAME TO \"select\"",
                "ALTER FUNCTION public.\"select\"(integer) SET SCHEMA \"App\"",
                "COMMENT ON FUNCTION \"App\".\"select\"(int) IS NULL",
                "ALTER FUNCTION \"App\".\"select\"(int) OWNER TO b",
            ]
        );
        insta::assert_snapshot!(render(&pl));
        // Procedure aynı sıra, PROCEDURE sözcüğüyle.
        let old = FunctionDef { owner: Some("a".into()), ..p() };
        let new = FunctionDef { name: "q".into(), body: "BEGIN END".into(), owner: Some("b".into()), ..old.clone() };
        let pl = balter(&op(&old), &op(&new), &ctx()).unwrap();
        assert_eq!(
            sqls(&pl),
            [
                "CREATE OR REPLACE PROCEDURE public.p() LANGUAGE plpgsql AS $pgt$BEGIN END$pgt$",
                "ALTER PROCEDURE public.p() RENAME TO q",
                "ALTER PROCEDURE public.q() OWNER TO b",
            ]
        );
    }

    #[test]
    fn needs_recreate_table() {
        let base = FunctionDef {
            args: vec![a_in("a", "integer"), arg(ArgMode::In, Some("b"), "text", Some("'x'")), arg(ArgMode::Out, Some("o"), "integer", None)],
            returns: ty("integer", false),
            ..f()
        };
        let (a0, a1, a2) = (base.args[0].clone(), base.args[1].clone(), base.args[2].clone());
        let with_args = |args: Vec<FunctionArg>| FunctionDef { args, ..base.clone() };
        let cases: Vec<(&str, FunctionDef, Vec<&str>)> = vec![
            ("imza tipi", with_args(vec![a_in("a", "bigint"), a1.clone(), a2.clone()]), vec!["args[0].typeName"]),
            ("argüman ekleme", with_args(vec![a0.clone(), a1.clone(), arg(ArgMode::In, Some("c"), "int", Some("1")), a2.clone()]), vec!["args"]),
            ("argüman silme", with_args(vec![a0.clone(), a2.clone()]), vec!["args"]),
            ("giriş adı", with_args(vec![a_in("z", "integer"), a1.clone(), a2.clone()]), vec!["args[0].name"]),
            ("giriş adı kaldırma", with_args(vec![arg(ArgMode::In, None, "integer", None), a1.clone(), a2.clone()]), vec!["args[0].name"]),
            ("default kaldırma", with_args(vec![a0.clone(), a_in("b", "text"), a2.clone()]), vec!["args[1].default"]),
            ("OUT tipi", with_args(vec![a0.clone(), a1.clone(), arg(ArgMode::Out, Some("o"), "text", None)]), vec!["args"]),
            ("OUT adı", with_args(vec![a0.clone(), a1.clone(), arg(ArgMode::Out, Some("p"), "integer", None)]), vec!["args"]),
            ("OUT kaldırma", with_args(vec![a0.clone(), a1.clone()]), vec!["args"]),
            ("IN → INOUT", with_args(vec![arg(ArgMode::Inout, Some("a"), "integer", None), a1.clone(), a2.clone()]), vec!["args"]),
            ("dönüş tipi", FunctionDef { returns: ty("bigint", false), ..base.clone() }, vec!["returns"]),
            ("SETOF", FunctionDef { returns: ty("integer", true), ..base.clone() }, vec!["returns"]),
            ("void", FunctionDef { returns: Some(FunctionReturn::Void), ..base.clone() }, vec!["returns"]),
            (
                "imza + dönüş",
                FunctionDef { returns: ty("text", false), args: vec![a_in("a", "bigint"), a1.clone(), a2.clone()], ..base.clone() },
                vec!["args[0].typeName", "returns"],
            ),
        ];
        let mut out = String::new();
        for (label, new, want) in cases {
            let got = needs_recreate(ObjectKind::Function, &base, &new);
            assert!(got.iter().all(|e| e.message == RECREATE_MSG), "{label}");
            assert_eq!(paths(got), want, "{label}");
            // `recreate` olmadan alter aynı alan hatalarını verir (PG'ye gitmez).
            assert_eq!(alter_err(&base, &new), want, "alter {label}");
            out.push_str(&format!("{label}: {want:?}\n"));
        }
        // OR REPLACE ile serbest: DEFAULT ekleme/değiştirme, tip takma adı, gövde/öznitelik.
        for (label, new) in [
            ("default değiştirme", with_args(vec![a0.clone(), arg(ArgMode::In, Some("b"), "text", Some("'y'")), a2.clone()])),
            ("default ekleme", with_args(vec![arg(ArgMode::In, Some("a"), "integer", Some("0")), a1.clone(), a2.clone()])),
            ("gövde", FunctionDef { body: "BEGIN o := 2; END".into(), strict: true, ..base.clone() }),
        ] {
            assert!(needs_recreate(ObjectKind::Function, &base, &new).is_empty(), "{label}");
            assert_eq!(alter_ok(&base, &new).statements.len(), 1, "{label}");
        }
        // Tip takma adı (int4 = integer) değişiklik değildir: boş plan.
        let alias = with_args(vec![a_in("a", "int4"), a1.clone(), a2.clone()]);
        assert!(needs_recreate(ObjectKind::Function, &base, &alias).is_empty());
        assert!(alter_ok(&base, &alias).statements.is_empty());
        // Adsız giriş argümanına ad vermek serbest (PG yalnız var olan adın değişimini reddeder).
        let unnamed = FunctionDef { args: vec![arg(ArgMode::In, None, "integer", None)], ..f() };
        let named = FunctionDef { args: vec![a_in("a", "integer")], ..f() };
        assert!(needs_recreate(ObjectKind::Function, &unnamed, &named).is_empty());
        // TABLE kolonları dönüş tipidir.
        let t = |n: &str| FunctionDef { returns: tbl(&[(n, "integer")]), ..f() };
        assert_eq!(paths(needs_recreate(ObjectKind::Function, &t("a"), &t("b"))), ["returns"]);
        assert!(needs_recreate(ObjectKind::Function, &t("a"), &FunctionDef { returns: tbl(&[("a", "int4")]), ..f() }).is_empty());
        insta::assert_snapshot!(out);
    }

    #[test]
    fn recreate_plan() {
        let old = FunctionDef { owner: Some("a".into()), comment: Some("c".into()), args: vec![a_in("x", "integer")], ..f() };
        let new = FunctionDef {
            schema: "app".into(),
            name: "g".into(),
            args: vec![a_in("x", "bigint")],
            returns: ty("bigint", false),
            owner: None,
            recreate: true,
            security_definer: true,
            ..old.clone()
        };
        let pl = alter_ok(&old, &new);
        assert_eq!(
            sqls(&pl),
            [
                "DROP FUNCTION public.f(integer)",
                "CREATE FUNCTION app.g(x bigint) RETURNS bigint LANGUAGE plpgsql VOLATILE SECURITY DEFINER PARALLEL UNSAFE AS $pgt$BEGIN RETURN 1; END$pgt$",
                "COMMENT ON FUNCTION app.g(bigint) IS 'c'",
                "ALTER FUNCTION app.g(bigint) OWNER TO a",
            ]
        );
        assert_eq!(pl.statements[0].risk, Risk::DataLoss);
        assert_eq!(pl.statements[0].note.as_deref(), Some(RECREATE_DROP_NOTE));
        assert!(pl.statements[1..].iter().all(|s| s.risk == Risk::Safe));
        assert_eq!(pl.risk(), Risk::DataLoss);
        assert_eq!(pl.warnings, [RECREATE_GRANT_WARNING, SECURITY_DEFINER_WARNING]);
        insta::assert_snapshot!(render(&pl));
        // Yorumsuz: yorum ifadesi yok; yeni sahip öncelikli.
        let new2 = FunctionDef { comment: None, owner: Some("b".into()), returns: ty("text", false), recreate: true, ..old.clone() };
        let pl = alter_ok(&old, &new2);
        assert_eq!(
            sqls(&pl),
            [
                "DROP FUNCTION public.f(integer)",
                "CREATE FUNCTION public.f(x integer) RETURNS text LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE AS $pgt$BEGIN RETURN 1; END$pgt$",
                "ALTER FUNCTION public.f(integer) OWNER TO b",
            ]
        );
        assert_eq!(pl.warnings, [RECREATE_GRANT_WARNING]);
    }

    #[test]
    fn recreate_without_need_is_ignored_silently() {
        let new = FunctionDef { recreate: true, body: "BEGIN RETURN 3; END".into(), ..f() };
        let pl = alter_ok(&f(), &new);
        assert_eq!(pl.statements.len(), 1);
        assert!(pl.statements[0].sql.starts_with("CREATE OR REPLACE FUNCTION public.f()"));
        assert!(pl.warnings.is_empty());
        let pl = alter_ok(&f(), &FunctionDef { recreate: true, ..f() });
        assert!(pl.statements.is_empty() && pl.warnings.is_empty());
    }

    #[test]
    fn security_definer_warning_only_when_enabled() {
        let on = FunctionDef { security_definer: true, ..f() };
        let pl = alter_ok(&f(), &on);
        assert_eq!(pl.warnings, [SECURITY_DEFINER_WARNING]);
        assert!(pl.statements[0].sql.contains(" SECURITY DEFINER "));
        let pl = alter_ok(&on, &FunctionDef { body: "BEGIN RETURN 5; END".into(), ..on.clone() });
        assert!(pl.warnings.is_empty());
        let pl = alter_ok(&on, &f());
        assert!(pl.warnings.is_empty());
        assert!(!pl.statements[0].sql.contains("SECURITY DEFINER"));
    }

    #[test]
    fn settings_change_rewrites_full_list() {
        let old = FunctionDef { settings: vec![set("work_mem", "1MB"), set("search_path", "public")], ..f() };
        let new = FunctionDef { settings: vec![set("work_mem", "2MB"), set("search_path", "public")], ..f() };
        let pl = alter_ok(&old, &new);
        assert_eq!(pl.statements.len(), 1);
        assert!(pl.statements[0].sql.contains(" SET work_mem = '2MB' SET search_path = 'public' AS "), "{}", pl.statements[0].sql);
        // Ayar kaldırma: listesiz OR REPLACE (PG proconfig'i boşaltır).
        let pl = alter_ok(&old, &f());
        assert!(!pl.statements[0].sql.contains(" SET "));
        // Yazılan yeni girdi doğrulanır.
        let bad = FunctionDef { settings: vec![set("work_mem", "a\nb")], ..f() };
        assert_eq!(alter_err(&f(), &bad), ["settings[0].value"]);
    }

    #[test]
    fn alter_noop_is_empty() {
        let x = FunctionDef {
            owner: Some("a".into()),
            comment: Some("c".into()),
            args: vec![a_in("a", "integer"), arg(ArgMode::In, Some("b"), "text", Some("'x'::text"))],
            settings: vec![set("work_mem", "1MB")],
            cost: Some("100".into()),
            security_definer: true,
            ..f()
        };
        let pl = alter_ok(&x, &x);
        assert!(pl.statements.is_empty() && pl.warnings.is_empty());
        // Sayısal metin eşdeğerliği: "100" = "100.0".
        let pl = alter_ok(&x, &FunctionDef { cost: Some("100.0".into()), ..x.clone() });
        assert!(pl.statements.is_empty());
        let pr = FunctionDef { owner: Some("a".into()), ..p() };
        assert!(balter(&op(&pr), &op(&pr), &ctx()).unwrap().statements.is_empty());
    }

    #[test]
    fn read_only_definition_rejects_changes_and_drop() {
        let ro = FunctionDef { editable: false, unsupported: vec!["C dili".into()], language: "c".into(), ..f() };
        // Değişiklik yoksa boş plan (sabit nokta); her değişiklik `object` hatası.
        assert!(alter_ok(&ro, &ro).statements.is_empty());
        for new in [
            FunctionDef { name: "g".into(), ..ro.clone() },
            FunctionDef { owner: Some("b".into()), ..ro.clone() },
            FunctionDef { comment: Some("k".into()), ..ro.clone() },
            FunctionDef { body: "x".into(), ..ro.clone() },
            FunctionDef { recreate: true, returns: ty("text", false), ..ro.clone() },
        ] {
            let e = balter(&of(&ro), &of(&new), &ctx()).unwrap_err();
            assert_eq!(paths(e.clone()), ["object"]);
            assert_eq!(e[0].message, READ_ONLY_MSG);
        }
        assert_eq!(paths(bdrop(&of(&ro), false, &ctx()).unwrap_err()), ["object"]);
        assert_eq!(paths(bdrop(&of(&ro), true, &ctx()).unwrap_err()), ["object"]);
    }

    #[test]
    fn drop_matrix() {
        let mut out = String::new();
        let args = vec![a_in("a", "integer"), arg(ArgMode::Out, Some("o"), "text", None), arg(ArgMode::Variadic, Some("r"), "text[]", None)];
        for (d, cascade, want, risk) in [
            (of(&f()), false, "DROP FUNCTION public.f()", Risk::Destructive),
            (of(&f()), true, "DROP FUNCTION public.f() CASCADE", Risk::DataLoss),
            (of(&FunctionDef { args: args.clone(), ..f() }), false, "DROP FUNCTION public.f(integer, text[])", Risk::Destructive),
            (of(&FunctionDef { name: "My \"F\"".into(), ..f() }), false, "DROP FUNCTION public.\"My \"\"F\"\"\"()", Risk::Destructive),
            (op(&p()), false, "DROP PROCEDURE public.p()", Risk::Destructive),
            (op(&p()), true, "DROP PROCEDURE public.p() CASCADE", Risk::DataLoss),
        ] {
            let pl = bdrop(&d, cascade, &ctx()).unwrap();
            assert_single_statements(&pl);
            assert_eq!(sqls(&pl), [want]);
            assert_eq!(pl.statements[0].risk, risk);
            assert_eq!(pl.warnings.is_empty(), !cascade);
            out.push_str(&render(&pl));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
        // Gövde/öznitelik doğrulanmaz; imza tipi doğrulanır (ifadeye yazılır).
        assert!(bdrop(&of(&FunctionDef { body: String::new(), cost: Some("x".into()), ..f() }), false, &ctx()).is_ok());
        let bad = FunctionDef { args: vec![a_in("a", "int); DROP TABLE t; --")], ..f() };
        assert_eq!(paths(bdrop(&of(&bad), false, &ctx()).unwrap_err()), ["args[0].typeName"]);
        assert_eq!(paths(bdrop(&of(&FunctionDef { schema: "pg_catalog".into(), ..f() }), false, &ctx()).unwrap_err()), ["schema"]);
    }

    #[test]
    fn hostile_bodies_stay_single_statement_and_byte_exact() {
        for body in [
            "$pgt$",
            "$pgt$; DROP TABLE t; $pgt$",
            "$pgt$ $pgt1$ $pgt2$",
            "$$; DROP TABLE t; $$",
            "$_$ x $_$",
            "SELECT 1; SELECT 2;",
            "SELECT 'it''s';",
            "SELECT 'unterminated",
            "SELECT '\\'",
            "BEGIN\r\n  RETURN 1;\r\nEND",
            "-- yorum",
            "/* açık yorum",
            "\"",
            "x $pgt",
            "Ünicode ş 日本 \u{202e}",
            "$",
            "1$$",
        ] {
            let d = FunctionDef { body: body.into(), ..f() };
            let tag = dollar_tag(body);
            assert!(!body.contains(&tag), "{body:?}");
            let want_tail = format!(" AS {tag}{body}{tag}");
            for pl in [
                bcreate(&of(&d), &ctx()).unwrap(),
                alter_ok(&f(), &d),
                alter_ok(&f(), &FunctionDef { returns: ty("text", false), recreate: true, ..d.clone() }),
            ] {
                assert_single_statements(&pl);
                let s = pl.statements.iter().find(|s| s.sql.starts_with("CREATE")).unwrap();
                assert!(s.sql.ends_with(&want_tail), "{body:?}: {}", s.sql);
                assert!(composed_is_clean(&s.sql), "{body:?}");
            }
        }
    }

    #[test]
    fn hostile_names_stay_single_statement_or_fail_on_field() {
        let long63 = "a".repeat(63);
        for n in ["f\"; DROP FUNCTION x(); --", "select", "Fonksiyon ş", "日本", long63.as_str(), "'", "\\", "/*", "$$", "$pgt$"] {
            let d = FunctionDef {
                schema: n.into(),
                name: n.into(),
                owner: Some(n.into()),
                comment: Some(n.into()),
                language: n.into(),
                args: vec![arg(ArgMode::In, Some(n), "integer", Some("1"))],
                settings: vec![set("app.x", n)],
                ..f()
            };
            let new = FunctionDef { name: format!("{}x", &n[..n.len().min(60)]), body: "BEGIN RETURN 2; END".into(), ..d.clone() };
            for pl in [
                bcreate(&of(&d), &ctx()).unwrap(),
                alter_ok(&d, &new),
                alter_ok(&d, &FunctionDef { returns: ty("text", false), recreate: true, ..new.clone() }),
                alter_ok(&f(), &FunctionDef { schema: "public".into(), args: vec![], ..d.clone() }),
                bdrop(&of(&d), true, &ctx()).unwrap(),
                bdrop(&of(&d), false, &ctx()).unwrap(),
            ] {
                assert!(!pl.statements.is_empty(), "{n:?}");
                assert_single_statements(&pl);
            }
        }
        let pl = bcreate(&of(&FunctionDef { name: "f\"; DROP FUNCTION x(); --".into(), ..f() }), &ctx()).unwrap();
        assert!(pl.statements[0].sql.starts_with("CREATE FUNCTION public.\"f\"\"; DROP FUNCTION x(); --\"() RETURNS"));
        // Eski taraf: kimlik `old.` önekiyle; gevşek (sıfır genişlikli ad yeniden adlandırılabilir), NUL/boş red.
        let zw = FunctionDef { name: "f\u{200b}x".into(), ..f() };
        assert_eq!(sqls(&alter_ok(&zw, &FunctionDef { name: "fx".into(), ..zw.clone() })), ["ALTER FUNCTION public.\"f\u{200b}x\"() RENAME TO fx"]);
        assert_eq!(alter_err(&f(), &zw), ["name"]);
        assert_eq!(alter_err(&FunctionDef { name: String::new(), ..f() }, &f()), ["old.name"]);
        assert_eq!(alter_err(&FunctionDef { schema: "pg_catalog".into(), ..f() }, &f()), ["old.schema"]);
        let bad_old = FunctionDef { args: vec![a_in("a", "int; x")], ..f() };
        assert_eq!(alter_err(&bad_old, &bad_old), ["old.args[0].typeName"]);
    }

    #[test]
    fn alter_validates_only_changed_or_written_fields() {
        // Değişmeyen yüklü (gevşek) argüman adı OR REPLACE'te yeniden yazılır, serbest; değişen ad katı.
        let old = FunctionDef { args: vec![a_in("a\u{200b}", "integer")], ..f() };
        assert_single_statements(&alter_ok(&old, &FunctionDef { body: "BEGIN RETURN 9; END".into(), ..old.clone() }));
        // Değişmeyen NUL'lu yorum: yalnızca yeniden kurmada (yeniden yazılır) hata.
        let old = FunctionDef { comment: Some("a\0".into()), ..f() };
        assert!(balter(&of(&old), &of(&FunctionDef { name: "g".into(), ..old.clone() }), &ctx()).is_ok());
        let r = FunctionDef { returns: ty("text", false), recreate: true, ..old.clone() };
        assert_eq!(alter_err(&old, &r), ["comment"]);
        // Yalnız ad değişince gövde doğrulanmaz.
        let old = FunctionDef { body: String::new(), ..f() };
        assert!(balter(&of(&old), &of(&FunctionDef { name: "g".into(), ..old.clone() }), &ctx()).is_ok());
    }
}
