//! Yedekleme ve geri yükleme (spec 2026-10-18): PostgreSQL istemci araçlarını (`pg_dump`, `pg_restore`, `psql`)
//! bulma, sürüm seçimi, saf argüman/ortam üretimi, format algılama, satır sınıflandırma ve (sonraki görevlerde)
//! iş yürütme.
//!
//! `tracing`'e yalnız iş türü, süre, çıkış kodu ve sonuç yazılır; yol, argüman ve nesne adı yazılmaz.

pub mod args;
pub mod detect;
pub mod lines;
pub mod model;
pub mod tools;

pub use args::{
    BackupOptions, BackupRequest, BackupScope, Conn, DumpFormat, Invocation, OutputPlan, PARTIAL_SUFFIX,
    RestoreOptions, RestoreRequest, RestoreTarget, backup_invocation, dump_pattern, restore_invocation,
};
pub use detect::{ArchiveInfo, SourceFormat, is_gzip, parse_list, sniff_format};
pub use lines::{LineSeverity, classify, failure_hint, progress_tick};
pub use model::{JobKind, ToolDetection, ToolKind, ToolSet, ToolVersion};
