//! Dashboard'un arayüze giden yazılı yapıları (camelCase; sayaçlar `number`).

use serde::{Deserialize, Serialize};
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

/// Engelleme ilişkisine giren oturum (bekleyen ∪ engelleyen).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BlockSession {
    pub pid: i32,
    pub user: Option<String>,
    pub application: String,
    pub state: Option<String>,
    /// En çok 1024 karakter; yetkisiz kullanıcıda başkasının sorgusu `None`.
    pub query: Option<String>,
    /// `pg_blocking_pids`: bu oturumu bekleten pid'ler.
    pub blocked_by: Vec<i32>,
    /// `state_change`'ten bu yana geçen süre (bekleyen oturum için bekleme süresi).
    #[ts(type = "number | null")]
    pub wait_secs: Option<f64>,
}

/// `pg_locks` satırı.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LockRow {
    pub pid: i32,
    pub locktype: String,
    pub mode: String,
    pub granted: bool,
    /// Yalnızca bağlı veritabanındaki ilişki için `regclass` metni.
    pub object: Option<String>,
    pub database_oid: Option<u32>,
}

/// Kilit raporu; `truncated` true ise `locks` ilk `LOCKS_LIMIT` satıra kırpılmıştır.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LockReport {
    pub sessions: Vec<BlockSession>,
    pub locks: Vec<LockRow>,
    pub truncated: bool,
}

/// `pg_settings` satırı.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Setting {
    pub name: String,
    pub value: Option<String>,
    pub unit: Option<String>,
    pub category: String,
    pub description: String,
    pub source: String,
    pub context: String,
    pub boot_value: Option<String>,
    pub pending_restart: bool,
}

/// Hazırlanmış (two-phase) transaction.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PreparedXact {
    pub gid: String,
    pub transaction: String,
    pub owner: String,
    pub database: String,
    #[ts(type = "number")]
    pub age_secs: f64,
}

/// `pg_stat_statements` sıralama ölçütü (yalnızca bu üç sabit değer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StatementsOrder {
    TotalTime,
    MeanTime,
    Calls,
}

/// `pg_stat_statements` satırı.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StatementRow {
    pub query_id: Option<String>,
    /// En çok 2048 karakter; yetkisiz satırda `None`.
    pub query: Option<String>,
    #[ts(type = "number")]
    pub calls: i64,
    #[ts(type = "number")]
    pub total_ms: f64,
    #[ts(type = "number")]
    pub mean_ms: f64,
    #[ts(type = "number")]
    pub rows: i64,
    #[ts(type = "number")]
    pub shared_blks_hit: i64,
    #[ts(type = "number")]
    pub shared_blks_read: i64,
}

/// Eklenti durumuna göre sonuç.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Statements {
    Available { rows: Vec<StatementRow> },
    Unavailable { reason: String },
}
