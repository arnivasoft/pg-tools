//! Çok ifadeli serbest SQL'i reddetmek için hafif tarayıcı (Task 4 kullanır): tırnaklı/yorum bölgeleri dışındaki `;` sayısı.

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// `$tag$` açılışı varsa etiketin (iki `$` dahil) bayt uzunluğunu verir. `$1` gibi parametreler açılış değildir.
fn dollar_tag_len(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if j < b.len() && b[j] == b'$' {
        return Some(2);
    }
    if j >= b.len() || !(b[j].is_ascii_alphabetic() || b[j] == b'_' || b[j] >= 0x80) {
        return None;
    }
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] >= 0x80) {
        j += 1;
    }
    (j < b.len() && b[j] == b'$').then(|| j + 1 - i)
}

/// Tarama sonucu. Çağıran `unterminated`'ı **RED** sayar: kapanmamış tırnak/yorum, sonraki parçalarla
/// birleşince ifade sınırlarını değiştirir (ör. alanda `'; DROP TABLE t; --'` tek başına kapanmamış dizedir).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanResult {
    /// Tırnak/yorum bölgeleri dışındaki `;` sayısı.
    pub outer_semicolons: usize,
    /// Kapanmamış tek tırnaklı dize, çift tırnaklı tanımlayıcı, dolar tırnak ya da blok yorum var.
    /// (Satır sonuna kadar süren `--` yorumu kapanmamış sayılmaz.)
    pub unterminated: bool,
}

impl ScanResult {
    /// Dış `;` yok ve kapanmamış bölge yok.
    pub fn is_clean(&self) -> bool {
        self.outer_semicolons == 0 && !self.unterminated
    }
}

