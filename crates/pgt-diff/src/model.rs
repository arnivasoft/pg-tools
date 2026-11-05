//! Şema karşılaştırma IPC türleri. Struct'lar camelCase, enum'lar snake_case serileşir.

use pgt_ddl::{ObjectKind, Risk};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Karşılaştırmanın bir tarafı: sunucu, veritabanı ve şema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffSide {
    pub server_id: Uuid,
    #[ts(type = "number")]
    pub database_oid: u32,
    pub schema: String,
}

/// Karşılaştırma seçenekleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffOptions {
    pub ownership: bool,
    pub tablespace: bool,
    pub comments: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            ownership: false,
            tablespace: false,
            comments: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SideTag {
    Source,
    Target,
}

/// İlerleme olayı; her tarafta son olayda `loaded == total`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum DiffEvent {
    Progress {
        side: SideTag,
        loaded: u32,
        total: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ItemStatus {
    OnlySource,
    OnlyTarget,
    Different,
    Same,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ItemProblem {
    Manual,
    LoadFailed,
    BuildFailed,
}

/// Sonuç listesindeki bir nesne. `key` opaktır: ön yüz yalnız eşitlik ve geri gönderme için kullanır.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffItem {
    pub key: String,
    pub kind: ObjectKind,
    pub target_kind: Option<ObjectKind>,
    pub name: String,
    pub table: Option<String>,
    pub signature: Option<String>,
    pub status: ItemStatus,
    pub problem: Option<ItemProblem>,
    pub risk: Risk,
    pub notes: Vec<String>,
    pub errors: Vec<String>,
    pub selectable: bool,
    pub default_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffResult {
    pub items: Vec<DiffItem>,
    pub source_version: String,
    pub target_version: String,
    pub warnings: Vec<String>,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LineKind {
    Same,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    pub source_no: Option<u32>,
    pub target_no: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffDetail {
    pub source_text: String,
    pub target_text: String,
    pub lines: Option<Vec<DiffLine>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffScript {
    pub sql: String,
    pub transactional: bool,
    pub warnings: Vec<String>,
    pub risk: Risk,
    pub statements: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffDatabase {
    #[ts(type = "number")]
    pub oid: u32,
    pub name: String,
}

impl DiffItem {
    /// Seçilebilirlik kuralları (spec §2.4): `problem` doluysa ve `Same`'de seçilemez;
    /// varsayılan seçim yalnız seçilebilir `OnlySource`/`Different` için. `Same`'de risk `Safe`'tir.
    pub fn finalize_selection(&mut self) {
        self.selectable = self.problem.is_none() && self.status != ItemStatus::Same;
        self.default_selected = self.selectable
            && matches!(self.status, ItemStatus::OnlySource | ItemStatus::Different);
        if self.status == ItemStatus::Same {
            self.risk = Risk::Safe;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options() {
        let o = DiffOptions::default();
        assert_eq!(
            o,
            DiffOptions {
                ownership: false,
                tablespace: false,
                comments: true
            }
        );
    }

    #[test]
    fn progress_serializes_tagged() {
        let e = DiffEvent::Progress {
            side: SideTag::Source,
            loaded: 1,
            total: 2,
        };
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"event":"progress","side":"source","loaded":1,"total":2}"#
        );
    }

    #[test]
    fn status_snake_case() {
        assert_eq!(
            serde_json::to_string(&ItemStatus::OnlySource).unwrap(),
            r#""only_source""#
        );
    }

    fn item(status: ItemStatus, problem: Option<ItemProblem>) -> DiffItem {
        DiffItem {
            key: "k".into(),
            kind: ObjectKind::Table,
            target_kind: None,
            name: "x".into(),
            table: None,
            signature: None,
            status,
            problem,
            risk: Risk::DataLoss,
            notes: vec![],
            errors: vec![],
            selectable: true,
            default_selected: true,
        }
    }

    #[test]
    fn item_fields_camel_case() {
        let v = serde_json::to_value(item(ItemStatus::Different, None)).unwrap();
        assert!(v.get("targetKind").is_some());
        assert!(v.get("defaultSelected").is_some());
    }

    #[test]
    fn selection_rules() {
        let mut a = item(ItemStatus::Same, None);
        a.finalize_selection();
        assert!(!a.selectable && !a.default_selected);
        assert_eq!(a.risk, Risk::Safe);

        let mut b = item(ItemStatus::Different, Some(ItemProblem::Manual));
        b.finalize_selection();
        assert!(!b.selectable && !b.default_selected);

        let mut c = item(ItemStatus::OnlyTarget, None);
        c.finalize_selection();
        assert!(c.selectable && !c.default_selected);

        let mut d = item(ItemStatus::OnlySource, None);
        d.finalize_selection();
        assert!(d.selectable && d.default_selected);
    }
}
