# Veri Düzenleyici — Arka Uç (Plan 16) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Veri düzenleyicinin (pgAdmin "View/Edit Data" karşılığı) **arka ucunu** eklemek: bir tablo/görünümün **kaynak tanımını** (kolonlar, satır kimliği anahtarı, yetkiler, salt okunur nedeni) yükleme, Query Tool oturumunda çalışacak **SELECT'i üretme** (anahtar sıralı, `xmin` sürüm kolonlu, doğrulanmış filtreli), istemcinin gönderdiği **değişiklik listesinden** `INSERT`/`UPDATE`/`DELETE` ifadelerini üretip **önizleme** olarak sunma ve **kapılı, tek transaction'lı, bayat-veri denetimli uygulama** (`data_load`, `data_select`, `data_preview`, `data_apply`, `data_cancel` Tauri komutları). **Ön yüz bu planda değil:** arayüz **Plan 17**'dedir (bu plan ön yüze yalnızca üretilen ts-rs bağlamalarını bırakır).

**Mimari:** Spec (`docs/superpowers/specs/2026-10-07-data-editor-design.md`) §2. `pgt-ddl`'e yeni bölüm `crates/pgt-ddl/src/data/` (apply altyapısı ortak olduğu için ayrı crate yok): saf modüller (`select.rs`, `plan.rs`, anahtar seçimi) + katalog okuma (`source.rs`) + `DataManager` (`manager.rs`). Plan 10–14 kalıbı aynen: istemci SQL göndermez (tek istisna doğrulanan filtre gövdesi), arka uç ifadeleri üretir; `DataManager.apply` DDL'deki gibi kapılardan geçirir. Yenilikler: (1) **satır sürümü** — her satırın `xmin::text` değeri okumada gelir, `UPDATE/DELETE`te `WHERE <anahtar> AND xmin::text = '<sürüm>'` ile bayat veri algılanır; (2) **tam 1 satır etkilenmeli** — değilse tüm transaction geri alınır (`row_conflict`); (3) **`RETURNING`** — `INSERT/UPDATE` sonrası dönen satır (kolonlar + yeni `xmin`) istemciye taşınır; (4) **`SessionProfile::Data`** — Ddl'in yazılabilir/sonsuz `statement_timeout`/`lock_timeout` kopyası, ayrı `application_name`.

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, insta, mevcut Docker matrisi (`scripts/test-matrix.ps1`).

**Spec:** `docs/superpowers/specs/2026-10-07-data-editor-design.md` (§1 kararlar, §2 mimari, §4 güvenlik, §5 test). Önceki planlar: `2026-10-06-object-dialogs-p10-arka-uc.md`, `2026-10-06-object-dialogs-p12-tablo-arka-uc.md`, `2026-10-07-object-dialogs-p14-view-database-role.md` — burada tekrarlanmayan **tüm Global Constraints ve Review Focus** maddeleri bağlayıcıdır (tırnaklama, NUL/denetim karakteri reddi, serbest SQL kapıları, kilit/onay kapıları, günlük gizliliği, Docker kuralları, `Box::pin`, ts-rs).

## Kararlar

