//! Yürütücü testlerinin ortak yardımcıları: sahte araçla `Invocation` ve olay toplayan alıcı.
#![allow(dead_code)]

use pgt_backup::{BackupEvent, EventSink, Invocation, LineSeverity, OutputPlan, PARTIAL_SUFFIX, ToolKind};
use pgt_ddl::Risk;

pub const FAKE: &str = env!("CARGO_BIN_EXE_pgt-fake-tool");

pub fn inv(args: &[&str], output: Option<OutputPlan>) -> Invocation {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    Invocation {
        exe: FAKE.to_string(),
        preview: format!("pg_dump {}", args.join(" ")),
        args,
        env: vec![("PGHOST".into(), "h.example".into()), ("LC_MESSAGES".into(), "C".into())],
        env_remove: vec!["LC_ALL".into(), "LANGUAGE".into()],
        risk: Risk::Safe,
        notes: Vec::new(),
        output,
        tool: ToolKind::PgDump,
        tolerates_ignored_errors: false,
    }
}

/// `--line-hex` değeri: UTF-8 baytlarının onaltılık yazımı (ASCII dışı metin argv'ye girmez).
pub fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

pub fn plan(final_path: &std::path::Path, is_dir: bool) -> OutputPlan {
    let final_path = final_path.to_string_lossy().into_owned();
    OutputPlan { partial_path: format!("{final_path}{PARTIAL_SUFFIX}"), final_path, is_dir }
}

type OnLine = Box<dyn FnMut(&str) -> bool + Send>;

/// Olayları toplar; `on_line` `false` dönerse alıcı "gitmiş" olur.
#[derive(Default)]
pub struct VecSink {
    pub events: Vec<BackupEvent>,
    pub on_line: Option<OnLine>,
}

impl VecSink {
    pub fn with(on_line: impl FnMut(&str) -> bool + Send + 'static) -> Self {
        VecSink { events: Vec::new(), on_line: Some(Box::new(on_line)) }
    }

    pub fn lines(&self) -> Vec<(LineSeverity, String)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                BackupEvent::Line { severity, text } => Some((*severity, text.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn progress(&self) -> Vec<(u32, Option<u32>, Option<u64>)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                BackupEvent::Progress { done, total, bytes } => Some((*done, *total, *bytes)),
                _ => None,
            })
            .collect()
    }
}

impl EventSink for VecSink {
    fn send(&mut self, e: BackupEvent) -> bool {
        let keep = match (&e, self.on_line.as_mut()) {
            (BackupEvent::Line { text, .. }, Some(f)) => f(text),
            _ => true,
        };
        self.events.push(e);
        keep
    }
}
