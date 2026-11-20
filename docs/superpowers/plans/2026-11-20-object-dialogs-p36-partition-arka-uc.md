# Nesne Diyalogları — Bölüm (Partition) Arka Ucu (Plan 36) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** `pgt-ddl`'e bölümlü tablonun **bölüm üyeliği ve sınırı** için yeni `partition` türünün arka ucunu eklemek: bölüm **oluşturma** (`CREATE TABLE … PARTITION OF`), var olan tabloyu **bağlama** (`ATTACH PARTITION`), **sınır değiştirme** (aynı transaction'da `DETACH` + `ATTACH`) ve **ayırma** (`DETACH PARTITION [CONCURRENTLY | FINALIZE]`); sınır metni ayrıştırıcısı, yükleyici, Explorer hedefi, `attachable_tables` açılır listesi, yönetici entegrasyonu. **Ön yüz bu planda değil** (Plan 37); bu plan ön yüze yalnızca **tembel derleme uyumu** için dokunur (Task 1).

**Mimari:** Plan 10–18/34 kalıbı aynen: istemci SQL göndermez, `ObjectDef` gönderir; arka uç `build::{create, alter, drop_with}` ile planı üretir; `DdlManager` aynı kapılarla uygular (yeniden yükleme eşitliği, plan özeti, onay, kilit). Bölüm tablosunun kendi özellikleri (ad, sahip, tablespace, yorum, kolonlar) **Table** türünde kalır (Plan 12/13 `isPartition` kuralı değişmez); `partition` türü yalnız **üst tabloya bağlılığı ve sınırı** modeller. Nesne kimliği bölüm tablosunun oid'idir (`ObjectRef { kind: partition, oid: C }`). Ayırma, `PlanRequest::Drop`'un `partition` türündeki anlamıdır (tabloyu silmez).

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, insta, mevcut Docker matrisi (`scripts/test-matrix.ps1`).

**Spec:** `docs/superpowers/specs/2026-11-19-object-dialogs-remaining-design.md` (§1 kararlar, §3 Grup B, §5–§7). Temel spec'ler: `docs/superpowers/specs/2026-10-06-object-dialogs-design.md`, `…/2026-10-16-object-dialogs-routines-types-design.md` (R5 kuralı). Önceki plan: `2026-11-19-object-dialogs-p34-policy-tablespace-arka-uc.md` — burada tekrarlanmayan **tüm Global Constraints ve Review Focus** maddeleri bağlayıcıdır.

## Kararlar

1. **`ObjectKind::Partition`** (`"partition"`), veritabanı kapsamlı. `ObjectRef.oid` = bölüm tablosunun (çocuk) `pg_class` oid'i.
2. **`PartitionDef`** (serde camelCase):
   - `parent_schema: String`, `parent_table: String` — bağlamdan; alter'da değişim alan hatası (`parentTable`).
   - `strategy: PartitionStrategy` (mevcut `range|list|hash`), `key_count: u32` — üst tablodan; **salt okunur** (yükleyici/`defaults` doldurur; üretici yalnız doğrulamada okur; istemcinin değiştirmesi alan hatası değil, yeniden yüklemede bayat denetimi yakalar).
   - `mode: PartitionMode` (`create` | `attach`) — yalnız create'te anlamlı; yükleyici sabit `create` yazar, alter yok sayar.
   - `schema: String`, `name: String` — create: yeni tablo; attach: var olan tablo. Alter'da değişim alan hatası (`schema`/`name`; yeniden adlandırma Table diyaloğundadır).
   - `bound: PartitionBound`.
   - `partition_by: Option<PartitionByDef>` (yalnız create+create modu; alt bölümleme), `tablespace: Option<String>`, `owner: Option<String>`, `comment: Option<String>` — yalnız `mode = create` oluşturmada yazılır; `attach`'te **yok sayılır**; yükleyici dördünü de `None` yazar (bu özellikler Table diyaloğundadır); alter'da `None` dışı değer ya da değişim alan hatası (ilgili yol, ileti "Bölüm tablosunun özellikleri Tablo diyaloğunda düzenlenir.").
   - `detach_pending: bool` (`#[serde(default)]`; PG 14+ `pg_inherits.inhdetachpending`) — salt okunur.
   - `editable: bool`, `unsupported: Vec<String>` (serde varsayılanı **yok**, R5).
