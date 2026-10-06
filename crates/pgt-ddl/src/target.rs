//! Ağaç düğüm kimliğinden diyalog hedefi: arayüz kimliği ayrıştırmaz (`qt_target_of` ilkesi); düzenle/sil için nesne,
//! "Yeni …" için koleksiyon bağlamı buradan gelir. Desteklenmeyen düğümler için ikisi de `None` (arayüz menü açmaz).

use pgt_catalog::node::{Coll, NodeId, Seg};
use pgt_core::{Error, Result};
use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

use crate::{CreateContext, ObjectKind, ObjectRef};

/// "Yeni …" diyaloğunun türü ve bağlamı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DdlCreate {
    pub kind: ObjectKind,
    pub context: CreateContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DdlTarget {
    pub server_id: Uuid,
    #[ts(type = "number | null")]
    pub database_oid: Option<u32>,
    pub object: Option<ObjectRef>,
    pub create: Option<DdlCreate>,
}

/// Düğüm kimliğini çözer; bozuk kimlik ya da sunucuya ait olmayan (kök/grup) düğüm → `Error::Invalid`.
pub fn target_of(node_id: &str) -> Result<DdlTarget> {
    let NodeId::Node { server, path } = NodeId::parse(node_id)? else {
        return Err(Error::Invalid(format!("düğüm bir sunucuya ait değil: {node_id}")));
    };
    let database_oid = path.iter().find_map(|s| if let Seg::Db(o) = s { Some(*o) } else { None });
    let mut t = DdlTarget { server_id: server, database_oid, object: None, create: None };
    let obj = |kind, oid| Some(ObjectRef { kind, oid });
    let create = |kind, schema_oid, table_oid| Some(DdlCreate { kind, context: CreateContext { schema_oid, table_oid } });
    let prev = path.len().checked_sub(2).map(|i| path[i]);
    match path.last().copied() {
        Some(Seg::Schema(oid)) => t.object = obj(ObjectKind::Schema, oid),
        Some(Seg::Sequence(oid)) => t.object = obj(ObjectKind::Sequence, oid),
        Some(Seg::Extension(oid)) => t.object = obj(ObjectKind::Extension, oid),
        Some(Seg::Index(oid)) => t.object = obj(ObjectKind::Index, oid),
        Some(Seg::Coll(Coll::Schemas)) if matches!(prev, Some(Seg::Db(_))) => t.create = create(ObjectKind::Schema, None, None),
        Some(Seg::Coll(Coll::Extensions)) if matches!(prev, Some(Seg::Db(_))) => t.create = create(ObjectKind::Extension, None, None),
        Some(Seg::Coll(Coll::Sequences)) => {
            if let Some(Seg::Schema(s)) = prev {
                t.create = create(ObjectKind::Sequence, Some(s), None);
            }
        }
        Some(Seg::Coll(Coll::Indexes)) => {
            // Şema, yoldaki en yakın `Schema` parçasıdır; tablo koleksiyonun hemen üstündeki parçadır.
            if let Some(Seg::Table(tbl)) = prev
                && let Some(s) = path.iter().rev().find_map(|s| if let Seg::Schema(o) = s { Some(*o) } else { None })
            {
                t.create = create(ObjectKind::Index, Some(s), Some(tbl));
            }
        }
        _ => {}
    }
    // Veritabanı dışındaki düğümde (ör. sunucu düzeyi rol) nesne/oluşturma anlamsızdır.
    if t.database_oid.is_none() {
        t.object = None;
        t.create = None;
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "3f2a6d1e-8a4b-4c2e-9f00-0123456789ab";

    fn t(path: &str) -> DdlTarget {
        let id = if path.is_empty() { format!("srv:{U}") } else { format!("srv:{U}/{path}") };
        target_of(&id).unwrap()
    }
    fn obj(kind: ObjectKind, oid: u32) -> Option<ObjectRef> {
        Some(ObjectRef { kind, oid })
    }
    fn cr(kind: ObjectKind, schema_oid: Option<u32>, table_oid: Option<u32>) -> Option<DdlCreate> {
        Some(DdlCreate { kind, context: CreateContext { schema_oid, table_oid } })
    }

    #[test]
    fn object_nodes() {
        let u = Uuid::parse_str(U).unwrap();
        let x = t("db:5/sch:2200");
        assert_eq!((x.server_id, x.database_oid, x.object, x.create), (u, Some(5), obj(ObjectKind::Schema, 2200), None));
        assert_eq!(t("db:5/sch:2200/sequences/seq:35").object, obj(ObjectKind::Sequence, 35));
        assert_eq!(t("db:5/sch:2200/seq:35").object, obj(ObjectKind::Sequence, 35));
        assert_eq!(t("db:5/extensions/ext:9").object, obj(ObjectKind::Extension, 9));
        assert_eq!(t("db:5/ext:9").object, obj(ObjectKind::Extension, 9));
        let i = t("db:5/sch:2200/tbl:24576/indexes/idx:41");
        assert_eq!((i.object, i.create, i.database_oid), (obj(ObjectKind::Index, 41), None, Some(5)));
        assert_eq!(t("db:5/sch:2200/tbl:24576/idx:41").object, obj(ObjectKind::Index, 41));
    }

    #[test]
    fn collections_create() {
        assert_eq!(t("db:5/schemas").create, cr(ObjectKind::Schema, None, None));
        assert_eq!(t("db:5/extensions").create, cr(ObjectKind::Extension, None, None));
        assert_eq!(t("db:5/sch:2200/sequences").create, cr(ObjectKind::Sequence, Some(2200), None));
        assert_eq!(t("db:5/sch:2200/tbl:24576/indexes").create, cr(ObjectKind::Index, Some(2200), Some(24576)));
        for p in ["db:5/schemas", "db:5/sch:2200/sequences", "db:5/sch:2200/tbl:24576/indexes"] {
            assert_eq!(t(p).object, None);
            assert_eq!(t(p).database_oid, Some(5));
        }
    }

    #[test]
    fn other_nodes_are_empty() {
        for p in [
            "",
            "dbs",
            "db:5",
            "roles",
            "role:10",
            "tablespaces",
            "ts:1663",
            "db:5/sch:2200/tables",
            "db:5/sch:2200/tbl:24576",
            "db:5/sch:2200/tbl:24576/columns",
            "db:5/sch:2200/tbl:24576/col:3",
            "db:5/sch:2200/tbl:24576/constraints/con:40",
            "db:5/sch:2200/view:30",
            "db:5/sch:2200/views",
            "db:5/sch:2200/fn:32",
            "db:5/sch:2200/tbl:24576/trg:42",
            // Yolun biçimi beklenenle uyuşmayan koleksiyonlar menü açmaz.
            "db:5/sequences",
            "sch:2200/schemas",
            "db:5/sch:2200/indexes",
        ] {
            let x = t(p);
            assert_eq!((x.object, x.create), (None, None), "{p}");
        }
        assert_eq!(t("").database_oid, None);
    }

    #[test]
    fn malformed_is_invalid() {
        for bad in ["", "grp:A", "x", "srv:nope", &format!("srv:{U}/wat"), &format!("srv:{U}/db:abc")] {
            assert!(matches!(target_of(bad), Err(Error::Invalid(_))), "{bad}");
        }
    }

    #[test]
    fn serializes_camel_case() {
        let v = serde_json::to_value(t("db:5/sch:2200/tbl:24576/indexes")).unwrap();
        assert_eq!(v["databaseOid"], 5);
        assert_eq!(v["create"]["kind"], "index");
        assert_eq!(v["create"]["context"]["tableOid"], 24576);
        assert!(v["object"].is_null());
    }
}
