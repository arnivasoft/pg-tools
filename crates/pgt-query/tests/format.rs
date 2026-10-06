//! Biçimlendirici testleri: altın dosyalar (`tests/format/*.sql` → `*.expected.sql`, varsayılan seçenekler), seçenek
//! birleşimleri, fuzz (belirteç eşitliği + eşkuvvetlilik) ve 10.000 satır süre testi.
//! Altın dosyaları yeniden üretmek için: `PGT_BLESS=1 cargo test -p pgt-query --test format` (sonra gözle incele).

use std::path::Path;
use std::time::Instant;

use pgt_core::Error;
use pgt_query::format::{CommaStyle, FormatOptions, KeywordCase, format, is_recasable};
use pgt_query::lex::{Kind, lex};

fn opts() -> FormatOptions {
    FormatOptions::default()
}

fn f(sql: &str) -> String {
    format(sql, &opts()).unwrap_or_else(|e| panic!("biçimlendirilemedi: {e} — {sql:?}"))
}

/// Testin kendi denetimi: anlamlı belirteçler aynı (yorumlar dahil); yalnız anahtar sözcükler harfe duyarsız.
fn assert_tokens_preserved(input: &str, output: &str) {
    let (a, b) = (lex(input), lex(output));
    assert_eq!(a.len(), b.len(), "belirteç sayısı: {input:?} → {output:?}");
    for (x, y) in a.iter().zip(&b) {
        let (xs, ys) = (x.text(input), y.text(output));
        assert_eq!(x.kind, y.kind, "{input:?} → {output:?}");
        if x.kind == Kind::Word && is_recasable(xs) {
            assert!(xs.eq_ignore_ascii_case(ys), "{xs:?} ≠ {ys:?} — {input:?}");
        } else {
            assert_eq!(xs, ys, "{input:?} → {output:?}");
        }
    }
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap().replace("\r\n", "\n")
}