3. **`PartitionBound`** (etiket `kind`, snake_case): `range { from: Vec<BoundValue>, to: Vec<BoundValue> }`, `list { values: Vec<String> }`, `hash { modulus: u32, remainder: u32 }`, `default`. **`BoundValue`** (etiket `kind`): `value { sql: String }` | `minvalue` | `maxvalue`. `list.values` öğeleri serbest SQL (sabit ifade; `NULL` serbest).
4. **Doğrulama (alan hataları):** strateji–sınır uyuşmazlığı (`bound`, ör. "Üst tablo RANGE ile bölümlenmiş; sınır FROM … TO … olmalı."); `range`: `from`/`to` uzunluğu = `key_count` (`bound.from`/`bound.to`), bir tarafta `MINVALUE`/`MAXVALUE`'dan sonra aynı taraftaki sonraki öğeler aynı anahtar sözcük olmalı (PG kuralı; `bound.from[i]`/`bound.to[i]`); `list`: en az bir değer (`bound.values`), öğeler boş değil; `hash`: `modulus ≥ 1` (`bound.modulus`), `remainder < modulus` (`bound.remainder`); `default` PG < 11 sürüm kapısı (`bound`), `hash` PG < 11 (`bound`); her serbest SQL değer `check_free_sql(path, …)` (`bound.from[i]`, `bound.to[i]`, `bound.values[i]`) + nihai ifadede `composed_is_clean`; `schema`/`name` `check_schema_ident`/`check_ident`; `parent_*` `check_old_schema_ident`/`check_old_ident`; `partition_by` (yalnız create modu): strateji sürüm kapısı (HASH PG 11+), anahtar en az bir, kolon adı `check_ident`, ifade `check_free_sql` (kolon varlığını PG doğrular — üst tablonun kolonları üreticide bilinmez); `tablespace`/`owner` `check_ident`, `comment` `check_text`.
5. **İfadeler:**
   - Sınır metni: `FOR VALUES FROM (<v>, …) TO (<v>, …)` (değer: `wrap()` uygulanmış serbest SQL ya da `MINVALUE`/`MAXVALUE`), `FOR VALUES IN (<v>, …)`, `FOR VALUES WITH (MODULUS m, REMAINDER r)`, `DEFAULT`.
   - create (`mode = create`): `CREATE TABLE <q s.n> PARTITION OF <q ps.pt> <sınır>[ PARTITION BY <strateji> (<anahtarlar>)][ TABLESPACE <q>]` (Safe; `PARTITION BY` metni `table.rs`'in mevcut yardımcısıyla — gerekirse `pub(crate)` yapılır), ardından `ALTER TABLE <q s.n> OWNER TO <q>` (sahip varsa; Safe), `COMMENT ON TABLE <q s.n> IS <literal>` (yorum varsa; Safe). Sıra: CREATE, COMMENT, OWNER TO (OWNER en sonda; ana spec kuralı).
   - attach (`mode = attach`): `ALTER TABLE <q ps.pt> ATTACH PARTITION <q s.n> <sınır>` — **Destructive**, not sürüme göre: PG ≥ 12 "Tablo taranır ve sınır doğrulanır; üst tabloda SHARE UPDATE EXCLUSIVE kilidi alınır.", PG < 12 "Tablo taranır ve sınır doğrulanır; üst tabloda ACCESS EXCLUSIVE kilidi alınır.".
   - sınır değiştirme (alter, `bound` farklı): aynı transaction'da `ALTER TABLE <q ps.pt> DETACH PARTITION <q s.n>` + `ALTER TABLE <q ps.pt> ATTACH PARTITION <q s.n> <yeni sınır>` — ikisi de **Destructive**, not "Bölüm aynı transaction içinde yeniden bağlanır; tablo yeniden taranır.". Sınır aynıysa **boş plan**. `detach_pending = true` iken alter `object` alan hatası ("Yarım kalmış ayırma var; önce ayırmayı tamamlayın (FINALIZE).").
   - ayırma (drop): `ALTER TABLE <q ps.pt> DETACH PARTITION <q s.n>` (transaction içi, **Destructive**, not "Satırlar üst tablodan görünmez olur; tablo silinmez."); `concurrently = true` → `… DETACH PARTITION <q s.n> CONCURRENTLY` **`stmt_nt`** (PG < 14 alan hatası `concurrently`); `detach_pending = true` → yalnız `… DETACH PARTITION <q s.n> FINALIZE` **`stmt_nt`** (Destructive; `concurrently` yok sayılır). `cascade = true` → alan hatası `cascade` ("Bölüm ayırmada CASCADE yoktur."). `force` mevcut ortak kuralla reddedilir.
6. **`PlanRequest::Drop`**'a `#[serde(default)] #[ts(as = "Option<bool>", optional)] concurrently: bool` eklenir (TS'de isteğe bağlı → mevcut ön yüz çağrıları değişmez). `DropOptions { cascade, force, concurrently }`; `concurrently = true` yalnız `partition` türünde geçerli, diğer türlerde alan hatası `concurrently` ("CONCURRENTLY yalnızca bölüm ayırmada kullanılır."). Index'in mevcut `ctx.concurrently` yolu (Create) değişmez.
7. **Sınır ayrıştırıcısı** (saf, `build/partition.rs`): `pub(crate) fn parse_bound(text: &str) -> Option<PartitionBound>` — girdi `pg_get_expr(relpartbound, oid)` metni: `FOR VALUES FROM (…) TO (…)`, `FOR VALUES IN (…)`, `FOR VALUES WITH (modulus <n>, remainder <n>)`, `DEFAULT`. Parantez içi **üst düzey virgüllerden** bölünür (tek/çift tırnak, `E'…'`, dolar tırnağı, iç içe parantez ve köşeli parantez farkında; yeni yardımcı `sqlscan::split_top_level_commas(&str) -> Option<Vec<String>>` — kapanmamış bölge → `None`); öğeler kırpılır; büyük/küçük harf duyarsız tam `MINVALUE`/`MAXVALUE` → anahtar sözcük. Ayrıştırılamayan metin → `None` (yükleyici `unsupported = ["sınır ifadesi ayrıştırılamadı"]`, salt okunur). Sabit nokta: üretilen sınır metni `parse_bound`'dan geçince aynı `PartitionBound` (birim testi).
8. **Yükleyici** (`load_partition.sql`, PG 14+ sürümü `load_partition.pg14.sql` `inhdetachpending` ile): `pg_class c` (`c.relispartition`) + `pg_inherits i` (`i.inhrelid = c.oid`) + üst `pg_class p` (`relkind = 'p'`) + `pg_partitioned_table pt` (`partstrat` `r|l|h` → strateji, `partnatts` → `key_count`) + şema adları + `pg_get_expr(c.relpartbound, c.oid)`. Bölüm değil / üst bölümlü değil → `NotFound("bölüm")`. Bilinmeyen `partstrat` ya da ayrıştırılamayan sınır → `unsupported` + `editable = false`; çocuk ya da üst tablo eklentiye aitse `unsupported = ["eklentiye ait"]`; geçersiz UTF-8 → `INVALID_UTF8_REASON` (mevcut kalıp). `mode = create`, `partition_by`/`tablespace`/`owner`/`comment` = `None`.
9. **Hedef (`target.rs`):** `DdlTarget`'a iki alan: `partition: Option<ObjectRef>` ve `tables_refresh_node_id: Option<String>` (ikisi de `#[serde(skip_serializing_if = "Option::is_none")] #[ts(optional)]`).
   - Bölüm düğümü `…/sch:S/tbl:P/tbl:C` (`table_at` geçerli ve önceki parça `Table`): `object = Table C` (değişmez), `partition = Some({partition, C})`; `refresh_node_id` değişmez (üst tablonun `partitions` koleksiyonu); `tables_refresh_node_id = …/sch:S/tables` (yoldaki en yakın şemanın `tables` koleksiyonu).
   - `…/tbl:P/partitions` koleksiyonu: `create = DdlCreate { kind: partition, context: { schemaOid: en yakın şema, tableOid: P } }`, `refresh_node_id` = koleksiyonun kendisi, `tables_refresh_node_id = …/sch:S/tables`.
   - **Bilinen sınır (ledger kararı):** ayrılan/bağlanan tablonun şeması üst tablonunkinden farklıysa o şemanın `tables` koleksiyonu yenilenmez (arayüz kimlik ayrıştırmaz; kullanıcı elle yeniler) — smoke notu.
