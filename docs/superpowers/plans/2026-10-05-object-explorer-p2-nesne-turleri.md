# Object Explorer — Plan 2/3: Nesne Türleri, Tam DDL ve Doğrulama Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Çekirdek nesne setinin tamamını (constraint, index, trigger, RLS policy, partition, view, materialized view, function, procedure, trigger function, sequence, type, domain, extension, role, tablespace) ağaçta listelemek, her biri için Properties ve DDL üretmek; Table DDL'ini eksiksiz hale getirmek; DDL'in doğruluğunu gidiş-dönüş testiyle, ölçek hedeflerini performans testiyle, PG 10–18 uyumunu Docker sürüm matrisiyle kanıtlamak; Plan 1'den kalan iki işi (bağlantı-koptu göstergesi, bellek hedefi) kapatmak.

**Architecture:** Plan 1'deki `pgt-catalog` dağıtıcısı yol-parçası kalıbı eşleştirmesinden **son parça + üst bağlam** eşleştirmesine geçirilir. Yeni nesne türleri Rust kodu yerine **veri sözleşmeli SQL dosyalarıyla** tanımlanır: her tür bir `list_*.sql`, `props_*.sql`, `ddl_*.sql` üçlüsü ve `registry.rs` içinde bir kayıttır; genel motor (`kinds/engine.rs`) sonucu `TreeNode` / `PropertySection` / DDL metnine çevirir. Plan 1'in elle yazılmış Database/Schema/Table/Column modülleri yerinde kalır; yalnızca Table DDL'i genişler.

**Tech Stack:** Plan 1 ile aynı (Rust 2024, Tauri 2.12, tokio-postgres, ts-rs, insta; bun, SolidJS, Vitest). Ek: Docker (yalnızca Task 9), PowerShell betikleri.

**Spec:** `docs/superpowers/specs/2026-10-05-object-explorer-design.md` (§3 veri modeli, §5 katalog/DDL, §8 test stratejisi, §1.3 başarı ölçütleri)

**Önceki plan:** `docs/superpowers/plans/2026-10-05-object-explorer-p1-cekirdek.md` — tamamlandı ve `main`'e birleşti. Bu plan onun kodunu temel alır; aşağıdaki "Plan 1 sonrası kod gerçekleri" bölümü planın dayandığı mevcut yapıyı özetler.

## Plan 1 sonrası kod gerçekleri (bu planın dayandığı yapı)

- Düğüm kimliği: `NodeId::Node { server, path: Vec<Seg> }`; metin biçimi `srv:<uuid>/db:<oid>/sch:<oid>/tbl:<oid>/col:<attnum>`. **Koleksiyon düğümlerinin kimliği** koleksiyon parçasını taşır (`.../tables`, `.../columns`), ama **koleksiyonun çocuklarının kimliği koleksiyon parçasını atlar** (`.../sch:2200/tbl:24576`, `.../tbl:24576/col:3`). Bu kural yeni türlerde de korunur: `.../tbl:T/idx:I`, `.../tbl:T/tbl:P` (partition), `.../db:D/ext:E`, `srv:U/role:R`.
- `crates/pgt-catalog/src/kinds/mod.rs`: `list_children`, `properties`, `ddl` dağıtıcıları ve `acl()` yardımcısı. `kinds/{server,database,schema,table,column}.rs`: elle yazılmış tür modülleri. SQL dosyaları `crates/pgt-catalog/src/sql/`, `VersionedSql(&[(min_ana_sürüm, sql)])` ile sürüme göre seçilir (`.pg10.sql` soneki PG 10–11/12 varyantları).
- `crates/pgt-catalog/src/ddl.rs`: `header`, `owner`, `comment`, `grants`, `column_line`, `join_blocks`, `AclEntry`, `ColumnDef`. `db.rs`: `query`, `query_one`, `p![...]`.
- Test altyapısı: `crates/pgt-catalog/tests/common/mod.rs` (`fixtures()`, `Fixture` ile `schema()`, `table()`, `normalize()`, `major()`), `tests/fixtures/schema.sql`, `insta` snapshot'ları `tests/snapshots/*@pg18.snap`. Entegrasyon testleri `PGT_TEST_URLS` yoksa atlanır.
- Ön yüz: `src/features/explorer/{tree-model,controller,explorer-store,TreeView,TreeRow,NodeIcon}`, `src/features/inspector/{Inspector,labels,for-node,...}`; `labels.ts` `Record<NodeKind, string>` olduğu için yeni `NodeKind` eklenince derleme hatası verir (kasıtlı).
- Tauri komutları (`list_children`, `get_properties`, `get_ddl`, …) değişmez; yeni türler yalnızca katalog katmanında eklenir.

## Global Constraints

