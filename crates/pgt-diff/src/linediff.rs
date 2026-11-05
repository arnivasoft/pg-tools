//! Satır farkı (doğrusal bellekli Myers) ve ayrıntı DDL metni.

use std::collections::HashMap;

use pgt_ddl::ObjectDef;
use pgt_ddl::build::{self, BuildCtx};

use crate::model::{DiffLine, LineKind};
use crate::script::comment_safe;

/// İki metnin satır toplamı bunu aşarsa satır farkı üretilmez.
pub const MAX_DIFF_LINES: usize = 20_000;

#[derive(Clone, Copy)]
enum Op {
    Same(usize, usize),
    /// Yalnız hedefte (hedef satır indeksi).
    Removed(usize),
    /// Yalnız kaynakta (kaynak satır indeksi).
    Added(usize),
}

/// Satırlar `\n` ile bölünür, satır sonundaki `\r` atılır, metin sonundaki tek `\n` boş satır üretmez.
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    body.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// target → source dönüşümü: `Removed` yalnız hedefte (`target_no`), `Added` yalnız kaynakta (`source_no`),
/// `Same` ikisinde. İki metnin satır toplamı `MAX_DIFF_LINES`'ı aşarsa `None`.
pub fn line_diff(source: &str, target: &str) -> Option<Vec<DiffLine>> {
    let src = split_lines(source);
    let tgt = split_lines(target);
    if src.len() + tgt.len() > MAX_DIFF_LINES {
        return None;
    }
    // Bayt eşitliği: satırlar tamsayı kimliğine çevrilir (karşılaştırma ucuzlar).
    let mut ids: HashMap<&str, u32> = HashMap::new();
    let a: Vec<u32> = src.iter().map(|l| intern_id(&mut ids, l)).collect();
    let b: Vec<u32> = tgt.iter().map(|l| intern_id(&mut ids, l)).collect();

    let mut ops = Vec::with_capacity(a.len().max(b.len()));
    diff_range(&a, &b, (0, a.len()), (0, b.len()), &mut ops);

    Some(
        ops.into_iter()
            .map(|op| match op {
                Op::Same(si, ti) => DiffLine {
                    kind: LineKind::Same,
                    text: src[si].to_string(),
                    source_no: Some(si as u32 + 1),
                    target_no: Some(ti as u32 + 1),
                },
                Op::Removed(ti) => DiffLine {
                    kind: LineKind::Removed,
                    text: tgt[ti].to_string(),
                    source_no: None,
                    target_no: Some(ti as u32 + 1),
                },
                Op::Added(si) => DiffLine {
                    kind: LineKind::Added,
                    text: src[si].to_string(),
                    source_no: Some(si as u32 + 1),
                    target_no: None,
                },
            })
            .collect(),
    )
}

fn intern_id<'a>(ids: &mut HashMap<&'a str, u32>, l: &'a str) -> u32 {
    let next = ids.len() as u32;
    *ids.entry(l).or_insert(next)
}

/// `a[a_range]` (kaynak) ile `b[b_range]` (hedef) farkı: ortak ön/son kırpılır, kalanı orta yılanla bölünür.
fn diff_range(
    a: &[u32],
    b: &[u32],
    a_range: (usize, usize),
    b_range: (usize, usize),
    ops: &mut Vec<Op>,
) {
    let ((mut a_lo, mut a_hi), (mut b_lo, mut b_hi)) = (a_range, b_range);
    while a_lo < a_hi && b_lo < b_hi && a[a_lo] == b[b_lo] {
        ops.push(Op::Same(a_lo, b_lo));
        a_lo += 1;
        b_lo += 1;
    }
    let mut tail = 0;
    while a_lo < a_hi && b_lo < b_hi && a[a_hi - 1] == b[b_hi - 1] {
        a_hi -= 1;
        b_hi -= 1;
        tail += 1;
    }
    if a_lo == a_hi {
        ops.extend((b_lo..b_hi).map(Op::Removed));
    } else if b_lo == b_hi {
        ops.extend((a_lo..a_hi).map(Op::Added));
    } else {
        // Kırpmadan sonra iki taraf da boş değilse D >= 2: orta yılan her iki yarıyı da küçültür.
        let (x, y, u, v) = middle_snake(&a[a_lo..a_hi], &b[b_lo..b_hi]);
        diff_range(a, b, (a_lo, a_lo + x), (b_lo, b_lo + y), ops);
        for k in 0..(u - x) {
            ops.push(Op::Same(a_lo + x + k, b_lo + y + k));
        }
        diff_range(a, b, (a_lo + u, a_hi), (b_lo + v, b_hi), ops);
    }
    for k in 0..tail {
        ops.push(Op::Same(a_hi + k, b_hi + k));
    }
}

