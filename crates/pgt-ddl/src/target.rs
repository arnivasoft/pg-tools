//! Ağaç düğüm kimliğinden diyalog hedefi: arayüz kimliği ayrıştırmaz (`qt_target_of` ilkesi); düzenle/sil için nesne,
//! "Yeni …" için koleksiyon bağlamı buradan gelir. Desteklenmeyen düğümler için ikisi de `None` (arayüz menü açmaz).
//!
//! Tablo düğümleri: `…/sch:S/tbl:T` (bölümler `…/tbl:P/tbl:C`, koleksiyon parçası atlanır), kolon `…/tbl:T/col:A`,
//! kısıt `…/tbl:T/con:K`. Kolon/kısıt düğümü tablonun kendisini hedefler ve `focus` ile diyaloğun açılacağı satırı
//! bildirir. Görünüm `…/sch:S/view:V`, materialized view `…/sch:S/mv:M` (koleksiyonları `views`/`matviews`).
//! Sunucu kapsamlı: veritabanı `db:N` (koleksiyon `dbs`), rol `role:N` (koleksiyon `roles`) — `databaseOid` `null`.
//! Fonksiyon `…/sch:S/fn:F`, procedure `proc:P`, trigger fonksiyonu `tfn:F` (Function), tip `typ:T`, domain `dom:D`
//! (koleksiyonları `functions`/`procedures`/`trigfuncs`/`types`/`domains`; `trigfuncs`'tan oluşturma `preset` taşır);
//! trigger `…/tbl:T/trg:G` (bölüm tablosu dahil; koleksiyonu `triggers`, oluşturmada `tableOid = T`).
//! Görünüm/materialized view kolonları ve diğer her düğüm boştur.

use pgt_catalog::node::{Coll, NodeId, Seg};
use pgt_core::{Error, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{CreateContext, ObjectKind, ObjectRef};

/// "Yeni …" diyaloğunun ön ayarı (koleksiyondan gelir).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CreatePreset {
    /// "Trigger Functions" koleksiyonu: dönüşü `trigger` olan Function.
    TriggerFunction,
}

