//! Tip metninin karşılaştırma için kanonik biçimi (saf, toplam işlev; SQL üretmez). Alter "tip değişti mi" kararını
//! bununla verir: istemci takma ad yazsa da (`varchar(20)`) yüklemedeki `format_type` çıktısıyla
//! (`character varying(20)`) eşit sayılır ve "değiştirmeden düzenle → boş plan" korunur. Üretilen SQL her zaman
//! kullanıcının metnini kullanır; kanonik biçim yalnızca karşılaştırmadır.
//!
//! Kurallar:
//! - Boşluklar daraltılır; `(`, `)`, `,`, `[`, `]`, `.` çevresindeki boşluklar kaldırılır (`numeric (10, 2)` =
//!   `numeric(10,2)`); `)` ile sonraki sözcük arasında tek boşluk kalır (`timestamp(3) with time zone`).
//! - Yalnızca TIRNAKSIZ sözcükler küçük harfe çevrilir; tırnaklı kısımlar aynen kalır (`"MyType"` ≠ `mytype`,
//!   `"char"` ≠ `char`).
//! - Baştaki `pg_catalog.` yalnızca kalan ad bilinen bir yerleşik takma ad/kanonik ad ise atılır.
//! - Yerleşik takma adlar `format_type` çıktısına eşlenir (bkz. `canon_builtin`); tip değiştiricileri anlamlıdır
//!   (`timestamp(6)` ≠ `timestamp`, `numeric` ≠ `numeric(1000)`).
//! - Dizi ekleri (`[]`, `[n]`, `[][]`) tek `[]`'a indirgenir (PG boyut/uzunluk bilgisini saklamaz).
//!
//! Bilinen sınırlar (bilinçli): arama yoluna bağlı eşdeğerlik çözülmez (`public.mytype` ≠ `mytype`); tırnaklı
//! yerleşik adlar (`"int4"`) takma ad sayılmaz; `text` gibi takma adı olmayan tiplerde `pg_catalog.` atılmaz.

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// Tırnaksız sözcük (küçük harfli).
    Word(String),
    /// Tırnaklı ad, tırnaklarıyla birlikte aynen.
    Quoted(String),
    /// Rakam dizisi (isteğe bağlı başta `-`).
    Num(String),
    Punct(char),
}

fn tokenize(s: &str) -> Vec<Tok> {
    let c: Vec<char> = s.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '"' {
            let start = i;
            i += 1;
            while i < c.len() {
                if c[i] == '"' {
                    if c.get(i + 1) == Some(&'"') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(Tok::Quoted(c[start..i].iter().collect()));
        } else if ch.is_ascii_digit() || (ch == '-' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            i += 1;
            while i < c.len() && c[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Tok::Num(c[start..i].iter().collect()));
        } else if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_' || c[i] == '$') {
                i += 1;
            }
            out.push(Tok::Word(c[start..i].iter().collect::<String>().to_ascii_lowercase()));
        } else {
            out.push(Tok::Punct(ch));
            i += 1;
        }
    }
    out
}

fn render(toks: &[Tok]) -> String {
    let mut s = String::new();
    let mut prev: Option<&Tok> = None;
    for t in toks {
        let wordish = matches!(t, Tok::Word(_) | Tok::Quoted(_));
        let space = match prev {
            Some(Tok::Word(_) | Tok::Quoted(_)) => wordish,
            Some(Tok::Punct(')')) => wordish,
            _ => false,
        };
        if space {
            s.push(' ');
        }
        match t {
            Tok::Word(w) | Tok::Quoted(w) | Tok::Num(w) => s.push_str(w),
            Tok::Punct(p) => s.push(*p),
        }
        prev = Some(t);
    }
    s
}

/// Bilinen yerleşik ad (takma ad ya da kanonik baş sözcük): `pg_catalog.` yalnızca bunlarda atılır.
const BUILTIN_HEADS: &[&str] = &[
    "int", "int2", "int4", "int8", "integer", "smallint", "bigint", "bool", "boolean", "float", "float4", "float8",
    "real", "double", "decimal", "numeric", "varchar", "char", "character", "nchar", "national", "bpchar", "bit",
    "varbit", "timestamp", "timestamptz", "time", "timetz",
];

