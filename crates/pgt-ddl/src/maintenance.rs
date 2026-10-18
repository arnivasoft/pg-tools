//! Bakım işlemleri (VACUUM / ANALYZE / REINDEX / CLUSTER) modeli (Plan 24). Saf veri; üretici `build::maintenance`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Bakım hedefinin türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MaintenanceTargetKind {
    Table,
    MaterializedView,
    Index,
    Schema,
    Database,
}

/// `INDEX_CLEANUP` değeri (PG 12+; `AUTO` PG 14+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum IndexCleanup {
    Auto,
    On,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VacuumOptions {
    pub full: bool,
    pub freeze: bool,
    pub analyze: bool,
    pub verbose: bool,
    pub disable_page_skipping: bool,
    /// PG 12+.
    pub skip_locked: bool,
    /// PG 12+ (`Auto` PG 14+).
    pub index_cleanup: Option<IndexCleanup>,
    /// PG 13+; 0–1024; `full` ile birlikte kullanılamaz.
    pub parallel: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AnalyzeOptions {
    pub verbose: bool,
    /// PG 12+.
    pub skip_locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReindexOptions {
    /// PG 12+; sistem nesnesinde değil.
    pub concurrently: bool,
    pub verbose: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ClusterOptions {
    pub verbose: bool,
    /// Tablonun kendi indeksi; yoksa tablodaki kümeleme indeksi kullanılır.
    pub using_index: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum MaintenanceOp {
    Vacuum(VacuumOptions),
    Analyze(AnalyzeOptions),
    Reindex(ReindexOptions),
    Cluster(ClusterOptions),
}

/// Yüklenmiş bakım hedefi (salt okunur kimlik).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MaintenanceTarget {
    pub kind: MaintenanceTargetKind,
    /// Şema/veritabanı hedefinde `None`.
    pub schema: Option<String>,
    pub name: String,
    /// İndeks hedefinde ait tablo adı (aynı şema).
    pub table: Option<String>,
    pub partitioned: bool,
    pub system: bool,
    /// Tabloda kümeleme indeksi varsa adı.
    pub cluster_index: Option<String>,
    /// Tablonun indeksleri (CLUSTER USING seçimi).
    pub indexes: Vec<String>,
    pub editable: bool,
    pub unsupported: Vec<String>,
}

/// Hedefe ve sürüme göre kullanılabilir işlemler (`"vacuum" | "analyze" | "reindex" | "cluster"`).
pub fn ops_for(kind: MaintenanceTargetKind, partitioned: bool, version_num: u32) -> &'static [&'static str] {
    use MaintenanceTargetKind::*;
    match kind {
        Table if !partitioned || version_num >= 150000 => &["vacuum", "analyze", "reindex", "cluster"],
        // Bölümlenmiş tabloda REINDEX PG 14+ (PG 11–13 uyarıyla atlar); CLUSTER PG 15+.
        Table if version_num >= 140000 => &["vacuum", "analyze", "reindex"],
        Table => &["vacuum", "analyze"],
        MaterializedView => &["vacuum", "analyze", "reindex"],
        // Bölümlenmiş indekste REINDEX PG 14+.
        Index if partitioned && version_num < 140000 => &[],
        Index | Schema => &["reindex"],
        Database => &["vacuum", "analyze", "reindex"],
    }
}

/// Süren bakım işleminin ilerlemesi (`pg_stat_progress_*`, uygulama bağlantısının pid'i için; Plan 24).
/// `phase` PG'nin İngilizce aşama metnidir (ön yüz çevirir ya da olduğu gibi gösterir). `done`/`total` bilinmiyorsa
/// `None` (belirsiz ilerleme); `total` olmadan `done` sayaçtır (ör. yazılan satır).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MaintenanceProgress {
    pub phase: String,
    #[ts(type = "number | null")]
    pub done: Option<u64>,
    #[ts(type = "number | null")]
    pub total: Option<u64>,
    #[ts(type = "\"blocks\" | \"tuples\" | \"indexes\" | null")]
    pub unit: Option<String>,
}