10. **`LookupRequest::AttachableTables { parent_oid: u32 }`** (`#[ts(type = "number")]`): aynı veritabanında `relkind IN ('r','p')`, `NOT relispartition`, kalıtımı olmayan (`pg_inherits` ne çocuk ne üst), sistem şeması ve eklentiye ait olmayan, üst tablonun kendisi ve **üst tablonun ataları** dışındaki tablolar; value = tablo adı, detail = şema adı (ayrı alanlar; Plan 18 `TriggerFunctions` kalıbı); ad sırası `COLLATE "C"`; en çok 5000. Veritabanı kapsamlı.
11. **`defaults(Partition)`:** `table_oid` zorunlu (yoksa kodsuz `invalid`); tablo bölümlü değilse `invalid("Bölüm yalnızca bölümlenmiş bir tablo altında oluşturulabilir.")`; yoksa `not_found`. Doldurur: `parent_schema`, `parent_table`, `strategy`, `key_count`, `mode = create`, `schema = parent_schema`, `name = ""`, `bound` = strateji varsayılanı (`range` → `from` ve `to` her biri `key_count` adet `value { sql: "" }` öğesi; `list` → `values: []`; `hash` → `modulus 1, remainder 0`), diğerleri `None`, `detach_pending = false`, `editable = true`, `unsupported = []`.
12. **Yönetici:** `find_created(Partition)` = çocuk tablonun oid'i (`schema` + `name`, `relispartition` şartı; tür `Partition`); `identity_changed(Partition)` = her zaman `false` (oid değişmez); nesne denetimi (yeniden yükleme eşitliği) partition için; salt okunur kapısı (R5); ayırmadan sonra `created = None`. `hint_for`: `42P17` + `ATTACH PARTITION` → "Tablonun satırları ya da kolonları/kısıtları bölüm sınırıyla uyumsuz (ya da başka bir bölümle çakışıyor)." (PG ipucu yoksa); `55000`/`0A000` + `DETACH PARTITION … CONCURRENTLY` (DEFAULT bölüm varken ya da transaction içinde) → "CONCURRENTLY, üst tabloda DEFAULT bölüm varken kullanılamaz; normal ayırmayı seçin." (kodlar Docker'da doğrulanır, gerçek kod kullanılır); `42809`/`42P16` + `ATTACH PARTITION` (yanlış tablo türü/kalıtım) → PG iletisi (özel ipucu yok). Kural ipuçları yalnız PG ipucu yoksa (mevcut kural).
13. **Şema karşılaştırma etkisi:** `pgt-diff` yeni türü karşılaştırmaz (bölüm üyeliği bugün de Table diff'inde yoktur); yalnız derleme uyumu (`match` kolları).
14. **Kapsam dışı (backlog):** bölüm bölme/birleştirme, ayrılan tablonun farklı şemadaki koleksiyon yenilemesi, `ATTACH` öncesi kısıt önerisi (taramayı kısaltan CHECK), `CREATE TABLE … PARTITION OF` kolon/kısıt tanımları (kolon varsayılanları vb. bölüm tablosunda Table diyaloğundan).

## Global Constraints

- Plan 10/12/14/18/34 Global Constraints aynen: PG **10–18** vanilla; tüm tanımlayıcı/değer yerleştirmeleri yalnızca `quote_ident`/`quote_literal`/`qualified`; ad alanlarında `check_ident`, yüklü taraf gevşek (`check_old_*`); alter yalnızca **değişen ya da ifadeye yazılan** alanları doğrular; `Statement.sql` sonda `;` taşımaz ve tek ifadedir.
- **Serbest SQL alanları:** sınır değerleri → `check_free_sql` + `wrap()` + nihai ifadede `composed_is_clean`; sıradan `'…'` içinde `\` reddedilir (`E'…'`). Yüklenen (`pg_get_expr`) değer düz `\` içerirse yalnız o sınırı yazan düzenleme reddedilir (Plan 18 §8.7 emsali); ayırma (yalnız adlar) etkilenmez.
- **Risk sınıfları:** spec §3.3 aynen: create (+ sahip/yorum) Safe; attach, sınır değiştirme, detach (her biçim) Destructive; DataLoss yok. Destructive yalnız `confirmation_required`; üretim ad onayı yalnız DataLoss içindir (Plan 34 ledger kararı).
- **Transaction:** create, attach, sınır değiştirme ve normal detach transaction içinde; `DETACH … CONCURRENTLY` ve `DETACH … FINALIZE` `stmt_nt`.
- **Günlük/gizlilik:** sınır değerleri, ad ve SQL asla `tracing`'e, hata iletisine ya da diske yazılmaz (yalnız işlem/tür/ifade sayısı/süre/kod).
- **Docker:** yalnızca `scripts/test-matrix.ps1` (`pgt-matrix-<v>` geçici kapsayıcılar; bitince silinir); başka kapsayıcılara dokunulmaz; `PGT_TEST_URLS` yalnızca test komutunun ortamında; sır adlı dosya yazılmaz. Test nesneleri benzersiz önekli şema/ad (`pgt_ddl_rt_<etiket>`) ve `guarded` temizliğiyle. Bu planda matris betiği **değişmez**.
- **Ön yüz derleme uyumu (yalnızca Task 1):** yeni `ObjectKind`/`ObjectDef`/`LookupRequest` değerleri TS tiplerini değiştirir; uyum **yalnızca tembel designer/schema-diff dosyalarında** (Plan 34 kalıbı: `FORMS` geçici `formMissing` koruması ve metni, `KIND_LABELS` `partition: "Bölüm"`, `defName`, `FORM_PATHS` boş küme, `drop-model` tablosu `partition`: cascade yok, force yok). `ddl-kinds.ts` ve `ACL_KINDS` **değişmez** (menü Plan 37'de). Ana paket **tam 126,553 B**, başlangıç **154,226 B** kalmalı (`npm run bundle:check`); bayt artarsa durup raporla.
- `ts-rs`, `Box::pin`, git worktree yasağı; `cargo test` `src/ipc/bindings/*.ts`'i LF ile yeniden yazar → `git checkout -- src/ipc/bindings/` ile gürültü geri alınır, **yeni** ve **içeriği değişen** binding dosyaları CRLF'e çevrilerek commit edilir (depodaki diğer bindinglerle aynı satır sonu). Dosyalar CRLF'dir; düzenlemeler satır sonunu korur. Crate geneli `cargo fmt` yok. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Yorumlar Türkçe, çevredeki yoğunlukta. `graphify-out/` ve `.superpowers/` commit edilmez.

## Review Focus

- **Sınır ayrıştırma sabit noktası:** `alter(loaded, loaded)` her strateji ve sürümde **boş plan** — tarih/zaman damgası/metin/sayısal/çok kolonlu range, `MINVALUE`/`MAXVALUE`, `IN (…, NULL)`, virgül ve parantez içeren değerler (`'a,b'`, `'(x)'`, `'it''s'`), hash, default. Ayrıştırılamayan metin salt okunur olur, panik/yanlış parça üretmez.
- **Sınır değiştirmenin atomikliği:** DETACH + ATTACH aynı transaction'da; ATTACH başarısızsa (çakışan sınır, uyumsuz satır) bölüm **bağlı kalır** ve eski sınırı korunur (Docker testi).
- **Ayırma biçimleri:** normal (tx), CONCURRENTLY (`stmt_nt`, PG 14+; DEFAULT bölüm varken ipucu), yarım kalmış ayırmada yalnız FINALIZE; ayırma tabloyu silmez, satırlar korunur; `cascade`/`force` reddi; `concurrently` diğer türlerde reddi.
- **Create/attach kapsamı:** attach'te sahip/yorum/tablespace/partition_by yok sayılır; create'te alt bölümleme ve OWNER en sonda; attach kilit notu sürüme göre; `attachable_tables` üst tabloyu, atalarını, bölümleri, kalıtım üyelerini, sistem/eklenti tablolarını listelemez.
- **Hedefler:** `partition` yalnız gerçek bölüm düğümünde (`…/tbl:P/tbl:C`), `partitions` koleksiyonu create bağlamı doğru (`tableOid = P`); şema altındaki sıradan tablo düğümünde `partition = None`; günlükte sınır değeri/ad yok.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-ddl/src/model.rs` | `ObjectKind::Partition`, `PartitionMode`, `PartitionBound`, `BoundValue`, `PartitionDef`, `ObjectDef::Partition` |
| `crates/pgt-ddl/src/build/partition.rs` (+ snapshots) | Sınır metni üretimi, `parse_bound`, create/attach/alter/detach, doğrulama |
| `crates/pgt-ddl/src/build/sqlscan.rs` | `split_top_level_commas` |
| `crates/pgt-ddl/src/build/table.rs` | `PARTITION BY` yardımcısının paylaşımı (`pub(crate)`) |
| `crates/pgt-ddl/src/build/mod.rs` | Dispatch, `validate`, `validate_identity`, `DropOptions.concurrently` |
| `crates/pgt-ddl/src/load.rs`, `src/sql/load_partition.sql`, `load_partition.pg14.sql` | Yükleyici |
| `crates/pgt-ddl/src/lookup.rs`, `src/sql/lookup_attachable_tables.sql` | `AttachableTables` |
| `crates/pgt-ddl/src/target.rs` | `partition`, `tables_refresh_node_id`, `partitions` koleksiyonu |
| `crates/pgt-ddl/src/manager.rs`, `error_codes.rs` | `PlanRequest::Drop.concurrently`, `find_created`, `defaults`, `identity_changed`, kapılar, ipuçları |
| `crates/pgt-ddl/tests/partition_roundtrip.rs` (yeni), `manager.rs`, `json.rs`, `logging.rs` | Docker gidiş-dönüş ve kapılar |
| `crates/pgt-diff/src/*`, `src-tauri/src/*` | Yalnız derleme uyumu (`match` kolları) |
| `src/features/designer/*`, `src/features/schema-diff/*` (yalnız tembel, Task 1) | Derleme uyumu |
| `docs/smoke-test.md`, `docs/testing.md`, spec | Notlar, durum |

---

### Task 1: Model, hedefler, istek alanı ve tembel derleme uyumu

**Dosyalar:**
- Değiştir: `crates/pgt-ddl/src/model.rs`, `build/mod.rs` (geçici `Invalid("henüz desteklenmiyor")` kolları; `DropOptions.concurrently` ve ortak kuralı), `load.rs` (geçici kol), `lookup.rs` (yalnız enum varyantı + geçici `Invalid`), `manager.rs` (`PlanRequest::Drop.concurrently`, `plan_view` aktarımı, geçici kollar), `target.rs`; derleme hatası veren `pgt-diff`/`src-tauri` `match`'leri; `src/features/designer/{DesignerView.tsx, forms-model.ts, designer-model.ts, drop-model.ts}` ve TS hatası veren diğer **tembel** dosyalar; ilgili testler
- Test: `model.rs` serde testleri, `target.rs` birim testleri, `build/mod.rs` birim testleri, `tests/json.rs`

**Arayüzler:**
- Üretir (`model.rs`; serde camelCase, enum'lar snake_case):

```rust
pub enum ObjectKind { /* mevcutlar */, Partition }          // server_scoped() değişmez (false)
pub enum PartitionMode { Create, Attach }
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BoundValue { Value { sql: String }, Minvalue, Maxvalue }
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PartitionBound {
    Range { from: Vec<BoundValue>, to: Vec<BoundValue> },
    List { values: Vec<String> },
    Hash { modulus: u32, remainder: u32 },             // ts: number
    Default,
}
pub struct PartitionDef {
    pub parent_schema: String, pub parent_table: String,
    pub strategy: PartitionStrategy, pub key_count: u32,   // ts: number
    pub mode: PartitionMode,
    pub schema: String, pub name: String,
    pub bound: PartitionBound,
    pub partition_by: Option<PartitionByDef>,
    pub tablespace: Option<String>, pub owner: Option<String>, pub comment: Option<String>,
    #[serde(default)] pub detach_pending: bool,
    pub editable: bool, pub unsupported: Vec<String>,
}
// ObjectDef'e: Partition(PartitionDef); kind() → Partition; name() → `name`
```
- Üretir (`build/mod.rs`): `pub struct DropOptions { pub cascade: bool, pub force: bool, pub concurrently: bool }` (`Default`); `drop_with` ortak kuralı: `concurrently && kind != Partition` → `err("concurrently", "CONCURRENTLY yalnızca bölüm ayırmada kullanılır.")`.
- Üretir (`manager.rs`): `PlanRequest::Drop { object, def, cascade, #[serde(default)] force, #[serde(default)] #[ts(as = "Option<bool>", optional)] concurrently }`; `plan_view` `concurrently`'yi `DropOptions`'a geçirir.
- Üretir (`target.rs`): Karar 9 (`DdlTarget.partition`, `DdlTarget.tables_refresh_node_id`).
- Üretir (`lookup.rs`): `LookupRequest::AttachableTables { #[ts(type = "number")] parent_oid: u32 }` (`server_wide` = false); gövde Task 3'te.
- Ön yüz (yalnız tembel): `FORMS` `partition` için `formMissing` koruması, `KIND_LABELS.partition = "Bölüm"`, `defName` (Partition: `name`), `FORM_PATHS` boş küme, `drop-model` (`partition`: cascade yok, force yok). `ddl-kinds.ts` **değişmez**.

- [ ] **Adım 1: Başarısız testler.** Serde: `PartitionDef` camelCase şekli (`parentSchema`, `parentTable`, `keyCount`, `partitionBy`, `detachPending`), `PartitionBound` dört varyantın JSON'u (`{"kind":"range","from":[{"kind":"value","sql":"1"},{"kind":"minvalue"}],"to":[…]}`, `{"kind":"default"}`), `detachPending` eksikken `false`, `editable`/`unsupported` eksikken **hata**; `ObjectKind` `"partition"`; `tests/json.rs` tür listesine `partition`; `PlanRequest::Drop` JSON'unda `concurrently` yokken `false`, varken `true`; `drop_with` `concurrently` diğer türde `["concurrently"]` hatası; `target.rs`: `db:5/sch:2200/tbl:24576/tbl:24580` → `object = Table 24580`, `partition = Partition 24580`, `refresh_node_id = …/tbl:24576/partitions`, `tables_refresh_node_id = …/sch:2200/tables`; `db:5/sch:2200/tbl:24576/partitions` → `create(Partition, schemaOid 2200, tableOid 24576)`, `tables_refresh_node_id = …/sch:2200/tables`; `db:5/sch:2200/tbl:24576` (şema altındaki tablo) → `partition = None`, `tables_refresh_node_id = None`; alt bölüm `…/tbl:24576/tbl:24580/tbl:24590` → `partition = Partition 24590`, refresh `…/tbl:24580/partitions`; bozuk konum (`db:5/tbl:1/partitions`, `db:5/sch:2200/partitions`) boş. JSON çıktısında yeni `DdlTarget` alanları `None` iken **yazılmaz**.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula (üretici/yükleyici/yönetici/lookup kollarında geçici `Invalid`; derleme her yerde geçer).
- [ ] **Adım 4:** `cargo test -p pgt-ddl -p pgt-diff` (Docker'sız), `cargo clippy --workspace --all-targets -- -D warnings`; bindingler üretilir; `npm run typecheck`, `npm run test`, `npm run build`, `npm run bundle:check` (ana **126,553 B**, başlangıç **154,226 B**); `git checkout -- src/ipc/bindings/` gürültüsü; yeni/değişen binding dosyaları (CRLF) commit.
- [ ] **Adım 5: Commit** (`feat(ddl): Partition modeli, ayırma isteği alanı ve bölüm hedef eşlemeleri`).

---

### Task 2: Partition üreticisi ve sınır ayrıştırıcısı

**Dosyalar:** Oluştur `crates/pgt-ddl/src/build/partition.rs` (+ snapshots); değiştir `build/mod.rs`, `build/sqlscan.rs`, `build/table.rs` (yalnız görünürlük).

**Arayüzler:**

```rust
pub fn validate(ctx: &BuildCtx, d: &PartitionDef) -> Vec<FieldError>;
pub fn create(ctx: &BuildCtx, d: &PartitionDef) -> Result<Plan, Vec<FieldError>>;       // mode'a göre create|attach
pub fn alter(ctx: &BuildCtx, old: &PartitionDef, new: &PartitionDef) -> Result<Plan, Vec<FieldError>>;
pub fn detach(ctx: &BuildCtx, d: &PartitionDef, concurrently: bool) -> Result<Plan, Vec<FieldError>>;
pub(crate) fn bound_sql(b: &PartitionBound) -> String;           // "FOR VALUES …" | "DEFAULT"
pub(crate) fn parse_bound(text: &str) -> Option<PartitionBound>; // Karar 7
// sqlscan.rs:
pub(crate) fn split_top_level_commas(s: &str) -> Option<Vec<String>>;
```
- Karar 4–5 ve 7; `build::drop_with` Partition kolu `partition::detach(ctx, d, o.concurrently)` (cascade reddi Karar 5); `validate_identity(Partition)`: `parent_schema` `check_old_schema_ident`, `parent_table`/`name` `check_old_ident`, `schema` `check_old_schema_ident`; `invalid_utf8_locked`/`droppable_despite_invalid_utf8` Partition'ı kapsar (ayırma yalnız adları yazar → tek gerekçe buysa izinli).
- [ ] **Adım 1: Başarısız testler (insta + birim):**
  - create: range tek anahtar (`FROM ('2024-01-01') TO ('2025-01-01')`), çok anahtarlı range `MINVALUE`/`MAXVALUE` ile, list (`IN (1, 2, NULL)`), hash, default; alt bölümleme (`PARTITION BY LIST (region)`); tablespace + sahip + yorum → sıra CREATE, COMMENT, OWNER TO; PG 10'da default/hash sürüm hatası; strateji uyuşmazlığı; `key_count` uyuşmazlığı; MINVALUE sonrası değer reddi (`bound.from[1]`); boş list; `remainder ≥ modulus`; `modulus = 0`; boş değer (`bound.values[0]`).
  - attach: Destructive + PG 11 ve PG 16 notu farkı; sahip/yorum/tablespace/partition_by dolu olsa da ifadeye girmez.
  - alter: aynı sınır → boş plan; sınır değişimi → DETACH + ATTACH iki Destructive transaction içi ifade + not; `schema`/`name`/`parentTable` değişimi alan hatası; `owner`/`comment` dolu → alan hatası; `detach_pending` → `object` hatası; salt okunur eski tanım → `object`.
  - detach: normal (tx, Destructive, not), CONCURRENTLY (`non-tx`, PG 13'te `concurrently` hatası), `detach_pending` → FINALIZE `non-tx`, `cascade` hatası, geçersiz UTF-8 tek gerekçeyle ayrılabilir.
  - hostile: adlarda `"`, `;`, satır sonu; değerlerde `'); DROP TABLE x; --`, kapanmamış `'`, düz `\` (sıradan literal) reddi, `E'\\x'` kabulü; `assert_single_statements` her planda.
  - `parse_bound` tablosu: `FOR VALUES FROM ('2024-01-01') TO ('2025-01-01')`, `FOR VALUES FROM (MINVALUE, MINVALUE) TO (10, MAXVALUE)`, `FOR VALUES IN ('a,b', 'it''s', NULL)`, `FOR VALUES IN ('(x)')`, `FOR VALUES FROM ('2024-01-01 00:00:00+00') TO ('2024-02-01 00:00:00+00')`, `FOR VALUES WITH (modulus 4, remainder 3)`, `DEFAULT`, büyük/küçük harf, fazladan boşluk; reddedilenler (`None`): boş, `FOR VALUES FROM (1`, kapanmamış tırnak, bilinmeyen biçim, `modulus` sayısal değil; ve her `bound_sql(b)` için `parse_bound(bound_sql(b)) == Some(b)` (yalnız `wrap` etkisi olmayan basit değerlerle; `wrap` satır sonu ekliyorsa sabit nokta testi yükleyici metniyle Task 3'tedir — davranışı birim testinde belgeleyin).
  - `split_top_level_commas`: iç içe parantez, köşeli parantez (`ARRAY[1,2]`), tek/çift tırnak, `E'\''`, dolar tırnağı, boş öğe.
- [ ] **Adım 2–5:** uygula, `cargo test -p pgt-ddl`, clippy, commit (`feat(ddl): Partition üreticisi — oluştur/bağla/sınır değiştir/ayır ve sınır ayrıştırıcısı`).

---

### Task 3: Yükleyici, `attachable_tables` ve Docker gidiş-dönüşü

**Dosyalar:** Değiştir `load.rs`, `lookup.rs`; oluştur `src/sql/load_partition.sql`, `load_partition.pg14.sql`, `lookup_attachable_tables.sql`; oluştur `crates/pgt-ddl/tests/partition_roundtrip.rs`; gerekirse `tests/common/mod.rs`.

- Yükleme: Karar 8 (`VersionedSql` kalıbı; PG 14+ `inhdetachpending`).
- Lookup: Karar 10 (`VersionedSql`; tek sürüm).
- [ ] **Adım 1: Başarısız testler (Docker, her sunucuda):**
  - create → load → eşit (create'in yük-ilgisiz alanları — sahip/yorum/tablespace/partition_by/mode — karşılaştırmadan önce yükleyicinin yazdığı değerlere çekilir) ve `alter(loaded, loaded)` boş: range `date` tek anahtar, `timestamptz` (oturum saat dilimi sabit), `text` değer virgüllü/tırnaklı, çok kolonlu range MINVALUE/MAXVALUE, list `NULL` dahil, hash (11+), default (11+), alt bölümlü bölüm (11+ değil: PG 10 da destekler — tüm sürümler).
  - attach: sıradan tablo (uyumlu satırla) bağlanır → yüklenir; uyumsuz satır → `42P17` benzeri hata, tablo bağlanmaz.
  - sınır değiştirme: `alter(old, new)` → load → `new.bound`; çakışan sınır → hata ve **eski sınır korunur** (aynı transaction).
  - ayırma: normal → tablo durur, `relispartition = false`, satırlar korunur; CONCURRENTLY (14+; DEFAULT bölüm yokken) başarılı; DEFAULT bölüm varken CONCURRENTLY → hata kodu kaydedilir (Task 4 ipucu için; testte kod sabitlenir); `detach_pending` yüklemesi: PG 14+'da yarım kalmış ayırma üretmek zorsa **birim düzeyinde** (yükleyici satır eşlemesi) test edilir, Docker'da yalnız `false` doğrulanır.
  - `attachable_tables`: üst tablo, atası (alt bölümlemede), bölümler, INHERITS üyeleri, `pg_catalog` tabloları listede yok; sıradan ve bölümlü (kendi bölümü olmayan) tablo var; detail şema adı.
  - bölüm olmayan oid → `NotFound`; eklentiye ait tablo yolu varsa salt okunur (yoksa yalnız birim).
- [ ] **Adım 2–4:** uygula; `cargo test --workspace` (Docker'sız), clippy, matris (`powershell -NoProfile -File scripts/test-matrix.ps1 -Versions 10,11,12,13,14,15,16,17,18 -SkipPerf`; kapsayıcılar silinir).
- [ ] **Adım 5: Commit** (`feat(ddl): Partition yükleyicisi, bağlanabilir tablolar listesi ve PG 10–18 gidiş-dönüş testleri`).

---

### Task 4: Yönetici entegrasyonu ve kapılar

**Dosyalar:** Değiştir `manager.rs`, `error_codes.rs` (+ `tests/manager.rs`, `tests/logging.rs`).

- Karar 11–12: `defaults(Partition)`, `find_created`, `identity_changed`, nesne denetimi, salt okunur kapısı (R5), `hint_for` (Task 3'te sabitlenen gerçek SQLSTATE'lerle).
- [ ] **Adım 1: Başarısız testler (birim + Docker):**
  - birim: `plan_view` Drop `concurrently` aktarımı; `read_only_refusal` partition; `identity_changed` false; `hint_for` tablo (attach 42P17, detach concurrently kodu, PG ipucu varken kural ipucu yok).
  - Docker: `load_and_apply_targets_agree_for_every_kind` partition ile; `defaults` (range iki anahtarlı üst tabloda `from`/`to` iki boş değer; bölümsüz tablo → `invalid`; tablo yok → `not_found`); create ve attach → `created` çocuk oid'i (tür `partition`); sınır değiştirme ve ayırma → `created = None`; kilitli sunucuda `locked`; Destructive (attach, sınır değiştirme, detach) `confirmation_required`, üretimde ek ad onayı **istenmez**; bayat `old` (başka oturumda sınır değişmiş) → `plan_changed`; CONCURRENTLY `stmt_nt` ile uygulanır ve kısmi uygulama kuralı bozulmaz; günlükte sınır değeri/ad yok (`logging.rs` kalıbı: ayırt edici dizge sınır değerinde ve tablo adında).
- [ ] **Adım 2–4:** uygula; `cargo test --workspace`, clippy, matris. **Adım 5: Commit** (`feat(ddl): Partition yönetici entegrasyonu — varsayılanlar, find_created, kapılar, ipuçları`).

---

### Task 5: Belgeler ve bütünleşik doğrulama

**Dosyalar:** Değiştir `docs/smoke-test.md`, `docs/superpowers/specs/2026-11-19-object-dialogs-remaining-design.md`, `docs/testing.md` (yalnız gerekirse).

- [ ] **Adım 1:** Tam Docker matrisi `powershell -NoProfile -File scripts/test-matrix.ps1 -Versions 10,11,12,13,14,15,16,17,18 -SkipPerf` — yeşil.
- [ ] **Adım 2: `docs/smoke-test.md`** — "Nesne diyalogları: Bölüm arka ucu (Plan 36)" bölümü (GUI maddeleri Plan 37'de `[—]`); bilinen sınırlar (farklı şemaya bağlanan/ayrılan tablonun şema koleksiyonu yenilenmez; yarım kalmış ayırma yalnız FINALIZE; bölüm tablosunun sahip/yorum/tablespace'i Tablo diyaloğunda).
- [ ] **Adım 3: Spec** — durum "Plan 36 arka ucu uygulandı; Plan 37 ön yüzü kalan"; **§11 Plan 36 sözleşme notları (Plan 37 için)**: model alanları (camelCase) ve salt okunurlar (`strategy`, `keyCount`, `detachPending`), `editable`/`unsupported` zorunluluğu, alan yolları (`bound`, `bound.from[i]`, `bound.to[i]`, `bound.values`, `bound.values[i]`, `bound.modulus`, `bound.remainder`, `schema`, `name`, `parentTable`, `partitionBy.*`, `tablespace`, `owner`, `comment`, `concurrently`, `cascade`, `object`, `old.*`), notlar ve uyarı metinleri, `PlanRequest::Drop.concurrently`, hedef alanları (`partition`, `tablesRefreshNodeId`) ve yenileme kuralı, `AttachableTables` biçimi, `defaults` davranışı, ipucu kodları (Task 3'te sabitlenen), ön yüz bütçe notu (ana 126,553 B; Plan 37 menü kancası telafi ister).
- [ ] **Adım 4: Bütünleşik doğrulama.** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `npm run test`, `npm run typecheck`, `npm run build`, `npm run bundle:check` (ana **126,553 B**, başlangıç **154,226 B**).
- [ ] **Adım 5: Kapanış greps:** `rg "tracing::" crates/pgt-ddl/src/build/partition.rs` boş.
- [ ] **Adım 6: Commit** (`docs(ddl): Plan 36 sözleşme notları, smoke ve bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 36 bitişi)

- Oluştur/bağla/sınır değiştir/ayır (normal, CONCURRENTLY, FINALIZE) ve yükleme PG 10–18'de yeşil; sınır ayrıştırma sabit noktası her stratejide.
- Sınır değiştirme atomik; ayırma tabloyu ve satırları korur; günlükte sınır değeri/ad yok.
- Ana paket **126,553 B**, başlangıç **154,226 B** (değişmedi).
- Elle kontrol maddeleri Plan 37'dedir; `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §3.1 model (→ T1; doğrulama → T2), §3.2 eylemler (create/attach/sınır/ayır → T2; CONCURRENTLY/FINALIZE → T2/T3; yükleme ve ayrıştırma → T2/T3; hedef → T1; `attachable_tables` → T3; yenileme → T1 Karar 9), §3.3 risk (T2), §1 serbest SQL ve transaction dışı (Global Constraints, T2), §6 günlük (T4), §7 test (T2–T4).
- **Bilinçli kararlar:** ayırma `PlanRequest::Drop`'un partition anlamıdır (yeni istek türü yerine; nesne denetimi ve onay kapıları hazır); `concurrently` TS'de isteğe bağlı (ana paket etkilenmez); bölüm tablosu özellikleri yüklemede `None` ve alter'da yasak (Table diyaloğunun alanı); farklı şemaya bağlanan/ayrılan tablonun şema koleksiyonu yenilenmez (kimlik ayrıştırma yasağı).
- **Yer tutucu taraması:** Task 1'deki geçici `Invalid` kolları Task 2–4'te kalkar (açıkça belirtilmiş ara durum). İpucu SQLSTATE'leri Task 3'te Docker'da sabitlenir (PG'nin gerçek kodu; tahmin yazılmaz).
- **Tip tutarlılığı:** `PartitionDef`/`PartitionBound`/`BoundValue` (T1) → `partition.rs` (T2) → yükleyici (T3) → `defaults` (T4); `DropOptions.concurrently` (T1) → `detach` (T2) → `plan_view` (T1) → yönetici testleri (T4); `DdlTarget.partition`/`tables_refresh_node_id` (T1) → spec notları (T5); `LookupRequest::AttachableTables` (T1) → gövde (T3).
