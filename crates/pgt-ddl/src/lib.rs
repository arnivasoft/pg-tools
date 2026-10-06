//! Nesne oluşturma/düzenleme diyaloglarının DDL arka ucu: tırnaklama, tanım modeli, plan/risk ve hata kodları.
//! Bu katman saftır (veritabanı mantığı yok); Saf üreticiler `build` altında; uygulama sonraki görevlerde eklenir.

pub mod build;
pub mod error_codes;
pub mod model;
pub mod plan;
pub mod quote;

pub use build::{BuildCtx, outer_semicolon_count};
pub use error_codes::{DDL_ERROR_CODES, DdlErrorCode, ddl_payload};
pub use model::*;
pub use plan::{Plan, PlanView, Risk, Statement, plan_hash};
pub use quote::{qualified, quote_ident, quote_literal};