/// Tek tırnaklı dize, çift tırnaklı tanımlayıcı, dolar tırnaklı dize, `--` satır yorumu ve (iç içe) `/* */` yorumu
/// **dışındaki** `;` sayısını ve kapanmamış bölge olup olmadığını verir.
/// `E'…'` dizelerinde `\` sonraki karakteri kaçırır; sıradan `'…'` içinde `\` sıradan karakterdir:
/// tarayıcı `standard_conforming_strings = on` varsayar (Ddl oturum profili bunu sabitler); `''` her ikisinde de çiftleme.
pub fn scan_semicolons(sql: &str) -> ScanResult {
    let b = sql.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut count = 0;
    let mut unterminated = false;
    while i < n {
        match b[i] {
            b';' => {
                count += 1;
                i += 1;
            }
            b'\'' => {
                // E'…' mi: önceki bayt e/E ve ondan önceki bayt tanımlayıcı parçası değil.
                let escape = i >= 1
                    && matches!(b[i - 1], b'e' | b'E')
                    && (i < 2 || !is_ident_byte(b[i - 2]));
                i += 1;
                let mut closed = false;
                while i < n {
                    if escape && b[i] == b'\\' {
                        i += 2;
                    } else if b[i] == b'\'' {
                        if i + 1 < n && b[i + 1] == b'\'' {
                            i += 2;
                        } else {
                            i += 1;
                            closed = true;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                unterminated |= !closed;
            }
            b'"' => {
                i += 1;
                let mut closed = false;
                while i < n {
                    if b[i] == b'"' {
                        if i + 1 < n && b[i + 1] == b'"' {
                            i += 2;
                        } else {
                            i += 1;
                            closed = true;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                unterminated |= !closed;
            }
            b'$' if i == 0 || !is_ident_byte(b[i - 1]) => match dollar_tag_len(b, i) {
                Some(len) => {
                    let tag = &b[i..i + len];
                    i += len;
                    match b[i.min(n)..].windows(len).position(|w| w == tag) {
                        Some(p) => i += p + len,
                        None => {
                            unterminated = true;
                            i = n;
                        }
                    }
                }
                None => i += 1,
            },
            b'-' if i + 1 < n && b[i + 1] == b'-' => {
                while i < n && b[i] != b'\n' && b[i] != b'\r' {
                    i += 1;
                }
            }
            b'/' if i + 1 < n && b[i + 1] == b'*' => {
                let mut depth = 1;
                i += 2;
                while i < n && depth > 0 {
                    if b[i] == b'/' && i + 1 < n && b[i + 1] == b'*' {
                        depth += 1;
                        i += 2;
                    } else if b[i] == b'*' && i + 1 < n && b[i + 1] == b'/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                unterminated |= depth > 0;
            }
            _ => i += 1,
        }
    }
    ScanResult {
        outer_semicolons: count,
        unterminated,
    }
}

#[cfg(test)]
mod tests {
    use super::scan_semicolons;

    /// Dış `;` sayısı; kapanmamış bölge varsa ayrıca `u` testleri bakar.
    fn c(s: &str) -> usize {
        scan_semicolons(s).outer_semicolons
    }
    fn u(s: &str) -> bool {
        scan_semicolons(s).unterminated
    }

    #[test]
    fn plain_semicolons() {
        assert_eq!(c(""), 0);
        assert_eq!(c("SELECT 1"), 0);
        assert_eq!(c("SELECT 1;"), 1);
        assert_eq!(c("SELECT 1; SELECT 2;"), 2);
        assert_eq!(c(";;;"), 3);
    }

    #[test]
    fn single_quoted_strings() {
        assert_eq!(c("SELECT 'a;b'"), 0);
        assert_eq!(c("SELECT 'a''; b'; "), 1); // '' çiftleme: dize sürer, sondaki ; dışarıda
        assert_eq!(c(r"SELECT 'a\'; x"), 1); // sıradan dizede \ kaçış değil: dize `a\` ile biter
        assert_eq!(c("SELECT 'unterminated; x"), 0);
    }

    #[test]
    fn e_strings_use_backslash_escapes() {
        assert_eq!(c(r"SELECT E'a\'; b'"), 0); // \' kaçış: dize sürer
        assert_eq!(c(r"SELECT e'a\\'; x"), 1); // \\ sonra ' kapatır: ; dışarıda
        assert_eq!(c(r"SELECT E'a\\\'; b'"), 0);
        assert_eq!(c("SELECT E'a''; b'"), 0);
        // Tanımlayıcı sonundaki e kaçış başlatmaz: `ae'…'` sıradan dize.
        assert_eq!(c(r"SELECT ae'x\'; y"), 1);
        assert_eq!(c(r"SELECT (E'a\'; b')"), 0);
    }

    #[test]
    fn double_quoted_identifiers() {
        assert_eq!(c(r#"SELECT "a;b""#), 0);
        assert_eq!(c(r#"SELECT "a"";b""#), 0);
        assert_eq!(c(r#"SELECT "x"; SELECT 1"#), 1);
        assert_eq!(c(r#"SELECT "x""y"; "#), 1);
        // Açık kalan tırnak sonuna kadar sürer.
        assert_eq!(c(r#"SELECT "x; y"#), 0);
        // Enjeksiyon biçimi: ad olarak tek parça.
        assert_eq!(c(r#"DROP SCHEMA "x""; DROP SCHEMA public; --""#), 0);
    }

    #[test]
    fn dollar_quoted() {
        assert_eq!(c("SELECT $$a;b$$"), 0);
        assert_eq!(c("SELECT $$a;b$$;"), 1);
        assert_eq!(c("SELECT $tag$ a; $$ b; $tag$; x"), 1);
        assert_eq!(c("SELECT $a$ x; $b$ y; $a$"), 0);
        assert_eq!(c("SELECT $$ unterminated; "), 0);
        // $1 parametre; tanımlayıcı içi $ açılış değil.
        assert_eq!(c("SELECT $1; SELECT 2"), 1);
        assert_eq!(c("SELECT a$b$; x"), 1);
        assert_eq!(c("SELECT x$$; y"), 1);
        // Etiket büyük/küçük harfe duyarlı.
        assert_eq!(c("SELECT $A$ ; $a$ ; $A$"), 0);
        // Unicode etiket.
        assert_eq!(c("SELECT $ş$ ; $ş$ ;"), 1);
    }

    #[test]
    fn line_comments() {
        assert_eq!(c("SELECT 1 -- a; b\n"), 0);
        assert_eq!(c("SELECT 1 -- a; b\n;"), 1);
        assert_eq!(c("SELECT 1 -- ;\r;"), 1);
        assert_eq!(c("SELECT 1 --;"), 0);
        assert_eq!(c("SELECT 1 - -;"), 1);
        assert_eq!(c("SELECT '--'; x"), 1);
    }

    #[test]
    fn block_comments_nested() {
        assert_eq!(c("SELECT /* ; */ 1"), 0);
        assert_eq!(c("SELECT /* a /* ; */ ; */ 1;"), 1);
        assert_eq!(c("SELECT /* unterminated ;"), 0);
        assert_eq!(c("SELECT /* a */ ; /* b */ ;"), 2);
        assert_eq!(c("SELECT 1 /* -- */ ;"), 1);
        assert_eq!(c("SELECT '/*'; /* ; */"), 1);
        assert_eq!(c("SELECT 1 / 2; "), 1);
    }

    #[test]
    fn mixed_and_unicode() {
        assert_eq!(c("SELECT 'ş;ğ', \"ü;\" ; -- ö;\n"), 1);
    }

    #[test]
    fn unterminated_regions_are_reported() {
        for s in [
            "a'",
            "'",
            "\"abc",
            "$$x",
            "$t$ x $u$",
            "/* x",
            "/* a /* b */",
            r"E'a\'",
            "x; 'y",
        ] {
            assert!(u(s), "{s}");
            assert!(!scan_semicolons(s).is_clean(), "{s}");
        }
        // Alanda tek başına duran enjeksiyon biçimi kapanmamış dizedir.
        let r = scan_semicolons("x'; DROP TABLE t; --");
        assert!(r.unterminated && r.outer_semicolons == 0);
        let r = scan_semicolons("'; DROP TABLE t; --");
        assert!(r.unterminated && !r.is_clean());
        // Kapalı bölgeler ve satır yorumu kapanmamış sayılmaz.
        for s in [
            "'a'",
            "\"a\"",
            "$$a$$",
            "$t$a$t$",
            "/* a */",
            "/* a /* b */ c */",
            "a -- x",
            r"E'a\''",
        ] {
            assert!(!u(s), "{s}");
            assert!(scan_semicolons(s).is_clean(), "{s}");
        }
        assert!(!scan_semicolons("a;").is_clean());
    }
}
