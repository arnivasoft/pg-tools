//! Arayüzün göndereceği birebir JSON (camelCase alanlar, `op`/`kind` etiketleri) ayrıştırılır ve geri yazılınca
//! aynı JSON çıkar: ts-rs tipleriyle serde şekli ayrışırsa bu test kırılır.

use pgt_core::ErrorPayload;
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
        (ObjectKind::Table, table_json()),
        (
            ObjectKind::View,
            json!({"kind": "view", "schema": "public", "name": "v", "owner": null, "query": "SELECT 1", "checkOption": "local",
                   "securityBarrier": false, "securityInvoker": true, "comment": null, "recreate": false,
                   "columns": [{"name": "?column?", "dataType": "integer"}], "unsupported": []}),
        ),
        (
            ObjectKind::MaterializedView,
            json!({"kind": "materialized_view", "schema": "public", "name": "m", "owner": "me", "query": "SELECT 1",
                   "tablespace": null, "fillfactor": null, "withData": true, "comment": "c", "refresh": null,
                   "columns": [], "unsupported": ["INDEX m_i"]}),
        ),
        (
            ObjectKind::Database,
            json!({"kind": "database", "name": "d", "owner": "me", "template": null, "encoding": "UTF8", "localeProvider": "libc",
                   "lcCollate": "C", "lcCtype": "C", "providerLocale": null, "tablespace": "pg_default", "connectionLimit": -1,
                   "isTemplate": false, "allowConnections": true, "settings": [{"name": "work_mem", "value": "4MB"}], "comment": null}),
        ),
        (
            ObjectKind::Role,
            json!({"kind": "role", "name": "r", "login": true, "superuser": false, "createdb": false, "createrole": false,
                   "inherit": true, "replication": false, "bypassrls": false, "connectionLimit": 3, "validUntil": null,
                   "password": "keep", "memberOf": [{"role": "g", "admin": false, "inheritOption": null, "setOption": null}],
                   "settings": [], "comment": null, "unsupported": []}),
        ),
        (
            ObjectKind::Function,
            json!({"kind": "function", "schema": "public", "name": "f", "owner": "me", "comment": null, "language": "plpgsql",
                   "args": [{"mode": "in", "name": "a", "typeName": "integer", "default": "0"}, {"mode": "out", "name": "b", "typeName": "text", "default": null}],
                   "returns": {"kind": "table", "columns": [{"name": "b", "typeName": "text"}]},
                   "body": "BEGIN\n  RETURN QUERY SELECT 'x';\nEND", "volatility": "volatile", "strict": false, "securityDefiner": false,
                   "leakproof": false, "parallel": "unsafe", "cost": "100", "rows": "1000", "settings": [], "recreate": false,
                   "editable": true, "unsupported": []}),
        ),
        (
            ObjectKind::Procedure,
            json!({"kind": "procedure", "schema": "public", "name": "p", "owner": "me", "comment": "c", "language": "sql",
                   "args": [], "returns": null, "body": "SELECT 1", "volatility": "volatile", "strict": false, "securityDefiner": true,
                   "leakproof": false, "parallel": "unsafe", "cost": null, "rows": null,
                   "settings": [{"name": "search_path", "value": "public"}], "recreate": false, "editable": true, "unsupported": []}),
        ),
        (
            ObjectKind::Trigger,
            json!({"kind": "trigger", "schema": "public", "table": "t", "name": "tr", "timing": "after",
                   "events": {"insert": true, "update": false, "updateColumns": [], "delete": true, "truncate": false},
                   "level": "statement", "when": null, "functionSchema": "public", "functionName": "tf", "args": [],
                   "oldTable": "o", "newTable": null, "constraint": false, "deferrable": false, "initiallyDeferred": false,
                   "enabled": "origin", "comment": null, "editable": true, "unsupported": []}),
        ),
        (
            ObjectKind::Type,
            json!({"kind": "type", "schema": "public", "name": "mood", "owner": "me", "comment": null,
                   "body": {"variant": "enum", "values": [{"label": "sad", "original": "sad"}, {"label": "ok", "original": null}]},
                   "editable": true, "unsupported": []}),
        ),
        (
            ObjectKind::Domain,
            json!({"kind": "domain", "schema": "public", "name": "posint", "owner": "me", "comment": null, "baseType": "integer",
                   "collation": null, "default": null, "notNull": false,
                   "constraints": [{"name": null, "expression": "VALUE > 0", "notValid": true, "validated": false, "original": null, "validate": false}],
                   "editable": true, "unsupported": []}),
        ),
    ]
}

