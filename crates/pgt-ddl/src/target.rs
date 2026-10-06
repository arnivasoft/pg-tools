//! Ağaç düğüm kimliğinden diyalog hedefi: arayüz kimliği ayrıştırmaz (`qt_target_of` ilkesi); düzenle/sil için nesne,
//! "Yeni …" için koleksiyon bağlamı buradan gelir. Desteklenmeyen düğümler için ikisi de `None` (arayüz menü açmaz).
//!
//! Tablo düğümleri: `…/sch:S/tbl:T` (bölümler `…/tbl:P/tbl:C`, koleksiyon parçası atlanır), kolon `…/tbl:T/col:A`,
//! kısıt `…/tbl:T/con:K`. Kolon/kısıt düğümü tablonun kendisini hedefler ve `focus` ile diyaloğun açılacağı satırı
//! bildirir; görünüm/materialized view kolonları ve diğer her düğüm boştur.

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

/// Tablo diyaloğunun açılınca odaklanacağı satır: kolon (`attnum`) ya da kısıt (`pg_constraint.oid`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum DdlFocus {
    Column {
        #[ts(type = "number")]
        id: u32,
    },
    Constraint {
        #[ts(type = "number")]
        id: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DdlTarget {
    pub server_id: Uuid,
    #[ts(type = "number | null")]
    pub database_oid: Option<u32>,
    pub object: Option<ObjectRef>,
    pub create: Option<DdlCreate>,
    /// Yalnızca kolon/kısıt düğümünde: `object` (tablo) diyaloğunda odaklanılacak satır.
    pub focus: Option<DdlFocus>,
    /// Uygulama sonrası arayüzün yenileyeceği ağaç düğümü: `create` için koleksiyonun kendisi, `object` için üst
    /// koleksiyon (bölüm tablosunda üst tablonun `partitions` koleksiyonu); kolon/kısıt düğümünde tablonun kendisi.
    pub refresh_node_id: Option<String>,
}

/// `path[i]` ağacın gerçekten tablo olarak listelediği bir yerde mi: şema altında ya da (bölüm) tablo altında.
fn table_at(path: &[Seg], i: usize) -> Option<u32> {
    match (path.get(i), i.checked_sub(1).and_then(|p| path.get(p))) {
        (Some(Seg::Table(t)), Some(Seg::Schema(_) | Seg::Table(_))) => Some(*t),
        _ => None,
    }
}

/// Düğüm kimliğini çözer; bozuk kimlik ya da sunucuya ait olmayan (kök/grup) düğüm → `Error::Invalid`.
pub fn target_of(node_id: &str) -> Result<DdlTarget> {
    let id = NodeId::parse(node_id)?;
    let NodeId::Node { server, ref path } = id else {
        return Err(Error::Invalid(format!("düğüm bir sunucuya ait değil: {node_id}")));
    };
    let database_oid = path.iter().find_map(|s| if let Seg::Db(o) = s { Some(*o) } else { None });
    let mut t = DdlTarget { server_id: server, database_oid, object: None, create: None, focus: None, refresh_node_id: None };
    let obj = |kind, oid| Some(ObjectRef { kind, oid });
    let create = |kind, schema_oid, table_oid| Some(DdlCreate { kind, context: CreateContext { schema_oid, table_oid } });
    let last = path.len().checked_sub(1);
    let prev = path.len().checked_sub(2).map(|i| path[i]);
    // Üst tablo (kolon/kısıt düğümünün hemen üstündeki, ağaçta geçerli konumdaki tablo).
    let parent_table = path.len().checked_sub(2).and_then(|i| table_at(path, i));
    match path.last().copied() {
        Some(Seg::Schema(oid)) => t.object = obj(ObjectKind::Schema, oid),
        Some(Seg::Sequence(oid)) => t.object = obj(ObjectKind::Sequence, oid),
        Some(Seg::Extension(oid)) => t.object = obj(ObjectKind::Extension, oid),
        Some(Seg::Index(oid)) => t.object = obj(ObjectKind::Index, oid),
        Some(Seg::Table(_)) => {
            if let Some(oid) = last.and_then(|i| table_at(path, i)) {
                t.object = obj(ObjectKind::Table, oid);
            }
        }
        // Sistem kolonları (attnum ≤ 0) ağaçta listelenmez; yine de düzenlenemez.
        Some(Seg::Column(attnum)) if attnum > 0 => {
            if let Some(tbl) = parent_table {
                t.object = obj(ObjectKind::Table, tbl);
                t.focus = Some(DdlFocus::Column { id: attnum as u32 });
            }
        }
        Some(Seg::Constraint(oid)) => {
            if let Some(tbl) = parent_table {
                t.object = obj(ObjectKind::Table, tbl);
                t.focus = Some(DdlFocus::Constraint { id: oid });
            }
        }
        Some(Seg::Coll(Coll::Schemas)) if matches!(prev, Some(Seg::Db(_))) => t.create = create(ObjectKind::Schema, None, None),
        Some(Seg::Coll(Coll::Extensions)) if matches!(prev, Some(Seg::Db(_))) => t.create = create(ObjectKind::Extension, None, None),
        Some(Seg::Coll(Coll::Tables)) => {
            if let Some(Seg::Schema(s)) = prev {
                t.create = create(ObjectKind::Table, Some(s), None);
            }
        }
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
        t.focus = None;
    }
    // Yenilenecek düğüm: oluşturmada koleksiyonun kendisi; düzenle/silde nesnenin üst koleksiyonu (çocuk kimlikleri
    // koleksiyon parçasını atlar: `parent.child(seg(oid))`, koleksiyon kimliği `parent.child(Coll)`); kolon/kısıt
    // düğümünde tablonun kendisi (alt koleksiyonları yenilensin).
    if t.create.is_some() {
        t.refresh_node_id = Some(id.to_string());
    } else if t.focus.is_some() {
        t.refresh_node_id = Some(id.parent().to_string());
    } else if let Some(o) = t.object {
        let coll = match o.kind {
            ObjectKind::Schema => Coll::Schemas,
            ObjectKind::Sequence => Coll::Sequences,
            ObjectKind::Extension => Coll::Extensions,
            ObjectKind::Index => Coll::Indexes,
            // Bölüm tablosu üst tablonun `partitions` koleksiyonunda listelenir.
            ObjectKind::Table if matches!(prev, Some(Seg::Table(_))) => Coll::Partitions,
            ObjectKind::Table => Coll::Tables,
        };
        t.refresh_node_id = Some(id.parent().child(Seg::Coll(coll)).to_string());
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
        assert_eq!(t("db:5/sch:2200/seq:35").object, obj(ObjectKind::Sequence, 35));
        assert_eq!(t("db:5/ext:9").object, obj(ObjectKind::Extension, 9));
        let i = t("db:5/sch:2200/tbl:24576/idx:41");
        assert_eq!((i.object, i.create, i.database_oid), (obj(ObjectKind::Index, 41), None, Some(5)));
        for p in ["db:5/sch:2200", "db:5/sch:2200/seq:35", "db:5/ext:9", "db:5/sch:2200/tbl:24576/idx:41"] {
            assert_eq!(t(p).focus, None, "{p}");
        }
    }

    #[test]
    fn table_nodes() {
        let x = t("db:5/sch:2200/tbl:24576");
        assert_eq!((x.object, x.create, x.focus, x.database_oid), (obj(ObjectKind::Table, 24576), None, None, Some(5)));
        // Bölüm tablosu da bir tablodur (yapı değişikliğini yükleyicinin `isPartition` bayrağı engeller).
        let p = t("db:5/sch:2200/tbl:24576/tbl:24580");
        assert_eq!((p.object, p.focus), (obj(ObjectKind::Table, 24580), None));
        assert_eq!(t("db:5/sch:2200/tbl:1/tbl:2/tbl:3").object, obj(ObjectKind::Table, 3));
    }

    #[test]
    fn column_and_constraint_nodes_focus_the_table() {
        let c = t("db:5/sch:2200/tbl:24576/col:3");
        assert_eq!((c.object, c.create, c.focus), (obj(ObjectKind::Table, 24576), None, Some(DdlFocus::Column { id: 3 })));
        let k = t("db:5/sch:2200/tbl:24576/con:40");
        assert_eq!((k.object, k.create, k.focus), (obj(ObjectKind::Table, 24576), None, Some(DdlFocus::Constraint { id: 40 })));
        // Bölüm tablosunun kolonu/kısıtı: bölümün kendisi.
        let pc = t("db:5/sch:2200/tbl:24576/tbl:24580/col:1");
        assert_eq!((pc.object, pc.focus), (obj(ObjectKind::Table, 24580), Some(DdlFocus::Column { id: 1 })));
        let pk = t("db:5/sch:2200/tbl:24576/tbl:24580/con:7");
        assert_eq!((pk.object, pk.focus), (obj(ObjectKind::Table, 24580), Some(DdlFocus::Constraint { id: 7 })));
        assert_eq!(t("db:5/sch:2200/tbl:1/col:32767").focus, Some(DdlFocus::Column { id: 32767 }));
    }

    #[test]
    fn refresh_node_is_the_collection() {
        // Düzenle/sil: üst koleksiyon; oluştur: koleksiyonun kendisi; kolon/kısıt: tablo; diğerleri: yok.
        let r = |p: &str| t(p).refresh_node_id;
        let s = |p: &str| Some(format!("srv:{U}/{p}"));
        assert_eq!(r("db:5/sch:2200"), s("db:5/schemas"));
        assert_eq!(r("db:5/sch:2200/seq:35"), s("db:5/sch:2200/sequences"));
        assert_eq!(r("db:5/ext:9"), s("db:5/extensions"));
        assert_eq!(r("db:5/sch:2200/tbl:24576/idx:41"), s("db:5/sch:2200/tbl:24576/indexes"));
        assert_eq!(r("db:5/sch:2200/tbl:24576"), s("db:5/sch:2200/tables"));
        assert_eq!(r("db:5/sch:2200/tbl:24576/tbl:24580"), s("db:5/sch:2200/tbl:24576/partitions"));
        assert_eq!(r("db:5/sch:2200/tbl:24576/col:3"), s("db:5/sch:2200/tbl:24576"));
        assert_eq!(r("db:5/sch:2200/tbl:24576/con:40"), s("db:5/sch:2200/tbl:24576"));
        assert_eq!(r("db:5/sch:2200/tbl:24576/tbl:24580/con:7"), s("db:5/sch:2200/tbl:24576/tbl:24580"));
        assert_eq!(r("db:5/schemas"), s("db:5/schemas"));
        assert_eq!(r("db:5/sch:2200/sequences"), s("db:5/sch:2200/sequences"));
        assert_eq!(r("db:5/sch:2200/tables"), s("db:5/sch:2200/tables"));
        assert_eq!(r("db:5/extensions"), s("db:5/extensions"));
        assert_eq!(r("db:5/sch:2200/tbl:24576/indexes"), s("db:5/sch:2200/tbl:24576/indexes"));
        for p in ["", "db:5", "role:10", "db:5/sch:2200/tbl:24576/columns", "db:5/sch:2200/tbl:24576/partitions"] {
            assert_eq!(r(p), None, "{p}");
        }
        // Üretilen kimlik gerçekten ayrıştırılıp aynen geri yazılabilir.
        for p in ["db:5/sch:2200/seq:35", "db:5/sch:2200/tbl:24576/tbl:24580", "db:5/sch:2200/tbl:24576/col:3"] {
            let id = r(p).unwrap();
            assert_eq!(NodeId::parse(&id).unwrap().to_string(), id);
        }
    }

    #[test]
    fn collections_create() {
        assert_eq!(t("db:5/schemas").create, cr(ObjectKind::Schema, None, None));
        assert_eq!(t("db:5/extensions").create, cr(ObjectKind::Extension, None, None));
        assert_eq!(t("db:5/sch:2200/sequences").create, cr(ObjectKind::Sequence, Some(2200), None));
        assert_eq!(t("db:5/sch:2200/tables").create, cr(ObjectKind::Table, Some(2200), None));
        assert_eq!(t("db:5/sch:2200/tbl:24576/indexes").create, cr(ObjectKind::Index, Some(2200), Some(24576)));
        for p in ["db:5/schemas", "db:5/sch:2200/sequences", "db:5/sch:2200/tables", "db:5/sch:2200/tbl:24576/indexes"] {
            assert_eq!((t(p).object, t(p).focus), (None, None), "{p}");
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
            "db:5/sch:2200/tbl:24576/columns",
            "db:5/sch:2200/tbl:24576/constraints",
            "db:5/sch:2200/tbl:24576/partitions",
            "db:5/sch:2200/tbl:24576/triggers",
            "db:5/sch:2200/tbl:24576/policies",
            "db:5/sch:2200/tbl:24576/trg:42",
            "db:5/sch:2200/tbl:24576/pol:43",
            // Görünüm / materialized view kolonları tablo diyaloğu açmaz.
            "db:5/sch:2200/view:30",
            "db:5/sch:2200/view:30/columns",
            "db:5/sch:2200/view:30/col:1",
            "db:5/sch:2200/mv:31/col:1",
            "db:5/sch:2200/views",
            "db:5/sch:2200/fn:32",
            // Sistem kolonları (attnum ≤ 0).
            "db:5/sch:2200/tbl:24576/col:0",
            "db:5/sch:2200/tbl:24576/col:-1",
            // Yolun biçimi ağacın ürettiğiyle uyuşmayanlar menü açmaz.
            "db:5/sch:2200/tbl:24576/constraints/con:40",
            "db:5/sch:2200/tbl:24576/columns/col:3",
            "db:5/sch:2200/tables/tbl:24576",
            "db:5/tbl:24576",
            "db:5/tbl:24576/col:1",
            "db:5/sch:2200/view:30/con:4",
            "db:5/sch:2200/view:30/tbl:4",
            "db:5/con:4",
            "db:5/col:1",
            "db:5/tables",
            "db:5/sch:2200/tbl:24576/tables",
            "db:5/sequences",
            "sch:2200/schemas",
            "sch:2200/tables",
            "sch:2200/tbl:1",
            "sch:2200/tbl:1/col:1",
            "db:5/sch:2200/indexes",
        ] {
            let x = t(p);
            assert_eq!((x.object, x.create, x.focus, x.refresh_node_id), (None, None, None, None), "{p}");
        }
        assert_eq!(t("").database_oid, None);
    }

    #[test]
    fn malformed_is_invalid() {
        for bad in [
            "",
            "grp:A",
            "x",
            "srv:nope",
            &format!("srv:{U}/wat"),
            &format!("srv:{U}/db:abc"),
            &format!("srv:{U}/db:5/sch:2200/tbl:x"),
            &format!("srv:{U}/db:5/sch:2200/tbl:-1"),
            &format!("srv:{U}/db:5/sch:2200/tbl:1/col:99999"),
            &format!("srv:{U}/db:5/sch:2200/tbl:1/col:x"),
            &format!("srv:{U}/db:5/sch:2200/tbl:1/con:-4"),
            &format!("srv:{U}/db:5/sch:2200/tbl:1/con:"),
            &format!("srv:{U}/db:5/sch:2200/tbl:1/"),
        ] {
            assert!(matches!(target_of(bad), Err(Error::Invalid(_))), "{bad}");
        }
    }

    #[test]
    fn serializes_camel_case() {
        let v = serde_json::to_value(t("db:5/sch:2200/tbl:24576/indexes")).unwrap();
        assert_eq!(v["databaseOid"], 5);
        assert_eq!(v["refreshNodeId"], format!("srv:{U}/db:5/sch:2200/tbl:24576/indexes"));
        assert_eq!(v["create"]["kind"], "index");
        assert_eq!(v["create"]["context"]["tableOid"], 24576);
        assert!(v["object"].is_null() && v["focus"].is_null());
        let v = serde_json::to_value(t("db:5/sch:2200/tbl:24576/col:3")).unwrap();
        assert_eq!(v["object"], serde_json::json!({"kind": "table", "oid": 24576}));
        assert_eq!(v["focus"], serde_json::json!({"kind": "column", "id": 3}));
        let v = serde_json::to_value(t("db:5/sch:2200/tbl:24576/con:40")).unwrap();
        assert_eq!(v["focus"], serde_json::json!({"kind": "constraint", "id": 40}));
        let v = serde_json::to_value(t("db:5/sch:2200/tables")).unwrap();
        assert_eq!(v["create"], serde_json::json!({"kind": "table", "context": {"schemaOid": 2200, "tableOid": null}}));
    }
}
