//! Serbest SQL alanlarının ortak kapısı (İndeks: `whereClause`/ifade anahtarı; Tablo: varsayılan, CHECK, üretilmiş
//! ifade, bölümleme ifadesi, `USING`). Alan **olduğu gibi** ama `wrap` ile parantezli ve kapanıştan önce `\n` eklenerek
//! yerleştirilir; her alan `check_free_sql`'den, birleşmiş NİHAİ ifade de `composed_is_clean`'den geçmelidir (alan
//! sınırlarında bölge/parantez kaymasını yakalar: ör. `--` ile biten alan kapanış parantezini yutar).

use super::{err, scan_semicolons};
use crate::model::FieldError;

/// Serbest SQL alanı: NUL yok, boş değil, kapanmamış bölge yok, `--` ile bitmiyor, rakam-bitişik `'`/`$` yok,
/// dış `;` yok, parantezler dengeli. Alan başına en çok bir hata eklenir (sıra sabittir).
pub fn check_free_sql(path: &str, sql: &str, errs: &mut Vec<FieldError>) {
    if sql.contains('\0') {
        errs.push(err(path, "SQL NUL karakteri içeremez."));
        return;
    }
    if sql.trim().is_empty() {
        errs.push(err(path, "SQL boş olamaz."));
        return;
    }
    let r = scan_semicolons(sql);
    if r.unterminated {
        errs.push(err(
            path,
            "SQL'de kapanmamış tırnak, dolar tırnak ya da yorum var.",
        ));
    } else if r.ends_in_line_comment {
        errs.push(err(
            path,
            "SQL satır yorumu (--) ile bitemez; yorumdan sonra yeni satır ekleyin ya da yorumu kaldırın.",
        ));
    } else if r.digit_adjacent_quote {
        errs.push(err(
            path,
            "SQL'de rakamın (ya da sayı/$ ile başlayan belirtecin) hemen ardından ' ya da $ gelemez; araya boşluk koyun.",
        ));
    } else if r.backslash_in_plain_string {
        errs.push(err(
            path,
            "SQL'de sıradan '…' dizesi içinde \\ kullanılamaz; kaçış gerekiyorsa E'…' biçimini kullanın.",
        ));
    } else if r.outer_semicolons > 0 {
        errs.push(err(
            path,
            "SQL tek bir ifade olmalı; tırnak/yorum dışında ';' kullanılamaz.",
        ));
    } else if !r.parens_balanced() {
        errs.push(err(
            path,
            "SQL'de parantezler dengeli olmalı (fazladan ')' ya da kapanmamış '(' var).",
        ));
    }
}

/// Serbest SQL'in yerleştirme biçimi: kapanış parantezi yeni satırda, böylece alan yorumla bitse bile `)` yorumda kalmaz.
pub fn wrap(sql: &str) -> String {
    format!("({sql}\n)")
}

