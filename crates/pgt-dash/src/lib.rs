//! Dashboard motoru. Sorgu metinleri bu crate'te asla günlüğe yazılmaz.

pub mod manager;
pub mod queries;
pub mod types;

pub use manager::DashManager;
pub use types::{
    ActivityRow, BlockSession, DashId, DashInfo, LockReport, LockRow, Overview, PreparedXact,
    Sample, Setting, SignalKind, SignalOutcome, StatementRow, Statements, StatementsOrder,
};