/// Yerleşik takma adı `format_type` biçimine eşler: (baş, değiştiriciler, saat dilimi eki) üçlüsünden metin.
/// Tanınmayan biçimler `None` (genel kanonik biçim kullanılır).
fn canon_builtin(words: &[String], mods: Option<&[String]>, tail: &[String]) -> Option<String> {
    let head = words.join(" ");
    let m = mods.map(|m| format!("({})", m.join(","))).unwrap_or_default();
    let tz = || -> Option<&'static str> {
        match tail.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
            [] | ["without", "time", "zone"] => Some("without time zone"),
            ["with", "time", "zone"] => Some("with time zone"),
            _ => None,
        }
    };
    let plain = |name: &str| tail.is_empty().then(|| format!("{name}{m}"));
    match head.as_str() {
        "int" | "int4" | "integer" => plain("integer"),
        "int2" | "smallint" => plain("smallint"),
        "int8" | "bigint" => plain("bigint"),
        "bool" | "boolean" => plain("boolean"),
        "float4" | "real" => plain("real"),
        "float8" | "double precision" => plain("double precision"),
        "float" if tail.is_empty() => match mods {
            None => Some("double precision".into()),
            Some([p]) => match p.parse::<u32>() {
                Ok(1..=24) => Some("real".into()),
                Ok(25..=53) => Some("double precision".into()),
                _ => None,
            },
            _ => None,
        },
        "decimal" | "numeric" => plain("numeric"),
        "varchar" | "char varying" | "character varying" | "national character varying"
        | "national char varying" => plain("character varying"),
        "char" | "character" | "nchar" | "national character" | "national char" => {
            tail.is_empty().then(|| format!("character{}", if m.is_empty() { "(1)" } else { &m }))
        }
        "bpchar" => plain("bpchar"),
        "varbit" | "bit varying" => plain("bit varying"),
        "bit" => tail.is_empty().then(|| format!("bit{}", if m.is_empty() { "(1)" } else { &m })),
        "timestamp" => tz().map(|z| format!("timestamp{m} {z}")),
        "timestamptz" => tail.is_empty().then(|| format!("timestamp{m} with time zone")),
        "time" => tz().map(|z| format!("time{m} {z}")),
        "timetz" => tail.is_empty().then(|| format!("time{m} with time zone")),
        _ => None,
    }
}

/// Tip metninin kanonik karşılaştırma biçimi (bkz. modül belgesi). Geçersiz metinde de paniklemez.
pub fn canonical_type(s: &str) -> String {
    let mut toks = tokenize(s);
    // Dizi eki: ilk `[`'den sonrası tek `[]`.
    let array = match toks.iter().position(|t| *t == Tok::Punct('[')) {
        Some(p) => {
            toks.truncate(p);
            true
        }
        None => false,
    };
    // `pg_catalog.` öneki yalnızca yerleşik adlarda atılır.
    if let [Tok::Word(schema), Tok::Punct('.'), Tok::Word(w), ..] = toks.as_slice()
        && schema == "pg_catalog"
        && BUILTIN_HEADS.contains(&w.as_str())
    {
        toks.drain(..2);
    }
    let mut out = builtin_form(&toks).unwrap_or_else(|| render(&toks));
    if array {
        out.push_str("[]");
    }
    out
}

/// Yalnızca tırnaksız sözcüklerden oluşan (isteğe bağlı tek değiştirici listeli) yerleşik biçim.
fn builtin_form(toks: &[Tok]) -> Option<String> {
    let mut words = vec![];
    let mut i = 0;
    while let Some(Tok::Word(w)) = toks.get(i) {
        words.push(w.clone());
        i += 1;
    }
    if words.is_empty() {
        return None;
    }
    let mut mods = None;
    if toks.get(i) == Some(&Tok::Punct('(')) {
        let mut nums = vec![];
        i += 1;
        loop {
            match toks.get(i) {
                Some(Tok::Num(n)) => nums.push(n.clone()),
                _ => return None,
            }
            i += 1;
            match toks.get(i) {
                Some(Tok::Punct(',')) => i += 1,
                Some(Tok::Punct(')')) => {
                    i += 1;
                    break;
                }
                _ => return None,
            }
        }
        mods = Some(nums);
    }
    let mut tail = vec![];
    while let Some(Tok::Word(w)) = toks.get(i) {
        tail.push(w.clone());
        i += 1;
    }
    if i != toks.len() {
        return None;
    }
    canon_builtin(&words, mods.as_deref(), &tail)
}

#[cfg(test)]
mod tests {
    use super::canonical_type as c;

