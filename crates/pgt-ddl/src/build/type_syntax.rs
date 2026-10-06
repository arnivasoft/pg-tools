//! Kolon veri tipi (`ColumnDef.dataType`) için katı dilbilgisi. Tip metni serbest SQL DEĞİLDİR: doğrulanırsa olduğu
//! gibi yerleştirilir, bu yüzden bu işlev bir GÜVENLİK KAPISIDIR; açıkça izin verilmeyen her şey reddedilir.
//!
//! ```text
//! tip        := sözcük ( ' ' sözcük )* dizi*          (sözcükler arasında TEK ASCII boşluk; baş/son boşluk yok)
//! sözcük     := ad ( '.' ad )? değiştirici?            (en çok `şema.tip`; değiştirici tüm tipte en çok bir kez)
//! ad         := [A-Za-z_][A-Za-z0-9_$]*  |  '"' ( [^"denetim] | '""' )+ '"'     (her biri en çok 63 bayt)
//! değiştirici:= '(' ' '* rakam+ ' '* ( ',' ' '* rakam+ ' '* )* ')'       (sözcüğe bitişik)
//! dizi       := '[' rakam* ']'                         (yalnızca sonda, bitişik)
//! ```
//!
//! Ek kısıtlar: en çok 128 karakter; tırnaksız adda `$$` yok (dolar tırnak açılışına benzemesin); NUL ve tüm denetim
//! karakterleri (Unicode Cc, ör. `\u{85}`) her yerde yasak. Anahtar sözcük denetimi yapılmaz: tipin varlığı sunucuda
//! denetlenir (uygulamada hata; transaction geri alınır).

use super::MAX_IDENT_BYTES;

/// Tip metninin karakter üst sınırı.
pub const MAX_TYPE_CHARS: usize = 128;

/// Tip ifadesini doğrular; hata iletisi doğrudan alan hatası olarak gösterilebilir (Türkçe).
pub fn validate_type(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("Veri tipi boş olamaz.".into());
    }
    if s.contains('\0') {
        return Err("Veri tipi NUL karakteri içeremez.".into());
    }
    let n = s.chars().count();
    if n > MAX_TYPE_CHARS {
        return Err(format!(
            "Veri tipi en çok {MAX_TYPE_CHARS} karakter olabilir (şu an {n})."
        ));
    }
    if s.chars().any(char::is_control) {
        return Err("Veri tipi denetim karakteri (satır sonu, sekme vb.) içeremez.".into());
    }
    if s.starts_with(' ') || s.ends_with(' ') {
        return Err("Veri tipinin başında ya da sonunda boşluk olamaz.".into());
    }
    let mut p = Parser {
        c: s.chars().collect(),
        i: 0,
    };
    p.parse()
}

struct Parser {
    c: Vec<char>,
    i: usize,
}

