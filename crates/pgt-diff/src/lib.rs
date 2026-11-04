//! Şema karşılaştırma arka ucu: iki şemadaki nesneleri listeler, anahtarla eşleştirir, tanımları
//! `pgt-ddl` yükleyicileriyle okuyup normalleştirilmiş DDL metinleriyle kıyaslar ve hedefi kaynağa
//! getirecek betiği üretir.
//!
//! Mimari: `key` (nesne anahtarı + eşleştirme) -> `model` (IPC türleri). Karşılaştırma salt okunurdur
//! (READ ONLY transaction, katalog profili); betik uygulanmaz, yalnız metin üretilir.

pub mod key;
pub mod model;

pub use key::{KeyClass, Listed, Pair, class_of, item_key, match_objects};
pub use model::*;
