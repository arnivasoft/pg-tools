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
    /// Tırnak/yorum bölgeleri dışındaki parantez derinliğinin ulaştığı en küçük değer (`)` fazlalığı → negatif).
    pub min_paren_depth: i32,
    /// Tarama sonundaki parantez derinliği (kapanmamış `(` → pozitif).
    pub final_paren_depth: i32,
    /// Metin, satır sonu görmeden biten bir `--` yorumunun içinde bitiyor (sonraki parçayı yutar).
    pub ends_in_line_comment: bool,
    /// Bölge dışında bir `'` ya da `$`, ASCII rakamın hemen ardından geliyor (`1$$…`, `1'x'`). PG sayı belirteci
    /// ile tarayıcının tanımlayıcı varsayımı burada ayrışabilir; çağıran bunu RED sayar.
    pub digit_adjacent_quote: bool,
    /// E'…' olmayan (sıradan, `B'`/`X'`/`U&'` dahil) tek tırnaklı dizenin içinde `\` var. Tarayıcı
    /// `standard_conforming_strings = on` varsayar; uygulama sırasında çalışan bir ifade ayarı kapatırsa sonraki
    /// ifadelerde `\` kaçış olur ve bölge sınırları kayar. Çağıran bunu RED sayar (kaçış gerekiyorsa `E'…'`).
    pub backslash_in_plain_string: bool,
}

impl ScanResult {
    /// Dış `;` yok ve kapanmamış bölge yok.
    pub fn is_clean(&self) -> bool {
        self.outer_semicolons == 0 && !self.unterminated
    }