- Hedef istemci OS: Windows 10/11, Windows Server 2019/2022. PostgreSQL 10–18, yalnızca vanilla. Sürüme özel SQL `.pg10.sql` (10–11 veya 10–12) / varsayılan (üst sürümler) olarak ayrılır; `VersionedSql` dilimi artan ana sürümle sıralanır.
- Her PostgreSQL oturumu salt okunur (`default_transaction_read_only = on`); katalog sorguları yalnızca `include_str!` ile gömülü sabit SQL'dir, değerler `$n` parametresiyle bağlanır; arayüze rastgele SQL açılmaz.
- Tırnaklama sunucuda yapılır (`quote_ident`, `quote_literal`, `format('%I')`, `%L`, `format_type`); Rust'ta rezerve kelime listesi yoktur.
- Nesneler OID ile tanımlanır; ön yüz kimlik string'ini ayrıştırmaz.
- Sistem şemaları (`pg_catalog`, `information_schema`, `pg_toast*`, `pg_temp_*`) varsayılan olarak gizlidir; `pg_` ile başlayan yerleşik roller ve tablespace'ler (`pg_default`, `pg_global`) aynı `show_system` ayarına bağlıdır.
- Liste sorguları koleksiyon başına **tek sorgu** (N+1 yok), yalnızca ağaçta gösterilecek kolonları çeker; boyut/istatistik hesapları yalnızca Properties'te ve yalnızca seçimde yapılır.
- Extension'a ait nesneler normal listelerde gösterilir ve `flags.fromExtension` ile işaretlenir.
- DDL çıktı biçimi Plan 1'dekiyle aynıdır: başlık yorumu (`-- Index: app.customers_lower_name_idx`), yorum satırı `-- DROP …;`, bloklar arasında bir boş satır, dosya `\n` ile biter. Rol DDL'inde **parola asla yer almaz**.
- Parola hiçbir dosyaya ve log'a yazılmaz. Test kimlik bilgileri (`PGT_TEST_URLS`) hiçbir rapora, commit'e veya log'a yazılmaz.
- Arayüz metinleri `src/app/strings.ts` içindedir (Türkçe komutlar, İngilizce PostgreSQL terimleri). Koleksiyon etiketleri aynen: `Constraints`, `Indexes`, `Triggers`, `RLS Policies`, `Partitions`, `Views`, `Materialized Views`, `Functions`, `Procedures`, `Trigger Functions`, `Sequences`, `Types`, `Domains`, `Extensions`, `Login/Group Roles`, `Tablespaces`.
- Paket yöneticisi bun (`~/.bun/bin/bun`, PATH'te olmayabilir). TLS kripto sağlayıcısı ring; `aws-lc-rs` açılmaz.
- Entegrasyon testleri için test sunucusunun kullanıcısı `CREATEDB` + `CREATEROLE` (veya superuser) olmalı; scram-sha-256 kimlik doğrulaması.

### Bu planın bilinçli sınırları

- Kapsam dışı (Plan 3): pgAdmin içe aktarma, filtre, sağ tık menüsü, kopyalama eylemleri, sunucu silme/bağlantıyı kesme, toast/NotFound akışı, durum çubuğu uyarıları.
- Kapsam dışı (sonraki spec'ler): Aggregates, Foreign Tables, FDW, Publications/Subscriptions, Event Triggers, Casts, Languages, Collations, FTS, Operators, Rules.
- Bilinen sınırlar (DDL): base type DDL'i desteklenmez (açıklayıcı yorum satırı üretilir); range type yalnızca `SUBTYPE` ile üretilir; PG 18 adlandırılmış NOT NULL kısıtlarının adı DDL'de korunmaz (kolon satırında `NOT NULL`); kolon başına `STORAGE`/`COMPRESSION` yazılmaz; `GRANT` satırlarında veren (grantor) kullanılmaz.

## Review Focus

1. **Aşırı yüklenmiş (overloaded) fonksiyonlar** (`app.fmt(integer)` ve `app.fmt(text)`): ağaçta kimlik argümanlarıyla ayrışmalı, Properties/DDL doğru olanı getirmeli (OID ile). Test: Task 2 `functions_are_listed_with_identity_arguments`, Task 4 `overloaded_functions_resolve_by_oid`.
2. **Tırnak gerektiren adlar** (`app."Mixed View"`, `"Çok Kolon"`, `"Order Items"` üzerindeki nesneler): ağaçta ham ad, DDL'de doğru tırnaklı ad görünmeli ve DDL çalıştırılabilmeli. Test: Task 4 `quoted_names_survive_in_ddl`, Task 7 gidiş-dönüş.
3. **Listelendikten sonra silinen nesne:** `get_properties`/`get_ddl` yok olan OID için çökmemeli, `NotFound` dönmeli. Test: Task 3 `missing_oid_is_not_found_for_every_kind`.
4. **Boş koleksiyonlar** (ör. fonksiyonu olmayan şema, tetikleyicisi olmayan tablo) açılınca boş liste dönmeli, hata değil. Test: Task 2 `empty_collections_expand_to_nothing`.
5. **PG 10'da olmayan özellikler:** `Procedures` koleksiyonu PG 10'da gösterilmemeli; partition'ı olmayan (düz) tabloda `Partitions` koleksiyonu gösterilmemeli. Test: Task 2 `schema_collections_depend_on_version`, `partitions_collection_only_for_partitioned_tables`.
6. **Extension'ın kendi nesneleri** (`pg_trgm` fonksiyonları): listelerde görünür ama `fromExtension` ile işaretli; gidiş-dönüş testinde yeniden oluşturulmaz. Test: Task 2 `extension_members_are_flagged`, Task 7.

## Dosya Haritası (bu plan)

```
crates/pgt-catalog/src/
  node.rs                         Coll/Seg/NodeKind/NodeFlags genişler (Task 1)
  kinds/mod.rs                    yeniden yazılan dağıtıcı (Task 2)
  kinds/engine.rs                 genel list / properties / ddl motoru (Task 2–4)
  kinds/registry.rs               tür kayıtları: CollDef / ObjDef (Task 2–4)
  kinds/{column,table}.rs         Task 2: Column listesi üst düğüm kimliği alır; Task 5: tam Table DDL
  ddl.rs                          ColumnDef.identity_opts, column_line (Task 5)
  sql/list_*.sql                  18 dosya (Task 2)
  sql/props_*.sql                 13 dosya (Task 3)
  sql/ddl_*.sql, acl_{proc,type,tablespace}.sql   (Task 4)
  sql/tdd_*.sql                   Task 5: tam Table DDL parçaları
crates/pgt-catalog/tests/
  fixtures/{schema.sql,schema_pg11.sql,schema_pg12.sql,signature.sql}
  common/mod.rs                   sürüm dosyalarını yükler, yardımcılar (Task 2)
  {listing,properties,ddl}.rs     genişler; objects_properties.rs, objects_ddl.rs (yeni)
  roundtrip.rs, perf.rs           Task 7, Task 8
scripts/{test-matrix.ps1,measure.ps1}   Task 9, Task 10
src/features/explorer/{NodeIcon,TreeRow,controller}.ts(x)   Task 1, Task 6
src/features/inspector/{labels,Inspector}.ts(x)             Task 1, Task 6
docs/{testing.md,performance.md}  Task 9, Task 10
```

---

### Task 1: Düğüm modelini genişlet (Coll/Seg/NodeKind/NodeFlags) ve ön yüz türleri

**Files:**
- Modify: `crates/pgt-catalog/src/node.rs`
- Modify: `src/features/inspector/labels.ts`, `src/features/explorer/NodeIcon.tsx`, `src/features/explorer/TreeRow.tsx`
- Modify: `src/features/explorer/controller.test.ts`, `src/features/explorer/tree-model.test.ts` (test yardımcılarındaki `flags` nesneleri)
- Regenerate (cargo test ile): `src/ipc/bindings/NodeKind.ts`, `NodeFlags.ts`

**Interfaces:**
- Produces (Rust, `pgt_catalog::node`):
  - `Coll::{Databases, Schemas, Tables, Columns, Views, MatViews, Functions, Procedures, TriggerFunctions, Sequences, Types, Domains, Constraints, Indexes, Triggers, Policies, Partitions, Extensions, Roles, Tablespaces}`; kimlik etiketleri: `dbs, schemas, tables, columns, views, matviews, functions, procedures, trigfuncs, sequences, types, domains, constraints, indexes, triggers, policies, partitions, extensions, roles, tablespaces`.
  - `Seg::{Coll(Coll), Db(u32), Schema(u32), Table(u32), Column(i16), Constraint(u32), Index(u32), Trigger(u32), Policy(u32), View(u32), MatView(u32), Function(u32), Procedure(u32), TriggerFunction(u32), Sequence(u32), Type(u32), Domain(u32), Extension(u32), Role(u32), Tablespace(u32)}`; kimlik etiketleri: `db, sch, tbl, col, con, idx, trg, pol, view, mv, fn, proc, tfn, seq, typ, dom, ext, role, ts`.
  - `Seg::oid(self) -> Option<u32>`: `Db…Tablespace` için OID (Column ve Coll için `None`).
  - `NodeKind` (camelCase): mevcutlara ek olarak `constraintCollection, constraint, indexCollection, index, triggerCollection, trigger, policyCollection, policy, partitionCollection, viewCollection, view, materializedViewCollection, materializedView, functionCollection, function, procedureCollection, procedure, triggerFunctionCollection, triggerFunction, sequenceCollection, sequence, typeCollection, type, domainCollection, domain, extensionCollection, extension, roleCollection, role, tablespaceCollection, tablespace`. (Partition düğümleri `table` türündedir; `partitionCollection` yalnızca koleksiyondur.)
  - `NodeFlags.from_extension: bool` (TS: `fromExtension`).
- Produces (TS): `kindLabel` tüm türler için; `NodeIcon` tüm türler için; `TreeRow` `fromExtension` düğümlerini italik/soluk gösterir.

- [ ] **Step 1: Başarısız Rust testlerini yaz**

`crates/pgt-catalog/src/node.rs` içindeki `mod tests` bloğunun **sonuna** (son `}`'den önce) ekle:

```rust
    #[test]
    fn round_trips_new_shapes() {
        let cases = [
            format!("srv:{U}/roles"),
            format!("srv:{U}/tablespaces"),
            format!("srv:{U}/role:10"),
            format!("srv:{U}/ts:1663"),
            format!("srv:{U}/db:16384/extensions"),
            format!("srv:{U}/db:16384/ext:9"),
            format!("srv:{U}/db:16384/sch:2200/views"),
            format!("srv:{U}/db:16384/sch:2200/matviews"),
            format!("srv:{U}/db:16384/sch:2200/functions"),
            format!("srv:{U}/db:16384/sch:2200/procedures"),
            format!("srv:{U}/db:16384/sch:2200/trigfuncs"),
            format!("srv:{U}/db:16384/sch:2200/sequences"),
            format!("srv:{U}/db:16384/sch:2200/types"),
            format!("srv:{U}/db:16384/sch:2200/domains"),
            format!("srv:{U}/db:16384/sch:2200/view:30"),
            format!("srv:{U}/db:16384/sch:2200/mv:31"),
            format!("srv:{U}/db:16384/sch:2200/fn:32"),
            format!("srv:{U}/db:16384/sch:2200/proc:33"),
            format!("srv:{U}/db:16384/sch:2200/tfn:34"),
            format!("srv:{U}/db:16384/sch:2200/seq:35"),
            format!("srv:{U}/db:16384/sch:2200/typ:36"),
            format!("srv:{U}/db:16384/sch:2200/dom:37"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/constraints"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/con:40"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/idx:41"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/trg:42"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/pol:43"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/partitions"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/tbl:24580"),
            format!("srv:{U}/db:16384/sch:2200/view:30/col:-1"),
        ];
        for s in cases {
            assert_eq!(NodeId::parse(&s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn typed_segments_for_new_kinds() {
        let id = NodeId::parse(&format!("srv:{U}/db:1/sch:2/tbl:3/con:4")).unwrap();
        let NodeId::Node { path, .. } = id else { panic!() };
        assert_eq!(path, vec![Seg::Db(1), Seg::Schema(2), Seg::Table(3), Seg::Constraint(4)]);
        assert_eq!(Seg::Constraint(4).oid(), Some(4));
        assert_eq!(Seg::Role(10).oid(), Some(10));
        assert_eq!(Seg::Column(3).oid(), None);
        assert_eq!(Seg::Coll(Coll::Views).oid(), None);
    }

    #[test]
    fn rejects_new_garbage() {
        for bad in [format!("srv:{U}/con:abc"), format!("srv:{U}/con:-5"), format!("srv:{U}/nope:1"), format!("srv:{U}/rolez")] {
            assert!(matches!(NodeId::parse(&bad), Err(Error::Invalid(_))), "{bad}");
        }
    }

    #[test]
    fn new_kinds_and_flag_serialize_camel_case() {
        assert_eq!(serde_json::to_value(NodeKind::MaterializedViewCollection).unwrap(), "materializedViewCollection");
        assert_eq!(serde_json::to_value(NodeKind::TriggerFunction).unwrap(), "triggerFunction");
        let f = serde_json::to_value(NodeFlags::default()).unwrap();
        assert_eq!(f["fromExtension"], false);
    }
```

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run: `cargo test -p pgt-catalog node::tests`
Expected: FAIL — `Coll::Views`, `Seg::Constraint`, `Seg::oid`, `NodeKind::MaterializedViewCollection` tanımlı değil (derleme hatası).

- [ ] **Step 3: `node.rs` üst bölümünü uygula**

`crates/pgt-catalog/src/node.rs` içinde `use std::fmt;` satırından `pub struct NodeFlags { … }` yapısının kapanışına kadar olan bölümü (yani `TreeNode` tanımından önceki her şeyi, `mod tests` hariç) aşağıdakiyle **değiştir**:

```rust
use std::fmt;

use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Coll {
    Databases,
    Schemas,
    Tables,
    Columns,
    Views,
    MatViews,
    Functions,
    Procedures,
    TriggerFunctions,
    Sequences,
    Types,
    Domains,
    Constraints,
    Indexes,
    Triggers,
    Policies,
    Partitions,
    Extensions,
    Roles,
    Tablespaces,
}

const COLL_TAGS: [(Coll, &str); 20] = [
    (Coll::Databases, "dbs"),
    (Coll::Schemas, "schemas"),
    (Coll::Tables, "tables"),
    (Coll::Columns, "columns"),
    (Coll::Views, "views"),
    (Coll::MatViews, "matviews"),
    (Coll::Functions, "functions"),
    (Coll::Procedures, "procedures"),
    (Coll::TriggerFunctions, "trigfuncs"),
    (Coll::Sequences, "sequences"),
    (Coll::Types, "types"),
    (Coll::Domains, "domains"),
    (Coll::Constraints, "constraints"),
    (Coll::Indexes, "indexes"),
    (Coll::Triggers, "triggers"),
    (Coll::Policies, "policies"),
    (Coll::Partitions, "partitions"),
    (Coll::Extensions, "extensions"),
    (Coll::Roles, "roles"),
    (Coll::Tablespaces, "tablespaces"),
];

impl Coll {
    fn tag(self) -> &'static str {
        COLL_TAGS.iter().find(|(c, _)| *c == self).map(|(_, t)| *t).expect("her Coll'un etiketi var")
    }
    fn from_tag(tag: &str) -> Option<Self> {
        COLL_TAGS.iter().find(|(_, t)| *t == tag).map(|(c, _)| *c)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seg {
    Coll(Coll),
    Db(u32),
    Schema(u32),
    Table(u32),
    Column(i16),
    Constraint(u32),
    Index(u32),
    Trigger(u32),
    Policy(u32),
    View(u32),
    MatView(u32),
    Function(u32),
    Procedure(u32),
    TriggerFunction(u32),
    Sequence(u32),
    Type(u32),
    Domain(u32),
    Extension(u32),
    Role(u32),
    Tablespace(u32),
}

impl Seg {
    /// OID taşıyan nesne parçaları için `(etiket, oid)`. `Column` (attnum, negatif olabilir) ve `Coll` hariç.
    fn oid_tag(self) -> Option<(&'static str, u32)> {
        Some(match self {
            Seg::Db(o) => ("db", o),
            Seg::Schema(o) => ("sch", o),
            Seg::Table(o) => ("tbl", o),
            Seg::Constraint(o) => ("con", o),
            Seg::Index(o) => ("idx", o),
            Seg::Trigger(o) => ("trg", o),
            Seg::Policy(o) => ("pol", o),
            Seg::View(o) => ("view", o),
            Seg::MatView(o) => ("mv", o),
            Seg::Function(o) => ("fn", o),
            Seg::Procedure(o) => ("proc", o),
            Seg::TriggerFunction(o) => ("tfn", o),
            Seg::Sequence(o) => ("seq", o),
            Seg::Type(o) => ("typ", o),
            Seg::Domain(o) => ("dom", o),
            Seg::Extension(o) => ("ext", o),
            Seg::Role(o) => ("role", o),
            Seg::Tablespace(o) => ("ts", o),
            Seg::Column(_) | Seg::Coll(_) => return None,
        })
    }

    fn from_oid_tag(tag: &str, oid: u32) -> Option<Seg> {
        Some(match tag {
            "db" => Seg::Db(oid),
            "sch" => Seg::Schema(oid),
            "tbl" => Seg::Table(oid),
            "con" => Seg::Constraint(oid),
            "idx" => Seg::Index(oid),
            "trg" => Seg::Trigger(oid),
            "pol" => Seg::Policy(oid),
            "view" => Seg::View(oid),
            "mv" => Seg::MatView(oid),
            "fn" => Seg::Function(oid),
            "proc" => Seg::Procedure(oid),
            "tfn" => Seg::TriggerFunction(oid),
            "seq" => Seg::Sequence(oid),
            "typ" => Seg::Type(oid),
            "dom" => Seg::Domain(oid),
            "ext" => Seg::Extension(oid),
            "role" => Seg::Role(oid),
            "ts" => Seg::Tablespace(oid),
            _ => return None,
        })
    }

    /// Nesne parçalarının OID'i (`Column` ve `Coll` için `None`).
    pub fn oid(self) -> Option<u32> {
        self.oid_tag().map(|(_, o)| o)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeId {
    Root,
    Group(String),
    Node { server: Uuid, path: Vec<Seg> },
}

fn invalid(s: &str) -> Error {
    Error::Invalid(format!("geçersiz düğüm kimliği: {s}"))
}

impl NodeId {
    pub fn server(server: Uuid) -> Self {
        NodeId::Node { server, path: Vec::new() }
    }

    pub fn child(&self, seg: Seg) -> Self {
        match self {
            NodeId::Node { server, path } => {
                let mut path = path.clone();
                path.push(seg);
                NodeId::Node { server: *server, path }
            }
            _ => panic!("yalnızca sunucu altındaki düğümlerin çocuğu olabilir"),
        }
    }

    /// Son parçayı atar (koleksiyon düğümünün üst nesnesini bulmak için). Sunucu kimliğinde kendisini döndürür.
    pub fn parent(&self) -> Self {
        match self {
            NodeId::Node { server, path } if !path.is_empty() => {
                NodeId::Node { server: *server, path: path[..path.len() - 1].to_vec() }
            }
            other => other.clone(),
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        if s.is_empty() {
            return Ok(NodeId::Root);
        }
        if let Some(name) = s.strip_prefix("grp:") {
            return if name.is_empty() { Err(invalid(s)) } else { Ok(NodeId::Group(name.to_string())) };
        }
        let mut parts = s.split('/');
        let server = parts
            .next()
            .and_then(|p| p.strip_prefix("srv:"))
            .and_then(|u| Uuid::parse_str(u).ok())
            .ok_or_else(|| invalid(s))?;
        let mut path = Vec::new();
        for part in parts {
            let seg = match part.split_once(':') {
                Some(("col", n)) => Seg::Column(n.parse().map_err(|_| invalid(s))?),
                Some((tag, n)) => {
                    let oid: u32 = n.parse().map_err(|_| invalid(s))?;
                    Seg::from_oid_tag(tag, oid).ok_or_else(|| invalid(s))?
                }
                None => Seg::Coll(Coll::from_tag(part).ok_or_else(|| invalid(s))?),
            };
            path.push(seg);
        }
        Ok(NodeId::Node { server, path })
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeId::Root => Ok(()),
            NodeId::Group(name) => write!(f, "grp:{name}"),
            NodeId::Node { server, path } => {
                write!(f, "srv:{server}")?;
                for seg in path {
                    match seg {
                        Seg::Coll(c) => write!(f, "/{}", c.tag())?,
                        Seg::Column(n) => write!(f, "/col:{n}")?,
                        other => {
                            let (tag, oid) = other.oid_tag().expect("OID taşıyan parça");
                            write!(f, "/{tag}:{oid}")?
                        }
                    }
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NodeKind {
    ServerGroup,
    Server,
    DatabaseCollection,
    Database,
    SchemaCollection,
    Schema,
    TableCollection,
    Table,
    ColumnCollection,
    Column,
    ConstraintCollection,
    Constraint,
    IndexCollection,
    Index,
    TriggerCollection,
    Trigger,
    PolicyCollection,
    Policy,
    PartitionCollection,
    ViewCollection,
    View,
    MaterializedViewCollection,
    MaterializedView,
    FunctionCollection,
    Function,
    ProcedureCollection,
    Procedure,
    TriggerFunctionCollection,
    TriggerFunction,
    SequenceCollection,
    Sequence,
    TypeCollection,
    Type,
    DomainCollection,
    Domain,
    ExtensionCollection,
    Extension,
    RoleCollection,
    Role,
    TablespaceCollection,
    Tablespace,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodeFlags {
    pub connected: bool,
    pub partitioned: bool,
    pub system: bool,
    pub primary_key: bool,
    pub not_null: bool,
    pub from_extension: bool,
}
```

(Mevcut `TreeNode` tanımı ve `impl TreeNode` olduğu gibi kalır; `mod tests` bloğundaki eski testler de kalır.)

- [ ] **Step 4: Rust testlerinin geçtiğini doğrula**

Run: `cargo test -p pgt-catalog node::tests`
Expected: PASS (eski 5 + yeni 4). `src/ipc/bindings/NodeKind.ts` yeni birleşim tipini, `NodeFlags.ts` `fromExtension` alanını içerecek şekilde yeniden üretilir.

Run: `cargo test --workspace` (`PGT_TEST_URLS` olmadan)
Expected: PASS (Rust tarafı başka yerde `NodeFlags { … }` alan listesini tam yazmıyor; hepsi `..NodeFlags::default()` kullanır).

- [ ] **Step 5: Ön yüzde başarısız derlemeyi gör**

Run: `bun run typecheck`
Expected: FAIL — `labels.ts` (`Record<NodeKind, string>` eksik anahtarlar) ve test yardımcılarındaki `flags` nesnelerinde `fromExtension` eksik.

- [ ] **Step 6: Ön yüzü güncelle**

`src/features/inspector/labels.ts` tamamını değiştir:

```ts
import type { NodeKind } from "../../ipc/bindings/NodeKind";

const LABELS: Record<NodeKind, string> = {
  serverGroup: "Server Group",
  server: "Server",
  databaseCollection: "Databases",
  database: "Database",
  schemaCollection: "Schemas",
  schema: "Schema",
  tableCollection: "Tables",
  table: "Table",
  columnCollection: "Columns",
  column: "Column",
  constraintCollection: "Constraints",
  constraint: "Constraint",
  indexCollection: "Indexes",
  index: "Index",
  triggerCollection: "Triggers",
  trigger: "Trigger",
  policyCollection: "RLS Policies",
  policy: "RLS Policy",
  partitionCollection: "Partitions",
  viewCollection: "Views",
  view: "View",
  materializedViewCollection: "Materialized Views",
  materializedView: "Materialized View",
  functionCollection: "Functions",
  function: "Function",
  procedureCollection: "Procedures",
  procedure: "Procedure",
  triggerFunctionCollection: "Trigger Functions",
  triggerFunction: "Trigger Function",
  sequenceCollection: "Sequences",
  sequence: "Sequence",
  typeCollection: "Types",
  type: "Type",
  domainCollection: "Domains",
  domain: "Domain",
  extensionCollection: "Extensions",
  extension: "Extension",
  roleCollection: "Login/Group Roles",
  role: "Role",
  tablespaceCollection: "Tablespaces",
  tablespace: "Tablespace",
};

export function kindLabel(kind: NodeKind): string {
  return LABELS[kind];
}
```

`src/features/inspector/labels.test.ts` içindeki `it` bloğuna ekle (mevcut `expect`'lerin altına):

```ts
    expect(kindLabel("policyCollection")).toBe("RLS Policies");
    expect(kindLabel("triggerFunctionCollection")).toBe("Trigger Functions");
    expect(kindLabel("materializedView")).toBe("Materialized View");
    expect(kindLabel("roleCollection")).toBe("Login/Group Roles");
```

`src/features/explorer/NodeIcon.tsx` tamamını değiştir:

```tsx
import {
  Box, Columns3, Database, Eye, DatabaseZap, Folder, FunctionSquare, HardDrive, Hash, KeyRound, Layers, ListOrdered,
  Puzzle, Server, Shapes, Shield, ShieldCheck, SquareTerminal, Table2, TableProperties, User, Users, Zap,
} from "lucide-solid";
import { Dynamic } from "solid-js/web";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { isCollection } from "./tree-model";

function iconFor(node: TreeNode) {
  switch (node.kind) {
    case "server":
      return Server;
    case "database":
      return Database;
    case "schema":
      return Layers;
    case "table":
      return node.flags.partitioned ? TableProperties : Table2;
    case "column":
      return node.flags.primaryKey ? KeyRound : Columns3;
    case "constraint":
      return node.flags.primaryKey ? KeyRound : ShieldCheck;
    case "index":
      return ListOrdered;
    case "trigger":
    case "triggerFunction":
      return Zap;
    case "policy":
      return Shield;
    case "view":
      return Eye;
    case "materializedView":
      return DatabaseZap;
    case "function":
      return FunctionSquare;
    case "procedure":
      return SquareTerminal;
    case "sequence":
      return Hash;
    case "type":
      return Shapes;
    case "domain":
      return Box;
    case "extension":
      return Puzzle;
    case "role":
      return node.detail === "group" ? Users : User;
    case "tablespace":
      return HardDrive;
    default:
      return Folder;
  }
}

export function NodeIcon(props: { node: TreeNode }) {
  const color = () => {
    if (props.node.kind === "server") return props.node.flags.connected ? "text-accent" : "text-muted";
    return isCollection(props.node.kind) ? "text-muted" : "";
  };
  return <Dynamic component={iconFor(props.node)} size={14} class={`shrink-0 ${color()}`} aria-hidden="true" />;
}
```

`src/features/explorer/TreeRow.tsx` içinde ana `<div … classList=…>` öğesinin `classList`'ini şu şekilde değiştir (extension üyesi nesneler soluk/italik):

```tsx
      classList={{
        "bg-selected": props.selected,
        "hover:bg-hover": !props.selected,
        "italic text-muted": r().node.flags.fromExtension,
      }}
```

Test yardımcılarında (`src/features/explorer/controller.test.ts` ve `tree-model.test.ts`, ayrıca `tsc` başka dosyada `flags` literal'i bulursa orada da) `flags` sabitini güncelle:

```ts
const flags = { connected: false, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false };
```

- [ ] **Step 7: Ön yüz doğrulaması**

Run: `bun run typecheck && bun run test && bun run build`
Expected: hata yok; tüm testler PASS; build başarılı.

- [ ] **Step 8: Commit**

```bash
git add crates/pgt-catalog/src/node.rs src/ipc/bindings src/features
git commit -m "feat(catalog,ui): yeni nesne türleri için düğüm modeli, etiketler ve ikonlar"
```

---

### Task 2: Fixture genişlemesi, genel liste motoru ve 16 yeni türün listelenmesi

**Files:**
- Modify: `crates/pgt-catalog/tests/fixtures/schema.sql` (sona ekle)
- Create: `crates/pgt-catalog/tests/fixtures/schema_pg11.sql`, `crates/pgt-catalog/tests/fixtures/schema_pg12.sql`
- Modify: `crates/pgt-catalog/tests/common/mod.rs`
- Create: `crates/pgt-catalog/src/kinds/engine.rs`, `crates/pgt-catalog/src/kinds/registry.rs`
- Modify: `crates/pgt-catalog/src/kinds/mod.rs` (dağıtıcılar yeniden yazılır), `crates/pgt-catalog/src/kinds/column.rs` (`list` imzası)
- Create (hepsi `crates/pgt-catalog/src/sql/` altında): `list_constraints.sql`, `list_indexes.sql`, `list_triggers.sql`, `list_policies.sql`, `list_partitions.sql`, `list_views.sql`, `list_matviews.sql`, `list_sequences.sql`, `list_functions.sql`, `list_functions.pg10.sql`, `list_procedures.sql`, `list_trigger_functions.sql`, `list_trigger_functions.pg10.sql`, `list_types.sql`, `list_domains.sql`, `list_extensions.sql`, `list_roles.sql`, `list_tablespaces.sql`
- Modify: `crates/pgt-catalog/tests/listing.rs`

**Interfaces:**
- Consumes: Task 1 (`Coll`, `Seg`, `NodeKind`, `NodeFlags.from_extension`, `NodeId::parent()`); Plan 1 (`Catalog`, `db::{query, query_one, p!}`, `VersionedSql`, `PgVersion`).
- Produces:
  - `kinds::engine::{Level, CollDef, list, collection_node, target}` — `Level::{Server, Database}`; `CollDef { coll, label, coll_kind, child_kind, seg: fn(u32) -> Seg, list: &'static VersionedSql, level, takes_parent: bool, min_version: u32 }`.
  - `kinds::registry::{coll_def(Coll) -> Option<&'static CollDef>, schema_collections(PgVersion) -> Vec<&'static CollDef>, ROLES, TABLESPACES, EXTENSIONS, …}`
  - Yeni dağıtıcı davranışı (`Catalog::list_children`):
    - `srv` → `Databases`, `Login/Group Roles`, `Tablespaces`; `srv/roles`, `srv/tablespaces` → roller / tablespace'ler (`system` bayraklı olanlar `show_system` kapalıyken gizli).
    - `db` → `Schemas`, `Extensions`; `db/extensions` → extension'lar.
    - `sch` → `Tables`, `Views`, `Materialized Views`, `Functions`, `Procedures` (yalnızca PG ≥ 11), `Trigger Functions`, `Sequences`, `Types`, `Domains`.
    - `tbl` → `Columns`, `Constraints`, `Indexes`, `Triggers`, `RLS Policies`, + `Partitions` (yalnızca `relkind = 'p'`); `view` → `Columns`; `mv` → `Columns`, `Indexes`.
    - Yaprak düğümler (`col`, `con`, `idx`, `trg`, `pol`, `seq`, `fn`, `proc`, `tfn`, `typ`, `dom`, `ext`, `role`, `ts`) → boş liste.
  - `column::list(cat, server, db, rel: &NodeId, rel_oid: u32)`: çocuk kimlikleri `rel.child(Seg::Column(attnum))`.
  - Test yardımcıları (`tests/common`): `Fixture::{nodes_of(&NodeId, Coll) -> Vec<TreeNode>, labels_of(&NodeId, Coll) -> Vec<String>, child_of(&NodeId, Coll, &str) -> NodeId}`.

**Liste SQL sözleşmesi (her `list_*.sql` için):** sonuç kolonları `oid` (oid), `name` (text), `detail` (text, NULL olabilir), `leaf` (bool, `true` = çocuğu yok). İsteğe bağlı bool kolonlar: `partitioned`, `system`, `primary_key`, `not_null`, `from_extension` (yoksa `false`). `$1` yalnızca üst nesne OID'idir (şema / ilişki); `list_extensions`, `list_roles`, `list_tablespaces` parametresizdir. Sıralama `COLLATE "C"` ile ortamdan bağımsızdır.

- [ ] **Step 1: Fixture'ı genişlet**

`crates/pgt-catalog/tests/fixtures/schema.sql` dosyasının **sonuna** ekle:

```sql

-- ===== Plan 2 genişlemesi (PG 10+ taşınabilir) =====
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pgt_app') THEN
        CREATE ROLE pgt_app LOGIN CONNECTION LIMIT 5;
        GRANT pgt_reader TO pgt_app;
    END IF;
END
$$;

CREATE TYPE app.order_status AS ENUM ('new', 'paid', 'shipped');
CREATE TYPE app.address AS (street text, city text, zip varchar(10));
CREATE TYPE app.int_span AS RANGE (subtype = int4);
CREATE DOMAIN app.positive_int AS integer DEFAULT 1 NOT NULL CONSTRAINT positive_check CHECK (VALUE > 0);
COMMENT ON TYPE app.order_status IS 'Sipariş durumu';

CREATE SEQUENCE app.invoice_seq AS bigint INCREMENT BY 5 MINVALUE 100 MAXVALUE 100000 START 100 CACHE 10 CYCLE;
COMMENT ON SEQUENCE app.invoice_seq IS 'Fatura numarası';
GRANT USAGE ON SEQUENCE app.invoice_seq TO pgt_reader;

CREATE FUNCTION app.add_tax(amount numeric, rate numeric DEFAULT 0.18) RETURNS numeric
    LANGUAGE sql IMMUTABLE AS $$ SELECT amount * (1 + rate) $$;
COMMENT ON FUNCTION app.add_tax(numeric, numeric) IS 'KDV ekler';
GRANT EXECUTE ON FUNCTION app.add_tax(numeric, numeric) TO pgt_reader;
CREATE FUNCTION app.fmt(v integer) RETURNS text LANGUAGE sql AS $$ SELECT v::text $$;
CREATE FUNCTION app.fmt(v text) RETURNS text LANGUAGE sql AS $$ SELECT v $$;
CREATE FUNCTION app.touch_row() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.name := coalesce(NEW.name, 'x');

    RETURN NEW;
END
$$;
CREATE TRIGGER customers_touch BEFORE UPDATE ON app.customers
    FOR EACH ROW WHEN (OLD.email IS DISTINCT FROM NEW.email) EXECUTE PROCEDURE app.touch_row();
COMMENT ON TRIGGER customers_touch ON app.customers IS 'Dokunma';

CREATE INDEX orders_customer_idx ON app.orders (customer_id, placed_on DESC) WHERE total > 0;
CREATE UNIQUE INDEX customers_lower_name_idx ON app.customers (lower(name));
COMMENT ON INDEX app.orders_customer_idx IS 'Müşteri dizini';

CREATE VIEW app.customer_orders WITH (security_barrier = true) AS
    SELECT c.id AS customer_id, c.email, o.id AS order_id, o.total
    FROM app.customers c JOIN app.orders o ON o.customer_id = c.id;
COMMENT ON VIEW app.customer_orders IS 'Müşteri siparişleri';
GRANT SELECT ON app.customer_orders TO pgt_reader;
CREATE VIEW app."Mixed View" AS SELECT 1 AS "Çok Kolon";
CREATE MATERIALIZED VIEW app.order_totals AS
    SELECT customer_id, sum(total) AS total FROM app.orders GROUP BY customer_id WITH DATA;
CREATE UNIQUE INDEX order_totals_pk ON app.order_totals (customer_id);

ALTER TABLE app.customers ENABLE ROW LEVEL SECURITY;
CREATE POLICY customers_select ON app.customers FOR SELECT TO pgt_reader USING (true);
CREATE POLICY customers_own ON app.customers AS RESTRICTIVE FOR UPDATE USING (id > 0) WITH CHECK (id > 0);
GRANT SELECT (email) ON app.customers TO pgt_app;

CREATE SCHEMA extra;
CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA extra;
CREATE TABLE extra.audit_base (id bigint, note text);
CREATE TABLE extra.audit_child (extra_col int) INHERITS (extra.audit_base);
CREATE TABLE extra.tickets (
    id integer GENERATED BY DEFAULT AS IDENTITY (START WITH 10 INCREMENT BY 2),
    title text NOT NULL
);
```

> `pg_trgm` bir contrib eklentisidir; test sunucusunda `postgresql-contrib` (Windows EDB kurulumunda varsayılan) kurulu olmalıdır.

`crates/pgt-catalog/tests/fixtures/schema_pg11.sql` (yeni):

```sql
CREATE PROCEDURE app.archive_orders(older_than date) LANGUAGE sql AS $$ DELETE FROM app.orders WHERE placed_on < older_than $$;
COMMENT ON PROCEDURE app.archive_orders(date) IS 'Eski siparişleri siler';
CREATE INDEX events_day_idx ON app.events (happened_on);
CREATE INDEX orders_cover_idx ON app.orders (customer_id) INCLUDE (total);
CREATE TABLE extra.hits (id bigint, bucket int) PARTITION BY HASH (id);
CREATE TABLE extra.hits_0 PARTITION OF extra.hits FOR VALUES WITH (MODULUS 2, REMAINDER 0);
CREATE TABLE extra.hits_1 PARTITION OF extra.hits FOR VALUES WITH (MODULUS 2, REMAINDER 1);
```

`crates/pgt-catalog/tests/fixtures/schema_pg12.sql` (yeni):

```sql
CREATE TABLE extra.line_items (
    qty integer NOT NULL,
    price numeric(10, 2) NOT NULL,
    total numeric(12, 2) GENERATED ALWAYS AS (qty * price) STORED
);
```

`crates/pgt-catalog/tests/common/mod.rs` içinde:

1. Sabitlerin altına ekle:

```rust
const SCHEMA_PG11_SQL: &str = include_str!("../fixtures/schema_pg11.sql");
const SCHEMA_PG12_SQL: &str = include_str!("../fixtures/schema_pg12.sql");
```

2. `setup_all` içinde `db.batch_execute(SCHEMA_SQL).await.unwrap();` satırının hemen altına ekle:

```rust
        let version_num: i32 = db
            .query_one("SELECT current_setting('server_version_num')::int4", &[])
            .await
            .unwrap()
            .get(0);
        if version_num / 10_000 >= 11 {
            db.batch_execute(SCHEMA_PG11_SQL).await.unwrap();
        }
        if version_num / 10_000 >= 12 {
            db.batch_execute(SCHEMA_PG12_SQL).await.unwrap();
        }
```

3. `impl Fixture` bloğuna ekle:

```rust
    /// `parent`'ın `coll` koleksiyonunun çocukları.
    pub async fn nodes_of(&self, parent: &NodeId, coll: Coll) -> Vec<TreeNode> {
        self.catalog.list_children(&parent.child(Seg::Coll(coll))).await.unwrap()
    }

    pub async fn labels_of(&self, parent: &NodeId, coll: Coll) -> Vec<String> {
        self.nodes_of(parent, coll).await.into_iter().map(|n| n.label).collect()
    }

    /// Koleksiyonda etiketi bulur ve düğümün kimliğini döndürür.
    pub async fn child_of(&self, parent: &NodeId, coll: Coll, label: &str) -> NodeId {
        let list = self.nodes_of(parent, coll).await;
        NodeId::parse(&find(&list, label).id).unwrap()
    }
```

- [ ] **Step 2: Başarısız listeleme testlerini yaz**

`crates/pgt-catalog/tests/listing.rs` içinde:

(a) Dosyanın başındaki `use` satırlarını şu hale getir:

```rust
mod common;

use common::{find, fixtures, labels};
use pgt_catalog::node::{Coll, NodeId, NodeKind, Seg};
use pgt_core::Error;
use pgt_core::config::ServerConfig;
```

(b) Eski `server_has_databases_collection` testini **değiştir**:

```rust
#[tokio::test]
async fn server_has_databases_roles_and_tablespaces() {
    for fx in fixtures().await {
        let children = fx.catalog.list_children(&fx.server_id()).await.unwrap();
        assert_eq!(
            children.iter().map(|c| (c.kind, c.label.as_str())).collect::<Vec<_>>(),
            [
                (NodeKind::DatabaseCollection, "Databases"),
                (NodeKind::RoleCollection, "Login/Group Roles"),
                (NodeKind::TablespaceCollection, "Tablespaces"),
            ]
        );
    }
}
```

(c) Eski `collection_children_of_table_and_schema` testini **değiştir**:

```rust
#[tokio::test]
async fn schema_collections_depend_on_version() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let kids = fx.catalog.list_children(&schema).await.unwrap();
        let mut expected = vec!["Tables", "Views", "Materialized Views", "Functions"];
        if fx.major().await >= 11 {
            expected.push("Procedures");
        }
        expected.extend(["Trigger Functions", "Sequences", "Types", "Domains"]);
        assert_eq!(labels(&kids), expected);
    }
}

#[tokio::test]
async fn partitions_collection_only_for_partitioned_tables() {
    for fx in fixtures().await {
        let orders = fx.catalog.list_children(&fx.table("app", "orders").await).await.unwrap();
        assert_eq!(labels(&orders), ["Columns", "Constraints", "Indexes", "Triggers", "RLS Policies"]);
        let events = fx.catalog.list_children(&fx.table("app", "events").await).await.unwrap();
        assert_eq!(labels(&events), ["Columns", "Constraints", "Indexes", "Triggers", "RLS Policies", "Partitions"]);
    }
}
```

(d) Dosyanın **sonuna** ekle:

```rust
#[tokio::test]
async fn roles_and_tablespaces_hide_system_by_default() {
    for fx in fixtures().await {
        let roles = fx.labels_of(&fx.server_id(), Coll::Roles).await;
        for expected in ["pgt_reader", "pgt_app"] {
            assert!(roles.iter().any(|r| r == expected), "{expected} yok: {roles:?}");
        }
        assert!(!roles.iter().any(|r| r.starts_with("pg_")), "yerleşik roller gizli olmalı: {roles:?}");
        let node = find(&fx.nodes_of(&fx.server_id(), Coll::Roles).await, "pgt_app").clone();
        assert_eq!(node.detail.as_deref(), Some("login"));
        assert!(!node.has_children);
        assert!(fx.labels_of(&fx.server_id(), Coll::Tablespaces).await.is_empty());
        fx.catalog.set_show_system(true);
        let all = fx.labels_of(&fx.server_id(), Coll::Tablespaces).await;
        assert_eq!(all, ["pg_default", "pg_global"]);
        fx.catalog.set_show_system(false);
    }
}

#[tokio::test]
async fn database_children_and_extensions() {
    for fx in fixtures().await {
        let kids = fx.catalog.list_children(&fx.db()).await.unwrap();
        assert_eq!(labels(&kids), ["Schemas", "Extensions"]);
        let ext = fx.labels_of(&fx.db(), Coll::Extensions).await;
        assert_eq!(ext, ["pg_trgm", "plpgsql"]);
        let nodes = fx.nodes_of(&fx.db(), Coll::Extensions).await;
        assert_eq!(find(&nodes, "pg_trgm").kind, NodeKind::Extension);
        assert!(find(&nodes, "pg_trgm").detail.is_some());
    }
}

#[tokio::test]
async fn table_sub_objects_are_listed() {
    for fx in fixtures().await {
        let customers = fx.table("app", "customers").await;
        let constraints = fx.nodes_of(&customers, Coll::Constraints).await;
        assert_eq!(labels(&constraints), ["customers_pkey", "customers_email_key", "customers_email_check"]);
        assert!(find(&constraints, "customers_pkey").flags.primary_key);
        assert_eq!(find(&constraints, "customers_email_check").detail.as_deref(), Some("check"));
        assert!(constraints.iter().all(|c| !c.has_children && c.kind == NodeKind::Constraint));

        let indexes = fx.nodes_of(&customers, Coll::Indexes).await;
        assert_eq!(labels(&indexes), ["customers_email_key", "customers_lower_name_idx", "customers_pkey"]);
        assert_eq!(find(&indexes, "customers_pkey").detail.as_deref(), Some("btree, primary key"));

        assert_eq!(fx.labels_of(&customers, Coll::Triggers).await, ["customers_touch"]);
        assert_eq!(fx.labels_of(&customers, Coll::Policies).await, ["customers_own", "customers_select"]);
        let policies = fx.nodes_of(&customers, Coll::Policies).await;
        assert_eq!(find(&policies, "customers_select").detail.as_deref(), Some("select"));
    }
}

#[tokio::test]
async fn partitions_are_tables_with_bounds() {
    for fx in fixtures().await {
        let events = fx.table("app", "events").await;
        let parts = fx.nodes_of(&events, Coll::Partitions).await;
        assert_eq!(labels(&parts), ["events_2026"]);
        assert_eq!(parts[0].kind, NodeKind::Table);
        assert!(parts[0].has_children);
        assert!(parts[0].detail.as_deref().unwrap().starts_with("FOR VALUES FROM"));
        // bir partition normal bir tablo gibi açılır
        let part_id = NodeId::parse(&parts[0].id).unwrap();
        let cols = fx.catalog.list_children(&part_id).await.unwrap();
        assert_eq!(labels(&cols)[0], "Columns");
    }
}

#[tokio::test]
async fn views_and_materialized_views_with_columns_and_indexes() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let views = fx.nodes_of(&schema, Coll::Views).await;
        assert_eq!(labels(&views), ["Mixed View", "customer_orders"]);
        assert!(views.iter().all(|v| v.kind == NodeKind::View && v.has_children));

        let view = fx.child_of(&schema, Coll::Views, "customer_orders").await;
        assert_eq!(labels(&fx.catalog.list_children(&view).await.unwrap()), ["Columns"]);
        assert_eq!(fx.labels_of(&view, Coll::Columns).await, ["customer_id", "email", "order_id", "total"]);
        let mixed = fx.child_of(&schema, Coll::Views, "Mixed View").await;
        assert_eq!(fx.labels_of(&mixed, Coll::Columns).await, ["Çok Kolon"]);

        let mvs = fx.nodes_of(&schema, Coll::MatViews).await;
        assert_eq!(labels(&mvs), ["order_totals"]);
        let mv = fx.child_of(&schema, Coll::MatViews, "order_totals").await;
        assert_eq!(labels(&fx.catalog.list_children(&mv).await.unwrap()), ["Columns", "Indexes"]);
        assert_eq!(fx.labels_of(&mv, Coll::Indexes).await, ["order_totals_pk"]);
        assert_eq!(fx.labels_of(&mv, Coll::Columns).await, ["customer_id", "total"]);
    }
}

#[tokio::test]
async fn functions_are_listed_with_identity_arguments() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let funcs = fx.labels_of(&schema, Coll::Functions).await;
        for expected in ["add_tax(amount numeric, rate numeric)", "fmt(v integer)", "fmt(v text)"] {
            assert!(funcs.iter().any(|f| f == expected), "{expected} yok: {funcs:?}");
        }
        assert!(!funcs.iter().any(|f| f.starts_with("touch_row")), "trigger fonksiyonu burada görünmemeli");
        assert!(!funcs.iter().any(|f| f.starts_with("int_span")), "range kurucuları gizli olmalı: {funcs:?}");
        assert_eq!(fx.labels_of(&schema, Coll::TriggerFunctions).await, ["touch_row()"]);
        if fx.major().await >= 11 {
            assert_eq!(fx.labels_of(&schema, Coll::Procedures).await, ["archive_orders(older_than date)"]);
        }
    }
}

#[tokio::test]
async fn sequences_types_and_domains() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        // kimlik (identity) sequence'leri listelenmez; bigserial'ın sequence'i listelenir
        assert_eq!(fx.labels_of(&schema, Coll::Sequences).await, ["invoice_seq", "orders_id_seq"]);
        let extra = fx.schema("extra").await;
        assert!(fx.labels_of(&extra, Coll::Sequences).await.is_empty());
        let types = fx.nodes_of(&schema, Coll::Types).await;
        assert_eq!(labels(&types), ["address", "int_span", "order_status"]);
        assert_eq!(find(&types, "order_status").detail.as_deref(), Some("enum"));
        assert_eq!(find(&types, "address").detail.as_deref(), Some("composite"));
        assert_eq!(find(&types, "int_span").detail.as_deref(), Some("range"));
        let domains = fx.nodes_of(&schema, Coll::Domains).await;
        assert_eq!(labels(&domains), ["positive_int"]);
        assert_eq!(domains[0].detail.as_deref(), Some("integer"));
    }
}

#[tokio::test]
async fn extension_members_are_flagged() {
    for fx in fixtures().await {
        let extra = fx.schema("extra").await;
        let funcs = fx.nodes_of(&extra, Coll::Functions).await;
        assert!(!funcs.is_empty(), "pg_trgm fonksiyonları extra şemasında olmalı");
        assert!(funcs.iter().all(|f| f.flags.from_extension));
        let app = fx.schema("app").await;
        assert!(fx.nodes_of(&app, Coll::Functions).await.iter().all(|f| !f.flags.from_extension));
    }
}

#[tokio::test]
async fn empty_collections_expand_to_nothing() {
    for fx in fixtures().await {
        let extra = fx.schema("extra").await;
        assert!(fx.nodes_of(&extra, Coll::Views).await.is_empty());
        assert!(fx.nodes_of(&extra, Coll::Domains).await.is_empty());
        let items = fx.table("app", "Order Items").await;
        assert!(fx.nodes_of(&items, Coll::Triggers).await.is_empty());
        assert!(fx.nodes_of(&items, Coll::Policies).await.is_empty());
    }
}

#[tokio::test]
async fn leaf_nodes_have_no_children_and_wrong_collections_are_invalid() {
    for fx in fixtures().await {
        let customers = fx.table("app", "customers").await;
        let pkey = fx.child_of(&customers, Coll::Constraints, "customers_pkey").await;
        assert!(fx.catalog.list_children(&pkey).await.unwrap().is_empty());
        // tabloda "Views" koleksiyonu yok
        let wrong = customers.child(Seg::Coll(Coll::Views));
        assert!(matches!(fx.catalog.list_children(&wrong).await, Err(Error::Invalid(_))));
        // sunucu altında "Tables" koleksiyonu yok
        let wrong = fx.server_id().child(Seg::Coll(Coll::Tables));
        assert!(matches!(fx.catalog.list_children(&wrong).await, Err(Error::Invalid(_))));
    }
}
```

- [ ] **Step 3: Testlerin başarısız olduğunu doğrula**

Run (PowerShell): `$env:PGT_TEST_URLS = (Get-Content .superpowers/sdd/test-url.txt); cargo test -p pgt-catalog --test listing`
Expected: FAIL — `Err(Invalid)`: yeni koleksiyonlar henüz dağıtılmıyor; eski `kinds::list_children` yeni yol şekillerini tanımıyor.

- [ ] **Step 4: SQL dosyalarını yaz** (`crates/pgt-catalog/src/sql/`)

Aşağıdaki sorgular PostgreSQL 18'de çalıştırılarak doğrulanmıştır.

`list_constraints.sql`:

```sql
SELECT k.oid, k.conname::text AS name,
       CASE k.contype WHEN 'p' THEN 'primary key' WHEN 'f' THEN 'foreign key' WHEN 'u' THEN 'unique'
                      WHEN 'c' THEN 'check' WHEN 'x' THEN 'exclusion' END AS detail,
       true AS leaf, k.contype = 'p' AS primary_key
FROM pg_constraint k
WHERE k.conrelid = $1 AND k.contype IN ('p', 'u', 'c', 'x', 'f')
ORDER BY CASE k.contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'c' THEN 2 WHEN 'x' THEN 3 ELSE 4 END,
         k.conname COLLATE "C"
```

`list_indexes.sql`:

```sql
SELECT x.indexrelid AS oid, c.relname::text AS name,
       concat_ws(', ', am.amname, CASE WHEN x.indisprimary THEN 'primary key' WHEN x.indisunique THEN 'unique' END) AS detail,
       true AS leaf, x.indisprimary AS primary_key
FROM pg_index x
JOIN pg_class c ON c.oid = x.indexrelid
JOIN pg_am am ON am.oid = c.relam
WHERE x.indrelid = $1
ORDER BY c.relname COLLATE "C"
```

`list_triggers.sql`:

```sql
SELECT t.oid, t.tgname::text AS name,
       CASE t.tgenabled WHEN 'D' THEN 'disabled' WHEN 'R' THEN 'replica' WHEN 'A' THEN 'always' END AS detail,
       true AS leaf
FROM pg_trigger t
WHERE t.tgrelid = $1 AND NOT t.tgisinternal
ORDER BY t.tgname COLLATE "C"
```

`list_policies.sql`:

```sql
SELECT p.oid, p.polname::text AS name,
       CASE p.polcmd WHEN 'r' THEN 'select' WHEN 'a' THEN 'insert' WHEN 'w' THEN 'update' WHEN 'd' THEN 'delete' ELSE 'all' END AS detail,
       true AS leaf
FROM pg_policy p
WHERE p.polrelid = $1
ORDER BY p.polname COLLATE "C"
```

`list_partitions.sql`:

```sql
SELECT c.oid, c.relname::text AS name, pg_get_expr(c.relpartbound, c.oid) AS detail,
       false AS leaf, c.relkind = 'p' AS partitioned
FROM pg_inherits i
JOIN pg_class c ON c.oid = i.inhrelid
WHERE i.inhparent = $1 AND c.relispartition
ORDER BY c.relname COLLATE "C"
```

`list_views.sql`:

```sql
SELECT c.oid, c.relname::text AS name, NULL::text AS detail, false AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e') AS from_extension
FROM pg_class c
WHERE c.relnamespace = $1 AND c.relkind = 'v'
ORDER BY c.relname COLLATE "C"
```

`list_matviews.sql` (aynı, yalnızca `c.relkind = 'm'`):

```sql
SELECT c.oid, c.relname::text AS name, NULL::text AS detail, false AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e') AS from_extension
FROM pg_class c
WHERE c.relnamespace = $1 AND c.relkind = 'm'
ORDER BY c.relname COLLATE "C"
```

`list_sequences.sql` (kimlik/identity sequence'leri hariç; onlar kolon tanımının parçasıdır):

```sql
SELECT c.oid, c.relname::text AS name, format_type(s.seqtypid, NULL) AS detail, true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e') AS from_extension
FROM pg_class c
JOIN pg_sequence s ON s.seqrelid = c.oid
WHERE c.relnamespace = $1 AND c.relkind = 'S'
  AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'i')
ORDER BY c.relname COLLATE "C"
```

`list_functions.sql` (PG 11+; aggregate'ler hariç, trigger fonksiyonları hariç, range kurucuları gibi iç bağımlılıklı olanlar hariç):

```sql
SELECT p.oid, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')' AS name,
       pg_get_function_result(p.oid) AS detail, true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e') AS from_extension
FROM pg_proc p
WHERE p.pronamespace = $1 AND p.prokind IN ('f', 'w') AND p.prorettype <> 'trigger'::regtype
  AND NOT EXISTS (SELECT 1 FROM pg_depend i WHERE i.classid = 'pg_proc'::regclass AND i.objid = p.oid AND i.deptype = 'i')
ORDER BY p.proname COLLATE "C", p.oid
```

`list_functions.pg10.sql` (`prokind` PG 11'de eklendi):

```sql
SELECT p.oid, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')' AS name,
       pg_get_function_result(p.oid) AS detail, true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e') AS from_extension
FROM pg_proc p
WHERE p.pronamespace = $1 AND NOT p.proisagg AND p.prorettype <> 'trigger'::regtype
  AND NOT EXISTS (SELECT 1 FROM pg_depend i WHERE i.classid = 'pg_proc'::regclass AND i.objid = p.oid AND i.deptype = 'i')
ORDER BY p.proname COLLATE "C", p.oid
```

`list_procedures.sql` (yalnızca PG 11+):

```sql
SELECT p.oid, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')' AS name,
       NULL::text AS detail, true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e') AS from_extension
FROM pg_proc p
WHERE p.pronamespace = $1 AND p.prokind = 'p'
  AND NOT EXISTS (SELECT 1 FROM pg_depend i WHERE i.classid = 'pg_proc'::regclass AND i.objid = p.oid AND i.deptype = 'i')
ORDER BY p.proname COLLATE "C", p.oid
```

`list_trigger_functions.sql`:

```sql
SELECT p.oid, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')' AS name,
       pg_get_function_result(p.oid) AS detail, true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e') AS from_extension
FROM pg_proc p
WHERE p.pronamespace = $1 AND p.prokind IN ('f', 'w') AND p.prorettype = 'trigger'::regtype
  AND NOT EXISTS (SELECT 1 FROM pg_depend i WHERE i.classid = 'pg_proc'::regclass AND i.objid = p.oid AND i.deptype = 'i')
ORDER BY p.proname COLLATE "C", p.oid
```

`list_trigger_functions.pg10.sql`:

```sql
SELECT p.oid, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')' AS name,
       pg_get_function_result(p.oid) AS detail, true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e') AS from_extension
FROM pg_proc p
WHERE p.pronamespace = $1 AND NOT p.proisagg AND p.prorettype = 'trigger'::regtype
  AND NOT EXISTS (SELECT 1 FROM pg_depend i WHERE i.classid = 'pg_proc'::regclass AND i.objid = p.oid AND i.deptype = 'i')
ORDER BY p.proname COLLATE "C", p.oid
```

`list_types.sql` (tablo satır tipleri, dizi tipleri, domain'ler ve çoklu-aralık (`m`) tipleri hariç):

```sql
SELECT t.oid, t.typname::text AS name,
       CASE t.typtype WHEN 'e' THEN 'enum' WHEN 'c' THEN 'composite' WHEN 'r' THEN 'range' ELSE 'base' END AS detail,
       true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_type'::regclass AND d.objid = t.oid AND d.deptype = 'e') AS from_extension
FROM pg_type t
WHERE t.typnamespace = $1
  AND t.typtype IN ('e', 'c', 'r', 'b')
  AND (t.typtype <> 'c' OR EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c'))
  AND NOT EXISTS (SELECT 1 FROM pg_type el WHERE el.oid = t.typelem AND el.typarray = t.oid)
ORDER BY t.typname COLLATE "C"
```

`list_domains.sql`:

```sql
SELECT t.oid, t.typname::text AS name, format_type(t.typbasetype, t.typtypmod) AS detail, true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_type'::regclass AND d.objid = t.oid AND d.deptype = 'e') AS from_extension
FROM pg_type t
WHERE t.typnamespace = $1 AND t.typtype = 'd'
ORDER BY t.typname COLLATE "C"
```

`list_extensions.sql` (parametresiz):

```sql
SELECT e.oid, e.extname::text AS name, e.extversion AS detail, true AS leaf
FROM pg_extension e
ORDER BY e.extname COLLATE "C"
```

`list_roles.sql` (parametresiz; bakım bağlantısı):

```sql
SELECT r.oid, r.rolname::text AS name,
       CASE WHEN r.rolsuper THEN 'superuser' WHEN r.rolcanlogin THEN 'login' ELSE 'group' END AS detail,
       true AS leaf, (r.rolname LIKE 'pg\_%') AS system
FROM pg_roles r
ORDER BY (r.rolname LIKE 'pg\_%'), r.rolname COLLATE "C"
```

`list_tablespaces.sql` (parametresiz; bakım bağlantısı):

```sql
SELECT t.oid, t.spcname::text AS name, NULL::text AS detail, true AS leaf,
       (t.spcname LIKE 'pg\_%') AS system
FROM pg_tablespace t
ORDER BY (t.spcname LIKE 'pg\_%'), t.spcname COLLATE "C"
```

- [ ] **Step 5: Genel motoru yaz**

`crates/pgt-catalog/src/kinds/engine.rs` (yeni; Task 3 ve 4 bu dosyaya fonksiyon ekler):

```rust
//! Veri sözleşmeli SQL dosyalarını `TreeNode` / `PropertySection` / DDL metnine çeviren genel motor.

use pgt_core::connection::Target;
use pgt_core::{Error, Result};
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{Coll, NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

/// Sorgunun hangi bağlantıda çalışacağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    /// Sunucu geneli kataloglar (roller, tablespace'ler): bakım veritabanı bağlantısı.
    Server,
    /// Veritabanına özel kataloglar.
    Database,
}

pub(crate) fn target(level: Level, db: Option<u32>) -> Result<Target> {
    match (level, db) {
        (Level::Server, _) => Ok(Target::Maintenance),
        (Level::Database, Some(d)) => Ok(Target::Database(d)),
        (Level::Database, None) => Err(Error::Invalid("veritabanı kimliği yok".into())),
    }
}

/// Bir koleksiyonun (ve çocuklarının) tanımı.
pub(crate) struct CollDef {
    pub coll: Coll,
    pub label: &'static str,
    pub coll_kind: NodeKind,
    pub child_kind: NodeKind,
    pub seg: fn(u32) -> Seg,
    pub list: &'static VersionedSql,
    pub level: Level,
    /// Liste sorgusu `$1` olarak üst nesnenin OID'ini alır mı.
    pub takes_parent: bool,
    /// Koleksiyonun gösterildiği en düşük PostgreSQL ana sürümü.
    pub min_version: u32,
}

/// `parent` altında koleksiyon düğümünü üretir.
pub(crate) fn collection_node(parent: &NodeId, def: &CollDef) -> TreeNode {
    TreeNode::new(&parent.child(Seg::Coll(def.coll)), def.coll_kind, def.label)
}

/// Koleksiyonun çocuklarını listeler. Çocuk kimlikleri koleksiyon parçasını atlar: `parent.child(seg(oid))`.
pub(crate) async fn list(
    cat: &Catalog,
    server: Uuid,
    db: Option<u32>,
    def: &CollDef,
    parent: &NodeId,
    parent_oid: Option<u32>,
) -> Result<Vec<TreeNode>> {
    let params = match (def.takes_parent, parent_oid) {
        (true, Some(oid)) => p![oid],
        (true, None) => return Err(Error::Invalid(format!("{} için üst nesne yok", def.label))),
        (false, _) => p![],
    };
    let rows = query(&cat.mgr, server, target(def.level, db)?, def.list, params).await?;
    let show_system = cat.show_system();
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let flag = |c: &str| r.try_get::<_, bool>(c).unwrap_or(false);
        if flag("system") && !show_system {
            continue;
        }
        let oid: u32 = r.get("oid");
        let mut node = TreeNode::new(&parent.child((def.seg)(oid)), def.child_kind, r.get::<_, String>("name"))
            .with_detail(r.try_get::<_, Option<String>>("detail").ok().flatten())
            .with_flags(NodeFlags {
                partitioned: flag("partitioned"),
                system: flag("system"),
                primary_key: flag("primary_key"),
                not_null: flag("not_null"),
                from_extension: flag("from_extension"),
                ..NodeFlags::default()
            });
        if r.try_get::<_, bool>("leaf").unwrap_or(true) {
            node = node.leaf();
        }
        out.push(node);
    }
    Ok(out)
}
```

- [ ] **Step 6: Kayıt defterini yaz**

`crates/pgt-catalog/src/kinds/registry.rs` (yeni; Task 3 ve 4 `props_def` / `ddl_def` ekler):

```rust
//! Yeni nesne türlerinin kayıtları. Bir türü eklemek: SQL dosyaları + burada bir kayıt.

use super::engine::{CollDef, Level};
use crate::node::{Coll, NodeKind, Seg};
use crate::version::{PgVersion, VersionedSql};

macro_rules! sql {
    ($name:literal) => {
        include_str!(concat!("../sql/", $name))
    };
}

pub(crate) static CONSTRAINTS: CollDef = CollDef {
    coll: Coll::Constraints,
    label: "Constraints",
    coll_kind: NodeKind::ConstraintCollection,
    child_kind: NodeKind::Constraint,
    seg: Seg::Constraint,
    list: &VersionedSql(&[(10, sql!("list_constraints.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static INDEXES: CollDef = CollDef {
    coll: Coll::Indexes,
    label: "Indexes",
    coll_kind: NodeKind::IndexCollection,
    child_kind: NodeKind::Index,
    seg: Seg::Index,
    list: &VersionedSql(&[(10, sql!("list_indexes.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static TRIGGERS: CollDef = CollDef {
    coll: Coll::Triggers,
    label: "Triggers",
    coll_kind: NodeKind::TriggerCollection,
    child_kind: NodeKind::Trigger,
    seg: Seg::Trigger,
    list: &VersionedSql(&[(10, sql!("list_triggers.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static POLICIES: CollDef = CollDef {
    coll: Coll::Policies,
    label: "RLS Policies",
    coll_kind: NodeKind::PolicyCollection,
    child_kind: NodeKind::Policy,
    seg: Seg::Policy,
    list: &VersionedSql(&[(10, sql!("list_policies.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

/// Partition'lar `table` düğümleridir; yalnızca koleksiyon türü farklıdır.
pub(crate) static PARTITIONS: CollDef = CollDef {
    coll: Coll::Partitions,
    label: "Partitions",
    coll_kind: NodeKind::PartitionCollection,
    child_kind: NodeKind::Table,
    seg: Seg::Table,
    list: &VersionedSql(&[(10, sql!("list_partitions.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static VIEWS: CollDef = CollDef {
    coll: Coll::Views,
    label: "Views",
    coll_kind: NodeKind::ViewCollection,
    child_kind: NodeKind::View,
    seg: Seg::View,
    list: &VersionedSql(&[(10, sql!("list_views.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static MATVIEWS: CollDef = CollDef {
    coll: Coll::MatViews,
    label: "Materialized Views",
    coll_kind: NodeKind::MaterializedViewCollection,
    child_kind: NodeKind::MaterializedView,
    seg: Seg::MatView,
    list: &VersionedSql(&[(10, sql!("list_matviews.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static FUNCTIONS: CollDef = CollDef {
    coll: Coll::Functions,
    label: "Functions",
    coll_kind: NodeKind::FunctionCollection,
    child_kind: NodeKind::Function,
    seg: Seg::Function,
    list: &VersionedSql(&[(10, sql!("list_functions.pg10.sql")), (11, sql!("list_functions.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static PROCEDURES: CollDef = CollDef {
    coll: Coll::Procedures,
    label: "Procedures",
    coll_kind: NodeKind::ProcedureCollection,
    child_kind: NodeKind::Procedure,
    seg: Seg::Procedure,
    list: &VersionedSql(&[(11, sql!("list_procedures.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 11,
};

pub(crate) static TRIGGER_FUNCTIONS: CollDef = CollDef {
    coll: Coll::TriggerFunctions,
    label: "Trigger Functions",
    coll_kind: NodeKind::TriggerFunctionCollection,
    child_kind: NodeKind::TriggerFunction,
    seg: Seg::TriggerFunction,
    list: &VersionedSql(&[(10, sql!("list_trigger_functions.pg10.sql")), (11, sql!("list_trigger_functions.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static SEQUENCES: CollDef = CollDef {
    coll: Coll::Sequences,
    label: "Sequences",
    coll_kind: NodeKind::SequenceCollection,
    child_kind: NodeKind::Sequence,
    seg: Seg::Sequence,
    list: &VersionedSql(&[(10, sql!("list_sequences.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static TYPES: CollDef = CollDef {
    coll: Coll::Types,
    label: "Types",
    coll_kind: NodeKind::TypeCollection,
    child_kind: NodeKind::Type,
    seg: Seg::Type,
    list: &VersionedSql(&[(10, sql!("list_types.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static DOMAINS: CollDef = CollDef {
    coll: Coll::Domains,
    label: "Domains",
    coll_kind: NodeKind::DomainCollection,
    child_kind: NodeKind::Domain,
    seg: Seg::Domain,
    list: &VersionedSql(&[(10, sql!("list_domains.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static EXTENSIONS: CollDef = CollDef {
    coll: Coll::Extensions,
    label: "Extensions",
    coll_kind: NodeKind::ExtensionCollection,
    child_kind: NodeKind::Extension,
    seg: Seg::Extension,
    list: &VersionedSql(&[(10, sql!("list_extensions.sql"))]),
    level: Level::Database,
    takes_parent: false,
    min_version: 10,
};

pub(crate) static ROLES: CollDef = CollDef {
    coll: Coll::Roles,
    label: "Login/Group Roles",
    coll_kind: NodeKind::RoleCollection,
    child_kind: NodeKind::Role,
    seg: Seg::Role,
    list: &VersionedSql(&[(10, sql!("list_roles.sql"))]),
    level: Level::Server,
    takes_parent: false,
    min_version: 10,
};

pub(crate) static TABLESPACES: CollDef = CollDef {
    coll: Coll::Tablespaces,
    label: "Tablespaces",
    coll_kind: NodeKind::TablespaceCollection,
    child_kind: NodeKind::Tablespace,
    seg: Seg::Tablespace,
    list: &VersionedSql(&[(10, sql!("list_tablespaces.sql"))]),
    level: Level::Server,
    takes_parent: false,
    min_version: 10,
};

/// Genel motorla yönetilen koleksiyonun tanımı (`Databases`, `Schemas`, `Tables`, `Columns` elle yazılmış modüllerdedir).
pub(crate) fn coll_def(coll: Coll) -> Option<&'static CollDef> {
    Some(match coll {
        Coll::Constraints => &CONSTRAINTS,
        Coll::Indexes => &INDEXES,
        Coll::Triggers => &TRIGGERS,
        Coll::Policies => &POLICIES,
        Coll::Partitions => &PARTITIONS,
        Coll::Views => &VIEWS,
        Coll::MatViews => &MATVIEWS,
        Coll::Functions => &FUNCTIONS,
        Coll::Procedures => &PROCEDURES,
        Coll::TriggerFunctions => &TRIGGER_FUNCTIONS,
        Coll::Sequences => &SEQUENCES,
        Coll::Types => &TYPES,
        Coll::Domains => &DOMAINS,
        Coll::Extensions => &EXTENSIONS,
        Coll::Roles => &ROLES,
        Coll::Tablespaces => &TABLESPACES,
        Coll::Databases | Coll::Schemas | Coll::Tables | Coll::Columns => return None,
    })
}

/// Şema altındaki genel koleksiyonlar (`Tables` hariç), sunucu sürümüne göre.
pub(crate) fn schema_collections(version: PgVersion) -> Vec<&'static CollDef> {
    [&VIEWS, &MATVIEWS, &FUNCTIONS, &PROCEDURES, &TRIGGER_FUNCTIONS, &SEQUENCES, &TYPES, &DOMAINS]
        .into_iter()
        .filter(|d| version.major() >= d.min_version)
        .collect()
}
```

- [ ] **Step 7: `column::list` imzasını değiştir**

`crates/pgt-catalog/src/kinds/column.rs` içinde `list` fonksiyonunu **değiştir** (üst `use` satırları aynı kalır):

```rust
pub(crate) async fn list(cat: &Catalog, server: Uuid, db: u32, rel: &NodeId, rel_oid: u32) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &LIST, p![rel_oid]).await?;
    Ok(rows
        .iter()
        .map(|r| {
            TreeNode::new(&rel.child(Seg::Column(r.get("attnum"))), NodeKind::Column, r.get::<_, String>("name"))
                .with_detail(Some(r.get("type_name")))
                .with_flags(NodeFlags { primary_key: r.get("is_pk"), not_null: r.get("not_null"), ..NodeFlags::default() })
                .leaf()
        })
        .collect())
}
```

Bu değişiklik view / materialized view / partition altındaki kolonların doğru üst kimlikle üretilmesini sağlar.

- [ ] **Step 8: Dağıtıcıları yeniden yaz**

`crates/pgt-catalog/src/kinds/mod.rs` tamamını **değiştir**:

```rust
mod column;
mod database;
pub(crate) mod engine;
pub(crate) mod registry;
mod schema;
mod server;
mod table;

use pgt_core::{Error, Result};

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{Coll, NodeId, NodeKind, Seg, TreeNode};
use crate::version::{PgVersion, VersionedSql};

pub(crate) fn invalid(id: &NodeId) -> Error {
    Error::Invalid(format!("desteklenmeyen düğüm: {id}"))
}

/// Elle yazılmış koleksiyonlar için (`Databases`, `Schemas`, `Tables`, `Columns`) koleksiyon düğümü.
pub(crate) fn collection(parent: &NodeId, coll: Coll, kind: NodeKind, label: &str) -> TreeNode {
    TreeNode::new(&parent.child(Seg::Coll(coll)), kind, label)
}

fn db_req(db: Option<u32>) -> Result<u32> {
    db.ok_or_else(|| Error::Invalid("veritabanı kimliği yok".into()))
}

/// Yolun sondan ikinci parçasının OID'i (koleksiyonun veya yaprağın üst nesnesi).
fn parent_oid(path: &[Seg]) -> Option<u32> {
    path.len().checked_sub(2).and_then(|i| path[i].oid())
}

static IS_PARTITIONED: VersionedSql = VersionedSql(&[(10, "SELECT c.relkind = 'p' AS partitioned FROM pg_class c WHERE c.oid = $1")]);

async fn is_partitioned(cat: &Catalog, server: uuid::Uuid, db: u32, table: u32) -> Result<bool> {
    let rows = query(&cat.mgr, server, pgt_core::connection::Target::Database(db), &IS_PARTITIONED, p![table]).await?;
    Ok(rows.first().map(|r| r.get::<_, bool>("partitioned")).unwrap_or(false))
}

pub(crate) async fn list_children(cat: &Catalog, id: &NodeId) -> Result<Vec<TreeNode>> {
    let (server, path) = match id {
        NodeId::Root => return server::root_children(cat).await,
        NodeId::Group(name) => return server::group_children(cat, name).await,
        NodeId::Node { server, path } => (*server, path.as_slice()),
    };
    if !cat.mgr.is_connected(server) {
        return Err(Error::NotConnected);
    }
    let version = PgVersion(cat.mgr.info(server).await.ok_or(Error::NotConnected)?.version_num);
    let db = match path.first() {
        Some(Seg::Db(d)) => Some(*d),
        _ => None,
    };
    match path.last() {
        None => Ok(vec![
            collection(id, Coll::Databases, NodeKind::DatabaseCollection, "Databases"),
            engine::collection_node(id, &registry::ROLES),
            engine::collection_node(id, &registry::TABLESPACES),
        ]),
        Some(Seg::Coll(coll)) => list_collection(cat, server, db, id, path, *coll, version).await,
        Some(Seg::Db(_)) => Ok(vec![
            collection(id, Coll::Schemas, NodeKind::SchemaCollection, "Schemas"),
            engine::collection_node(id, &registry::EXTENSIONS),
        ]),
        Some(Seg::Schema(_)) => {
            let mut out = vec![collection(id, Coll::Tables, NodeKind::TableCollection, "Tables")];
            out.extend(registry::schema_collections(version).into_iter().map(|d| engine::collection_node(id, d)));
            Ok(out)
        }
        Some(Seg::Table(table)) => {
            let mut out = vec![collection(id, Coll::Columns, NodeKind::ColumnCollection, "Columns")];
            let mut defs = vec![&registry::CONSTRAINTS, &registry::INDEXES, &registry::TRIGGERS, &registry::POLICIES];
            if is_partitioned(cat, server, db_req(db)?, *table).await? {
                defs.push(&registry::PARTITIONS);
            }
            out.extend(defs.into_iter().map(|d| engine::collection_node(id, d)));
            Ok(out)
        }
        Some(Seg::View(_)) => Ok(vec![collection(id, Coll::Columns, NodeKind::ColumnCollection, "Columns")]),
        Some(Seg::MatView(_)) => Ok(vec![
            collection(id, Coll::Columns, NodeKind::ColumnCollection, "Columns"),
            engine::collection_node(id, &registry::INDEXES),
        ]),
        // Yapraklar: kolon, constraint, index, trigger, policy, sequence, fonksiyonlar, tip, domain, extension, rol, tablespace
        Some(_) => Ok(Vec::new()),
    }
}

async fn list_collection(
    cat: &Catalog,
    server: uuid::Uuid,
    db: Option<u32>,
    id: &NodeId,
    path: &[Seg],
    coll: Coll,
    version: PgVersion,
) -> Result<Vec<TreeNode>> {
    let parent = id.parent();
    let above = path.len().checked_sub(2).map(|i| path[i]);
    match (above, coll) {
        (None, Coll::Databases) => database::list(cat, server).await,
        (None, Coll::Roles) => engine::list(cat, server, None, &registry::ROLES, &parent, None).await,
        (None, Coll::Tablespaces) => engine::list(cat, server, None, &registry::TABLESPACES, &parent, None).await,
        (Some(Seg::Db(_)), Coll::Schemas) => schema::list(cat, server, db_req(db)?).await,
        (Some(Seg::Db(_)), Coll::Extensions) => engine::list(cat, server, db, &registry::EXTENSIONS, &parent, None).await,
        (Some(Seg::Schema(_)), Coll::Tables) => {
            let schema = path[path.len() - 2].oid().expect("şema OID'i");
            table::list(cat, server, db_req(db)?, schema).await
        }
        (Some(Seg::Schema(schema)), c) => match registry::coll_def(c) {
            Some(def) if registry::schema_collections(version).iter().any(|d| d.coll == c) => {
                engine::list(cat, server, db, def, &parent, Some(schema)).await
            }
            _ => Err(invalid(id)),
        },
        (Some(Seg::Table(rel) | Seg::View(rel) | Seg::MatView(rel)), Coll::Columns) => {
            column::list(cat, server, db_req(db)?, &parent, rel).await
        }
        (Some(Seg::Table(rel)), c @ (Coll::Constraints | Coll::Indexes | Coll::Triggers | Coll::Policies | Coll::Partitions)) => {
            let def = registry::coll_def(c).ok_or_else(|| invalid(id))?;
            engine::list(cat, server, db, def, &parent, Some(rel)).await
        }
        (Some(Seg::MatView(rel)), Coll::Indexes) => engine::list(cat, server, db, &registry::INDEXES, &parent, Some(rel)).await,
        _ => Err(invalid(id)),
    }
}

pub(crate) async fn properties(cat: &Catalog, id: &NodeId) -> Result<Vec<crate::props::PropertySection>> {
    let (server, path) = match id {
        NodeId::Root | NodeId::Group(_) => return Ok(Vec::new()),
        NodeId::Node { server, path } => (*server, path.as_slice()),
    };
    if path.is_empty() {
        return server::properties(cat, server).await;
    }
    if !cat.mgr.is_connected(server) {
        return Err(Error::NotConnected);
    }
    let db = match path.first() {
        Some(Seg::Db(d)) => Some(*d),
        _ => None,
    };
    match path.last().expect("boş olmayan yol") {
        Seg::Coll(_) => Ok(Vec::new()),
        Seg::Db(d) => database::properties(cat, server, *d).await,
        Seg::Schema(s) => schema::properties(cat, server, db_req(db)?, *s).await,
        Seg::Table(t) => table::properties(cat, server, db_req(db)?, *t).await,
        Seg::Column(c) => {
            let rel = parent_oid(path).ok_or_else(|| invalid(id))?;
            column::properties(cat, server, db_req(db)?, rel, *c).await
        }
        _ => Err(invalid(id)),
    }
}

pub(crate) async fn ddl(cat: &Catalog, id: &NodeId) -> Result<String> {
    let (server, path) = match id {
        NodeId::Root | NodeId::Group(_) => return Ok(String::new()),
        NodeId::Node { server, path } => (*server, path.as_slice()),
    };
    if path.is_empty() || matches!(path.last(), Some(Seg::Coll(_))) {
        return Ok(String::new());
    }
    if !cat.mgr.is_connected(server) {
        return Err(Error::NotConnected);
    }
    let db = match path.first() {
        Some(Seg::Db(d)) => Some(*d),
        _ => None,
    };
    match path.last().expect("boş olmayan yol") {
        Seg::Db(d) => database::ddl(cat, server, *d).await,
        Seg::Schema(s) => schema::ddl(cat, server, db_req(db)?, *s).await,
        Seg::Table(t) => table::ddl(cat, server, db_req(db)?, *t).await,
        Seg::Column(c) => {
            let rel = parent_oid(path).ok_or_else(|| invalid(id))?;
            column::ddl(cat, server, db_req(db)?, rel, *c).await
        }
        _ => Err(invalid(id)),
    }
}

/// Bir nesnenin ACL satırlarını okur (`acl_*.sql`).
pub(crate) async fn acl(
    cat: &Catalog,
    server: uuid::Uuid,
    target: pgt_core::connection::Target,
    sql: &'static VersionedSql,
    oid: u32,
) -> Result<Vec<crate::ddl::AclEntry>> {
    let rows = query(&cat.mgr, server, target, sql, p![oid]).await?;
    Ok(rows.iter().map(crate::ddl::AclEntry::from_row).collect())
}
```

> Not: `column::properties` ve `column::ddl` imzaları Plan 1'de `(cat, server, db, table, attnum)` idi; burada `table` yerine ilişki OID'i (tablo, view, materialized view veya partition) geçirilir — imza aynı kalır, yalnızca üst parça artık yoldan okunur. `Seg::Schema(schema)` desenindeki `schema` değişkeni `u32` olmalıdır (`Seg` `Copy` olduğundan `path[i]` kopyasıdır).

- [ ] **Step 9: Derle ve testleri çalıştır**

Run: `cargo build -p pgt-catalog`
Expected: derlenir (uyarı yok).

Run (PowerShell): `$env:PGT_TEST_URLS = (Get-Content .superpowers/sdd/test-url.txt); cargo test -p pgt-catalog --test listing`
Expected: PASS (Plan 1'den kalan 8 + yeni testler; toplam 17). Hata varsa SQL'i sunucu çıktısına göre düzelt; testlerdeki sıralamalar `COLLATE "C"` sırasıdır.

Run: `cargo test -p pgt-catalog` (aynı env ile) ve `cargo test --workspace`
Expected: PASS — `properties`/`ddl`/`listing` testlerinin hepsi (Plan 1'dekiler dahil) geçer.

Run: `cargo test -p pgt-catalog` **env olmadan**
Expected: PASS (entegrasyon testleri atlanır).

- [ ] **Step 10: Commit**

```bash
git add crates/pgt-catalog
git commit -m "feat(catalog): genel liste motoru ve 16 yeni nesne türünün listelenmesi"
```

---

### Task 3: Yeni türler için Properties

**Files:**
- Modify: `crates/pgt-catalog/src/kinds/engine.rs`, `crates/pgt-catalog/src/kinds/registry.rs`, `crates/pgt-catalog/src/kinds/mod.rs`
- Create (hepsi `crates/pgt-catalog/src/sql/` altında): `props_constraint.sql`, `props_index.sql`, `props_trigger.sql`, `props_policy.sql`, `props_view.sql`, `props_sequence.sql`, `props_function.sql`, `props_type.sql`, `props_extension.sql`, `props_role.sql`, `props_tablespace.sql`
- Create: `crates/pgt-catalog/tests/objects_properties.rs`

**Interfaces:**
- Consumes: Task 2 (`engine::{Level, target}`, `registry`, dağıtıcı), `props::PropertySection`.
- Produces:
  - `engine::properties(cat, server, db: Option<u32>, def: &PropsDef, oid: u32) -> Result<Vec<PropertySection>>`
  - `registry::PropsDef { level: Level, sql: &'static VersionedSql, what: &'static str }` ve `registry::props_def(Seg) -> Option<&'static PropsDef>` (constraint, index, trigger, policy, view, matview, sequence, function, procedure, trigger function, type, domain, extension, role, tablespace için `Some`).
  - Davranış: `Catalog::properties` bu türlerin düğümleri için bölümleri döndürür; yok olan OID için `Error::NotFound`.

**Properties SQL sözleşmesi:** sonuç kolonları `section` (text), `label` (text), `value` (text, NULL olabilir). Satır sırası `v.n` ile belirlenir; ardışık aynı `section` değerleri tek bölümde toplanır; değeri NULL veya boş olan satırlar gösterilmez. Hiç satır yoksa nesne yok demektir (`NotFound`). Etiketler Türkçe, PostgreSQL terimleri İngilizce kalır (Plan 1 ile aynı: `Genel`, `Tanım`, `İstatistik`).

- [ ] **Step 1: Başarısız testleri yaz**

`crates/pgt-catalog/tests/objects_properties.rs` (yeni):

```rust
mod common;

use common::fixtures;
use pgt_catalog::node::{Coll, Seg};
use pgt_catalog::props::PropertySection;
use pgt_core::Error;

fn prop<'a>(sections: &'a [PropertySection], title: &str, label: &str) -> Option<&'a str> {
    sections.iter().find(|s| s.title == title)?.rows.iter().find(|r| r.label == label).map(|r| r.value.as_str())
}

#[tokio::test]
async fn constraint_properties() {
    for fx in fixtures().await {
        let customers = fx.table("app", "customers").await;
        let pk = fx.child_of(&customers, Coll::Constraints, "customers_pkey").await;
        let s = fx.catalog.properties(&pk).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("customers_pkey"));
        assert_eq!(prop(&s, "Genel", "Tablo"), Some("app.customers"));
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("primary key"));
        assert_eq!(prop(&s, "Tanım", "Tanım"), Some("PRIMARY KEY (id)"));
        assert_eq!(prop(&s, "Tanım", "Ertelenebilir"), Some("hayır"));
        assert_eq!(prop(&s, "Tanım", "Doğrulanmış"), Some("evet"));
        assert_eq!(prop(&s, "Genel", "Yorum"), None, "boş değerli satır gösterilmez");
    }
}

#[tokio::test]
async fn index_trigger_and_policy_properties() {
    for fx in fixtures().await {
        let customers = fx.table("app", "customers").await;

        let idx = fx.child_of(&customers, Coll::Indexes, "customers_lower_name_idx").await;
        let s = fx.catalog.properties(&idx).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Tablo"), Some("customers"));
        assert_eq!(prop(&s, "Tanım", "Erişim yöntemi"), Some("btree"));
        assert!(prop(&s, "Tanım", "Tanım").unwrap().contains("CREATE UNIQUE INDEX customers_lower_name_idx ON app.customers"));
        assert_eq!(prop(&s, "Tanım", "Benzersiz"), Some("evet"));
        assert_eq!(prop(&s, "Tanım", "Birincil anahtar"), Some("hayır"));
        assert!(prop(&s, "İstatistik", "Boyut").is_some());

        let trg = fx.child_of(&customers, Coll::Triggers, "customers_touch").await;
        let s = fx.catalog.properties(&trg).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Dokunma"));
        assert_eq!(prop(&s, "Tanım", "Durum"), Some("etkin"));
        assert_eq!(prop(&s, "Tanım", "Fonksiyon"), Some("app.touch_row"));
        assert!(prop(&s, "Tanım", "Tanım").unwrap().contains("BEFORE UPDATE ON app.customers"));

        let pol = fx.child_of(&customers, Coll::Policies, "customers_own").await;
        let s = fx.catalog.properties(&pol).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Komut"), Some("UPDATE"));
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("restrictive"));
        assert_eq!(prop(&s, "Tanım", "Roller"), Some("PUBLIC"));
        assert_eq!(prop(&s, "Tanım", "USING"), Some("id > 0"));
        assert_eq!(prop(&s, "Tanım", "WITH CHECK"), Some("id > 0"));
    }
}

#[tokio::test]
async fn view_and_materialized_view_properties() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let view = fx.child_of(&schema, Coll::Views, "customer_orders").await;
        let s = fx.catalog.properties(&view).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Müşteri siparişleri"));
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("View"));
        assert_eq!(prop(&s, "Tanım", "Seçenekler"), Some("security_barrier=true"));
        assert_eq!(prop(&s, "Tanım", "Doldurulmuş"), None);

        let mv = fx.child_of(&schema, Coll::MatViews, "order_totals").await;
        let s = fx.catalog.properties(&mv).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("Materialized view"));
        assert_eq!(prop(&s, "Tanım", "Doldurulmuş"), Some("evet"));
        assert!(prop(&s, "İstatistik", "Boyut").is_some());
    }
}

#[tokio::test]
async fn sequence_properties() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let seq = fx.child_of(&schema, Coll::Sequences, "invoice_seq").await;
        let s = fx.catalog.properties(&seq).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Fatura numarası"));
        assert_eq!(prop(&s, "Tanım", "Veri tipi"), Some("bigint"));
        assert_eq!(prop(&s, "Tanım", "Başlangıç"), Some("100"));
        assert_eq!(prop(&s, "Tanım", "Artış"), Some("5"));
        assert_eq!(prop(&s, "Tanım", "En küçük"), Some("100"));
        assert_eq!(prop(&s, "Tanım", "En büyük"), Some("100000"));
        assert_eq!(prop(&s, "Tanım", "Önbellek"), Some("10"));
        assert_eq!(prop(&s, "Tanım", "Döngü"), Some("evet"));
        let owned = fx.child_of(&schema, Coll::Sequences, "orders_id_seq").await;
        let s = fx.catalog.properties(&owned).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Sahibi olan kolon"), Some("app.orders.id"));
    }
}

#[tokio::test]
async fn function_properties_resolve_overloads_by_oid() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let tax = fx.child_of(&schema, Coll::Functions, "add_tax(amount numeric, rate numeric)").await;
        let s = fx.catalog.properties(&tax).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("add_tax"));
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("KDV ekler"));
        assert_eq!(prop(&s, "Tanım", "Argümanlar"), Some("amount numeric, rate numeric DEFAULT 0.18"));
        assert_eq!(prop(&s, "Tanım", "Dönüş tipi"), Some("numeric"));
        assert_eq!(prop(&s, "Tanım", "Dil"), Some("sql"));
        assert_eq!(prop(&s, "Tanım", "Değişkenlik"), Some("immutable"));

        let int_fmt = fx.child_of(&schema, Coll::Functions, "fmt(v integer)").await;
        let text_fmt = fx.child_of(&schema, Coll::Functions, "fmt(v text)").await;
        assert_ne!(int_fmt, text_fmt);
        assert_eq!(prop(&fx.catalog.properties(&int_fmt).await.unwrap(), "Tanım", "Argümanlar"), Some("v integer"));
        assert_eq!(prop(&fx.catalog.properties(&text_fmt).await.unwrap(), "Tanım", "Argümanlar"), Some("v text"));

        let trg_fn = fx.child_of(&schema, Coll::TriggerFunctions, "touch_row()").await;
        assert_eq!(prop(&fx.catalog.properties(&trg_fn).await.unwrap(), "Tanım", "Dönüş tipi"), Some("trigger"));
        if fx.major().await >= 11 {
            let proc = fx.child_of(&schema, Coll::Procedures, "archive_orders(older_than date)").await;
            let s = fx.catalog.properties(&proc).await.unwrap();
            assert_eq!(prop(&s, "Genel", "Yorum"), Some("Eski siparişleri siler"));
            assert_eq!(prop(&s, "Tanım", "Argümanlar"), Some("older_than date"));
        }
    }
}

#[tokio::test]
async fn type_and_domain_properties() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let en = fx.child_of(&schema, Coll::Types, "order_status").await;
        let s = fx.catalog.properties(&en).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("enum"));
        assert_eq!(prop(&s, "Tanım", "Etiketler"), Some("'new', 'paid', 'shipped'"));
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Sipariş durumu"));

        let comp = fx.child_of(&schema, Coll::Types, "address").await;
        let s = fx.catalog.properties(&comp).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Alanlar"), Some("street text, city text, zip character varying(10)"));

        let range = fx.child_of(&schema, Coll::Types, "int_span").await;
        assert_eq!(prop(&fx.catalog.properties(&range).await.unwrap(), "Tanım", "Alt tip"), Some("integer"));

        let dom = fx.child_of(&schema, Coll::Domains, "positive_int").await;
        let s = fx.catalog.properties(&dom).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("domain"));
        assert_eq!(prop(&s, "Tanım", "Taban tip"), Some("integer"));
        assert_eq!(prop(&s, "Tanım", "Boş olamaz"), Some("evet"));
        assert_eq!(prop(&s, "Tanım", "Varsayılan"), Some("1"));
    }
}

#[tokio::test]
async fn extension_role_and_tablespace_properties() {
    for fx in fixtures().await {
        let ext = fx.child_of(&fx.db(), Coll::Extensions, "pg_trgm").await;
        let s = fx.catalog.properties(&ext).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Şema"), Some("extra"));
        assert!(prop(&s, "Tanım", "Sürüm").is_some());

        let role = fx.child_of(&fx.server_id(), Coll::Roles, "pgt_app").await;
        let s = fx.catalog.properties(&role).await.unwrap();
        assert_eq!(prop(&s, "Yetkiler", "Oturum açabilir"), Some("evet"));
        assert_eq!(prop(&s, "Yetkiler", "Superuser"), Some("hayır"));
        assert_eq!(prop(&s, "Tanım", "Bağlantı sınırı"), Some("5"));
        assert_eq!(prop(&s, "Tanım", "Üye olduğu roller"), Some("pgt_reader"));

        fx.catalog.set_show_system(true);
        let ts = fx.child_of(&fx.server_id(), Coll::Tablespaces, "pg_default").await;
        fx.catalog.set_show_system(false);
        let s = fx.catalog.properties(&ts).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("pg_default"));
        assert!(prop(&s, "İstatistik", "Boyut").is_some());
    }
}

#[tokio::test]
async fn missing_oid_is_not_found_for_every_kind() {
    for fx in fixtures().await {
        let ghost = 4_000_000_000u32;
        let schema = fx.schema("app").await;
        let customers = fx.table("app", "customers").await;
        let ids = [
            customers.child(Seg::Constraint(ghost)),
            customers.child(Seg::Index(ghost)),
            customers.child(Seg::Trigger(ghost)),
            customers.child(Seg::Policy(ghost)),
            schema.child(Seg::View(ghost)),
            schema.child(Seg::MatView(ghost)),
            schema.child(Seg::Sequence(ghost)),
            schema.child(Seg::Function(ghost)),
            schema.child(Seg::Procedure(ghost)),
            schema.child(Seg::TriggerFunction(ghost)),
            schema.child(Seg::Type(ghost)),
            schema.child(Seg::Domain(ghost)),
            fx.db().child(Seg::Extension(ghost)),
            fx.server_id().child(Seg::Role(ghost)),
            fx.server_id().child(Seg::Tablespace(ghost)),
        ];
        for id in ids {
            assert!(matches!(fx.catalog.properties(&id).await, Err(Error::NotFound(_))), "{id}");
        }
        // koleksiyonlar için Properties boş
        assert!(fx.catalog.properties(&customers.child(Seg::Coll(Coll::Indexes))).await.unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run (PowerShell): `$env:PGT_TEST_URLS = (Get-Content .superpowers/sdd/test-url.txt); cargo test -p pgt-catalog --test objects_properties`
Expected: FAIL — `Err(Invalid)`: yeni türler için `props_def` yok (dağıtıcı `_ => Err(invalid(id))`).

- [ ] **Step 3: Properties SQL dosyalarını yaz** (`crates/pgt-catalog/src/sql/`)

Sorgular PG 18'de doğrulanmıştır. Hepsi aynı kalıbı izler: kaynak satır + `LATERAL (VALUES …) AS v(n, section, label, value)`.

`props_constraint.sql`:

```sql
SELECT v.section, v.label, v.value
FROM pg_constraint k
JOIN pg_class c ON c.oid = k.conrelid
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', k.conname::text),
  (2, 'Genel', 'Tablo', quote_ident(n.nspname) || '.' || quote_ident(c.relname)),
  (3, 'Genel', 'OID', k.oid::text),
  (4, 'Genel', 'Yorum', obj_description(k.oid, 'pg_constraint')),
  (5, 'Tanım', 'Tür', CASE k.contype WHEN 'p' THEN 'primary key' WHEN 'f' THEN 'foreign key' WHEN 'u' THEN 'unique' WHEN 'c' THEN 'check' WHEN 'x' THEN 'exclusion' END),
  (6, 'Tanım', 'Tanım', pg_get_constraintdef(k.oid, true)),
  (7, 'Tanım', 'Ertelenebilir', CASE WHEN k.condeferrable THEN CASE WHEN k.condeferred THEN 'evet (başlangıçta ertelenmiş)' ELSE 'evet' END ELSE 'hayır' END),
  (8, 'Tanım', 'Doğrulanmış', CASE WHEN k.convalidated THEN 'evet' ELSE 'hayır' END)
) AS v(n, section, label, value)
WHERE k.oid = $1
ORDER BY v.n
```

`props_index.sql`:

```sql
SELECT v.section, v.label, v.value
FROM pg_index x
JOIN pg_class i ON i.oid = x.indexrelid
JOIN pg_class t ON t.oid = x.indrelid
JOIN pg_namespace n ON n.oid = i.relnamespace
JOIN pg_am am ON am.oid = i.relam
LEFT JOIN pg_tablespace ts ON ts.oid = i.reltablespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', i.relname::text),
  (2, 'Genel', 'Şema', n.nspname::text),
  (3, 'Genel', 'Tablo', t.relname::text),
  (4, 'Genel', 'OID', i.oid::text),
  (5, 'Genel', 'Yorum', obj_description(i.oid, 'pg_class')),
  (6, 'Tanım', 'Erişim yöntemi', am.amname::text),
  (7, 'Tanım', 'Tanım', pg_get_indexdef(i.oid)),
  (8, 'Tanım', 'Benzersiz', CASE WHEN x.indisunique THEN 'evet' ELSE 'hayır' END),
  (9, 'Tanım', 'Birincil anahtar', CASE WHEN x.indisprimary THEN 'evet' ELSE 'hayır' END),
  (10, 'Tanım', 'Geçerli', CASE WHEN x.indisvalid THEN 'evet' ELSE 'hayır' END),
  (11, 'Tanım', 'Tablespace', COALESCE(ts.spcname::text, 'pg_default')),
  (12, 'Tanım', 'Seçenekler', array_to_string(i.reloptions, ', ')),
  (13, 'İstatistik', 'Boyut', pg_size_pretty(pg_relation_size(i.oid)))
) AS v(n, section, label, value)
WHERE x.indexrelid = $1
ORDER BY v.n
```

`props_trigger.sql`:

```sql
SELECT v.section, v.label, v.value
FROM pg_trigger g
JOIN pg_class c ON c.oid = g.tgrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_proc p ON p.oid = g.tgfoid
JOIN pg_namespace pn ON pn.oid = p.pronamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', g.tgname::text),
  (2, 'Genel', 'Tablo', quote_ident(n.nspname) || '.' || quote_ident(c.relname)),
  (3, 'Genel', 'OID', g.oid::text),
  (4, 'Genel', 'Yorum', obj_description(g.oid, 'pg_trigger')),
  (5, 'Tanım', 'Durum', CASE g.tgenabled WHEN 'O' THEN 'etkin' WHEN 'D' THEN 'devre dışı' WHEN 'R' THEN 'yalnızca replica' WHEN 'A' THEN 'her zaman' END),
  (6, 'Tanım', 'Fonksiyon', quote_ident(pn.nspname) || '.' || quote_ident(p.proname)),
  (7, 'Tanım', 'Tanım', pg_get_triggerdef(g.oid, true))
) AS v(n, section, label, value)
WHERE g.oid = $1
ORDER BY v.n
```

`props_policy.sql`:

```sql
SELECT v.section, v.label, v.value
FROM pg_policy p
JOIN pg_class c ON c.oid = p.polrelid
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', p.polname::text),
  (2, 'Genel', 'Tablo', quote_ident(n.nspname) || '.' || quote_ident(c.relname)),
  (3, 'Genel', 'OID', p.oid::text),
  (4, 'Genel', 'Yorum', obj_description(p.oid, 'pg_policy')),
  (5, 'Tanım', 'Komut', CASE p.polcmd WHEN 'r' THEN 'SELECT' WHEN 'a' THEN 'INSERT' WHEN 'w' THEN 'UPDATE' WHEN 'd' THEN 'DELETE' ELSE 'ALL' END),
  (6, 'Tanım', 'Tür', CASE WHEN p.polpermissive THEN 'permissive' ELSE 'restrictive' END),
  (7, 'Tanım', 'Roller', CASE WHEN p.polroles = '{0}' THEN 'PUBLIC' ELSE (SELECT string_agg(quote_ident(r.rolname), ', ' ORDER BY r.rolname) FROM pg_roles r WHERE r.oid = ANY (p.polroles)) END),
  (8, 'Tanım', 'USING', pg_get_expr(p.polqual, p.polrelid, true)),
  (9, 'Tanım', 'WITH CHECK', pg_get_expr(p.polwithcheck, p.polrelid, true))
) AS v(n, section, label, value)
WHERE p.oid = $1
ORDER BY v.n
```

`props_view.sql` (view ve materialized view için ortak):

```sql
SELECT v.section, v.label, v.value
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', c.relname::text),
  (2, 'Genel', 'Şema', n.nspname::text),
  (3, 'Genel', 'OID', c.oid::text),
  (4, 'Genel', 'Sahip', pg_get_userbyid(c.relowner)::text),
  (5, 'Genel', 'Yorum', obj_description(c.oid, 'pg_class')),
  (6, 'Tanım', 'Tür', CASE c.relkind WHEN 'm' THEN 'Materialized view' ELSE 'View' END),
  (7, 'Tanım', 'Seçenekler', array_to_string(c.reloptions, ', ')),
  (8, 'Tanım', 'Doldurulmuş', CASE WHEN c.relkind = 'm' THEN CASE WHEN c.relispopulated THEN 'evet' ELSE 'hayır' END END),
  (9, 'İstatistik', 'Boyut', CASE WHEN c.relkind = 'm' THEN pg_size_pretty(pg_total_relation_size(c.oid)) END)
) AS v(n, section, label, value)
WHERE c.oid = $1 AND c.relkind IN ('v', 'm')
ORDER BY v.n
```

`props_sequence.sql`:

```sql
SELECT v.section, v.label, v.value
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_sequence s ON s.seqrelid = c.oid
LEFT JOIN LATERAL (
  SELECT quote_ident(tn.nspname) || '.' || quote_ident(tc.relname) || '.' || quote_ident(ta.attname) AS owner_col
  FROM pg_depend d
  JOIN pg_class tc ON tc.oid = d.refobjid
  JOIN pg_namespace tn ON tn.oid = tc.relnamespace
  JOIN pg_attribute ta ON ta.attrelid = d.refobjid AND ta.attnum = d.refobjsubid
  WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.refclassid = 'pg_class'::regclass AND d.deptype = 'a'
  LIMIT 1
) own ON true,
LATERAL (VALUES
  (1, 'Genel', 'Ad', c.relname::text),
  (2, 'Genel', 'Şema', n.nspname::text),
  (3, 'Genel', 'OID', c.oid::text),
  (4, 'Genel', 'Sahip', pg_get_userbyid(c.relowner)::text),
  (5, 'Genel', 'Yorum', obj_description(c.oid, 'pg_class')),
  (6, 'Tanım', 'Veri tipi', format_type(s.seqtypid, NULL)),
  (7, 'Tanım', 'Başlangıç', s.seqstart::text),
  (8, 'Tanım', 'Artış', s.seqincrement::text),
  (9, 'Tanım', 'En küçük', s.seqmin::text),
  (10, 'Tanım', 'En büyük', s.seqmax::text),
  (11, 'Tanım', 'Önbellek', s.seqcache::text),
  (12, 'Tanım', 'Döngü', CASE WHEN s.seqcycle THEN 'evet' ELSE 'hayır' END),
  (13, 'Tanım', 'Sahibi olan kolon', own.owner_col)
) AS v(n, section, label, value)
WHERE c.oid = $1
ORDER BY v.n
```

`props_function.sql` (function / procedure / trigger function için ortak; `proparallel` PG 9.6'dan beri vardır):

```sql
SELECT v.section, v.label, v.value
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
JOIN pg_language l ON l.oid = p.prolang,
LATERAL (VALUES
  (1, 'Genel', 'Ad', p.proname::text),
  (2, 'Genel', 'Şema', n.nspname::text),
  (3, 'Genel', 'OID', p.oid::text),
  (4, 'Genel', 'Sahip', pg_get_userbyid(p.proowner)::text),
  (5, 'Genel', 'Yorum', obj_description(p.oid, 'pg_proc')),
  (6, 'Tanım', 'Argümanlar', pg_get_function_arguments(p.oid)),
  (7, 'Tanım', 'Dönüş tipi', pg_get_function_result(p.oid)),
  (8, 'Tanım', 'Dil', l.lanname::text),
  (9, 'Tanım', 'Değişkenlik', CASE p.provolatile WHEN 'i' THEN 'immutable' WHEN 's' THEN 'stable' ELSE 'volatile' END),
  (10, 'Tanım', 'Güvenlik', CASE WHEN p.prosecdef THEN 'security definer' ELSE 'security invoker' END),
  (11, 'Tanım', 'Paralel', CASE p.proparallel WHEN 's' THEN 'safe' WHEN 'r' THEN 'restricted' ELSE 'unsafe' END)
) AS v(n, section, label, value)
WHERE p.oid = $1
ORDER BY v.n
```

`props_type.sql` (type ve domain için ortak):

```sql
SELECT v.section, v.label, v.value
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', t.typname::text),
  (2, 'Genel', 'Şema', n.nspname::text),
  (3, 'Genel', 'OID', t.oid::text),
  (4, 'Genel', 'Sahip', pg_get_userbyid(t.typowner)::text),
  (5, 'Genel', 'Yorum', obj_description(t.oid, 'pg_type')),
  (6, 'Tanım', 'Tür', CASE t.typtype WHEN 'e' THEN 'enum' WHEN 'c' THEN 'composite' WHEN 'r' THEN 'range' WHEN 'd' THEN 'domain' ELSE 'base' END),
  (7, 'Tanım', 'Etiketler', CASE WHEN t.typtype = 'e' THEN (SELECT string_agg(quote_literal(e.enumlabel), ', ' ORDER BY e.enumsortorder) FROM pg_enum e WHERE e.enumtypid = t.oid) END),
  (8, 'Tanım', 'Alanlar', CASE WHEN t.typtype = 'c' THEN (SELECT string_agg(quote_ident(a.attname) || ' ' || format_type(a.atttypid, a.atttypmod), ', ' ORDER BY a.attnum) FROM pg_attribute a WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped) END),
  (9, 'Tanım', 'Alt tip', CASE WHEN t.typtype = 'r' THEN (SELECT format_type(r.rngsubtype, NULL) FROM pg_range r WHERE r.rngtypid = t.oid) END),
  (10, 'Tanım', 'Taban tip', CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod) END),
  (11, 'Tanım', 'Boş olamaz', CASE WHEN t.typtype = 'd' THEN CASE WHEN t.typnotnull THEN 'evet' ELSE 'hayır' END END),
  (12, 'Tanım', 'Varsayılan', t.typdefault)
) AS v(n, section, label, value)
WHERE t.oid = $1
ORDER BY v.n
```

`props_extension.sql`:

```sql
SELECT v.section, v.label, v.value
FROM pg_extension e
JOIN pg_namespace n ON n.oid = e.extnamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', e.extname::text),
  (2, 'Genel', 'OID', e.oid::text),
  (3, 'Genel', 'Sahip', pg_get_userbyid(e.extowner)::text),
  (4, 'Genel', 'Yorum', obj_description(e.oid, 'pg_extension')),
  (5, 'Tanım', 'Sürüm', e.extversion),
  (6, 'Tanım', 'Şema', n.nspname::text),
  (7, 'Tanım', 'Yeniden konumlandırılabilir', CASE WHEN e.extrelocatable THEN 'evet' ELSE 'hayır' END)
) AS v(n, section, label, value)
WHERE e.oid = $1
ORDER BY v.n
```

`props_role.sql`:

```sql
SELECT v.section, v.label, v.value
FROM pg_roles r,
LATERAL (VALUES
  (1, 'Genel', 'Ad', r.rolname::text),
  (2, 'Genel', 'OID', r.oid::text),
  (3, 'Genel', 'Yorum', shobj_description(r.oid, 'pg_authid')),
  (4, 'Yetkiler', 'Oturum açabilir', CASE WHEN r.rolcanlogin THEN 'evet' ELSE 'hayır' END),
  (5, 'Yetkiler', 'Superuser', CASE WHEN r.rolsuper THEN 'evet' ELSE 'hayır' END),
  (6, 'Yetkiler', 'Veritabanı oluşturabilir', CASE WHEN r.rolcreatedb THEN 'evet' ELSE 'hayır' END),
  (7, 'Yetkiler', 'Rol oluşturabilir', CASE WHEN r.rolcreaterole THEN 'evet' ELSE 'hayır' END),
  (8, 'Yetkiler', 'Kalıtım', CASE WHEN r.rolinherit THEN 'evet' ELSE 'hayır' END),
  (9, 'Yetkiler', 'Replikasyon', CASE WHEN r.rolreplication THEN 'evet' ELSE 'hayır' END),
  (10, 'Yetkiler', 'RLS atlayabilir', CASE WHEN r.rolbypassrls THEN 'evet' ELSE 'hayır' END),
  (11, 'Tanım', 'Bağlantı sınırı', CASE WHEN r.rolconnlimit >= 0 THEN r.rolconnlimit::text END),
  (12, 'Tanım', 'Geçerlilik sonu', to_char(r.rolvaliduntil, 'YYYY-MM-DD HH24:MI:SS')),
  (13, 'Tanım', 'Üye olduğu roller', (SELECT string_agg(quote_ident(g.rolname), ', ' ORDER BY g.rolname) FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid WHERE m.member = r.oid))
) AS v(n, section, label, value)
WHERE r.oid = $1
ORDER BY v.n
```

`props_tablespace.sql`:

```sql
SELECT v.section, v.label, v.value
FROM pg_tablespace t,
LATERAL (VALUES
  (1, 'Genel', 'Ad', t.spcname::text),
  (2, 'Genel', 'OID', t.oid::text),
  (3, 'Genel', 'Sahip', pg_get_userbyid(t.spcowner)::text),
  (4, 'Genel', 'Yorum', shobj_description(t.oid, 'pg_tablespace')),
  (5, 'Tanım', 'Konum', NULLIF(pg_tablespace_location(t.oid), '')),
  (6, 'Tanım', 'Seçenekler', array_to_string(t.spcoptions, ', ')),
  (7, 'İstatistik', 'Boyut', pg_size_pretty(pg_tablespace_size(t.oid)))
) AS v(n, section, label, value)
WHERE t.oid = $1
ORDER BY v.n
```

- [ ] **Step 4: Motora `properties` ekle**

`crates/pgt-catalog/src/kinds/engine.rs` dosyasının `use` satırlarına ekle:

```rust
use crate::props::PropertySection;
use super::registry::PropsDef;
```

ve dosyanın sonuna ekle:

```rust
/// Properties SQL'inin `(section, label, value)` satırlarını bölümlere çevirir.
pub(crate) async fn properties(
    cat: &Catalog,
    server: Uuid,
    db: Option<u32>,
    def: &PropsDef,
    oid: u32,
) -> Result<Vec<PropertySection>> {
    let rows = query(&cat.mgr, server, target(def.level, db)?, def.sql, p![oid]).await?;
    if rows.is_empty() {
        return Err(Error::NotFound(def.what.to_string()));
    }
    let mut sections: Vec<PropertySection> = Vec::new();
    for r in &rows {
        let title: String = r.get("section");
        let label: String = r.get("label");
        let value: Option<String> = r.get("value");
        if sections.last().is_none_or(|s| s.title != title) {
            sections.push(PropertySection::new(title));
        }
        let last = sections.pop().expect("az önce eklendi");
        sections.push(last.row_opt(label, value));
    }
    Ok(sections.into_iter().filter(|s| !s.rows.is_empty()).collect())
}
```

- [ ] **Step 5: Kayıt defterine `PropsDef` ekle**

`crates/pgt-catalog/src/kinds/registry.rs` içinde `use` satırlarına `Level` zaten var; dosyanın sonuna ekle:

```rust
/// Bir nesne türünün Properties sorgusu.
pub(crate) struct PropsDef {
    pub level: Level,
    pub sql: &'static VersionedSql,
    /// `NotFound` iletisinde kullanılan ad.
    pub what: &'static str,
}

macro_rules! props {
    ($what:literal, $level:expr, $file:literal) => {
        PropsDef { level: $level, sql: &VersionedSql(&[(10, sql!($file))]), what: $what }
    };
}

static P_CONSTRAINT: PropsDef = props!("constraint", Level::Database, "props_constraint.sql");
static P_INDEX: PropsDef = props!("index", Level::Database, "props_index.sql");
static P_TRIGGER: PropsDef = props!("trigger", Level::Database, "props_trigger.sql");
static P_POLICY: PropsDef = props!("policy", Level::Database, "props_policy.sql");
static P_VIEW: PropsDef = props!("view", Level::Database, "props_view.sql");
static P_SEQUENCE: PropsDef = props!("sequence", Level::Database, "props_sequence.sql");
static P_FUNCTION: PropsDef = props!("fonksiyon", Level::Database, "props_function.sql");
static P_TYPE: PropsDef = props!("tip", Level::Database, "props_type.sql");
static P_EXTENSION: PropsDef = props!("extension", Level::Database, "props_extension.sql");
static P_ROLE: PropsDef = props!("rol", Level::Server, "props_role.sql");
static P_TABLESPACE: PropsDef = props!("tablespace", Level::Server, "props_tablespace.sql");

/// Genel motorla yönetilen nesne türünün Properties kaydı (`Db`, `Schema`, `Table`, `Column` elle yazılmıştır).
pub(crate) fn props_def(seg: Seg) -> Option<&'static PropsDef> {
    Some(match seg {
        Seg::Constraint(_) => &P_CONSTRAINT,
        Seg::Index(_) => &P_INDEX,
        Seg::Trigger(_) => &P_TRIGGER,
        Seg::Policy(_) => &P_POLICY,
        Seg::View(_) | Seg::MatView(_) => &P_VIEW,
        Seg::Sequence(_) => &P_SEQUENCE,
        Seg::Function(_) | Seg::Procedure(_) | Seg::TriggerFunction(_) => &P_FUNCTION,
        Seg::Type(_) | Seg::Domain(_) => &P_TYPE,
        Seg::Extension(_) => &P_EXTENSION,
        Seg::Role(_) => &P_ROLE,
        Seg::Tablespace(_) => &P_TABLESPACE,
        Seg::Coll(_) | Seg::Db(_) | Seg::Schema(_) | Seg::Table(_) | Seg::Column(_) => return None,
    })
}
```

- [ ] **Step 6: Dağıtıcıya bağla**

`crates/pgt-catalog/src/kinds/mod.rs` içindeki `properties` fonksiyonunda son eşleşme kolunu değiştir (`_ => Err(invalid(id)),`):

```rust
        seg => match registry::props_def(*seg) {
            Some(def) => {
                let oid = seg.oid().ok_or_else(|| invalid(id))?;
                engine::properties(cat, server, db, def, oid).await
            }
            None => Err(invalid(id)),
        },
```

(`match path.last().expect(...)` ifadesi referans döndürdüğü için kol deseni `seg` bir `&Seg`'dir; yukarıdaki `*seg` kullanımı buna göredir.)

- [ ] **Step 7: Testlerin geçtiğini doğrula**

Run (PowerShell, env ile): `cargo test -p pgt-catalog --test objects_properties`
Expected: PASS (7 test).

Run: `cargo test -p pgt-catalog` (env ile), ardından env'siz `cargo test --workspace`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/pgt-catalog
git commit -m "feat(catalog): yeni nesne türleri için Properties"
```

---

### Task 4: Yeni türler için DDL üretimi (genel motor, ACL ve snapshot'lar)

**Files:**
- Modify: `crates/pgt-catalog/src/kinds/engine.rs`, `crates/pgt-catalog/src/kinds/registry.rs`, `crates/pgt-catalog/src/kinds/mod.rs`
- Create (hepsi `crates/pgt-catalog/src/sql/` altında): `ddl_constraint.sql`, `ddl_index.sql`, `ddl_trigger.sql`, `ddl_policy.sql`, `ddl_view.sql`, `ddl_sequence.sql`, `ddl_function.sql`, `ddl_function.pg10.sql`, `ddl_type.sql`, `ddl_domain.sql`, `ddl_extension.sql`, `ddl_role.sql`, `ddl_tablespace.sql`, `acl_proc.sql`, `acl_type.sql`, `acl_tablespace.sql`
- Create: `crates/pgt-catalog/tests/objects_ddl.rs` (+ `tests/snapshots/objects_ddl__*@pg18.snap`)

**Interfaces:**
- Consumes: Task 2–3 (`engine`, `registry`, dağıtıcı), Plan 1 `ddl::{header, join_blocks, grants, AclEntry}`, `kinds::acl`.
- Produces:
  - `engine::ddl(cat, server, db: Option<u32>, def: &DdlDef, oid: u32) -> Result<String>`
  - `registry::DdlDef { level: Level, sql: &'static VersionedSql, what: &'static str }`, `registry::ddl_def(Seg) -> Option<&'static DdlDef>`
  - `Catalog::ddl` bu türler için pgAdmin biçimli DDL üretir; yok olan OID için `NotFound`.

**DDL SQL sözleşmesi (her `ddl_*.sql` tek satır döndürür):**

| Kolon | Anlamı |
|---|---|
| `title` | Başlık etiketi (`Index`, `Table`…): `-- <title>: <qname>` |
| `qname` | Başlıkta görünen ad |
| `drop_stmt` | `-- ` önekiyle yorum olarak yazılan DROP ifadesi (tam, `;` ile) |
| `body` | CREATE ifadesi/ifadeleri, `;\n` ile biter |
| `extra_stmt` | NULL olabilir; `body`'den sonra gelen ek ifade (ör. `OWNED BY`, rol üyelikleri) |
| `owner_stmt` | NULL olabilir; `ALTER … OWNER TO …;\n` |
| `comment_stmt` | NULL olabilir; `COMMENT ON …;\n` |
| `acl_catalog` | NULL olabilir; `relation` \| `proc` \| `type` \| `tablespace` (hangi ACL sorgusu) |
| `acl_kw`, `acl_target` | `GRANT … ON <acl_kw> <acl_target> TO …` için (acl_catalog doluysa dolu) |

Rust bloklarını şu sırayla birleştirir: `body`, `extra_stmt`, `owner_stmt`, `comment_stmt`, GRANT'lar (aralarında bir boş satır). Parola gibi gizli değerler hiçbir `ddl_*.sql` içinde yer almaz.

- [ ] **Step 1: Başarısız testleri yaz**

`crates/pgt-catalog/tests/objects_ddl.rs` (yeni):

```rust
mod common;

use common::fixtures;
use pgt_catalog::node::{Coll, Seg};
use pgt_core::Error;

macro_rules! snapshot {
    ($fx:expr, $name:expr, $ddl:expr) => {{
        let major = $fx.major().await;
        insta::with_settings!({ snapshot_suffix => format!("pg{major}") }, {
            insta::assert_snapshot!($name, $fx.normalize(&$ddl));
        });
    }};
}

#[tokio::test]
async fn table_sub_object_ddl_snapshots() {
    for fx in fixtures().await {
        let customers = fx.table("app", "customers").await;
        for (coll, label, name) in [
            (Coll::Constraints, "customers_pkey", "constraint_pk"),
            (Coll::Constraints, "customers_email_check", "constraint_check"),
            (Coll::Indexes, "customers_lower_name_idx", "index_expr"),
            (Coll::Triggers, "customers_touch", "trigger"),
            (Coll::Policies, "customers_own", "policy_restrictive"),
            (Coll::Policies, "customers_select", "policy_select"),
        ] {
            let id = fx.child_of(&customers, coll, label).await;
            let ddl = fx.catalog.ddl(&id).await.unwrap();
            snapshot!(fx, format!("objects_{name}"), ddl);
        }
        let orders = fx.table("app", "orders").await;
        let idx = fx.child_of(&orders, Coll::Indexes, "orders_customer_idx").await;
        snapshot!(fx, "objects_index_partial", fx.catalog.ddl(&idx).await.unwrap());
    }
}

#[tokio::test]
async fn schema_object_ddl_snapshots() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        for (coll, label, name) in [
            (Coll::Views, "customer_orders", "view"),
            (Coll::Views, "Mixed View", "view_quoted"),
            (Coll::MatViews, "order_totals", "matview"),
            (Coll::Sequences, "invoice_seq", "sequence_custom"),
            (Coll::Sequences, "orders_id_seq", "sequence_owned"),
            (Coll::Functions, "add_tax(amount numeric, rate numeric)", "function"),
            (Coll::TriggerFunctions, "touch_row()", "trigger_function"),
            (Coll::Types, "order_status", "type_enum"),
            (Coll::Types, "address", "type_composite"),
            (Coll::Types, "int_span", "type_range"),
            (Coll::Domains, "positive_int", "domain"),
        ] {
            let id = fx.child_of(&schema, coll, label).await;
            let ddl = fx.catalog.ddl(&id).await.unwrap();
            snapshot!(fx, format!("objects_{name}"), ddl);
        }
    }
}

#[tokio::test]
async fn database_and_server_object_ddl_snapshots() {
    for fx in fixtures().await {
        let ext = fx.child_of(&fx.db(), Coll::Extensions, "pg_trgm").await;
        snapshot!(fx, "objects_extension", fx.catalog.ddl(&ext).await.unwrap());
        let role = fx.child_of(&fx.server_id(), Coll::Roles, "pgt_app").await;
        let role_ddl = fx.catalog.ddl(&role).await.unwrap();
        assert!(!role_ddl.to_lowercase().contains("password"), "rol DDL'inde parola olmamalı:\n{role_ddl}");
        snapshot!(fx, "objects_role", role_ddl);
        fx.catalog.set_show_system(true);
        let ts = fx.child_of(&fx.server_id(), Coll::Tablespaces, "pg_default").await;
        fx.catalog.set_show_system(false);
        snapshot!(fx, "objects_tablespace", fx.catalog.ddl(&ts).await.unwrap());
    }
}

#[tokio::test]
async fn quoted_names_survive_in_ddl() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let view = fx.child_of(&schema, Coll::Views, "Mixed View").await;
        let ddl = fx.catalog.ddl(&view).await.unwrap();
        assert!(ddl.contains("-- View: app.\"Mixed View\""), "{ddl}");
        assert!(ddl.contains("CREATE OR REPLACE VIEW app.\"Mixed View\""), "{ddl}");
        assert!(ddl.contains("\"Çok Kolon\""), "{ddl}");
        assert!(ddl.contains("DROP VIEW IF EXISTS app.\"Mixed View\";"), "{ddl}");
    }
}

#[tokio::test]
async fn overloaded_functions_resolve_by_oid() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let int_fmt = fx.child_of(&schema, Coll::Functions, "fmt(v integer)").await;
        let text_fmt = fx.child_of(&schema, Coll::Functions, "fmt(v text)").await;
        let a = fx.catalog.ddl(&int_fmt).await.unwrap();
        let b = fx.catalog.ddl(&text_fmt).await.unwrap();
        assert!(a.contains("app.fmt(v integer)") && !a.contains("v text"), "{a}");
        assert!(b.contains("app.fmt(v text)") && !b.contains("v integer"), "{b}");
        assert!(a.contains("DROP FUNCTION IF EXISTS app.fmt(v integer);"), "{a}");
    }
}

#[tokio::test]
async fn procedure_ddl_on_pg11_and_newer() {
    for fx in fixtures().await {
        if fx.major().await < 11 {
            continue;
        }
        let schema = fx.schema("app").await;
        let proc = fx.child_of(&schema, Coll::Procedures, "archive_orders(older_than date)").await;
        let ddl = fx.catalog.ddl(&proc).await.unwrap();
        assert!(ddl.contains("-- Procedure: app.archive_orders(older_than date)"), "{ddl}");
        assert!(ddl.contains("CREATE OR REPLACE PROCEDURE app.archive_orders(older_than date)"), "{ddl}");
        assert!(ddl.contains("COMMENT ON PROCEDURE app.archive_orders"), "{ddl}");
    }
}

#[tokio::test]
async fn function_grants_are_listed() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let f = fx.child_of(&schema, Coll::Functions, "add_tax(amount numeric, rate numeric)").await;
        let ddl = fx.catalog.ddl(&f).await.unwrap();
        assert!(ddl.contains("GRANT EXECUTE ON FUNCTION app.add_tax(amount numeric, rate numeric) TO pgt_reader;"), "{ddl}");
    }
}

#[tokio::test]
async fn missing_oid_ddl_is_not_found_and_collections_are_empty() {
    for fx in fixtures().await {
        let ghost = 4_000_000_000u32;
        let schema = fx.schema("app").await;
        let customers = fx.table("app", "customers").await;
        for id in [
            customers.child(Seg::Constraint(ghost)),
            customers.child(Seg::Index(ghost)),
            customers.child(Seg::Trigger(ghost)),
            customers.child(Seg::Policy(ghost)),
            schema.child(Seg::View(ghost)),
            schema.child(Seg::Sequence(ghost)),
            schema.child(Seg::Function(ghost)),
            schema.child(Seg::Type(ghost)),
            schema.child(Seg::Domain(ghost)),
            fx.db().child(Seg::Extension(ghost)),
            fx.server_id().child(Seg::Role(ghost)),
            fx.server_id().child(Seg::Tablespace(ghost)),
        ] {
            assert!(matches!(fx.catalog.ddl(&id).await, Err(Error::NotFound(_))), "{id}");
        }
        assert_eq!(fx.catalog.ddl(&customers.child(Seg::Coll(Coll::Indexes))).await.unwrap(), "");
    }
}
```

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run (PowerShell): `$env:PGT_TEST_URLS = (Get-Content .superpowers/sdd/test-url.txt); cargo test -p pgt-catalog --test objects_ddl`
Expected: FAIL — `Err(Invalid)` (`ddl_def` yok).

- [ ] **Step 3: ACL SQL'lerini yaz** (`crates/pgt-catalog/src/sql/`)

`acl_proc.sql`:

```sql
-- acl_proc.sql
SELECT grantee, privilege, grantable FROM (
  SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee,
         a.privilege_type AS privilege, a.is_grantable AS grantable,
         a.grantee = o.proowner AS is_owner
  FROM pg_proc o, LATERAL aclexplode(o.proacl) a
  WHERE o.oid = $1
) s
ORDER BY is_owner DESC, grantee COLLATE "C", privilege
```

`acl_type.sql`:

```sql
-- acl_type.sql
SELECT grantee, privilege, grantable FROM (
  SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee,
         a.privilege_type AS privilege, a.is_grantable AS grantable,
         a.grantee = o.typowner AS is_owner
  FROM pg_type o, LATERAL aclexplode(o.typacl) a
  WHERE o.oid = $1
) s
ORDER BY is_owner DESC, grantee COLLATE "C", privilege
```

`acl_tablespace.sql`:

```sql
-- acl_tablespace.sql
SELECT grantee, privilege, grantable FROM (
  SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee,
         a.privilege_type AS privilege, a.is_grantable AS grantable,
         a.grantee = o.spcowner AS is_owner
  FROM pg_tablespace o, LATERAL aclexplode(o.spcacl) a
  WHERE o.oid = $1
) s
ORDER BY is_owner DESC, grantee COLLATE "C", privilege
```

- [ ] **Step 4: DDL SQL dosyalarını yaz** (`crates/pgt-catalog/src/sql/`)

Sorgular PG 18'de doğrulanmıştır. Her biri sözleşmedeki 10 kolonu döndürür.

`ddl_constraint.sql`:

```sql
SELECT 'Constraint' AS title,
       quote_ident(k.conname) || ' ON ' || t.qname AS qname,
       format('ALTER TABLE IF EXISTS %s DROP CONSTRAINT IF EXISTS %s;', t.qname, quote_ident(k.conname)) AS drop_stmt,
       format(E'ALTER TABLE IF EXISTS %s\n    ADD CONSTRAINT %s %s;\n', t.qname, quote_ident(k.conname), pg_get_constraintdef(k.oid, true)) AS body,
       NULL::text AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN obj_description(k.oid, 'pg_constraint') IS NOT NULL
            THEN format(E'COMMENT ON CONSTRAINT %s ON %s\n    IS %L;\n', quote_ident(k.conname), t.qname, obj_description(k.oid, 'pg_constraint')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_constraint k
JOIN pg_class c ON c.oid = k.conrelid
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname) t
WHERE k.oid = $1
```

`ddl_index.sql`:

```sql
SELECT 'Index' AS title,
       q.qname,
       'DROP INDEX IF EXISTS ' || q.qname || ';' AS drop_stmt,
       pg_get_indexdef(i.oid) || E';\n' AS body,
       NULL::text AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN obj_description(i.oid, 'pg_class') IS NOT NULL
            THEN format(E'COMMENT ON INDEX %s\n    IS %L;\n', q.qname, obj_description(i.oid, 'pg_class')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_class i
JOIN pg_namespace n ON n.oid = i.relnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(i.relname) AS qname) q
WHERE i.oid = $1 AND i.relkind IN ('i', 'I')
```

`ddl_trigger.sql`:

```sql
SELECT 'Trigger' AS title,
       quote_ident(g.tgname) || ' ON ' || t.qname AS qname,
       format('DROP TRIGGER IF EXISTS %s ON %s;', quote_ident(g.tgname), t.qname) AS drop_stmt,
       pg_get_triggerdef(g.oid, true) || E';\n' AS body,
       CASE g.tgenabled
         WHEN 'D' THEN format(E'ALTER TABLE IF EXISTS %s DISABLE TRIGGER %s;\n', t.qname, quote_ident(g.tgname))
         WHEN 'R' THEN format(E'ALTER TABLE IF EXISTS %s ENABLE REPLICA TRIGGER %s;\n', t.qname, quote_ident(g.tgname))
         WHEN 'A' THEN format(E'ALTER TABLE IF EXISTS %s ENABLE ALWAYS TRIGGER %s;\n', t.qname, quote_ident(g.tgname))
       END AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN obj_description(g.oid, 'pg_trigger') IS NOT NULL
            THEN format(E'COMMENT ON TRIGGER %s ON %s\n    IS %L;\n', quote_ident(g.tgname), t.qname, obj_description(g.oid, 'pg_trigger')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_trigger g
JOIN pg_class c ON c.oid = g.tgrelid
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname) t
WHERE g.oid = $1
```

`ddl_policy.sql`:

```sql
SELECT 'Policy' AS title,
       quote_ident(p.polname) || ' ON ' || t.qname AS qname,
       format('DROP POLICY IF EXISTS %s ON %s;', quote_ident(p.polname), t.qname) AS drop_stmt,
       format(E'CREATE POLICY %s\n    ON %s\n    AS %s\n    FOR %s\n    TO %s',
              quote_ident(p.polname), t.qname,
              CASE WHEN p.polpermissive THEN 'PERMISSIVE' ELSE 'RESTRICTIVE' END,
              CASE p.polcmd WHEN 'r' THEN 'SELECT' WHEN 'a' THEN 'INSERT' WHEN 'w' THEN 'UPDATE' WHEN 'd' THEN 'DELETE' ELSE 'ALL' END,
              CASE WHEN p.polroles = '{0}' THEN 'PUBLIC'
                   ELSE (SELECT string_agg(quote_ident(r.rolname), ', ' ORDER BY r.rolname) FROM pg_roles r WHERE r.oid = ANY (p.polroles)) END)
         || CASE WHEN p.polqual IS NOT NULL THEN E'\n    USING (' || pg_get_expr(p.polqual, p.polrelid, true) || ')' ELSE '' END
         || CASE WHEN p.polwithcheck IS NOT NULL THEN E'\n    WITH CHECK (' || pg_get_expr(p.polwithcheck, p.polrelid, true) || ')' ELSE '' END
         || E';\n' AS body,
       NULL::text AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN obj_description(p.oid, 'pg_policy') IS NOT NULL
            THEN format(E'COMMENT ON POLICY %s ON %s\n    IS %L;\n', quote_ident(p.polname), t.qname, obj_description(p.oid, 'pg_policy')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_policy p
JOIN pg_class c ON c.oid = p.polrelid
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname) t
WHERE p.oid = $1
```

`ddl_view.sql` (view ve materialized view; matview index'leri gövdeye eklenir):

```sql
SELECT CASE c.relkind WHEN 'm' THEN 'Materialized View' ELSE 'View' END AS title,
       q.qname,
       CASE c.relkind WHEN 'm' THEN 'DROP MATERIALIZED VIEW IF EXISTS ' ELSE 'DROP VIEW IF EXISTS ' END || q.qname || ';' AS drop_stmt,
       CASE c.relkind
         WHEN 'm' THEN
           'CREATE MATERIALIZED VIEW IF NOT EXISTS ' || q.qname
           || CASE WHEN c.reloptions IS NOT NULL THEN E'\nWITH (' || array_to_string(c.reloptions, ', ') || ')' ELSE '' END
           || E'\nAS\n' || rtrim(pg_get_viewdef(c.oid, true), E';\n ')
           || CASE WHEN c.relispopulated THEN E'\nWITH DATA;\n' ELSE E'\nWITH NO DATA;\n' END
           || coalesce((SELECT E'\n' || string_agg(pg_get_indexdef(x.indexrelid) || E';\n', E'\n' ORDER BY ic.relname COLLATE "C")
                        FROM pg_index x JOIN pg_class ic ON ic.oid = x.indexrelid WHERE x.indrelid = c.oid), '')
         ELSE
           'CREATE OR REPLACE VIEW ' || q.qname
           || CASE WHEN c.reloptions IS NOT NULL THEN E'\nWITH (' || array_to_string(c.reloptions, ', ') || ')' ELSE '' END
           || E'\nAS\n' || pg_get_viewdef(c.oid, true) || E'\n'
       END AS body,
       NULL::text AS extra_stmt,
       format(E'ALTER %s %s\n    OWNER TO %s;\n', CASE c.relkind WHEN 'm' THEN 'MATERIALIZED VIEW' ELSE 'TABLE' END, q.qname, quote_ident(pg_get_userbyid(c.relowner))) AS owner_stmt,
       CASE WHEN obj_description(c.oid, 'pg_class') IS NOT NULL
            THEN format(E'COMMENT ON %s %s\n    IS %L;\n', CASE c.relkind WHEN 'm' THEN 'MATERIALIZED VIEW' ELSE 'VIEW' END, q.qname, obj_description(c.oid, 'pg_class')) END AS comment_stmt,
       'relation' AS acl_catalog, 'TABLE' AS acl_kw, q.qname AS acl_target
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname) q
WHERE c.oid = $1 AND c.relkind IN ('v', 'm')
```

`ddl_sequence.sql` (`OWNED BY` ayrı ifadedir: tablo oluşturulduktan sonra uygulanmalıdır):

```sql
SELECT 'Sequence' AS title,
       q.qname,
       'DROP SEQUENCE IF EXISTS ' || q.qname || ';' AS drop_stmt,
       format(E'CREATE SEQUENCE IF NOT EXISTS %s\n    AS %s\n    INCREMENT BY %s\n    MINVALUE %s\n    MAXVALUE %s\n    START WITH %s\n    CACHE %s\n    %s;\n',
              q.qname, format_type(s.seqtypid, NULL), s.seqincrement, s.seqmin, s.seqmax, s.seqstart, s.seqcache,
              CASE WHEN s.seqcycle THEN 'CYCLE' ELSE 'NO CYCLE' END) AS body,
       (SELECT format(E'ALTER SEQUENCE %s\n    OWNED BY %s.%s.%s;\n', q.qname, quote_ident(tn.nspname), quote_ident(tc.relname), quote_ident(ta.attname))
        FROM pg_depend d
        JOIN pg_class tc ON tc.oid = d.refobjid
        JOIN pg_namespace tn ON tn.oid = tc.relnamespace
        JOIN pg_attribute ta ON ta.attrelid = d.refobjid AND ta.attnum = d.refobjsubid
        WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.refclassid = 'pg_class'::regclass AND d.deptype = 'a'
        LIMIT 1) AS extra_stmt,
       format(E'ALTER SEQUENCE %s\n    OWNER TO %s;\n', q.qname, quote_ident(pg_get_userbyid(c.relowner))) AS owner_stmt,
       CASE WHEN obj_description(c.oid, 'pg_class') IS NOT NULL
            THEN format(E'COMMENT ON SEQUENCE %s\n    IS %L;\n', q.qname, obj_description(c.oid, 'pg_class')) END AS comment_stmt,
       'relation' AS acl_catalog, 'SEQUENCE' AS acl_kw, q.qname AS acl_target
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_sequence s ON s.seqrelid = c.oid,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname) q
WHERE c.oid = $1
```

`ddl_function.sql` (PG 11+; function / procedure / trigger function):

```sql
SELECT CASE p.prokind WHEN 'p' THEN 'Procedure' ELSE 'Function' END AS title,
       q.target AS qname,
       format('DROP %s IF EXISTS %s;', CASE p.prokind WHEN 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END, q.target) AS drop_stmt,
       rtrim(pg_get_functiondef(p.oid), E'\n') || E';\n' AS body,
       NULL::text AS extra_stmt,
       format(E'ALTER %s %s\n    OWNER TO %s;\n', CASE p.prokind WHEN 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END, q.target, quote_ident(pg_get_userbyid(p.proowner))) AS owner_stmt,
       CASE WHEN obj_description(p.oid, 'pg_proc') IS NOT NULL
            THEN format(E'COMMENT ON %s %s\n    IS %L;\n', CASE p.prokind WHEN 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END, q.target, obj_description(p.oid, 'pg_proc')) END AS comment_stmt,
       'proc' AS acl_catalog, CASE p.prokind WHEN 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END AS acl_kw, q.target AS acl_target
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(p.proname) || '(' || pg_get_function_identity_arguments(p.oid) || ')' AS target) q
WHERE p.oid = $1
```

`ddl_function.pg10.sql` (`prokind` yok; yalnızca fonksiyon):

```sql
SELECT 'Function' AS title,
       q.target AS qname,
       format('DROP FUNCTION IF EXISTS %s;', q.target) AS drop_stmt,
       rtrim(pg_get_functiondef(p.oid), E'\n') || E';\n' AS body,
       NULL::text AS extra_stmt,
       format(E'ALTER FUNCTION %s\n    OWNER TO %s;\n', q.target, quote_ident(pg_get_userbyid(p.proowner))) AS owner_stmt,
       CASE WHEN obj_description(p.oid, 'pg_proc') IS NOT NULL
            THEN format(E'COMMENT ON FUNCTION %s\n    IS %L;\n', q.target, obj_description(p.oid, 'pg_proc')) END AS comment_stmt,
       'proc' AS acl_catalog, 'FUNCTION' AS acl_kw, q.target AS acl_target
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(p.proname) || '(' || pg_get_function_identity_arguments(p.oid) || ')' AS target) q
WHERE p.oid = $1
```

`ddl_type.sql` (enum / composite / range; base type için açıklayıcı yorum):

```sql
SELECT 'Type' AS title,
       q.qname,
       'DROP TYPE IF EXISTS ' || q.qname || ';' AS drop_stmt,
       CASE t.typtype
         WHEN 'e' THEN format(E'CREATE TYPE %s AS ENUM (\n%s\n);\n', q.qname,
                              (SELECT string_agg('    ' || quote_literal(e.enumlabel), E',\n' ORDER BY e.enumsortorder) FROM pg_enum e WHERE e.enumtypid = t.oid))
         WHEN 'c' THEN format(E'CREATE TYPE %s AS (\n%s\n);\n', q.qname,
                              (SELECT string_agg('    ' || quote_ident(a.attname) || ' ' || format_type(a.atttypid, a.atttypmod), E',\n' ORDER BY a.attnum)
                               FROM pg_attribute a WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped))
         WHEN 'r' THEN format(E'CREATE TYPE %s AS RANGE (\n    SUBTYPE = %s\n);\n', q.qname,
                              (SELECT format_type(r.rngsubtype, NULL) FROM pg_range r WHERE r.rngtypid = t.oid))
         ELSE E'-- Temel (base) tiplerin DDL\'i desteklenmez.\n'
       END AS body,
       NULL::text AS extra_stmt,
       format(E'ALTER TYPE %s\n    OWNER TO %s;\n', q.qname, quote_ident(pg_get_userbyid(t.typowner))) AS owner_stmt,
       CASE WHEN obj_description(t.oid, 'pg_type') IS NOT NULL
            THEN format(E'COMMENT ON TYPE %s\n    IS %L;\n', q.qname, obj_description(t.oid, 'pg_type')) END AS comment_stmt,
       'type' AS acl_catalog, 'TYPE' AS acl_kw, q.qname AS acl_target
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(t.typname) AS qname) q
WHERE t.oid = $1 AND t.typtype IN ('e', 'c', 'r', 'b')
```

`ddl_domain.sql`:

```sql
SELECT 'Domain' AS title,
       q.qname,
       'DROP DOMAIN IF EXISTS ' || q.qname || ';' AS drop_stmt,
       'CREATE DOMAIN ' || q.qname || ' AS ' || format_type(t.typbasetype, t.typtypmod)
         || CASE WHEN t.typcollation <> 0 AND t.typcollation <> bt.typcollation
                 THEN ' COLLATE ' || (SELECT quote_ident(cn.nspname) || '.' || quote_ident(co.collname) FROM pg_collation co JOIN pg_namespace cn ON cn.oid = co.collnamespace WHERE co.oid = t.typcollation)
                 ELSE '' END
         || CASE WHEN t.typdefault IS NOT NULL THEN E'\n    DEFAULT ' || t.typdefault ELSE '' END
         || CASE WHEN t.typnotnull THEN E'\n    NOT NULL' ELSE '' END
         || coalesce((SELECT string_agg(E'\n    CONSTRAINT ' || quote_ident(k.conname) || ' ' || pg_get_constraintdef(k.oid, true), '' ORDER BY k.conname COLLATE "C")
                      FROM pg_constraint k WHERE k.contypid = t.oid AND k.contype = 'c'), '')
         || E';\n' AS body,
       NULL::text AS extra_stmt,
       format(E'ALTER DOMAIN %s\n    OWNER TO %s;\n', q.qname, quote_ident(pg_get_userbyid(t.typowner))) AS owner_stmt,
       CASE WHEN obj_description(t.oid, 'pg_type') IS NOT NULL
            THEN format(E'COMMENT ON DOMAIN %s\n    IS %L;\n', q.qname, obj_description(t.oid, 'pg_type')) END AS comment_stmt,
       'type' AS acl_catalog, 'DOMAIN' AS acl_kw, q.qname AS acl_target
FROM pg_type t
JOIN pg_type bt ON bt.oid = t.typbasetype
JOIN pg_namespace n ON n.oid = t.typnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(t.typname) AS qname) q
WHERE t.oid = $1 AND t.typtype = 'd'
```

`ddl_extension.sql`:

```sql
SELECT 'Extension' AS title,
       quote_ident(e.extname) AS qname,
       'DROP EXTENSION IF EXISTS ' || quote_ident(e.extname) || ';' AS drop_stmt,
       format(E'CREATE EXTENSION IF NOT EXISTS %s\n    SCHEMA %s\n    VERSION %s;\n', quote_ident(e.extname), quote_ident(n.nspname), quote_ident(e.extversion)) AS body,
       NULL::text AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN obj_description(e.oid, 'pg_extension') IS NOT NULL
            THEN format(E'COMMENT ON EXTENSION %s\n    IS %L;\n', quote_ident(e.extname), obj_description(e.oid, 'pg_extension')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_extension e
JOIN pg_namespace n ON n.oid = e.extnamespace
WHERE e.oid = $1
```

`ddl_role.sql` (**parola yok**; üyelikler `extra_stmt`):

```sql
SELECT 'Role' AS title,
       quote_ident(r.rolname) AS qname,
       'DROP ROLE IF EXISTS ' || quote_ident(r.rolname) || ';' AS drop_stmt,
       format(E'CREATE ROLE %s WITH\n    %s\n    %s\n    %s\n    %s\n    %s\n    %s\n    %s\n    CONNECTION LIMIT %s%s;\n',
              quote_ident(r.rolname),
              CASE WHEN r.rolcanlogin THEN 'LOGIN' ELSE 'NOLOGIN' END,
              CASE WHEN r.rolsuper THEN 'SUPERUSER' ELSE 'NOSUPERUSER' END,
              CASE WHEN r.rolinherit THEN 'INHERIT' ELSE 'NOINHERIT' END,
              CASE WHEN r.rolcreatedb THEN 'CREATEDB' ELSE 'NOCREATEDB' END,
              CASE WHEN r.rolcreaterole THEN 'CREATEROLE' ELSE 'NOCREATEROLE' END,
              CASE WHEN r.rolreplication THEN 'REPLICATION' ELSE 'NOREPLICATION' END,
              CASE WHEN r.rolbypassrls THEN 'BYPASSRLS' ELSE 'NOBYPASSRLS' END,
              r.rolconnlimit,
              CASE WHEN r.rolvaliduntil IS NOT NULL THEN format(E'\n    VALID UNTIL %L', to_char(r.rolvaliduntil, 'YYYY-MM-DD HH24:MI:SS')) ELSE '' END) AS body,
       (SELECT string_agg(format('GRANT %s TO %s%s;', quote_ident(g.rolname), quote_ident(r.rolname), CASE WHEN m.admin_option THEN ' WITH ADMIN OPTION' ELSE '' END) || E'\n', '' ORDER BY g.rolname COLLATE "C")
        FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid WHERE m.member = r.oid) AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN shobj_description(r.oid, 'pg_authid') IS NOT NULL
            THEN format(E'COMMENT ON ROLE %s\n    IS %L;\n', quote_ident(r.rolname), shobj_description(r.oid, 'pg_authid')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_roles r
WHERE r.oid = $1
```

`ddl_tablespace.sql`:

```sql
SELECT 'Tablespace' AS title,
       quote_ident(t.spcname) AS qname,
       'DROP TABLESPACE IF EXISTS ' || quote_ident(t.spcname) || ';' AS drop_stmt,
       CASE WHEN t.spcname LIKE 'pg\_%' THEN E'-- Yerleşik tablespace; oluşturulamaz.\n'
            ELSE format(E'CREATE TABLESPACE %s\n    OWNER %s\n    LOCATION %L;\n', quote_ident(t.spcname), quote_ident(pg_get_userbyid(t.spcowner)), pg_tablespace_location(t.oid)) END AS body,
       CASE WHEN t.spcoptions IS NOT NULL THEN format(E'ALTER TABLESPACE %s SET (%s);\n', quote_ident(t.spcname), array_to_string(t.spcoptions, ', ')) END AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN shobj_description(t.oid, 'pg_tablespace') IS NOT NULL
            THEN format(E'COMMENT ON TABLESPACE %s\n    IS %L;\n', quote_ident(t.spcname), shobj_description(t.oid, 'pg_tablespace')) END AS comment_stmt,
       'tablespace' AS acl_catalog, 'TABLESPACE' AS acl_kw, quote_ident(t.spcname) AS acl_target
FROM pg_tablespace t
WHERE t.oid = $1
```

- [ ] **Step 5: Motora `ddl` ekle**

`crates/pgt-catalog/src/kinds/engine.rs` içine `use` satırlarına ekle:

```rust
use crate::ddl::{grants, header, join_blocks};
use super::registry::DdlDef;
```

ve dosyanın sonuna ekle:

```rust
static ACL_RELATION: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_relation.sql"))]);
static ACL_PROC: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_proc.sql"))]);
static ACL_TYPE: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_type.sql"))]);
static ACL_TABLESPACE: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_tablespace.sql"))]);

fn acl_sql(catalog: &str) -> Result<&'static VersionedSql> {
    match catalog {
        "relation" => Ok(&ACL_RELATION),
        "proc" => Ok(&ACL_PROC),
        "type" => Ok(&ACL_TYPE),
        "tablespace" => Ok(&ACL_TABLESPACE),
        other => Err(Error::Invalid(format!("bilinmeyen ACL kataloğu: {other}"))),
    }
}

/// DDL SQL'inin tek satırlık sonucunu pgAdmin biçimli metne çevirir.
pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: Option<u32>, def: &DdlDef, oid: u32) -> Result<String> {
    let target = target(def.level, db)?;
    let r = crate::db::query_one(&cat.mgr, server, target, def.sql, p![oid], def.what).await?;
    let text = |c: &str| r.get::<_, Option<String>>(c);
    let acl = match text("acl_catalog") {
        Some(name) => super::acl(cat, server, target, acl_sql(&name)?, oid).await?,
        None => Vec::new(),
    };
    let grants_block = match (text("acl_kw"), text("acl_target")) {
        (Some(kw), Some(target_name)) => grants(&kw, &target_name, &acl),
        _ => String::new(),
    };
    let blocks = vec![
        r.get::<_, String>("body"),
        text("extra_stmt").unwrap_or_default(),
        text("owner_stmt").unwrap_or_default(),
        text("comment_stmt").unwrap_or_default(),
        grants_block,
    ];
    Ok(header(&r.get::<_, String>("title"), &r.get::<_, String>("qname"), &r.get::<_, String>("drop_stmt")) + &join_blocks(blocks))
}
```

- [ ] **Step 6: Kayıt defterine `DdlDef` ekle**

`crates/pgt-catalog/src/kinds/registry.rs` sonuna ekle:

```rust
/// Bir nesne türünün DDL sorgusu.
pub(crate) struct DdlDef {
    pub level: Level,
    pub sql: &'static VersionedSql,
    pub what: &'static str,
}

macro_rules! ddl {
    ($what:literal, $level:expr, $file:literal) => {
        DdlDef { level: $level, sql: &VersionedSql(&[(10, sql!($file))]), what: $what }
    };
}

static D_CONSTRAINT: DdlDef = ddl!("constraint", Level::Database, "ddl_constraint.sql");
static D_INDEX: DdlDef = ddl!("index", Level::Database, "ddl_index.sql");
static D_TRIGGER: DdlDef = ddl!("trigger", Level::Database, "ddl_trigger.sql");
static D_POLICY: DdlDef = ddl!("policy", Level::Database, "ddl_policy.sql");
static D_VIEW: DdlDef = ddl!("view", Level::Database, "ddl_view.sql");
static D_SEQUENCE: DdlDef = ddl!("sequence", Level::Database, "ddl_sequence.sql");
static D_FUNCTION: DdlDef = DdlDef {
    level: Level::Database,
    sql: &VersionedSql(&[(10, sql!("ddl_function.pg10.sql")), (11, sql!("ddl_function.sql"))]),
    what: "fonksiyon",
};
static D_TYPE: DdlDef = ddl!("tip", Level::Database, "ddl_type.sql");
static D_DOMAIN: DdlDef = ddl!("domain", Level::Database, "ddl_domain.sql");
static D_EXTENSION: DdlDef = ddl!("extension", Level::Database, "ddl_extension.sql");
static D_ROLE: DdlDef = ddl!("rol", Level::Server, "ddl_role.sql");
static D_TABLESPACE: DdlDef = ddl!("tablespace", Level::Server, "ddl_tablespace.sql");

/// Genel motorla yönetilen nesne türünün DDL kaydı.
pub(crate) fn ddl_def(seg: Seg) -> Option<&'static DdlDef> {
    Some(match seg {
        Seg::Constraint(_) => &D_CONSTRAINT,
        Seg::Index(_) => &D_INDEX,
        Seg::Trigger(_) => &D_TRIGGER,
        Seg::Policy(_) => &D_POLICY,
        Seg::View(_) | Seg::MatView(_) => &D_VIEW,
        Seg::Sequence(_) => &D_SEQUENCE,
        Seg::Function(_) | Seg::Procedure(_) | Seg::TriggerFunction(_) => &D_FUNCTION,
        Seg::Type(_) => &D_TYPE,
        Seg::Domain(_) => &D_DOMAIN,
        Seg::Extension(_) => &D_EXTENSION,
        Seg::Role(_) => &D_ROLE,
        Seg::Tablespace(_) => &D_TABLESPACE,
        Seg::Coll(_) | Seg::Db(_) | Seg::Schema(_) | Seg::Table(_) | Seg::Column(_) => return None,
    })
}
```

- [ ] **Step 7: Dağıtıcıya bağla**

`crates/pgt-catalog/src/kinds/mod.rs` içindeki `ddl` fonksiyonunda son kolu (`_ => Err(invalid(id)),`) değiştir:

```rust
        seg => match registry::ddl_def(*seg) {
            Some(def) => {
                let oid = seg.oid().ok_or_else(|| invalid(id))?;
                engine::ddl(cat, server, db, def, oid).await
            }
            None => Err(invalid(id)),
        },
```

- [ ] **Step 8: Snapshot'ları üret ve gözden geçir**

Run (PowerShell): `$env:PGT_TEST_URLS = (Get-Content .superpowers/sdd/test-url.txt); $env:INSTA_UPDATE = "new"; cargo test -p pgt-catalog --test objects_ddl`
Expected: `*_snapshots` testleri yeni `.snap.new` dosyalarıyla FAIL; diğerleri PASS.

Her `.snap.new` dosyasını **okuyarak** doğrula (`cargo insta` kurulu değilse `.snap.new` → `.snap` olarak yeniden adlandır ve `assertion_line:` satırını sil; sonra `Remove-Item Env:INSTA_UPDATE`). Beklentiler:
- Başlık `-- <Tür>: <ad>` + `-- DROP …;` satırı; bloklar arasında bir boş satır; dosya sonunda satır sonu.
- `objects_index_expr`: `CREATE UNIQUE INDEX customers_lower_name_idx ON app.customers USING btree (lower((name)::text));`
- `objects_view` / `objects_view_quoted`: `CREATE OR REPLACE VIEW …\nWITH (security_barrier=true)\nAS\n …;` ve `ALTER TABLE app.customer_orders\n    OWNER TO <owner>;`; `Mixed View` için `app."Mixed View"` ve `"Çok Kolon"`.
- `objects_matview`: `WITH DATA;` ve ardından `CREATE UNIQUE INDEX order_totals_pk …;`.
- `objects_sequence_owned`: `ALTER SEQUENCE app.orders_id_seq\n    OWNED BY app.orders.id;`; `objects_sequence_custom`: `CYCLE`, `GRANT USAGE ON SEQUENCE app.invoice_seq TO pgt_reader;`.
- `objects_function`: `CREATE OR REPLACE FUNCTION app.add_tax(amount numeric, rate numeric DEFAULT 0.18)`, `GRANT EXECUTE ON FUNCTION … TO pgt_reader;`.
- `objects_role`: `CREATE ROLE pgt_app WITH`, `CONNECTION LIMIT 5`, `GRANT pgt_reader TO pgt_app;`, **parola yok**.
- `objects_tablespace`: yerleşik tablespace yorum satırı.
- Snapshot'larda `<owner>` ve `pgt_reader`/`pgt_app`/`PUBLIC` dışında gerçek rol adı olmamalı.

Run: `cargo test -p pgt-catalog --test objects_ddl` (INSTA_UPDATE olmadan, env'li)
Expected: PASS (7 test); iki kez çalıştırıp snapshot kararlılığını doğrula.

- [ ] **Step 9: Tüm paketi doğrula**

Run: `cargo test --workspace` (env'li ve env'siz)
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/pgt-catalog
git commit -m "feat(catalog): yeni nesne türleri için DDL üretimi ve snapshot testleri"
```

---

### Task 5: Tam Table DDL (partition, INHERITS, index, trigger, RLS, policy, kolon yetkileri, identity seçenekleri)

**Files:**
- Modify (içerik tamamen değişir): `crates/pgt-catalog/src/sql/table_ddl_head.sql`, `table_ddl_columns.sql`, `table_ddl_columns.pg10.sql`, `table_ddl_constraints.sql`
- Create (`crates/pgt-catalog/src/sql/`): `table_ddl_indexes.sql`, `table_ddl_triggers.sql`, `table_ddl_triggers.pg10.sql`, `table_ddl_policies.sql`, `table_ddl_column_acl.sql`
- Modify: `crates/pgt-catalog/src/ddl.rs` (ColumnDef, column_line + birim testleri), `crates/pgt-catalog/src/kinds/table.rs` (`head`, `ddl`)
- Modify: `crates/pgt-catalog/tests/ddl.rs` (yeni testler; Plan 1'in `ddl_executes_in_scratch_database` testi kaldırılır — Task 7 gidiş-dönüş testi onun yerini alır), `tests/snapshots/ddl__table_*@pg18.snap` (yeniden üretilir) + yeni snapshot'lar

**Interfaces:**
- Consumes: Task 2–4 (dağıtıcı `table::ddl`, `kinds::acl`), Plan 1 `ddl::{ColumnDef, column_line, comment, grants, header, join_blocks, owner}`.
- Produces: `ColumnDef.identity_opts: Option<String>`; `Catalog::ddl` bir tablo (veya partition) için şu blokları üretir: `CREATE TABLE …` (veya `… PARTITION OF …`), `OWNER`, tablo/kolon `COMMENT`'leri, tablo `GRANT`'ları, kolon `GRANT (kolon)`'ları, `ENABLE/FORCE ROW LEVEL SECURITY`, `CREATE POLICY` blokları, kısıta bağlı olmayan `CREATE INDEX`'ler, `CREATE TRIGGER`'lar (+ devre dışı/replica durumları).

**Kurallar:**
- Kolon listesinde yalnızca yerel kolonlar yer alır (`attislocal OR attinhcount = 0`); `INHERITS` ile devralınan kolonlar ve devralınan kısıtlar (`conislocal = false`) yazılmaz. Tek kolon DDL'inde (`ADD COLUMN`) bu filtre uygulanmaz.
- Partition tablolar: `CREATE TABLE IF NOT EXISTS <ad> PARTITION OF <üst>\n    FOR VALUES …` (kolon listesi yok); alt-partition'lı partition'da `PARTITION BY …` eklenir. Üst tablodan gelen index'ler (`pg_inherits` kaydı olanlar) ve PG ≥ 13'te `tgparentid <> 0` olan trigger'lar yazılmaz.
- Identity kolonlar: sequence seçenekleri varsayılandan farklıysa `GENERATED … AS IDENTITY (START WITH … INCREMENT BY … MINVALUE … MAXVALUE … CACHE …[ CYCLE])`.
- Blokların sırası: create, owner, tablo yorumu, kolon yorumları, tablo GRANT'ları, kolon GRANT'ları, RLS, policy'ler, index'ler, trigger'lar.
- Sorgular `tokio::try_join!` ile aynı bağlantıda eş zamanlı (pipeline) çalışır; gecikme tek tur gidiş-dönüşe yakın olur (spec §1.3: DDL < 200 ms).

- [ ] **Step 1: Başarısız birim testlerini yaz (`ddl.rs`)**

`crates/pgt-catalog/src/ddl.rs` içindeki `mod tests` bloğunda `col()` yardımcısını güncelle ve test ekle:

```rust
    fn col() -> ColumnDef {
        ColumnDef {
            name: "c".into(),
            type_name: "integer".into(),
            not_null: false,
            default_expr: None,
            identity: String::new(),
            generated: String::new(),
            collation: None,
            comment: None,
            identity_opts: None,
        }
    }

    #[test]
    fn identity_options_are_appended() {
        let c = ColumnDef { not_null: true, identity: "d".into(), identity_opts: Some("START WITH 10 INCREMENT BY 2".into()), ..col() };
        assert_eq!(column_line(&c), "c integer NOT NULL GENERATED BY DEFAULT AS IDENTITY (START WITH 10 INCREMENT BY 2)");
        let a = ColumnDef { identity: "a".into(), identity_opts: Some("CACHE 5".into()), ..col() };
        assert_eq!(column_line(&a), "c integer GENERATED ALWAYS AS IDENTITY (CACHE 5)");
    }
```

(Mevcut `col()` tanımını yukarıdakiyle **değiştir**; `column_lines` testi aynen kalır.)

Run: `cargo test -p pgt-catalog --lib ddl`
Expected: FAIL — `ColumnDef` alanı `identity_opts` yok (derleme hatası).

- [ ] **Step 2: `ColumnDef` ve `column_line`'ı güncelle (`ddl.rs`)**

`ColumnDef` struct'ına alan ekle ve `from_row`'a oku:

```rust
pub(crate) struct ColumnDef {
    pub name: String,
    pub type_name: String,
    pub not_null: bool,
    pub default_expr: Option<String>,
    /// `pg_attribute.attidentity`: '' | 'a' | 'd'
    pub identity: String,
    /// `pg_attribute.attgenerated`: '' | 's' | 'v'
    pub generated: String,
    pub collation: Option<String>,
    /// `quote_literal` uygulanmış yorum
    pub comment: Option<String>,
    /// Varsayılandan farklı identity sequence seçenekleri, ör. `START WITH 10 INCREMENT BY 2 …`
    pub identity_opts: Option<String>,
}

impl ColumnDef {
    pub(crate) fn from_row(r: &Row) -> Self {
        Self {
            name: r.get("name"),
            type_name: r.get("type_name"),
            not_null: r.get("not_null"),
            default_expr: r.get("default_expr"),
            identity: r.get("identity"),
            generated: r.get("generated"),
            collation: r.get("collation"),
            comment: r.get("comment"),
            identity_opts: r.get("identity_opts"),
        }
    }
}
```

`column_line` içindeki identity kollarını değiştir:

```rust
    let identity_suffix = c.identity_opts.as_deref().map(|o| format!(" ({o})")).unwrap_or_default();
    match (c.generated.as_str(), c.identity.as_str(), &c.default_expr) {
        ("s", _, Some(expr)) => line.push_str(&format!(" GENERATED ALWAYS AS ({expr}) STORED")),
        ("v", _, Some(expr)) => line.push_str(&format!(" GENERATED ALWAYS AS ({expr}) VIRTUAL")),
        (_, "a", _) => line.push_str(&format!(" GENERATED ALWAYS AS IDENTITY{identity_suffix}")),
        (_, "d", _) => line.push_str(&format!(" GENERATED BY DEFAULT AS IDENTITY{identity_suffix}")),
        (_, _, Some(expr)) => line.push_str(&format!(" DEFAULT {expr}")),
        _ => {}
    }
```

Run: `cargo test -p pgt-catalog --lib ddl`
Expected: derleme hatası değil, ama `column::` / `table::` SQL'leri `identity_opts` kolonunu henüz döndürmediği için entegrasyon testleri (Step 5'te) kırılır; birim testler PASS.

- [ ] **Step 3: SQL dosyalarını yaz** (`crates/pgt-catalog/src/sql/`)

Sorgular PG 18'de doğrulanmıştır.

`table_ddl_head.sql` (**değiştir**):

```sql
SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname,
       quote_ident(pg_get_userbyid(c.relowner)) AS owner,
       CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END AS partkey,
       c.relpersistence = 'u' AS unlogged,
       array_to_string(c.reloptions, ', ') AS options,
       quote_ident(t.spcname) AS tablespace,
       quote_literal(obj_description(c.oid, 'pg_class')) AS comment,
       CASE WHEN c.relispartition
            THEN (SELECT quote_ident(pn.nspname) || '.' || quote_ident(pc.relname)
                  FROM pg_inherits i JOIN pg_class pc ON pc.oid = i.inhparent JOIN pg_namespace pn ON pn.oid = pc.relnamespace
                  WHERE i.inhrelid = c.oid) END AS partition_of,
       CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid) END AS partition_bound,
       CASE WHEN NOT c.relispartition
            THEN (SELECT string_agg(quote_ident(pn.nspname) || '.' || quote_ident(pc.relname), ', ' ORDER BY i.inhseqno)
                  FROM pg_inherits i JOIN pg_class pc ON pc.oid = i.inhparent JOIN pg_namespace pn ON pn.oid = pc.relnamespace
                  WHERE i.inhrelid = c.oid) END AS inherits,
       c.relrowsecurity AS rls_enabled,
       c.relforcerowsecurity AS rls_forced
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_tablespace t ON t.oid = c.reltablespace
WHERE c.oid = $1 AND c.relkind IN ('r', 'p')
```

`table_ddl_columns.sql` (**değiştir**; PG 12+):

```sql
SELECT quote_ident(a.attname) AS name, format_type(a.atttypid, a.atttypmod) AS type_name, a.attnotnull AS not_null,
       pg_get_expr(d.adbin, d.adrelid) AS default_expr,
       a.attidentity::text AS identity, a.attgenerated::text AS generated,
       CASE WHEN a.attcollation <> 0 AND a.attcollation <> t.typcollation
            THEN quote_ident(cn.nspname) || '.' || quote_ident(co.collname) END AS collation,
       quote_literal(col_description(a.attrelid, a.attnum)) AS comment,
       CASE WHEN a.attidentity <> '' AND (s.seqstart <> 1 OR s.seqincrement <> 1 OR s.seqmin <> 1 OR s.seqcache <> 1 OR s.seqcycle
                OR s.seqmax <> CASE s.seqtypid WHEN 'int2'::regtype THEN 32767 WHEN 'int4'::regtype THEN 2147483647 ELSE 9223372036854775807 END)
            THEN format('START WITH %s INCREMENT BY %s MINVALUE %s MAXVALUE %s CACHE %s%s', s.seqstart, s.seqincrement, s.seqmin, s.seqmax, s.seqcache,
                        CASE WHEN s.seqcycle THEN ' CYCLE' ELSE '' END) END AS identity_opts
FROM pg_attribute a
JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_collation co ON co.oid = a.attcollation
LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace
LEFT JOIN pg_depend dep ON dep.classid = 'pg_class'::regclass AND dep.refobjid = a.attrelid AND dep.refobjsubid = a.attnum
                       AND dep.refclassid = 'pg_class'::regclass AND dep.deptype = 'i' AND a.attidentity <> ''
LEFT JOIN pg_sequence s ON s.seqrelid = dep.objid
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
  AND ($2::int2 IS NULL OR a.attnum = $2::int2)
  AND (a.attislocal OR a.attinhcount = 0 OR $2::int2 IS NOT NULL)
ORDER BY a.attnum
```

`table_ddl_columns.pg10.sql` (**değiştir**; PG 10–11: `attgenerated` yok — yukarıdakiyle aynı, tek fark `generated` kolonu):

```sql
SELECT quote_ident(a.attname) AS name, format_type(a.atttypid, a.atttypmod) AS type_name, a.attnotnull AS not_null,
       pg_get_expr(d.adbin, d.adrelid) AS default_expr,
       a.attidentity::text AS identity, ''::text AS generated,
       CASE WHEN a.attcollation <> 0 AND a.attcollation <> t.typcollation
            THEN quote_ident(cn.nspname) || '.' || quote_ident(co.collname) END AS collation,
       quote_literal(col_description(a.attrelid, a.attnum)) AS comment,
       CASE WHEN a.attidentity <> '' AND (s.seqstart <> 1 OR s.seqincrement <> 1 OR s.seqmin <> 1 OR s.seqcache <> 1 OR s.seqcycle
                OR s.seqmax <> CASE s.seqtypid WHEN 'int2'::regtype THEN 32767 WHEN 'int4'::regtype THEN 2147483647 ELSE 9223372036854775807 END)
            THEN format('START WITH %s INCREMENT BY %s MINVALUE %s MAXVALUE %s CACHE %s%s', s.seqstart, s.seqincrement, s.seqmin, s.seqmax, s.seqcache,
                        CASE WHEN s.seqcycle THEN ' CYCLE' ELSE '' END) END AS identity_opts
FROM pg_attribute a
JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_collation co ON co.oid = a.attcollation
LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace
LEFT JOIN pg_depend dep ON dep.classid = 'pg_class'::regclass AND dep.refobjid = a.attrelid AND dep.refobjsubid = a.attnum
                       AND dep.refclassid = 'pg_class'::regclass AND dep.deptype = 'i' AND a.attidentity <> ''
LEFT JOIN pg_sequence s ON s.seqrelid = dep.objid
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
  AND ($2::int2 IS NULL OR a.attnum = $2::int2)
  AND (a.attislocal OR a.attinhcount = 0 OR $2::int2 IS NOT NULL)
ORDER BY a.attnum
```

`table_ddl_constraints.sql` (**değiştir**; yalnızca yerel kısıtlar):

```sql
SELECT quote_ident(k.conname) AS name, pg_get_constraintdef(k.oid, true) AS def
FROM pg_constraint k
WHERE k.conrelid = $1 AND k.contype IN ('p', 'u', 'c', 'x', 'f') AND k.conislocal
ORDER BY CASE k.contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'c' THEN 2 WHEN 'x' THEN 3 ELSE 4 END, k.conname COLLATE "C"
```

`table_ddl_indexes.sql` (kısıta bağlı olmayan, üst index'ten türememiş index'ler):

```sql
SELECT pg_get_indexdef(x.indexrelid) || ';' AS def
FROM pg_index x
JOIN pg_class ic ON ic.oid = x.indexrelid
WHERE x.indrelid = $1
  AND NOT EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conindid = x.indexrelid AND k.contype IN ('p', 'u', 'x'))
  AND NOT EXISTS (SELECT 1 FROM pg_inherits ih WHERE ih.inhrelid = x.indexrelid)
ORDER BY ic.relname COLLATE "C"
```

`table_ddl_triggers.sql` (PG 13+; `tgparentid` ile üstten klonlananlar hariç):

```sql
SELECT pg_get_triggerdef(t.oid, true) || ';' AS def,
       CASE t.tgenabled WHEN 'D' THEN 'DISABLE' WHEN 'R' THEN 'ENABLE REPLICA' WHEN 'A' THEN 'ENABLE ALWAYS' END AS state,
       quote_ident(t.tgname) AS name
FROM pg_trigger t
WHERE t.tgrelid = $1 AND NOT t.tgisinternal AND t.tgparentid = 0
ORDER BY t.tgname COLLATE "C"
```

`table_ddl_triggers.pg10.sql` (PG 10–12):

```sql
SELECT pg_get_triggerdef(t.oid, true) || ';' AS def,
       CASE t.tgenabled WHEN 'D' THEN 'DISABLE' WHEN 'R' THEN 'ENABLE REPLICA' WHEN 'A' THEN 'ENABLE ALWAYS' END AS state,
       quote_ident(t.tgname) AS name
FROM pg_trigger t
WHERE t.tgrelid = $1 AND NOT t.tgisinternal
ORDER BY t.tgname COLLATE "C"
```

`table_ddl_policies.sql`:

```sql
SELECT format(E'CREATE POLICY %s\n    ON %s\n    AS %s\n    FOR %s\n    TO %s',
              quote_ident(p.polname), quote_ident(n.nspname) || '.' || quote_ident(c.relname),
              CASE WHEN p.polpermissive THEN 'PERMISSIVE' ELSE 'RESTRICTIVE' END,
              CASE p.polcmd WHEN 'r' THEN 'SELECT' WHEN 'a' THEN 'INSERT' WHEN 'w' THEN 'UPDATE' WHEN 'd' THEN 'DELETE' ELSE 'ALL' END,
              CASE WHEN p.polroles = '{0}' THEN 'PUBLIC'
                   ELSE (SELECT string_agg(quote_ident(r.rolname), ', ' ORDER BY r.rolname) FROM pg_roles r WHERE r.oid = ANY (p.polroles)) END)
         || CASE WHEN p.polqual IS NOT NULL THEN E'\n    USING (' || pg_get_expr(p.polqual, p.polrelid, true) || ')' ELSE '' END
         || CASE WHEN p.polwithcheck IS NOT NULL THEN E'\n    WITH CHECK (' || pg_get_expr(p.polwithcheck, p.polrelid, true) || ')' ELSE '' END
         || ';' AS def
FROM pg_policy p
JOIN pg_class c ON c.oid = p.polrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE p.polrelid = $1
ORDER BY p.polname COLLATE "C"
```

`table_ddl_column_acl.sql` (kolon başına yetkiler; `GRANT SELECT (email) ON TABLE … TO …;`):

```sql
SELECT stmt FROM (
  SELECT format('GRANT %s ON TABLE %s TO %s%s;',
                string_agg(a2.privilege_type || ' (' || quote_ident(a.attname) || ')', ', ' ORDER BY a.attnum, a2.privilege_type),
                quote_ident(n.nspname) || '.' || quote_ident(c.relname),
                CASE WHEN a2.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a2.grantee)) END,
                CASE WHEN a2.is_grantable THEN ' WITH GRANT OPTION' ELSE '' END) AS stmt
  FROM pg_attribute a
  JOIN pg_class c ON c.oid = a.attrelid
  JOIN pg_namespace n ON n.oid = c.relnamespace,
  LATERAL aclexplode(a.attacl) a2
  WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped AND a.attacl IS NOT NULL
  GROUP BY n.nspname, c.relname, a2.grantee, a2.is_grantable
) x
ORDER BY stmt COLLATE "C"
```

- [ ] **Step 4: Başarısız entegrasyon testlerini yaz**

`crates/pgt-catalog/tests/ddl.rs`:

(a) `ddl_executes_in_scratch_database` testini **sil** (gidiş-dönüş testi Task 7'de bunu kapsar). Artık kullanılmayan `use pgt_core::testing::admin_client;` satırını da kaldır.

(b) Dosyanın sonuna ekle:

```rust
#[tokio::test]
async fn customers_ddl_has_rls_policies_indexes_triggers_and_column_grants() {
    for fx in fixtures().await {
        let ddl = fx.catalog.ddl(&fx.table("app", "customers").await).await.unwrap();
        for part in [
            "ALTER TABLE IF EXISTS app.customers ENABLE ROW LEVEL SECURITY;",
            "CREATE POLICY customers_own\n    ON app.customers\n    AS RESTRICTIVE\n    FOR UPDATE\n    TO PUBLIC\n    USING (id > 0)\n    WITH CHECK (id > 0);",
            "CREATE POLICY customers_select\n    ON app.customers\n    AS PERMISSIVE\n    FOR SELECT\n    TO pgt_reader\n    USING (true);",
            "CREATE UNIQUE INDEX customers_lower_name_idx ON app.customers USING btree (lower((name)::text));",
            "CREATE TRIGGER customers_touch BEFORE UPDATE ON app.customers FOR EACH ROW",
            "GRANT SELECT (email) ON TABLE app.customers TO pgt_app;",
        ] {
            assert!(ddl.contains(part), "'{part}' eksik:\n{ddl}");
        }
        // kısıta bağlı index'ler (customers_pkey, customers_email_key) ayrıca yazılmaz
        assert!(!ddl.contains("CREATE UNIQUE INDEX customers_pkey"), "{ddl}");
        assert!(!ddl.contains("CREATE UNIQUE INDEX customers_email_key"), "{ddl}");
    }
}