/// "Yeni …" diyaloğunun türü ve bağlamı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DdlCreate {
    pub kind: ObjectKind,
    pub context: CreateContext,
    /// Yalnızca `trigfuncs` koleksiyonunda dolu.
    pub preset: Option<CreatePreset>,
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
    let create = |kind, schema_oid, table_oid| Some(DdlCreate { kind, context: CreateContext { schema_oid, table_oid }, preset: None });
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
        Some(Seg::View(oid)) if matches!(prev, Some(Seg::Schema(_))) => t.object = obj(ObjectKind::View, oid),
        Some(Seg::MatView(oid)) if matches!(prev, Some(Seg::Schema(_))) => t.object = obj(ObjectKind::MaterializedView, oid),
        Some(Seg::Coll(Coll::Views)) => {
            if let Some(Seg::Schema(s)) = prev {
                t.create = create(ObjectKind::View, Some(s), None);
            }
        }
        Some(Seg::Coll(Coll::MatViews)) => {
            if let Some(Seg::Schema(s)) = prev {
                t.create = create(ObjectKind::MaterializedView, Some(s), None);
            }
        }
        // Şema altındaki fonksiyon/procedure/trigger fonksiyonu/tip/domain.
        Some(Seg::Function(oid) | Seg::TriggerFunction(oid)) if matches!(prev, Some(Seg::Schema(_))) => {
            t.object = obj(ObjectKind::Function, oid)
        }
        Some(Seg::Procedure(oid)) if matches!(prev, Some(Seg::Schema(_))) => t.object = obj(ObjectKind::Procedure, oid),
        Some(Seg::Type(oid)) if matches!(prev, Some(Seg::Schema(_))) => t.object = obj(ObjectKind::Type, oid),
        Some(Seg::Domain(oid)) if matches!(prev, Some(Seg::Schema(_))) => t.object = obj(ObjectKind::Domain, oid),
        Some(Seg::Coll(c @ (Coll::Functions | Coll::Procedures | Coll::TriggerFunctions | Coll::Types | Coll::Domains))) => {
            if let Some(Seg::Schema(s)) = prev {
                let kind = match c {
                    Coll::Procedures => ObjectKind::Procedure,
                    Coll::Types => ObjectKind::Type,
                    Coll::Domains => ObjectKind::Domain,
                    _ => ObjectKind::Function,
                };
                t.create = create(kind, Some(s), None);
                if c == Coll::TriggerFunctions
                    && let Some(cr) = t.create.as_mut()
                {
                    cr.preset = Some(CreatePreset::TriggerFunction);
                }
            }
        }
        // Trigger: ağaçta geçerli konumdaki (bölüm) tablonun altında.
        Some(Seg::Trigger(oid)) => {
            if parent_table.is_some() {
                t.object = obj(ObjectKind::Trigger, oid);
            }
        }
        Some(Seg::Coll(Coll::Triggers)) => {
            if let Some(tbl) = parent_table
                && let Some(s) = path.iter().rev().find_map(|s| if let Seg::Schema(o) = s { Some(*o) } else { None })
            {
                t.create = create(ObjectKind::Trigger, Some(s), Some(tbl));
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
    // Sunucu kapsamlı türler (Database/Role): yalnızca sunucunun hemen altındaki düğüm (`dbs`, `db:N`, `roles`,
    // `role:N`); bakım veritabanında çalışırlar → `database_oid` her zaman `None` (yol `Db` içerse bile).
    if path.len() == 1 {
        // Tek parçalı yolda veritabanı kapsamlı bir hedef olamaz (ör. `sch:2200`, `tables`).
        (t.object, t.create, t.focus) = (None, None, None);
        match path[0] {
            Seg::Coll(Coll::Databases) => t.create = create(ObjectKind::Database, None, None),
            Seg::Coll(Coll::Roles) => t.create = create(ObjectKind::Role, None, None),
            Seg::Db(oid) => t.object = obj(ObjectKind::Database, oid),
            Seg::Role(oid) => t.object = obj(ObjectKind::Role, oid),
            _ => {}
        }
        if t.object.is_some() || t.create.is_some() {
            t.database_oid = None;
        }
    } else if t.database_oid.is_none() {
        // Veritabanı dışındaki düğümde veritabanı kapsamlı nesne/oluşturma anlamsızdır.
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
            ObjectKind::View => Coll::Views,
            ObjectKind::MaterializedView => Coll::MatViews,
            ObjectKind::Database => Coll::Databases,
            ObjectKind::Role => Coll::Roles,
            // Trigger fonksiyonu kendi koleksiyonunda listelenir.
            ObjectKind::Function if matches!(path.last(), Some(Seg::TriggerFunction(_))) => Coll::TriggerFunctions,
            ObjectKind::Function => Coll::Functions,
            ObjectKind::Procedure => Coll::Procedures,
            ObjectKind::Trigger => Coll::Triggers,
            ObjectKind::Type => Coll::Types,
            ObjectKind::Domain => Coll::Domains,
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
        Some(DdlCreate { kind, context: CreateContext { schema_oid, table_oid }, preset: None })
    }

    #[test]
    fn routine_type_domain_and_trigger_nodes() {
        // (düğüm, nesne, oluşturma, yenilenecek düğüm)
        let s = |p: &str| Some(format!("srv:{U}/{p}"));
        let trig_fn = Some(DdlCreate {
            kind: ObjectKind::Function,
            context: CreateContext { schema_oid: Some(2200), table_oid: None },
            preset: Some(CreatePreset::TriggerFunction),
        });
        let cases = [
            ("db:5/sch:2200/fn:32", obj(ObjectKind::Function, 32), None, s("db:5/sch:2200/functions")),
            ("db:5/sch:2200/proc:33", obj(ObjectKind::Procedure, 33), None, s("db:5/sch:2200/procedures")),
            // Trigger fonksiyonu bir Function'dır; kendi koleksiyonunda yenilenir.
            ("db:5/sch:2200/tfn:34", obj(ObjectKind::Function, 34), None, s("db:5/sch:2200/trigfuncs")),
            ("db:5/sch:2200/typ:35", obj(ObjectKind::Type, 35), None, s("db:5/sch:2200/types")),
            ("db:5/sch:2200/dom:36", obj(ObjectKind::Domain, 36), None, s("db:5/sch:2200/domains")),
            ("db:5/sch:2200/tbl:24576/trg:42", obj(ObjectKind::Trigger, 42), None, s("db:5/sch:2200/tbl:24576/triggers")),
            // Bölüm tablosunun trigger'ı (klonlanmışsa yükleyici salt okunur döner).
            (
                "db:5/sch:2200/tbl:24576/tbl:24580/trg:43",
                obj(ObjectKind::Trigger, 43),
                None,
                s("db:5/sch:2200/tbl:24576/tbl:24580/triggers"),
            ),
            ("db:5/sch:2200/functions", None, cr(ObjectKind::Function, Some(2200), None), s("db:5/sch:2200/functions")),
            ("db:5/sch:2200/procedures", None, cr(ObjectKind::Procedure, Some(2200), None), s("db:5/sch:2200/procedures")),
            ("db:5/sch:2200/trigfuncs", None, trig_fn, s("db:5/sch:2200/trigfuncs")),
            ("db:5/sch:2200/types", None, cr(ObjectKind::Type, Some(2200), None), s("db:5/sch:2200/types")),
            ("db:5/sch:2200/domains", None, cr(ObjectKind::Domain, Some(2200), None), s("db:5/sch:2200/domains")),
            ("db:5/sch:2200/tbl:24576/triggers", None, cr(ObjectKind::Trigger, Some(2200), Some(24576)), s("db:5/sch:2200/tbl:24576/triggers")),
            (
                "db:5/sch:2200/tbl:24576/tbl:24580/triggers",
                None,
                cr(ObjectKind::Trigger, Some(2200), Some(24580)),
                s("db:5/sch:2200/tbl:24576/tbl:24580/triggers"),
            ),
        ];
        for (p, object, create, refresh) in cases {
            let x = t(p);
            assert_eq!((x.object, x.create, x.focus, x.database_oid), (object, create, None, Some(5)), "{p}");
            assert_eq!(x.refresh_node_id, refresh, "{p}");
            let id = x.refresh_node_id.unwrap();
            assert_eq!(NodeId::parse(&id).unwrap().to_string(), id, "{p}");
        }
    }

    #[test]
    fn create_preset_serializes_snake_case() {
        let v = serde_json::to_value(t("db:5/sch:2200/trigfuncs")).unwrap();
        assert_eq!(
            v["create"],
            serde_json::json!({"kind": "function", "context": {"schemaOid": 2200, "tableOid": null}, "preset": "trigger_function"})
        );
        let v = serde_json::to_value(t("db:5/sch:2200/functions")).unwrap();
        assert!(v["create"]["preset"].is_null());
        let v = serde_json::to_value(t("db:5/sch:2200/tbl:24576/triggers")).unwrap();
        assert_eq!(v["create"], serde_json::json!({"kind": "trigger", "context": {"schemaOid": 2200, "tableOid": 24576}, "preset": null}));
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
        assert_eq!(r("dbs"), s("dbs"));
        assert_eq!(r("db:5"), s("dbs"));
        assert_eq!(r("roles"), s("roles"));
        assert_eq!(r("role:10"), s("roles"));
        assert_eq!(r("db:5/sch:2200/views"), s("db:5/sch:2200/views"));
        assert_eq!(r("db:5/sch:2200/view:30"), s("db:5/sch:2200/views"));
        assert_eq!(r("db:5/sch:2200/matviews"), s("db:5/sch:2200/matviews"));
        assert_eq!(r("db:5/sch:2200/mv:31"), s("db:5/sch:2200/matviews"));
        for p in ["", "db:5/sch:2200/tbl:24576/columns", "db:5/sch:2200/tbl:24576/partitions", "tablespaces", "ts:1663"] {
            assert_eq!(r(p), None, "{p}");
        }
        // Üretilen kimlik gerçekten ayrıştırılıp aynen geri yazılabilir.
        for p in ["db:5/sch:2200/seq:35", "db:5/sch:2200/tbl:24576/tbl:24580", "db:5/sch:2200/tbl:24576/col:3", "db:5", "role:10", "db:5/sch:2200/mv:31"] {
            let id = r(p).unwrap();
            assert_eq!(NodeId::parse(&id).unwrap().to_string(), id);
        }
    }

    #[test]
    fn view_database_and_role_nodes() {
        // (düğüm, nesne, oluşturma, databaseOid)
        let cases = [
            ("dbs", None, cr(ObjectKind::Database, None, None), None),
            ("db:5", obj(ObjectKind::Database, 5), None, None),
            ("roles", None, cr(ObjectKind::Role, None, None), None),
            ("role:10", obj(ObjectKind::Role, 10), None, None),
            ("db:5/sch:2200/views", None, cr(ObjectKind::View, Some(2200), None), Some(5)),
            ("db:5/sch:2200/view:30", obj(ObjectKind::View, 30), None, Some(5)),
            ("db:5/sch:2200/matviews", None, cr(ObjectKind::MaterializedView, Some(2200), None), Some(5)),
            ("db:5/sch:2200/mv:31", obj(ObjectKind::MaterializedView, 31), None, Some(5)),
        ];
        for (p, object, create, db) in cases {
            let x = t(p);
            assert_eq!((x.object, x.create, x.database_oid, x.focus), (object, create, db, None), "{p}");
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
            "tablespaces",
            "ts:1663",
            "db:5/sch:2200/tbl:24576/columns",
            "db:5/sch:2200/tbl:24576/constraints",
            "db:5/sch:2200/tbl:24576/partitions",
            "db:5/sch:2200/tbl:24576/policies",
            "db:5/sch:2200/tbl:24576/pol:43",
            // Görünüm / materialized view kolonları tablo diyaloğu açmaz.
            "db:5/sch:2200/view:30/columns",
            "db:5/sch:2200/view:30/col:1",
            "db:5/sch:2200/mv:31/col:1",
            "db:5/sch:2200/mv:31/columns",
            // Fonksiyon/tip/domain yalnızca şema altında, trigger yalnızca (bölüm) tablo altında.
            "db:5/fn:32",
            "db:5/proc:32",
            "db:5/tfn:32",
            "db:5/typ:32",
            "db:5/dom:32",
            "db:5/trg:42",
            "db:5/functions",
            "db:5/procedures",
            "db:5/trigfuncs",
            "db:5/types",
            "db:5/domains",
            "db:5/triggers",
            "db:5/sch:2200/triggers",
            "db:5/sch:2200/trg:42",
            "db:5/sch:2200/view:30/triggers",
            "db:5/sch:2200/view:30/trg:42",
            "db:5/sch:2200/mv:31/trg:42",
            "db:5/sch:2200/tbl:24576/functions",
            "db:5/sch:2200/tbl:24576/fn:32",
            "db:5/sch:2200/tbl:24576/typ:32",
            "db:5/sch:2200/fn:32/fn:33",
            "db:5/sch:2200/tbl:24576/triggers/trg:42",
            "db:5/sch:2200/tables/tbl:24576/trg:42",
            "fn:32",
            "functions",
            "sch:2200/fn:32",
            "sch:2200/functions",
            "sch:2200/tbl:1/trg:2",
            "sch:2200/tbl:1/triggers",
            // Sunucu kapsamlı düğümler yalnızca sunucunun hemen altında; görünümler yalnızca şema altında.
            "db:5/roles",
            "db:5/role:10",
            "db:5/dbs",
            "db:5/db:6",
            "db:5/views",
            "db:5/view:30",
            "db:5/matviews",
            "db:5/mv:31",
            "sch:2200",
            "seq:35",
            "ext:9",
            "tables",
            "views",
            "view:30",
            "sch:2200/views",
            "sch:2200/view:30",
            "sch:2200/mv:31",
            "db:5/sch:2200/tbl:1/views",
            "db:5/sch:2200/tbl:1/view:30",
            "db:5/sch:2200/view:30/view:31",
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
        for p in ["db:5", "dbs", "roles", "role:10"] {
            let v = serde_json::to_value(t(p)).unwrap();
            assert!(v["databaseOid"].is_null(), "{p}");
        }
        let v = serde_json::to_value(t("db:5")).unwrap();
        assert_eq!(v["object"], serde_json::json!({"kind": "database", "oid": 5}));
        let v = serde_json::to_value(t("roles")).unwrap();
        assert_eq!(v["create"], serde_json::json!({"kind": "role", "context": {"schemaOid": null, "tableOid": null}, "preset": null}));
        let v = serde_json::to_value(t("db:5/sch:2200/matviews")).unwrap();
        assert_eq!(v["create"]["kind"], "materialized_view");
        assert_eq!(v["databaseOid"], 5);
        let v = serde_json::to_value(t("db:5/sch:2200/tables")).unwrap();
        assert_eq!(v["create"], serde_json::json!({"kind": "table", "context": {"schemaOid": 2200, "tableOid": null}, "preset": null}));
    }
}
