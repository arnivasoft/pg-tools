//! Nesne oluşturma/düzenleme diyaloglarının DDL arka ucu: tırnaklama, tanım modeli, plan/risk ve hata kodları.
//! Saf üreticiler `build` altında; katalog okuma `load`/`lookup`; önizleme/kapılı uygulama/iptal `DdlManager`'da.

mod apply;
pub mod acl;
pub mod build;
pub mod data;
pub mod error_codes;
pub mod load;
pub mod lookup;
pub mod manager;
pub mod model;
pub mod plan;
pub mod quote;
pub mod secret;
pub mod target;

pub use build::{BuildCtx, ScanResult, scan_semicolons};
pub use error_codes::{DDL_ERROR_CODES, DdlErrorCode, ddl_payload};
pub use load::{INVALID_UTF8_REASON, ObjectRef, load, server_version};
pub use lookup::{LookupItem, LookupRequest, lookup};
pub use manager::{ApplyOptions, ApplyOutcome, CreateContext, DdlManager, PlanRequest};
pub use acl::*;
pub use model::*;
pub use plan::{Plan, PlanView, REDACTED, Risk, Statement, plan_hash};
pub use secret::Secret;
pub use target::{CreatePreset, DdlCreate, DdlFocus, DdlTarget, target_of};
pub use quote::{qualified, quote_ident, quote_literal};