#[tokio::test]
async fn inherited_table_lists_only_local_columns() {
    for fx in fixtures().await {
        let ddl = fx.catalog.ddl(&fx.table("extra", "audit_child").await).await.unwrap();
        assert!(ddl.contains("extra_col integer"), "{ddl}");
        assert!(!ddl.contains("note text") && !ddl.contains("id bigint"), "devralınan kolonlar yazılmamalı:\n{ddl}");
        assert!(ddl.contains("INHERITS (extra.audit_base)"), "{ddl}");
    }
}

#[tokio::test]
async fn identity_options_are_kept() {
    for fx in fixtures().await {
        let ddl = fx.catalog.ddl(&fx.table("extra", "tickets").await).await.unwrap();
        assert!(
            ddl.contains("id integer NOT NULL GENERATED BY DEFAULT AS IDENTITY (START WITH 10 INCREMENT BY 2 MINVALUE 1 MAXVALUE 2147483647 CACHE 1)"),
            "{ddl}"
        );
        // varsayılan seçenekli identity kolon (customers.id) seçenek yazmaz
        let customers = fx.catalog.ddl(&fx.table("app", "customers").await).await.unwrap();
        assert!(customers.contains("id bigint NOT NULL GENERATED ALWAYS AS IDENTITY,"), "{customers}");
    }
}