/// Birleşmiş nihai ifade: dış `;` yok, kapanmamış bölge yok, parantezler dengeli (ifade sonunda `;` yoktur).
pub fn composed_is_clean(sql: &str) -> bool {
    let r = scan_semicolons(sql);
    r.is_clean() && r.parens_balanced() && !r.backslash_in_plain_string
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(sql: &str) -> Vec<FieldError> {
        let mut e = vec![];
        check_free_sql("f", sql, &mut e);
        e
    }

    /// İndeks testlerindeki saldırı matrisinin işlev düzeyindeki kopyası: her biri tam olarak bir hata verir.
    #[test]
    fn attacks_are_rejected_with_one_error() {
        for a in [
            "x); DROP TABLE t; --",
            "x; DROP TABLE t",
            "x'",
            "'; DROP TABLE t; --",
            "1) , (2",
            "1)",
            "(1",
            "$$",
            "$a$ x",
            "/*",
            "\"unterminated",
            "",
            "   \n\t ",
            "a\0b",
            "x) WHERE (y",
            "a -- yorum",
            "a --",
            "1$$'$$ ) ; DROP TABLE t; --'",
            "1'x'",
            "1e'x'",
            "x > 1$$a$$",
            "a /* b",
            "a; ",
        ] {
            let e = check(a);
            assert_eq!(e.len(), 1, "{a:?}: {e:?}");
            assert_eq!(e[0].path, "f");
        }
    }

    #[test]
    fn messages_follow_fixed_priority() {
        let m = |s: &str| check(s)[0].message.clone();
        assert!(m("a\0'").contains("NUL"));
        assert!(m(" \t\n").contains("boş"));
        assert!(m("x'; --").contains("kapanmamış"));
        assert!(m("a; b --").contains("satır yorumu"));
        assert!(m("1'x'; b").contains("rakamın"));
        assert!(m("a; b)").contains("tek bir ifade"));
        assert!(m("a)").contains("parantez"));
        assert!(m("(a").contains("parantez"));
    }

    #[test]
    fn legit_uses_pass() {
        for a in [
            "x > 1",
            "a = ';'",
            "a = 'it''s;'",
            "a = E'\\'; x'",
            "a = $$;)$$",
            "f(g(x), (y))",
            "a /* ; ) */ > 1",
            "a = \"we;ird\"",
            "a -- not closed by paren but newline\n> 1",
            "s = ')' AND (b OR c)",
            "a = 1 AND b = 'x1'",
            "c > 0 -- k\n",
            "now()",
            "0",
        ] {
            assert!(check(a).is_empty(), "{a:?}: {:?}", check(a));
        }
    }

    #[test]
    fn wrap_puts_closing_paren_on_new_line() {
        assert_eq!(wrap("a"), "(a\n)");
        assert_eq!(wrap("a -- x"), "(a -- x\n)");
        // Kabul edilen her alan sarıldığında da temiz ve dengeli kalır.
        for a in ["a -- yorum\nb", "c > 0 -- k\n", "f(x)", "'x'"] {
            assert!(check(a).is_empty());
            assert!(composed_is_clean(&wrap(a)), "{a:?}");
        }
    }

    #[test]
    fn composed_statement_check() {
        assert!(composed_is_clean("CREATE INDEX i ON t ((a\n))"));
        assert!(!composed_is_clean("CREATE INDEX i ON t ((a --\n)) x;"));
        assert!(!composed_is_clean("CREATE INDEX i ON t ((a -- )"));
        assert!(!composed_is_clean("SELECT 'x"));
        assert!(!composed_is_clean("SELECT 1)"));
    }

    #[test]
    fn freesql_rejects_backslash_in_plain_string() {
        for a in [r"'\'", r"'a\'b' || 'c'", r"x = 'a\b'", r"U&'\0041'"] {
            let e = check(a);
            assert_eq!(e.len(), 1, "{a:?}: {e:?}");
        }
        // E'…' ve diğer bölgelerde `\` serbest.
        for a in [r"x = E'a\\b'", r"x = E'\''", r#"x = "a\b""#, r"x = $$a\b$$"] {
            assert!(check(a).is_empty(), "{a:?}: {:?}", check(a));
        }
        assert!(!composed_is_clean(r"SELECT 'a\b'"));
        assert!(composed_is_clean(r"SELECT E'a\\b'"));
    }

    #[test]
    fn freesql_rejects_hex_literal_dollar_quote_and_lone_dollar_prefix() {
        for a in ["0xAF$$ x $$", r"$e'\' ; x'"] {
            let e = check(a);
            assert_eq!(e.len(), 1, "{a:?}: {e:?}");
        }
    }

    /// Kabul edilen her alan tek başına ve `(<f>\n)` sarmalıyla (farklı bağlam öneklerinde) aynı temizlik sonucunu verir.
    #[test]
    fn freesql_wrap_context_parity() {
        let fields = [
            "x > 1",
            "a = ';'",
            "a = E'\\'; x'",
            "a = $$;)$$",
            "f(g(x), (y))",
            "a /* ; ) */ > 1",
            "a -- c\n> 1",
            "c > 0 -- k\n",
            "x'",
            "1'x'",
            "a; b",
            "(a",
            "a)",
            "$a$ x",
            "a --",
            "'\\'",
        ];
        for f in fields {
            let alone_ok = check(f).is_empty();
            for ctx in ["DEFAULT ", "CHECK ", "USING ", "AS ", "ADD COLUMN x int GENERATED ALWAYS AS "] {
                let wrapped = format!("{ctx}{}", wrap(f));
                // Kapı kabul ettiyse nihai ifade temiz olmalı.
                if alone_ok {
                    assert!(composed_is_clean(&wrapped), "{ctx}{f:?}");
                }
            }
        }
    }
}
