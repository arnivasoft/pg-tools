//! Görsel EXPLAIN'in saf kısmı: ifadenin EXPLAIN'e uygunluğu, seçeneklerin sürüm/ANALYZE kapıları, çalıştırılacak
//! `EXPLAIN (FORMAT JSON, …)` metni ve otomatik geri alma koşulu. Ağ işi `QueryManager::explain`'dedir.
//!
//! Güvenlik: sarmalanmış metin her zaman yeniden sınıflandırılır (`classify`: `EXPLAIN ANALYZE <yazan ifade>` →
//! `Write` + tehlike); kilit ve onay kapıları o sınıflandırmayla çalışır. ANALYZE'ın tek doğruluk kaynağı
//! seçeneklerdir: metin yalnızca seçeneklerden üretilir. SQL metni günlüğe yazılmaz.

use pgt_core::Error;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::classify::StmtKind;
use crate::error_codes::QueryErrorCode;
use crate::events::NoticeInfo;
use crate::lex::{Kind, Token, lex};
use crate::session::TxState;
use crate::split::split;

/// Planın (tek hücrelik JSON metni) en büyük boyutu.
pub const EXPLAIN_MAX_BYTES: usize = 32 * 1024 * 1024;

/// Geri alma yapıldığında sonuca eklenen not.
pub const ROLLED_BACK_NOTE: &str = "Değişiklikler geri alındı (sıra değerleri gibi transaction dışı etkiler kalabilir).";
/// ANALYZE ile yazan ifade açık bir transaction'da çalıştığında sonuca eklenen not.
pub const IN_TX_NOTE: &str = "Etkiler açık transaction'ınızda kalır.";

/// EXPLAIN seçenekleri. `timing` yalnız ANALYZE ile anlamlıdır (kapatılırsa `TIMING OFF`); `summary` PostgreSQL
/// varsayılanından farklıysa yazılır (ANALYZE ile açık, ANALYZE'sız kapalı). `rollback`: ANALYZE ile yazan ifade,
/// oturum transaction dışındayken `BEGIN … ROLLBACK` içinde çalıştırılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExplainOptions {
    pub analyze: bool,
    pub buffers: bool,
    pub verbose: bool,
    pub settings: bool,
    pub wal: bool,
    pub timing: bool,
    pub costs: bool,
    pub summary: bool,
    pub rollback: bool,
}

impl Default for ExplainOptions {
    fn default() -> Self {
        ExplainOptions {
            analyze: false,
            buffers: false,
            verbose: false,
            settings: false,
            wal: false,
            timing: true,
            costs: true,
            summary: true,
            rollback: true,
        }
    }
}

/// `qt_explain` sonucu. `json`: sunucunun döndürdüğü plan metni (tek hücre, ayrıştırılmadan); `sql`: çalıştırılan
/// EXPLAIN metni; `rolled_back`: ifade `BEGIN … ROLLBACK` içinde çalıştı; `note`: kullanıcıya gösterilecek not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExplainResult {
    pub json: String,
    pub notices: Vec<NoticeInfo>,
    pub elapsed_ms: u64,
    pub rolled_back: bool,
    pub sql: String,
    pub note: Option<String>,
}

/// EXPLAIN doğrulama hataları. Her biri sabit bir Türkçe iletiyle `Error::Invalid` olarak taşınır;
/// `error_codes::query_payload` iletiyi `explain_*` koduna çevirir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplainError {
    /// Kapsamda ifade yok ya da birden çok ifade var.
    Scope,
    /// İfade zaten EXPLAIN.
    AlreadyExplain,
    /// EXPLAIN'e uygun olmayan ifade (DDL, yardımcı komut, BEGIN/SET…).
    Unsupported,
    /// MERGE planı PostgreSQL 15 öncesinde alınamaz.
    MergeVersion,
    SettingsVersion,
    WalVersion,
    WalNeedsAnalyze,
    /// ANALYZE'sız BUFFERS PostgreSQL 13 öncesinde kullanılamaz.
    BuffersNeedsAnalyze,
    /// Plan metni `EXPLAIN_MAX_BYTES`'ı aştı.
    TooLarge,
}