#[test]
fn every_object_kind_has_a_json_fixture() {
    let kinds: Vec<ObjectKind> = defs().into_iter().map(|(k, _)| k).collect();
    assert_eq!(kinds.len(), 14);
    for k in [ObjectKind::Function, ObjectKind::Procedure, ObjectKind::Trigger, ObjectKind::Type, ObjectKind::Domain] {
        assert!(kinds.contains(&k), "{k:?}");
    }
}

#[test]
fn type_bodies_parse_in_every_variant() {
    for body in [
        json!({"variant": "composite", "attributes": [{"name": "a", "typeName": "integer", "collation": null, "original": "a"}]}),
        json!({"variant": "range", "subtype": "numeric", "subtypeOpclass": "numeric_ops", "collation": null, "canonical": null,
               "subtypeDiff": null, "multirangeTypeName": null}),
    ] {
        let mut v = defs()[12].1.clone();
        v["body"] = body;
        let _: ObjectDef = roundtrip(v);
    }
}

#[test]
fn trigger_function_target_carries_preset() {
    let u = "3f2a6d1e-8a4b-4c2e-9f00-0123456789ab";
    let v = serde_json::to_value(target_of(&format!("srv:{u}/db:5/sch:2200/trigfuncs")).unwrap()).unwrap();
    assert_eq!(v["create"], json!({"kind": "function", "context": {"schemaOid": 2200, "tableOid": null}, "preset": "trigger_function"}));
    let v = serde_json::to_value(target_of(&format!("srv:{u}/db:5/sch:2200/tbl:24576/trg:9")).unwrap()).unwrap();
    assert_eq!(v["object"], json!({"kind": "trigger", "oid": 9}));
    assert_eq!(v["refreshNodeId"], format!("srv:{u}/db:5/sch:2200/tbl:24576/triggers"));
}

/// Yüklenmiş biçimde tablo (attnum/oid ve yükleme alanları dolu); dört kısıt türünün etiketli biçimleriyle.
fn table_json() -> Value {
    json!({
        "kind": "table", "schema": "public", "name": "t", "owner": "me", "tablespace": null, "persistence": "permanent",
        "columns": [
            {"attnum": 1, "name": "id", "dataType": "bigint", "notNull": true, "default": null, "identity": "always",
             "generated": null, "collation": null, "comment": null, "using": null},
            {"attnum": 2, "name": "a", "dataType": "numeric(10,2)", "notNull": false, "default": "0", "identity": null,
             "generated": null, "collation": null, "comment": "c", "using": null},
            {"attnum": 3, "name": "g", "dataType": "text", "notNull": false, "default": null, "identity": null,
             "generated": {"expr": "a::text", "stored": true}, "collation": "C", "comment": null, "using": null}
        ],
        "constraints": [
            {"oid": 16401, "name": "t_pkey", "kind": {"type": "primary_key", "columns": ["id"], "include": []},
             "deferrable": false, "initiallyDeferred": false, "notValid": false},
            {"oid": 16402, "name": "t_a_key", "kind": {"type": "unique", "columns": ["a"], "include": [], "nullsNotDistinct": false},
             "deferrable": true, "initiallyDeferred": false, "notValid": false},
            {"oid": 16403, "name": "t_a_check", "kind": {"type": "check", "expr": "a > 0", "noInherit": false},
             "deferrable": false, "initiallyDeferred": false, "notValid": false},
            {"oid": 16404, "name": "t_fk", "kind": {"type": "foreign_key", "columns": ["id"], "refSchema": "public", "refTable": "u",
             "refColumns": ["id"], "matchType": "simple", "onUpdate": "cascade", "onDelete": "restrict"},
             "deferrable": false, "initiallyDeferred": false, "notValid": true}
        ],
        "partitionBy": null, "fillfactor": null, "comment": null,
        "partitionText": null, "isPartition": false, "inherits": false, "unsupported": []
    })
}

