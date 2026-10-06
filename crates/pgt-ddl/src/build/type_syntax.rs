//! Kolon veri tipi (`ColumnDef.dataType`) için katı dilbilgisi. Tip metni serbest SQL DEĞİLDİR: doğrulanırsa olduğu
//! gibi yerleştirilir, bu yüzden bu işlev bir GÜVENLİK KAPISIDIR; açıkça izin verilmeyen her şey reddedilir.
//!
//! ```text
//! tip         := baş dizi*                               (dizi ekleri yalnızca sonda, bitişik)
//! baş         := yerleşik                                 (kapalı küme, büyük/küçük harf duyarsız, tırnaksız sözcükler)
//!              | ad ('.' ad)? değiştirici?               (tek ad, en çok `şema.tip`)
//! yerleşik    := 'double precision'                      (değiştirici almaz)
//!              | ('character' | 'char') (' varying')? değiştirici?
//!              | 'national' (' character' | ' char') (' varying')? değiştirici?
//!              | 'bit' (' varying')? değiştirici?
//!              | ('timestamp' | 'time') değiştirici? (' with time zone' | ' without time zone')?
//!              | 'interval' değiştirici?                 (değiştiriciden sonra alan yok)
//!              | 'interval ' alan değiştirici?
//! alan        := year | month | day | hour | minute | second | year to month | day to hour | day to minute
//!              | day to second | hour to minute | hour to second | minute to second
//! ad          := [A-Za-z_][A-Za-z0-9_$]*  |  '"' ( [^"] | '""' )+ '"'          (her biri en çok 63 bayt)
//! değiştirici := '(' ' '* rakam+ ' '* ( ',' ' '* '-'? rakam+ ' '* )? ')'       (ada bitişik; tüm tipte en çok bir)
//! dizi        := '[' rakam* ']'
//! ```
//!
//! Sözcükler arasında TEK ASCII boşluk. Yerleşik kalıplar dışındaki her ek sözcük reddedilir: tip alanından kolon
//! tanımına `NOT NULL`, `DEFAULT …`, `REFERENCES …`, `COLLATE …`, `USING …`, `GENERATED …` gibi yan tümceler sızamaz.
//!
//! Ek kısıtlar: en çok 128 karakter; tırnaksız adda `$$` ve rakamdan hemen sonra `$` yok (dolar tırnak/parametreye
//! benzemesin; nihai ifade tarayıcısı da reddederdi); NUL ve tüm denetim karakterleri (Unicode Cc, ör. `\u{85}`) her
//! yerde yasak; tırnaklı adda sıfır genişlikli/yön denetimi biçim karakterleri (`U+200B–U+200F`, `U+2028–U+202E`,
//! `U+2060–U+2064`, `U+FEFF`) yasak. Tipin varlığı sunucuda denetlenir (uygulamada hata; transaction geri alınır).
//!
//! Bilinen yanlış retler (bilinçli): sözcüklü değiştiriciler (ör. PostGIS `geometry(Point,4326)`), üçten fazla parçalı
//! ad, değiştirici ile ad arasında boşluk (`numeric (10)`), `setof`/`array` sözdizimi. Negatif ölçek
//! (`numeric(10,-2)`, PG 15+) yalnızca İKİNCİ sayıda tek `-` ile kabul edilir.

use super::MAX_IDENT_BYTES;

/// Tip metninin karakter üst sınırı.
pub const MAX_TYPE_CHARS: usize = 128;

const UNKNOWN_MULTIWORD: &str = "Bilinmeyen çok sözcüklü tip; kullanıcı tipleri tek ad olmalı (gerekirse çift tırnak). Tip alanına NOT NULL, DEFAULT, COLLATE gibi ek yan tümceler yazılamaz.";
const MODIFIER_PLACE: &str = "Veri tipinde en çok bir değiştirici listesi olabilir ve yalnızca izin verilen yerde: ör. numeric(10,2), character varying(50), timestamp(3) with time zone, interval day to second(3).";

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

/// Sıfır genişlikli ve yön denetimi biçim karakterleri (görsel aldatma). Ad doğrulayıcısı (`check_ident`) da kullanır.
pub(crate) fn is_spoofing_format(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{feff}')
}