#[test]
fn golden_files() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/format");
    let bless = std::env::var_os("PGT_BLESS").is_some();
    let mut inputs: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            n.ends_with(".sql") && !n.ends_with(".expected.sql")
        })
        .collect();
    inputs.sort();
    assert!(inputs.len() >= 25, "en az 25 altın dosya: {}", inputs.len());
    let mut failed = Vec::new();
    for input in &inputs {
        let src = read(input);
        let got = f(&src);
        assert_tokens_preserved(&src, &got);
        assert_eq!(f(&got), got, "eşkuvvetli değil: {}", input.display());
        let exp_path = input.with_extension("expected.sql");
        if bless {
            std::fs::write(&exp_path, &got).unwrap();
            continue;
        }
        let exp = read(&exp_path);
        if got != exp {
            failed.push(format!("--- {}\n{got}\n--- beklenen\n{exp}", input.display()));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n\n"));
}

const SAMPLE: &str = "select a.id, a.name, count(*) as n, case when a.kind = 1 then 'x' else 'y' end as k from accounts a \
                      left join orders o on o.account_id = a.id and o.status <> 'void' where a.active and a.created_at > now() - interval '1 day' \
                      group by a.id, a.name order by n desc nulls last limit 10";

#[test]
fn option_lower_and_preserve_case() {
    let lower = format(SAMPLE, &FormatOptions { keyword_case: KeywordCase::Lower, ..opts() }).unwrap();
    assert!(lower.starts_with("select a.id"), "{lower}");
    assert!(lower.contains("\nleft join orders o on"), "{lower}");
    assert!(lower.contains("desc nulls last"), "{lower}");
    let mixed = "SeLeCt a FrOm t";
    let kept = format(mixed, &FormatOptions { keyword_case: KeywordCase::Preserve, ..opts() }).unwrap();
    assert_eq!(kept, "SeLeCt a\nFrOm t");
    let upper = f(SAMPLE);
    assert!(upper.contains("ORDER BY n DESC NULLS LAST"), "{upper}");
    // `interval`, `now`, tanımlayıcılar ve tür adları değişmez.
    assert!(upper.contains("now() - interval '1 day'"), "{upper}");
}

#[test]
fn option_indent_comma_and_line_width() {
    let sql = "select aaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbb, cccccccccccccccccccc, dddddddddddddddddddd, eeeeeeeeeeeeeeee from t";
    // 107 karakter: varsayılan 120'ye sığar, 80'e sığmaz.
    assert_eq!(f(sql).lines().next().unwrap().len(), 107);
    let narrow = FormatOptions { max_line: 80, ..opts() };
    assert_eq!(
        format(sql, &narrow).unwrap(),
        "SELECT\n  aaaaaaaaaaaaaaaa,\n  bbbbbbbbbbbbbbbbbbbb,\n  cccccccccccccccccccc,\n  dddddddddddddddddddd,\n  eeeeeeeeeeeeeeee\nFROM t"
    );
    let four_leading = FormatOptions { max_line: 80, indent: 4, comma: CommaStyle::Leading, ..opts() };
    assert_eq!(
        format(sql, &four_leading).unwrap(),
        "SELECT\n    aaaaaaaaaaaaaaaa\n    , bbbbbbbbbbbbbbbbbbbb\n    , cccccccccccccccccccc\n    , dddddddddddddddddddd\n    , eeeeeeeeeeeeeeee\nFROM t"
    );
    // Başta virgül, satır içinde de `a, b` biçimindedir.
    let leading = FormatOptions { comma: CommaStyle::Leading, ..opts() };
    assert_eq!(format("select a,b from t", &leading).unwrap(), "SELECT a, b\nFROM t");
    let sub = format("select * from (select a from t where x in (select y from u)) s", &FormatOptions { indent: 4, ..opts() }).unwrap();
    assert_eq!(sub, "SELECT *\nFROM (\n    SELECT a\n    FROM t\n    WHERE x IN (\n        SELECT y\n        FROM u\n    )\n) s");
}

#[test]
fn every_option_combination_is_safe_and_idempotent() {
    let corpus = [SAMPLE, "with x as (select 1) select * from x; -- son\n", "update t set a = 1, b = $$x$$ where id = $1 returning *"];
    for kc in [KeywordCase::Upper, KeywordCase::Lower, KeywordCase::Preserve] {
        for indent in [2u8, 4] {
            for comma in [CommaStyle::Trailing, CommaStyle::Leading] {
                for max_line in [80u16, 120, 200] {
                    let o = FormatOptions { keyword_case: kc, indent, comma, max_line };
                    for sql in corpus {
                        let out = format(sql, &o).unwrap_or_else(|e| panic!("{o:?} {sql:?}: {e}"));
                        assert_tokens_preserved(sql, &out);
                        assert_eq!(format(&out, &o).unwrap(), out, "{o:?}");
                        assert!(out.lines().all(|l| l == l.trim_end()), "satır sonu boşluğu: {out:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_options_are_rejected() {
    for o in [FormatOptions { indent: 0, ..opts() }, FormatOptions { max_line: 60, ..opts() }] {
        assert!(matches!(format("select 1", &o), Err(Error::Invalid(_))));
    }
}

#[test]
fn options_deserialize_camel_case_with_defaults() {
    let o: FormatOptions =
        serde_json::from_str(r#"{"keywordCase":"lower","indent":4,"comma":"leading","maxLine":100}"#).unwrap();
    assert_eq!(o, FormatOptions { keyword_case: KeywordCase::Lower, indent: 4, comma: CommaStyle::Leading, max_line: 100 });
    let d: FormatOptions = serde_json::from_str("{}").unwrap();
    assert_eq!(d, FormatOptions::default());
}

#[test]
fn protected_content_is_byte_exact() {
    let sql = "select 'çok   boşluklu  😀 dize', E'a\\'  b', $q$ iç   $$ gövde\n  korunur $q$, \"Karışık  Ad\", B'0101', \
               x::timestamp with time zone, j->>'k', j #>> '{a,b}' /* yorum   metni */ from t -- son   yorum";
    let out = f(sql);
    for piece in ["'çok   boşluklu  😀 dize'", "E'a\\'  b'", "$q$ iç   $$ gövde\n  korunur $q$", "\"Karışık  Ad\"", "B'0101'",
        "x::timestamp WITH time zone", "j->>'k'", "j #>> '{a,b}'", "/* yorum   metni */", "-- son   yorum"] {
        assert!(out.contains(piece), "{piece:?} yok: {out}");
    }
}

#[test]
fn line_comments_always_end_their_line() {
    for sql in [
        "select a -- c\n, b from t",
        "select a, -- c\nb from t",
        "select ( -- c\n1)",
        "select 1 -- c\n;",
        "select 1; -- c\nselect 2",
        "select case -- c\nwhen a then 1 end",
        "select * from t where a -- c\nand b",
        "-- only\n",
        "copy t from stdin -- c\n",
    ] {
        let out = f(sql);
        assert_tokens_preserved(sql, &out);
        for t in lex(&out) {
            if t.kind == Kind::Comment && t.text(&out).starts_with("--") {
                let rest = &out[t.end..];
                assert!(rest.is_empty() || rest.starts_with('\n'), "{sql:?} → {out:?}");
            }
        }
    }
}

#[test]
fn string_continuation_keeps_its_newline() {
    for sql in [
        "select 'a'\n'b'",
        "select x from t where s = 'a'\n'b' and y = 1",
        "values (1, 'a'\n'b')",
        "select f('a'\n'b')",
        "select case when a then 'x'\n'y' else 'z' end",
        "select E'a'\n  'b'",
        "select 'a'\r'b'",
    ] {
        let out = f(sql);
        assert_tokens_preserved(sql, &out);
        let strs: Vec<_> = lex(&out).into_iter().filter(|t| t.kind == Kind::Str).collect();
        for w in strs.windows(2) {
            let gap = &out[w[0].end..w[1].start];
            if gap.trim().is_empty() {
                assert!(gap.contains('\n'), "{sql:?} → {out:?}");
            }
        }
    }
    // Yalnız boşlukla ayrılmış dizeler (sözdizimi hatası) satıra bölünmez.
    assert_eq!(f("select 'a' 'b'"), "SELECT 'a' 'b'");
}

#[test]
fn commas_next_to_comments() {
    let trailing = f("select a -- c\n, b, c from t");
    assert!(!trailing.lines().any(|l| l.trim() == ","), "{trailing}");
    let leading = format("select a, -- c\nb, c from t", &FormatOptions { comma: CommaStyle::Leading, ..opts() }).unwrap();
    assert!(leading.contains("a, -- c"), "{leading}");
}

#[test]
fn oversized_input_is_rejected() {
    let big = "select 1;\n".repeat(600_000);
    match format(&big, &opts()) {
        Err(Error::Invalid(m)) => assert_eq!(m, pgt_query::format::FORMAT_TOO_LARGE),
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Fuzz

fn next(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

/// Plan 5 ayırıcı test korpusu (split.rs `splits_the_hard_cases`) + biçimlendirici parçaları.
const CORPUS: &[&str] = &[
    "select 1; select 2",
    "select 1;;; select 2;",
    "select 'a;b'; select 2",
    "select E'\\'; x'; select 2",
    "select 'it''s;'; select 2",
    "select \"a;b\" from t; select 2",
    "create function f() returns int as $$ begin return 1; end $$ language plpgsql; select 2",
    "do $fn$ begin perform 1; end $fn$; select 2",
    "select 1 /* ; /* ; */ ; */ ; select 2",
    "select 1 -- ; \n ; select 2",
    "create rule r as on insert to t do also (insert into a values (1); insert into b values (2)); select 3",
    "create function f(a int) returns int language sql begin atomic select case when a > 0 then 1 else 2 end; select a; end; select 9",
    "begin; select 1; commit",
    "select a$b$ from t; select $1",
    "select 'çğ;ü'; select 'x'",
    "-- yalnızca yorum\n/* ve bu */",
    "",
    "select 'açık",
    "select $$açık",
];

const FRAGS: &[&str] = &[
    "select",
    "select a, b",
    "from t",
    "from t a join u b on a.id = b.id",
    "left join v using (id)",
    "where x = 1 and y between 1 and 2 or z",
    "group by a order by b desc nulls first",
    "limit 10 offset 5",
    "(select 1)",
    "(",
    ")",
    ",",
    ";",
    "case when a then 1 else 2 end",
    "case",
    "end",
    "-- yorum",
    "-- yorum\n",
    "/* blok */",
    "/* çok\nsatırlı */",
    "$$ gövde ; $$",
    "$t$ x $t$",
    "E'kaçış \\' dize'",
    "'emoji 😀'",
    "\"Tırnaklı Ad\"",
    "x::int",
    "data->>'k'",
    "j #> '{a}'",
    "a.b.c",
    "1.5e3",
    "$1",
    "with x as (select 1)",
    "insert into t (a) values (1), (2)",
    "on conflict (a) do update set a = excluded.a",
    "update t set a = 1",
    "delete from t using u",
    "returning *",
    "union all",
    "row_number() over (partition by a order by b)",
    "count(*) filter (where x)",
    "values (1, 'a')",
    "do $$ begin null; end $$",
    "copy t from stdin",
    "\n",
    "\t",
    "-",
    "*",
    "B'01'",
    "not exists (select 1 from u where u.id = t.id)",
    "is distinct from",
    "for update skip locked",
    "'a'\n'b'",
    "'x' 'y'",
    "E'p'\n  'q'",
    "'s' -- c\n't'",
];

#[test]
fn fuzz_token_equality_and_idempotence() {
    let o = opts();
    let mut seed = 0x5DEECE66Du64;
    let (mut ok, mut err) = (0usize, 0usize);
    let mut cases: Vec<String> = CORPUS.iter().map(|s| s.to_string()).collect();
    let seps = [" ", "\n", "", "  ", "\n\n", "; ", ";\n"];
    while cases.len() < 2_000 {
        let n = 1 + (next(&mut seed) % 8) as usize;
        let mut s = String::new();
        for _ in 0..n {
            let src = if next(&mut seed).is_multiple_of(5) { CORPUS } else { FRAGS };
            s.push_str(src[(next(&mut seed) % src.len() as u64) as usize]);
            s.push_str(seps[(next(&mut seed) % seps.len() as u64) as usize]);
        }
        cases.push(s);
    }
    let mut failures = Vec::new();
    for sql in &cases {
        match format(sql, &o) {
            Ok(out) => {
                assert_tokens_preserved(sql, &out);
                assert_eq!(format(&out, &o).as_deref().ok(), Some(out.as_str()), "eşkuvvetli değil: {sql:?}");
                ok += 1;
            }
            Err(Error::Invalid(m)) if m == pgt_query::format::FORMAT_FAILED => {
                err += 1;
                failures.push(sql.clone());
            }
            Err(e) => panic!("beklenmeyen hata {e}: {sql:?}"),
        }
    }
    let ratio = err as f64 / cases.len() as f64;
    println!("fuzz: {} deneme, {ok} Ok, {err} Err (oran {ratio:.4})", cases.len());
    for s in failures.iter().take(5) {
        println!("  Err örneği: {s:?}");
    }
    assert!(ratio < 0.01, "Err oranı çok yüksek: {ratio}");
}

#[test]
fn simple_inputs_never_fail() {
    for sql in [
        "select 1",
        "SELECT 1;",
        "select * from t",
        "select a from t where b = 1 order by a",
        "insert into t values (1)",
        "update t set a = 1",
        "delete from t",
        "with x as (select 1) select * from x",
        "values (1)",
        "select 1 union select 2",
        "copy t to stdout",
        "create table t (a int)",
        "-- c",
    ] {
        assert!(format(sql, &opts()).is_ok(), "{sql:?}");
    }
}

#[test]
fn ten_thousand_lines_are_fast() {
    let block = "select a.id, a.name, b.total -- satır\n\
                 from accounts a\n\
                 join (select account_id, sum(x) as total from orders group by account_id) b on b.account_id = a.id\n\
                 where a.active and a.kind in (1, 2, 3)\n\
                 order by a.name;\n\
                 update t set a = 1, b = 'x' where id = 2;\n\
                 insert into log (msg) values ('ok 😀');\n\
                 /* blok */\n\
                 do $$ begin perform 1; end $$;\n\
                 \n";
    let sql = block.repeat(1_000);
    assert_eq!(sql.lines().count(), 10_000);
    let t = Instant::now();
    let out = format(&sql, &opts()).unwrap();
    let elapsed = t.elapsed();
    println!("10.000 satır: {elapsed:?} (debug={})", cfg!(debug_assertions));
    assert!(!out.is_empty());
    let limit = if cfg!(debug_assertions) { 2_000 } else { 200 };
    assert!(elapsed.as_millis() < limit, "{elapsed:?}");
}

#[test]
fn deep_nesting_does_not_overflow() {
    let sql = format!("select {}1{}", "(".repeat(5_000), ")".repeat(5_000));
    let out = f(&sql);
    assert_tokens_preserved(&sql, &out);
    let cases = format!("select {} 1 {}", "case when a then ".repeat(500), "end ".repeat(500));
    let out = f(&cases);
    assert_tokens_preserved(&cases, &out);
}

#[test]
fn deep_subqueries_below_the_limit_format_without_overflow() {
    let n = 90;
    let sql = format!("select * from {} t {}", "(select * from ".repeat(n), ") s".repeat(n));
    let out = f(&sql);
    assert_tokens_preserved(&sql, &out);
    assert!(out.lines().count() > n, "biçimlendirilmiş olmalı");
}

/// Karakter düzeyinde rastgele girdi: panik yok; Ok ise sözleşme sağlanır; Err oranı raporlanır.
#[test]
fn fuzz_character_soup() {
    let alphabet: Vec<&str> = vec![
        "select ", "from ", "where ", "and ", "or ", "case ", "when ", "then ", "else ", "end ", "join ", "on ",
        "union ", "with ", "as ", "(", ")", ",", ";", "'", "\"", "$$", "$", "--", "/*", "*/", "\n", " ", "e'", "\\",
        "a", "1", ".", ":", "-", ">", "😀", "ç", "between ", "values ", "set ", "insert ", "into ",
    ];
    let mut seed = 0xC0FFEE1234567u64;
    let (mut ok, mut err) = (0usize, 0usize);
    let mut samples = Vec::new();
    for _ in 0..3_000 {
        let n = (next(&mut seed) % 40) as usize;
        let sql: String = (0..n).map(|_| alphabet[(next(&mut seed) % alphabet.len() as u64) as usize]).collect();
        match format(&sql, &opts()) {
            Ok(out) => {
                assert_tokens_preserved(&sql, &out);
                assert_eq!(format(&out, &opts()).as_deref().ok(), Some(out.as_str()), "{sql:?}");
                ok += 1;
            }
            Err(_) => {
                err += 1;
                if samples.len() < 5 {
                    samples.push(sql);
                }
            }
        }
    }
    println!("karakter fuzz: {ok} Ok, {err} Err; örnekler: {samples:?}");
    assert!(err * 50 < ok + err, "Err oranı çok yüksek: {err}");
}
