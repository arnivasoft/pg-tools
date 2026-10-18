//! Yedekleme ve geri yükleme (spec 2026-10-18): PostgreSQL istemci araçlarını (`pg_dump`, `pg_restore`, `psql`)
//! bulma, sürüm seçimi, saf argüman/ortam üretimi, format algılama, satır sınıflandırma, iş yürütme ve iş kaydı.
//!
//! `tracing`'e yalnız iş türü, süre, çıkış kodu ve sonuç yazılır; yol, argüman ve nesne adı yazılmaz.

pub mod args;
pub mod detect;
pub mod events;
pub mod inspect;
pub mod lines;
pub mod model;
pub mod registry;
pub mod runner;
pub mod tools;

pub use args::{
    BackupOptions, BackupRequest, BackupScope, Conn, DumpFormat, Invocation, OutputPlan, PARTIAL_SUFFIX,
    RestoreOptions, RestoreRequest, RestoreTarget, backup_invocation, dump_pattern, pattern_regex, restore_invocation,
    windows_name_problems,
};
pub use detect::{ArchiveInfo, SourceFormat, is_gzip, parse_list, sniff_format};
pub use lines::{LineSeverity, classify, failure_hint, progress_tick};
pub use inspect::{RestoreSource, inspect, sniff_path};
pub use model::{JobKind, JobPlan, ToolDetection, ToolKind, ToolSet, ToolVersion};
pub use events::{BackupEvent, EventSink, JobOutcome, JobSummary};
pub use registry::{CancelToken, JobGuard, JobRegistry, RegisterError};
pub use runner::{MAX_LINE_BYTES, MAX_LOG_LINES, Secret, check_output, run};