/// Dilbilgisi dışı karakter için ileti (sık saldırı karakterlerine özel açıklama).
fn unexpected(ch: char) -> String {
    match ch {
        ';' => "Veri tipi ';' içeremez.".into(),
        '\'' => "Veri tipi tek tırnak (') içeremez.".into(),
        '\\' => "Veri tipi '\\' içeremez.".into(),
        '-' => "Veri tipi '-' içeremez (yorum ve eksi işareti yasak).".into(),
        '/' => "Veri tipi '/' içeremez (yorum yasak).".into(),
        '$' => "Veri tipinde '$' yalnızca tırnaksız adın içinde kullanılabilir.".into(),
        '"' => "Çift tırnak (\") yalnızca tırnaklı adın başında olabilir.".into(),
        '(' | ')' => "Veri tipinde parantez yalnızca ada bitişik tek bir değiştirici listesinde olabilir, ör. numeric(10,2).".into(),
        '[' | ']' => "Veri tipinde köşeli parantez yalnızca sondaki dizi ekinde ([] ya da [n]) olabilir.".into(),
        '.' => "Veri tipinde nokta yalnızca iki ad arasında olabilir (şema.tip).".into(),
        ' ' => "Veri tipinde sözcükler arasında tek boşluk olmalı.".into(),
        c if c.is_ascii_digit() => "Veri tipinde ad rakamla başlayamaz.".into(),
        c if !c.is_ascii() => format!(
            "Veri tipinde '{c}' karakteri tırnaksız kullanılamaz; tırnaksız ad yalnızca ASCII harf, rakam, _ ve $ içerebilir (diğer karakterler için çift tırnak kullanın)."
        ),
        c => format!("Veri tipinde '{c}' karakteri kullanılamaz."),
    }
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn parse(&mut self) -> Result<(), String> {
        let mut has_modifier = false;
        loop {
            self.name()?;
            if self.peek() == Some('(') {
                if has_modifier {
                    return Err("Veri tipinde en çok bir değiştirici listesi (parantez) olabilir.".into());
                }
                self.modifiers()?;
                has_modifier = true;
            }
            match self.peek() {
                None => return Ok(()),
                Some(' ') => {
                    self.i += 1;
                    if self.peek() == Some(' ') {
                        return Err("Veri tipinde sözcükler arasında tek boşluk olmalı.".into());
                    }
                }
                Some('[') => return self.arrays(),
                Some(c) => return Err(unexpected(c)),
            }
        }
    }

    /// `ad ( '.' ad )?`
    fn name(&mut self) -> Result<(), String> {
        self.part()?;
        if self.peek() == Some('.') {
            self.i += 1;
            if self.peek().is_none() {
                return Err("Veri tipi nokta ile bitemez; noktadan sonra ad gelmeli.".into());
            }
            self.part()?;
            if self.peek() == Some('.') {
                return Err("Veri tipi en çok şema.tip biçiminde nitelenebilir.".into());
            }
        }
        Ok(())
    }

    fn part(&mut self) -> Result<(), String> {
        match self.peek() {
            Some('"') => self.quoted(),
            Some(c) if c.is_ascii_alphabetic() || c == '_' => self.bare(),
            Some('.') => Err("Veri tipinde nokta yalnızca iki ad arasında olabilir (şema.tip); noktadan önce ve sonra ad gelmeli.".into()),
            Some(c) => Err(unexpected(c)),
            None => Err("Veri tipinde ad bekleniyordu.".into()),
        }
    }

    fn bare(&mut self) -> Result<(), String> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '$') {
            self.i += 1;
        }
        let word: String = self.c[start..self.i].iter().collect();
        if word.contains("$$") {
            return Err("Veri tipinde '$$' kullanılamaz (dolar tırnak açılışına benzer).".into());
        }
        check_len(word.len())
    }

    fn quoted(&mut self) -> Result<(), String> {
        self.i += 1; // açılış `"`
        let mut bytes = 0;
        loop {
            match self.peek() {
                None => return Err("Veri tipinde kapanmamış çift tırnak var.".into()),
                Some('"') if self.c.get(self.i + 1) == Some(&'"') => {
                    bytes += 1;
                    self.i += 2;
                }
                Some('"') => {
                    self.i += 1;
                    break;
                }
                // Denetim karakterleri en başta reddedildi; burada her karakter adın parçasıdır.
                Some(c) => {
                    bytes += c.len_utf8();
                    self.i += 1;
                }
            }
        }
        if bytes == 0 {
            return Err("Veri tipinde tırnaklı ad boş olamaz (\"\").".into());
        }
        check_len(bytes)
    }

    /// `'(' ' '* rakam+ ' '* ( ',' ' '* rakam+ ' '* )* ')'`
    fn modifiers(&mut self) -> Result<(), String> {
        const ONLY: &str = "Veri tipi değiştirici listesi yalnızca rakam, virgül ve boşluk içerebilir, ör. numeric(10,2).";
        self.i += 1; // `(`
        loop {
            self.spaces();
            let start = self.i;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == start {
                return match self.peek() {
                    None => Err("Veri tipinde kapanmamış parantez var.".into()),
                    Some(')') | Some(',') => Err("Veri tipi değiştirici listesinde boş değer olamaz.".into()),
                    Some(_) => Err(ONLY.into()),
                };
            }
            self.spaces();
            match self.peek() {
                Some(',') => self.i += 1,
                Some(')') => {
                    self.i += 1;
                    return Ok(());
                }
                None => return Err("Veri tipinde kapanmamış parantez var.".into()),
                Some(_) => return Err(ONLY.into()),
            }
        }
    }

    fn spaces(&mut self) {
        while self.peek() == Some(' ') {
            self.i += 1;
        }
    }

    /// `( '[' rakam* ']' )+` ve ardından metin sonu.
    fn arrays(&mut self) -> Result<(), String> {
        while self.peek() == Some('[') {
            self.i += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
            match self.peek() {
                Some(']') => self.i += 1,
                None => return Err("Veri tipinde kapanmamış dizi eki var ('[' kapanmamış).".into()),
                Some(_) => return Err("Veri tipinde dizi eki yalnızca [] ya da [n] olabilir.".into()),
            }
        }
        match self.peek() {
            None => Ok(()),
            Some(_) => Err("Veri tipinde dizi eki ([] ya da [n]) yalnızca sonda olabilir.".into()),
        }
    }
}

