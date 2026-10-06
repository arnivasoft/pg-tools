//! Örnek / genel bakış / oturum sorguları. SQL metinleri sabittir ve asla günlüğe yazılmaz;
//! sütunlar sıraya değil ada göre okunur.

use pgt_core::{Error, Result};
use tokio_postgres::Client;

use crate::types::{
    ActivityReport, ActivityRow, BlockSession, LockReport, LockRow, Overview, PreparedXact, Sample, Setting, StatementRow, Statements,
    StatementsOrder,
};

const SAMPLE: &str = include_str!("sql/sample.sql");
const OVERVIEW: &str = include_str!("sql/overview.sql");
const ACTIVITY_PG10: &str = include_str!("sql/activity.sql");
const ACTIVITY_PG13: &str = include_str!("sql/activity.pg13.sql");
const LOCKS_SESSIONS: &str = include_str!("sql/locks_sessions.sql");
const LOCKS: &str = include_str!("sql/locks.sql");
const SETTINGS: &str = include_str!("sql/settings.sql");
const PREPARED: &str = include_str!("sql/prepared.sql");
const STATEMENTS_EXT: &str = include_str!("sql/statements_ext.sql");
const STATEMENTS_PG10: &str = include_str!("sql/statements.sql");
const STATEMENTS_PG13: &str = include_str!("sql/statements.pg13.sql");

/// Oturum listesinde gösterilecek en çok satır. SQL bir fazla (`LIMIT 5001`) satır getirir; `activity`
/// fazlayı atar ve aşım `ActivityReport::truncated` ile bildirilir.
pub const ACTIVITY_LIMIT: usize = 5000;
/// Kilit listesinde tutulan en çok satır (SQL `LIMIT 1001`); aşım `LockReport::truncated` ile bildirilir.
pub const LOCKS_LIMIT: usize = 1000;
/// `pg_stat_statements` sonucunda en çok satır (sabit).
const STATEMENTS_LIMIT: usize = 50;

/// Yetkisiz kullanıcıya PostgreSQL'in verdiği sabit (yerelleştirilmeyen) metin.
const NO_PRIVILEGE: &str = "<insufficient privilege>";