const ALL_ERRORS: [ExplainError; 9] = [
    ExplainError::Scope,
    ExplainError::AlreadyExplain,
    ExplainError::Unsupported,
    ExplainError::MergeVersion,
    ExplainError::SettingsVersion,
    ExplainError::WalVersion,
    ExplainError::WalNeedsAnalyze,
    ExplainError::BuffersNeedsAnalyze,
    ExplainError::TooLarge,
];

impl ExplainError {
    pub fn message(self) -> &'static str {
        match self {
            ExplainError::Scope => "Plan için tek bir ifade seçin.",
            ExplainError::AlreadyExplain => "İfade zaten EXPLAIN; çalıştırıp 'Planı göster'i kullanın.",
            ExplainError::Unsupported => {
                "Bu ifadenin planı alınamaz. Plan yalnız SELECT, WITH, VALUES, TABLE, INSERT, UPDATE, DELETE, MERGE, \
                 EXECUTE, DECLARE, CREATE TABLE … AS, CREATE MATERIALIZED VIEW … AS ve SELECT … INTO için alınabilir."
            }
            ExplainError::MergeVersion => "MERGE planı PostgreSQL 15 ve üstünde alınabilir.",
            ExplainError::SettingsVersion => "SETTINGS seçeneği PostgreSQL 12 ve üstünde kullanılabilir.",
            ExplainError::WalVersion => "WAL seçeneği PostgreSQL 13 ve üstünde kullanılabilir.",
            ExplainError::WalNeedsAnalyze => "WAL seçeneği yalnız ANALYZE ile kullanılabilir.",
            ExplainError::BuffersNeedsAnalyze => "ANALYZE'sız BUFFERS PostgreSQL 13 ve üstünde kullanılabilir.",
            ExplainError::TooLarge => "Plan çok büyük (32 MiB sınırı aşıldı).",
        }
    }

    pub fn code(self) -> QueryErrorCode {
        match self {
            ExplainError::Scope => QueryErrorCode::ExplainScope,
            ExplainError::AlreadyExplain => QueryErrorCode::ExplainAlready,
            ExplainError::Unsupported | ExplainError::MergeVersion => QueryErrorCode::ExplainUnsupported,
            ExplainError::SettingsVersion
            | ExplainError::WalVersion
            | ExplainError::WalNeedsAnalyze
            | ExplainError::BuffersNeedsAnalyze => QueryErrorCode::ExplainOption,
            ExplainError::TooLarge => QueryErrorCode::ExplainTooLarge,
        }
    }

    /// `Error::Invalid` iletisinden hatayı bulur (komut katmanı eşlemesi).
    pub fn from_message(m: &str) -> Option<ExplainError> {
        ALL_ERRORS.into_iter().find(|e| e.message() == m)
    }
}

impl From<ExplainError> for Error {
    fn from(e: ExplainError) -> Self {
        Error::Invalid(e.message().into())
    }
}

/// Yorumsuz belirteçler.
fn tokens(sql: &str) -> Vec<Token> {
    lex(sql).into_iter().filter(|t| t.kind != Kind::Comment).collect()
}