fn check_len(bytes: usize) -> Result<(), String> {
    if bytes > MAX_IDENT_BYTES {
        Err(format!(
            "Veri tipindeki her ad en çok {MAX_IDENT_BYTES} bayt olabilir (şu an {bytes} bayt)."
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_type_expressions() {
        for t in [
            "integer",
            "int4",
            "text",
            "_int4",
            "my_type$1",
            "double precision",
            "character varying",
            "character varying(50)",
            "varchar(255)",
            "bit varying(5)",
            "numeric(10,2)",
            "numeric(10, 2)",
            "numeric( 10 , 2 )",
            "timestamp(3) with time zone",
            "timestamp without time zone",
            "time(6) without time zone",
            "interval year to month",
            "interval day to second(3)",
            "interval(2)",
            "public.mytype",
            "\"My Type\"",
            "\"My Type\"[]",
            "public.\"My Type\"",
            "\"My Schema\".\"My Type\"[][]",
            "\"a\"\"b\"",
            "\"a;b\"",
            "\"tür\"",
            "\"a b  c\"",
            "\"--\"",
            "\"/*\"",
            "\"'\"",
            "\"$$\"",
            "\"\\\"",
            "text[]",
            "int4[][]",
            "integer[3]",
            "numeric(10,2)[3]",
            "numeric(10,2)[3][]",
            "character varying(50)[]",
            "Integer",
            "INTEGER",
            "a$",
        ] {
            assert_eq!(validate_type(t), Ok(()), "{t:?}");
        }
        // Tam 128 karakter: sınırda kabul.
        let t = format!("a{}", " b".repeat(63)) + "c";
        assert_eq!(t.chars().count(), 128);
        assert_eq!(validate_type(&t), Ok(()));
        // 63 baytlık ad kabul.
        assert_eq!(validate_type(&"a".repeat(63)), Ok(()));
        assert_eq!(validate_type(&format!("\"{}\"", "ş".repeat(31))), Ok(()));
    }

    #[test]
    fn rejects_hostile_and_malformed_expressions() {
        let long = format!("a{}", " b".repeat(64));
        assert_eq!(long.chars().count(), 129);
        for t in [
            // Briefteki saldırgan örnekler
            "int; drop table t",
            "int'x'",
            "text --",
            "a/*b*/",
            "numeric(1;2)",
            "numeric(a)",
            "int[",
            "\"unterminated",
            "\"a\nb\"",
            "",
            long.as_str(),
            "int\0",
            "$$",
            "numeric(precision)",
            "numeric(1, a)",
            "int[a]",
            "int[1;2]",
            // Ek: ayraçlar, yorum, tırnak, kaçış
            ";",
            "int;",
            "int ;",
            "'text'",
            "text'",
            "int --x",
            "int--",
            "--",
            "int /* x */",
            "/*",
            "int */",
            "text\\",
            "\\x",
            "int$$x$$",
            "a$$b",
            "$1",
            "$a$",
            "int -1",
            "e'x'",
            // Parantez biçimleri
            "numeric()",
            "numeric( )",
            "numeric(,)",
            "numeric(1,)",
            "numeric(,1)",
            "numeric(1,,2)",
            "numeric((1))",
            "numeric(1)(2)",
            "timestamp(3) with time zone(2)",
            "numeric(1",
            "numeric 1)",
            "numeric)",
            "numeric (10)",
            "numeric(10,-2)",
            "numeric(1.5)",
            "numeric(1e2)",
            "f(x)",
            "(1)",
            "numeric(10)x",
            "numeric(10)\"x\"",
            // Dizi biçimleri
            "int[] not null",
            "int[]x",
            "int []",
            "int[ ]",
            "int[-1]",
            "int[1",
            "int]",
            "int[[]]",
            "int[]](",
            "[]",
            "int[](1)",
            "int[] ",
            // Boşluk ve denetim karakterleri
            " int",
            "int ",
            "double  precision",
            "double\tprecision",
            "double\nprecision",
            "double\rprecision",
            "int\n",
            "\u{a0}int",
            "double\u{a0}precision",
            "\"a\u{85}b\"",
            "\"a\tb\"",
            "\"a\x7fb\"",
            "\"a\0b\"",
            "\"a\u{1b}b\"",
            "   ",
            // Adlar ve noktalar
            "tür",
            "ıd",
            "1int",
            "_ x$ 9",
            "int.",
            ".int",
            "a..b",
            "a.b.c",
            "a. b",
            "a .b",
            "\"\"",
            "\"\".x",
            "a\"b\"",
            "\"a\"b",
            "\"a\"\"",
            "\"a\"\"b",
            "a.\"\"",
            "a.(1)",
            // Diğer işaretler
            "int,text",
            "int, text",
            "int = 1",
            "int::text",
            "int%TYPE",
            "int*",
            "int@",
            "int!",
            "int{1}",
            "int#",
            "int`",
            "int+1",
        ] {
            let r = validate_type(t);
            assert!(r.is_err(), "kabul edilmemeli: {t:?}");
        }
        // Uzun adlar: 64 bayt (tırnaklı ya da tırnaksız) red.
        assert!(validate_type(&"a".repeat(64)).is_err());
        assert!(validate_type(&format!("\"{}\"", "a".repeat(64))).is_err());
        assert!(validate_type(&format!("\"{}\"", "ş".repeat(32))).is_err());
        assert!(validate_type(&format!("public.{}", "a".repeat(64))).is_err());
    }

    #[test]
    fn quoted_identifier_length_counts_unescaped_bytes() {
        // `""` tek `"` sayılır: 62 × a + `""` = 63 bayt ad → kabul.
        assert_eq!(validate_type(&format!("\"{}\"\"\"", "a".repeat(62))), Ok(()));
        assert!(validate_type(&format!("\"{}\"\"\"", "a".repeat(63))).is_err());
    }

    #[test]
    fn error_messages_are_specific_turkish() {
        let m = |s: &str| validate_type(s).unwrap_err();
        assert!(m("").contains("boş"), "{}", m(""));
        assert!(m("int\0").contains("NUL"));
        assert!(m(&"a".repeat(129)).contains("128"));
        assert!(m("double\nprecision").contains("denetim"));
        assert!(m(" int").contains("boşluk"));
        assert!(m("double  precision").contains("tek boşluk"));
        assert!(m("int; drop table t").contains("';'"));
        assert!(m("int'x'").contains("tek tırnak"));
        assert!(m("text --").contains("'-'"));
        assert!(m("a/*b*/").contains("'/'"));
        assert!(m("numeric(a)").contains("rakam"));
        assert!(m("numeric(1").contains("parantez"));
        assert!(m("timestamp(3) with time zone(2)").contains("bir"));
        assert!(m("int[").contains("dizi"));
        assert!(m("int[] not null").contains("sonda"));
        assert!(m("\"unterminated").contains("tırnak"));
        assert!(m("\"\"").contains("boş"));
        assert!(m("tür").contains("çift tırnak"));
        assert!(m("a.b.c").contains("şema.tip"));
        assert!(m("int.").contains("nokta"));
        assert!(m("a$$b").contains("$$"));
        assert!(m(&"a".repeat(64)).contains("63"));
        assert!(m("text\\").contains("'\\'"));
    }
}
