//! Yapay zekâ asistanı arka ucu: Claude Messages API'sine akışlı (SSE) istek.
//!
//! Mimari: `model` (IPC türleri) -> `prompt` (istem metni) -> `sse` (olay ayrıştırma) -> `client` (HTTP).
//! Asistan yalnız metin üretir; SQL asla kendiliğinden çalıştırılmaz. İstem/yanıt metni günlüğe yazılmaz.

pub mod agent;
pub mod client;
pub mod conversation;
pub mod model;
pub mod prompt;
pub mod session_tools;
pub mod sse;
pub mod tools;

pub use agent::{Claude, Model, ToolExec, run_turn};
pub use client::{AiClient, Cancel};
pub use conversation::{Conversations, Pending};
pub use model::{AiEvent, AiMode, AiRequest};
