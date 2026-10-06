//! Arayüzün göndereceği birebir JSON (camelCase alanlar, `op`/`kind` etiketleri) ayrıştırılır ve geri yazılınca
//! aynı JSON çıkar: ts-rs tipleriyle serde şekli ayrışırsa bu test kırılır.

use pgt_ddl::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn roundtrip<T: Serialize + DeserializeOwned>(v: Value) -> T {
    let t: T = serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("ayrıştırılamadı: {e}: {v}"));
    assert_eq!(serde_json::to_value(&t).unwrap(), v, "geri yazım farklı");
    t
}

fn defs() -> Vec<(ObjectKind, Value)> {
    vec![
        (ObjectKind::Schema, json!({"kind": "schema", "name": "s", "owner": null, "comment": "c"})),
        (
            ObjectKind::Sequence,
            json!({"kind": "sequence", "schema": "public", "name": "q", "owner": "me", "dataType": "integer", "increment": "2",
                   "minValue": null, "maxValue": "100", "start": "5", "cache": "1", "cycle": true, "ownedBy": null, "comment": null}),
        ),
        (ObjectKind::Extension, json!({"kind": "extension", "name": "citext", "schema": "public", "version": null, "cascade": true})),
        (
            ObjectKind::Index,
            json!({"kind": "index", "schema": "public", "table": "t", "name": "i", "method": "btree", "unique": true,
                   "columns": [
                       {"key": {"kind": "column", "name": "id"}, "opclass": null, "collation": null, "descending": true, "nullsFirst": false},
                       {"key": {"kind": "expression", "sql": "lower(x)"}, "opclass": null, "collation": null, "descending": false, "nullsFirst": null}
                   ],
                   "include": ["a"], "whereClause": "id > 0", "tablespace": null, "fillfactor": 90, "comment": null, "constraint": null, "valid": true}),
        ),
    ]
}

#[test]
fn plan_requests_parse_for_every_kind() {
    for (kind, def) in defs() {
        let c: PlanRequest = roundtrip(json!({"op": "create", "def": def, "concurrently": false}));
        assert!(matches!(&c, PlanRequest::Create { def, concurrently: false } if def.kind() == kind));
        let a: PlanRequest = roundtrip(json!({"op": "alter", "object": {"kind": kind, "oid": 16384}, "old": def, "new": def}));
        assert!(matches!(&a, PlanRequest::Alter { new, object, .. } if new.kind() == kind && object.kind == kind));
        let d: PlanRequest = roundtrip(json!({"op": "drop", "object": {"kind": kind, "oid": 16384}, "def": def, "cascade": true}));
        assert!(matches!(&d, PlanRequest::Drop { def, cascade: true, .. } if def.kind() == kind));
        // `object` zorunludur: eksik olan alter/drop reddedilir.
        assert!(serde_json::from_value::<PlanRequest>(json!({"op": "drop", "def": def, "cascade": true})).is_err());
        assert!(serde_json::from_value::<PlanRequest>(json!({"op": "alter", "old": def, "new": def})).is_err());
    }
}

#[test]
fn unknown_op_or_kind_is_rejected() {
    assert!(serde_json::from_value::<PlanRequest>(json!({"op": "truncate", "def": defs()[0].1})).is_err());
    assert!(serde_json::from_value::<ObjectDef>(json!({"kind": "table", "name": "x"})).is_err());
}

#[test]
fn apply_options_parse() {
    let o: ApplyOptions = roundtrip(json!({
        "applyId": "3f2a6d1e-8a4b-4c2e-9f00-0123456789ab", "lockTimeoutMs": 5000, "expectedHash": "abc123",
        "confirmed": true, "confirmedProduction": false
    }));
    assert_eq!((o.lock_timeout_ms, o.confirmed, o.confirmed_production), (5000, true, false));
}

#[test]
fn supporting_requests_parse() {
    let _: ObjectRef = roundtrip(json!({"kind": "index", "oid": 16384}));
    let c: CreateContext = roundtrip(json!({"schemaOid": 2200, "tableOid": null}));
    assert_eq!((c.schema_oid, c.table_oid), (Some(2200), None));
    let _: LookupRequest = roundtrip(json!({"kind": "roles"}));
    let _: LookupRequest = roundtrip(json!({"kind": "extension_versions", "name": "citext"}));
    let _: LookupRequest = roundtrip(json!({"kind": "tables", "schema": null}));
    let _: LookupRequest = roundtrip(json!({"kind": "table_columns", "schema": "public", "table": "t"}));
    let t: LookupRequest = roundtrip(json!({"kind": "types", "query": "int"}));
    assert_eq!(t, LookupRequest::Types { query: Some("int".into()) });
    let _: LookupRequest = roundtrip(json!({"kind": "types", "query": null}));
    let c: LookupRequest = roundtrip(json!({"kind": "collations"}));
    assert_eq!(c, LookupRequest::Collations);
    let _: ObjectRef = roundtrip(json!({"kind": "table", "oid": 16385}));
}

#[test]
fn outcomes_serialize_as_the_frontend_expects() {
    let a = serde_json::to_value(ApplyOutcome::Applied { statements: 2, elapsed_ms: 7, created: Some(ObjectRef { kind: ObjectKind::Schema, oid: 9 }) })
        .unwrap();
    assert_eq!(a, json!({"outcome": "applied", "statements": 2, "elapsedMs": 7, "created": {"kind": "schema", "oid": 9}}));
    let a = serde_json::to_value(ApplyOutcome::Applied { statements: 1, elapsed_ms: 1, created: None }).unwrap();
    assert!(a["created"].is_null());
}

#[test]
fn sequence_numbers_accept_integers_on_input_but_serialize_as_strings() {
    let mut v = defs()[1].1.clone();
    v["increment"] = json!(2);
    v["maxValue"] = json!(100);
    v["start"] = json!(5);
    v["cache"] = json!(1);
    let d: ObjectDef = serde_json::from_value(v).unwrap();
    assert_eq!(serde_json::to_value(&d).unwrap(), defs()[1].1);
}

#[test]
fn index_valid_defaults_to_true_when_omitted() {
    let mut v = defs()[3].1.clone();
    v.as_object_mut().unwrap().remove("valid");
    let ObjectDef::Index(d) = serde_json::from_value::<ObjectDef>(v).unwrap() else { panic!() };
    assert!(d.valid);
}