1. **`xmin` sürüm denetimi (SPEC §1.1):** satırın tüm hücre değerlerini "özgün değer" olarak göndermek yerine tek `xmin::text` sürümü kullanılır. Gerekçe: `json`, `xml`, `point` gibi tiplerin `=` işleci yoktur; `kolon::text` karşılaştırması da çıktı fonksiyonuyla farklıdır (`bool::text` = `true` iken metin protokolü `t`). `xmin` her tipte çalışır, tek karşılaştırmadır ve **kapalı başarısız olur** (anahtar + sürüm uyuşmazsa 0 satır → çakışma; yanlış satır yazılmaz). Bedel: aynı değere yazan eşzamanlı bir `UPDATE` de çakışma sayılır (kabul). Dondurulmuş (frozen) satırlar ham `xmin`'i korur (PG ≥ 9.4); `VACUUM FREEZE` sürümü değiştirmez.
2. **Anahtar kolonlarına tipsiz sabit:** `WHERE "k" = 'v'` — PG sabiti kolon tipine çevirir (giriş fonksiyonu). Anahtar tipinin metin gidiş-dönüşü kayıpsız olmalıdır; kayıplı olabilen durumlar (ör. PG 10–11'de `float8` + `extra_float_digits = 0`) **kapalı başarısız olur** (0 satır → `row_conflict`), yanlış satır yazmaz. Float anahtarlar ayrıca dışlanmaz (nadir; smoke notu).
3. **Risk:** `INSERT`/`UPDATE` **Safe**, `DELETE` **Destructive**; üretim sunucusunda boş olmayan **her** plan `confirmed` ister (`CONFIRMATION_REQUIRED`; Query Tool `production_write` eşdeğeri, ad yazma yok). `confirmed_production` bu planda yoktur (veri düzenleyicide DataLoss sınıfı yok).
4. **Tek transaction, hepsi ya da hiçbiri:** DDL'in "bloklar" mantığı gerekmez (DML hep transactional). `committed` alanı **yoktur**; başarısızlıkta hiçbir şey kalıcı değildir. `COMMIT` sonucu bilinmiyorsa (SQLSTATE'siz hata) DDL'deki `unknown_outcome_payload` yolu.
5. **Kimlik/üretilmiş kolonlar:** `attgenerated <> ''` kolonlar yazılamaz (alan hatası); `attidentity = 'a'` yalnızca `INSERT`te `Default` alır, `UPDATE` hata; `'d'` serbest. `OVERRIDING` üretilmez.
6. **Salt okunur kaynak** (`editable = false`) için `data_preview`/`data_apply` doğrulama hatası verir (`read_only_source`, alan yolu `object`); `data_select` her kaynakta çalışır (görünüm/MV/yabancı tablo dahil; anahtarsızda sıralama yok).
7. **`fingerprint`:** kaynak tanımının (şema, ad, kolon sırası/ad/tip OID'si/üretilmiş/kimlik, anahtar kolonları) 16 haneli hex özeti (`DefaultHasher`; `plan_hash` gibi bütünlük amaçlı). `data_select`/`data_preview`/`data_apply` istekleri bunu taşır; sunucudaki güncel kaynakla farklıysa `plan_changed` (kolon eklendi/silindi, anahtar değişti → arayüz "Yeniden yükle" der).
8. **Sürüm sütunu adı:** `__pgt_xmin`; kaynakta aynı adlı kolon varsa `__pgt_xmin_1`, `_2`… (ilk boş ad). `DataSelect.version_index` = kolon sayısı (sürüm her zaman **son** kolon).
9. **Filtre = tek serbest SQL alanı:** ≤ 4096 bayt, NUL/denetim karakteri yok, `scan_semicolons` temiz; sorguya `WHERE (<filtre>\n)` olarak sarılır; sıradan `'…'` içinde `\` reddedilir (Plan 12 serbest SQL kuralı: `E'…'` kullanın). Sıralama `OrderBy{column, desc}` listesi; kolon adı kaynak kolonlarından olmalı (en çok 8 anahtar).
10. **Sınırlar:** bir uygulamada ≤ **2000** değişiklik, her `CellValue::Text` ≤ **16 MiB** (bayt) ve NUL yok, toplam metin ≤ **64 MiB**; aşım alan hatası. Boş değişiklik listesi boş plan (`empty`).
11. **Kapsam dışı (backlog):** anahtarsız düzenleme (`ctid`), kullanıcı seçimli anahtar, updatable view düzenleme, `OVERRIDING SYSTEM VALUE`, büyük nesne düzenleyici, `COPY`, kolon bazlı `fingerprint` yumuşatması, `UPDATE` için "değişmeyen hücreleri hiç göndermeme" dışı optimizasyonlar.

## Global Constraints

- Plan 10/12/14'ün Global Constraints'i aynen geçerlidir: PostgreSQL **10–18** vanilla; tüm tanımlayıcılar `quote_ident`/`qualified`, tüm değerler `quote_literal` (NUL/denetim karakteri doğrulaması **çağıran**dadır); kilit `read_only_lock` → `locked`; günlük gizliliği.
- **Günlük/gizlilik (en yüksek öncelik):** hücre değerleri, anahtar değerleri, `xmin`, SQL metni, kolon/şema/tablo adı, filtre ve URL asla `tracing`'e, `Debug` çıktısına (özel `Debug`: `CellValue::Text` değeri `…` yazar, uzunluk yok), `ErrorPayload`'a (bizim ürettiğimiz iletiler; PG'nin kendi iletisi yalnızca kullanıcıya döner, günlüğe girmez), diske ya da geçmişe yazılmaz. Günlük yalnızca işlem (`load|select|preview|apply`), değişiklik/ifade sayısı, süre ve sonuç kodunu içerir. Test `crates/pgt-ddl/tests/logging.rs` kalıbıyla yakalar.
- **Yazma kapısı sırası (`apply`):** (1) kayıtsız sunucu, (2) `read_only_lock` → `locked`, (3) kaynak yükleme (`fingerprint` eşitliği → `plan_changed`; `editable=false` → alan hatası), (4) plan doğrulama/üretim (alan hataları → `invalid_def`; boş plan → `Invalid("Değişiklik yok")`), (5) `expected_hash` → `plan_changed`, (6) onay: plan riski ≥ Destructive ya da üretim sunucusu → `confirmed` yoksa `confirmation_required`, (7) `lock_timeout` sınırı (`MAX_STATEMENT_TIMEOUT_MS`), (8) yürütme. `apply_id` zaten kullanımdaysa `Invalid(APPLY_ID_IN_USE)`.
- **İfade biçimleri (insta ile sabitlenir):**
  - `INSERT INTO <q> (<kolonlar>) VALUES (<değerler>) RETURNING <tüm kolonlar>, xmin::text` — hiç kolon yoksa `INSERT INTO <q> DEFAULT VALUES RETURNING …`. `CellValue::Default` → `DEFAULT`, `Null` → `NULL`, `Text(s)` → `quote_literal(s)`.
  - `UPDATE <q> SET <c1> = <v1>, … WHERE <k1> = '<a>' AND … AND xmin::text = '<sürüm>' RETURNING <tüm kolonlar>, xmin::text` (SET kolonları kaynak sırasıyla).
  - `DELETE FROM <q> WHERE <k1> = '<a>' AND … AND xmin::text = '<sürüm>'`.
  - Sıra: istemcinin değişiklik sırası aynen korunur (kullanıcı sırasını öngörür); plan ifadesi başına **tek** satır etkiler.
  - `RETURNING` kolon listesi = `DataSource.columns` sırası + `xmin::text`.
- **Hata eşleme:** yeni kod `row_conflict` (`DdlErrorCode`'a eklenir; `DDL_ERROR_CODES` 7 öğe); `Failed` yükü `conflict: bool` taşır (conflict ise `sqlstate = None`). PG 42501 → mevcut `failure_payload` ipucu; 55P03 → mevcut kilit zaman aşımı iletisi; 23xxx kısıt ihlalleri PG iletisiyle (değer içerebilir; yalnızca kullanıcıya).
- **Sürüm bağımlılığı:** `attgenerated` PG 12+, `attidentity` PG 10+; `.pg10` varyantları `VersionedSql` ile (`load_table_columns.pg10.sql` kalıbı). PG 18 `virtual` üretilmiş kolon (`attgenerated = 'v'`) yazılamaz. PG 10–11'de `ORDER BY name COLLATE "C"` yazımı `x::text COLLATE "C"` (bu planda sıralama kullanıcı kolonlarıdır, katalog sıralamaları SQL dosyalarında bu kurala uyar).
- **Docker:** yalnızca `scripts/test-matrix.ps1` (`pgt-matrix-<v>` geçici kapsayıcılar); başka kapsayıcılara dokunulmaz. Testlerdeki tablolar/roller benzersiz önekli ad alır (`pgt_data_<etiket>`) ve `guarded` temizliğiyle silinir. Test sunucu URL'leri/parolaları yazdırılmaz; rol parolaları rastgele üretilir.
- `ts-rs` (`#[ts(export)]`, bindingler commit edilir), `Box::pin` her Tauri komutunda, git worktree **kullanılmaz**, `src/ipc/bindings` satır sonu gürültüsü `git checkout -- src/ipc/bindings`. Commit eki: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Yorumlar Türkçe, çevredeki yoğunlukta. Ham NUL baytı kabuk üzerinden yazılmaz (`'\0'` kaçışı).
- **Ön yüz:** Rust tarafı değişiklikleri TS bindinglerini ekler/değiştirir; **ana paket** (`index-*.js`) **tam 232,805 B** kalmalı (yeni bindingler yalnızca tip; `ddl-kinds.ts`/`actions.ts` bu planda değişmez; `DdlErrorCode`'a `row_conflict` eklenmesi ana paketteki tip eşlemelerini etkiliyorsa yalnızca tip düzeyinde kalmalı ve `bun run build` ile doğrulanmalı).

## Review Focus

- **Gizlilik:** hücre/anahtar/sürüm değerleri hiçbir günlükte, `Debug`'ta, `ErrorPayload`'ta (bizimkiler), `PlanView`'in dışındaki hiçbir yerde yok; `PlanView.statements[].sql` değer içerir (kullanıcıya gösterilmek üzere) ama `tracing`'e girmez.
- **Enjeksiyon/tırnaklama:** tüm kolon/şema/tablo adları ve değerler hostile girdilerle (`'`, `"`, `\`, `--`, `/*`, `;`, NUL, denetim, Unicode, anahtar sözcük, 63/64 bayt kolon adı, `E'…'` gerektiren ters eğik çizgi) tek ifade kalır; filtre kapısı `;`/kapanmamış tırnak/yorum/parantezle atlatılamaz (`1=1); DROP TABLE x; --`, `x) OR (1=1`, `$$`); `__pgt_xmin` takma adı çakışması.
- **Bayatlık ve atomiklik:** başka oturumun güncellediği/sildiği satır → `row_conflict` ve **hiçbir ifade kalıcı değil** (önceki ifadeler geri alınır); `BEFORE` tetikleyicisi `RETURN NULL` → conflict; kısıt ihlali ortada → tam geri alma; `fingerprint` uyuşmazlığı (kolon eklendi/anahtar düştü) → `plan_changed`; kilit zaman aşımı 55P03; iptal 57014; `COMMIT` belirsiz sonuç.
- **Kapılar:** kilitli sunucuda `locked` (okuma `data_select` serbest); üretimde Safe plan bile `confirmation_required`; DELETE `confirmation_required`; salt okunur kaynak (`no_key`, `no_privilege`, `system_object`, `unsupported_kind`) yazma reddi; yetkisiz kolon/`GENERATED ALWAYS` kimlik `UPDATE`/üretilmiş kolon alan hatası; sınır aşımları.
- **Gidiş-dönüş PG 10–18:** `insert → returned == data_select satırı` (varsayılanlar, serial, tetikleyici `NEW` değişikliği), `update → returned`, `NULL` ≠ `''`, `bytea`/`json`/`jsonb`/dizi/`timestamptz`/`numeric`/`boolean`/`uuid`/`float8`/`citext`/enum/`xml` değerleri metin gidiş-dönüşü; bölümlü tabloda satır bölümler arası taşıma (PG 11+), kalıtım çocuğu; üretilmiş/kimlik kolonları (sürüm kapılı).
- **Anahtar seçimi:** PK; PK yok → en küçük geçerli NOT NULL benzersiz indeks; kısmi/ifadeli/geçersiz/`NULL` içeren benzersiz indeks seçilmez; `INCLUDE` anahtara girmez; çok kolonlu anahtar; anahtar kolonu düzenlenmesi (`UPDATE SET pk = …`) yeni anahtar `RETURNING` ile döner.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-core/src/connection.rs` | `SessionProfile::Data { lock_timeout_ms }` (Ddl'in kopyası; `application_name = "pg-tools data"`) |
| `crates/pgt-ddl/src/data/mod.rs`, `model.rs` | Bölüm belge başlığı; model tipleri (`DataSource`, `DataColumn`, `DataKey`, `ReadOnlyReason`, `RowChange`, `CellValue`, `DataChangeSet`, `DataApplyOptions`, `DataOutcome`, `RowResult`, `SelectRequest`, `OrderBy`, `DataSelect`) |
| `crates/pgt-ddl/src/data/source.rs`, `src/sql/data_source_columns{,.pg10}.sql`, `data_source_keys.sql`, `data_source_info.sql` | `load_source`, anahtar seçimi (saf `choose_key`), `fingerprint`, yetkiler |
| `crates/pgt-ddl/src/data/select.rs` | `select_sql`, filtre/sıralama doğrulaması |
| `crates/pgt-ddl/src/data/plan.rs`, `snapshots/` | `plan_changes` (doğrulama + ifade üretimi + risk + uyarılar) |
| `crates/pgt-ddl/src/data/manager.rs`, `exec.rs` | `DataManager` (`load/select/preview/apply/cancel/cancel_all`), RETURNING'li tek transaction yürütücü |
| `crates/pgt-ddl/src/error_codes.rs`, `lib.rs` | `row_conflict` kodu; `pub mod data` ve yeniden dışa aktarımlar |
| `src-tauri/src/data.rs`, `state.rs`, `main.rs`, `src/ipc/commands.ts` (yalnızca tip/sarmalayıcı) | `data_*` komutları, `AppState.data`, yenileme/çıkışta `cancel_all`, TS sarmalayıcılar |
| `crates/pgt-ddl/tests/data_roundtrip.rs`, `data_manager.rs`, `logging.rs` (ek) | Docker gidiş-dönüş, kapılar, çakışma/atomiklik, günlük sızıntısı |
| `docs/smoke-test.md`, spec | Arka uç notları, durum |

---

### Task 1: Model, hata kodu ve `SessionProfile::Data`

**Dosyalar:**
- Değiştir: `crates/pgt-core/src/connection.rs` (profil + testler), `crates/pgt-ddl/src/error_codes.rs`, `crates/pgt-ddl/src/lib.rs`
- Oluştur: `crates/pgt-ddl/src/data/mod.rs`, `crates/pgt-ddl/src/data/model.rs`

**Arayüzler:**
- Üretir (`connection.rs`): `SessionProfile::Data { lock_timeout_ms: u32 }` — `Ddl` ile **aynı** başlangıç seçenekleri/doğrulama (`validate_profile`: yazılabilir, `statement_timeout = 0`, `lock_timeout_ms ≤ MAX_STATEMENT_TIMEOUT_MS`); `application_name()` = `"pg-tools data"` (yeni `DATA_APPLICATION_NAME`); `Ddl`'in `match` kollarının her biri için `Data` kolu eklenir (derleme zorlar); `validate_profile`/oturum doğrulaması `Ddl` ile aynı beklenti.
- Üretir (`error_codes.rs`): `pub const ROW_CONFLICT_MESSAGE: &str = "Satır başka bir oturum tarafından değiştirilmiş ya da silinmiş; değişiklikler uygulanmadı.";`, `DdlErrorCode::RowConflict` (serde `row_conflict`), `DDL_ERROR_CODES: [&str; 7]` (sıra: mevcut 6 + `row_conflict`), `from_marker`/`as_str` kolları, `ddl_payload` eşlemesi.
- Üretir (`data/model.rs`; hepsi `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]`, `#[serde(rename_all = "camelCase")]`, `#[ts(export)]` — aksi belirtilmedikçe):

```rust
pub enum RelKind { Table, Partitioned, View, MatView, Foreign }        // serde snake_case (matview → "mat_view" yerine "matview": rename = "matview")
pub enum ReadOnlyReason { NoKey, NoPrivilege, SystemObject, UnsupportedKind }   // serde snake_case
pub enum Identity { None, Always, ByDefault }                          // serde snake_case
pub struct DataColumn {
    #[ts(type = "number")] pub attnum: i16,
    pub name: String,
    #[ts(type = "number")] pub type_oid: u32,
    pub type_display: String,
    pub not_null: bool,
    pub has_default: bool,
    pub identity: Identity,
    pub generated: bool,
    pub insertable: bool,   // kolon INSERT yetkisi ∧ !generated
    pub updatable: bool,    // kolon UPDATE yetkisi ∧ !generated ∧ identity != Always
}
pub enum KeySource { PrimaryKey, UniqueIndex }                         // serde snake_case
pub struct DataKey { pub source: KeySource, pub name: String, pub columns: Vec<String> }
pub struct DataSource {
    pub object: ObjectRef,
    pub schema: String,
    pub name: String,
    pub relkind: RelKind,
    pub columns: Vec<DataColumn>,
    pub key: Option<DataKey>,
    pub can_select: bool, pub can_insert: bool, pub can_update: bool, pub can_delete: bool,
    pub locked: bool,                              // sunucu read_only_lock
    pub editable: bool,                            // aşağıdaki kural
    pub read_only_reason: Option<ReadOnlyReason>,  // editable=false ise Some
    pub fingerprint: String,                       // 16 hane hex
}
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CellValue { Null, Default, Text { value: String } }           // özel Debug: Text değeri yazmaz
pub struct CellEdit { pub column: String, pub value: CellValue }
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum RowChange {
    Insert { values: Vec<CellEdit> },
    Update { key: Vec<String>, version: String, set: Vec<CellEdit> },   // key: DataKey.columns sırasıyla
    Delete { key: Vec<String>, version: String },
}
pub struct DataChangeSet { pub object: ObjectRef, pub fingerprint: String, pub changes: Vec<RowChange> }
pub struct OrderBy { pub column: String, pub desc: bool }
pub struct SelectRequest { pub order: Vec<OrderBy>, pub filter: Option<String> }
pub struct DataSelect { pub sql: String, #[ts(type = "number")] pub version_index: u32, pub has_version: bool }   // has_version: yalnız Table/Partitioned (xmin sistem kolonu)
pub struct DataApplyOptions {
    pub apply_id: Uuid,
    #[ts(type = "number")] pub lock_timeout_ms: u32,
    pub expected_hash: String,
    pub confirmed: bool,
}
pub struct RowResult { #[ts(type = "number")] pub index: u32, pub returned: Option<Vec<Option<String>>> }   // columns + xmin; Delete → None
#[serde(tag = "outcome", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum DataOutcome {
    Applied { statements: u32, #[ts(type = "number")] elapsed_ms: u64, results: Vec<RowResult> },
    Failed { failed_index: u32, sqlstate: Option<String>, position: Option<u32>, error: ErrorPayload, conflict: bool },
}
```
  `editable` kuralı (Task 2'de hesaplanır, burada belgelenir): `relkind ∈ {Table, Partitioned}` ∧ sistem şeması değil ∧ `key.is_some()` ∧ (`can_insert ∨ can_update ∨ can_delete`); `read_only_reason` sırası: `UnsupportedKind` > `SystemObject` > `NoKey` > `NoPrivilege`. `locked` `editable`'ı değiştirmez (arayüz ayrıca gösterir; `apply` `locked` kapısı verir).
- `CellValue`, `RowChange`, `DataChangeSet` için **elle `Debug`**: `CellValue::Text` → `Text(…)`, anahtar/sürüm dizileri → `[…]`, kolon adları yazılır olabilir ama **değer yazılmaz** (kolon adı da gizli sayılır: yalnızca sayılar).

- [ ] **Adım 1: Başarısız testler.**
  - `connection.rs`: `Data` profilinin başlangıç seçenekleri `Ddl` ile aynı (`lock_timeout` dahil), `application_name = "pg-tools data"`, `validate_profile(Data{0}) ok`, `Data{MAX+1}` hata; mevcut profil testlerinin tablolarına eklenir.
  - `error_codes.rs`: `DDL_ERROR_CODES.len() == 7`, her kodun `as_str`/`from_marker` gidiş-dönüşü, `ddl_payload(&Error::Invalid(ROW_CONFLICT_MESSAGE))` → kod `row_conflict`.
  - `model.rs`: `serde_json` ile her tipin camelCase/etiketli şekli (`CellValue` üç hâl, `RowChange` üç `op`, `DataOutcome` iki `outcome`); `format!("{:?}", CellValue::Text{value: "gizli-değer".into()})` değeri **içermez**; `format!("{:?}", RowChange::Update{…})` anahtar/sürüm/değer içermez.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula (`lib.rs`: `pub mod data;` + `pub use data::model::*` adlarıyla çakışma yok — `Identity` gibi adlar `model.rs` ile çakışırsa `data::` önekiyle dışa aktarılır, `lib.rs`'te yalnızca `pub mod data;`).
- [ ] **Adım 4:** `cargo test -p pgt-core -p pgt-ddl` (Docker'sız), `cargo clippy --workspace --all-targets -- -D warnings`; bindingler üretilir (`cargo test -p pgt-ddl` export) ve `bun run typecheck` + `bun run build` (ana paket **232,805 B**; `git checkout -- src/ipc/bindings` gürültüsü; yeni binding dosyaları commit).
- [ ] **Adım 5: Commit** (`feat(ddl): veri düzenleyici modeli, SessionProfile::Data ve row_conflict kodu`).

---

### Task 2: Kaynak yükleyici (kolonlar, anahtar seçimi, yetkiler, parmak izi)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/data/source.rs`, `crates/pgt-ddl/src/sql/data_source_info.sql`, `data_source_columns.sql`, `data_source_columns.pg10.sql`, `data_source_keys.sql`
- Değiştir: `crates/pgt-ddl/src/data/mod.rs`
- Test: `crates/pgt-ddl/src/data/source.rs` (saf birim), `crates/pgt-ddl/tests/data_roundtrip.rs` (Docker)

**Arayüzler:**
- Üretir (`source.rs`):

```rust
/// Katalogdan kaynak tanımı. `locked`: çağıran (yönetici) registry'den doldurur → burada `false`.
pub async fn load_source(client: &Client, object: ObjectRef, major: u32) -> Result<DataSource>;
/// Saf: aday indekslerden satır kimliği anahtarını seçer (spec §2 kuralı).
pub fn choose_key(candidates: &[KeyCandidate]) -> Option<DataKey>;
pub struct KeyCandidate {
    pub source: KeySource, pub name: String, pub columns: Vec<String>,
    pub valid: bool, pub ready: bool, pub partial: bool, pub has_expr: bool, pub all_not_null: bool,
}
/// Saf: kolonlar + anahtar + şema/ad → 16 hane hex.
pub fn fingerprint(schema: &str, name: &str, columns: &[DataColumn], key: Option<&DataKey>) -> String;
/// Saf: relkind + sistem şeması + anahtar + yetkiler → (editable, reason).
pub fn editability(relkind: RelKind, schema: &str, key: Option<&DataKey>, can_insert: bool, can_update: bool, can_delete: bool) -> (bool, Option<ReadOnlyReason>);
```
- `data_source_info.sql` (parametre `$1` = oid): `pg_class` + `pg_namespace` → şema, ad, `relkind` (`r p v m f` dışı → `UnsupportedKind`'a düşer: sıra/`sequence`/`index` nesneleri `Invalid("Bu nesnenin verisi görüntülenemez.")`), `has_table_privilege(c.oid, 'SELECT'|'INSERT'|'UPDATE'|'DELETE')` dört bool.
- `data_source_columns.sql` (PG 12+; `.pg10` varyantı `attgenerated` yerine sabit `''`): `attnum > 0 AND NOT attisdropped`, `attnum` sırası; `format_type(atttypid, atttypmod)` (`type_display`), `atttypid`, `attnotnull`, `atthasdef`, `attidentity` (`''|'a'|'d'`), `attgenerated <> ''` → `generated`, `has_column_privilege(c.oid, a.attnum, 'INSERT')` ve `'UPDATE'`; `insertable = insert ∧ !generated`, `updatable = update ∧ !generated ∧ identity != 'a'` (SQL ya da Rust'ta; tek yerde).
- `data_source_keys.sql` (`$1` oid): (a) `contype = 'p'` kısıtı: ad = kısıt adı, kolonlar `conkey` sırasıyla `attname`; (b) `pg_index` satırları (`indisunique`): `indisvalid`, `indisready`, `indpred IS NULL` (→ `partial = indpred IS NOT NULL`), `indkey` ilk `indnkeyatts` öğede `0` var mı (`has_expr`), kolon adları `indkey` sırasıyla, `all_not_null` = bu kolonların hepsi `attnotnull`; PK'nın kendi indeksi (b)'den hariç (`NOT indisprimary`), ad = indeks adı. `INCLUDE` kolonları (`indnkeyatts` ötesi) anahtara girmez. PG 10: `indnkeyatts` yok (PG 11+) → `.pg10` yerine `array_length(indkey)`'i `indnatts` ile kullan; `load_index.pg10.sql` kalıbına bak (tek sorgu içinde `CASE`/`to_jsonb` gerekmiyorsa ayrı `.pg10` dosyası aç).
- `choose_key`: (1) `source = PrimaryKey` olan ilk aday geçerliyse (`valid ∧ ready ∧ !has_expr`; PK için `partial/all_not_null` aranmaz) → o; (2) aksi halde `UniqueIndex` adaylarından `valid ∧ ready ∧ !partial ∧ !has_expr ∧ all_not_null` olanlar arasında **en az kolonlu**, eşitlikte **ad sırasıyla (bayt)** ilk; (3) yoksa `None`.
- `fingerprint`: şema, ad, her kolon için `(name, type_oid, generated, identity)` sırasıyla ve anahtar `(source, name, columns)` üzerinden `DefaultHasher` → `format!("{:016x}")`; **kolon sırası** dahil (yeniden sıralama görünür), `type_display` ve izinler **dahil değil** (yetki değişimi parmak izini bozmaz; yazma yetkisi yine PG'de denetlenir).

- [ ] **Adım 1: Başarısız testler.**
  - Saf `choose_key`: PK varsa PK (benzersiz indeks daha küçük olsa da); PK yoksa en küçük kolonlu geçerli NOT NULL indeks; kısmi/ifadeli/geçersiz/hazır değil/NULL'lanabilir kolonlu indeks atlanır; eşitlikte ad sırası; boş liste `None`; çok kolonlu sıra korunur.
  - Saf `editability`: `Table` + anahtar + yetki → `(true, None)`; `View` → `UnsupportedKind`; `pg_catalog` tablosu → `SystemObject`; anahtarsız → `NoKey`; anahtarlı ama üç yetki de yok → `NoPrivilege`; sıra önceliği (`View` + anahtarsız → `UnsupportedKind`).
  - Saf `fingerprint`: aynı girdi aynı çıktı; kolon eklenince/sırası değişince/anahtar değişince/tip OID'si değişince farklı; `type_display` değişimi aynı kalır.
  - Docker (`data_roundtrip.rs`, her sunucuda; `common` `guarded`): tablo kurulumları — PK'lı; PK'sız ama `UNIQUE NOT NULL` kolonlu; yalnız `UNIQUE` (nullable) → anahtarsız; çok kolonlu PK; `INCLUDE`'lu PK; bölümlü tablo (PG 11+: bölümlü tabloda PK); kalıtım çocuğu; üretilmiş kolon (PG 12+, sürüm kapılı); `GENERATED ALWAYS/BY DEFAULT AS IDENTITY` kolon (PG 10+); view; matview; yabancı tablo gerektirmeyen: yalnızca view/matview; sistem tablosu (`pg_catalog.pg_class`); yetkisiz rol (`GRANT SELECT` yalnız; `SET ROLE`/ayrı bağlantı ile `load_source` çağrısı) → `can_insert=false`, kolon `insertable=false`; `type_display` (`numeric(10,2)`, `text[]`) ve `type_oid` doğru; `fingerprint` ALTER TABLE ADD COLUMN sonrası değişir, `COMMENT` sonrası değişmez.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula (`load_source` `ObjectRef.kind ∈ {Table, View, MaterializedView}` dışını `Invalid` yapar; `ObjectKind` değerlerini mevcut `model.rs`'ten al).
- [ ] **Adım 4:** `cargo test -p pgt-ddl` (saf) + `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf`'tan yalnız `data_roundtrip` testi (komut satırı `cargo test -p pgt-ddl --test data_roundtrip`, `PGT_TEST_URLS` yalnızca test komutunun ortamında); clippy.
- [ ] **Adım 5: Commit** (`feat(ddl): veri düzenleyici kaynak yükleyici — kolonlar, satır kimliği anahtarı, yetkiler, parmak izi`).

---

### Task 3: SELECT üretici (filtre/sıralama kapısı, sürüm kolonu)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/data/select.rs` (+ `snapshots/`)
- Değiştir: `crates/pgt-ddl/src/data/mod.rs`

**Arayüzler:**
- Üretir:

```rust
pub const MAX_FILTER_BYTES: usize = 4096;
pub const MAX_ORDER_KEYS: usize = 8;
/// Saf. `Err` alan hatalarıdır (yol: `filter`, `order[i].column`).
pub fn select_sql(source: &DataSource, req: &SelectRequest) -> std::result::Result<DataSelect, Vec<FieldError>>;
/// Sürüm kolonu takma adı: kaynakta çakışmayan ilk ad (`__pgt_xmin`, `__pgt_xmin_1`, …).
pub fn version_alias(source: &DataSource) -> String;
```
- Çıktı biçimi: `SELECT "c1", "c2", …, xmin::text AS "__pgt_xmin" FROM "s"."t"[ WHERE (<filtre>\n)][ ORDER BY "k1" ASC, "k2" DESC]`. Varsayılan sıralama (`req.order` boş ve `key` var): anahtar kolonları `ASC`; anahtarsız ve `order` boş → `ORDER BY` yok. Kolon listesi **tüm** kaynak kolonları `attnum` sırasıyla. `version_index = columns.len()`. Görünüm/MV/yabancı tablodan `xmin` sistem kolonu yoktur → **`xmin` yalnızca `Table`/`Partitioned`'da** eklenir; diğerlerinde sürüm kolonu yoktur, `has_version = false` ve `version_index = columns.len()` yine döner (arayüz yok sayar).
- Doğrulama: sıralama kolonu `source.columns[].name` içinde olmalı (aksi `order[i].column` hatası), tekrarlı kolon hata, `> MAX_ORDER_KEYS` hata; filtre boş/yalnız boşluksa yok sayılır; filtre NUL/denetim karakteri (`\t \n \r` hariç) içeremez, ≤ `MAX_FILTER_BYTES`, `check_free_sql("filter", …)` (mevcut serbest SQL kapısı: `;` yok, kapanmamış tırnak/yorum/parantez yok, sıradan `'…'` içinde `\` yok) + `freesql::wrap` ile sarma + nihai ifadede `composed_is_clean`. (`check_free_sql`/`wrap`/`composed_is_clean` `build::freesql`'da; görünürlükleri `pub(crate)`'e açılır.)

- [ ] **Adım 1: Başarısız testler (insta + birim).**
  - Anlık görüntü: PK'lı tablo varsayılan; çok kolonlu anahtar sıralama; `desc`; filtreli (`status = 'a'`); anahtarsız tablo `ORDER BY` yok; view (`has_version = false`); kolon adı `"__pgt_xmin"` var → takma ad `__pgt_xmin_1` (ikisi de varsa `_2`); hostile kolon adları (`a"b`, `ç`, boşluklu, anahtar sözcük `user`) `quote_ident` ile tek ifade.
  - Reddedilen filtreler (alan hatası `filter`): `1=1; DROP TABLE x`, `x) OR (1=1`, `a = 'b`, `a /* b`, `a = $$x$$`?? (dolar tırnak: `scan_semicolons` kuralına göre — mevcut kapı ne yapıyorsa o davranış testlenir; **kapı kabul ediyorsa ifade tek kalmalı**: nihai ifadenin `composed_is_clean` olduğu doğrulanır), `a = 'x\y'` (E'…' ipucu), NUL, 4097 bayt.
  - Sıralama hataları: bilinmeyen kolon, tekrar, 9 anahtar.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula.
- [ ] **Adım 4:** `cargo test -p pgt-ddl`, clippy; Docker (`data_roundtrip.rs`'e ek): `select_sql` çıktısı her sürümde **gerçekten çalışır** (PK'lı tablo, view, bölümlü tablo, hostile kolon adlı tablo), dönen kolon sayısı `columns.len() + (has_version ? 1 : 0)`, sürüm kolonu `xmin` ile eşit, filtre çalışır (`WHERE (id > 5\n)`), filtre enjeksiyon denemeleri sunucuya **gitmeden** reddedilir.
- [ ] **Adım 5: Commit** (`feat(ddl): veri düzenleyici SELECT üretici — anahtar sıralı, xmin sürüm kolonlu, doğrulanmış filtre`).

---

### Task 4: Değişiklik planlayıcı (doğrulama + INSERT/UPDATE/DELETE üretimi)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/data/plan.rs` (+ `snapshots/`)
- Değiştir: `crates/pgt-ddl/src/data/mod.rs`

**Arayüzler:**
- Üretir:

```rust
pub const MAX_CHANGES: usize = 2000;
pub const MAX_VALUE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// Saf. `source` güncel yüklenen kaynak; `source.fingerprint == set.fingerprint` denetimi YÖNETİCİDEDİR.
/// Alan hatası yolları: `changes[i]` (genel), `changes[i].values[j]`/`changes[i].set[j]` (hücre), `changes[i].key[j]`,
/// `changes[i].version`, `object` (salt okunur kaynak).
pub fn plan_changes(source: &DataSource, set: &DataChangeSet) -> std::result::Result<Plan, Vec<FieldError>>;
/// Her ifadenin RETURNING kolonları (kaynak sırası + xmin) ve beklenen etkilenen satır sayısı (hep 1) yönetici/yürütücü için.
pub fn returning_clause(source: &DataSource) -> String;       // `"c1", "c2", xmin::text`
```
- Doğrulama kuralları (hepsi toplu: tüm hatalar tek geçişte): `source.editable == false` → tek hata `object` ("Bu nesne düzenlenemez: <neden metni>"; metinler `ReadOnlyReason`'a göre sabit Türkçe); `changes.len() > MAX_CHANGES`; her `CellEdit.column` kaynak kolonlarında olmalı ve aynı değişiklikte tekrar edemez; `Text` değeri NUL içeremez ve ≤ `MAX_VALUE_BYTES`, toplam ≤ `MAX_TOTAL_BYTES`; `Insert`: kolon `insertable` olmalı (aksi hata), `Identity::Always` kolon yalnızca `Default` alabilir, `generated` kolon hata; `Update`: boş `set` hata, kolon `updatable` olmalı, `key.len() == source.key.columns.len()` ve her anahtar öğesi NUL'suz, `version` boş değil ve yalnız ASCII rakam (`xmin` metni: `^[0-9]+$`, ≤ 20 hane), kaynakta `xmin` yoksa (view…) zaten `editable=false`; `Delete`: anahtar + sürüm aynı kurallar; kolon adı eşlemesi **tam** eşleşmedir (büyük/küçük harf duyarlı); `can_insert/can_update/can_delete` false iken ilgili işlem hatası (yetki tablo düzeyindedir; kolon düzeyi `insertable/updatable`).
- Üretim: spec/Global Constraints "İfade biçimleri"; kolon listeleri/atamalar **kaynak kolon sırasıyla** sıralanır (istemci sırası değil); `Insert`te `Default` değerli kolonlar `VALUES`'ta `DEFAULT` olarak **yazılır** (kolon listesinde bırakılmaz → ifade, hangi kolonların açıkça verildiğini gösterir); hiç kolon yoksa `DEFAULT VALUES`. `Statement.risk`: Insert/Update `Safe`, Delete `Destructive`; `transactional = true`; `note`: Delete'te "Satır silinir (geri alınamaz)." Uyarılar (`Plan.warnings`): anahtar kolonu `Update` ile değişiyorsa "Anahtar kolonu değiştiriliyor; bu satırı gösteren yabancı anahtarlar etkilenebilir."; aynı satırı (aynı anahtar) hedefleyen birden çok `Update`/`Delete` varsa "Aynı satır için birden çok değişiklik var; ikincisi çakışma verebilir (satır sürümü değişir)."; `Delete`+`Update` aynı anahtar → aynı uyarı. Boş `changes` → `Ok(Plan::default())`.

- [ ] **Adım 1: Başarısız testler (insta + birim).**
  - Anlık görüntüler: tek `Insert` (karışık `Text/Null/Default`), `DEFAULT VALUES`, `Update` (iki kolon), çok kolonlu anahtarlı `Update`, `Delete`, karışık sıralı küme (istemci sırası korunur), anahtar kolonu güncelleme (+ uyarı), aynı satır iki kez (+ uyarı).
  - Hostile değerler (anlık görüntü + "tek ifade" birim doğrulaması): `'`, `\` (→ `E'…'`), `--`, `;`, `/*`, Unicode (ç, emoji, bidi), çok satırlı metin, boş dize (`''` ≠ NULL), `NULL` metni (`'NULL'` ≠ `NULL`), `DEFAULT` metni (`'DEFAULT'`), uzun değer (sınırda 16 MiB geçer, +1 bayt hata — testte bellek şişirmemek için sınırı **parametreli iç fonksiyonla** küçült), NUL → hata.
  - Reddedilenler: bilinmeyen kolon, yinelenen kolon, `generated` kolona yazma, `Always` kimliğe `Text`, `Always` kimlikte `Update`, `updatable=false` kolon, anahtar uzunluğu uyuşmazlığı, `version` `"12a"`/boş/21 hane, `key` içinde NUL, `editable=false` kaynak (her `ReadOnlyReason`), `can_delete=false`'ta `Delete`, 2001 değişiklik, boş `set`.
  - `returning_clause` kolon adlarını `quote_ident` ile üretir (hostile ad).
  - Risk: yalnız Insert/Update planı `Safe`; Delete içeren plan `Destructive`; `plan_hash` aynı girdide kararlı, tek değer değişince farklı.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula.
- [ ] **Adım 4:** `cargo test -p pgt-ddl`, clippy.
- [ ] **Adım 5: Commit** (`feat(ddl): veri düzenleyici değişiklik planlayıcı — INSERT/UPDATE/DELETE üretimi, doğrulama, risk`).

---

### Task 5: `DataManager` (kapılı uygulama, RETURNING'li tek transaction, iptal)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/data/manager.rs`, `crates/pgt-ddl/src/data/exec.rs`
- Değiştir: `crates/pgt-ddl/src/data/mod.rs`, `crates/pgt-ddl/src/apply.rs` (yalnızca görünürlük: `CancelMap`, `Registration`, `open_apply_conn`, `failure_payload`, `unknown_outcome_payload`, `sqlstate`, `error_position`, `cancelled_error`, `CANCELED_SQLSTATE`, `LOCK_TIMEOUT_SQLSTATE` zaten `pub(crate)`; `open_apply_conn` `SessionProfile::Ddl` sabitliyorsa profil parametreli bir sarmalayıcıya genelleştirilir — `Data` profiliyle çağrılabilmeli; mevcut DDL davranışı **değişmez**)
- Test: `crates/pgt-ddl/tests/data_manager.rs` (Docker), `crates/pgt-ddl/tests/logging.rs` (ek)

**Arayüzler:**
- Üretir (`manager.rs`):

```rust
pub struct DataManager { /* mgr, registry, cancels: CancelMap, rt (DdlManager gibi) */ }
impl DataManager {
    pub fn new(mgr: Arc<ConnectionManager>, registry: Arc<RwLock<Registry>>) -> Self;
    /// Kaynak tanımı (katalog havuzu bağlantısı); `locked` registry'den.
    pub async fn load(&self, server: Uuid, db_oid: u32, object: ObjectRef) -> Result<DataSource>;
    /// SELECT üretimi (kaynağı yükler, `fingerprint` eşitliğini denetler: fark → `plan_changed`).
    pub async fn select(&self, server: Uuid, db_oid: u32, object: ObjectRef, fingerprint: &str, req: &SelectRequest) -> Result<std::result::Result<DataSelect, Vec<FieldError>>>;
    /// Önizleme: yükler, parmak izini denetler, `plan_changes` → `PlanView` (nesne denetimi/kilit YOK; DDL `preview` gibi).
    pub async fn preview(&self, server: Uuid, db_oid: u32, set: &DataChangeSet) -> Result<PlanView>;
    pub async fn apply(&self, server: Uuid, db_oid: u32, set: &DataChangeSet, opts: &DataApplyOptions) -> Result<DataOutcome>;
    pub fn cancel(&self, apply_id: Uuid);
    pub fn cancel_all(&self);
}
```
- `apply` kapı sırası: Global Constraints "Yazma kapısı sırası" (1–8) **aynen**. Hedef: `Target::Database(db_oid)` (ddl `target_for` kalıbı; Database/Role gibi sunucu kapsamı yok). Üretim sunucusu = registry `production` bayrağı (Query Tool'un kullandığı alan; `QueryManager.open`'ın okuduğu `{locked, production}` guard'ının aynı kaynağı — `pgt-core` registry API'sine bak).
- `exec.rs` — `run_changes(client, &statements, reg) -> ExecResult`: `BEGIN`; **her ifade ayrı `client.simple_query`** ile çalışır (metin protokolü: değerler `Option<&str>` olarak zaten metin biçimindedir; ifade tek olduğundan çok ifadeli dizge sorunu yok). `SimpleQueryMessage::Row`'lar `RowResult.returned` (`RETURNING` kolonları + son `xmin`) olur; etkilenen satır sayısı `SimpleQueryMessage::CommandComplete(n)`'den alınır (`DELETE` dahil). Koşul: **her ifade tam 1 satır** etkiler; ≠ 1 → `ROLLBACK` + `ExecResult::Conflict{ index }`. PG hatası → `ROLLBACK` + `ExecResult::Failed{ index, error }` (konum `error_position`). İptal kaydı (`Registration.cancelled()`) her ifadeden önce denetlenir (DDL `run_statements` kalıbı: `cancelled_error`). `COMMIT` hatası SQLSTATE'siz ise `unknown_outcome_payload`; SQLSTATE'liyse (ör. ertelenmiş kısıt 23xxx) `Failed{ index: son ifade }`. Uygulama oturumu hazırlığı `prepare_session` (Ddl ile aynı: `lock_timeout`, `statement_timeout = 0`, `default_transaction_read_only = off`, `standard_conforming_strings = on`).
- Başarısızlıkta `DataOutcome::Failed{ failed_index, sqlstate, position, error: failure_payload(&e) (PG hata iletisi + ipucu), conflict: false }`; çakışmada `conflict: true`, `sqlstate: None`, `position: None`, `error` = `ROW_CONFLICT_MESSAGE` kodlu yük (`ddl_payload` → `row_conflict`).
- `cancel`/`cancel_all`: `DdlManager` ile aynı mantık (`CancelMap::request`, en çok 3 yeniden gönderme; erken gelen iptal için tombstone). `apply_id` kullanımdaysa `Invalid(APPLY_ID_IN_USE)`.
- Günlük: yalnızca `op = "data_apply"`/`"data_preview"`, değişiklik sayısı, ifade sayısı, süre, sonuç kodu (`ok|conflict|failed|locked|…`).

- [ ] **Adım 1: Başarısız testler (Docker, her sunucuda; `data_manager.rs`, `common::guarded`).** Her test kendi tablosunu kurar (`pgt_data_<etiket>`).
  - **Mutlu yol:** `insert` (serial + varsayılan + tetikleyici `NEW` değişikliği) → `returned` satırı ile `select_sql` çıktısı satırı **eşit** (xmin dahil); `update` iki kolon → `returned` yeni değerler ve **yeni xmin**; `delete` → satır yok; üç işlemlik tek apply → hepsi görünür.
  - **Atomiklik:** 3 işlemlik kümede 2. işlem kısıt ihlali (UNIQUE) → `Failed{ failedIndex: 1, conflict: false, sqlstate 23505 }` ve **1. işlem de yok**; ortada çakışma (başka oturum satırı güncelledi) → `conflict: true`, önceki işlemler yok.
  - **Bayatlık:** okuma → başka bağlantı `UPDATE` → bizim `Update` (eski sürüm) → `conflict`; başka bağlantı `DELETE` → `Delete`/`Update` conflict; `BEFORE UPDATE` tetikleyicisi `RETURN NULL` → conflict; aynı satıra iki `Update` (aynı sürüm) → ikincisi conflict (uyarı da önizlemede); `UPDATE SET pk = …` yeni anahtar `returned`'da, yeni anahtarla sonraki `Update` çalışır.
  - **Parmak izi:** `ALTER TABLE ADD COLUMN` sonrası `preview`/`apply`/`select` → `plan_changed`; `COMMENT ON` sonrası etkilenmez.
  - **Kapılar:** kilitli sunucuda `apply` → `locked`, `select`/`load`/`preview` çalışır; üretim sunucusunda `Insert` planı `confirmed=false` → `confirmation_required`, `confirmed=true` → uygulanır; `Delete` planı `confirmed=false` → `confirmation_required`; `expected_hash` oynanırsa `plan_changed`; boş plan `Invalid`; salt okunur kaynakta (view) → `preview` hatası `object`, `apply` `invalid_def`; yetkisiz kolon `updatable=false`; `INSERT` yetkisiz rol (ayrı sunucu kaydı gerektirmeden test için `SET ROLE` yerine **kolon bazlı yetki + sunucudaki test rolü**: Task 2'nin yetkisiz rol altyapısını yeniden kullan; olanaksızsa `lock_timeout`/yetki testini `can_insert=false` kaynak-düzeyi doğrulamasıyla (Task 4) sınırla ve raporla).
  - **Zaman aşımı/iptal:** başka oturum satırı `FOR UPDATE` kilitler → `lock_timeout_ms = 300` → `Failed{ sqlstate: 55P03 }` + hiçbir şey yazılmadı; `lock_timeout_ms = 0` + `cancel(apply_id)` → `57014`; iptal `apply` başlamadan önce gelirse tombstone (iptal edilmiş kalır, DDL kalıbı); `cancel_all`.
  - **Tip gidiş-dönüşü** (tek test, çok kolonlu tablo, `Insert` → `data_select` ile oku → `Update` aynı değerlerle → sürüm değişir ama değerler eşit): `boolean` (`t`/`f`), `numeric`, `float8`, `bytea` (`\x…`), `json`, `jsonb`, `text[]`, `timestamptz`, `uuid`, `enum`, `NULL` ve `''` ayrımı; `xml` PG yapılandırmasına bağlıysa atlanır; `citext` yoksa atlanır.
  - **Bölümlü tablo** (PG 11+): satırın bölüm anahtarını değiştiren `Update` (PG 11+: bölümler arası taşıma) → `returned` + sonraki okuma; kalıtım çocuğu.
  - **Eşzamanlı apply:** aynı `apply_id` ikinci çağrı `Invalid(APPLY_ID_IN_USE)`.
  - **Günlük (`logging.rs` kalıbı):** tracing yakalama altında insert/update/delete + çakışma + hata: günlükte **hiçbir hücre değeri, anahtar, kolon/tablo adı, SQL parçası** yok (benzersiz izleyici dizgeleri `zz_secret_value_zz` ile aranır).
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula (`apply.rs` görünürlükleri + genelleme, `DataManager`, `exec.rs`).
- [ ] **Adım 4:** `cargo test --workspace` (Docker'sız), clippy, `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` (tam matris; PG 10–18 yeşil; bitince kapsayıcılar silinir, diğer kapsayıcılara dokunulmaz).
- [ ] **Adım 5: Commit** (`feat(ddl): DataManager — kapılı, tek transaction'lı, bayat-veri denetimli veri uygulaması (RETURNING, iptal)`).

---

### Task 6: Tauri komutları ve bütünleşme

**Dosyalar:**
- Oluştur: `src-tauri/src/data.rs`
- Değiştir: `src-tauri/src/state.rs` (`AppState.data: DataManager`), `src-tauri/src/main.rs` (`mod data;`, `invoke_handler` kayıtları, sayfa yenilemede ve çıkışta `data.cancel_all()` — `ddl` ile aynı yerlerde), `src/ipc/commands.ts` (yalnızca **tip + ince `call` sarmalayıcıları**)
- Test: `src-tauri/src/data.rs` (Docker kapılı; `ddl.rs`'in `preview_apply_load_roundtrip_and_gates` kalıbı, `testkit::fixture()`)

**Arayüzler:**
- Üretir (`data.rs`; `ddl.rs` modül başlığı kuralları: hiçbir komut SQL/değer günlüklemez; hepsi `Box::pin(...)`; `CmdResult<T> = Result<T, ErrorPayload>`; hata eşleme `ddl_payload`):

```rust
#[tauri::command] pub async fn data_load(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, object: ObjectRef) -> CmdResult<DataSource>;
#[tauri::command] pub async fn data_select(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, object: ObjectRef, fingerprint: String, request: SelectRequest) -> CmdResult<DataSelectResult>;
#[tauri::command] pub async fn data_preview(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, change_set: DataChangeSet) -> CmdResult<PlanView>;
#[tauri::command] pub async fn data_apply(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, change_set: DataChangeSet, options: DataApplyOptions) -> CmdResult<DataOutcome>;
#[tauri::command] pub fn data_cancel(state: State<'_, AppState>, apply_id: Uuid);
```
  `DataSelectResult`: `#[serde(tag = "result", rename_all = "snake_case")] enum { Ok { select: DataSelect }, Invalid { errors: Vec<FieldError> } }` (ts-rs export) — filtre/sıralama alan hataları kapı hatası değil **veri** olarak döner (önizleme gibi). `data_select`'in kapı hataları (`plan_changed`, kayıtsız sunucu) `Err`.
- `src/ipc/commands.ts`: `dataLoad`, `dataSelect`, `dataPreview`, `dataApply`, `dataCancel` — mevcut `ddl*` sarmalayıcılarının biçiminde (`call<T>("data_load", { serverId, databaseOid, object })`). Ana paket bayt artışı **yalnızca** bu beş satırdır: önce `bun run build` ile ölç; **232,805 B'yi aşarsa** sarmalayıcılar `commands.ts`'ten ayrılıp tembel `src/features/data/data-commands.ts`'e (Plan 17 sahibi) taşınır ve `commands.ts` değişmeden kalır (karar: **hiç eklenmez**, sarmalayıcılar Plan 17'de tembel dosyada doğar — bu görevde `commands.ts`'e dokunulmaz, yalnızca bindingler üretilir).
- **Ruling (kesin):** `src/ipc/commands.ts` **değişmez**; TS tarafına yalnızca ts-rs bindingleri girer (ana paket 232,805 B kalır). Task bu cümleye uyar.

- [ ] **Adım 1: Başarısız testler.** `data.rs` Docker testleri (`fixture()`; `PGT_TEST_URLS` yoksa erken dönüş): `data_load` → `data_select` (üretilen SQL `qt_run` ile çalıştırılır, `Collect` sink) → `data_preview` (insert+update+delete karışık) → `data_apply` (confirmed) → yeniden `data_select` ile sonuç beklenenle eşit; `data_preview` hatalı kaynakta alan hatası verir; `data_apply` kilitli sunucuda `locked` kodu; `data_select` filtre hatası `Invalid` verisi; komut kayıt listesi (`main.rs`) beş komutu içerir (derleme).
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula; `main.rs` sayfa yenileme (`on_page_load`) ve çıkış kancalarına `state.data.cancel_all()`.
- [ ] **Adım 4:** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `bun run typecheck`, `bun run build` (**ana `index-*.js` tam 232,805 B**, `index.html` yalnız solid/web/Icon, `__vite__mapDeps` değişmez), `git checkout -- src/ipc/bindings` gürültüsü; yeni binding dosyaları (`DataSource`, `DataColumn`, `DataKey`, `ReadOnlyReason`, `RelKind`, `Identity`, `KeySource`, `CellValue`, `CellEdit`, `RowChange`, `DataChangeSet`, `OrderBy`, `SelectRequest`, `DataSelect`, `DataSelectResult`, `DataApplyOptions`, `RowResult`, `DataOutcome`) commit.
- [ ] **Adım 5: Commit** (`feat(tauri): data_load/data_select/data_preview/data_apply/data_cancel komutları ve AppState.data`).

---

### Task 7: Docker matrisi, belgeler ve bütünleşik doğrulama

**Dosyalar:**
- Değiştir: `docs/smoke-test.md`, `docs/superpowers/specs/2026-10-07-data-editor-design.md`, `docs/testing.md` (Docker test listesine veri düzenleyici testleri)

- [ ] **Adım 1: Tam Docker matrisi.** `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` — yeşil (kapsayıcılar silinir, diğerlerine dokunulmaz). PG 10–18'de yalnızca sürüm kapılı adımlar atlanır (generated PG 12+, bölümlü PK PG 11+); atlamalar rapora.
- [ ] **Adım 2: `docs/smoke-test.md`.** "Veri düzenleyici arka ucu (Plan 16)" bölümü: GUI maddeleri **Plan 17 bölümündedir** notu; arka uç elle kontrol (Query Tool'dan değil, Docker testleriyle kapsanan maddelerin özeti), bilinen sınırlar (float anahtar + PG 10–11 `extra_float_digits`, aynı değere eşzamanlı yazma da çakışma sayılır, filtrede `\` için `E'…'`), "Ölçümler" altına son ana paket baytı ("Plan 16 sonu: index-*.js = N B").
- [ ] **Adım 3: Spec.** Durum satırı "Plan 16 arka ucu uygulandı; Plan 17 ön yüzü kalan"; **Plan 16 sözleşme notları (Plan 17 için)** bölümü: `DataSource`/`editable`/`readOnlyReason`/`locked` anlamı; sürüm kolonu **son kolon** (`DataSelect.versionIndex`, `hasVersion`); `RowChange` anahtarı `DataKey.columns` sırasıyla ve **özgün okunan** değerlerle; `version` = okunan `xmin` metni; çakışma sonrası arayüzün yeniden yüklemesi; `DataOutcome.results[i].returned` ile ızgara tazeleme (kolonlar + son `xmin`); `PlanView.statements[].sql` değer içerir (kısaltarak göster); hata kodları (`locked`, `confirmation_required`, `plan_changed`, `invalid_def`, `row_conflict`); filtre/sıralama hataları `DataSelectResult::Invalid`; ana paket bütçesi (232,805 B, tembel parça, yalnızca sekme türü/eylem kimliği/menü metni ana pakete; telafi) ve backlog (anahtarsız düzenleme, kullanıcı seçimli anahtar, updatable view, OVERRIDING, büyük nesne düzenleyici).
- [ ] **Adım 4: Bütünleşik doğrulama.** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `bun run test`, `bun run typecheck`, `bun run build` (ana `index-*.js` **tam 232,805 B**; preload yalnız solid/web/Icon).
- [ ] **Adım 5: Kapanış greps:** `rg "tracing::" crates/pgt-ddl/src/data` çıktısı yalnızca sayı/süre/kod alanları (tek tek incelenir); `rg "value|key|sql" crates/pgt-ddl/src/data --glob "*.rs"` içinde `tracing` bağlamında değer yok; `rg "expose|Debug" crates/pgt-ddl/src/data/model.rs` özel `Debug` kolları; `rg "ctid"` boş.
- [ ] **Adım 6: Commit** (`docs(data): veri düzenleyici arka ucu sözleşme notları, smoke ve bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 16 bitişi)

- `data_load/select/preview/apply/cancel` uçtan uca PG 10–18'de yeşil; çakışma/kısıt hatası/tetikleyici iptali/iptal/kilit zaman aşımında hiçbir kısmi yazma yok.
- Hücre/anahtar değerleri hiçbir günlükte yok (yakalama testi).
- Ana paket **232,805 B** (değişmedi); yeni TS yalnızca bindingler.
- Elle kontrol maddeleri Plan 17'dedir; final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §1.1 kararlar (anahtar seçimi — Task 2; okuma yolu — Task 3/6; yazma yolu/bayat veri/atomiklik/değer taşıma/risk/sınırlar/üretilmiş kolonlar/gizlilik — Task 4–5), §2 mimari (oturum profili — Task 1; kapı sırası — Task 5; SELECT üretici — Task 3; yetki/sistem şeması — Task 2), §2.1 komutlar (Task 6), §4 güvenlik (Global Constraints + Task 5 günlük testi), §5 test (her görevde), §3 ön yüz **Plan 17**.
- **Bilinçli kararlar:** `xmin` sürümü (Karar 1), tipsiz sabitler (Karar 2), üretimde Safe plan bile `confirmed` ister (Karar 3), ayrı crate yerine `pgt-ddl/data` (apply altyapısı ortak), `commands.ts` değişmez (Task 6 ruling).
- **Yer tutucu taraması:** "TBD/TODO" yok; Task 5'te iki yerde uygulayıcıya bırakılan seçim vardır ve ikisi de kural olarak bağlanmıştır (metin protokolü için `simple_query` kararı; yetkisiz rol testi olanaksızsa rapor).
- **Tip tutarlılığı:** `DataSource.fingerprint` (Task 2) → `DataChangeSet.fingerprint`/`data_select` (Task 3/5/6); `DataSelect.has_version` Task 1 modelinde tanımlıdır, Task 3 doldurur; `RowChange::Update.version` (Task 1) ↔ plan doğrulaması (Task 4) ↔ `xmin::text` karşılaştırması (Task 4/5); `DataOutcome::Failed.conflict` (Task 1) ↔ yürütücü (Task 5) ↔ `row_conflict` kodu (Task 1).
