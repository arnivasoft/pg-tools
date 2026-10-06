//! Dashboard'un arayüze giden yazılı yapıları (camelCase; sayaçlar `number`).

use serde::Serialize;
use ts_rs::TS;

/// Grafik örneği: `pg_stat_database` kümülatif sayaçları + anlık oturum dağılımı.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Sample {
    /// Sunucu `clock_timestamp()` epoch saniyesi.
    #[ts(type = "number")]
    pub at: f64,
    #[ts(type = "number")]
    pub xact_commit: i64,
    #[ts(type = "number")]
    pub xact_rollback: i64,
    #[ts(type = "number")]
    pub tup_returned: i64,
    #[ts(type = "number")]
    pub tup_fetched: i64,
    #[ts(type = "number")]
    pub tup_inserted: i64,
    #[ts(type = "number")]
    pub tup_updated: i64,
    #[ts(type = "number")]
    pub tup_deleted: i64,
    #[ts(type = "number")]
    pub blks_read: i64,
    #[ts(type = "number")]
    pub blks_hit: i64,
    #[ts(type = "number")]
    pub sessions_active: i64,
    #[ts(type = "number")]
    pub sessions_idle: i64,
    /// `idle in transaction` + `idle in transaction (aborted)`.
    #[ts(type = "number")]
    pub sessions_idle_in_tx: i64,
    #[ts(type = "number")]
    pub sessions_other: i64,
}

/// Genel bakış kartları.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Overview {
    /// `version()` tam metni.
    pub version: String,
    #[ts(type = "number")]
    pub uptime_secs: f64,
    #[ts(type = "number")]
    pub max_connections: i64,
    /// Client backend sayısı.
    #[ts(type = "number")]
    pub connections: i64,
    #[ts(type = "number | null")]
    pub oldest_xact_secs: Option<f64>,
    /// Veritabanı düzeyinde görünümde ad; sunucu düzeyinde `None`.
    pub database: Option<String>,
}

/// `pg_stat_activity` satırı.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ActivityRow {
    pub pid: i32,
    pub leader_pid: Option<i32>,
    pub database: Option<String>,
    pub user: Option<String>,
    pub application: String,
    pub client_addr: Option<String>,
    pub backend_type: String,
    pub state: Option<String>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
    #[ts(type = "number | null")]
    pub backend_age_secs: Option<f64>,
    #[ts(type = "number | null")]
    pub xact_age_secs: Option<f64>,
    #[ts(type = "number | null")]
    pub query_age_secs: Option<f64>,
    #[ts(type = "number | null")]
    pub state_age_secs: Option<f64>,
    /// Yetkisiz kullanıcıda başkalarının sorgusu `None`.
    pub query: Option<String>,
    /// Bu dashboard bağlantısı.
    pub is_self: bool,
}