fn is_bare_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_bare_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
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
        '(' | ')' => MODIFIER_PLACE.into(),
        '[' | ']' => "Veri tipinde köşeli parantez yalnızca sondaki dizi ekinde ([] ya da [n]) olabilir.".into(),
        '.' => "Veri tipinde nokta yalnızca iki ad arasında olabilir (şema.tip).".into(),
        ' ' => "Veri tipinde sözcükler arasında tek boşluk olmalı.".into(),
        c if c.is_ascii_digit() => "Veri tipinde ad rakamla başlayamaz.".into(),
        c if !c.is_ascii() => format!(
            "Veri tipinde U+{:04X} karakteri tırnaksız kullanılamaz; tırnaksız ad yalnızca ASCII harf, rakam, _ ve $ içerebilir (diğer karakterler için çift tırnak kullanın).",
            c as u32
        ),
        c => format!("Veri tipinde '{c}' karakteri kullanılamaz."),
    }
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    /// Konumda tırnaksız bir sözcük varsa (sınır dahil) küçük harfli hâli ve bitiş konumu.
    fn word_at(&self, at: usize) -> Option<(String, usize)> {
        let mut j = at;
        if !matches!(self.c.get(j), Some(&c) if is_bare_start(c)) {
            return None;
        }
        while matches!(self.c.get(j), Some(&c) if is_bare_char(c)) {
            j += 1;
        }
        Some((self.c[at..j].iter().collect::<String>().to_ascii_lowercase(), j))
    }

    /// `' ' w` (büyük/küçük harf duyarsız, tam sözcük) varsa tüketir.
    fn eat_word(&mut self, w: &str) -> bool {
        if self.peek() != Some(' ') {
            return false;
        }
        match self.word_at(self.i + 1) {
            Some((x, end)) if x == w => {
                self.i = end;
                true
            }
            _ => false,
        }
    }

    fn eat_any(&mut self, ws: &[&'static str]) -> Option<&'static str> {
        ws.iter().copied().find(|w| self.eat_word(w))
    }

    fn opt_modifiers(&mut self) -> Result<bool, String> {
        if self.peek() == Some('(') {
            self.modifiers()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn parse(&mut self) -> Result<(), String> {
        let first = match self.word_at(0) {
            Some((w, end)) if !matches!(self.c.get(end), Some('.') | Some('"')) => Some(w),
            _ => None,
        };
        let builtin = match first.as_deref() {
            Some("double") if self.c.get(6) == Some(&' ') => {
                self.i = 6;
                if self.c.get(7) == Some(&' ') {
                    return Err(unexpected(' '));
                }
                if !self.eat_word("precision") {
                    return Err(UNKNOWN_MULTIWORD.into());
                }
                true
            }
            Some("national") if self.c.get(8) != Some(&' ') => false,
            Some(w @ ("character" | "char" | "bit" | "national")) => {
                self.i = w.len();
                if w == "national" && self.eat_any(&["character", "char"]).is_none() {
                    return Err("national ardından character ya da char gelmeli.".into());
                }
                self.eat_word("varying");
                self.opt_modifiers()?;
                true
            }
            Some(w @ ("timestamp" | "time")) => {
                self.i = w.len();
                self.opt_modifiers()?;
                if self.eat_any(&["with", "without"]).is_some()
                    && !(self.eat_word("time") && self.eat_word("zone"))
                {
                    return Err("Saat dilimi yan tümcesi 'with time zone' ya da 'without time zone' olmalı.".into());
                }
                true
            }
            Some("interval") => {
                self.i = "interval".len();
                if !self.opt_modifiers()? {
                    self.interval_fields()?;
                    self.opt_modifiers()?;
                }
                true
            }
            _ => false,
        };
        if !builtin {
            self.name()?;
            self.opt_modifiers()?;
        }
        match self.peek() {
            None => Ok(()),
            Some('[') => self.arrays(),
            Some(' ') => match self.c.get(self.i + 1) {
                Some(&c) if is_bare_start(c) || c == '"' => Err(UNKNOWN_MULTIWORD.into()),
                Some(&c) => Err(unexpected(c)),
                None => Err(unexpected(' ')),
            },
            Some(c) => Err(unexpected(c)),
        }
    }

    /// `interval` alanları (isteğe bağlı): sabit küme.
    fn interval_fields(&mut self) -> Result<(), String> {
        let Some(f) = self.eat_any(&["year", "month", "day", "hour", "minute", "second"]) else {
            return Ok(());
        };
        if !self.eat_word("to") {
            return Ok(());
        }
        let allowed: &[&'static str] = match f {
            "year" => &["month"],
            "day" => &["hour", "minute", "second"],
            "hour" => &["minute", "second"],
            "minute" => &["second"],
            _ => &[],
        };
        if self.eat_any(allowed).is_none() {
            return Err("Geçersiz interval alanı: year to month, day to hour|minute|second, hour to minute|second ya da minute to second olmalı.".into());
        }
        Ok(())
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
            Some(c) if is_bare_start(c) => self.bare(),
            Some('.') => Err("Veri tipinde nokta yalnızca iki ad arasında olabilir (şema.tip); noktadan önce ve sonra ad gelmeli.".into()),
            Some(c) => Err(unexpected(c)),
            None => Err("Veri tipinde ad bekleniyordu.".into()),
        }
    }

    fn bare(&mut self) -> Result<(), String> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if is_bare_char(c)) {
            if self.peek() == Some('$') && self.c[self.i - 1].is_ascii_digit() {
                return Err("Veri tipinde rakamdan hemen sonra '$' gelemez (ör. x1$); adı çift tırnakla yazın.".into());
            }
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
                Some(c) if is_spoofing_format(c) => {
                    return Err(format!(
                        "Veri tipinde tırnaklı ad görünmez/yön denetimi karakteri (U+{:04X}) içeremez.",
                        c as u32
                    ));
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

    /// `'(' ' '* rakam+ ' '* ( ',' ' '* '-'? rakam+ ' '* )? ')'` — en çok iki sayı; yalnızca ikincisi (ölçek) negatif olabilir.
    fn modifiers(&mut self) -> Result<(), String> {
        const ONLY: &str = "Veri tipi değiştirici listesi yalnızca rakam, virgül ve boşluk içerebilir (en çok iki sayı; yalnızca ikincisi '-' ile negatif olabilir), ör. numeric(10,2).";
        self.i += 1; // `(`
        for k in 0..2 {
            self.spaces();
            if k == 1 && self.peek() == Some('-') {
                self.i += 1;
            }
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
                Some(',') if k == 0 => self.i += 1,
                Some(')') => {
                    self.i += 1;
                    return Ok(());
                }
                None => return Err("Veri tipinde kapanmamış parantez var.".into()),
                Some(_) => return Err(ONLY.into()),
            }
        }
        unreachable!("döngü ikinci sayıdan sonra döner")
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
            "a1b$",
            // Yerleşik çok sözcüklü kalıplar (büyük/küçük harf duyarsız)
            "Double Precision",
            "DOUBLE PRECISION[]",
            "character",
            "character(10)",
            "char(1)",
            "char varying(3)",
            "CHARACTER VARYING(50)",
            "national character varying(10)",
            "national char(2)",
            "national character",
            "bit",
            "bit(8)",
            "bit varying",
            "timestamp",
            "timestamp(3)",
            "time",
            "TIME WITH TIME ZONE",
            "timestamp with time zone[]",
            "interval",
            "interval year",
            "interval month",
            "interval day",
            "interval hour",
            "interval minute",
            "interval second",
            "interval second(3)",
            "interval day to hour",
            "interval day to minute",
            "interval day to second",
            "interval hour to minute",
            "interval hour to second",
            "interval minute to second",
            "interval minute to second(2)[]",
            // Tek ad olarak yerleşik sözcükler (kullanıcı tipi ya da şema nitelikli)
            "national",
            "double",
            "public.double",
            "char.x",
            "\"double\"",
            "timestamptz(3)",
            // Negatif ölçek yalnızca ikinci sayıda (PG 15+)
            "numeric(10,-2)",
            "numeric(10, -2)",
        ] {
            assert_eq!(validate_type(t), Ok(()), "{t:?}");
        }
        // Tam 128 karakter: sınırda kabul.
        let t = format!("{}.{}[]", "a".repeat(63), "b".repeat(62));
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
            "numeric(-10,2)",
            "numeric(-2)",
            "numeric(10,--2)",
            "numeric(10,- 2)",
            "numeric(10,-)",
            "numeric(10,-a)",
            "numeric(1,2,3)",
            "numeric(1,2,-3)",
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
            // Yan tümce sızdırma (inceleme bulgusu): tip alanı kolon tanımına ek sözcük taşıyamaz.
            "integer NOT NULL",
            "INTEGER NOT NULL",
            "Integer not null",
            "integer null",
            "integer PRIMARY KEY",
            "integer UNIQUE",
            "integer REFERENCES u",
            "integer DEFAULT pg_sleep(100)",
            "integer DEFAULT current_user",
            "integer USING pg_terminate_backend(1)",
            "integer USING c",
            "integer GENERATED ALWAYS AS IDENTITY",
            "text COLLATE \"C\"",
            "text COLLATE C",
            "integer CONSTRAINT x CHECK",
            "integer CHECK",
            "setof integer",
            "SETOF integer",
            "integer array[3]",
            "integer ARRAY",
            "x varying with time zone(1)",
            "x varying",
            "double precision(5)",
            "double precision not null",
            "double x",
            "\"double\" precision",
            "public.mytype not null",
            "\"My Type\" not null",
            "numeric(10,2) not null",
            "text[] not null",
            "character varying(50) not null",
            "character(1) varying",
            "char varying varying",
            "national varchar",
            "national character varying(10) collate x",
            "bit varying(5) default 1",
            "timestamp with time zone(3)",
            "timestamp(3) with time zone not null",
            "timestamp with time",
            "timestamp with zone",
            "timestamp without",
            "timestamp with local time zone",
            "timestamp  with time zone",
            "time zone",
            "interval(2) year",
            "interval year(2) to month",
            "interval year to day",
            "interval month to year",
            "interval second to minute",
            "interval day to",
            "interval to second",
            "interval week",
            "interval day to second(3) not null",
            "interval day(3)(4)",
            "integer\u{a0}not null",
            // Rakamdan sonra `$`
            "x1$",
            "int4$",
            "a1$b",
            "public.x1$",
            // Tırnaklı adda görünmez/yön denetimi karakterleri
            "\"a\u{200b}b\"",
            "\"a\u{200f}b\"",
            "\"a\u{2028}b\"",
            "\"a\u{202e}b\"",
            "\"a\u{2060}b\"",
            "\"a\u{2064}b\"",
            "\"\u{feff}a\"",
        ] {
            let r = validate_type(t);
            assert!(r.is_err(), "kabul edilmemeli: {t:?}");
        }
        // Görünmez karakterlerin hemen dışındaki biçim karakterleri tırnak içinde serbesttir.
        for t in ["\"a\u{200a}b\"", "\"a\u{2027}\"","\"a\u{202f}b\"", "\"a\u{2065}b\""] {
            assert_eq!(validate_type(t), Ok(()), "{t:?}");
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
        assert!(m("tür").contains("U+00FC"), "{}", m("tür"));
        assert!(!m("tür").contains('ü'));
        for t in ["integer NOT NULL", "Integer not null", "setof integer", "integer array[3]", "x varying with time zone(1)"] {
            assert!(m(t).contains("Bilinmeyen çok sözcüklü tip"), "{t}: {}", m(t));
        }
        assert!(m("double precision(5)").contains("bir değiştirici"));
        assert!(m("timestamp with time zone(3)").contains("timestamp(3) with time zone"));
        assert!(m("timestamp with time").contains("with time zone"));
        assert!(m("interval year to day").contains("interval"));
        assert!(m("x1$").contains("rakamdan hemen sonra"));
        assert!(m("int4$").contains("rakamdan hemen sonra"));
        assert!(m("\"a\u{202e}b\"").contains("U+202E"));
        assert!(m("numeric(-1)").contains("negatif"));
        assert!(m("national varchar").contains("national"));
    }
}