/// İfade EXPLAIN'e uygun mu (ilk anahtar sözcüğe ve `CREATE` biçimine göre; baştaki `(` ve yorumlar atlanır).
pub fn explainable(stmt: &str, version_num: u32) -> Result<(), ExplainError> {
    let toks = tokens(stmt);
    let Some(first) = toks.iter().position(|t| !(t.kind == Kind::Punct && t.text(stmt) == "(")) else {
        return Err(ExplainError::Scope);
    };
    let word = |i: usize| toks.get(i).filter(|t| t.kind == Kind::Word).map(|t| t.text(stmt).to_ascii_lowercase());
    let Some(lead) = word(first) else { return Err(ExplainError::Unsupported) };
    match lead.as_str() {
        "explain" => Err(ExplainError::AlreadyExplain),
        "select" | "with" | "values" | "table" | "insert" | "update" | "delete" | "execute" | "declare" => Ok(()),
        "merge" if version_num >= 150_000 => Ok(()),
        "merge" => Err(ExplainError::MergeVersion),
        "create" => {
            const MODIFIERS: [&str; 5] = ["temp", "temporary", "unlogged", "global", "local"];
            let mut i = first + 1;
            while word(i).is_some_and(|w| MODIFIERS.contains(&w.as_str())) {
                i += 1;
            }
            let base = toks[first].depth;
            let as_at_base = toks.iter().any(|t| t.depth == base && t.is_word(stmt, "as"));
            match word(i).as_deref() {
                Some("table") if as_at_base => Ok(()),
                Some("materialized") if word(i + 1).as_deref() == Some("view") && as_at_base => Ok(()),
                _ => Err(ExplainError::Unsupported),
            }
        }
        _ => Err(ExplainError::Unsupported),
    }
}

/// `sql[floor..start]` içinde `start`'tan hemen önce gelen (aralarında başka belirteç olmayan) yorumların başı;
/// yorum yoksa `start`. İfadenin baştaki yorumu (ör. pg_hint_plan ipucu `/*+ … */`) ifadeye aittir.
pub(crate) fn leading_comments_start(sql: &str, floor: usize, start: usize) -> usize {
    lex(&sql[floor..start])
        .iter()
        .rev()
        .take_while(|t| t.kind == Kind::Comment)
        .last()
        .map_or(start, |t| floor + t.start)
}

/// İfadenin sarmalanacak çekirdeği: tam bir ifade olmalı; baştaki yorumlar (ipuçları) aynen kalır, baştaki
/// boşluklar ile sondaki yorumlar, boşluklar ve `;` atılır (aradakiler olduğu gibi kalır).
fn core(stmt: &str) -> Result<&str, ExplainError> {
    let parts = split(stmt);
    let [one] = parts.as_slice() else { return Err(ExplainError::Scope) };
    let toks = tokens(&stmt[one.start..one.end]);
    let last = toks.last().ok_or(ExplainError::Scope)?;
    Ok(&stmt[leading_comments_start(stmt, 0, one.start)..one.start + last.end])
}

/// `EXPLAIN (FORMAT JSON, …) <ifade>` metni. Sürüm altı ya da ANALYZE'sız kullanılamayan seçenek doğrulama
/// hatasıdır (sessizce atlanmaz). PostgreSQL 18'de ANALYZE ile BUFFERS varsayılan açık olduğundan kapalıysa
/// `BUFFERS OFF` yazılır.
pub fn explain_sql(stmt: &str, opts: &ExplainOptions, version_num: u32) -> Result<String, ExplainError> {
    let body = core(stmt)?;
    explainable(body, version_num)?;
    let mut parts = vec!["FORMAT JSON"];
    if opts.analyze {
        parts.push("ANALYZE");
    }
    if opts.verbose {
        parts.push("VERBOSE");
    }
    if !opts.costs {
        parts.push("COSTS OFF");
    }
    if opts.settings {
        if version_num < 120_000 {
            return Err(ExplainError::SettingsVersion);
        }
        parts.push("SETTINGS");
    }
    if opts.buffers {
        if !opts.analyze && version_num < 130_000 {
            return Err(ExplainError::BuffersNeedsAnalyze);
        }
        parts.push("BUFFERS");
    } else if opts.analyze && version_num >= 180_000 {
        parts.push("BUFFERS OFF");
    }
    if opts.wal {
        if version_num < 130_000 {
            return Err(ExplainError::WalVersion);
        }
        if !opts.analyze {
            return Err(ExplainError::WalNeedsAnalyze);
        }
        parts.push("WAL");
    }
    if opts.analyze && !opts.timing {
        parts.push("TIMING OFF");
    }
    match (opts.analyze, opts.summary) {
        (true, false) => parts.push("SUMMARY OFF"),
        (false, true) => parts.push("SUMMARY"),
        _ => {}
    }
    Ok(format!("EXPLAIN ({}) {body}", parts.join(", ")))
}