/// Myers orta yılanı: `(x, y, u, v)` = yılanın başı ve sonu (`a`/`b` yerel indeksleri). Bellek O(N+M).
fn middle_snake(a: &[u32], b: &[u32]) -> (usize, usize, usize, usize) {
    let n = a.len() as isize;
    let m = b.len() as isize;
    let delta = n - m;
    let odd = delta & 1 != 0;
    let max = (n + m + 1) / 2;
    let off = max + 1;
    let mut vf = vec![0isize; (2 * max + 3) as usize];
    let mut vb = vec![0isize; (2 * max + 3) as usize];
    let ix = |k: isize| (k + off) as usize;

    for d in 0..=max {
        // İleri yol.
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && vf[ix(k - 1)] < vf[ix(k + 1)]) {
                vf[ix(k + 1)]
            } else {
                vf[ix(k - 1)] + 1
            };
            let mut y = x - k;
            let (x0, y0) = (x, y);
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            vf[ix(k)] = x;
            if odd
                && k >= delta - (d - 1)
                && k <= delta + (d - 1)
                && vf[ix(k)] + vb[ix(delta - k)] >= n
            {
                return (x0 as usize, y0 as usize, x as usize, y as usize);
            }
            k += 2;
        }
        // Geri yol (ters dizilerde).
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && vb[ix(k - 1)] < vb[ix(k + 1)]) {
                vb[ix(k + 1)]
            } else {
                vb[ix(k - 1)] + 1
            };
            let mut y = x - k;
            let (x0, y0) = (x, y);
            while x < n && y < m && a[(n - 1 - x) as usize] == b[(m - 1 - y) as usize] {
                x += 1;
                y += 1;
            }
            vb[ix(k)] = x;
            if !odd && k >= delta - d && k <= delta + d && vf[ix(delta - k)] + vb[ix(k)] >= n {
                return (
                    (n - x) as usize,
                    (m - y) as usize,
                    (n - x0) as usize,
                    (m - y0) as usize,
                );
            }
            k += 2;
        }
    }
    unreachable!("Myers orta yılanı D <= (N+M+1)/2 içinde bulunur")
}