#[tokio::test]
async fn partitions_use_partition_of_and_skip_inherited_indexes() {
    for fx in fixtures().await {
        let events = fx.table("app", "events").await;
        let parts = fx.nodes_of(&events, Coll::Partitions).await;
        let part = NodeId::parse(&parts[0].id).unwrap();
        let ddl = fx.catalog.ddl(&part).await.unwrap();
        assert!(
            ddl.contains("CREATE TABLE IF NOT EXISTS app.events_2026 PARTITION OF app.events\n    FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');"),
            "{ddl}"
        );
        assert!(!ddl.contains("CREATE INDEX"), "üst tablodan gelen index'ler yazılmamalı:\n{ddl}");
        if fx.major().await >= 11 {
            let parent = fx.catalog.ddl(&events).await.unwrap();
            assert!(parent.contains("CREATE INDEX events_day_idx ON ONLY app.events"), "{parent}");
            let hits = fx.table("extra", "hits").await;
            let hash_parts = fx.nodes_of(&hits, Coll::Partitions).await;
            assert_eq!(hash_parts.len(), 2);
            let hash_ddl = fx.catalog.ddl(&NodeId::parse(&hash_parts[0].id).unwrap()).await.unwrap();
            assert!(hash_ddl.contains("PARTITION OF extra.hits\n    FOR VALUES WITH (modulus 2, remainder 0);"), "{hash_ddl}");
        }
    }
}

