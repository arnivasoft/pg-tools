# Nesne Arama (Plan 32) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bir veritabanında nesneleri adına göre arayan tembel bir diyalog ve sonuçtan Object Explorer ağacındaki düğüme gitme.

**Architecture:** Arka uç `pgt-catalog`'a `search` modülü eklenir: seçilen türlerin sabit SQL parçalarından tek bir `UNION ALL` sorgusu kurulur (kullanıcı metni yalnız parametredir), bölüm zincirleri ikinci bir sorguyla okunur, düğüm kimliği ve ata zinciri listelemedeki `NodeId` yapıcılarıyla üretilir. Tauri komutu `search_objects`. Ön yüzde diyalog, model, metinler ve komut sarmalayıcısı tembel parçadadır; ana pakete yalnız bağlam menüsü eylemi, `App.tsx` kancası ve `ExplorerController.reveal` girer, bunun baytları yalnız tembel parçaların kullandığı komutlar `commands.ts`'ten taşınarak telafi edilir.

**Tech Stack:** Rust (tokio-postgres, ts-rs), Tauri 2, SolidJS, Kobalte (`ui/Dialog`), Tailwind 4, vitest.

**Spec:** `docs/superpowers/specs/2026-11-17-object-search-design.md`

## Global Constraints

- PostgreSQL 10–18; sürüm farkları `PgVersion::major()` ile (PG10: `prokind` yok, `proisagg` var; procedure yok).
- Arama metni SQL metnine **asla** eklenmez; yalnız parametredir. SQL yalnız `search.rs` içindeki sabit parçalardan kurulur.
- Arama metni ve nesne adları günlüğe (`tracing`, `console`), `localStorage`'a, toast'a yazılmaz; günlüğe yalnız sayı ve süre.
- Ana paket tavanı (`scripts/bundle-budget.json`) **yükseltilmez**; `bun run bundle:check` her ön yüz görevinin sonunda geçmeli. Ana pakete eklenen baytlar yalnız tembel parçaların kullandığı kodun ana paketten çıkarılmasıyla telafi edilir; telafi yetmezse dur ve sor.
- Tembel modüller `app/strings.ts`, `explorer/*`, `tree-model` gibi ana modülleri **içe aktarmaz** (Rolldown: ortak parça bölünür ve ön yükleme listesi büyür); ihtiyaç duyduklarını App'ten props/deps ile alır. Tür içe aktarmaları (`import type`) serbesttir.
- Arayüz düğüm kimliğini ayrıştırmaz ve kurmaz; kimlik ve ata zinciri arka uçtan gelir.
- Arayüz metinleri Türkçe; yeni tembel metinler `src/app/strings-search.ts` (`ts`) içinde.
- Testler: `cargo test -p pgt-catalog`, Docker matrisi yalnız `scripts/test-matrix.ps1 -Versions 10,11,12,13,14,15,16,17,18 -SkipPerf` ile (geçici `pgt-matrix-*` kapları; test URL'leri ve parolalar yazdırılmaz). `bun run typecheck`, `npx vitest run`, `bun run build`, `bun run bundle:check`, `cargo clippy --all-targets`.
- Satır sonları: depo dosyaları CRLF; yeni dosyalar da CRLF (`.gitattributes`/mevcut dosyalarla tutarlı).
- crate genelinde `cargo fmt` çalıştırma; yalnız dokunduğun dosyaları biçimle.
- Commit sonu: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. `graphify-out/` ve `.superpowers/` commit'lenmez.

## Review Focus

1. **Ağaçta bulunamayan sonuç:** her türün `WHERE`'i listeleme SQL'inden saparsa (ör. iç trigger, kimlik sequence'ı, tablo satır tipi, PG18 not-null kısıtı `contype='n'`) sonuç ağaçta yoktur. Entegrasyon testi her sonucun zincirini `list_children` ile yürür (Task 2).
2. **İç içe bölümler ve farklı şemadaki bölüm:** zincir kök tablonun şemasından geçer, bölümün kendi şemasından değil (Task 1 birim + Task 2 entegrasyon).
3. **`%`, `_`, `\` ve Türkçe harf içeren arama:** joker olarak yorumlanmamalı; `ılık` / `ILIK` eşleşmesi sunucunun `lower`'ına bağlıdır, testte büyük/küçük ASCII ve `_` içeren ad kullanılır (Task 1 birim + Task 2).
4. **Eskimiş ağaç:** arama sonrası eklenen nesnenin listesi ağaçta önceden yüklüyse `reveal` o listeyi bir kez yeniden yükler; silinmiş nesnede durur ve `false` döner (Task 4).
5. **Yarış:** hızlı yazımda eski aramanın yanıtı yenisinin üstüne yazılmamalı (nesil sayacı, Task 5).

---

### Task 1: `pgt-catalog` arama modülü (SQL kurma, zincir, `Catalog::search`)

**Files:**
- Create: `crates/pgt-catalog/src/search.rs`
- Modify: `crates/pgt-catalog/src/lib.rs` (modül + `Catalog::search`)
- Modify: `crates/pgt-catalog/src/db.rs` (`query_text`)
- Test: `crates/pgt-catalog/src/search.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pgt_catalog::search::{SearchKind, SearchHit, SearchResult}` (`ts(export)`, camelCase): `SearchKind` = `schema | table | partition | view | matview | function | procedure | triggerFunction | sequence | type | domain | column | index | constraint | trigger | policy | extension`.
  - `SearchHit { id: String, path: Vec<String>, kind: NodeKind, name: String, schema: Option<String>, owner: Option<String>, system: bool }`, `SearchResult { hits: Vec<SearchHit>, truncated: bool }`.
  - `Catalog::search(&self, server: Uuid, db: u32, query: &str, kinds: &[SearchKind]) -> Result<SearchResult>`.
  - `pub const MAX_QUERY_CHARS: usize = 200; pub const MAX_HITS: usize = 500;`

- [ ] **Step 1: Birim testlerini yaz** (`search.rs` sonunda)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "3f2a6d1e-8a4b-4c2e-9f00-0123456789ab";
    fn srv() -> Uuid { Uuid::parse_str(U).unwrap() }

    #[test]
    fn escapes_like_wildcards() {
        assert_eq!(escape_like("a%b_c\\d"), "a\\%b\\_c\\\\d");
        assert_eq!(escape_like("Müşteri"), "Müşteri");
    }

    #[test]
    fn rejects_long_and_ignores_blank_queries() {
        assert!(matches!(normalize(&"x".repeat(201)), Err(Error::Invalid(_))));
        assert_eq!(normalize("   ").unwrap(), None);
        assert_eq!(normalize("  ad ").unwrap(), Some("ad".to_string()));
    }

    #[test]
    fn builds_sql_only_for_requested_kinds_and_version() {
        let sql = build_sql(&[SearchKind::Table], 16);
        assert!(sql.contains("'table'"));
        assert!(!sql.contains("'column'"));
        let all = build_sql(&[], 10);
        assert!(!all.contains("'procedure'"), "PG10'da procedure yok");
        assert!(all.contains("proisagg"), "PG10 fonksiyon süzgeci");
        let all16 = build_sql(&[], 16);
        assert!(all16.contains("'procedure'"));
        assert!(all16.contains("prokind"));
        assert!(!all16.contains("$q"), "kullanıcı metni SQL'e girmez; yalnız $1..$4");
    }

    fn raw(kind: SearchKind, rel_kind: &str) -> RawHit {
        RawHit {
            kind, oid: 50, attnum: 0, name: "n".into(), schema_oid: 2200, schema_name: Some("public".into()),
            rel_oid: 40, rel_kind: rel_kind.into(), owner: Some("t".into()), system: false,
        }
    }

    #[test]
    fn table_column_path_matches_tree_ids() {
        let mut h = raw(SearchKind::Column, "r");
        h.attnum = 3;
        let hit = to_hit(srv(), 16384, &h, &Default::default());
        let b = format!("srv:{U}/db:16384/sch:2200");
        assert_eq!(hit.id, format!("{b}/tbl:40/col:3"));
        assert_eq!(
            hit.path,
            vec![
                format!("srv:{U}/dbs"), format!("srv:{U}/db:16384"), format!("srv:{U}/db:16384/schemas"), b.clone(),
                format!("{b}/tables"), format!("{b}/tbl:40"), format!("{b}/tbl:40/columns"),
            ]
        );
        assert_eq!(hit.kind, NodeKind::Column);
    }

    #[test]
    fn partition_chain_goes_through_root_schema() {
        // tbl:50 bölüm; zincir kök 30 (şema 2201) → 40 → 50.
        let mut h = raw(SearchKind::Partition, "r");
        h.oid = 50;
        h.rel_oid = 50;
        let chains = std::collections::HashMap::from([(50u32, Chain { root_schema: 2201, parents: vec![30, 40] })]);
        let hit = to_hit(srv(), 1, &h, &chains);
        let s = format!("srv:{U}/db:1/sch:2201");
        assert_eq!(hit.id, format!("{s}/tbl:30/tbl:40/tbl:50"));
        assert_eq!(
            hit.path[3..].to_vec(),
            vec![s.clone(), format!("{s}/tables"), format!("{s}/tbl:30"), format!("{s}/tbl:30/partitions"),
                 format!("{s}/tbl:30/tbl:40"), format!("{s}/tbl:30/tbl:40/partitions")]
        );
    }

    #[test]
    fn view_and_matview_owners() {
        let mut c = raw(SearchKind::Column, "v");
        c.attnum = 1;
        let col = to_hit(srv(), 1, &c, &Default::default());
        assert!(col.id.ends_with("/sch:2200/view:40/col:1"), "koleksiyon parçası nesne kimliğinde yok");
        assert!(col.path.iter().any(|p| p.ends_with("/sch:2200/views")));
        let i = raw(SearchKind::Index, "m");
        let hit = to_hit(srv(), 1, &i, &Default::default());
        assert!(hit.id.ends_with("/mv:40/idx:50"));
        assert!(hit.path.iter().any(|p| p.ends_with("/matviews")));
        assert!(hit.path.last().unwrap().ends_with("/mv:40/indexes"));
    }

    #[test]
    fn schema_level_and_db_level_kinds() {
        let f = to_hit(srv(), 1, &raw(SearchKind::TriggerFunction, ""), &Default::default());
        assert!(f.id.ends_with("/sch:2200/tfn:50"));
        assert!(f.path.last().unwrap().ends_with("/sch:2200/trigfuncs"));
        let mut e = raw(SearchKind::Extension, "");
        e.schema_name = None;
        let hit = to_hit(srv(), 1, &e, &Default::default());
        assert_eq!(hit.id, format!("srv:{U}/db:1/ext:50"));
        assert_eq!(hit.path.last().unwrap(), &format!("srv:{U}/db:1/extensions"));
        let s = to_hit(srv(), 1, &raw(SearchKind::Schema, ""), &Default::default());
        assert_eq!(s.id, format!("srv:{U}/db:1/sch:50"));
    }
}
```

- [ ] **Step 2: Testi çalıştır, derleme hatasıyla başarısız olduğunu gör**

Run: `cargo test -p pgt-catalog --lib search`
Expected: FAIL (`search` modülü yok)

- [ ] **Step 3: `db.rs`'e dinamik metinli sorgu ekle**

```rust
/// Sabit parçalardan çalışma anında kurulan SQL için `query` (ör. `search.rs`). Metin yalnız kod sabitlerinden
/// oluşmalıdır; kullanıcı verisi her zaman parametredir.
pub(crate) async fn query_text(mgr: &ConnectionManager, server: Uuid, target: Target, sql: Arc<String>, params: Params) -> Result<Vec<Row>> {
    mgr.run(server, target, move |c| {
        let params = params.clone();
        let sql = sql.clone();
        async move {
            let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p.as_ref() as &(dyn ToSql + Sync)).collect();
            c.client.query(sql.as_str(), &refs).await
        }
    })
    .await
}
```

- [ ] **Step 4: `search.rs`'i yaz**

Parametreler: `$1` = `%<kaçırılmış>%` (ILIKE deseni), `$2` = `lower(<metin>)` (tam eşleşme), `$3` = `<kaçırılmış lower>%` (önek), `$4` = `show_system` (bool). Her parça aynı 11 sütunu döndürür: `kind text, oid oid, attnum int2, name text, match_name text, schema_oid oid, schema_name text, rel_oid oid, rel_kind text, owner text, is_system bool`.

```rust
//! Nesne arama (spec 2026-11-17-object-search-design.md). SQL yalnız bu dosyadaki sabit parçalardan kurulur;
//! arama metni her zaman parametredir. Süzgeçler listeleme SQL'leriyle (`sql/list_*.sql`) birebir aynıdır:
//! sapma, sonucun ağaçta bulunamaması demektir.
use std::collections::HashMap;
use std::sync::Arc;

use pgt_core::connection::Target;
use pgt_core::{Error, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query_text};
use crate::node::{Coll, NodeId, NodeKind, Seg};
use crate::version::PgVersion;

pub const MAX_QUERY_CHARS: usize = 200;
pub const MAX_HITS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SearchKind {
    Schema, Table, Partition, View, Matview, Function, Procedure, TriggerFunction, Sequence, Type, Domain,
    Column, Index, Constraint, Trigger, Policy, Extension,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHit {
    pub id: String,
    pub path: Vec<String>,
    pub kind: NodeKind,
    pub name: String,
    pub schema: Option<String>,
    pub owner: Option<String>,
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub truncated: bool,
}

const SYS: &str = "(n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\\_toast%' OR n.nspname LIKE 'pg\\_temp\\_%')";
const EXT: &str = "EXISTS (SELECT 1 FROM pg_depend i WHERE i.classid = 'pg_proc'::regclass AND i.objid = p.oid AND i.deptype = 'i')";

/// `(tür, en düşük ana sürüm, en yüksek ana sürüm, parça)`. `{SYS}` ve `{INT}` yer tutucuları `build_sql`'de
/// sabitlerle değiştirilir. Süzgeçler `sql/list_*.sql` ile aynıdır.
const FRAGMENTS: &[(SearchKind, u32, u32, &str)] = &[
    (SearchKind::Schema, 10, 99, "SELECT 'schema' AS kind, n.oid, 0::int2 AS attnum, n.nspname::text AS name, n.nspname::text AS match_name, n.oid AS schema_oid, n.nspname::text AS schema_name, 0::oid AS rel_oid, ''::text AS rel_kind, NULL::text AS owner, {SYS} AS is_system FROM pg_namespace n WHERE n.nspname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Table, 10, 99, "SELECT 'table', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, NULL::text, {SYS} FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p') AND NOT c.relispartition AND c.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Partition, 10, 99, "SELECT 'partition', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, (SELECT pc.relname::text FROM pg_inherits i JOIN pg_class pc ON pc.oid = i.inhparent WHERE i.inhrelid = c.oid LIMIT 1), {SYS} FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relispartition AND c.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::View, 10, 99, "SELECT 'view', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, c.oid, 'v', NULL::text, {SYS} FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'v' AND c.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Matview, 10, 99, "SELECT 'matview', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, c.oid, 'm', NULL::text, {SYS} FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'm' AND c.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Function, 10, 10, "SELECT 'function', p.oid, 0::int2, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')', p.proname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE NOT p.proisagg AND p.prorettype <> 'trigger'::regtype AND NOT {INT} AND p.proname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Function, 11, 99, "SELECT 'function', p.oid, 0::int2, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')', p.proname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE p.prokind IN ('f', 'w') AND p.prorettype <> 'trigger'::regtype AND NOT {INT} AND p.proname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Procedure, 11, 99, "SELECT 'procedure', p.oid, 0::int2, p.proname::text || '(' || regexp_replace(pg_get_function_identity_arguments(p.oid), '(^|, )IN ', '\\1', 'g') || ')', p.proname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE p.prokind = 'p' AND NOT {INT} AND p.proname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::TriggerFunction, 10, 10, "SELECT 'triggerFunction', p.oid, 0::int2, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')', p.proname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE NOT p.proisagg AND p.prorettype = 'trigger'::regtype AND NOT {INT} AND p.proname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::TriggerFunction, 11, 99, "SELECT 'triggerFunction', p.oid, 0::int2, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')', p.proname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE p.prokind IN ('f', 'w') AND p.prorettype = 'trigger'::regtype AND NOT {INT} AND p.proname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Sequence, 10, 99, "SELECT 'sequence', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'S' AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'i') AND c.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Type, 10, 99, "SELECT 'type', t.oid, 0::int2, t.typname::text, t.typname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE t.typtype IN ('e', 'c', 'r', 'b') AND (t.typtype <> 'c' OR EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c')) AND NOT EXISTS (SELECT 1 FROM pg_type el WHERE el.oid = t.typelem AND el.typarray = t.oid) AND t.typname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Domain, 10, 99, "SELECT 'domain', t.oid, 0::int2, t.typname::text, t.typname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE t.typtype = 'd' AND t.typname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Column, 10, 99, "SELECT 'column', 0::oid, a.attnum, a.attname::text, a.attname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, c.relname::text, {SYS} FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p', 'v', 'm') AND a.attnum > 0 AND NOT a.attisdropped AND a.attname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Index, 10, 99, "SELECT 'index', x.indexrelid, 0::int2, ic.relname::text, ic.relname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, c.relname::text, {SYS} FROM pg_index x JOIN pg_class ic ON ic.oid = x.indexrelid JOIN pg_class c ON c.oid = x.indrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p', 'm') AND ic.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Constraint, 10, 99, "SELECT 'constraint', k.oid, 0::int2, k.conname::text, k.conname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, c.relname::text, {SYS} FROM pg_constraint k JOIN pg_class c ON c.oid = k.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p') AND k.contype IN ('p', 'u', 'c', 'x', 'f') AND k.conname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Trigger, 10, 99, "SELECT 'trigger', g.oid, 0::int2, g.tgname::text, g.tgname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, c.relname::text, {SYS} FROM pg_trigger g JOIN pg_class c ON c.oid = g.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p') AND NOT g.tgisinternal AND g.tgname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Policy, 10, 99, "SELECT 'policy', o.oid, 0::int2, o.polname::text, o.polname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, c.relname::text, {SYS} FROM pg_policy o JOIN pg_class c ON c.oid = o.polrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p') AND o.polname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Extension, 10, 99, "SELECT 'extension', e.oid, 0::int2, e.extname::text, e.extname::text, 0::oid, NULL::text, 0::oid, '', NULL::text, false FROM pg_extension e WHERE e.extname ILIKE $1 ESCAPE '\\'"),
];

pub(crate) fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Kırpılmış metin; boşsa `None`; `MAX_QUERY_CHARS` üstü `Error::Invalid`.
pub(crate) fn normalize(query: &str) -> Result<Option<String>> {
    let q = query.trim();
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(Error::Invalid(format!("arama en fazla {MAX_QUERY_CHARS} karakter olabilir")));
    }
    Ok((!q.is_empty()).then(|| q.to_string()))
}

/// Seçilen türlerin (boş = hepsi) sürüme uyan parçalarından tek sorgu. Sıralama: tam eşleşme, önek, ad, şema, tür.
pub(crate) fn build_sql(kinds: &[SearchKind], major: u32) -> String {
    let parts: Vec<String> = FRAGMENTS
        .iter()
        .filter(|(k, min, max, _)| (kinds.is_empty() || kinds.contains(k)) && major >= *min && major <= *max)
        .map(|(_, _, _, sql)| sql.replace("{SYS}", SYS).replace("{INT}", EXT))
        .collect();
    format!(
        "SELECT * FROM ({}) h WHERE ($4 OR NOT h.is_system) \
         ORDER BY CASE WHEN lower(h.match_name) = $2 THEN 0 WHEN lower(h.match_name) LIKE $3 ESCAPE '\\' THEN 1 ELSE 2 END, \
         h.name COLLATE \"C\", h.schema_name COLLATE \"C\", h.kind LIMIT {}",
        parts.join(" UNION ALL "),
        MAX_HITS + 1
    )
}
```

Kalan kısım (aynı dosyada): `RawHit` (yukarıdaki 11 sütun; `kind` metinden `SearchKind`'a), `Chain { root_schema: u32, parents: Vec<u32> }` (kökten doğrudan ebeveyne), `to_hit(server, db, &RawHit, &HashMap<u32, Chain>) -> SearchHit` ve `Catalog::search`:

```rust
#[derive(Debug, Clone)]
pub(crate) struct RawHit {
    pub kind: SearchKind,
    pub oid: u32,
    pub attnum: i16,
    pub name: String,
    pub schema_oid: u32,
    pub schema_name: Option<String>,
    pub rel_oid: u32,
    pub rel_kind: String,
    pub owner: Option<String>,
    pub system: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Chain {
    pub root_schema: u32,
    pub parents: Vec<u32>,
}

/// İlişkinin (tablo/bölüm/view/MV) düğümü ve o düğüme kadar olan zincir (ilişkinin kendisi dahil).
fn rel_node(schema: &NodeId, path: &mut Vec<NodeId>, rel_oid: u32, rel_kind: &str, chain: Option<&Chain>) -> NodeId {
    let (coll, seg) = match rel_kind {
        "v" => (Coll::Views, Seg::View(rel_oid)),
        "m" => (Coll::MatViews, Seg::MatView(rel_oid)),
        _ => (Coll::Tables, Seg::Table(rel_oid)),
    };
    path.push(schema.child(Seg::Coll(coll)));
    let mut cur = schema.clone();
    if let Some(ch) = chain {
        for parent in &ch.parents {
            cur = cur.child(Seg::Table(*parent));
            path.push(cur.clone());
            path.push(cur.child(Seg::Coll(Coll::Partitions)));
        }
    }
    let node = cur.child(seg);
    path.push(node.clone());
    node
}

pub(crate) fn to_hit(server: Uuid, db: u32, h: &RawHit, chains: &HashMap<u32, Chain>) -> SearchHit {
    let srv = NodeId::server(server);
    let db_id = srv.child(Seg::Db(db));
    let mut path = vec![srv.child(Seg::Coll(Coll::Databases)), db_id.clone()];
    let chain = chains.get(&h.rel_oid);
    // Bölümler kök tablonun şemasından geçer (bölümün kendi şeması farklı olabilir).
    let schema_oid = chain.map(|c| c.root_schema).unwrap_or(h.schema_oid);
    let schema_id = db_id.child(Seg::Schema(schema_oid));
    let in_schema = |path: &mut Vec<NodeId>| {
        path.push(db_id.child(Seg::Coll(Coll::Schemas)));
        path.push(schema_id.clone());
    };
    let (id, kind) = match h.kind {
        SearchKind::Schema => {
            path.push(db_id.child(Seg::Coll(Coll::Schemas)));
            (db_id.child(Seg::Schema(h.oid)), NodeKind::Schema)
        }
        SearchKind::Extension => {
            path.push(db_id.child(Seg::Coll(Coll::Extensions)));
            (db_id.child(Seg::Extension(h.oid)), NodeKind::Extension)
        }
        SearchKind::Table | SearchKind::Partition | SearchKind::View | SearchKind::Matview => {
            in_schema(&mut path);
            let node = rel_node(&schema_id, &mut path, h.rel_oid, &h.rel_kind, chain);
            path.pop();
            let kind = match h.kind {
                SearchKind::View => NodeKind::View,
                SearchKind::Matview => NodeKind::MaterializedView,
                _ => NodeKind::Table,
            };
            (node, kind)
        }
        SearchKind::Column | SearchKind::Index | SearchKind::Constraint | SearchKind::Trigger | SearchKind::Policy => {
            in_schema(&mut path);
            let rel = rel_node(&schema_id, &mut path, h.rel_oid, &h.rel_kind, chain);
            let (coll, seg, kind) = match h.kind {
                SearchKind::Column => (Coll::Columns, Seg::Column(h.attnum), NodeKind::Column),
                SearchKind::Index => (Coll::Indexes, Seg::Index(h.oid), NodeKind::Index),
                SearchKind::Constraint => (Coll::Constraints, Seg::Constraint(h.oid), NodeKind::Constraint),
                SearchKind::Trigger => (Coll::Triggers, Seg::Trigger(h.oid), NodeKind::Trigger),
                _ => (Coll::Policies, Seg::Policy(h.oid), NodeKind::Policy),
            };
            path.push(rel.child(Seg::Coll(coll)));
            (rel.child(seg), kind)
        }
        k => {
            in_schema(&mut path);
            let (coll, seg, kind) = match k {
                SearchKind::Function => (Coll::Functions, Seg::Function(h.oid), NodeKind::Function),
                SearchKind::Procedure => (Coll::Procedures, Seg::Procedure(h.oid), NodeKind::Procedure),
                SearchKind::TriggerFunction => (Coll::TriggerFunctions, Seg::TriggerFunction(h.oid), NodeKind::TriggerFunction),
                SearchKind::Sequence => (Coll::Sequences, Seg::Sequence(h.oid), NodeKind::Sequence),
                SearchKind::Type => (Coll::Types, Seg::Type(h.oid), NodeKind::Type),
                _ => (Coll::Domains, Seg::Domain(h.oid), NodeKind::Domain),
            };
            path.push(schema_id.child(Seg::Coll(coll)));
            (schema_id.child(seg), kind)
        }
    };
    SearchHit {
        id: id.to_string(),
        path: path.iter().map(ToString::to_string).collect(),
        kind,
        name: h.name.clone(),
        schema: h.schema_name.clone(),
        owner: h.owner.clone(),
        system: h.system,
    }
}
```

> Not: bölüm hit'inde (`SearchKind::Partition`) `rel_oid == oid`'dir; zincir `chains[oid]`'den gelir ve `rel_node` bölümün kendisini en sona koyar, sonra `path.pop()` onu zincirden çıkarır (düğümün kendisi `path`'te olmaz). Testteki `partition_chain_goes_through_root_schema` bunu doğrular.

Bölüm zinciri sorgusu (yalnız sonuçlarda `rel_kind IN ('r','p')` olan ve `relispartition` ilişkiler için; sonuçta bölüm varsa çalışır):

```sql
WITH RECURSIVE up(child, parent, depth) AS (
  SELECT i.inhrelid, i.inhparent, 1 FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
  WHERE i.inhrelid = ANY($1::oid[]) AND c.relispartition
  UNION ALL
  SELECT u.child, i.inhparent, u.depth + 1 FROM up u JOIN pg_inherits i ON i.inhrelid = u.parent
  JOIN pg_class c ON c.oid = i.inhrelid WHERE c.relispartition
)
SELECT u.child, array_agg(u.parent ORDER BY u.depth DESC) AS chain,
       (SELECT r.relnamespace FROM pg_class r WHERE r.oid = (array_agg(u.parent ORDER BY u.depth DESC))[1]) AS root_schema
FROM up u GROUP BY u.child
```

`Catalog::search`:

```rust
impl Catalog {
    pub async fn search(&self, server: Uuid, db: u32, query: &str, kinds: &[SearchKind]) -> Result<SearchResult> {
        let Some(q) = normalize(query)? else { return Ok(SearchResult { hits: Vec::new(), truncated: false }) };
        if !self.mgr.is_connected(server) {
            return Err(Error::NotConnected);
        }
        let major = PgVersion(self.mgr.info(server).ok_or(Error::NotConnected)?.version_num).major();
        let started = std::time::Instant::now();
        let lower = q.to_lowercase();
        let sql = Arc::new(build_sql(kinds, major));
        let params = p![format!("%{}%", escape_like(&q)), lower.clone(), format!("{}%", escape_like(&lower)), self.show_system()];
        let rows = query_text(&self.mgr, server, Target::Database(db), sql, params).await?;
        let truncated = rows.len() > MAX_HITS;
        let raw: Vec<RawHit> = rows.iter().take(MAX_HITS).map(RawHit::from_row).collect::<Result<_>>()?;
        let parts: Vec<u32> = raw.iter().filter(|h| h.rel_oid != 0 && matches!(h.rel_kind.as_str(), "r" | "p")).map(|h| h.rel_oid).collect();
        let chains = if parts.is_empty() { HashMap::new() } else { load_chains(self, server, db, parts).await? };
        let hits = raw.iter().map(|h| to_hit(server, db, h, &chains)).collect();
        tracing::debug!(count = raw.len(), truncated, ms = started.elapsed().as_millis() as u64, "nesne araması");
        Ok(SearchResult { hits, truncated })
    }
}
```

`RawHit::from_row` metin sütunlarını `pgt_core::pgtext::lossy`/`lossy_opt` ile okur (geçersiz UTF-8 ad listelemede de kayıplı gösterilir); bilinmeyen `kind` metni `Error::Invalid`. `load_chains` yukarıdaki SQL'i `VersionedSql` olarak (`static CHAINS: VersionedSql = VersionedSql(&[(10, "...")])`) ve `p![parts]` (`Vec<u32>` → `oid[]`) ile çalıştırır; `chain: Vec<u32>`, `root_schema: u32`.

`lib.rs`: `pub mod search;` ve `Catalog::search`'i `search.rs`'te `impl Catalog` olarak tanımla (ya da `lib.rs`'te ince sarmalayıcı). `show_system()` `pub(crate)`'tir; aynı crate'te erişilebilir.

- [ ] **Step 5: Testleri çalıştır**

Run: `cargo test -p pgt-catalog --lib search`
Expected: PASS. Ardından `cargo clippy -p pgt-catalog --all-targets` uyarısız.

- [ ] **Step 6: ts-rs çıktılarını üret ve commit**

Run: `cargo test -p pgt-catalog` (ts-rs `export_bindings_*` testleri `src/ipc/bindings/SearchKind.ts`, `SearchHit.ts`, `SearchResult.ts` dosyalarını yazar). Yeni bağlama dosyalarını commit'e ekle.

```bash
git add crates/pgt-catalog/src/search.rs crates/pgt-catalog/src/lib.rs crates/pgt-catalog/src/db.rs src/ipc/bindings/SearchKind.ts src/ipc/bindings/SearchHit.ts src/ipc/bindings/SearchResult.ts
git commit -m "feat(search): katalogda ada göre nesne arama ve ağaç zinciri"
```

---

### Task 2: Entegrasyon testi (Docker PG 10–18) — her sonucun zinciri ağaçta yürünür

**Files:**
- Create: `crates/pgt-catalog/tests/search.rs`
- Create: `crates/pgt-catalog/tests/fixtures/search.sql`
- Modify: `crates/pgt-catalog/tests/common/mod.rs` (fikstüre `search.sql`'i ekle; PG11+/12+ parçaları mevcut desenle)

**Interfaces:**
- Consumes: `Catalog::search`, `Catalog::list_children`, `Catalog::set_show_system`, `common::fixtures()`.

- [ ] **Step 1: Fikstürü yaz** (`search.sql`; mevcut `schema.sql`'den sonra çalışır, yalnız `srch_` önekli nesneler)

```sql
CREATE SCHEMA srch_a;
CREATE SCHEMA srch_b;
CREATE TABLE srch_a.srch_tbl (srch_id int PRIMARY KEY, "Srch_Mixed" text, srch_under_x int,
                              CONSTRAINT srch_chk CHECK (srch_under_x > 0));
CREATE INDEX srch_idx ON srch_a.srch_tbl (srch_under_x);
CREATE TABLE srch_a.srch_ident (id int GENERATED BY DEFAULT AS IDENTITY, srch_ref int REFERENCES srch_a.srch_tbl);
CREATE SEQUENCE srch_a.srch_seq;
CREATE VIEW srch_a.srch_view AS SELECT srch_id AS srch_vcol FROM srch_a.srch_tbl;
CREATE MATERIALIZED VIEW srch_a.srch_mv AS SELECT srch_id AS srch_mvcol FROM srch_a.srch_tbl;
CREATE INDEX srch_mv_idx ON srch_a.srch_mv (srch_mvcol);
CREATE FUNCTION srch_a.srch_fn(a int) RETURNS int LANGUAGE sql AS 'SELECT a';
CREATE FUNCTION srch_a.srch_trgfn() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END';
CREATE TRIGGER srch_trg BEFORE INSERT ON srch_a.srch_tbl FOR EACH ROW EXECUTE PROCEDURE srch_a.srch_trgfn();
CREATE TYPE srch_a.srch_enum AS ENUM ('x');
CREATE DOMAIN srch_a.srch_dom AS int CHECK (VALUE > 0);
ALTER TABLE srch_a.srch_tbl ENABLE ROW LEVEL SECURITY;
CREATE POLICY srch_pol ON srch_a.srch_tbl USING (true);
-- Bölümler: kök srch_a'da, ara ve yaprak bölüm srch_b'de (zincir kök şemadan geçmeli).
CREATE TABLE srch_a.srch_part (k int, srch_pcol int) PARTITION BY RANGE (k);
CREATE TABLE srch_b.srch_part_mid PARTITION OF srch_a.srch_part FOR VALUES FROM (0) TO (100) PARTITION BY RANGE (k);
CREATE TABLE srch_b.srch_part_leaf PARTITION OF srch_b.srch_part_mid FOR VALUES FROM (0) TO (50);
-- Joker karakter içeren ad: "srch%x" araması yalnız bunu bulmalı.
CREATE TABLE srch_a."srch%x" (c int);
CREATE TABLE srch_a.srchzx (c int);
```

PG11+ parçası (`search_pg11.sql`, mevcut `schema_pg11.sql` deseniyle yüklenir): `CREATE PROCEDURE srch_a.srch_proc(IN a int) LANGUAGE sql AS 'SELECT 1';`

> PG10'da `CREATE TRIGGER … EXECUTE PROCEDURE` geçerlidir (PG11+ `FUNCTION` de kabul eder); bölümlü tablo PG10'da desteklenir; `GENERATED … AS IDENTITY` PG10+.

- [ ] **Step 2: Testleri yaz** (`tests/search.rs`)

Yardımcı: `async fn walk(cat: &Catalog, hit: &SearchHit, server_id: &str)` — `parent = server_id` (sunucu düğümü kimliği `NodeId::server(server).to_string()`), her `p` için `list_children(parent)` sonucunda `p` var mı (`assert!`), `parent = p`; sonunda `list_children(parent)` sonucunda `hit.id` var mı. Hata iletisi yalnız tür ve sıra numarasını yazar (nesne adları/test URL'leri yazdırılmaz — adlar fikstürden olduğu için sorun değil ama URL asla).

Testler (her fikstür sunucusunda):
1. `every_hit_is_reachable_in_tree`: `search(server, db_oid, "srch", &[])` → `truncated == false`, sonuç türleri en az şu kümeyi kapsar: schema, table, partition, view, matview, function, triggerFunction, sequence, type, domain, column, index, constraint, trigger, policy (PG11+ procedure). Her sonuç için `walk`.
2. `excludes_hidden_objects`: `srch_ident` tablosunun kimlik sequence'ı (`srch_ident_id_seq`) sonuçta yok; iç trigger'lar (FK `RI_ConstraintTrigger_*`) yok; tablo satır tipi `srch_tbl` `type` olarak yok; dizi tipi `_srch_enum` yok.
3. `partition_chain_uses_root_schema`: `srch_part_leaf` sonucu (`kind == Table`) `path` içinde `srch_a` şemasının kimliğinden geçer, `srch_b`'den geçmez; `walk` geçer.
4. `wildcards_are_literal`: `"srch%x"` → yalnız `srch%x` tablosu (srchzx yok); `"SRCH_MIXED"` → `Srch_Mixed` kolonu (büyük/küçük harf duyarsız); `"_under_"` → `srch_under_x`.
5. `kinds_filter`: `kinds = [Column]` → yalnız kolonlar.
6. `system_objects_follow_setting`: `set_show_system(false)`; `"pg_class"` → sistem sonucu yok; `set_show_system(true)` → `pg_catalog.pg_class` (`system == true`) var ve `walk` geçer; testin sonunda `set_show_system(false)`.
7. `ordering_exact_then_prefix`: `"srch_tbl"` → ilk sonuç `srch_tbl` tablosu.
8. `blank_and_too_long`: `"  "` → boş, `truncated == false`; 201 karakter → `Error::Invalid`.

- [ ] **Step 3: Matrisi çalıştır**

Run (PowerShell): `scripts/test-matrix.ps1 -Versions 10,11,12,13,14,15,16,17,18 -SkipPerf`
Expected: tüm sürümlerde PASS. Kaplar betik sonunda kaldırılır; çıktıda test URL'si/parola görünmez.

- [ ] **Step 4: Commit**

```bash
git add crates/pgt-catalog/tests/search.rs crates/pgt-catalog/tests/fixtures/search.sql crates/pgt-catalog/tests/fixtures/search_pg11.sql crates/pgt-catalog/tests/common/mod.rs
git commit -m "test(search): nesne aramasının her sonucu ağaçta yürünür (PG 10–18)"
```

---

### Task 3: Tauri komutu ve tembel komut sarmalayıcısı

**Files:**
- Modify: `src-tauri/src/commands.rs` (`search_objects`)
- Modify: `src-tauri/src/main.rs` (`generate_handler!`)
- Create: `src/features/search/search-commands.ts`
- Test: `src/features/search/search-commands.test.ts`

**Interfaces:**
- Produces: `searchObjects(serverId: string, databaseOid: number, query: string, kinds: SearchKind[]): Promise<SearchResult>` (hata: `Error` + `ErrorPayload` alanları; `diff-commands.ts` deseni).

- [ ] **Step 1: Komut**

```rust
#[tauri::command]
pub async fn search_objects(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: u32,
    query: String,
    kinds: Vec<pgt_catalog::search::SearchKind>,
) -> CmdResult<pgt_catalog::search::SearchResult> {
    Ok(Box::pin(state.catalog.search(server_id, database_oid, &query, &kinds)).await?)
}
```

`main.rs` `generate_handler!` listesine `commands::search_objects` ekle. `cargo clippy --all-targets` uyarısız.

- [ ] **Step 2: Sarmalayıcı testi** (`search-commands.test.ts`; `@tauri-apps/api/core` `invoke` mock'lanır)

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
import { searchObjects } from "./search-commands";

describe("searchObjects", () => {
  beforeEach(() => invoke.mockReset());
  it("parametre adları arka uçla birebir", async () => {
    invoke.mockResolvedValue({ hits: [], truncated: false });
    await searchObjects("S", 5, "ad", ["table"]);
    expect(invoke).toHaveBeenCalledWith("search_objects", { serverId: "S", databaseOid: 5, query: "ad", kinds: ["table"] });
  });
  it("hata yükünün alanlarını taşır", async () => {
    invoke.mockRejectedValue({ code: "not_connected", message: "Bağlı değil", detail: null, hint: null });
    await expect(searchObjects("S", 5, "ad", [])).rejects.toMatchObject({ code: "not_connected", message: "Bağlı değil" });
  });
});
```

- [ ] **Step 3: Sarmalayıcı**

```ts
// Nesne arama komut sarmalayıcısı (tembel parça; Plan 32). commands.ts'e eklenmez: ana paket büyümez.
import { invoke } from "@tauri-apps/api/core";
import type { SearchKind } from "../../ipc/bindings/SearchKind";
import type { SearchResult } from "../../ipc/bindings/SearchResult";
import { toPayload } from "../../ipc/errors";

export async function searchObjects(serverId: string, databaseOid: number, query: string, kinds: SearchKind[]): Promise<SearchResult> {
  try {
    return await invoke<SearchResult>("search_objects", { serverId, databaseOid, query, kinds });
  } catch (e) {
    const p = toPayload(e);
    throw Object.assign(new Error(p.message), p);
  }
}
```

- [ ] **Step 4: Çalıştır ve commit**

Run: `npx vitest run src/features/search` ve `bun run typecheck` → PASS.

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs src/features/search/search-commands.ts src/features/search/search-commands.test.ts
git commit -m "feat(search): search_objects komutu ve tembel sarmalayıcı"
```

---

### Task 4: `ExplorerController.reveal`

**Files:**
- Modify: `src/features/explorer/controller.ts`
- Test: `src/features/explorer/controller.test.ts`

**Interfaces:**
- Produces: `ExplorerController.reveal(serverId: string, path: string[], id: string): Promise<boolean>` — `true`: düğüm seçildi; `false`: sunucu bağlanamadı ya da zincirde düğüm bulunamadı.

Davranış (spec §2.5):
1. `setFilter("")`.
2. Sunucu düğümü `nodes` içinde `kind === "server" && serverId === serverId` olan düğümdür; yoksa `false`. Sunucu düğümünün ağaçtaki ataları (`parent` zinciri, `ROOT` hariç, kökten aşağı) sırayla `expand`; sonra sunucu düğümü `expand` (bağlanır; başarısızsa `false`).
3. `prev = sunucu düğümü`. Her `p` için ve son olarak `id` için: `p` `nodes`'ta yoksa ve `prev`'in listesi yüklüyse (`children.has(prev)`) bir kez `reload(prev)`; hâlâ yoksa `false`. Zincir adımında `expand(p)`; `prev = p`.
4. `select(id)`; `true`.

- [ ] **Step 1: Testleri yaz** (mevcut `controller.test.ts` kurulumunu kullan: sahte `listChildren` haritası)

Senaryolar:
- Gruptaki bağlı olmayan sunucu: grup açılır, sunucu bağlanır, zincir sırayla `listChildren` ile yüklenir, `selected === id`, `expanded` zincirin tüm düğümlerini içerir.
- Süzgeç doluyken çağrılınca `filter === ""`.
- Zincirde eskimiş liste: ebeveyn önceden yüklü ama hedefi içermiyor; ikinci `listChildren` çağrısı hedefi döndürüyor → `true`, ebeveyn için `listChildren` tam iki kez çağrıldı.
- Silinmiş düğüm: yeniden yüklemede de yok → `false`, seçim değişmedi, `listChildren` o ebeveyn için iki kezden fazla çağrılmadı.
- Sunucu bağlanamıyor (`connectServer` reddediyor, parola istemi `null`) → `false`.
- Bilinmeyen `serverId` → `false`.

- [ ] **Step 2: Başarısız olduğunu gör** — `npx vitest run src/features/explorer/controller.test.ts` → FAIL (`reveal` yok).

- [ ] **Step 3: Uygula**

```ts
  /**
   * Nesne arama sonucuna gider (Plan 32): süzgeci temizler, sunucunun ağaçtaki atalarını ve sunucuyu açar, arka uçtan
   * gelen zinciri (`path`, sunucunun çocuğundan hedefin ebeveynine) sırayla açar ve hedefi seçer. Yüklü ama eskimiş bir
   * liste bir kez yeniden okunur. `false`: sunucu bağlanamadı ya da bir adım ağaçta yok.
   */
  async reveal(serverId: string, path: string[], id: string): Promise<boolean> {
    this.setFilter("");
    const server = [...this.get().nodes.values()].find((n) => n.kind === "server" && n.serverId === serverId);
    if (!server) return false;
    const above: string[] = [];
    for (let cur = this.get().parent.get(server.id); cur !== undefined && cur !== ROOT; cur = this.get().parent.get(cur)) above.unshift(cur);
    for (const a of above) await this.expand(a);
    await this.expand(server.id);
    if (!this.get().expanded.has(server.id)) return false;
    let prev = server.id;
    for (const step of [...path, id]) {
      if (!this.get().nodes.has(step) && this.get().children.has(prev)) await this.reload(prev);
      if (!this.get().nodes.has(step)) return false;
      if (step !== id) await this.expand(step);
      prev = step;
    }
    this.select(id);
    return true;
  }
```

- [ ] **Step 4: Testler PASS; `bun run typecheck`.**

- [ ] **Step 5: Commit**

```bash
git add src/features/explorer/controller.ts src/features/explorer/controller.test.ts
git commit -m "feat(explorer): arama sonucuna gitmek için reveal"
```

---

### Task 5: Arama modeli (tembel)

**Files:**
- Create: `src/features/search/search-model.ts`
- Create: `src/app/strings-search.ts`
- Test: `src/features/search/search-model.test.ts`

**Interfaces:**
- Consumes: `searchObjects` (Task 3, deps ile enjekte).
- Produces:

```ts
export type SearchDeps = {
  search: (serverId: string, databaseOid: number, query: string, kinds: SearchKind[]) => Promise<SearchResult>;
  connect: (serverId: string) => Promise<boolean>;
  setTimer: (fn: () => void, ms: number) => unknown;
  clearTimer: (h: unknown) => void;
};
export type SearchState = {
  query: string;
  kinds: SearchKind[];          // boş = tümü
  status: "idle" | "searching" | "done" | "error";
  hits: SearchHit[];
  truncated: boolean;
  error: string | null;         // errorText çıktısı
  active: number;               // seçili sonuç dizini, -1 = yok
};
export const SEARCH_DELAY_MS = 250;
export function createSearch(deps: SearchDeps, target: { serverId: string; databaseOid: number }): {
  state: Accessor<SearchState>;
  setQuery(q: string): void;    // gecikmeli arama
  searchNow(): void;            // Enter
  toggleKind(k: SearchKind): void;  // arama yeniden çalışır
  setAllKinds(): void;
  move(delta: 1 | -1): void;    // active'i sınırlar içinde kaydırır
  activeHit(): SearchHit | null;
  dispose(): void;
};
export const ALL_KINDS: SearchKind[];  // spec §2.1 sırası
```

Kurallar:
- Boş (kırpılmış) sorgu: arama yok, `status: "idle"`, sonuçlar temizlenir.
- 200 karakter üstü: ön yüzde de engellenir (`error: ts.tooLong`, arama yok).
- Nesil sayacı: her aramada artar; eski yanıt/hata yok sayılır.
- `not_connected` hatası: `deps.connect(serverId)` → `true` ise **bir kez** yeniden dener; aksi halde hata metni.
- Yeni sonuçta `active` 0 (sonuç varsa) ya da -1.
- Hafıza: modül düzeyinde `Map<string, { query, kinds }>` anahtarı `${serverId}:${databaseOid}`; `createSearch` açılışta okur (ve sorgu boş değilse hemen arar), her değişiklikte yazar. `localStorage` yok.
- Hata metni: `errorText(toPayload(e))` (`ipc/errors.ts`; ana pakette zaten var, içe aktarması serbest).

`strings-search.ts` (`ts`): `title(db, server)`, `placeholder: "Ad ara…"`, `hint: "En az bir karakter yazın."`, `searching: "Aranıyor…"`, `none: "Sonuç yok."`, `count(n)`, `truncated: "İlk 500 sonuç gösteriliyor; aramayı daraltın."`, `tooLong: "Arama en fazla 200 karakter olabilir."`, `all: "Tümü"`, `kinds: Record<SearchKind, string>` (Şema, Tablo, Bölüm, View, Materialized view, Fonksiyon, Procedure, Trigger fonksiyonu, Sequence, Tip, Domain, Kolon, İndeks, Kısıt, Trigger, Policy, Extension), `notFound: "Nesne ağaçta bulunamadı; listeyi yenileyin."`, `listLabel: "Arama sonuçları"`, `filterLabel: "Türler"`, `chunkLoadFailed: "Arama penceresi yüklenemedi."`.

- [ ] **Step 1: Testleri yaz** (sahte zamanlayıcı ve ertelenmiş sözler):
  - 250 ms içinde üç `setQuery` → tek `search` çağrısı, son metinle.
  - `searchNow` beklemeden arar.
  - Eski aramanın yanıtı yenisinden sonra gelirse yok sayılır.
  - `toggleKind("column")` → `kinds === ["column"]`, yeniden arar; `setAllKinds` → `[]`.
  - Boş sorgu → `idle`, `search` çağrılmaz.
  - 201 karakter → `error === ts.tooLong`, `search` çağrılmaz.
  - `not_connected` → `connect` çağrılır, `true` ise bir kez yeniden arar; ikinci `not_connected`'da hata metni (sonsuz döngü yok).
  - `move` sınırlarda durur; `activeHit`.
  - Hafıza: aynı hedefle ikinci `createSearch` son sorgu/türleri geri getirir ve arar; farklı veritabanı boş başlar.

- [ ] **Step 2: FAIL gör, Step 3: uygula, Step 4: PASS.** `bun run typecheck`.

- [ ] **Step 5: Commit**

```bash
git add src/features/search/search-model.ts src/features/search/search-model.test.ts src/app/strings-search.ts
git commit -m "feat(search): arama modeli (gecikme, nesil, tür süzgeci, hafıza)"
```

---

### Task 6: Diyalog, bağlam menüsü, App bağlantısı, paket telafisi, belgeler

**Files:**
- Create: `src/features/search/SearchDialog.tsx`
- Modify: `src/features/explorer/actions.ts` (+ `actions.test.ts`): `searchObjects` eylemi
- Modify: `src/app/strings.ts`: `t.menu.searchObjects: "Nesne ara…"`
- Modify: `src/app/App.tsx`: açma kancası ve tembel diyalog
- Modify: `src/ipc/commands.ts` ve telafi için seçilen tembel modüller
- Modify: `scripts/bundle-budget.json` (yalnız **düşürme**)
- Modify: `docs/smoke-test.md`, `docs/testing.md`, spec durum satırı ve §7 uygulama notları

**Interfaces:**
- Consumes: `createSearch`, `searchObjects`, `ExplorerController.reveal`, `ts`.
- `SearchDialog` props:

```ts
export type SearchDialogProps = {
  /** Açık diyaloğun hedefi; null = kapalı. */
  target: { serverId: string; databaseOid: number; databaseName: string; serverName: string } | null;
  connect: (serverId: string) => Promise<boolean>;
  /** Sonuca git: App ağaçta açar; `false` → diyalog parçası `notify(ts.notFound, "error")` (ana pakete metin girmez). */
  onReveal: (hit: SearchHit, serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
  onClose: () => void;
};
```

- [ ] **Step 1: Eylem testleri** (`actions.test.ts`): `searchObjects` veritabanı düğümünde ve altındaki her düğümde (şema, koleksiyon, tablo, kolon) görünür; sunucu, grup ve roller/tablespace koleksiyonu altında görünmez; grup 3; kilitli sunucuda da açıktır; tıklama `deps.openSearch(id)`'yi çağırır. Görünürlük `ancestorOfKind(state, id, ["database"]) !== null` ile hesaplanır (ana pakette zaten var).

- [ ] **Step 2: Eylemi ve metni ekle**, testler PASS.

- [ ] **Step 3: Diyalog** (`SearchDialog.tsx`, `ui/Dialog`, genişlik `640px`):
  - Arama kutusu (`role="combobox"`, `aria-controls` liste, `aria-activedescendant` etkin satır, `aria-expanded`); `onOpenAutoFocus`'ta odak.
  - Tür süzgeci: "Tümü" + `ALL_KINDS` onay kutuları (`fieldset` + `legend` `ts.filterLabel`, sarılan satır).
  - Durum satırı: `ts.hint` / `ts.searching` / `ts.count(n)` + `ts.truncated` / `ts.none` / hata (`role="status"`, `aria-live="polite"`).
  - Liste: `role="listbox"`, satırlar `role="option"`, `aria-selected`; satırda tür adı (`ts.kinds`), ad, bağlam (`şema.sahip`, sistemse soluk). Etkin satır görünür kılınır (`scrollIntoView({ block: "nearest" })`).
  - Klavye: arama kutusunda ↓/↑ `move`, Enter `activeHit` varsa ona git, yoksa `searchNow`; Esc diyaloğu kapatır (Kobalte).
  - Çift tık / Enter: `onClose()` sonra `onReveal(hit, serverId)`.
  - Tembel modül kuralı: `app/strings`, `explorer/*`, `tree-model` içe aktarılmaz.

- [ ] **Step 4: App bağlantısı** (`App.tsx`; Bakım diyaloğu emsali, `lazy` + `everOpened`):

```ts
  const [searchTarget, setSearchTarget] = createSignal<SearchDialogProps["target"]>(null);
  const [searchEverOpened, setSearchEverOpened] = createSignal(false);
  createEffect(() => searchTarget() !== null && setSearchEverOpened(true));
  const openSearch = async (id: string) => {
    const db = ancestorOfKind(explorer.state(), id, ["database"]);
    if (!db) return;
    try {
      const target = await commands.qtTargetOf(id);
      if (target.databaseOid === null) return;
      const server = query.servers().find((s) => s.id === target.serverId);
      setSearchTarget({ serverId: target.serverId, databaseOid: target.databaseOid, databaseName: db.label, serverName: server?.name ?? "" });
    } catch (e) {
      toast.show(toPayload(e).message, "error");
    }
  };
  const revealHit = async (hit: SearchHit, serverId: string): Promise<boolean> => {
    const ok = await explorer.controller.reveal(serverId, hit.path, hit.id);
    if (ok) treeEl()?.focus();
    return ok;
  };
```

Diyalog `onReveal` `false` döndürünce `props.notify(ts.notFound, "error")` çağırır (diyalog kapanmış olsa da geçerlidir: bildirim App'in toast'ıdır). Diyaloğa `notify: toast.show`, `connect: connectServer`, `onReveal: revealHit` verilir.

`actionDeps.openSearch = (id) => void openSearch(id)`. Diyalog `<Show when={searchEverOpened()}><Suspense><SearchDialog …/></Suspense></Show>`; yükleyici `export const loadSearchDialog = () => import("../features/search/SearchDialog")`; yükleme hatası `toast.show(ts.chunkLoadFailed)` yerine `lazy` emsali (Bakım) neyse o.

- [ ] **Step 5: Paketi ölç ve telafi et**

Run: `bun run build` ve `bun run bundle:check`. Aşım varsa (beklenen ~0,7–1 KB): `commands.ts`'teki **yalnız tembel parçaların** kullandığı komutları kendi tembel modüllerine taşı (emsal: `backup-commands.ts`, `dash-commands.ts`, `maintenance-commands.ts`). Aday (her birinin tüketicilerini `grep` ile doğrula; ana pakette bir tüketici varsa taşıma):
  - Designer'a özgü: `ddlDefaults`, `ddlLoad`, `ddlLoadAcl`, `ddlLoadDefaultAcl`, `ddlLookup` (bakım/yedeklemede de kullanılıyorsa onlarla paylaşılan küçük bir tembel modül).
  - Query Tool sekmesine özgü (QueryTab tembel parçası): `qtExportCsv`, `qtFormat`, `qtCheck`, `qtComplete`, `settingsSetEditor`, `settingsSetHistory` — `query-controller.ts` (ana paket) bunları çağırıyorsa taşınamaz.
  Hiçbir aday yetmezse **dur ve sor** (tavan yükseltilmez). Geçince tavanları ölçülen değerlere **düşür**.

- [ ] **Step 6: Tüm doğrulama** — `bun run typecheck`, `npx vitest run`, `bun run build`, `bun run bundle:check`, `cargo clippy --all-targets`, `cargo test -p pgt-catalog --lib`.

- [ ] **Step 7: Belgeler**
  - `docs/smoke-test.md` "## Nesne arama (Plan 32)": menü görünürlüğü; arama gecikmesi ve Enter; tür süzgeci; `%`/`_` içeren arama; 500+ sonuçta uyarı; klavye (↓ ↑ Enter Esc); sonuca gitme (kapalı grup + bağlı olmayan sunucu → parola sorar → düğüm seçili ve görünür, Inspector gösterir); kolon/indeks/bölüm (iç içe) sonucu; aramadan sonra eklenen nesne (ağaç önceden yüklüyken) bulunur; silinmiş nesnede bildirim; sistem nesneleri ayarı; ekran okuyucu duyuruları; açık/koyu tema. Hepsi `[—]`.
  - `docs/testing.md`: Plan 32 bölümü (yeni testler, Docker fikstürü).
  - Spec durum satırı ve "§7 Plan 32 uygulama notları" (kararlar, telafi kaydı, ölçümler).

- [ ] **Step 8: Commit**

```bash
git add -A src scripts docs src-tauri
git commit -m "feat(search): nesne arama diyaloğu, bağlam menüsü ve ağaçta gitme"
```
