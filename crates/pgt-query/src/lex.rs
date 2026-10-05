//! PostgreSQL için hafif sözcük çözümleyici. Hiçbir girdide panik etmez; tüm ofsetler
//! geçerli UTF-8 sınırlarındadır (çok baytlı karakterler yalnızca Word/Str/Ident/Dollar/Comment
//! içinde bulunur, `Punct` her zaman tek ASCII bayttır).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Word,
    Ident,
    Str,
    Dollar,
    Comment,
    Punct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    /// Tokenden önceki parantez derinliği (`(` dış derinlikte, `)` azaltıldıktan sonraki derinlikte).
    pub depth: u32,
}

impl Token {
    pub fn text<'a>(&self, sql: &'a str) -> &'a str {
        &sql[self.start..self.end]
    }

    pub fn is_word(&self, sql: &str, w: &str) -> bool {
        self.kind == Kind::Word && self.text(sql).eq_ignore_ascii_case(w)
    }
}

pub fn lex(sql: &str) -> Vec<Token> {
    let b = sql.as_bytes();
    let (mut i, mut depth, mut out) = (0usize, 0u32, Vec::<Token>::new());
    let ident_start = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c >= 0x80;
    let ident_part = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let kind = if c == b'-' && b.get(i + 1) == Some(&b'-') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            Kind::Comment
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut nest = 1;
            i += 2;
            while i < b.len() && nest > 0 {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    nest += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    nest -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            Kind::Comment
        } else if c == b'\'' {
            // E'..': hemen önceki bitişik tek harfli E/e Word'ü bu dizeye katılır; ters bölü kaçıştır.
            let esc = matches!(out.last(), Some(t) if t.kind == Kind::Word
                && t.end == i && t.end - t.start == 1 && matches!(b[t.start], b'e' | b'E'));
            let start = if esc { out.pop().map_or(start, |t| t.start) } else { start };
            i += 1;
            while i < b.len() {
                if esc && b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if b[i] == b'\'' {
                    if b.get(i + 1) == Some(&b'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(Token { kind: Kind::Str, start, end: i.min(b.len()), depth });
            continue;
        } else if c == b'"' {
            i += 1;
            while i < b.len() {
                if b[i] == b'"' {
                    if b.get(i + 1) == Some(&b'"') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            Kind::Ident
        } else if let Some(tag_end) = (c == b'$').then(|| dollar_tag_end(b, i)).flatten() {
            let tag = &sql[i..=tag_end];
            i = match sql[tag_end + 1..].find(tag) {
                Some(p) => tag_end + 1 + p + tag.len(),
                None => b.len(),
            };
            Kind::Dollar
        } else if ident_start(c) || c.is_ascii_digit() {
            i += 1;
            while i < b.len() && ident_part(b[i]) {
                i += 1;
            }
            Kind::Word
        } else {
            i += 1;
            if c == b'(' {
                out.push(Token { kind: Kind::Punct, start, end: i, depth });
                depth += 1;
                continue;
            }
            if c == b')' {
                depth = depth.saturating_sub(1);
            }
            Kind::Punct
        };
        out.push(Token { kind, start, end: i.min(b.len()), depth });
    }
    out
}

/// `$tag$` açılışı: `$` + (boş | [A-Za-z_\x80+][alfanümerik _ \x80+]*) + `$`; son `$` indeksini döner.
/// `$1` (konum parametresi) açılış değildir.
fn dollar_tag_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if j < b.len() && (b[j].is_ascii_alphabetic() || b[j] == b'_' || b[j] >= 0x80) {
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] >= 0x80) {
            j += 1;
        }
    }
    (b.get(j) == Some(&b'$')).then_some(j)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Kind::*;

    fn kinds(sql: &str) -> Vec<(Kind, &str)> {
        lex(sql).iter().map(|t| (t.kind, t.text(sql))).collect()
    }

    #[test]
    fn basic_tables() {
        assert_eq!(kinds("select 'a;b'"), [(Word, "select"), (Str, "'a;b'")]);
        assert_eq!(kinds("select $$a;b$$"), [(Word, "select"), (Dollar, "$$a;b$$")]);
        assert_eq!(kinds("select $a$x$$y$a$"), [(Word, "select"), (Dollar, "$a$x$$y$a$")]);
        assert_eq!(
            kinds("select a$b$ from t"),
            [(Word, "select"), (Word, "a$b$"), (Word, "from"), (Word, "t")]
        );
        assert_eq!(
            kinds("select $1, $2"),
            [(Word, "select"), (Punct, "$"), (Word, "1"), (Punct, ","), (Punct, "$"), (Word, "2")]
        );
        assert_eq!(kinds("E'\\''"), [(Str, "E'\\''")]);
        assert_eq!(kinds("e'a\\'b'"), [(Str, "e'a\\'b'")]);
        assert_eq!(kinds("'it''s'"), [(Str, "'it''s'")]);
        assert_eq!(kinds("\"a;b\""), [(Ident, "\"a;b\"")]);
        assert_eq!(kinds("/* a /* b */ c */ x"), [(Comment, "/* a /* b */ c */"), (Word, "x")]);
        assert_eq!(kinds("-- ;\nx"), [(Comment, "-- ;"), (Word, "x")]);
    }

    #[test]
    fn e_prefix_only_when_single_adjacent_letter() {
        // `name'..'` ve `é'..'`: önek değil, Word ayrı kalır; ters bölü kaçış değildir.
        assert_eq!(kinds("name'a\\'"), [(Word, "name"), (Str, "'a\\'")]);
        assert_eq!(kinds("é'a'"), [(Word, "é"), (Str, "'a'")]);
        // Boşlukla ayrılmış E: önek değil.
        assert_eq!(kinds("E 'a\\'"), [(Word, "E"), (Str, "'a\\'")]);
        // Dizeden önceki başka belirteç yutulmaz.
        assert_eq!(kinds("x=E'a'"), [(Word, "x"), (Punct, "="), (Str, "E'a'")]);
    }

    #[test]
    fn unterminated_run_to_end() {
        for s in ["'abc", "/* abc", "$$abc", "\"abc", "E'abc\\", "E'abc\\ç", "-- x", "$t$ab", "/* a /* b */"] {
            let t = lex(s);
            let last = t.last().unwrap();
            assert_eq!(last.end, s.len(), "{s:?}");
            let _ = last.text(s);
        }
        assert_eq!(lex("select 'abc").len(), 2);
        assert_eq!(lex("/* abc").len(), 1);
        assert_eq!(lex("$$abc").len(), 1);
    }

    #[test]
    fn multibyte_offsets_are_char_boundaries() {
        let sql = "select 'çğ;ü' ; select 2 -- é\n/* ş */ \"ı\" $ü$x$ü$ ü";
        for t in lex(sql) {
            let _ = &sql[t.start..t.end];
        }
        assert_eq!(kinds("select 'çğ;ü' ; select 2")[1], (Str, "'çğ;ü'"));
        assert_eq!(kinds("E'\\ç'"), [(Str, "E'\\ç'")]);
    }

    #[test]
    fn depth_semantics() {
        let sql = "f(a,(b))";
        let d: Vec<(&str, u32)> = lex(sql).iter().map(|t| (t.text(sql), t.depth)).collect();
        assert_eq!(
            d,
            [("f", 0), ("(", 0), ("a", 1), (",", 1), ("(", 1), ("b", 2), (")", 1), (")", 0)]
        );
        // Fazla kapatma panik etmez.
        assert_eq!(lex("))x").last().unwrap().depth, 0);
    }

    #[test]
    fn is_word_is_case_insensitive() {
        let sql = "BeGiN x";
        let t = lex(sql);
        assert!(t[0].is_word(sql, "begin"));
        assert!(!t[1].is_word(sql, "begin"));
    }

    fn xorshift(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    #[test]
    fn fuzz_ranges_are_valid() {
        let alphabet: Vec<char> = "'\"$-/*;()\\eE_ \n\t1aBx=,.çğüİıé€😀".chars().collect();
        let mut seed = 0x9E3779B97F4A7C15u64;
        for _ in 0..20_000 {
            let n = (xorshift(&mut seed) % 40) as usize;
            let sql: String =
                (0..n).map(|_| alphabet[(xorshift(&mut seed) % alphabet.len() as u64) as usize]).collect();
            let mut prev_end = 0;
            for t in lex(&sql) {
                assert!(t.start < t.end && t.end <= sql.len(), "{sql:?}");
                assert!(t.start >= prev_end, "{sql:?}");
                assert!(sql.is_char_boundary(t.start) && sql.is_char_boundary(t.end), "{sql:?}");
                prev_end = t.end;
            }
        }
    }
}
