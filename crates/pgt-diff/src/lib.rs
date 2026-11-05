//! Şema karşılaştırma arka ucu: iki şemadaki nesneleri listeler, anahtarla eşleştirir, tanımları
//! `pgt-ddl` yükleyicileriyle okuyup normalleştirilmiş DDL metinleriyle kıyaslar ve hedefi kaynağa
//! getirecek betiği üretir.
//!
//! Mimari: `key` (nesne anahtarı + eşleştirme) -> `model` (IPC türleri). Karşılaştırma salt okunurdur
//! (READ ONLY transaction, katalog profili); betik uygulanmaz, yalnız metin üretilir.

pub mod cancel;
pub mod key;
pub mod linediff;
pub mod list;
pub mod model;
pub mod script;
pub mod snapshot;

pub use key::{KeyClass, Listed, Pair, class_of, item_key, match_objects};
pub use model::*;
pub use linediff::{MAX_DIFF_LINES, ddl_text, line_diff};
pub use cancel::CancelToken;
pub use list::{SchemaCheck, list_objects, schema_check};
pub use snapshot::{LOAD_PIPELINE, LoadErrorClass, Loaded, PROGRESS_EVERY, SideSnapshot, SnapshotError, classify_load_error, load_side};