#[tokio::test]
async fn generated_columns_on_pg12_and_newer() {
    for fx in fixtures().await {
        if fx.major().await < 12 {
            continue;
        }
        let ddl = fx.catalog.ddl(&fx.table("extra", "line_items").await).await.unwrap();
        assert!(ddl.contains("total numeric(12,2) GENERATED ALWAYS AS ((qty * price)) STORED"), "{ddl}");
    }
}

#[tokio::test]
async fn full_table_ddl_snapshots_for_new_tables() {
    for fx in fixtures().await {
        for (schema, table) in [("extra", "audit_child"), ("extra", "tickets")] {
            let ddl = fx.catalog.ddl(&fx.table(schema, table).await).await.unwrap();
            snapshot!(fx, format!("table_{schema}_{table}"), ddl);
        }
    }
}
```

Dosyanın başındaki `use` satırları şunları içermeli: `use pgt_catalog::node::{Coll, NodeId, Seg};` (zaten `Coll`/`Seg` varsa `NodeId`'yi ekle).

- [ ] **Step 5: Testlerin başarısız olduğunu doğrula**

Run (PowerShell): `$env:PGT_TEST_URLS = (Get-Content .superpowers/sdd/test-url.txt); cargo test -p pgt-catalog --test ddl`
Expected: FAIL — `identity_opts` kolonu yok / RLS-policy-index bloğu yok (yeni testler), mevcut snapshot'lar uyuşmaz.

- [ ] **Step 6: `table.rs` içindeki `head` ve `ddl`'i yeniden yaz**

`crates/pgt-catalog/src/kinds/table.rs` içinde `static HEAD`'den dosya sonuna kadar olan bölümü (`HEAD`, `COLUMNS`, `CONSTRAINTS`, `ACL` static'leri, `TableHead`, `head`, `ddl`) **değiştir**:

```rust
static HEAD: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_head.sql"))]);
pub(crate) static COLUMNS: VersionedSql = VersionedSql(&[
    (10, include_str!("../sql/table_ddl_columns.pg10.sql")),
    (12, include_str!("../sql/table_ddl_columns.sql")),
]);
static CONSTRAINTS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_constraints.sql"))]);
static ACL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_relation.sql"))]);
static INDEXES: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_indexes.sql"))]);
static TRIGGERS: VersionedSql = VersionedSql(&[
    (10, include_str!("../sql/table_ddl_triggers.pg10.sql")),
    (13, include_str!("../sql/table_ddl_triggers.sql")),
]);
static POLICIES: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_policies.sql"))]);
static COLUMN_ACL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_column_acl.sql"))]);

