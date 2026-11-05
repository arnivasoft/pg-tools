//! Yapay zekâ asistanı arka ucu: Claude Messages API'sine akışlı (SSE) istek.
//!
//! Mimari: `model` (IPC türleri) -> `prompt` (istem metni) -> `sse` (olay ayrıştırma) -> `client` (HTTP).
//! Asistan yalnız metin üretir; SQL asla kendiliğinden çalıştırılmaz. İstem/yanıt metni günlüğe yazılmaz.

pub mod client;
pub mod model;
pub mod prompt;
pub mod sse;

pub use client::{AiClient, Cancel};
pub use model::{AiEvent, AiMode, AiRequest};
