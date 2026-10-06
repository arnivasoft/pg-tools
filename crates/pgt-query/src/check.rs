//! Parse tabanlı hata denetimi (tasarım §4): ifadeler oturum bağlantısında yalnızca **Parse** edilir (çalıştırılmaz;
//! korumalar `parse_guard`), sunucu hataları editör teşhisine çevrilir. Bu modül saf kısımdır: denetlenecek
//! ifadelerin seçimi (sınırlar) ve hata → teşhis eşlemesi. Ağ işi `QueryManager::check`'tedir.
//! SQL metni günlüğe yazılmaz.

use std::ops::Range;

use pgt_core::Error;
use serde::Serialize;
use ts_rs::TS;

use crate::classify::{StmtKind, analyze as classify};
use crate::parse_guard::Parsed;
use crate::split::split;

/// Bir denetimde en çok ifade sayısı ve toplam metin baytı.
pub const CHECK_MAX_STMTS: usize = 20;
pub const CHECK_MAX_BYTES: usize = 64 * 1024;

/// Betikte daha önce bir şema ifadesi (`Ddl`) varsa yalnızca **bilgi** düzeyinde gösterilen isim hataları: o ifade
/// henüz çalıştırılmadığından nesne yok sayılabilir.
const NAME_ERRORS: [&str; 5] = ["42P01", "42703", "42883", "42704", "3F000"];
/// Hiç gösterilmeyenler: parametre türü belirlenemedi (`$1`), kilit zaman aşımı, iptal.
const SUPPRESSED: [&str; 3] = ["42P18", "55P03", "57014"];

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
    /// Kapsamdaki ifade sırası (`qt_analyze`/`qt_run` ile aynı numaralama).
    pub stmt_index: u32,
    /// İfadenin betikteki Unicode scalar ofseti.
    pub char_offset: u32,
    /// İfade metninde 1 tabanlı Unicode scalar konum (betikteki yeri `char_offset + position - 1`).
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
    pub range: Range<usize>,
    /// Betikte bu ifadeden önce bir `Ddl` ifadesi var.
    pub after_ddl: bool,
}

/// Kapsamın ifadelerinden sınırlara (`CHECK_MAX_STMTS`, `CHECK_MAX_BYTES`) sığanlar, sırayla. Sınırı aşan ilk
/// ifadede durulur. `stmts`: (kapsam sırası, betik scalar ofseti, bayt aralığı).
pub(crate) fn select(sql: &str, stmts: impl IntoIterator<Item = (u32, u32, Range<usize>)>) -> Vec<CheckStmt> {
    let ddl_starts: Vec<usize> = split(sql)
        .into_iter()
        .filter(|s| classify(&sql[s.start..s.end]).kind == StmtKind::Ddl)
        .map(|s| s.start)
        .collect();
    let mut out = Vec::new();
    let mut bytes = 0usize;
    for (index, char_offset, range) in stmts {
        bytes += range.len();
        if out.len() == CHECK_MAX_STMTS || bytes > CHECK_MAX_BYTES {
            break;
        }
        let after_ddl = ddl_starts.iter().any(|&s| s < range.start);
        out.push(CheckStmt { index, char_offset, range, after_ddl });
    }
    out
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
        position,
        message,
        sqlstate: Some(code),
        severity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(code: &str) -> Parsed {
        Parsed::Failed(
            Error::Query { sqlstate: Some(code.into()), message: "m".into(), detail: None, hint: None },
            Some(3),
        )
    }

    fn stmts(sql: &str) -> Vec<(u32, u32, Range<usize>)> {
        split(sql)
            .into_iter()
            .enumerate()
            .map(|(i, s)| (i as u32, sql[..s.start].chars().count() as u32, s.start..s.end))
            .collect()
    }

    #[test]
    fn limits_and_ddl_context() {
        let sql = (0..30).map(|i| format!("select {i}")).collect::<Vec<_>>().join(";");
        assert_eq!(select(&sql, stmts(&sql)).len(), CHECK_MAX_STMTS);
        let big = format!("select '{}'; select 1", "x".repeat(CHECK_MAX_BYTES));
        assert!(select(&big, stmts(&big)).is_empty(), "sınırı aşan ilk ifadede durulur");
        let sql = "select 1; create table t(i int); select * from t; select 2";
        let got: Vec<bool> = select(sql, stmts(sql)).iter().map(|s| s.after_ddl).collect();
        assert_eq!(got, [false, false, true, true]);
        let sql = "select 1; create temp table t(i int); select * from t";
        assert!(select(sql, stmts(sql))[2].after_ddl);
    }

    #[test]
    fn severity_and_suppression() {
        let plain = CheckStmt { index: 2, char_offset: 10, range: 0..1, after_ddl: false };
        let after = CheckStmt { after_ddl: true, ..plain.clone() };
        let d = diag(&plain, q("42P01")).unwrap();
        assert_eq!((d.severity, d.position, d.stmt_index, d.char_offset), (CheckSeverity::Error, Some(3), 2, 10));
        for code in NAME_ERRORS {
            assert_eq!(diag(&after, q(code)).unwrap().severity, CheckSeverity::Info, "{code}");
        }
        assert_eq!(diag(&after, q("42601")).unwrap().severity, CheckSeverity::Error);
        for code in SUPPRESSED {
            assert!(diag(&plain, q(code)).is_none(), "{code}");
        }
        assert!(diag(&plain, Parsed::Skipped).is_none());
        assert!(diag(&plain, Parsed::Columns(vec![])).is_none());
        let local = Parsed::Failed(Error::Invalid("x".into()), None);
        assert!(diag(&plain, local).is_none());
    }

    #[test]
    fn serializes_camel_case() {
        let d = CheckDiag {
            stmt_index: 1,
            char_offset: 4,
            position: Some(2),
            message: "m".into(),
            sqlstate: Some("42601".into()),
            severity: CheckSeverity::Info,
        };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({"stmtIndex": 1, "charOffset": 4, "position": 2, "message": "m", "sqlstate": "42601", "severity": "info"})
        );
    }
}