    /// Parantezler dengeli ve hiçbir noktada fazladan `)` yok: ifade `( … )` sarmalından dışarı taşamaz.
    pub fn parens_balanced(&self) -> bool {
        self.min_paren_depth >= 0 && self.final_paren_depth == 0
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
    let mut depth: i32 = 0;
    let mut min_depth: i32 = 0;
    let mut ends_in_comment = false;
    let mut digit_quote = false;
    let mut plain_backslash = false;
    while i < n {
        if (b[i] == b'\'' || b[i] == b'$') && i > 0 {
            // `1'x'` ve `1e'x'` (üstel gibi görünen E dizesi öneki) de dahil.
            if b[i - 1].is_ascii_digit()
                || (b[i] == b'\'' && matches!(b[i - 1], b'e' | b'E') && i > 1 && b[i - 2].is_ascii_digit())
            {
                digit_quote = true;
            }
            // Sayı gibi başlayan belirteç (`0xAF`, `0b1`, `0o7`, `1_000`, `1.5e3`): PG 16+ bunu sayı olarak okur ve
            // ardından gelen `$`/`'` yeni bir dize açar; tarayıcı ise `AF$` gibi bir tanımlayıcı sanabilir.
            let mut j = i;
            while j > 0 && (b[j - 1].is_ascii_alphanumeric() || b[j - 1] == b'_') {
                j -= 1;
            }
            if j < i && b[j].is_ascii_digit() {
                digit_quote = true;
            }
            // Yalnız `$` sonra önekli dize (`$e'…'`, `$b'…'`, `$u&'…'`): PG `$`'ı tek karakterli belirteç sayıp
            // öneki dizenin parçası yapar, tarayıcı `$`'ı tanımlayıcı parçası sanar → RED.
            if b[i] == b'\'' {
                let pre = if b[i - 1] == b'&' && i >= 2 && matches!(b[i - 2], b'u' | b'U') { 2 } else { 1 };
                if (pre == 2 || matches!(b[i - 1], b'e' | b'E' | b'b' | b'B' | b'x' | b'X' | b'n' | b'N'))
                    && i > pre
                    && b[i - pre - 1] == b'$'
                {
                    digit_quote = true;
                }
            }
        }
        match b[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                min_depth = min_depth.min(depth);
                i += 1;
            }
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
                    } else if !escape && b[i] == b'\\' {
                        plain_backslash = true;
                        i += 1;
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
                ends_in_comment = i >= n;
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
        min_paren_depth: min_depth,
        final_paren_depth: depth,
        ends_in_line_comment: ends_in_comment,
        digit_adjacent_quote: digit_quote,
        backslash_in_plain_string: plain_backslash,
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

    fn depths(s: &str) -> (i32, i32) {
        let r = scan_semicolons(s);
        (r.min_paren_depth, r.final_paren_depth)
    }

    #[test]
    fn paren_depth_ignores_quoted_and_commented_regions() {
        assert_eq!(depths(""), (0, 0));
        assert_eq!(depths("(a)"), (0, 0));
        assert_eq!(depths("f(g(x), y)"), (0, 0));
        assert_eq!(depths("a)"), (-1, -1));
        assert_eq!(depths("1) , (2"), (-1, 0));
        assert_eq!(depths("(a"), (0, 1));
        assert_eq!(depths(")("), (-1, 0));
        assert_eq!(depths(r#"'(' || ")" || $$)$$ || /* ) */ 'x'"#), (0, 0));
        assert_eq!(depths("a -- )\n"), (0, 0));
        assert_eq!(depths(r"E'\')' )"), (-1, -1));
        assert!(scan_semicolons("(a)").parens_balanced());
        assert!(!scan_semicolons("a)").parens_balanced());
        assert!(!scan_semicolons("1) , (2").parens_balanced());
        assert!(!scan_semicolons("(a").parens_balanced());
    }

    #[test]
    fn line_comment_at_end_is_reported() {
        let e = |s: &str| scan_semicolons(s).ends_in_line_comment;
        assert!(e("a --"));
        assert!(e("a -- x"));
        assert!(e("--"));
        assert!(!e("a -- x\n"));
        assert!(!e("a -- x\r"));
        assert!(!e("a -- x\nb"));
        assert!(!e("a"));
        assert!(!e("'--'"));
        assert!(!e("/* -- */"));
    }

    #[test]
    fn digit_adjacent_quote_is_reported() {
        let d = |s: &str| scan_semicolons(s).digit_adjacent_quote;
        for s in ["1$$'$$ ) ; DROP TABLE t; --'", "1'x'", "1e'x'", "a = 12$$x$$", "x1'y'"] {
            assert!(d(s), "{s}");
        }
        for s in ["a = '1'", "a = '1''2'", "f(1, 'x')", "x $$1$$", "a = 'x' || '1'", "x = $1", "1 'x'", "-- 1'", "\"1\"", "x = 1"] {
            assert!(!d(s), "{s}");
        }
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

    fn d(s: &str) -> bool {
        scan_semicolons(s).digit_adjacent_quote
    }
    fn clean(s: &str) -> bool {
        let r = scan_semicolons(s);
        r.outer_semicolons == 0 && !r.unterminated
    }

    #[test]
    fn scan_hex_literal_then_dollar_quote_is_digit_adjacent() {
        for s in ["0xAF$$ x; y $$", "0Xff$t$ x; y $t$", "0b1$$ ; $$", "0o7$$ ; $$", "1_000$$ ; $$", "1.5e3$$ ; $$"] {
            assert!(d(s), "{s}");
        }
        // Harfle başlayan tanımlayıcı etkilenmez.
        assert!(!d("x $$a$$"));
    }

    #[test]
    fn scan_hex_literal_then_quote_boundaries() {
        for s in ["0xAF'x;y'", "0b1'x;y'", "0o7'x;y'", "1_000'x;y'"] {
            assert!(d(s), "{s}");
        }
    }

    #[test]
    fn scan_lone_dollar_then_e_string_is_escape_string() {
        for s in [r"$e'\' ; x'", r"$E'\' ; x'", "$b'x'", "$x'1F'", "$n'a'", "$u&'a'"] {
            assert!(d(s), "{s}");
        }
        // `$` olmadan e'…' önceki davranışı korur.
        assert!(!d("a = e'x'"));
    }

    #[test]
    fn scan_unicode_escape_string_backslash_does_not_escape_quote() {
        let r = scan_semicolons(r"U&'\' ; x'");
        assert_eq!(r.outer_semicolons, 1);
        assert!(r.unterminated && r.backslash_in_plain_string);
        let r = scan_semicolons("u&'a''b'");
        assert!(r.is_clean() && !r.backslash_in_plain_string);
    }

    #[test]
    fn scan_unicode_escape_identifier_doubling() {
        assert!(clean(r#"U&"a"";b""#));
        assert!(clean(r#"U&"x" UESCAPE '!'"#));
    }

    #[test]
    fn scan_bit_and_hex_strings_doubled_quote_boundaries() {
        for s in ["B'01''10'", "X'1F''2'", "N'a;b'"] {
            assert!(clean(s), "{s}");
        }
    }

    #[test]
    fn scan_unterminated_prefixed_strings() {
        for s in ["U&'abc", "U&\"abc", "B'01", "X'1F", "N'x", r"E'\", "E'\\ş"] {
            assert!(u(s), "{s}");
        }
    }

    #[test]
    fn scan_dollar_tag_case_and_prefix_overlap() {
        assert!(u("$a$ x $A$"));
        assert!(clean("$ab$ $a$ ; $ab$"));
        assert!(clean("$a$ $$a$"));
        assert!(clean("$a$ b$a$"));
        assert_eq!(c("$a$ ; $a$ ;"), 1);
    }

    #[test]
    fn scan_e_string_after_dot_and_quote() {
        assert!(clean(r"x.e'\''"));
        assert!(clean(r#""x"e'\''"#));
        // `1.e'x'`: ya işaretlenir ya da sınırlar sunucuyla aynı kalır (sıfır dış `;`, kapalı bölge).
        let r = scan_semicolons("1.e'x'");
        assert!(r.digit_adjacent_quote || (r.outer_semicolons == 0 && !r.unterminated));
    }

    #[test]
    fn scan_cr_only_line_comment_then_code() {
        assert_eq!(c("a -- c\r; b"), 1);
    }

    #[test]
    fn plain_string_backslash_is_reported_but_e_strings_are_not() {
        let b = |s: &str| scan_semicolons(s).backslash_in_plain_string;
        for s in [r"'\'", r"'a\'b' || 'c'", r"x = 'a\b'", r"U&'\0041'", r"B'\'"] {
            assert!(b(s), "{s}");
        }
        for s in [r"E'\''", r"e'a\\'", "'a'", r#""a\b""#, r"$$a\b$$", "-- 'a\\b\n", r"/* 'a\ */", "'a''b'"] {
            assert!(!b(s), "{s}");
        }
    }
}
