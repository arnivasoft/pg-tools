//! Üretilen DDL planı: ifadeler, risk düzeyi ve bütünlük özeti.

use std::hash::{DefaultHasher, Hash, Hasher};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::FieldError;

/// Risk düzeyi; sıralama artan önem (`Safe < Destructive < DataLoss`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Risk {
    Safe,
    Destructive,
    DataLoss,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Statement {
    /// Tek ifade; sonda `;` YOKTUR (çalıştırıcı gerekirse ekler).
    pub sql: String,
    pub risk: Risk,
    /// Transaction içinde çalışabilir mi (`CREATE INDEX CONCURRENTLY` gibi ifadeler değildir).
    pub transactional: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub statements: Vec<Statement>,
    pub warnings: Vec<String>,
}

impl Plan {
    /// Planın en yüksek riski; boş plan `Safe`.
    pub fn risk(&self) -> Risk {
        self.statements.iter().map(|s| s.risk).max().unwrap_or(Risk::Safe)
    }
}

/// İfadelerin 16 haneli hex özeti (`DefaultHasher`: sabit anahtarlı SipHash). Yalnızca `apply`'ın
/// "önizlenen plan = uygulanan plan" bütünlük denetimi içindir; güvenlik amaçlı değildir.
pub fn plan_hash(statements: &[Statement]) -> String {
    let mut h = DefaultHasher::new();
    statements.len().hash(&mut h);
    for s in statements {
        s.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Önizleme yanıtı.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlanView {
    pub statements: Vec<Statement>,
    pub warnings: Vec<String>,
    pub errors: Vec<FieldError>,
    pub hash: String,
    pub risk: Risk,
    /// Hiç ifade yok (değişiklik yok / doğrulama hatası).
    pub empty: bool,
}

impl PlanView {
    /// Üretici sonucundan önizleme: doğrulama hataları `errors`'ta döner (ifade yok, hash boş planın hash'i).
    pub fn from_build(result: std::result::Result<Plan, Vec<FieldError>>) -> Self {
        let (plan, errors) = match result {
            Ok(plan) => (plan, vec![]),
            Err(errors) => (Plan::default(), errors),
        };
        PlanView {
            empty: plan.statements.is_empty(),
            hash: plan_hash(&plan.statements),
            risk: plan.risk(),
            statements: plan.statements,
            warnings: plan.warnings,
            errors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_build_puts_errors_in_view() {
        let e = FieldError { path: "name".into(), message: "boş".into() };
        let v = PlanView::from_build(Err(vec![e.clone()]));
        assert_eq!((v.errors, v.empty, v.hash, v.risk), (vec![e], true, plan_hash(&[]), Risk::Safe));
        let p = Plan { statements: vec![st("DROP X", Risk::Destructive)], warnings: vec!["w".into()] };
        let v = PlanView::from_build(Ok(p.clone()));
        assert_eq!((v.hash, v.risk, v.empty, v.warnings), (plan_hash(&p.statements), Risk::Destructive, false, p.warnings));
        assert!(v.errors.is_empty());
    }

    fn st(sql: &str, risk: Risk) -> Statement {
        Statement { sql: sql.into(), risk, transactional: true, note: None }
    }

    #[test]
    fn risk_is_ordered_and_plan_risk_is_max() {
        assert!(Risk::Safe < Risk::Destructive && Risk::Destructive < Risk::DataLoss);
        assert_eq!(Plan::default().risk(), Risk::Safe);
        let p = Plan { statements: vec![st("a", Risk::Safe), st("b", Risk::DataLoss), st("c", Risk::Destructive)], warnings: vec![] };
        assert_eq!(p.risk(), Risk::DataLoss);
    }

    #[test]
    fn hash_is_deterministic_and_sensitive() {
        let a = vec![st("A", Risk::Safe), st("B", Risk::Safe)];
        assert_eq!(plan_hash(&a), plan_hash(&a.clone()));
        assert_eq!(plan_hash(&a).len(), 16);
        let reordered = vec![st("B", Risk::Safe), st("A", Risk::Safe)];
        assert_ne!(plan_hash(&a), plan_hash(&reordered));
        assert_ne!(plan_hash(&a), plan_hash(&[st("A", Risk::Safe), st("C", Risk::Safe)]));
        assert_ne!(plan_hash(&a), plan_hash(&[st("A", Risk::Safe), st("B", Risk::Destructive)]));
        let mut nt = a.clone();
        nt[0].transactional = false;
        assert_ne!(plan_hash(&a), plan_hash(&nt));
        // Sınır kayması: ["AB"] ile ["A","B"] farklı olmalı.
        assert_ne!(plan_hash(&[st("AB", Risk::Safe)]), plan_hash(&a));
    }

    #[test]
    fn hash_of_empty_differs_from_nonempty() {
        assert_eq!(plan_hash(&[]), plan_hash(&[]));
        assert_ne!(plan_hash(&[]), plan_hash(&[st("x", Risk::Safe)]));
    }

    #[test]
    fn plan_view_serializes_camel_case() {
        let v = PlanView {
            statements: vec![st("CREATE SCHEMA x", Risk::Safe)],
            warnings: vec![],
            errors: vec![],
            hash: "0".repeat(16),
            risk: Risk::Safe,
            empty: false,
        };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["risk"], "safe");
        assert_eq!(j["statements"][0]["transactional"], true);
        assert_eq!(j["empty"], false);
        assert_eq!(serde_json::from_value::<PlanView>(j).unwrap(), v);
    }

    #[test]
    fn empty_plan_view_flag_follows_statements() {
        let p = Plan::default();
        let v = PlanView {
            empty: p.statements.is_empty(),
            hash: plan_hash(&p.statements),
            risk: p.risk(),
            statements: p.statements,
            warnings: p.warnings,
            errors: vec![],
        };
        assert!(v.empty);
        assert_eq!(v.risk, Risk::Safe);
    }
}
