//! Dashboard motoru. Sorgu metinleri bu crate'te asla günlüğe yazılmaz.

pub mod manager;
pub mod queries;
pub mod types;

pub use manager::DashManager;
pub use types::{
    DashId, DashInfo, SignalKind, SignalOutcome,
    ActivityRow, BlockSession, LockReport, LockRow, Overview, PreparedXact, Sample, Setting, StatementRow, Statements,
    StatementsOrder,
};
