//! SQL metnini ifadelere ayırır (`;` yalnızca parantez dışında ve `BEGIN ATOMIC` gövdesi dışında bitirir).

use crate::lex::{Kind, lex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stmt {
    /// İlk anlamlı (yorum olmayan) belirtecin başı.
    pub start: usize,
    /// Sondaki `;` öncesindeki son belirtecin sonu; araya giren yorumlar (ve satır yorumunun
    /// satır sonu) ifadeye dahildir. Yalnız yorumdan oluşan parçalar ifade sayılmaz.
    pub end: usize,
    /// İmleç alanı: bir önceki ifadenin `;`'sinin hemen sonrası (ilk için 0)...
    pub region_start: usize,
    /// ...kendi `;`'sinin sonu (yoksa metin sonu).
    pub region_end: usize,
}

pub fn split(sql: &str) -> Vec<Stmt> {
    let toks = lex(sql);
    let mut out = Vec::new();
    let mut region_start = 0usize;
    let (mut first, mut last): (Option<usize>, usize) = (None, 0);
    let mut atomic = 0u32;
    let mut prev_sig_begin = false; // bir önceki yorum-dışı belirteç Word("BEGIN") mi
    let mut last_any = 0usize; // `;` öncesindeki son belirtecin (yorum dahil) sonu
    let line_comment_end = |t: &crate::lex::Token| {
        let end = t.end;
        if sql[t.start..end].starts_with("--") && sql.as_bytes().get(end) == Some(&b'\n') { end + 1 } else { end }
    };
    for t in &toks {
        if t.kind == Kind::Comment {
            last_any = line_comment_end(t);
            continue;
        }
        if t.kind == Kind::Punct && sql.as_bytes()[t.start] == b';' && t.depth == 0 && atomic == 0 {
            if let Some(s) = first.take() {
                out.push(Stmt { start: s, end: last_any.max(last), region_start, region_end: t.end });
                region_start = t.end;
            }
            prev_sig_begin = false;
            continue;
        }
        if first.is_none() {
            first = Some(t.start);
        }
        last = t.end;
        last_any = t.end;
        if t.kind == Kind::Word {
            if atomic == 0 {
                if prev_sig_begin && t.is_word(sql, "atomic") {
                    atomic = 1;
                }
            } else if t.is_word(sql, "case") {
                atomic += 1;
            } else if t.is_word(sql, "end") {
                atomic -= 1;
            }
        }
        prev_sig_begin = t.is_word(sql, "begin");
    }
    if let Some(s) = first {
        out.push(Stmt { start: s, end: last_any.max(last), region_start, region_end: sql.len() });
    }
    out
}

/// İmlecin (bayt ofseti) ait olduğu ifadenin indeksi. `;` sonrasındaki yalnız-boşluk aralığı
/// bir önceki ifadeye, metin sonu son ifadeye aittir. İfade yoksa `None`.
pub fn stmt_at(sql: &str, byte_offset: usize) -> Option<usize> {
    let stmts = split(sql);
    if stmts.is_empty() {
        return None;
    }
    let off = byte_offset.min(sql.len());
    let idx = stmts
        .iter()
        .position(|s| s.region_start <= off && off < s.region_end)
        .unwrap_or_else(|| stmts.iter().rposition(|s| s.region_start <= off).unwrap_or(0));
    let s = &stmts[idx];
    if idx > 0 && off < s.start && sql.as_bytes()[s.region_start..off].iter().all(u8::is_ascii_whitespace) {
        return Some(idx - 1);
    }
    Some(idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_the_hard_cases() {
        let cases: &[(&str, &[&str])] = &[
            ("select 1; select 2", &["select 1", "select 2"]),
            ("select 1;;; select 2;", &["select 1", "select 2"]),
            ("select 'a;b'; select 2", &["select 'a;b'", "select 2"]),
            ("select E'\\'; x'; select 2", &["select E'\\'; x'", "select 2"]),
            ("select 'it''s;'; select 2", &["select 'it''s;'", "select 2"]),
            ("select \"a;b\" from t; select 2", &["select \"a;b\" from t", "select 2"]),
            ("create function f() returns int as $$ begin return 1; end $$ language plpgsql; select 2", &["create function f() returns int as $$ begin return 1; end $$ language plpgsql", "select 2"]),
            ("do $fn$ begin perform 1; end $fn$; select 2", &["do $fn$ begin perform 1; end $fn$", "select 2"]),
            ("select 1 /* ; /* ; */ ; */ ; select 2", &["select 1 /* ; /* ; */ ; */", "select 2"]),
            ("select 1 -- ; \n ; select 2", &["select 1 -- ; \n", "select 2"]),
            ("create rule r as on insert to t do also (insert into a values (1); insert into b values (2)); select 3", &["create rule r as on insert to t do also (insert into a values (1); insert into b values (2))", "select 3"]),
            ("create function f(a int) returns int language sql begin atomic select case when a > 0 then 1 else 2 end; select a; end; select 9", &["create function f(a int) returns int language sql begin atomic select case when a > 0 then 1 else 2 end; select a; end", "select 9"]),
            ("begin; select 1; commit", &["begin", "select 1", "commit"]),
            ("select a$b$ from t; select $1", &["select a$b$ from t", "select $1"]),
            ("select 'çğ;ü'; select 'x'", &["select 'çğ;ü'", "select 'x'"]),
            ("-- yalnızca yorum\n/* ve bu */", &[]),
            ("", &[]),
            ("select 'açık", &["select 'açık"]),
            ("select $$açık", &["select $$açık"]),
        ];
        for (sql, want) in cases {
            let got: Vec<&str> = split(sql).iter().map(|s| &sql[s.start..s.end]).collect();
            assert_eq!(&got, want, "{sql:?}");
        }
    }

    #[test]
    fn begin_then_atomic_only_directly() {
        // `begin; atomic` ayrı ifadeler; ATOMIC tek başına gövde açmaz.
        assert_eq!(split("begin; atomic; select 1").len(), 3);
        // Yorumla ayrılmış BEGIN /*c*/ ATOMIC yine gövde açar.
        let sql = "create function f() returns int language sql begin /*c*/ atomic select 1; end; select 2";
        assert_eq!(split(sql).len(), 2);
    }

    #[test]
    fn regions_are_consistent() {
        let sql = "select 1;;  select 2; -- c\nselect 3";
        let v = split(sql);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].region_start, 0);
        assert_eq!(v[0].region_end, 9);
        assert_eq!(v[1].region_start, 9);
        assert_eq!(v[2].region_end, sql.len());
        for s in &v {
            assert!(s.region_start <= s.start && s.start < s.end && s.end <= s.region_end);
        }
    }

    #[test]
    fn stmt_at_picks_the_statement_under_the_cursor() {
        let sql = "select 1;\nselect 2;   \nselect 3";
        assert_eq!(stmt_at(sql, 3), Some(0));
        assert_eq!(stmt_at(sql, sql.find("select 2").unwrap() + 2), Some(1));
        assert_eq!(stmt_at(sql, sql.find("2;").unwrap() + 4), Some(1)); // `;` sonrası boşluk: önceki ifade
        assert_eq!(stmt_at(sql, sql.len()), Some(2));
        assert_eq!(stmt_at("", 0), None);
        assert_eq!(stmt_at("-- x", 2), None);
        assert_eq!(stmt_at(sql, sql.len() + 50), Some(2));
        // Sondaki `;` sonrası yorum/boşluk: son ifade.
        assert_eq!(stmt_at("select 1; -- x", 12), Some(0));
        // Çok baytlı ortası ofset panik etmez.
        let m = "select 'ç'; select 2";
        for off in 0..=m.len() {
            let _ = stmt_at(m, off);
        }
    }

    fn next(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    #[test]
    fn rejoining_statements_reproduces_input_modulo_separators() {
        let frags = ["select 1", "select 'a;b'", "select $$x;y$$", "select \"q;\"", "select /* ; */ 2", "select 3 -- ;\n"];
        let mut seed = 0x2545F491u64;
        for _ in 0..200 {
            let n = 1 + (next(&mut seed) % 6) as usize;
            let picked: Vec<&str> =
                (0..n).map(|_| frags[(next(&mut seed) % frags.len() as u64) as usize]).collect();
            let sql = picked.join(";");
            assert_eq!(split(&sql).len(), n, "{sql:?}");
        }
    }

    #[test]
    fn fuzz_never_panics_and_ranges_are_valid() {
        let alphabet: Vec<char> = "'\"$-/*;()\\eEbBgGiInN_ \n1aAtTcCsSoOx=,.çğüİıé€😀".chars().collect();
        let mut seed = 0xDEADBEEFCAFEF00Du64;
        for _ in 0..20_000 {
            let n = (next(&mut seed) % 60) as usize;
            let sql: String =
                (0..n).map(|_| alphabet[(next(&mut seed) % alphabet.len() as u64) as usize]).collect();
            let mut prev_end = 0;
            for s in split(&sql) {
                assert!(s.region_start <= s.start && s.start < s.end && s.end <= s.region_end, "{sql:?}");
                assert!(s.region_end <= sql.len() && s.start >= prev_end, "{sql:?}");
                for p in [s.start, s.end, s.region_start, s.region_end] {
                    assert!(sql.is_char_boundary(p), "{sql:?}");
                }
                prev_end = s.region_end;
            }
            for off in (0..=sql.len()).step_by(3) {
                let _ = stmt_at(&sql, off);
            }
        }
    }
}
