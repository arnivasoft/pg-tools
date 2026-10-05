//! Çalıştırıcının arayüze akıttığı olaylar. Olaylar SQL metni taşımaz.

use pgt_core::ErrorPayload;
use pgt_core::connection::Notice;
use serde::Serialize;
use ts_rs::TS;

use crate::classify::StmtKind;

/// Bir NOTICE iletisinin (ve ayrıntı/ipucunun) en fazla karakter sayısı.
pub const NOTICE_TEXT_MAX_CHARS: usize = 2000;
/// İfade başına toplanan en fazla NOTICE sayısı; fazlası atılır.
pub const NOTICES_PER_STMT_MAX: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Column {
    pub name: String,
    pub type_name: String,
    pub type_oid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NoticeInfo {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
}

fn clip(s: String) -> String {
    match s.char_indices().nth(NOTICE_TEXT_MAX_CHARS) {
        Some((byte, _)) => s[..byte].to_string(),
        None => s,
    }
}

impl From<Notice> for NoticeInfo {
    fn from(n: Notice) -> Self {
        NoticeInfo {
            severity: n.severity,
            code: n.code,
            message: clip(n.message),
            detail: n.detail.map(clip),
            hint: n.hint.map(clip),
        }
    }
}

/// `charOffset`: ifadenin betikteki **karakter** (Unicode scalar) ofseti. `position`: PostgreSQL'in 1 tabanlı,
/// **ifade metnine** göre karakter konumu; betikteki yeri `charOffset + position - 1`'dir.
/// Bir ifadenin olay sırası: `started` → sıfır ya da daha çok `rows` (son `rows.more` imleçten daha fazla
/// satır alınabileceğini söyler) → `done`; ya da herhangi bir noktada `failed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum QueryEvent {
    Started { stmt_index: u32, kind: StmtKind, columns: Vec<Column>, char_offset: u32 },
    Rows { stmt_index: u32, rows: Vec<Vec<Option<String>>>, more: bool },
    Done {
        stmt_index: u32,
        tag: String,
        rows_affected: Option<u64>,
        rows_sent: u64,
        truncated: bool,
        elapsed_ms: u64,
        notices: Vec<NoticeInfo>,
    },
    Failed { stmt_index: u32, error: ErrorPayload, position: Option<u32>, char_offset: u32 },
    ScriptDone { stmts_run: u32, stopped_on_error: bool },
}

/// Olayların alıcısı. `send` `false` dönerse alıcı gitmiştir: çalıştırma durur, açık imleç kapatılır.
pub trait EventSink: Send {
    fn send(&mut self, ev: QueryEvent) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_as_tagged_camel_case() {
        let ev = QueryEvent::Rows { stmt_index: 2, rows: vec![vec![None, Some(String::new())]], more: true };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            serde_json::json!({"type": "rows", "stmtIndex": 2, "rows": [[null, ""]], "more": true})
        );
        let done = QueryEvent::ScriptDone { stmts_run: 1, stopped_on_error: false };
        assert_eq!(
            serde_json::to_value(&done).unwrap(),
            serde_json::json!({"type": "scriptDone", "stmtsRun": 1, "stoppedOnError": false})
        );
    }

    #[test]
    fn notice_text_is_clipped_by_chars() {
        let long = "ğ".repeat(NOTICE_TEXT_MAX_CHARS + 5);
        let n = Notice {
            severity: "NOTICE".into(),
            code: "00000".into(),
            message: long.clone(),
            detail: Some(long),
            hint: Some("kısa".into()),
        };
        let info = NoticeInfo::from(n);
        assert_eq!(info.message.chars().count(), NOTICE_TEXT_MAX_CHARS);
        assert_eq!(info.detail.unwrap().chars().count(), NOTICE_TEXT_MAX_CHARS);
        assert_eq!(info.hint.as_deref(), Some("kısa"));
    }
}
