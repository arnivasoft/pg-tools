//! Yedekleme modeli: araç kümeleri ve sürümler.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Bir istemci aracının `--version` çıktısından okunan sürüm.
/// `version_num` sunucunun `server_version_num` biçimindedir: "18.1" → 180001, "9.6.24" → 90624, "17beta1" → 170000.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolVersion {
    /// Ana sürüm (9.x için 9).
    pub major: u32,
    pub version_num: u32,
    /// Çıktıdaki sürüm metni ("16.4", "18devel").
    pub text: String,
}

/// Bir klasörde bulunan araçlar. `version` klasörün sürümüdür (önce `pg_dump`, yoksa diğerlerinden).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolSet {
    pub dir: String,
    pub version: ToolVersion,
    pub pg_dump: bool,
    pub pg_restore: bool,
    pub psql: bool,
}

/// Araç bulma sonucu: bulunan kümeler (aday sırasıyla) ve kullanıcıya gösterilecek sorun (varsa).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolDetection {
    pub sets: Vec<ToolSet>,
    pub problem: Option<String>,
}

/// Seçilecek araç türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    PgDump,
    PgRestore,
    Psql,
}

impl ToolKind {
    /// Uzantısız çalıştırılabilir adı.
    pub fn program(self) -> &'static str {
        match self {
            ToolKind::PgDump => "pg_dump",
            ToolKind::PgRestore => "pg_restore",
            ToolKind::Psql => "psql",
        }
    }

    pub(crate) fn has(self, set: &ToolSet) -> bool {
        match self {
            ToolKind::PgDump => set.pg_dump,
            ToolKind::PgRestore => set.pg_restore,
            ToolKind::Psql => set.psql,
        }
    }
}

/// İş türü (ilerleme sayımı ve günlük için). Düz SQL geri yükleme de `Restore`'dur (psql satırları sayılmaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Backup,
    Restore,
}
