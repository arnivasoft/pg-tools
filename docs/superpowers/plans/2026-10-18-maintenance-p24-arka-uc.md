# Bakım İşlemleri — Arka Uç (Plan 24) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** `pgt-ddl`'e bakım işlemlerinin (VACUUM / ANALYZE / REINDEX / CLUSTER) arka ucunu eklemek. Kapsam: model, saf SQL üreticisi, hedef yükleyicisi, `PlanRequest::Maintenance` ile mevcut kapılardan uygulama, NOTICE/INFO iletilerinin toplanması, uygulama pid'i ve ilerleme sorgusu, Tauri komutları ve PG 10–18 Docker matrisi. **Ön yüz bu planda değil** (Plan 25).

**Mimari:** Diğer `PlanRequest` varyantlarıyla aynı akış: istemci yüklenen hedefi ve seçenekleri gönderir; arka uç tek, transaction dışı bir ifade üretir. `DdlManager::apply` kapıları uygulanır: kilit → hedef → plan → yeniden yükleme eşitliği → hash → onay → lock_timeout → çalıştır. Yenilikler:
1. `execute` uygulama bağlantısının NOTICE kanalını ifadelerle eşzamanlı okur ve sonucu `ApplyOutcome`'un `messages` alanına ekler. Bu tüm DDL uygulamaları için geçerlidir; ileti yoksa alan yazılmaz.
2. Uygulama bağlantısının arka uç pid'i `CancelMap` yuvasına kaydedilir. `DdlManager::maintenance_progress(apply_id)` katalog havuzundan ilgili `pg_stat_progress_*` görünümünü bu pid için okur.

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, insta, Docker matrisi (`scripts/test-matrix.ps1`).

