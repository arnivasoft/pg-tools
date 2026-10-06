//! Örnek / genel bakış / oturum sorguları. SQL metinleri sabittir ve asla günlüğe yazılmaz;
//! sütunlar sıraya değil ada göre okunur.

use pgt_core::{Error, Result};
use tokio_postgres::Client;

use crate::types::{ActivityRow, Overview, Sample};

const SAMPLE: &str = include_str!("sql/sample.sql");
const OVERVIEW: &str = include_str!("sql/overview.sql");
const ACTIVITY_PG10: &str = include_str!("sql/activity.sql");
const ACTIVITY_PG13: &str = include_str!("sql/activity.pg13.sql");

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

/// Oturum listesi (en çok 5001 satır; üst katman kırpmayı belirler).
pub async fn activity(c: &Client, version_num: u32, db: Option<&str>) -> Result<Vec<ActivityRow>> {
    let sql = pick(version_num, &[(10, ACTIVITY_PG10), (13, ACTIVITY_PG13)]);
    let rows = c.query(sql, &[&db]).await.map_err(Error::from_query)?;
    Ok(rows
        .iter()
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
        .collect())
}

#[cfg(test)]
mod tests {
    use super::pick;

    #[test]
    fn picks_by_major_version() {
        let v = [(10, "a"), (13, "b")];
        assert_eq!(pick(100000, &v), "a");
        assert_eq!(pick(120011, &v), "a");
        assert_eq!(pick(130000, &v), "b");
        assert_eq!(pick(180000, &v), "b");
        assert_eq!(pick(90600, &v), "a");
    }
}
