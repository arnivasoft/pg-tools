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
    /// Bağlı sunucunun sürümüne göre seçilen `pg_dump` klasörü (bağlı değilse ya da uygun yoksa `None`).
    pub chosen_dump: Option<String>,
    /// Seçilen `pg_restore` klasörü: sunucuyla aynı ana sürüm, yoksa en yeni (arşiv sürümü burada bilinmez).
    pub chosen_restore: Option<String>,
    /// Seçilen `psql` klasörü (en yeni).
    pub chosen_psql: Option<String>,
}

impl ToolDetection {
    /// Seçimleri işaretler: `pg_dump` yalnız sunucu sürümü biliniyorsa (`server_version_num` > 0), uygun araç
    /// yoksa `problem` seçim iletisiyle doldurulur (önceki sorun yoksa).
    pub fn with_choices(mut self, server_version_num: Option<u32>) -> Self {
        let dir = |kind, v| crate::tools::choose(&self.sets, kind, v).map(|s| s.dir.clone());
        // Arşiv bilinmez: sunucuyla aynı ana sürüm, yoksa en yeni (`restore_plan` arşive göre yeniden seçer).
        self.chosen_restore = crate::tools::choose_restore(&self.sets, server_version_num.unwrap_or(0), None)
            .map(|s| s.dir.clone())
            .ok();
        self.chosen_psql = dir(ToolKind::Psql, 0).ok();
        if let Some(v) = server_version_num.filter(|v| *v > 0) {
            match dir(ToolKind::PgDump, v) {
                Ok(d) => self.chosen_dump = Some(d),
                Err(m) => {
                    if self.problem.is_none() {
                        self.problem = Some(m);
                    }
                }
            }
        }
        self
    }
}

/// Komut önizlemesi ve doğrulama sonucu (`backup_plan`/`restore_plan`; çalıştırmaz). `errors` boş değilse
/// `preview` boştur ve iş başlatılamaz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobPlan {
    /// Parolasız tek satır (`PGPASSWORD=***`).
    pub preview: String,
    pub risk: pgt_ddl::Risk,
    pub notes: Vec<String>,
    pub errors: Vec<pgt_ddl::FieldError>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn set(dir: &str, num: u32) -> ToolSet {
        ToolSet {
            dir: dir.into(),
            version: ToolVersion { major: num / 10_000, version_num: num, text: dir.into() },
            pg_dump: true,
            pg_restore: true,
            psql: true,
        }
    }

    #[test]
    fn choices_follow_server_version() {
        let d = ToolDetection { sets: vec![set("16", 160004), set("18", 180001)], problem: None, chosen_dump: None, chosen_restore: None, chosen_psql: None };
        let c = d.clone().with_choices(Some(160002));
        assert_eq!((c.chosen_dump.as_deref(), c.chosen_restore.as_deref(), c.chosen_psql.as_deref()), (Some("16"), Some("16"), Some("18")));
        assert_eq!(d.clone().with_choices(Some(170000)).chosen_restore.as_deref(), Some("18"));
        assert!(c.problem.is_none());
        // Bağlı değil: pg_dump seçilmez, sorun yazılmaz.
        let c = d.clone().with_choices(None);
        assert_eq!((c.chosen_dump, c.problem), (None, None));
        // Sunucu araçlardan yeni: sorun iletisi.
        let c = d.with_choices(Some(190000));
        assert!(c.chosen_dump.is_none() && c.problem.as_deref().is_some_and(|p| p.contains("19")));
    }
}
