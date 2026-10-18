//! Yedekleme/geri yükleme işinin arayüze akıttığı olaylar (spec §1 "İlerleme"). Olaylar parola taşımaz.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::lines::LineSeverity;

/// İş sonucu. `CompletedWithErrors`: `pg_restore` "hatada dur" kapalıyken hataları atlayıp sonuna dek çalıştı ve
/// `errors ignored on restore: N` yazıp 1 ile çıktı (veri yüklenmiş olabilir; `errors` hata satırı sayısıdır).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum JobOutcome {
    Success,
    CompletedWithErrors,
    Failed,
    Cancelled,
}

impl JobOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            JobOutcome::Success => "success",
            JobOutcome::CompletedWithErrors => "completed_with_errors",
            JobOutcome::Failed => "failed",
            JobOutcome::Cancelled => "cancelled",
        }
    }
}

/// İşin özeti; son `Finished` olayıyla aynı bilgidir ve komutun dönüş değeridir.
/// `warnings`/`errors` gösterilmeyen (sınır aşan) satırları da sayar. `output_path` yalnız başarıda doludur.
/// `hint` yalnız `failed` ve `completed_with_errors` sonuçlarında dolabilir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobSummary {
    pub outcome: JobOutcome,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub warnings: u32,
    pub errors: u32,
    pub output_path: Option<String>,
    pub hint: Option<String>,
    /// Satır sınırı (2000) yüzünden gösterilmeyen satır sayısı.
    pub truncated_lines: u32,
}

/// Olay sırası: `started` → sıfır ya da daha çok `line`/`progress` → `finished` (her zaman son olaydır).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "event", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum BackupEvent {
    /// Parolasız komut önizlemesi.
    Started { preview: String },
    Line { severity: LineSeverity, text: String },
    /// `bytes`: geçici çıktının boyutu (klasörde dosyaların toplamı); çıktısız işte `None`.
    Progress { done: u32, total: Option<u32>, bytes: Option<u64> },
    Finished {
        outcome: JobOutcome,
        exit_code: Option<i32>,
        elapsed_ms: u64,
        warnings: u32,
        errors: u32,
        output_path: Option<String>,
        hint: Option<String>,
        truncated_lines: u32,
    },
}

impl From<&JobSummary> for BackupEvent {
    fn from(s: &JobSummary) -> Self {
        BackupEvent::Finished {
            outcome: s.outcome,
            exit_code: s.exit_code,
            elapsed_ms: s.elapsed_ms,
            warnings: s.warnings,
            errors: s.errors,
            output_path: s.output_path.clone(),
            hint: s.hint.clone(),
            truncated_lines: s.truncated_lines,
        }
    }
}

/// Olay alıcısı. `send` `false` dönerse istemci gitmiştir: iş iptal edilir.
pub trait EventSink: Send {
    fn send(&mut self, e: BackupEvent) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_names_are_snake_case() {
        assert_eq!(serde_json::to_value(JobOutcome::CompletedWithErrors).unwrap(), serde_json::json!("completed_with_errors"));
        assert_eq!(JobOutcome::CompletedWithErrors.as_str(), "completed_with_errors");
    }

    #[test]
    fn events_serialize_as_tagged_camel_case() {
        let ev = BackupEvent::Progress { done: 3, total: Some(10), bytes: None };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            serde_json::json!({"event": "progress", "done": 3, "total": 10, "bytes": null})
        );
        let line = BackupEvent::Line { severity: LineSeverity::Warning, text: "x".into() };
        assert_eq!(
            serde_json::to_value(&line).unwrap(),
            serde_json::json!({"event": "line", "severity": "warning", "text": "x"})
        );
        let s = JobSummary {
            outcome: JobOutcome::Cancelled,
            exit_code: None,
            elapsed_ms: 5,
            warnings: 0,
            errors: 1,
            output_path: None,
            hint: None,
            truncated_lines: 2,
        };
        assert_eq!(
            serde_json::to_value(BackupEvent::from(&s)).unwrap(),
            serde_json::json!({"event": "finished", "outcome": "cancelled", "exitCode": null, "elapsedMs": 5,
                "warnings": 0, "errors": 1, "outputPath": null, "hint": null, "truncatedLines": 2})
        );
    }
}