pub(crate) struct TableHead {
    pub qname: String,
    pub owner: String,
    pub partkey: Option<String>,
    pub unlogged: bool,
    pub options: Option<String>,
    pub tablespace: Option<String>,
    pub comment: Option<String>,
    pub partition_of: Option<String>,
    pub partition_bound: Option<String>,
    pub inherits: Option<String>,
    pub rls_enabled: bool,
    pub rls_forced: bool,
}

pub(crate) async fn head(cat: &Catalog, server: Uuid, db: u32, table: u32) -> Result<TableHead> {
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &HEAD, p![table], "tablo").await?;
    Ok(TableHead {
        qname: r.get("qname"),
        owner: r.get("owner"),
        partkey: r.get("partkey"),
        unlogged: r.get("unlogged"),
        options: r.get("options"),
        tablespace: r.get("tablespace"),
        comment: r.get("comment"),
        partition_of: r.get("partition_of"),
        partition_bound: r.get("partition_bound"),
        inherits: r.get("inherits"),
        rls_enabled: r.get("rls_enabled"),
        rls_forced: r.get("rls_forced"),
    })
}

pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: u32, table: u32) -> Result<String> {
    use crate::ddl::{ColumnDef, column_line, comment, grants, header, join_blocks, owner};
    let target = Target::Database(db);
    let q = |sql: &'static VersionedSql, params: crate::db::Params| query(&cat.mgr, server, target, sql, params);
    // Aynı bağlantıda eş zamanlı (pipeline) çalışır: tek tur gidiş-dönüşe yakın süre.
    let (h, col_rows, constraints, acl, indexes, triggers, policies, col_acl) = tokio::try_join!(
        head(cat, server, db, table),
        q(&COLUMNS, p![table, None::<i16>]),
        q(&CONSTRAINTS, p![table]),
        super::acl(cat, server, target, &ACL, table),
        q(&INDEXES, p![table]),
        q(&TRIGGERS, p![table]),
        q(&POLICIES, p![table]),
        q(&COLUMN_ACL, p![table]),
    )?;
    let cols: Vec<ColumnDef> = col_rows.iter().map(ColumnDef::from_row).collect();

    let unlogged = if h.unlogged { "UNLOGGED " } else { "" };
    let mut create = match (&h.partition_of, &h.partition_bound) {
        (Some(parent), Some(bound)) => {
            format!("CREATE {unlogged}TABLE IF NOT EXISTS {} PARTITION OF {parent}\n    {bound}", h.qname)
        }
        _ => {
            let mut lines: Vec<String> = cols.iter().map(column_line).collect();
            lines.extend(constraints.iter().map(|k| format!("CONSTRAINT {} {}", k.get::<_, String>("name"), k.get::<_, String>("def"))));
            let body = lines.iter().map(|l| format!("    {l}")).collect::<Vec<_>>().join(",\n");
            let mut s = format!("CREATE {unlogged}TABLE IF NOT EXISTS {}\n(\n{body}\n)", h.qname);
            if let Some(parents) = &h.inherits {
                s.push_str(&format!("\nINHERITS ({parents})"));
            }
            s
        }
    };
    if let Some(key) = &h.partkey {
        create.push_str(&format!("\nPARTITION BY {key}"));
    }
    if let Some(options) = h.options.as_deref().filter(|o| !o.is_empty()) {
        create.push_str(&format!("\nWITH ({options})"));
    }
    if let Some(ts) = &h.tablespace {
        create.push_str(&format!("\nTABLESPACE {ts}"));
    }
    create.push_str(";\n");

    let mut blocks = vec![create, owner("TABLE IF EXISTS", &h.qname, &h.owner), comment("TABLE", &h.qname, h.comment.as_deref())];
    for c in &cols {
        blocks.push(comment("COLUMN", &format!("{}.{}", h.qname, c.name), c.comment.as_deref()));
    }
    blocks.push(grants("TABLE", &h.qname, &acl));
    blocks.push(col_acl.iter().map(|r| format!("{}\n", r.get::<_, String>("stmt"))).collect());
    if h.rls_enabled {
        blocks.push(format!("ALTER TABLE IF EXISTS {} ENABLE ROW LEVEL SECURITY;\n", h.qname));
    }
    if h.rls_forced {
        blocks.push(format!("ALTER TABLE IF EXISTS {} FORCE ROW LEVEL SECURITY;\n", h.qname));
    }
    blocks.extend(policies.iter().map(|r| format!("{}\n", r.get::<_, String>("def"))));
    blocks.push(indexes.iter().map(|r| format!("{}\n", r.get::<_, String>("def"))).collect());
    blocks.push(
        triggers
            .iter()
            .map(|r| {
                let mut s = format!("{}\n", r.get::<_, String>("def"));
                if let Some(state) = r.get::<_, Option<String>>("state") {
                    s.push_str(&format!("ALTER TABLE IF EXISTS {} {state} TRIGGER {};\n", h.qname, r.get::<_, String>("name")));
                }
                s
            })
            .collect(),
    );
    Ok(header("Table", &h.qname, &format!("DROP TABLE IF EXISTS {};", h.qname)) + &join_blocks(blocks))
}
```

> `crate::db::Params` tipi `pub(crate)` olarak `db.rs`'te tanımlıdır (`Arc<Vec<Box<dyn ToSql + Sync + Send>>>`); `query` çağrısını `p![...]` makrosu doğrudan üretir. Kapanış (`q`) `cat`'ı ödünç aldığı için `try_join!` içinde aynı anda kullanılabilir. `h.qname` kapanışta değil, `ddl` gövdesinde kullanılır.

- [ ] **Step 7: Testleri çalıştır ve snapshot'ları gözden geçir**

Run: `$env:INSTA_UPDATE = "new"; cargo test -p pgt-catalog --test ddl`
Expected: `table_ddl_snapshots` (customers, orders, events, "Order Items") ve `full_table_ddl_snapshots_for_new_tables` yeni `.snap.new` ile FAIL; diğer testler PASS.

`.snap.new` dosyalarını **okuyarak** doğrula, sonra kabul et (cargo-insta yoksa `.snap.new` → `.snap`, `assertion_line:` sil, `Remove-Item Env:INSTA_UPDATE`). Beklenen farklar:
- `ddl__table_customers@pg18`: `CREATE TABLE …` bloğunun altında OWNER, COMMENT'ler, `GRANT SELECT, INSERT …`, `GRANT SELECT (email) … TO pgt_app;`, `ENABLE ROW LEVEL SECURITY`, iki `CREATE POLICY`, `CREATE UNIQUE INDEX customers_lower_name_idx …`, `CREATE TRIGGER customers_touch …`; kolon satırlarında değişiklik yok.
- `ddl__table_orders@pg18`: `CREATE INDEX orders_customer_idx … WHERE …` (ve PG ≥ 11'de `orders_cover_idx … INCLUDE (total)`; pg18 snapshot'ında ikisi de bulunur).
- `ddl__table_events@pg18`: `PARTITION BY RANGE (happened_on)` + `CREATE INDEX events_day_idx ON ONLY app.events …`.
- `ddl__table_Order_Items@pg18`: değişiklik yok (index/trigger yok).
- `ddl__table_extra_audit_child@pg18`: yalnızca `extra_col integer` ve `INHERITS (extra.audit_base)`; `ddl__table_extra_tickets@pg18`: identity seçenekleri.
- Hiçbir snapshot'ta gerçek rol adı (`postgres` vb.) kalmamalı; `<owner>`, `pgt_reader`, `pgt_app`, `PUBLIC` olabilir.

Run: `cargo test -p pgt-catalog --test ddl` (INSTA_UPDATE'siz; iki kez)
Expected: PASS.

Run: `cargo test --workspace` (env'li ve env'siz)
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/pgt-catalog
git commit -m "feat(catalog): tam Table DDL — partition, INHERITS, index, trigger, RLS, policy, kolon yetkileri, identity"
```

