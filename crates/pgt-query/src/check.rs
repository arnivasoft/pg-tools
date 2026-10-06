//! Parse tabanlı hata denetimi (tasarım §4): ifadeler oturum bağlantısında yalnızca **Parse** edilir (çalıştırılmaz;
//! korumalar `parse_guard`), sunucu hataları editör teşhisine çevrilir. Bu modül saf kısımdır: denetlenecek
//! ifadelerin seçimi (imleç + görünür alan, sınırlar) ve hata → teşhis eşlemesi. Ağ işi `QueryManager::check`'tedir.
//! SQL metni günlüğe yazılmaz.

use std::ops::Range;

use pgt_core::{Error, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::classify::{StmtKind, analyze as classify};
use crate::parse_guard::Parsed;
use crate::split::{split, stmt_at};

/// Bir denetimde en çok ifade sayısı ve toplam metin baytı.
pub const CHECK_MAX_STMTS: usize = 20;
pub const CHECK_MAX_BYTES: usize = 64 * 1024;

/// Betikte daha önce bir şema ifadesi (`Ddl`) varsa yalnızca **bilgi** düzeyinde gösterilen isim hataları: o ifade
/// henüz çalıştırılmadığından nesne yok sayılabilir.
const NAME_ERRORS: [&str; 5] = ["42P01", "42703", "42883", "42704", "3F000"];
/// Hiç gösterilmeyenler: parametre türü belirlenemedi (`$1`), kilit zaman aşımı, iptal.
const SUPPRESSED: [&str; 3] = ["42P18", "55P03", "57014"];

/// Neyin denetleneceği (tasarım §4.1): imlecin ifadesi önce, sonra görünür alanla kesişen **bütün** ifadeler (betiğin
/// tamamı bölünür; görünür alanın kestiği ifade yarım gönderilmez). Ofsetler betik içinde Unicode scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckTarget {
    pub cursor: u32,
    pub visible_start: u32,
    pub visible_end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CheckSeverity {
    Error,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckDiag {
    /// İfadenin **betiğin tamamındaki** sırası (0 tabanlı; `qt_analyze(…, all)` ile aynı numaralama).
    pub stmt_index: u32,
    /// İfadenin betikteki Unicode scalar ofseti.
    pub char_offset: u32,
    /// İfade metninin Unicode scalar uzunluğu (konumsuz hatada bütün ifade işaretlenir).
    pub char_len: u32,
    /// İfade metninde 1 tabanlı Unicode scalar konum (betikteki yeri `char_offset + position - 1`); ifade sonunu
    /// aşan konum ("end of input") son karaktere kırpılır.
    pub position: Option<u32>,
    pub message: String,
    pub sqlstate: Option<String>,
    pub severity: CheckSeverity,
}

/// Denetlenecek ifade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckStmt {
    pub index: u32,
    pub char_offset: u32,
    /// İfade metninin karakter uzunluğu.
    pub char_len: u32,
    pub range: Range<usize>,
    /// Betikte bu ifadeden önce bir `Ddl` ifadesi var.
    pub after_ddl: bool,
}

fn byte_at_char(sql: &str, ch: u32) -> usize {
    sql.char_indices().nth(ch as usize).map_or(sql.len(), |(b, _)| b)
}

/// Denetlenecek ifadeler, Parse sırasıyla: önce imlecin ifadesi, sonra görünür alanla (`[visible_start,
/// visible_end)`; boşsa o noktayı içeren) kesişen ifadeler betik sırasıyla. Sınırlar (`CHECK_MAX_STMTS`,
/// `CHECK_MAX_BYTES`) bu sırayla uygulanır; sınırı aşan ilk ifadede durulur.
pub(crate) fn select(sql: &str, target: CheckTarget) -> Result<Vec<CheckStmt>> {
    if target.visible_start > target.visible_end {
        return Err(Error::Invalid("Görünür aralık geçersiz.".into()));
    }
    let (mut chars, mut at) = (0u32, 0usize);
    let mut all = Vec::new();
    let mut seen_ddl = false;
    for (i, st) in split(sql).into_iter().enumerate() {
        chars += sql[at..st.start].chars().count() as u32;
        at = st.start;
        let text = &sql[st.start..st.end];
        all.push(CheckStmt {
            index: i as u32,
            char_offset: chars,
            char_len: text.chars().count() as u32,
            range: st.start..st.end,
            after_ddl: seen_ddl,
        });
        seen_ddl |= classify(text).kind == StmtKind::Ddl;
    }
    let cursor = stmt_at(sql, byte_at_char(sql, target.cursor));
    let (vs, ve) = (target.visible_start, target.visible_end);
    let visible = |s: &CheckStmt| {
        let end = s.char_offset + s.char_len;
        if vs == ve { s.char_offset <= vs && vs <= end } else { s.char_offset < ve && vs < end }
    };
    let order = cursor.into_iter().chain((0..all.len()).filter(|&i| Some(i) != cursor && visible(&all[i])));
    let mut out = Vec::new();
    let mut bytes = 0usize;
    for i in order {
        let s = &all[i];
        bytes += s.range.len();
        if out.len() == CHECK_MAX_STMTS || bytes > CHECK_MAX_BYTES {
            break;
        }
        out.push(s.clone());
    }
    Ok(out)
}

/// Parse sonucunu teşhise çevirir; başarı, atlanan ifade, bastırılan kod ya da sunucu dışı hata → `None`.
pub(crate) fn diag(stmt: &CheckStmt, parsed: Parsed) -> Option<CheckDiag> {
    let Parsed::Failed(Error::Query { sqlstate: Some(code), message, .. }, position) = parsed else { return None };
    if SUPPRESSED.contains(&code.as_str()) {
        return None;
    }
    let severity = if stmt.after_ddl && NAME_ERRORS.contains(&code.as_str()) {
        CheckSeverity::Info
    } else {
        CheckSeverity::Error
    };
    Some(CheckDiag {
        stmt_index: stmt.index,
        char_offset: stmt.char_offset,
        char_len: stmt.char_len,
        position: position.map(|p| p.min(stmt.char_len.max(1))),
        message,
        sqlstate: Some(code),
        severity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(code: &str, position: u32) -> Parsed {
        Parsed::Failed(
            Error::Query { sqlstate: Some(code.into()), message: "m".into(), detail: None, hint: None },
            Some(position),
        )
    }

    fn t(cursor: u32, visible_start: u32, visible_end: u32) -> CheckTarget {
        CheckTarget { cursor, visible_start, visible_end }
    }

    fn idx(sql: &str, target: CheckTarget) -> Vec<u32> {
        select(sql, target).unwrap().iter().map(|s| s.index).collect()
    }

    #[test]
    fn cursor_first_then_visible_with_limits() {
        // 100 ifade, her biri `select NNN;\n` (12 karakter).
        let sql: String = (0..100).map(|i| format!("select {i:03};\n")).collect();
        let at = |i: u32| i * 12 + 3;
        // Görünür alan ifade 40..60, imleç 50: önce 50, sonra 40.. (50 hariç) — 20 sınırı.
        let mut want = vec![50];
        want.extend((40..60).filter(|&i| i != 50));
        assert_eq!(idx(&sql, t(at(50), at(40), at(60))), want);
        // İmleç görünür alanın dışında da olsa önce gelir.
        assert_eq!(idx(&sql, t(at(90), at(10), 12 * 12)), [90, 10, 11]);
        // Boş görünür alan: yalnız o noktadaki ifade (+ imleç).
        assert_eq!(idx(&sql, t(at(3), at(7), at(7))), [3, 7]);
        // Uzun betiğin sonundaki görünür alan denetlenir (betiğin başı değil).
        assert_eq!(idx(&sql, t(at(99), at(97), at(100))), [99, 97, 98]);
        // Bayt sınırı imleç ifadesinden sonra sayılır; sınırı aşan ilk ifadede durulur.
        let big = format!("select 1;\nselect '{}';\nselect 3", "x".repeat(CHECK_MAX_BYTES - 20));
        let len = big.chars().count() as u32;
        assert_eq!(idx(&big, t(len, 0, len)), [2, 0]);
        assert!(select("select 1", t(0, 5, 2)).is_err());
        assert!(idx("", t(0, 0, 0)).is_empty());
    }

    #[test]
    fn viewport_cutting_a_statement_takes_the_whole_statement() {
        let sql = "select 1;\nselect a,\n  b,\n  c\nfrom t;\nselect 3";
        // Görünür alan ikinci ifadenin ortasından ("  b," satırı) başlıyor.
        let vs = sql.find("  b").unwrap() as u32;
        let got = select(sql, t(0, vs, sql.len() as u32)).unwrap();
        let texts: Vec<&str> = got.iter().map(|s| &sql[s.range.clone()]).collect();
        assert_eq!(texts, ["select 1", "select a,\n  b,\n  c\nfrom t", "select 3"]);
    }

    #[test]
    fn ddl_context_and_scalar_offsets() {
        let sql = "select '😀'; create table t(i int); select * from t; select 2";
        let n = sql.chars().count() as u32;
        let got = select(sql, t(0, 0, n)).unwrap();
        assert_eq!(got.iter().map(|s| s.after_ddl).collect::<Vec<_>>(), [false, false, true, true]);
        assert_eq!(got[1].char_offset, 12);
        let sql = "select 1; create temp table t(i int); select * from t";
        assert!(select(sql, t(0, 0, 60)).unwrap()[2].after_ddl);
    }

    #[test]
    fn severity_suppression_and_position_clamp() {
        let plain = CheckStmt { index: 2, char_offset: 10, char_len: 5, range: 0..1, after_ddl: false };
        let after = CheckStmt { after_ddl: true, ..plain.clone() };
        let d = diag(&plain, q("42P01", 3)).unwrap();
        assert_eq!((d.severity, d.position, d.stmt_index, d.char_offset, d.char_len), (CheckSeverity::Error, Some(3), 2, 10, 5));
        for code in NAME_ERRORS {
            assert_eq!(diag(&after, q(code, 3)).unwrap().severity, CheckSeverity::Info, "{code}");
        }
        assert_eq!(diag(&after, q("42601", 3)).unwrap().severity, CheckSeverity::Error);
        for code in SUPPRESSED {
            assert!(diag(&plain, q(code, 3)).is_none(), "{code}");
        }
        // "end of input" konumu ifade sonunu aşmaz.
        assert_eq!(diag(&plain, q("42601", 6)).unwrap().position, Some(5));
        assert!(diag(&plain, Parsed::Skipped).is_none());
        assert!(diag(&plain, Parsed::Columns(vec![])).is_none());
        assert!(diag(&plain, Parsed::Failed(Error::Invalid("x".into()), None)).is_none());
    }

    #[test]
    fn serializes_camel_case() {
        let d = CheckDiag {
            stmt_index: 1,
            char_offset: 4,
            char_len: 3,
            position: Some(2),
            message: "m".into(),
            sqlstate: Some("42601".into()),
            severity: CheckSeverity::Info,
        };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({"stmtIndex": 1, "charOffset": 4, "charLen": 3, "position": 2, "message": "m", "sqlstate": "42601", "severity": "info"})
        );
        let t: CheckTarget = serde_json::from_str(r#"{"cursor":1,"visibleStart":0,"visibleEnd":9}"#).unwrap();
        assert_eq!(t, CheckTarget { cursor: 1, visible_start: 0, visible_end: 9 });
    }
}