/// Ayrıntı metni: `build::create(def, ctx)` ifadeleri `";\n\n"` ile birleşir ve sonda `";"` olur. Üretici
/// reddederse `-- DDL üretilemedi:` satırı ve her ileti için `-- <ileti>` satırı gelir.
pub fn ddl_text(def: &ObjectDef, ctx: &BuildCtx) -> String {
    match build::create(def, ctx) {
        Ok(plan) => {
            let mut text = plan
                .statements
                .iter()
                .map(|s| s.sql.as_str())
                .collect::<Vec<_>>()
                .join(";\n\n");
            text.push(';');
            text
        }
        Err(errs) => {
            let mut text = String::from("-- DDL üretilemedi:");
            for e in errs {
                text.push_str("\n-- ");
                text.push_str(&comment_safe(&e.message));
            }
            text
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pgt_ddl::{
        ColumnDef, ConstraintDef, ConstraintKind, EnumValue, FkAction, FkMatch, Persistence,
        TableDef, TypeBody, TypeDef,
    };

    fn text(letters: &[char]) -> String {
        letters.iter().map(|c| format!("{c}\n")).collect()
    }

    fn lcs_len(a: &[char], b: &[char]) -> usize {
        let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for i in 0..a.len() {
            for j in 0..b.len() {
                dp[i + 1][j + 1] = if a[i] == b[j] {
                    dp[i][j] + 1
                } else {
                    dp[i][j + 1].max(dp[i + 1][j])
                };
            }
        }
        dp[a.len()][b.len()]
    }

    fn edits(d: &[DiffLine]) -> usize {
        d.iter().filter(|l| l.kind != LineKind::Same).count()
    }

    #[test]
    fn bos_metinler() {
        assert_eq!(line_diff("", "").unwrap(), vec![]);
    }

    #[test]
    fn yalniz_kaynakta_satir_added() {
        let d = line_diff("a\n", "").unwrap();
        assert_eq!(
            d,
            vec![DiffLine {
                kind: LineKind::Added,
                text: "a".into(),
                source_no: Some(1),
                target_no: None
            }]
        );
    }

    #[test]
    fn yalniz_hedefte_satir_removed() {
        let d = line_diff("", "a\nb").unwrap();
        assert_eq!(d.len(), 2);
        assert!(
            d.iter()
                .all(|l| l.kind == LineKind::Removed && l.source_no.is_none())
        );
        assert_eq!(d[1].target_no, Some(2));
    }

    #[test]
    fn ayni_uc_satir() {
        let d = line_diff("x\ny\nz\n", "x\ny\nz").unwrap();
        assert_eq!(d.len(), 3);
        for (i, l) in d.iter().enumerate() {
            assert_eq!(l.kind, LineKind::Same);
            assert_eq!(l.source_no, Some(i as u32 + 1));
            assert_eq!(l.target_no, Some(i as u32 + 1));
        }
    }

    #[test]
    fn klasik_ornek_bes_duzenleme() {
        let s: Vec<char> = "CBABAC".chars().collect();
        let t: Vec<char> = "ABCABBA".chars().collect();
        let d = line_diff(&text(&s), &text(&t)).unwrap();
        assert_eq!(edits(&d), 5);
    }

    #[test]
    fn ozellik_rastgele_ciftler() {
        let mut state: u64 = 42;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        for _ in 0..300 {
            let random_word = |len: usize, next: &mut dyn FnMut() -> usize| -> Vec<char> {
                (0..len)
                    .map(|_| (b'a' + (next() % 4) as u8) as char)
                    .collect()
            };
            let (ls, lt) = (next() % 41, next() % 41);
            let s = random_word(ls, &mut next);
            let t = random_word(lt, &mut next);
            let d = line_diff(&text(&s), &text(&t)).unwrap();
            assert_eq!(
                edits(&d),
                s.len() + t.len() - 2 * lcs_len(&s, &t),
                "{s:?} / {t:?}"
            );
            let rebuilt_t: Vec<&str> = d
                .iter()
                .filter(|l| l.kind != LineKind::Added)
                .map(|l| l.text.as_str())
                .collect();
            let rebuilt_s: Vec<&str> = d
                .iter()
                .filter(|l| l.kind != LineKind::Removed)
                .map(|l| l.text.as_str())
                .collect();
            assert_eq!(
                rebuilt_t,
                t.iter().map(|c| c.to_string()).collect::<Vec<_>>()
            );
            assert_eq!(
                rebuilt_s,
                s.iter().map(|c| c.to_string()).collect::<Vec<_>>()
            );
            let s_nos: Vec<u32> = d.iter().filter_map(|l| l.source_no).collect();
            let t_nos: Vec<u32> = d.iter().filter_map(|l| l.target_no).collect();
            assert_eq!(s_nos, (1..=s.len() as u32).collect::<Vec<_>>());
            assert_eq!(t_nos, (1..=t.len() as u32).collect::<Vec<_>>());
        }
    }

    #[test]
    fn crlf_ve_lf_ayni() {
        let d = line_diff("a\r\nb\r\n", "a\nb\n").unwrap();
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|l| l.kind == LineKind::Same));
    }

    fn many(prefix: &str, n: usize) -> String {
        (0..n).map(|i| format!("{prefix}{i}\n")).collect()
    }

    #[test]
    fn sinir_asilirsa_none() {
        assert!(line_diff(&many("s", 10_001), &many("t", 10_000)).is_none());
    }

    #[test]
    fn tamamen_farkli_sinirda_hizli() {
        let t0 = std::time::Instant::now();
        let d = line_diff(&many("s", 10_000), &many("t", 10_000)).unwrap();
        assert_eq!(d.len(), 20_000);
        assert!(t0.elapsed().as_secs() < 10);
    }

    fn col(name: &str, ty: &str) -> ColumnDef {
        ColumnDef {
            attnum: None,
            name: name.into(),
            data_type: ty.into(),
            not_null: false,
            default: None,
            identity: None,
            generated: None,
            collation: None,
            comment: None,
            using: None,
        }
    }

    fn table(name: &str, columns: Vec<ColumnDef>, constraints: Vec<ConstraintDef>) -> TableDef {
        TableDef {
            schema: "public".into(),
            name: name.into(),
            owner: None,
            tablespace: None,
            persistence: Persistence::Permanent,
            columns,
            constraints,
            partition_by: None,
            fillfactor: None,
            comment: None,
            partition_text: None,
            is_partition: false,
            inherits: false,
            unsupported: vec![],
        }
    }

    fn ctx() -> BuildCtx {
        BuildCtx {
            version_num: 160000,
            concurrently: false,
        }
    }

    #[test]
    fn ddl_text_fk_li_tablo() {
        let fk = ConstraintDef {
            oid: None,
            name: Some("orders_user_fk".into()),
            kind: ConstraintKind::ForeignKey {
                columns: vec!["user_id".into()],
                ref_schema: "public".into(),
                ref_table: "users".into(),
                ref_columns: vec!["id".into()],
                match_type: FkMatch::Simple,
                on_update: FkAction::NoAction,
                on_delete: FkAction::Cascade,
            },
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
        };
        let def = ObjectDef::Table(table(
            "orders",
            vec![col("id", "integer"), col("user_id", "integer")],
            vec![fk],
        ));
        insta::assert_snapshot!(ddl_text(&def, &ctx()));
    }

    #[test]
    fn ddl_text_enum_tipi() {
        let def = ObjectDef::Type(TypeDef {
            schema: "public".into(),
            name: "mood".into(),
            owner: None,
            comment: None,
            body: TypeBody::Enum {
                values: ["sad", "ok", "happy"]
                    .iter()
                    .map(|l| EnumValue {
                        label: (*l).into(),
                        original: None,
                    })
                    .collect(),
            },
            editable: true,
            unsupported: vec![],
        });
        insta::assert_snapshot!(ddl_text(&def, &ctx()));
    }

    #[test]
    fn ddl_text_reddedilen_tablo() {
        let mut t = table("bozuk", vec![col("id", "integer")], vec![]);
        t.unsupported = vec![pgt_ddl::INVALID_UTF8_REASON.to_string()];
        let out = ddl_text(&ObjectDef::Table(t), &ctx());
        assert!(out.starts_with("-- DDL üretilemedi:"));
        assert!(
            out.lines().all(|l| l.starts_with("-- ")),
            "SQL satırı olmamalı: {out}"
        );
        insta::assert_snapshot!(out);
    }
}