#[test]
fn plan_requests_parse_for_every_kind() {
    for (kind, def) in defs() {
        let c: PlanRequest = roundtrip(json!({"op": "create", "def": def, "concurrently": false}));
        assert!(matches!(&c, PlanRequest::Create { def, concurrently: false } if def.kind() == kind));
        let a: PlanRequest = roundtrip(json!({"op": "alter", "object": {"kind": kind, "oid": 16384}, "old": def, "new": def}));
        assert!(matches!(&a, PlanRequest::Alter { new, object, .. } if new.kind() == kind && object.kind == kind));
        let d: PlanRequest = roundtrip(json!({"op": "drop", "object": {"kind": kind, "oid": 16384}, "def": def, "cascade": true, "force": false}));
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
        "confirmed": true, "confirmedProduction": false, "password": null
    }));
    assert_eq!((o.lock_timeout_ms, o.confirmed, o.confirmed_production), (5000, true, false));
    // Parola girdisi düz dizgedir; geri yazımda değer asla çıkmaz.
    let o: ApplyOptions = serde_json::from_value(json!({
        "applyId": "3f2a6d1e-8a4b-4c2e-9f00-0123456789ab", "lockTimeoutMs": 0, "expectedHash": "h",
        "confirmed": false, "confirmedProduction": false, "password": "p@ss"
    }))
    .unwrap();
    // Değerin kendisi yalnızca crate içinde okunur (`Secret::expose` pub(crate)); burada varlığı ve maskesi denetlenir.
    assert!(o.password.is_some());
    assert!(!format!("{o:?}").contains("p@ss"));
    assert_eq!(serde_json::to_value(&o).unwrap()["password"], "********");
    // Drop isteği `force: true` taşır.
    let d: PlanRequest = roundtrip(json!({"op": "drop", "object": {"kind": "database", "oid": 1}, "def": defs()[0].1, "cascade": false, "force": true}));
    assert!(matches!(d, PlanRequest::Drop { force: true, .. }));
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
    let l: LookupRequest = roundtrip(json!({"kind": "locales", "provider": "icu"}));
    assert_eq!(l, LookupRequest::Locales { provider: LocaleProvider::Icu });
    for kind in ["grantable_roles", "encodings", "template_databases", "settings"] {
        let r: LookupRequest = roundtrip(json!({ "kind": kind }));
        assert!(r.server_wide(), "{kind}");
    }
    assert!(serde_json::from_value::<LookupRequest>(json!({"kind": "locales", "provider": "nope"})).is_err());
    let _: ObjectRef = roundtrip(json!({"kind": "table", "oid": 16385}));
}

#[test]
fn outcomes_serialize_as_the_frontend_expects() {
    let created = Some(ObjectRef { kind: ObjectKind::Schema, oid: 9 });
    let a = serde_json::to_value(ApplyOutcome::Applied { statements: 2, elapsed_ms: 7, created, warnings: None }).unwrap();
    assert_eq!(a, json!({"outcome": "applied", "statements": 2, "elapsedMs": 7, "created": {"kind": "schema", "oid": 9}}));
    let a = serde_json::to_value(ApplyOutcome::Applied { statements: 1, elapsed_ms: 1, created: None, warnings: Some(vec!["w".into()]) }).unwrap();
    assert!(a["created"].is_null());
    assert_eq!(a["warnings"], json!(["w"]));
    // Kısmi uygulama: `created` yalnızca doluysa yazılır.
    let error = ErrorPayload { code: "query".into(), message: "m".into(), detail: None, hint: None };
    let f = ApplyOutcome::Failed { failed_index: 1, committed: 1, sqlstate: Some("42704".into()), position: None, error: error.clone(), created: None };
    let v = serde_json::to_value(f).unwrap();
    assert!(v.get("created").is_none() && v["position"].is_null());
    let f = ApplyOutcome::Failed {
        failed_index: 1,
        committed: 1,
        sqlstate: None,
        position: None,
        error,
        created: Some(ObjectRef { kind: ObjectKind::Database, oid: 16390 }),
    };
    assert_eq!(serde_json::to_value(f).unwrap()["created"], json!({"kind": "database", "oid": 16390}));
}

