//! Yedekleme ve geri yükleme (spec 2026-10-18): PostgreSQL istemci araçlarını (`pg_dump`, `pg_restore`, `psql`)
//! bulma, sürüm seçimi ve (sonraki görevlerde) iş yürütme.
//!
//! `tracing`'e yalnız iş türü, süre, çıkış kodu ve sonuç yazılır; yol, argüman ve nesne adı yazılmaz.

pub mod model;
pub mod tools;

pub use model::{ToolDetection, ToolKind, ToolSet, ToolVersion};