    #[test]
    fn alias_table() {
        let cases: &[(&str, &str)] = &[
            ("int", "integer"),
            ("INT4", "integer"),
            ("integer", "integer"),
            ("int2", "smallint"),
            ("smallint", "smallint"),
            ("int8", "bigint"),
            ("BigInt", "bigint"),
            ("bool", "boolean"),
            ("float4", "real"),
            ("real", "real"),
            ("float(1)", "real"),
            ("float(24)", "real"),
            ("float(25)", "double precision"),
            ("float(53)", "double precision"),
            ("float", "double precision"),
            ("float8", "double precision"),
            ("double  precision", "double precision"),
            ("decimal", "numeric"),
            ("decimal(10,2)", "numeric(10,2)"),
            ("numeric (10, 2)", "numeric(10,2)"),
            ("numeric( 10 , -2 )", "numeric(10,-2)"),
            ("varchar", "character varying"),
            ("varchar(20)", "character varying(20)"),
            ("char varying(5)", "character varying(5)"),
            ("national character varying(5)", "character varying(5)"),
            ("Character Varying(20)", "character varying(20)"),
            ("char", "character(1)"),
            ("character", "character(1)"),
            ("nchar", "character(1)"),
            ("national character", "character(1)"),
            ("char(3)", "character(3)"),
            ("national char(3)", "character(3)"),
            ("bpchar", "bpchar"),
            ("varbit", "bit varying"),
            ("varbit(4)", "bit varying(4)"),
            ("bit varying(4)", "bit varying(4)"),
            ("bit", "bit(1)"),
            ("bit(8)", "bit(8)"),
            ("timestamp", "timestamp without time zone"),
            ("timestamp(3)", "timestamp(3) without time zone"),
            ("timestamp without time zone", "timestamp without time zone"),
            ("timestamptz", "timestamp with time zone"),
            ("timestamptz(3)", "timestamp(3) with time zone"),
            ("timestamp (3)  WITH time zone", "timestamp(3) with time zone"),
            ("time", "time without time zone"),
            ("time(6)", "time(6) without time zone"),
            ("timetz", "time with time zone"),
            ("timetz(2)", "time(2) with time zone"),
            ("time(2) with time zone", "time(2) with time zone"),
            ("pg_catalog.int4", "integer"),
            ("pg_catalog.varchar(20)", "character varying(20)"),
            ("text", "text"),
            ("TEXT", "text"),
            ("interval day to second(3)", "interval day to second(3)"),
            ("interval  year to month", "interval year to month"),
        ];
        for (a, want) in cases {
            assert_eq!(c(a), *want, "{a:?}");
        }
    }

    #[test]
    fn typmods_are_significant() {
        for (a, b) in [
            ("timestamp(6)", "timestamp"),
            ("numeric", "numeric(1000)"),
            ("varchar(20)", "varchar(21)"),
            ("char", "char(2)"),
            ("bit", "bit varying"),
            ("bpchar", "character(1)"),
            ("float(24)", "float(25)"),
            ("timestamp", "timestamptz"),
        ] {
            assert_ne!(c(a), c(b), "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn arrays_reduce_to_single_suffix() {
        for (a, want) in [
            ("int4[]", "integer[]"),
            ("integer[][]", "integer[]"),
            ("int[3]", "integer[]"),
            ("varchar(5)[2][2]", "character varying(5)[]"),
            ("\"My Type\"[][]", "\"My Type\"[]"),
            ("timestamptz [ ]", "timestamp with time zone[]"),
        ] {
            assert_eq!(c(a), want, "{a:?}");
        }
    }

    #[test]
    fn quoted_identifiers_are_kept_exactly() {
        assert_eq!(c("\"MyType\""), "\"MyType\"");
        assert_ne!(c("\"MyType\""), c("mytype"));
        assert_ne!(c("\"char\""), c("char"));
        assert_eq!(c("\"int4\""), "\"int4\"");
        assert_eq!(c("Public.\"My Type\""), "public.\"My Type\"");
        assert_eq!(c("public . MyType"), "public.mytype");
        assert_eq!(c("\"a\"\"b\""), "\"a\"\"b\"");
        // Arama yolu çözülmez; yerleşik olmayan adda pg_catalog atılmaz.
        assert_ne!(c("public.mytype"), c("mytype"));
        assert_eq!(c("pg_catalog.text"), "pg_catalog.text");
        assert_eq!(c("pg_catalog.\"int4\""), "pg_catalog.\"int4\"");
    }

    #[test]
    fn total_on_garbage() {
        for s in ["", " ", "garip tip ;", "\"unterminated", "((", "a(b)", "numeric(", "x'y", "€"] {
            let _ = c(s);
        }
        assert_eq!(c("  integer  "), "integer");
    }
}