/// Otomatik geri alma koşulu: ANALYZE ile yazan (sarmalanmış metnin sınıfı `Write`) ifade, seçenek açık ve oturum
/// transaction dışında. Açık transaction'a dokunulmaz; `Failed` transaction'da plan zaten alınamaz.
pub fn auto_rollback(opts: &ExplainOptions, kind: StmtKind, state: TxState) -> bool {
    opts.analyze && kind == StmtKind::Write && opts.rollback && state == TxState::Idle
}

/// Sonucun notu: geri alındıysa geri alma notu; geri alınmadan yazan bir ANALYZE açık transaction'da kaldıysa uyarı.
pub fn note(opts: &ExplainOptions, kind: StmtKind, rolled_back: bool, state_after: TxState) -> Option<String> {
    if rolled_back {
        Some(ROLLED_BACK_NOTE.into())
    } else if opts.analyze && kind == StmtKind::Write && state_after != TxState::Idle {
        Some(IN_TX_NOTE.into())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V10: u32 = 100_023;
    const V12: u32 = 120_000;
    const V13: u32 = 130_000;
    const V14: u32 = 140_010;
    const V15: u32 = 150_000;
    const V17: u32 = 170_002;
    const V18: u32 = 180_000;

    #[test]
    fn explainable_table() {
        use ExplainError::*;
        let cases: [(&str, u32, Result<(), ExplainError>); 31] = [
            ("select 1", V10, Ok(())),
            ("  /* c */ (select 1) union (select 2)", V10, Ok(())),
            ("with a as (select 1) select * from a", V10, Ok(())),
            ("with d as (delete from t returning *) select * from d", V10, Ok(())),
            ("values (1), (2)", V10, Ok(())),
            ("table t", V10, Ok(())),
            ("insert into t values (1)", V10, Ok(())),
            ("update t set a = 1 where id = 1", V10, Ok(())),
            ("delete from t", V10, Ok(())),
            ("merge into t using s on true when matched then delete", V14, Err(MergeVersion)),
            ("merge into t using s on true when matched then delete", V15, Ok(())),
            ("execute q(1)", V10, Ok(())),
            ("declare c cursor for select 1", V10, Ok(())),
            ("create table t2 as select 1", V10, Ok(())),
            ("create temp table t2 (a) as select 1", V10, Ok(())),
            ("create unlogged table if not exists t2 as table t", V10, Ok(())),
            ("create materialized view mv as select 1", V10, Ok(())),
            ("select * into t2 from t", V10, Ok(())),
            ("create table t2 (i int generated always as identity)", V10, Err(Unsupported)),
            ("create index i on t(a)", V10, Err(Unsupported)),
            ("create view v as select 1", V10, Err(Unsupported)),
            ("vacuum t", V10, Err(Unsupported)),
            ("analyze t", V10, Err(Unsupported)),
            ("begin", V10, Err(Unsupported)),
            ("set search_path = x", V10, Err(Unsupported)),
            ("do $$ begin end $$", V10, Err(Unsupported)),
            ("refresh materialized view mv", V10, Err(Unsupported)),
            ("explain select 1", V10, Err(AlreadyExplain)),
            ("/* c */ EXPLAIN (analyze) delete from t", V10, Err(AlreadyExplain)),
            ("-- yalnız yorum", V10, Err(Scope)),
            ("", V10, Err(Scope)),
        ];
        for (sql, v, want) in cases {
            assert_eq!(explainable(sql, v), want, "{sql}");
        }
    }

    fn opts() -> ExplainOptions {
        ExplainOptions::default()
    }

    #[test]
    fn option_gates_and_text() {
        let cases: Vec<(&str, ExplainOptions, u32)> = vec![
            ("varsayılan", opts(), V10),
            ("analyze", ExplainOptions { analyze: true, ..opts() }, V10),
            ("analyze+buffers", ExplainOptions { analyze: true, buffers: true, ..opts() }, V10),
            ("analyze pg18 (buffers kapalı)", ExplainOptions { analyze: true, ..opts() }, V18),
            ("analyze+buffers pg18", ExplainOptions { analyze: true, buffers: true, ..opts() }, V18),
            ("buffers analyzesiz pg12", ExplainOptions { buffers: true, ..opts() }, V12),
            ("buffers analyzesiz pg13", ExplainOptions { buffers: true, ..opts() }, V13),
            ("settings pg10", ExplainOptions { settings: true, ..opts() }, V10),
            ("settings pg12", ExplainOptions { settings: true, ..opts() }, V12),
            ("wal pg12", ExplainOptions { analyze: true, wal: true, ..opts() }, V12),
            ("wal analyzesiz pg13", ExplainOptions { wal: true, ..opts() }, V13),
            ("wal pg13", ExplainOptions { analyze: true, wal: true, ..opts() }, V13),
            ("timing kapalı analyzesiz (yok sayılır)", ExplainOptions { timing: false, ..opts() }, V10),
            ("timing kapalı", ExplainOptions { analyze: true, timing: false, ..opts() }, V10),
            ("summary kapalı analyzesiz", ExplainOptions { summary: false, ..opts() }, V10),
            ("summary kapalı analyze", ExplainOptions { analyze: true, summary: false, ..opts() }, V10),
            ("costs kapalı + verbose", ExplainOptions { costs: false, verbose: true, ..opts() }, V10),
            (
                "hepsi pg17",
                ExplainOptions {
                    analyze: true,
                    buffers: true,
                    verbose: true,
                    settings: true,
                    wal: true,
                    timing: false,
                    costs: false,
                    summary: false,
                    rollback: true,
                },
                V17,
            ),
        ];
        let mut out = String::new();
        for (name, o, v) in cases {
            let r = match explain_sql("select * from t", &o, v) {
                Ok(s) => s,
                Err(e) => format!("HATA {:?}: {}", e.code(), e.message()),
            };
            out.push_str(&format!("{name} [{v}]\n  {r}\n"));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn multi_statement_scope_is_rejected() {
        for sql in ["select 1; drop table t", "select 1; select 2;", ";", "  ", "-- c\n/* d */"] {
            assert_eq!(explain_sql(sql, &opts(), V17), Err(ExplainError::Scope), "{sql}");
        }
    }

    #[test]
    fn leading_comments_kept_trailing_semicolon_and_comments_trimmed() {
        for (sql, want) in [
            ("select 1;", "select 1"),
            ("select 1 ; -- son", "select 1"),
            ("-- baş\n/* c */ select 1 -- son\n", "-- baş\n/* c */ select 1"),
            ("  /*+ IndexScan(t) */ select * from t;", "/*+ IndexScan(t) */ select * from t"),
            ("/* a */ ; /* b */ select 1", "/* b */ select 1"),
            ("select 1 /* son */", "select 1"),
            ("select a -- ara\n  from t;\n", "select a -- ara\n  from t"),
            ("select ';' as x;", "select ';' as x"),
        ] {
            assert_eq!(explain_sql(sql, &opts(), V17).unwrap(), format!("EXPLAIN (FORMAT JSON, SUMMARY) {want}"), "{sql:?}");
        }
    }

    #[test]
    fn rollback_condition_table() {
        use StmtKind::*;
        use TxState::*;
        let on = ExplainOptions { analyze: true, ..opts() };
        let cases = [
            (on, Write, Idle, true),
            (on, Write, InTransaction, false),
            (on, Write, Failed, false),
            (on, Select, Idle, false),
            (on, Select, InTransaction, false),
            (ExplainOptions { rollback: false, ..on }, Write, Idle, false),
            (ExplainOptions { analyze: false, ..on }, Write, Idle, false),
        ];
        for (o, kind, state, want) in cases {
            assert_eq!(auto_rollback(&o, kind, state), want, "{o:?} {kind:?} {state:?}");
        }
        assert_eq!(note(&on, Write, true, Idle).as_deref(), Some(ROLLED_BACK_NOTE));
        assert_eq!(note(&on, Write, false, InTransaction).as_deref(), Some(IN_TX_NOTE));
        assert_eq!(note(&on, Write, false, Idle), None);
        assert_eq!(note(&on, Select, false, InTransaction), None);
        assert_eq!(note(&opts(), Write, false, InTransaction), None);
    }

    #[test]
    fn wrapped_text_is_classified_as_write_only_with_analyze() {
        use crate::classify::analyze;
        let a = ExplainOptions { analyze: true, ..opts() };
        for (stmt, o, kind) in [
            ("delete from t", a, StmtKind::Write),
            ("delete from t", opts(), StmtKind::Select),
            ("select * from t for update", a, StmtKind::Select),
            ("execute q", a, StmtKind::Write),
            ("create table t2 as select 1", a, StmtKind::Write),
            ("select 1", a, StmtKind::Select),
        ] {
            let sql = explain_sql(stmt, &o, V17).unwrap();
            assert_eq!(analyze(&sql).kind, kind, "{sql}");
        }
        assert!(analyze(&explain_sql("delete from t", &a, V17).unwrap()).danger.is_some());
        // Baştaki yorum (ipucu) sınıflandırmayı şaşırtmaz.
        let hinted = explain_sql("/* x */ delete from t", &a, V17).unwrap();
        assert_eq!(hinted, "EXPLAIN (FORMAT JSON, ANALYZE) /* x */ delete from t");
        assert_eq!(analyze(&hinted).kind, StmtKind::Write);
        assert!(analyze(&hinted).danger.is_some());
        assert_eq!(analyze(&explain_sql("-- x\ndelete from t", &opts(), V17).unwrap()).kind, StmtKind::Select);
        // "Zaten EXPLAIN" ve uygunluk baştaki yorumun ötesine bakar.
        assert_eq!(explain_sql("/*+ x */ explain select 1", &opts(), V17), Err(ExplainError::AlreadyExplain));
        assert_eq!(explain_sql("-- x\nvacuum t", &opts(), V17), Err(ExplainError::Unsupported));
    }

    #[test]
    fn errors_round_trip_through_messages() {
        for e in ALL_ERRORS {
            assert_eq!(ExplainError::from_message(e.message()), Some(e));
            assert!(e.code().as_str().starts_with("explain_"));
        }
        assert_eq!(ExplainError::from_message("başka"), None);
    }

    #[test]
    fn options_serialize_camel_case() {
        let o: ExplainOptions = serde_json::from_str(
            r#"{"analyze":true,"buffers":false,"verbose":false,"settings":false,"wal":false,"timing":true,"costs":true,"summary":true,"rollback":true}"#,
        )
        .unwrap();
        assert_eq!(o, ExplainOptions { analyze: true, ..opts() });
        let r = ExplainResult {
            json: "[]".into(),
            notices: vec![],
            elapsed_ms: 3,
            rolled_back: true,
            sql: "EXPLAIN".into(),
            note: None,
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::json!({"json": "[]", "notices": [], "elapsedMs": 3, "rolledBack": true, "sql": "EXPLAIN", "note": null})
        );
    }
}