#[test]
fn role_password_intent_and_secret_shapes() {
    // `set`/`clear` niyeti tanımda; değer hiçbir zaman.
    for intent in ["keep", "set", "clear"] {
        let mut v = defs()[8].1.clone();
        v["password"] = json!(intent);
        let d: ObjectDef = roundtrip(v);
        assert!(!serde_json::to_string(&d).unwrap().contains("********"));
    }
    // Girdi dizge, çıktı maskeli; Debug değer yazmaz.
    let o: ApplyOptions = serde_json::from_value(json!({
        "applyId": "3f2a6d1e-8a4b-4c2e-9f00-0123456789ab", "lockTimeoutMs": 0, "expectedHash": "h",
        "confirmed": false, "confirmedProduction": false, "password": "gizli-Q1"
    }))
    .unwrap();
    assert_eq!(serde_json::to_value(&o).unwrap()["password"], json!("********"));
    assert!(!format!("{o:?}").contains("gizli-Q1"));
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

#[test]
fn table_create_request_accepts_a_client_built_def_without_load_only_fields() {
    // İstemcinin kurduğu yeni tablo: attnum/oid null, yükleme alanları ve `using`/kısıt bayrakları gönderilmez.
    let v = json!({"op": "create", "concurrently": false, "def": {
        "kind": "table", "schema": "public", "name": "n", "owner": null, "tablespace": null, "persistence": "unlogged",
        "columns": [{"attnum": null, "name": "id", "dataType": "integer", "notNull": true, "default": null, "identity": null,
                     "generated": null, "collation": null, "comment": null}],
        "constraints": [{"oid": null, "name": null, "kind": {"type": "primary_key", "columns": ["id"], "include": []}}],
        "partitionBy": {"strategy": "hash", "keys": [{"kind": "column", "name": "id"}]}, "fillfactor": 80, "comment": null
    }});
    let r: PlanRequest = serde_json::from_value(v).unwrap();
    let PlanRequest::Create { def: ObjectDef::Table(t), .. } = r else { panic!("{r:?}") };
    assert_eq!((t.persistence, t.is_partition, t.inherits, t.unsupported.len()), (Persistence::Unlogged, false, false, 0));
    assert!(t.columns[0].using.is_none() && t.constraints[0].oid.is_none() && !t.constraints[0].not_valid);
    assert_eq!(t.partition_by.unwrap().strategy, PartitionStrategy::Hash);
    // Tanınmayan kısıt türü (EXCLUDE modellenmez) reddedilir.
    let mut bad = table_json();
    bad["constraints"][0]["kind"] = json!({"type": "exclude", "columns": ["id"]});
    assert!(serde_json::from_value::<ObjectDef>(bad).is_err());
}

#[test]
fn ddl_target_with_focus_serializes_as_the_frontend_expects() {
    let u = "3f2a6d1e-8a4b-4c2e-9f00-0123456789ab";
    let v = serde_json::to_value(target_of(&format!("srv:{u}/db:5/sch:2200/tbl:24576/con:40")).unwrap()).unwrap();
    assert_eq!(
        v,
        json!({"serverId": u, "databaseOid": 5, "object": {"kind": "table", "oid": 24576}, "create": null,
               "focus": {"kind": "constraint", "id": 40}, "refreshNodeId": format!("srv:{u}/db:5/sch:2200/tbl:24576")})
    );
    let v = serde_json::to_value(target_of(&format!("srv:{u}/db:5/sch:2200/tbl:24576/col:2")).unwrap()).unwrap();
    assert_eq!(v["focus"], json!({"kind": "column", "id": 2}));
    let v = serde_json::to_value(target_of(&format!("srv:{u}/db:5/sch:2200/tables")).unwrap()).unwrap();
    assert_eq!(
        v,
        json!({"serverId": u, "databaseOid": 5, "object": null, "create": {"kind": "table", "context": {"schemaOid": 2200, "tableOid": null}, "preset": null},
               "focus": null, "refreshNodeId": format!("srv:{u}/db:5/sch:2200/tables")})
    );
    assert_eq!(serde_json::to_value(DdlFocus::Column { id: 1 }).unwrap(), json!({"kind": "column", "id": 1}));
    // Sunucu kapsamlı: `databaseOid` null (yol `db:N` olsa bile).
    let v = serde_json::to_value(target_of(&format!("srv:{u}/db:5")).unwrap()).unwrap();
    assert_eq!(
        v,
        json!({"serverId": u, "databaseOid": null, "object": {"kind": "database", "oid": 5}, "create": null,
               "focus": null, "refreshNodeId": format!("srv:{u}/dbs")})
    );
    let v = serde_json::to_value(target_of(&format!("srv:{u}/roles")).unwrap()).unwrap();
    assert_eq!(v["create"], json!({"kind": "role", "context": {"schemaOid": null, "tableOid": null}, "preset": null}));
    assert!(v["databaseOid"].is_null());
    let v = serde_json::to_value(target_of(&format!("srv:{u}/db:5/sch:2200/view:30")).unwrap()).unwrap();
    assert_eq!((v["object"].clone(), v["databaseOid"].clone()), (json!({"kind": "view", "oid": 30}), json!(5)));
    assert_eq!(v["refreshNodeId"], format!("srv:{u}/db:5/sch:2200/views"));
}

#[test]
fn password_intent_deserialize_error_does_not_echo_value() {
    // Yanlış bağlanmış arayüz parolayı niyet alanına koyarsa hata iletisi değeri yankılamamalı.
    let secret = "Gizli-Parola-7x9";
    let mut v = defs()[8].1.clone();
    v["password"] = json!(secret);
    let e = serde_json::from_value::<ObjectDef>(v.clone()).unwrap_err().to_string();
    assert!(!e.contains(secret), "{e}");
    assert!(e.contains(PASSWORD_INTENT_MSG), "{e}");
    let req = json!({"op": "create", "def": v, "concurrently": false});
    let e = serde_json::from_value::<PlanRequest>(req).unwrap_err().to_string();
    assert!(!e.contains(secret), "{e}");
    // Dizge olmayan değer de yankılanmaz.
    let mut n = defs()[8].1.clone();
    n["password"] = json!(123456789);
    let e = serde_json::from_value::<ObjectDef>(n).unwrap_err().to_string();
    assert!(!e.contains("123456789"), "{e}");
    for ok in ["keep", "set", "clear"] {
        let mut v = defs()[8].1.clone();
        v["password"] = json!(ok);
        assert!(serde_json::from_value::<ObjectDef>(v).is_ok(), "{ok}");
    }
}

fn acl_json(kind: &str) -> Value {
    json!({"kind": kind, "schema": "public", "name": "t", "signature": null, "owner": "me", "viewer": "me", "explicit": true,
           "grants": [
               {"grantee": null, "privileges": [{"privilege": "select", "grantOption": false}]},
               {"grantee": "r", "privileges": [{"privilege": "temporary", "grantOption": true}]}
           ],
           "foreign": [{"grantor": "g", "grantee": null, "privilege": "usage", "grantOption": false}],
           "columns": [{"column": "c", "grantor": "me", "grantee": "r", "privilege": "update", "grantOption": false}],
           "editable": true, "unsupported": []})
}

#[test]
fn privileges_requests_roundtrip() {
    let r: PlanRequest = roundtrip(json!({"op": "privileges", "object": {"kind": "table", "oid": 16384},
        "old": acl_json("table"), "new": acl_json("table"), "cascade": true}));
    let PlanRequest::Privileges { old, cascade: true, .. } = &r else { panic!("{r:?}") };
    assert_eq!(old.grants[1].privileges[0], PrivilegeGrant { privilege: Privilege::Temporary, grant_option: true });
    assert_eq!(old.foreign[0].privilege, Privilege::Usage);

    let sec = |t: &str| json!({"objectType": t, "grants": [], "baseline": [{"grantee": null, "privileges": []}]});
    let def = json!({"role": "me", "schema": null, "viewer": "me", "sections": [sec("tables"), sec("schemas")], "editable": true, "unsupported": []});
    let d: PlanRequest = roundtrip(json!({"op": "default_privileges", "scope": {"role": null, "schemaOid": 2200}, "old": def, "new": def}));
    assert!(matches!(&d, PlanRequest::DefaultPrivileges { scope, .. } if scope.schema_oid == Some(2200) && scope.role.is_none()));
    // `cascade` zorunlu; bilinmeyen yetki adı reddedilir.
    assert!(serde_json::from_value::<PlanRequest>(json!({"op": "privileges", "object": {"kind": "table", "oid": 1}, "old": acl_json("table"), "new": acl_json("table")})).is_err());
    assert!(serde_json::from_value::<Privilege>(json!("TEMPORARY")).is_err());
}