**Spec:** `docs/superpowers/specs/2026-10-18-maintenance-design.md` (§1 kararlar — bağlayıcı, §2 model, §4 güvenlik, §5 test). Temel: `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (risk modeli). Plan 10/14/18/20 Global Constraints bağlayıcıdır.

## Global Constraints

- **PostgreSQL:** sürüm 10–18. Tanımlayıcılar yalnız `quote_ident`/`qualified` ile yazılır. `Statement.sql` sonda `;` taşımaz ve tek ifadedir. Bakım ifadeleri `transactional = false` olur.
- **Sürüm kapıları:** kapı dışı istek doğrulama hatası verir (spec §1 "Seçenekler"); sessiz atlama yoktur.
  - PG 12+: `SKIP_LOCKED`, `INDEX_CLEANUP`, `REINDEX CONCURRENTLY`; `pg_stat_progress_cluster` ve `create_index`.
  - PG 13+: `PARALLEL`; `pg_stat_progress_analyze`.
  - PG 14+: `INDEX_CLEANUP AUTO`.
  - PG 15+: bölümlenmiş tabloda `CLUSTER`.
- **Risk sınıfları:** spec §1 "Risk" aynen. Destructive ifadelerde `Statement.note` kilit notunu taşır: VACUUM FULL/CLUSTER "Tablo boyunca ACCESS EXCLUSIVE kilit alır; okuma/yazma bekler."; REINDEX (CONCURRENTLY'siz) "Tabloda SHARE, indekste ACCESS EXCLUSIVE kilit alır; yazma ve indeksi kullanan okumalar bekler." (SCHEMA/DATABASE: + " Tablolar sırayla kilitlenir.").
- **İletiler:**
  - Sınır: en çok 500 ileti; her `message`/`detail`/`hint` alanı en çok 4096 bayt. Kesilen alan UTF-8 sınırında kesilir ve sonuna `…` eklenir. Sınır aşılırsa son ileti "N ileti daha gösterilmedi" olur.
  - Toplama bağlantı düşene kadar sürer: ifade bittikten sonra kuyruktaki iletiler de alınır (son NOTICE'lar).
  - Gizlilik: iletiler `tracing`'e, hata iletisine ya da diske yazılmaz.
- **Günlük/gizlilik:** SQL, nesne adları ve ileti metni hiçbir yere yazılmaz; yalnız `op`, `kind`, süre ve sonuç kodu yazılır. İlerleme sorgusu yalnız kendi `apply_id`'mizin pid'i için çalışır; bilinmeyen ya da bitmiş `apply_id` → `Ok(None)`.
- **Docker:** yalnızca `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf`. Kapsayıcılar geçicidir (`pgt-matrix-<v>`) ve silinir; başka kapsayıcılara dokunulmaz. URL/parola yazdırılmaz. Test nesneleri `pgt_ddl_rt_<etiket>` şemalarında, `guarded` temizlikle kurulur.
- **Ön yüz derleme uyumu:**
  - Yeni TS bindingleri commit edilir. `PlanRequest` ve `ApplyOutcome` TS birleşimi genişler; gerekirse uyum yalnız tembel dosyalarda yapılır.
  - `src/ipc/commands.ts`'e ekleme yapılmaz (Plan 25).
  - `bun run bundle:check` geçer.
  - Satır sonu gürültüsü `git checkout -- src/ipc/bindings` ile atılır.
- **Genel:** Yorumlar Türkçe. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Crate genelinde `cargo fmt` çalıştırılmaz.

## Review Focus

- **Transaction dışı ve tek ifade:** her bakım planı tek ifadedir ve `transactional = false`'tur. `run_statements` bunu BEGIN'siz çalıştırır. REINDEX CONCURRENTLY / SCHEMA / DATABASE ve VACUUM transaction içinde hiç denenmez.
- **İptal ve kilit:**
  - Uzun işlem `ddl_cancel` ile 57014 alır ve `Failed` döner, `messages` korunur.
  - `lock_timeout` aşımı 55P03 verir ve ipucu gösterilir (`hint_for`: "Nesne başka bir oturumca kilitli; daha sonra deneyin ya da lock timeout'u artırın.").
  - İptal sonrası pid yuvası temizlenir ve ilerleme `None` döner.
- **İleti toplama yarışları:**
  - İleti, ifade sonucu ile bağlantı kapanışı arasında gelirse kaybolmaz.
  - Çok sayıda VERBOSE iletisinde bellek sınırı uygulanır.
  - İleti kanalının okunması ifadeyi bekletmez ya da kilitlemez.
- **Hedef doğruluğu:**
  - REINDEX DATABASE yalnız bağlı veritabanının adıyla yazılır.
  - İndeks hedefinde `REINDEX INDEX` kullanılır.
  - Bölümlenmiş tabloda sürüm kuralları uygulanır.
  - `CLUSTER USING` yalnız tablonun kendi indeksiyle yapılır. `USING` yoksa ve kümeleme indeksi de yoksa doğrulama hatası verilir.
  - Sistem şemasında `CLUSTER` ve `REINDEX CONCURRENTLY` reddedilir.
- **Bayat hedef:** nesne yeniden adlandırılmış ya da silinmişse `PLAN_CHANGED` ya da `NotFound` döner. Spoof edilmiş `target` (başka şema/ad) yeniden yükleme eşitliğinde yakalanır.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-ddl/src/maintenance.rs` (yeni) | Model: `MaintenanceOp`, seçenek yapıları, `MaintenanceTarget`, `MaintenanceTargetKind`, `IndexCleanup`, `MaintenanceProgress` |
| `crates/pgt-ddl/src/build/maintenance.rs` (+ `snapshots/`) | Saf üretici `maintenance(ctx, target, op) -> Result<Plan, Vec<FieldError>>` |
| `crates/pgt-ddl/src/load_maintenance.rs` (yeni), `src/sql/load_maintenance_*.sql` | Hedef yükleyicisi |
| `crates/pgt-ddl/src/apply.rs` | İleti toplama, pid kaydı (CancelMap yuvası) |
| `crates/pgt-ddl/src/manager.rs` | `PlanRequest::Maintenance`, kapılar, `messages`, `maintenance_progress`, ipuçları |
| `crates/pgt-ddl/src/sql/maintenance_progress*.sql` | İlerleme sorguları (sürüme göre) |
| `src-tauri/src/ddl.rs`, `src-tauri/src/main.rs` | `ddl_load_maintenance_target`, `ddl_maintenance_progress` |
| `crates/pgt-ddl/tests/maintenance_roundtrip.rs`, `tests/manager.rs`, `tests/logging.rs` | Docker testleri |
| `docs/…` | spec durumu, smoke (arka uç notu), testing.md |

---