/// Ana sürüme göre en yüksek eşleşen varyant: `(en düşük ana sürüm, sql)`, artan sırada.
/// `version_num` `server_version_num`'dur (ör. 160004); hiçbiri eşleşmezse ilk varyant.
fn pick(version_num: u32, variants: &[(u32, &'static str)]) -> &'static str {
    let major = version_num / 10_000;
    variants.iter().rev().find(|(min, _)| major >= *min).or_else(|| variants.first()).map(|(_, s)| *s).expect("en az bir varyant")
}

/// Kümülatif sayaçlar ve oturum dağılımı; `db` verilirse yalnızca o veritabanı.
pub async fn sample(c: &Client, db: Option<&str>) -> Result<Sample> {
    let r = c.query_one(SAMPLE, &[&db]).await.map_err(Error::from_query)?;
    Ok(Sample {
        at: r.get("at"),
        xact_commit: r.get("xact_commit"),
        xact_rollback: r.get("xact_rollback"),
        tup_returned: r.get("tup_returned"),
        tup_fetched: r.get("tup_fetched"),
        tup_inserted: r.get("tup_inserted"),
        tup_updated: r.get("tup_updated"),
        tup_deleted: r.get("tup_deleted"),
        blks_read: r.get("blks_read"),
        blks_hit: r.get("blks_hit"),
        sessions_active: r.get("active"),
        sessions_idle: r.get("idle"),
        sessions_idle_in_tx: r.get("idle_in_tx"),
        sessions_other: r.get("other"),
    })
}

pub async fn overview(c: &Client, db: Option<&str>) -> Result<Overview> {
    let r = c.query_one(OVERVIEW, &[&db]).await.map_err(Error::from_query)?;
    Ok(Overview {
        version: r.get("version"),
        uptime_secs: r.get("uptime_secs"),
        max_connections: r.get("max_connections"),
        connections: r.get("connections"),
        oldest_xact_secs: r.get("oldest_xact_secs"),
        database: db.map(str::to_string),
    })
}

/// Oturum listesi: en çok `ACTIVITY_LIMIT` satır; SQL bir fazla getirdiyse `truncated` true olur.
pub async fn activity(c: &Client, version_num: u32, db: Option<&str>) -> Result<ActivityReport> {
    let sql = pick(version_num, &[(10, ACTIVITY_PG10), (13, ACTIVITY_PG13)]);
    let rows = c.query(sql, &[&db]).await.map_err(Error::from_query)?;
    let truncated = rows.len() > ACTIVITY_LIMIT;
    let rows = rows
        .iter()
        .take(ACTIVITY_LIMIT)
        .map(|r| ActivityRow {
            pid: r.get("pid"),
            leader_pid: r.get("leader_pid"),
            database: r.get("database"),
            user: r.get("user"),
            application: r.get::<_, Option<String>>("application").unwrap_or_default(),
            client_addr: r.get("client_addr"),
            backend_type: r.get::<_, Option<String>>("backend_type").unwrap_or_default(),
            state: r.get("state"),
            wait_event_type: r.get("wait_event_type"),
            wait_event: r.get("wait_event"),
            backend_age_secs: r.get("backend_age_secs"),
            xact_age_secs: r.get("xact_age_secs"),
            query_age_secs: r.get("query_age_secs"),
            state_age_secs: r.get("state_age_secs"),
            query: r.get::<_, Option<String>>("query").filter(|q| q != NO_PRIVILEGE),
            is_self: r.get("is_self"),
        })
        .collect();
    Ok(ActivityReport { rows, truncated })
}

/// Engelleme ilişkisindeki oturumlar ve kilitler; `db` verilirse yalnızca o veritabanının oturumları.
pub async fn locks(c: &Client, db: Option<&str>) -> Result<LockReport> {
    let sessions = c.query(LOCKS_SESSIONS, &[&db]).await.map_err(Error::from_query)?;
    let sessions = sessions
        .iter()
        .map(|r| BlockSession {
            pid: r.get("pid"),
            user: r.get("user"),
            application: r.get::<_, Option<String>>("application").unwrap_or_default(),
            state: r.get("state"),
            query: r.get::<_, Option<String>>("query").filter(|q| q != NO_PRIVILEGE),
            blocked_by: r.get::<_, Option<Vec<i32>>>("blocked_by").unwrap_or_default(),
            wait_secs: r.get("wait_secs"),
        })
        .collect();
    let rows = c.query(LOCKS, &[&db]).await.map_err(Error::from_query)?;
    let truncated = rows.len() > LOCKS_LIMIT;
    let locks = rows
        .iter()
        .take(LOCKS_LIMIT)
        .map(|r| LockRow {
            pid: r.get("pid"),
            locktype: r.get("locktype"),
            mode: r.get("mode"),
            granted: r.get("granted"),
            object: r.get("object"),
            database_oid: r.get::<_, Option<i64>>("database_oid").and_then(|v| u32::try_from(v).ok()),
        })
        .collect();
    Ok(LockReport { sessions, locks, truncated })
}

/// Tüm yapılandırma parametreleri (`pending_restart` PG 10+'da vardır).
pub async fn settings(c: &Client) -> Result<Vec<Setting>> {
    let rows = c.query(SETTINGS, &[]).await.map_err(Error::from_query)?;
    Ok(rows
        .iter()
        .map(|r| Setting {
            name: r.get("name"),
            value: r.get("value"),
            unit: r.get("unit"),
            category: r.get::<_, Option<String>>("category").unwrap_or_default(),
            description: r.get::<_, Option<String>>("description").unwrap_or_default(),
            source: r.get::<_, Option<String>>("source").unwrap_or_default(),
            context: r.get::<_, Option<String>>("context").unwrap_or_default(),
            boot_value: r.get("boot_value"),
            pending_restart: r.get("pending_restart"),
        })
        .collect())
}

/// Hazırlanmış (two-phase) transaction'lar.
pub async fn prepared(c: &Client) -> Result<Vec<PreparedXact>> {
    let rows = c.query(PREPARED, &[]).await.map_err(Error::from_query)?;
    Ok(rows
        .iter()
        .map(|r| PreparedXact {
            gid: r.get("gid"),
            transaction: r.get::<_, Option<String>>("transaction").unwrap_or_default(),
            owner: r.get::<_, Option<String>>("owner").unwrap_or_default(),
            database: r.get::<_, Option<String>>("database").unwrap_or_default(),
            age_secs: r.get::<_, Option<f64>>("age_secs").unwrap_or_default(),
        })
        .collect())
}

/// Sıralama ifadesi: yalnızca üç sabit dizeden biri (kullanıcı metni asla girmez). Eklenti 1.8'de
/// sütunlar `*_exec_time` olarak yeniden adlandırıldı. Eşitlikte kararlı sıra için `queryid` eklenir.
fn order_clause(order: StatementsOrder, exec_cols: bool) -> &'static str {
    match (order, exec_cols) {
        (StatementsOrder::TotalTime, true) => "total_exec_time DESC, queryid",
        (StatementsOrder::MeanTime, true) => "mean_exec_time DESC, queryid",
        (StatementsOrder::TotalTime, false) => "total_time DESC, queryid",
        (StatementsOrder::MeanTime, false) => "mean_time DESC, queryid",
        (StatementsOrder::Calls, _) => "calls DESC, queryid",
    }
}

/// Eklenti sürümünden ("1.10" gibi) `(ana, ara)` çıkarır; sayısal karşılaştırma için (dize sırası "1.10" < "1.8" verir).
fn parse_ext_version(v: &str) -> Option<(u32, u32)> {
    let mut it = v.trim().split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    Some((major, minor))
}

/// Sütun adları kurulu EKLENTİ sürümüne bağlıdır (pg_upgrade ile gelen kümelerde sunucu PG 13+ iken
/// eklenti ≤ 1.7 kalabilir). Ayrıştırılamazsa sunucu ana sürümüne düşülür.
fn uses_exec_cols(ext_version: &str, version_num: u32) -> bool {
    parse_ext_version(ext_version).map_or(version_num / 10_000 >= 13, |v| v >= (1, 8))
}

/// `pg_stat_statements` en çok 50 satır. Eklenti kurulu değilse ya da yüklenmemişse `Unavailable`.
/// Sorgu metni günlüğe/hata mesajına asla girmez.
pub async fn statements(c: &Client, version_num: u32, db: Option<&str>, order: StatementsOrder) -> Result<Statements> {
    let installed = c.query_opt(STATEMENTS_EXT, &[]).await.map_err(Error::from_query)?;
    let Some(installed) = installed else {
        return Ok(Statements::Unavailable { reason: "pg_stat_statements eklentisi bu veritabanında kurulu değil.".into() });
    };
    let exec_cols = uses_exec_cols(&installed.get::<_, String>("extversion"), version_num);
    let base = if exec_cols { STATEMENTS_PG13 } else { STATEMENTS_PG10 };
    let sql = format!("{base}\nORDER BY {} LIMIT {STATEMENTS_LIMIT}", order_clause(order, exec_cols));
    let rows = match c.query(sql.as_str(), &[&db]).await.map_err(Error::from_query) {
        Ok(rows) => rows,
        // 55000: kitaplık shared_preload_libraries'te değil.
        Err(Error::Query { sqlstate: Some(code), .. }) if code == "55000" => {
            return Ok(Statements::Unavailable { reason: "pg_stat_statements yüklenmemiş (shared_preload_libraries).".into() });
        }
        // 42P01: eklentinin görünümü arama yolunda değil (başka şemada kurulu).
        Err(Error::Query { sqlstate: Some(code), .. }) if code == "42P01" => {
            return Ok(Statements::Unavailable { reason: "pg_stat_statements eklentisinin şeması arama yolunda değil.".into() });
        }
        Err(e) => return Err(e),
    };
    let rows = rows
        .iter()
        .map(|r| StatementRow {
            query_id: r.get("query_id"),
            query: r.get::<_, Option<String>>("query").filter(|q| q != NO_PRIVILEGE),
            calls: r.get("calls"),
            total_ms: r.get("total_ms"),
            mean_ms: r.get("mean_ms"),
            rows: r.get("rows"),
            shared_blks_hit: r.get("shared_blks_hit"),
            shared_blks_read: r.get("shared_blks_read"),
        })
        .collect();
    Ok(Statements::Available { rows })
}

#[cfg(test)]
mod tests {
    use super::{
        order_clause, parse_ext_version, pick, uses_exec_cols, ACTIVITY_LIMIT, ACTIVITY_PG10, ACTIVITY_PG13, LOCKS,
        LOCKS_LIMIT,
    };
    use crate::types::StatementsOrder::*;

    #[test]
    fn order_clauses_are_fixed_per_version() {
        assert_eq!(order_clause(TotalTime, true), "total_exec_time DESC, queryid");
        assert_eq!(order_clause(MeanTime, false), "mean_time DESC, queryid");
        assert_eq!(order_clause(Calls, true), order_clause(Calls, false));
    }

    #[test]
    fn picks_by_major_version() {
        let v = [(10, "a"), (13, "b")];
        assert_eq!(pick(100000, &v), "a");
        assert_eq!(pick(120011, &v), "a");
        assert_eq!(pick(130000, &v), "b");
        assert_eq!(pick(180000, &v), "b");
        assert_eq!(pick(90600, &v), "a");
    }

    #[test]
    fn ext_version_parses_numerically() {
        assert_eq!(parse_ext_version("1.7"), Some((1, 7)));
        assert_eq!(parse_ext_version("1.10"), Some((1, 10)));
        assert_eq!(parse_ext_version("2.0"), Some((2, 0)));
        assert_eq!(parse_ext_version(""), None);
        assert_eq!(parse_ext_version("abc"), None);
        assert_eq!(parse_ext_version("1"), None);
    }

    #[test]
    fn exec_cols_follow_extension_version() {
        assert!(!uses_exec_cols("1.7", 160000), "eski eklenti PG 16'da bile total_time kullanır");
        assert!(uses_exec_cols("1.8", 130000));
        assert!(uses_exec_cols("1.10", 160000));
        assert!(uses_exec_cols("1.11", 180000));
        assert!(uses_exec_cols("2.0", 180000));
        // Ayrıştırılamazsa sunucu ana sürümüne düşülür.
        assert!(uses_exec_cols("garip", 130000));
        assert!(!uses_exec_cols("", 120000));
    }

    #[test]
    fn limits_match_sql() {
        for sql in [ACTIVITY_PG10, ACTIVITY_PG13] {
            assert!(sql.contains(&format!("LIMIT {}", ACTIVITY_LIMIT + 1)));
        }
        assert!(LOCKS.contains(&format!("LIMIT {}", LOCKS_LIMIT + 1)));
    }
}
