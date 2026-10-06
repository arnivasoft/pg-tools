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

/// Tek tırnaklı dize, çift tırnaklı tanımlayıcı, dolar tırnaklı dize, `--` satır yorumu ve (iç içe) `/* */` yorumu
/// **dışındaki** `;` sayısı. Kapanmamış bölge sonuna kadar sürer (içindeki `;` sayılmaz).
/// `E'…'` dizelerinde `\` sonraki karakteri kaçırır; sıradan `'…'` içinde `\` sıradan karakterdir
/// (`standard_conforming_strings = on`, PG 10+ varsayılanı); `''` her ikisinde de tırnak çiftleme.
pub fn outer_semicolon_count(sql: &str) -> usize {
    let b = sql.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut count = 0;
    while i < n {
        match b[i] {
            b';' => {
                count += 1;
                i += 1;
            }
            b'\'' => {
                // E'…' mi: önceki bayt e/E ve ondan önceki bayt tanımlayıcı parçası değil.
                let escape = i >= 1 && matches!(b[i - 1], b'e' | b'E') && (i < 2 || !is_ident_byte(b[i - 2]));
                i += 1;
                while i < n {
                    if escape && b[i] == b'\\' {
                        i += 2;
                    } else if b[i] == b'\'' {
                        if i + 1 < n && b[i + 1] == b'\'' {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'"' => {
                i += 1;
                while i < n {
                    if b[i] == b'"' {
                        if i + 1 < n && b[i + 1] == b'"' {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'$' if i == 0 || !is_ident_byte(b[i - 1]) => match dollar_tag_len(b, i) {
                Some(len) => {
                    let tag = &b[i..i + len];
                    i += len;
                    match b[i.min(n)..].windows(len).position(|w| w == tag) {
                        Some(p) => i += p + len,
                        None => i = n,
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
            }
            _ => i += 1,
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::outer_semicolon_count as c;

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
}