### Task 1: Model ve saf üretici

**Dosyalar:** Oluştur `crates/pgt-ddl/src/maintenance.rs`, `crates/pgt-ddl/src/build/maintenance.rs` (+ snapshots); değiştir `lib.rs`, `build/mod.rs`.

**Arayüzler — Üretir:**

```rust
#[serde(rename_all = "snake_case")] pub enum MaintenanceTargetKind { Table, MaterializedView, Index, Schema, Database }
#[serde(rename_all = "snake_case")] pub enum IndexCleanup { Auto, On, Off }
pub struct VacuumOptions { pub full: bool, pub freeze: bool, pub analyze: bool, pub verbose: bool, pub disable_page_skipping: bool, pub skip_locked: bool, pub index_cleanup: Option<IndexCleanup>, pub parallel: Option<u16> }
pub struct AnalyzeOptions { pub verbose: bool, pub skip_locked: bool }
pub struct ReindexOptions { pub concurrently: bool, pub verbose: bool }
pub struct ClusterOptions { pub verbose: bool, pub using_index: Option<String> }
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MaintenanceOp { Vacuum(VacuumOptions), Analyze(AnalyzeOptions), Reindex(ReindexOptions), Cluster(ClusterOptions) }
pub struct MaintenanceTarget {
    pub kind: MaintenanceTargetKind, pub schema: Option<String>, pub name: String,
    pub table: Option<String> /* indeks: ait tablo adı (aynı şema) */, pub partitioned: bool, pub system: bool,
    pub cluster_index: Option<String>, pub indexes: Vec<String>, pub editable: bool, pub unsupported: Vec<String>,
}
pub fn ops_for(kind: MaintenanceTargetKind, partitioned: bool, version_num: u32) -> &'static [&'static str]; // "vacuum" | "analyze" | "reindex" | "cluster" (Plan 25 için)
// build/maintenance.rs
pub fn maintenance(ctx: &BuildCtx, target: &MaintenanceTarget, op: &MaintenanceOp) -> Result<Plan, Vec<FieldError>>;
```
(Tüm yapılar `Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS`, `camelCase`, `#[ts(export)]`.)

- Kurallar spec §1 (hedef/işlem matrisi, seçenekler, sözdizimi, risk, sistem şeması, sürüm kapıları):
  - `PARALLEL` 0–1024 aralığında olmalı ve `FULL` ile birlikte kullanılamaz.
  - `using_index` ∈ `target.indexes` olmalı.
  - `editable = false` → `object` hatası.
- Alan yolları: `op`, `options.<alan>` (ör. `options.parallel`, `options.usingIndex`), `object`.
- İfade biçimleri:
  - Tablo/MV hedefi `qualified(schema, name)`, indeks `qualified(schema, name)`, şema `quote_ident(name)`, veritabanı `quote_ident(name)`.
  - VACUUM/ANALYZE veritabanı hedefinde ad yazılmaz (`VACUUM (ANALYZE)` → bağlı veritabanının tümü).
  - REINDEX: `REINDEX [(VERBOSE)] {INDEX|TABLE|SCHEMA|DATABASE} [CONCURRENTLY] <ad>`.
  - CLUSTER: `CLUSTER [VERBOSE] <tablo> [USING <q>]`.

- [ ] **Adım 1: Başarısız testler (insta + birim):**
  - Her işlem × hedef birleşimi; seçenek kombinasyonları; sürüm kapıları (her kapı için kapının bir altında hata, kapıda serbest).
  - Risk ve not; `transactional == false`; tek ifade; `ops_for` tablosu.
  - Hatalar: `PARALLEL` + `FULL`, `PARALLEL` 1025, bilinmeyen `using_index`, kümeleme indeksi yokken `USING`'siz CLUSTER, sistem şemasında CLUSTER/CONCURRENTLY, view gibi desteklenmeyen birleşim → `op`.
  - Hostile adlar: `"`, Unicode, boşluk.
- [ ] **Adım 2–3:** uygula. **Adım 4:** `cargo test -p pgt-ddl`, clippy, bindingler. **Adım 5: Commit** (`feat(ddl): bakım işlemleri modeli ve SQL üreticisi`).

---

### Task 2: Hedef yükleyicisi