---

### Task 6: Bağlantı-koptu göstergesi (Inspector hatalarından sunucuyu bağlantısız işaretle)

Plan 1'in son incelemesinden kalan açık iş: oturum kaybı ilk olarak Inspector'ın `getProperties`/`getDdl` çağrısında fark edilirse ağaç sunucuyu "bağlı" göstermeye devam ediyordu (kurtarma yalnızca sunucu düğümünde F5 ile oluyordu).

**Files:**
- Modify: `src/features/explorer/controller.ts`, `src/features/explorer/controller.test.ts`
- Modify: `src/features/inspector/Inspector.tsx`

**Interfaces:**
- Consumes: `controller.ts` içindeki mevcut `SESSION_GONE` kümesi, `markDisconnected(s, loadingId, serverId, error)`, `serverOf`; `Explorer.controller`.
- Produces:
  - `export function isSessionGone(code: string): boolean`
  - `ExplorerController.sessionLost(id: string, error: ErrorPayload): boolean` — hata kodu `not_connected` / `connection_lost` ise ve `id` bir sunucunun altındaysa sunucuyu bağlantısız işaretler (alt ağacı boşaltır, hatayı sunucu düğümüne koyar, seçimi sunucuya taşır) ve `true` döner; aksi halde hiçbir şey yapmaz ve `false` döner.
  - `Inspector` backend çağrılarının hatalarını `sessionLost` ile denetleyiciye bildirir.

- [ ] **Step 1: Başarısız testleri yaz**

`src/features/explorer/controller.test.ts` içindeki `describe("ExplorerController", …)` bloğunun **sonuna** (son `});` öncesine) ekle:

```ts
  it("sessionLost marks the owning server disconnected, clears its subtree and the next expand reconnects", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const { c, get, connectServer } = setup({ [ROOT]: [srv], srv: [n("dbs")] });
    await c.load(ROOT);
    await c.expand("srv");
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
    c.select("dbs");

    const payload: ErrorPayload = { code: "connection_lost", message: "Bağlantı koptu", detail: "x", hint: null };
    expect(c.sessionLost("dbs", payload)).toBe(true);

    const server = get().nodes.get("srv")!;
    expect([server.flags.connected, server.detail]).toEqual([false, null]);
    expect(get().nodes.has("dbs")).toBe(false);
    expect(get().errors.get("srv")?.code).toBe("connection_lost");
    expect(get().selected).toBe("srv");

    await c.expand("srv");
    expect(connectServer).toHaveBeenCalledTimes(2);
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
    expect(flatten(get()).map((r) => r.id)).toEqual(["srv", "dbs"]);
  });

  it("sessionLost ignores unrelated error codes and nodes outside any server", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const { c, get } = setup({ [ROOT]: [srv, n("grp:G", { kind: "serverGroup" })], srv: [n("dbs")] });
    await c.load(ROOT);
    await c.expand("srv");
    const query: ErrorPayload = { code: "query", message: "m", detail: null, hint: null };
    expect(c.sessionLost("dbs", query)).toBe(false);
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
    const lost: ErrorPayload = { code: "not_connected", message: "m", detail: null, hint: null };
    expect(c.sessionLost("grp:G", lost)).toBe(false);
    expect(c.sessionLost("yok", lost)).toBe(false);
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
  });
```

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run: `bun run test`
Expected: FAIL — `c.sessionLost is not a function`.

- [ ] **Step 3: Denetleyiciyi güncelle**

`src/features/explorer/controller.ts` içinde `SESSION_GONE` tanımının hemen altına ekle:

```ts
export function isSessionGone(code: string): boolean {
  return SESSION_GONE.has(code);
}
```

`ExplorerController` sınıfına, `load` metodunun üstüne ekle:

```ts
  /**
   * Arka uç oturumunun düştüğü başka bir yoldan (ör. Inspector çağrısı) öğrenildiğinde çağrılır.
   * `id` bir sunucunun altındaysa sunucuyu bağlantısız işaretler; aksi halde hiçbir şey yapmaz.
   */
  sessionLost(id: string, error: ErrorPayload): boolean {
    if (!isSessionGone(error.code)) return false;
    const server = serverOf(this.get(), id);
    if (!server) return false;
    this.update((s) => markDisconnected(s, id, server.id, error));
    return true;
  }
```

`doLoad` içindeki `SESSION_GONE.has(error.code)` kullanımı olduğu gibi kalabilir.

- [ ] **Step 4: Inspector'ı bağla**

`src/features/inspector/Inspector.tsx` içinde `const [properties] = createResource(…)` ve `const [ddl] = createResource(…)` bloklarını **değiştir**:

```tsx
  // Backend çağrısı oturum kaybıyla başarısız olursa ağaç da sunucuyu bağlantısız göstermeli.
  const guarded = async <T,>(id: string, call: () => Promise<T>): Promise<T> => {
    try {
      return await call();
    } catch (e) {
      props.explorer.controller.sessionLost(id, toPayload(e));
      throw e;
    }
  };

  const [properties] = createResource(
    () => selected()?.id,
    async (id) => ({ id, value: await guarded(id, () => commands.getProperties(id)) }),
  );
  const [ddl] = createResource(
    () => (tab() === "ddl" ? selected()?.id : undefined),
    async (id) => ({ id, value: await guarded(id, () => commands.getDdl(id)) }),
  );
```

(`toPayload` zaten içe aktarılmıştır.)

- [ ] **Step 5: Doğrula**

Run: `bun run test && bun run typecheck && bun run build`
Expected: PASS (yeni 2 test dahil); build başarılı.

- [ ] **Step 6: Commit**

```bash
git add src
git commit -m "fix(ui): Inspector'da fark edilen oturum kaybı sunucuyu bağlantısız işaretlesin"
```

---

### Task 7: DDL gidiş-dönüş testi

Üretilen DDL'in doğruluğunun en güçlü kanıtı: fixture veritabanındaki tüm nesnelerin DDL'i boş bir veritabanına uygulanır ve iki veritabanının **katalog imzaları** karşılaştırılır.

**Files:**
- Create: `crates/pgt-catalog/tests/fixtures/signature.sql`, `crates/pgt-catalog/tests/roundtrip.rs`
- Modify (hata bulunursa): ilgili `ddl_*.sql` / `table_ddl_*.sql` dosyaları ve o türün testleri

**Interfaces:**
- Consumes: Task 2–5 (`Catalog::{list_children, ddl}`), `tests/common` (`fixtures`, `Fixture::{schema, nodes_of}`), `pgt_core::testing::admin_client`.
- Produces: `tests/roundtrip.rs::ddl_round_trip_reproduces_the_catalog` testi; `tests/fixtures/signature.sql` (`kind`, `key`, `def` kolonlu, OID içermeyen katalog imzası; `app` ve `extra` şemaları, extension üyeleri hariç).

**Algoritma:**
1. Fixture veritabanındaki her nesnenin DDL'i `Catalog::ddl` ile toplanır: şemalar (`app`, `extra`), extension'lar (`plpgsql` hariç), her şemada types, domains, sequences, functions, procedures (PG ≥ 11), trigger functions, views, matviews, tables (+ partition'ları özyinelemeli). `fromExtension` bayraklı nesneler atlanır. Constraint/index/trigger/policy DDL'leri ayrıca uygulanmaz (Table DDL'i bunları içerir).
2. `pgt_roundtrip` adlı boş veritabanı (`TEMPLATE template0 ENCODING 'UTF8'`) oluşturulur.
3. DDL'ler sabit nokta döngüsüyle uygulanır: her geçişte sırayla `batch_execute` (çok ifadeli sorgu örtük transaction'dır, bir DDL ya tamamen uygulanır ya hiç) denenir; başarısızlar sonraki geçişe kalır; ilerleme olmayınca durur (en çok 12 geçiş). Sequence DDL'lerindeki `ALTER SEQUENCE … OWNED BY` blokları tablo oluşana kadar ertelenir ve en sonda uygulanır (tablo varsayılanı sequence'e, `OWNED BY` ise tabloya bağlıdır — döngüsel bağımlılık).
4. İki veritabanında `signature.sql` çalıştırılır; satır kümeleri eşit olmalıdır.

- [ ] **Step 1: İmza SQL'ini yaz**