**Dosyalar:** Oluştur `crates/pgt-ddl/src/load_maintenance.rs`, `src/sql/load_maintenance_relation.sql`, `load_maintenance_index.sql`, `load_maintenance_schema.sql`, `load_maintenance_database.sql`; değiştir `lib.rs`; testler `crates/pgt-ddl/tests/maintenance_roundtrip.rs` (yükleme kısmı).

**Arayüzler — Üretir:**

```rust
pub async fn load_maintenance_target(mgr: &ConnectionManager, server: Uuid, database_oid: u32, obj: ObjectRef) -> Result<MaintenanceTarget>;
```
- `ObjectRef.kind` → hedef türü eşlemesi:
  - `Table` → relkind `r`/`p` (`partitioned = relkind = 'p'`)
  - `MaterializedView` → `m`
  - `Index` → `i`/`I` (bölümlenmiş indeks `I` → `unsupported: ["bölümlenmiş indeks"]`; PG'de doğrudan REINDEX kısıtlı, ilk sürümde salt okunur)
  - `Schema`
  - `Database` (bakım için **veritabanı kapsamlı**: `database_oid` = düğümün veritabanı; yükleyici `current_database()` ile eşleşmeyi doğrular, eşleşmezse `Invalid`)
  - Diğer türler → `Invalid("Bu nesne için bakım işlemi yoktur.")`.
- `indexes` alanı tablonun indeks adları (ad sırası). `cluster_index` alanı `indisclustered` olan indeks.
- `system` alanı `pg_catalog`, `information_schema` ve `pg_toast%` şemaları için true olur. Geçersiz UTF-8 ad → kayıpsız okuma deseni + `editable = false` (Plan 18/20 kalıbı).
- [ ] **Adım 1: Başarısız testler (Docker):**
  - Her tür için yükleme alanları doğru.
  - Bölümlenmiş tablo.
  - Kümelenmiş indeks.
  - Sistem tablosu (`pg_catalog.pg_class`) `system = true`.
  - Silinmiş oid → `NotFound`.
  - Yanlış veritabanı → `Invalid`.
- [ ] **Adım 2–4:** uygula; `cargo test --workspace`, clippy, matris. **Adım 5: Commit** (`feat(ddl): bakım hedefi yükleyicisi`).

---

### Task 3: İleti toplama ve uygulama pid'i (tüm DDL uygulamaları)

**Dosyalar:** Değiştir `crates/pgt-ddl/src/apply.rs`, `manager.rs` (`ApplyOutcome` alanı, `execute`); testler.

**Arayüzler — Üretir:**

```rust
pub struct ServerMessage { pub severity: String, pub code: Option<String>, pub message: String, pub detail: Option<String>, pub hint: Option<String> }  // camelCase, TS
// ApplyOutcome::Applied / Failed'e:
#[serde(skip_serializing_if = "Vec::is_empty", default)] #[ts(optional)] messages: Vec<ServerMessage>,
// CancelMap yuvasına: backend_pid: Option<i32>; pub(crate) fn backend_pid(&self, apply_id: Uuid) -> Option<i32>
```
- `execute`, `run_statements` ile eşzamanlı olarak `conn.notices`'ı sınırlı bir tampona boşaltır (Global Constraints sınırları). Çalıştırma bitince kuyrukta kalanları `try_recv` ile alır. `pgt-query`'nin `NoticeBuf`/`absorb` kalıbını inceleyin; mümkünse ortak bir yardımcıya taşımayın (crate sınırı), ama kesme kuralları aynı olmalı.
- Bağlantı açıldıktan hemen sonra `SELECT pg_backend_pid()` ile pid okunur ve yuvaya yazılır. Yuva kaydı sırası korunur: önce kayıt, sonra bağlantı, sonra pid. Yuva düşünce pid de düşer.
- Mevcut uygulamalarda (create/alter/drop/privileges) iletiler de döner; örneğin `DROP … CASCADE` NOTICE'ları. Ön yüz Plan 25'te gösterir; bu planda yalnız bindinge girer.
- [ ] **Adım 1: Başarısız testler:**
  - Birim (Docker'sız): kesme ve sınır yardımcısı.
  - Docker:
    - `DO $$ BEGIN RAISE NOTICE 'x'; END $$` içeren test planı (`apply_statements` yardımcısıyla) → `messages[0].message == "x"`.
    - 600 NOTICE → 500 ileti + "daha gösterilmedi".
    - Uzun ileti kesilir.
    - Başarısız ifadede de `messages` gelir.
    - `backend_pid` uygulama sırasında `Some`, sonra `None`.
- [ ] **Adım 2–4:** uygula; `cargo test --workspace`, clippy, matris; bindingler (`ApplyOutcome.ts`, `ServerMessage.ts`); `bun run typecheck` (mevcut `ApplyOutcome` kullanımları derlenir). **Adım 5: Commit** (`feat(ddl): uygulama iletileri ve arka uç pid'i`).

---

### Task 4: Yönetici entegrasyonu, ilerleme ve Tauri komutları

**Dosyalar:** Değiştir `manager.rs` (+ `tests/manager.rs`, `tests/logging.rs`), oluştur `src/sql/maintenance_progress_vacuum.sql`, `…_cluster.sql`, `…_create_index.sql`, `…_analyze.sql`; değiştir `src-tauri/src/ddl.rs`, `src-tauri/src/main.rs`; `tests/maintenance_roundtrip.rs` (uygulama kısmı).

**Arayüzler — Üretir:**

```rust
// PlanRequest
Maintenance { object: ObjectRef, target: MaintenanceTarget, op: MaintenanceOp },   // op() = "maintenance", kind() = object.kind
impl DdlManager {
    pub async fn load_maintenance_target(&self, server: Uuid, database_oid: u32, obj: ObjectRef) -> Result<MaintenanceTarget>;
    pub async fn maintenance_progress(&self, server: Uuid, database_oid: u32, apply_id: Uuid) -> Result<Option<MaintenanceProgress>>;
}
pub struct MaintenanceProgress { pub phase: String, pub done: Option<u64>, pub total: Option<u64>, pub unit: Option<String> }  // camelCase, TS
// src-tauri
#[tauri::command] async fn ddl_load_maintenance_target(server_id: Uuid, database_oid: u32, object: ObjectRef) -> Result<MaintenanceTarget, ErrorPayload>;
#[tauri::command] async fn ddl_maintenance_progress(server_id: Uuid, database_oid: u32, apply_id: Uuid) -> Result<Option<MaintenanceProgress>, ErrorPayload>;
```
- **Kapılar:**
  - `plan_view` → `build::maintenance::maintenance`.
  - `object_check` → `Expected::Maintenance(object, &target)` ve `load_maintenance_target` yeniden yüklenir; eşitsizlik `PLAN_CHANGED` döner.
  - `read_only_refusal`: `target.editable == false`.
  - Hedef her zaman `Target::Database(database_oid)`, Database türü için de. Bu istisna `target_for`'dan önce ele alınır ve belgelenir. `database_oid = None` → `NEEDS_DATABASE`.
  - `maintenance_refusal` uygulanmaz (bakım veritabanını kapatmaz).
  - `created = None`.
  - Uygulama sonrası doğrulama yoktur.
- **İlerleme:** pid yoksa `Ok(None)` döner. Sürüme göre sorgu sırası (işleme göre, istekte saklanmaz: hepsi tek sorguda `UNION ALL`, ya da sırayla — basit olan): vacuum (9.6+), cluster (12+), create_index (12+), analyze (13+). Yalnız `pid = $1` satırları okunur. `phase` PG'nin İngilizce aşama metnidir; ön yüz çevirir ya da olduğu gibi gösterir (Plan 25). `done`/`total`: vacuum `heap_blks_scanned/heap_blks_total` (blocks), cluster `heap_blks_scanned/heap_blks_total` ya da `heap_tuples_written` (tuples), create_index `blocks_done/blocks_total` (blocks), analyze `sample_blks_scanned/sample_blks_total` (blocks). İlerleme sorgusu katalog havuzunda (`mgr.run(server, Target::Database(db), …)`) çalışır; uygulama bağlantısını kullanmaz.
- **İpuçları (`hint_for`, op = "maintenance"):**
  - 55P03 → kilit ipucu (Review Focus metni).
  - 57014 → yok (iptal iletisi yeter).
  - 42501 → "Bakım işlemi için tablonun sahibi (ya da PG 17+'da MAINTAIN yetkisi) gerekir."
  - 0A000 (REINDEX CONCURRENTLY sistem kataloğu vb.) → "Bu nesnede bu seçenek desteklenmiyor."
  - 25001 → olmamalı (transaction dışı); yine de genel ipucu.
- [ ] **Adım 1: Başarısız testler:**
  - Birim:
    - `plan_view` maintenance;
    - `read_only_refusal`;
    - `hint_for` dört yeni ipucu;
    - Database hedefinin `Target::Database` olması.
  - Docker (her sunucuda):
    - VACUUM (`last_vacuum` dolu);
    - ANALYZE (`last_analyze`);
    - REINDEX TABLE/INDEX/SCHEMA/DATABASE (relfilenode değişir);
    - REINDEX CONCURRENTLY PG 12+;
    - CLUSTER USING (`indisclustered`);
    - VERBOSE → `messages` boş değil;
    - lock_timeout: başka bağlantı `LOCK TABLE … ACCESS EXCLUSIVE` tutarken VACUUM FULL → 55P03 + ipucu;
    - iptal: kilit beklerken `cancel(apply_id)` → 57014;
    - ilerleme: büyük tabloda (≈200k satır) VACUUM FULL ya da kilit bekleyen işlem sırasında `maintenance_progress` çağrısı. Satır gelirse alanlar tutarlı olmalı (zamanlamaya bağlı; `None` da kabul edilir, ama sorgu hatasız çalışmalı ve PG 10–11'de cluster/create_index görünümleri sorgulanmamalı);
    - kilitli sunucu `locked`;
    - bayat hedef `plan_changed`;
    - Destructive `confirmation_required`;
    - logging: SQL, ad ve ileti metni yok.
- [ ] **Adım 2–4:** uygula; `cargo test --workspace`, clippy, `cargo build -p pg-tools`, matris; bindingler.
- [ ] **Adım 5: Commit** (`feat(ddl): bakım istekleri için yönetici kapıları, ilerleme ve Tauri komutları`).

---

### Task 5: Belgeler ve bütünleşik doğrulama

**Dosyalar:** Spec (durum + **Plan 24 sözleşme notları (Plan 25 için)**: model alanları, alan yolları, `ops_for`, ipuçları, ileti sınırları, ilerleme alanları/birimler ve aşama metinleri, komut imzaları); `docs/smoke-test.md` ("Bakım işlemleri arka ucu (Plan 24)": GUI maddeleri Plan 25'te); `docs/testing.md`.

- [ ] **Adım 1:** tam matris `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` yeşil; sürüm kapılı atlamalar rapora.
- [ ] **Adım 2:** belgeler. **Adım 3:** `cargo test --workspace`, clippy, `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`.
- [ ] **Adım 4: Kapanış taramaları:**
  - `rg "tracing::" crates/pgt-ddl/src/build/maintenance.rs crates/pgt-ddl/src/load_maintenance.rs` → boş.
  - `rg "messages" crates/pgt-ddl/src/manager.rs` → günlük çağrılarında ileti metni yok.
- [ ] **Adım 5: Commit** (`docs(ddl): Plan 24 sözleşme notları, smoke ve bütünleşik doğrulama`).

## Öz inceleme

- **Spec §1 kapsamı:**
  - Hedef/işlem, seçenekler, sözdizimi → T1.
  - Çalıştırma → T1/T4.
  - Risk → T1.
  - Nesne denetimi → T2/T4.
  - İletiler → T3.
  - İlerleme → T3 (pid) / T4.
  - Kapsam dışı maddelere dokunulmadı.
  - §4 güvenlik → Global Constraints + T4 logging.
  - §5 test → T1–T4.
- **Tip tutarlılığı:**
  - `MaintenanceTarget` (T1) → yükleyici (T2) → `Expected` (T4).
  - `ServerMessage`/`messages` (T3) → spec notları (T5).
  - `MaintenanceProgress` (T4).
- **Bilinçli kararlar:**
  - İleti toplama tüm DDL uygulamalarına eklenir: tek yol, küçük maliyet.
  - Bölümlenmiş indeks ilk sürümde salt okunurdur.
  - Veritabanı hedefi veritabanı kapsamlıdır (`target_for` istisnası).