`crates/pgt-catalog/tests/fixtures/signature.sql` (yeni; PG 18'de doğrulandı, PG 10+ uyumlu — `regcollation` gibi yeni tipler kullanılmaz):

```sql
-- Gidiş-dönüş testi için katalog imzası: OID içermeyen, iki veritabanında karşılaştırılabilir satırlar (kind, key, def).
-- Kapsam: app ve extra şemaları; extension üyeleri hariç.
WITH ns AS (
    SELECT oid, nspname::text AS nspname FROM pg_namespace WHERE nspname IN ('app', 'extra')
),
rel AS (
    SELECT c.oid, c.relkind, c.relpersistence, c.reloptions, c.relrowsecurity, c.relforcerowsecurity, c.relowner, c.relacl,
           c.relispartition, c.relpartbound, c.relispopulated, n.nspname || '.' || c.relname AS qname
    FROM pg_class c
    JOIN ns n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm', 'S')
      AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e')
)
SELECT 'schema' AS kind, n.nspname AS key,
       concat_ws(' | ', pg_get_userbyid(s.nspowner), coalesce(s.nspacl::text, ''), coalesce(obj_description(s.oid, 'pg_namespace'), '')) AS def
FROM ns n JOIN pg_namespace s ON s.oid = n.oid
UNION ALL
SELECT 'relation', r.qname,
       concat_ws(' | ', r.relkind::text, r.relpersistence::text, coalesce(array_to_string(r.reloptions, ','), ''),
                 r.relrowsecurity, r.relforcerowsecurity, pg_get_userbyid(r.relowner), coalesce(r.relacl::text, ''),
                 coalesce(obj_description(r.oid, 'pg_class'), ''),
                 CASE WHEN r.relkind = 'p' THEN pg_get_partkeydef(r.oid) ELSE '' END,
                 CASE WHEN r.relispartition THEN pg_get_expr(r.relpartbound, r.oid) ELSE '' END,
                 coalesce((SELECT string_agg(pn.nspname || '.' || pc.relname, ',' ORDER BY i.inhseqno)
                           FROM pg_inherits i JOIN pg_class pc ON pc.oid = i.inhparent JOIN pg_namespace pn ON pn.oid = pc.relnamespace
                           WHERE i.inhrelid = r.oid), ''),
                 CASE WHEN r.relkind IN ('v', 'm') THEN pg_get_viewdef(r.oid, true) ELSE '' END,
                 CASE WHEN r.relkind = 'm' THEN r.relispopulated::text ELSE '' END)
FROM rel r
UNION ALL
SELECT 'column', r.qname || '.' || a.attname,
       concat_ws(' | ', format_type(a.atttypid, a.atttypmod), a.attnotnull, coalesce(pg_get_expr(d.adbin, d.adrelid), ''),
                 a.attidentity::text, a.attgenerated::text,
                 coalesce((SELECT co.collname::text FROM pg_collation co WHERE co.oid = a.attcollation), ''),
                 coalesce(a.attacl::text, ''), coalesce(col_description(a.attrelid, a.attnum), ''))
FROM rel r
JOIN pg_attribute a ON a.attrelid = r.oid AND a.attnum > 0 AND NOT a.attisdropped
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
WHERE r.relkind IN ('r', 'p', 'v', 'm')
UNION ALL
SELECT 'constraint', r.qname || '.' || k.conname, pg_get_constraintdef(k.oid, true) || ' | ' || k.convalidated
FROM rel r JOIN pg_constraint k ON k.conrelid = r.oid AND k.contype IN ('p', 'u', 'c', 'x', 'f')
UNION ALL
SELECT 'index', n.nspname || '.' || ic.relname, pg_get_indexdef(x.indexrelid) || ' | ' || coalesce(obj_description(ic.oid, 'pg_class'), '')
FROM pg_index x
JOIN pg_class ic ON ic.oid = x.indexrelid
JOIN ns n ON n.oid = ic.relnamespace
UNION ALL
SELECT 'trigger', r.qname || '.' || t.tgname, pg_get_triggerdef(t.oid, true) || ' | ' || t.tgenabled::text || ' | ' || coalesce(obj_description(t.oid, 'pg_trigger'), '')
FROM rel r JOIN pg_trigger t ON t.tgrelid = r.oid AND NOT t.tgisinternal
UNION ALL
SELECT 'policy', r.qname || '.' || p.polname,
       concat_ws(' | ', p.polcmd::text, p.polpermissive, p.polroles::regrole[]::text,
                 coalesce(pg_get_expr(p.polqual, p.polrelid, true), ''), coalesce(pg_get_expr(p.polwithcheck, p.polrelid, true), ''))
FROM rel r JOIN pg_policy p ON p.polrelid = r.oid
UNION ALL
SELECT 'sequence', r.qname, concat_ws(' | ', s.seqtypid::regtype::text, s.seqstart, s.seqincrement, s.seqmin, s.seqmax, s.seqcache, s.seqcycle)
FROM rel r JOIN pg_sequence s ON s.seqrelid = r.oid
UNION ALL
SELECT 'function', n.nspname || '.' || p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')',
       concat_ws(' | ', pg_get_functiondef(p.oid), coalesce(p.proacl::text, ''), pg_get_userbyid(p.proowner), coalesce(obj_description(p.oid, 'pg_proc'), ''))
FROM pg_proc p
JOIN ns n ON n.oid = p.pronamespace
WHERE NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype IN ('e', 'i'))
UNION ALL
SELECT 'type', n.nspname || '.' || t.typname,
       concat_ws(' | ', t.typtype::text, pg_get_userbyid(t.typowner), coalesce(t.typacl::text, ''), coalesce(obj_description(t.oid, 'pg_type'), ''),
                 CASE WHEN t.typtype = 'e' THEN (SELECT string_agg(e.enumlabel, ',' ORDER BY e.enumsortorder) FROM pg_enum e WHERE e.enumtypid = t.oid) END,
                 CASE WHEN t.typtype = 'c' THEN (SELECT string_agg(a.attname || ' ' || format_type(a.atttypid, a.atttypmod), ',' ORDER BY a.attnum)
                                                  FROM pg_attribute a WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped) END,
                 CASE WHEN t.typtype = 'r' THEN (SELECT format_type(r.rngsubtype, NULL) FROM pg_range r WHERE r.rngtypid = t.oid) END,
                 CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod) || ' ' || t.typnotnull || ' ' || coalesce(t.typdefault, '')
                        || ' ' || coalesce((SELECT string_agg(k.conname || ':' || pg_get_constraintdef(k.oid, true), ',' ORDER BY k.conname) FROM pg_constraint k WHERE k.contypid = t.oid), '') END)
FROM pg_type t
JOIN ns n ON n.oid = t.typnamespace
WHERE t.typtype IN ('e', 'c', 'r', 'd')
  AND (t.typtype <> 'c' OR EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c'))
  AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_type'::regclass AND d.objid = t.oid AND d.deptype = 'e')
ORDER BY kind, key
```

- [ ] **Step 2: Gidiş-dönüş testini yaz**

`crates/pgt-catalog/tests/roundtrip.rs` (yeni):

```rust
mod common;

use std::collections::BTreeSet;

use common::{Fixture, fixtures};
use pgt_catalog::node::{Coll, NodeId, TreeNode};
use pgt_core::testing::admin_client;

const SIGNATURE_SQL: &str = include_str!("fixtures/signature.sql");
const SCRATCH_DB: &str = "pgt_roundtrip";

fn parse(node: &TreeNode) -> NodeId {
    NodeId::parse(&node.id).unwrap()
}

/// `(başlık satırı, DDL)` çiftleri: fixture'daki her nesne için.
async fn collect_ddls(fx: &Fixture) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |ddl: String| {
        let title = ddl.lines().next().unwrap_or("").to_string();
        out.push((title, ddl));
    };

    for node in fx.nodes_of(&fx.db(), Coll::Extensions).await {
        if node.label != "plpgsql" {
            push(fx.catalog.ddl(&parse(&node)).await.unwrap());
        }
    }
    for schema_name in ["app", "extra"] {
        let schema = fx.schema(schema_name).await;
        push(fx.catalog.ddl(&schema).await.unwrap());
        let mut colls = vec![Coll::Types, Coll::Domains, Coll::Sequences, Coll::Functions, Coll::TriggerFunctions, Coll::Views, Coll::MatViews];
        if fx.major().await >= 11 {
            colls.push(Coll::Procedures);
        }
        for coll in colls {
            for node in fx.nodes_of(&schema, coll).await {
                if !node.flags.from_extension {
                    push(fx.catalog.ddl(&parse(&node)).await.unwrap());
                }
            }
        }
        // tablolar ve (özyinelemeli) partition'ları
        let mut stack: Vec<NodeId> = fx.nodes_of(&schema, Coll::Tables).await.iter().map(parse).collect();
        while let Some(id) = stack.pop() {
            push(fx.catalog.ddl(&id).await.unwrap());
            let kids = fx.catalog.list_children(&id).await.unwrap();
            if kids.iter().any(|k| k.label == "Partitions") {
                stack.extend(fx.nodes_of(&id, Coll::Partitions).await.iter().map(parse));
            }
        }
    }
    out
}

/// Sequence DDL'inin `OWNED BY` bloklarını ayırır (tablo olmadan uygulanamaz).
fn split_owned_by(ddl: &str) -> (String, Vec<String>) {
    if !ddl.starts_with("-- Sequence:") {
        return (ddl.to_string(), Vec::new());
    }
    let (owned, rest): (Vec<&str>, Vec<&str>) = ddl.split("\n\n").partition(|b| b.starts_with("ALTER SEQUENCE") && b.contains("OWNED BY"));
    (rest.join("\n\n"), owned.into_iter().map(str::to_string).collect())
}

async fn signature(db: &tokio_postgres::Client) -> BTreeSet<(String, String, String)> {
    db.query(SIGNATURE_SQL, &[])
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1), r.get::<_, String>(2)))
        .collect()
}

#[tokio::test]
async fn ddl_round_trip_reproduces_the_catalog() {
    for fx in fixtures().await {
        let ddls = collect_ddls(&fx).await;
        assert!(ddls.len() >= 25, "beklenenden az nesne toplandı: {}", ddls.len());

        let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        admin
            .execute(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
                &[&SCRATCH_DB],
            )
            .await
            .unwrap();
        admin.batch_execute(&format!("DROP DATABASE IF EXISTS {SCRATCH_DB}")).await.unwrap();
        admin.batch_execute(&format!("CREATE DATABASE {SCRATCH_DB} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
        let scratch = admin_client(&fx.ts, SCRATCH_DB).await;

        let mut deferred: Vec<String> = Vec::new();
        let mut pending: Vec<(String, String)> = ddls
            .iter()
            .map(|(title, ddl)| {
                let (ddl, owned) = split_owned_by(ddl);
                deferred.extend(owned);
                (title.clone(), ddl)
            })
            .collect();

        for _pass in 0..12 {
            let before = pending.len();
            let mut next = Vec::new();
            for (title, ddl) in pending {
                if scratch.batch_execute(&ddl).await.is_err() {
                    next.push((title, ddl));
                }
            }
            pending = next;
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        if !pending.is_empty() {
            let mut report = String::new();
            for (title, ddl) in &pending {
                let err = scratch.batch_execute(ddl).await.unwrap_err();
                let msg = err.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| err.to_string());
                report.push_str(&format!("{title}\n  hata: {msg}\n"));
            }
            panic!("{} DDL uygulanamadı:\n{report}", pending.len());
        }
        for stmt in deferred {
            scratch.batch_execute(&stmt).await.unwrap_or_else(|e| panic!("OWNED BY uygulanamadı: {e}\n{stmt}"));
        }

        let source = admin_client(&fx.ts, common::FIXTURE_DB).await;
        let (want, got) = (signature(&source).await, signature(&scratch).await);
        let missing: Vec<_> = want.difference(&got).take(20).collect();
        let extra: Vec<_> = got.difference(&want).take(20).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "katalog imzaları farklı\nyalnızca fixture'da ({}):\n{missing:#?}\nyalnızca gidiş-dönüş veritabanında ({}):\n{extra:#?}",
            want.difference(&got).count(),
            got.difference(&want).count()
        );

        drop(scratch);
        admin
            .execute(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
                &[&SCRATCH_DB],
            )
            .await
            .unwrap();
        admin.batch_execute(&format!("DROP DATABASE {SCRATCH_DB}")).await.unwrap();
    }
}
```

- [ ] **Step 3: Testi çalıştır**

Run (PowerShell): `$env:PGT_TEST_URLS = (Get-Content .superpowers/sdd/test-url.txt); cargo test -p pgt-catalog --test roundtrip -- --nocapture`
Expected: büyük olasılıkla ilk çalıştırmada FAIL (uygulanamayan DDL veya imza farkı). Bu, testin amacıdır: çıktıdaki her fark üretilen DDL'deki bir hatadır.

- [ ] **Step 4: Bulunan DDL hatalarını düzelt**

Çıktıdaki her uygulanamayan DDL / imza farkı için:
1. Hatanın hangi nesne türünden geldiğini belirle (başlık satırı `-- <Tür>: <ad>` bunu gösterir).
2. O türün SQL'ini düzelt (`ddl_*.sql` veya `table_ddl_*.sql`).
3. O türün ilgili testine (Task 4/5) kalıcı bir regresyon kontrolü ekle (alt dize doğrulaması veya snapshot).
4. Değişen snapshot'ları gözden geçirerek kabul et.
5. Testi yeniden çalıştır; tamamen geçene kadar tekrarla.

Beklenen aday bulgular (hepsi olmayabilir; gerçek çıktıya göre karar ver): view kolon takma adlarının `pg_get_viewdef` çıktısında korunması; `GRANT` satırlarının veren (grantor) farkı; matview'in `WITH DATA` yeniden doldurulması nedeniyle `relispopulated`; `INHERITS` çocuğunda devralınan kısıtların çifte yazılması; partition'daki index/trigger kopyaları; bigserial varsayılanındaki `nextval('app.orders_id_seq'::regclass)` ve sequence sırası. Çözülemeyen ve bilinçli olarak kapsam dışı bırakılan bir fark çıkarsa imza sorgusundan ilgili satırı **çıkarmak yerine** farkı spec'in "bilinen sınırlar" bölümüne (bu planın başındaki liste) ve `docs/testing.md`'ye yaz.

- [ ] **Step 5: Geçişi doğrula**

Run: `cargo test -p pgt-catalog --test roundtrip` (env'li, iki kez) ve `cargo test --workspace`
Expected: PASS; iki ardışık çalıştırmada aynı sonuç (`pgt_roundtrip` veritabanı her seferinde sıfırdan kurulur ve sonunda silinir).

- [ ] **Step 6: Commit**

```bash
git add crates/pgt-catalog
git commit -m "test(catalog): DDL gidiş-dönüş testi ve katalog imzası"
```

---

### Task 8: Ölçek performans testi (5.000 tablo)

Spec §1.3: 5.000 tablolu bir "Tables" düğümünü açma < 300 ms; seçili tablonun DDL'i < 200 ms.

**Files:**
- Create: `crates/pgt-catalog/tests/perf.rs`
- Modify: `crates/pgt-catalog/src/kinds/table.rs` (yalnızca ölçüm bütçeyi aşarsa)

**Interfaces:**
- Consumes: `Catalog`, `ConnectionManager`, `Registry`, `pgt_core::testing::{admin_client, test_servers}`.
- Produces: `tests/perf.rs::five_thousand_tables_stay_within_budget` — süreleri bütçeyle karşılaştırır; bütçeler `PGT_PERF_FACTOR` ortam değişkeniyle (varsayılan `1.0`) çarpılabilir (yavaş CI makineleri için).

- [ ] **Step 1: Testi yaz**

`crates/pgt-catalog/tests/perf.rs` (yeni):

```rust
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use pgt_catalog::Catalog;
use pgt_catalog::node::{Coll, NodeId, Seg};
use pgt_core::connection::ConnectionManager;
use pgt_core::registry::Registry;
use pgt_core::testing::{admin_client, test_servers};

const PERF_DB: &str = "pgt_perf";
const TABLES: usize = 5_000;
const RUNS: usize = 5;

fn factor() -> f64 {
    std::env::var("PGT_PERF_FACTOR").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

async fn timed<T, F, Fut>(f: F) -> (Duration, T)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let start = Instant::now();
    let value = f().await;
    (start.elapsed(), value)
}

#[tokio::test]
async fn five_thousand_tables_stay_within_budget() {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
        return;
    }
    for ts in servers {
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        admin
            .execute("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()", &[&PERF_DB])
            .await
            .unwrap();
        admin.batch_execute(&format!("DROP DATABASE IF EXISTS {PERF_DB}")).await.unwrap();
        admin.batch_execute(&format!("CREATE DATABASE {PERF_DB} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
        let db = admin_client(&ts, PERF_DB).await;
        db.batch_execute("CREATE SCHEMA perf").await.unwrap();
        // 100'lük gruplar halinde: tek transaction'da 5.000 tablo kilit tablosunu doldurabilir.
        for batch in 0..(TABLES / 100) {
            let mut sql = String::new();
            for i in 0..100 {
                let n = batch * 100 + i;
                sql.push_str(&format!("CREATE TABLE perf.t_{n:05} (id integer PRIMARY KEY, note text, created_at timestamptz DEFAULT now());\n"));
            }
            db.batch_execute(&sql).await.unwrap();
        }
        db.batch_execute("ANALYZE").await.unwrap();
        let db_oid: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&PERF_DB]).await.unwrap().get(0);

        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
        registry.upsert(ts.config.clone()).unwrap();
        let mgr = Arc::new(ConnectionManager::new());
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let catalog = Catalog::new(mgr.clone(), Arc::new(RwLock::new(registry)));

        let db_id = NodeId::server(ts.config.id).child(Seg::Db(db_oid));
        let schemas = catalog.list_children(&db_id.child(Seg::Coll(Coll::Schemas))).await.unwrap();
        let schema_id = NodeId::parse(&schemas.iter().find(|s| s.label == "perf").expect("perf şeması").id).unwrap();
        let tables_id = schema_id.child(Seg::Coll(Coll::Tables));

        // ısınma: bağlantıyı ve katalog önbelleklerini hazırla
        let tables = catalog.list_children(&tables_id).await.unwrap();
        assert_eq!(tables.len(), TABLES);
        let sample = NodeId::parse(&tables[TABLES / 2].id).unwrap();

        let mut list_runs = Vec::new();
        let mut props_runs = Vec::new();
        let mut ddl_runs = Vec::new();
        for _ in 0..RUNS {
            let (t, list) = timed(|| catalog.list_children(&tables_id)).await;
            assert_eq!(list.unwrap().len(), TABLES);
            list_runs.push(t);
            let (t, props) = timed(|| catalog.properties(&sample)).await;
            props.unwrap();
            props_runs.push(t);
            let (t, ddl) = timed(|| catalog.ddl(&sample)).await;
            assert!(ddl.unwrap().contains("CREATE TABLE IF NOT EXISTS perf.t_02500"));
            ddl_runs.push(t);
        }
        let (list, props, ddl) = (median(list_runs), median(props_runs), median(ddl_runs));
        eprintln!("5.000 tablo: liste {list:?}, properties {props:?}, ddl {ddl:?} (medyan, {RUNS} çalıştırma)");

        let f = factor();
        let budget = |ms: f64| Duration::from_secs_f64(ms * f / 1000.0);
        assert!(list < budget(300.0), "Tables listesi {list:?} > {:?}", budget(300.0));
        assert!(ddl < budget(200.0), "Table DDL'i {ddl:?} > {:?}", budget(200.0));
        assert!(props < budget(200.0), "Table Properties {props:?} > {:?}", budget(200.0));

        drop(db);
        mgr.disconnect(ts.config.id);
        admin
            .execute("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()", &[&PERF_DB])
            .await
            .unwrap();
        admin.batch_execute(&format!("DROP DATABASE {PERF_DB}")).await.unwrap();
    }
}
```

> Test debug derlemesinde çalışır; ölçüm yine sunucu tarafı sorgu süresine ve ağ gidiş-dönüşüne baskındır. `uuid` ve `tempfile` zaten `pgt-catalog` dev-bağımlılıklarındadır (`pgt_core::testing` `testing` özelliğiyle açılmıştır).

- [ ] **Step 2: Testi çalıştır**

Run (PowerShell): `$env:PGT_TEST_URLS = (Get-Content .superpowers/sdd/test-url.txt); cargo test -p pgt-catalog --test perf -- --nocapture`
Expected: PASS ve çıktıda `5.000 tablo: liste …, properties …, ddl …` satırı. Tablo oluşturma ~5–20 sn sürer.

- [ ] **Step 3: Bütçe aşılırsa**

Bütçe aşıldıysa **önce** ölçümü doğrula (`PGT_PERF_FACTOR` ile değil, tekrar çalıştırarak). Gerçek bir aşım varsa nedenini bul: liste için sorgu planı (`EXPLAIN ANALYZE` ile `tables.sql`), DDL için `tokio::try_join!` gerçekten eş zamanlı mı (Task 5), `reltuples` formatlaması gibi Rust tarafı işi. Düzeltmeyi yap ve süreleri `docs/performance.md` (Task 10) için not et. Aşım yoksa bu adım atlanır.

- [ ] **Step 4: Commit**

```bash
git add crates/pgt-catalog
git commit -m "test(catalog): 5.000 tablolu ölçek performans testi"
```

---

### Task 9: PostgreSQL 10–18 sürüm matrisi (Docker)

Spec §8: aynı entegrasyon testleri PG 10, 12, 14, 16, 17, 18 üzerinde çalışır. Docker Desktop'ın **çalışıyor** olması gerekir (bu makinede kurulu ama daemon kapalıydı: `docker info` hata veriyorsa görevi BLOCKED olarak raporla ve kullanıcıdan Docker Desktop'ı açmasını iste).

**Files:**
- Create: `scripts/test-matrix.ps1`, `docs/testing.md`
- Create: sürüme özel snapshot'lar `crates/pgt-catalog/tests/snapshots/*@pg10.snap`, `@pg12`, `@pg14`, `@pg16`, `@pg17`
- Modify (sürüm farkı bulunursa): ilgili `.pg10.sql` varyantları, testlerdeki sürüm koşulları

**Interfaces:**
- Consumes: tüm önceki task'ların testleri; `PGT_TEST_URLS` (virgülle ayrılmış birden çok adres desteklenir; testler her sunucuda ayrı ayrı çalışır).
- Produces: `scripts/test-matrix.ps1 [-Versions 10,12,…] [-KeepContainers]`: her sürüm için bir kapsayıcı başlatır, `PGT_TEST_URLS`'i ayarlar, `cargo test --workspace --all-features` çalıştırır, kapsayıcıları temizler.

- [ ] **Step 1: Betiği yaz**

`scripts/test-matrix.ps1` (yeni):

```powershell
<#
.SYNOPSIS
  PostgreSQL sürüm matrisinde pg-tools entegrasyon testlerini çalıştırır.
.PARAMETER Versions
  Test edilecek ana sürümler (varsayılan: 10, 12, 14, 16, 17, 18).
.PARAMETER KeepContainers
  Testten sonra kapsayıcıları silme (hata ayıklama için).
#>
param(
    [string[]]$Versions = @("10", "12", "14", "16", "17", "18"),
    [switch]$KeepContainers
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

docker info *> $null
if ($LASTEXITCODE -ne 0) {
    throw "Docker çalışmıyor. Docker Desktop'ı başlatıp tekrar deneyin."
}

# Geçici, her çalıştırmada rastgele parola; hiçbir yere yazılmaz.
$password = [guid]::NewGuid().ToString("N")
$urls = @()
$names = @()

try {
    foreach ($v in $Versions) {
        $name = "pgt-matrix-$v"
        $port = 55400 + [int]$v
        docker rm -f $name *> $null
        docker run -d --name $name `
            -e POSTGRES_PASSWORD=$password `
            -e POSTGRES_INITDB_ARGS="--auth-host=scram-sha-256" `
            -e POSTGRES_HOST_AUTH_METHOD=scram-sha-256 `
            -p "127.0.0.1:${port}:5432" "postgres:$v" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "postgres:$v başlatılamadı" }
        $names += $name
        $urls += "postgres://postgres:$password@127.0.0.1:$port/postgres"
    }

    foreach ($name in $names) {
        $ready = $false
        for ($i = 0; $i -lt 90; $i++) {
            docker exec $name pg_isready -U postgres -h 127.0.0.1 *> $null
            if ($LASTEXITCODE -eq 0) { $ready = $true; break }
            Start-Sleep -Seconds 1
        }
        if (-not $ready) { throw "$name hazır olmadı" }
    }
    # Resmi imaj ilk başlatmada geçici bir sunucu açıp kapatır; kalıcı sunucunun ayağa kalkması için kısa bekleme.
    Start-Sleep -Seconds 3

    $env:PGT_TEST_URLS = ($urls -join ",")
    cargo test --workspace --all-features
    if ($LASTEXITCODE -ne 0) { throw "testler başarısız" }
}
finally {
    Remove-Item Env:PGT_TEST_URLS -ErrorAction SilentlyContinue
    if (-not $KeepContainers) {
        foreach ($name in $names) { docker rm -f $name *> $null }
    }
}
```

- [ ] **Step 2: Matrisi çalıştır ve farkları bul**

Run (PowerShell, Docker Desktop açıkken): `./scripts/test-matrix.ps1`
Expected: ilk çalıştırmada büyük olasılıkla FAIL — (a) eski sürümlerde yeni snapshot'lar bulunmadığı için `*@pgNN.snap` eksik, (b) sürüme özel gerçek farklar.

Sürümlere özel snapshot'lar için `INSTA_UPDATE=new` ile çalıştır:

```powershell
$env:INSTA_UPDATE = "new"; ./scripts/test-matrix.ps1; Remove-Item Env:INSTA_UPDATE
```

Her yeni `*@pgNN.snap.new` dosyasını **PG 18 snapshot'ıyla karşılaştırarak oku**: farklar yalnızca sunucunun kendi çıktısından (ör. `pg_get_constraintdef` biçimi, `POSITION` deparse'ı, extension sürüm numarası) geliyorsa kabul et. Bir fark bizim SQL'imizin eski sürümde hatalı çalıştığını gösteriyorsa (ör. olmayan kolon, farklı `pg_*` yapısı) düzeltme yap: `.pg10.sql` varyantı ekle/değiştir ve `VersionedSql` dilimini güncelle. Beklenen sürüm kaynaklı bulgular: PG 10'da `pg_get_triggerdef` / procedure yokluğu, PG 10–11'de generated kolon yokluğu, PG < 13'te `tgparentid` yokluğu (Task 5 buna hazırdır), PG < 14'te `pg_sequence`/`reltuples` farkları.

- [ ] **Step 3: Test koşullarını sürüme göre düzelt**

Sürüme bağlı testler `fx.major().await >= N` koşuluyla korunmalıdır (Task 2–5 örnekleri: Procedures ≥ 11, partition index ≥ 11, generated kolon ≥ 12). Matriste başarısız olan başka bir test sürüm bağımlıysa aynı kalıpla koru; **koşulu gevşetmek yerine** önce nedeninin gerçek bir kod hatası olmadığını doğrula.

- [ ] **Step 4: Belgele**

`docs/testing.md` (yeni):

```markdown
# Test rehberi

## Birim testleri
    cargo test --workspace            # PGT_TEST_URLS yoksa entegrasyon testleri atlanır
    bun run test
    bun run typecheck

## Entegrasyon testleri (gerçek PostgreSQL)
`PGT_TEST_URLS` virgülle ayrılmış `postgres://kullanici:parola@host:port/postgres` adresleri içerir.
Test kullanıcısı `CREATEDB` ve `CREATEROLE` yetkisine sahip olmalı (superuser yeterli); sunucuda
`postgresql-contrib` (pg_trgm) kurulu olmalı; kimlik doğrulama scram-sha-256.

    $env:PGT_TEST_URLS = "postgres://postgres:PAROLA@127.0.0.1:5432/postgres"
    cargo test --workspace --all-features

Testler `pgt_fixture`, `pgt_roundtrip`, `pgt_perf`, `pgt_noconn`, `pgt_private` adlı veritabanlarını
ve `pgt_reader`, `pgt_app`, `pgt_limited` rollerini oluşturur/yeniden kurar; üretim sunucusunda çalıştırmayın.

## Sürüm matrisi (Docker)
    ./scripts/test-matrix.ps1                       # PG 10, 12, 14, 16, 17, 18
    ./scripts/test-matrix.ps1 -Versions 16,18

Sürüme özel DDL snapshot'ları `tests/snapshots/*@pgNN.snap` olarak tutulur. Yeni bir snapshot gerektiğinde
`INSTA_UPDATE=new` ile üretilip PG 18 ile karşılaştırılarak gözden geçirilir.

## Performans testi
`cargo test -p pgt-catalog --test perf -- --nocapture` — 5.000 tablolu şemada liste/Properties/DDL süreleri
(spec §1.3 bütçeleri). Yavaş makinede `PGT_PERF_FACTOR=2` ile bütçeler gevşetilebilir (CI'da kullanmayın).

## Bilinen DDL sınırları
base type DDL'i desteklenmez; range type yalnızca SUBTYPE; PG 18 adlandırılmış NOT NULL kısıt adları korunmaz;
kolon STORAGE/COMPRESSION yazılmaz; GRANT veren (grantor) bilgisi kullanılmaz; PG 11–12'de partition'lara
klonlanan trigger'lar partition DDL'inde tekrar yazılır.
```

- [ ] **Step 5: Doğrula ve commit**

Run: `./scripts/test-matrix.ps1`
Expected: PASS — 6 sürümün hepsinde tüm entegrasyon testleri; iki ardışık çalıştırmada aynı sonuç.

```bash
git add scripts docs/testing.md crates/pgt-catalog
git commit -m "test: PostgreSQL 10–18 sürüm matrisi betiği, sürüm snapshot'ları ve test rehberi"
```

---

### Task 10: Bellek ince ayarı ve hedefin yeniden tanımı

Plan 1 ölçümü: boşta toplam **çalışma kümesi** ~380 MB, **özel bellek** ~191 MB (spec hedefi < 150 MB); Rust süreci 30–40 MB (hedef < 30 MB). Çalışma kümesi paylaşılan WebView2 sayfalarını çift sayar; bu yüzden metrik **özel çalışma kümesi toplamı (Private Working Set — Görev Yöneticisi "Bellek" sütununun toplamı)** olarak yeniden tanımlanır. Bu task önce ince ayar dener, sonra hedefi **ölçülen sonuca göre** kayda geçirir.

**Files:**
- Create: `scripts/measure.ps1`, `docs/performance.md`
- Modify: `src-tauri/tauri.conf.json` (`additionalBrowserArgs`), `src-tauri/src/main.rs` (tokio çalışan iş parçacığı sayısı), `Cargo.toml` (`profile.release`), `docs/superpowers/specs/2026-10-05-object-explorer-design.md` §1.3, `docs/smoke-test.md` (Ölçümler tablosu başlıkları)

**Interfaces:**
- Consumes: Task 16/Plan 1 ölçüm yöntemi (PowerShell), release derlemesi (`bun tauri build`).
- Produces: `scripts/measure.ps1 -Exe <yol> [-Runs 3]` → her çalıştırma için pencere süresi (ms), **toplam özel çalışma kümesi (MB)**, toplam çalışma kümesi (MB), Rust süreci özel çalışma kümesi (MB); `docs/performance.md` (yöntem, taban çizgisi, denenen ayarlar, seçilen ayar, sonuç, yeni hedef).

**Karar kuralı (insan girdisi gerektirmez):**
1. Seçilen ayarla toplam özel çalışma kümesi **< 150 MB** ise spec hedefi aynen kalır.
2. Değilse spec §1.3'teki bellek satırı şu biçime çevrilir: "**boşta toplam özel çalışma kümesi < X MB**" (X = en iyi ayarlı ölçümün medyanı yukarı **10 MB**'a yuvarlanıp üzerine 10 MB eklenmiş hâli) ve Rust süreci için "< Y MB" (aynı kural, 5 MB adımıyla). Gerekçe `docs/performance.md`'ye yazılır; pgAdmin4 ile karşılaştırma (Plan 1'de ~400 MB+) not edilir.
3. Hiçbir ayar arayüz kullanılabilirliğini bozmamalıdır (kaydırma, DDL editörü, tema geçişi); bozan ayar seçilmez.

- [ ] **Step 1: Ölçüm betiğini yaz**

`scripts/measure.ps1` (yeni):

```powershell
<#
.SYNOPSIS
  pg-tools exe'sinin açılış süresini ve boşta belleğini ölçer.
.PARAMETER Exe   Ölçülecek exe (varsayılan: target\release\pg-tools.exe)
.PARAMETER Runs  Çalıştırma sayısı
#>
param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\pg-tools.exe"),
    [int]$Runs = 3
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path $Exe)) { throw "exe bulunamadı: $Exe" }

function Get-Tree([int]$rootPid) {
    $all = Get-CimInstance Win32_Process
    $ids = New-Object System.Collections.Generic.HashSet[int]
    [void]$ids.Add($rootPid)
    do {
        $added = $false
        foreach ($p in $all) {
            if ($ids.Contains([int]$p.ParentProcessId) -and $ids.Add([int]$p.ProcessId)) { $added = $true }
        }
    } while ($added)
    $ids
}

$results = @()
for ($i = 1; $i -le $Runs; $i++) {
    $appdata = Join-Path $env:TEMP ("pgt-measure-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory $appdata | Out-Null
    $env:APPDATA = $appdata
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process $Exe -PassThru
    while (-not $proc.MainWindowHandle -or $proc.MainWindowHandle -eq 0) { Start-Sleep -Milliseconds 20; $proc.Refresh() }
    $windowMs = $sw.ElapsedMilliseconds
    Start-Sleep -Seconds 8   # içerik çizimi ve WebView2 süreçlerinin yerleşmesi
    $ids = Get-Tree $proc.Id
    $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $ids.Contains([int]$_.IDProcess) }
    $privateWs = ($perf | Measure-Object WorkingSetPrivate -Sum).Sum / 1MB
    $ws = ($perf | Measure-Object WorkingSet -Sum).Sum / 1MB
    $rust = ($perf | Where-Object { [int]$_.IDProcess -eq $proc.Id }).WorkingSetPrivate / 1MB
    $results += [pscustomobject]@{
        Run = $i; WindowMs = $windowMs; TotalPrivateWsMB = [math]::Round($privateWs, 1)
        TotalWsMB = [math]::Round($ws, 1); RustPrivateWsMB = [math]::Round($rust, 1); Processes = $ids.Count
    }
    Get-Process -Id ([int[]]$ids) -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    Remove-Item $appdata -Recurse -Force -ErrorAction SilentlyContinue
}
$results | Format-Table -AutoSize
$med = { param($p) ($results | Sort-Object $p)[[int][math]::Floor($Runs / 2)].$p }
"Medyan: pencere $(& $med 'WindowMs') ms, toplam özel WS $(& $med 'TotalPrivateWsMB') MB, toplam WS $(& $med 'TotalWsMB') MB, Rust özel WS $(& $med 'RustPrivateWsMB') MB"
```

- [ ] **Step 2: Taban çizgisini ölç**

Run (PowerShell): `bun tauri build` ardından `./scripts/measure.ps1 -Runs 5`
Expected: tablo ve `Medyan: …` satırı. Değerleri `docs/performance.md` "Taban çizgisi" bölümüne yaz (makine bilgisi: `Get-CimInstance Win32_Processor | Select Name`, `Get-CimInstance Win32_ComputerSystem | Select TotalPhysicalMemory`).

- [ ] **Step 3: WebView2 ve Rust ayarlarını sırayla dene**

Her denemeden sonra `bun tauri build` + `./scripts/measure.ps1 -Runs 5`; sonucu `docs/performance.md` tablosuna ekle. Denenecek ayarlar (her biri tek başına, sonra en iyilerinin birleşimi):

1. **WebView2 argümanları** — `src-tauri/tauri.conf.json` içinde `app.windows[0]`'a `additionalBrowserArgs` ekle. Bu alan Tauri'nin varsayılan argümanlarının yerine geçer, bu yüzden varsayılanları koru:
   - A: `"--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --renderer-process-limit=1"`
   - B: A + `" --disable-gpu --disable-gpu-compositing"` (yazılım çizimi; kaydırma akıcılığını **elle** doğrula)
   - C: A + `" --js-flags=--max-old-space-size=64"`
2. **Rust tarafı** — `src-tauri/src/main.rs` içinde tauri çalışma zamanını iki işçi iş parçacığıyla kur (`main` içinde `tauri::Builder` çağrısından önce):

   ```rust
   let runtime = tokio::runtime::Builder::new_multi_thread()
       .worker_threads(2)
       .enable_all()
       .build()
       .expect("tokio çalışma zamanı kurulamadı");
   tauri::async_runtime::set(runtime.handle().clone());
   ```

   (`runtime` değişkeni `main` boyunca yaşamalıdır; `src-tauri/Cargo.toml`'a `tokio` zaten bağımlıdır.) Kök `Cargo.toml` `[profile.release]` bölümüne `panic = "abort"` ve `opt-level = "s"` ekleyerek de ayrıca dene.

Her ayar için: açılış süresi **< 1000 ms** kalmalı, pencere çizimi bozulmamalı, ağaç/DDL kullanımı normal çalışmalı (Task 16 duman testinin ilgili maddeleri). Bozan ayarı **geri al** ve nedenini `docs/performance.md`'ye yaz.

- [ ] **Step 4: Ayarı seç, hedefi kaydet**

En düşük toplam özel çalışma kümesini veren **güvenli** ayar birleşimini `tauri.conf.json` / `main.rs` / `Cargo.toml`'da bırak, diğerlerini geri al. Karar kuralını uygula:
- `docs/performance.md`: yöntem, taban çizgisi, her denemenin sonucu, seçilen ayar, sonuç, **yeni hedefler** ve gerekçe.
- `docs/superpowers/specs/2026-10-05-object-explorer-design.md` §1.3 tablosunda iki satırı güncelle (örnek biçim; değerler kural 1–2'ye göre): "Boşta toplam bellek (uygulama + WebView2 süreçleri, **özel çalışma kümesi**) | < X MB; Rust süreci < Y MB" ve altına bir dipnot: "Metrik Plan 2'de çalışma kümesinden özel çalışma kümesine çevrildi; çalışma kümesi paylaşılan WebView2 sayfalarını çift sayar. Ayrıntı: docs/performance.md".
- `docs/smoke-test.md` "Ölçümler" tablosunun başlığını `Toplam özel WS (MB)` / `Rust özel WS (MB)` olarak güncelle ve yeni ölçümü ekle.

- [ ] **Step 5: Doğrula**

Run: `bun run typecheck && bun run test && cargo test --workspace --all-features` (env'li) ve `cargo build -p pg-tools --release`
Expected: PASS / başarılı. `./scripts/measure.ps1 -Runs 3` son ayarla yeniden çalıştırılıp sonuç belgelenen değerle uyuşmalı.

- [ ] **Step 6: Commit**

```bash
git add scripts/measure.ps1 docs src-tauri Cargo.toml
git commit -m "perf: WebView2/Rust bellek ince ayarı, ölçüm betiği ve bellek hedefinin yeniden tanımı"
```

---

## Plan sonrası kabul ölçütleri

Plan 2 şu koşullarda tamamlanmış sayılır:

1. `cargo test --workspace --all-features` (gerçek PG 18 ile) ve `bun run test`/`typecheck`/`build` yeşil; `scripts/test-matrix.ps1` PG 10–18'in hepsinde yeşil.
2. `tests/roundtrip.rs` geçiyor: fixture'daki tüm nesnelerin DDL'i boş veritabanında uygulanınca katalog imzası eşit.
3. `tests/perf.rs` spec bütçeleri içinde (liste < 300 ms, DDL < 200 ms).
4. Gerçek uygulamada (CDP duman testi, Plan 1 sonundaki yöntemle) ağaçta yeni koleksiyonlar görünüyor ve Properties/DDL açılıyor; bağlantı koptuktan sonra Inspector hatası sunucuyu "bağlı değil" yapıyor.
5. `docs/performance.md` ve spec §1.3 ölçülen sonuca göre güncel.

## Self-review (plan yazımı sonrası)

- **Spec kapsama (§5.1 ağaç):** Constraints, Indexes, Triggers, RLS Policies, Partitions → Task 2 (liste), 3 (Properties), 4 (DDL), 5 (tablo DDL içinde tam); Views/MatViews (+Columns, +Indexes) → Task 2–4; Functions/Procedures/Trigger Functions → Task 2–4; Sequences, Types/Domains → Task 2–4; Extensions → Task 2–4; Roles, Tablespaces → Task 2–4. §5.5 Table DDL'in tüm bileşenleri → Task 5 (partition, INHERITS, UNLOGGED/WITH/TABLESPACE Plan 1'de, index, trigger, RLS, policy, kolon GRANT, identity). §8 DDL gidiş-dönüş → Task 7; sürüm matrisi → Task 9; performans → Task 8. Plan 1'den kalanlar: bağlantı-koptu göstergesi → Task 6; bellek hedefi → Task 10.
- **Tip tutarlılığı:** `Seg`/`Coll`/`NodeKind` adları Task 1'de tanımlandı ve sonraki task'larda aynen kullanıldı (`Coll::Policies`, `Seg::Policy`, `NodeKind::PolicyCollection`; partition düğümleri `NodeKind::Table`). `CollDef`/`PropsDef`/`DdlDef` alanları Task 2/3/4'te tutarlı. `ColumnDef.identity_opts` Task 5'te hem struct, `from_row`, SQL (iki varyant) ve testte var.
- **Bilinçli ödünler:** Task 5, Plan 1'in `ddl_executes_in_scratch_database` testini siler (Task 7 daha güçlü karşılığıdır); PG 18 adlandırılmış NOT NULL adları kapsam dışı (planın başındaki "bilinen sınırlar").

