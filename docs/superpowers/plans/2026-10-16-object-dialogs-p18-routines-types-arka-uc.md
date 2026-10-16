# Nesne Diyalogları — Function / Procedure / Trigger / Type / Domain Arka Ucu (Plan 18) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** `pgt-ddl`'e beş yeni nesne türünün **arka ucunu** eklemek: **Function** (trigger fonksiyonları dahil), **Procedure** (PG 11+), **Trigger**, **Type** (enum / composite / range) ve **Domain** — yapılandırılmış tanımlar, saf üreticiler (create / alter farkı / drop), katalog yükleyicileri, Explorer hedef eşlemesi, yeni açılır listeler (diller, trigger fonksiyonları), yönetici entegrasyonu ve PG 10–18 Docker matrisi. **Ön yüz bu planda değil** (Plan 19); bu plan ön yüze yalnızca **tembel derleme uyumu** için dokunur (Task 1).

**Mimari:** Plan 10–14 kalıbı aynen: istemci SQL göndermez, `ObjectDef` gönderir; arka uç `build::{create, alter, drop}` ile planı üretir; `DdlManager` aynı kapılarla uygular (yeniden yükleme eşitliği, plan özeti, onay, üretim onayı, kilit). Yenilikler: (1) **dolar tırnaklı gövde** — fonksiyon gövdesi taranmaz, arka uçta gövdede geçmeyen bir etiketle (`$pgt$`, `$pgt1$`, …) sarılır ve nihai ifade `composed_is_clean` ile tek ifade olduğu doğrulanır; (2) **imzalı kimlik** — fonksiyon `schema.name(giriş tipleri)` ile adreslenir, imza değişimi `OR REPLACE` ile değil açık yeniden kurmayla yapılır; (3) **tablo kapsamlı tür** — Trigger, tablosuna bağlıdır (`DdlCreate.tableOid`); (4) **sürüme bağlı transaction davranışı** — enum `ADD VALUE` PG < 12'de `transactional = false`.

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, insta, mevcut Docker matrisi (`scripts/test-matrix.ps1`).

**Spec:** `docs/superpowers/specs/2026-10-16-object-dialogs-routines-types-design.md` (§1 kararlar, §2 alanlar, §3 risk, §5 güvenlik, §6 test). Temel spec: `docs/superpowers/specs/2026-10-06-object-dialogs-design.md`. Önceki planlar: `2026-10-06-object-dialogs-p10-arka-uc.md`, `…-p12-tablo-arka-uc.md`, `2026-10-07-object-dialogs-p14-view-database-role.md` — burada tekrarlanmayan **tüm Global Constraints ve Review Focus** maddeleri bağlayıcıdır.

## Kararlar

1. **Gövde tırnaklama:** `dollar_tag(body) -> String`: `$pgt$` gövdede geçmiyorsa onu, geçiyorsa `$pgt1$`, `$pgt2$` … ilk geçmeyeni seçer (saf, testli). Gövde boş olamaz, NUL içeremez, ≤ 1 MiB; içeriği **taranmaz** (dil bağımsız). Nihai `CREATE [OR REPLACE] FUNCTION …` ifadesi `composed_is_clean` ile taranır; temiz değilse (olmaması gerekir) `Invalid`.
2. **`OR REPLACE` sınırları:** PG `CREATE OR REPLACE FUNCTION` ile şunları **yapamaz**: giriş argüman tiplerini (imzayı) değiştirmek (yeni fonksiyon olur), dönüş tipini değiştirmek (42P13), giriş argümanının adını değiştirmek (42P13), argüman varsayılanını kaldırmak (42P13), OUT argümanlarını değiştirmek (dönüş tipini değiştirir), Function ↔ Procedure dönüşümü (42809). Üretici bunları **öngörür**: `recreate = false` iken bu değişikliklerden biri varsa alan hatası ("Bu değişiklik fonksiyonu yeniden oluşturmayı gerektirir; 'Yeniden oluştur' seçin."); `recreate = true` iken `DROP FUNCTION <eski imza>` (**DataLoss**, not: "GRANT'lar ve bağımlılar kaybolur; bağımlı nesne varsa PG reddeder") + `CREATE FUNCTION <yeni>` + yorum + sahip. Diğer her gövde/öznitelik değişikliği tek `CREATE OR REPLACE` (Safe; sahiplik/yetkiler korunur).
3. **Eski imza ile adresleme:** `ALTER FUNCTION`, `DROP FUNCTION`, `COMMENT ON FUNCTION` ifadeleri **yüklenen** tanımın giriş tipleriyle (`IN`, `INOUT`, `VARIADIC`; `OUT` hariç) `qualified(schema, name)(tip, …)` biçiminde yazılır; `OR REPLACE` eski adla (ad değişiminden önce) çalışır.
4. **Desteklenmeyenler salt okunur:** `C`/`internal` dili, `prosqlbody` (PG 14+ `BEGIN ATOMIC`), `prokind ∈ {a, w}`, eklentiye ait (`pg_depend` deptype `e`) ve sistem şemasındaki nesneler: yükleyici `unsupported` listesine gerekçe yazar ve `ObjectDef`'te `editable: false` döner; üretici `editable = false` olan eski tanımda ad/sahip/yorum dahil hiçbir değişikliğe izin vermez (alan hatası `object`), `drop` da reddedilir.
5. **Trigger yeniden kurma:** zamanlama/olay/düzey/WHEN/fonksiyon/argüman/REFERENCING/constraint değişirse PG 14+'da `CREATE OR REPLACE TRIGGER` (constraint trigger hariç; Safe), aksi hâlde aynı transaction'da `DROP TRIGGER` + `CREATE [CONSTRAINT] TRIGGER` (**Destructive** + not). Ad değişimi her zaman en önce `ALTER TRIGGER … RENAME TO`; etkinlik `ALTER TABLE … ENABLE/DISABLE/ENABLE REPLICA/ENABLE ALWAYS TRIGGER`; yeniden kurma etkinliği `ORIGIN`'e döndürdüğü için hedef etkinlik `ORIGIN` değilse kurmadan sonra tekrar yazılır. İç trigger'lar (`tgisinternal`; FK ve kısıt tetikleyicileri) Explorer'da gösterilse bile salt okunur; klonlanmış (`tgparentid <> 0`, PG 13+) salt okunur ("bölümlenmiş üst tablodan yönetilir").
6. **Enum:** değerler yalnızca eklenir/yeniden adlandırılır; yüklü bir değerin silinmesi ya da sırasının değişmesi alan hatası ("PostgreSQL enum değerlerini silemez ya da yeniden sıralayamaz."). Eklenen değerin konumu taslaktaki sıradan çıkarılır (`ADD VALUE 'x' AFTER 'y'` ya da ilk konumdaysa `BEFORE 'ilk'`). PG < 12'de her `ADD VALUE` `transactional = false` (PG kuralı); PG ≥ 12'de transactional. Aynı planda yeniden adlandırma (`RENAME VALUE`, PG 10+) ve ekleme birlikte olabilir: önce yeniden adlandırmalar.
7. **Composite:** `ADD ATTRIBUTE` (Safe), `RENAME ATTRIBUTE` (Safe), `ALTER ATTRIBUTE … TYPE` (**DataLoss**), `DROP ATTRIBUTE` (**DataLoss**); typed table'lar kullanıyorsa PG `CASCADE` ister — üretici `CASCADE` yazmaz, PG hatası ipucuyla döner (Task 8 `hint_for`).
8. **Range:** yalnızca create; alter'da yalnız ad/şema/sahip/yorum; alt tip/opclass/collation/canonical/subtype_diff/multirange adı değişimi alan hatası. Multirange adı yalnız PG 14+ (sürüm kapısı).
9. **Domain:** temel tip ve collation create-only (alter'da değişim alan hatası); kısıt ad değişimi `RENAME CONSTRAINT`; ifade değişimi `DROP CONSTRAINT` + `ADD CONSTRAINT` (Destructive); `NOT VALID` yalnız yeni kısıtta; yüklü doğrulanmamış kısıtta "Doğrula" `VALIDATE CONSTRAINT`.
10. **Lookup'lar:** `Languages` (`pg_language` — `lanispl` ve `lanname NOT IN ('c','internal')`; güvenilirlik bilgisiyle; veritabanı kapsamlı), `TriggerFunctions{schema?}` (dönüşü `trigger` olan fonksiyonlar, `schema.name` olarak). Mevcut `Types`, `Collations`, `Schemas`, `Roles`, `TableColumns`, `Settings` yeniden kullanılır.
11. **Kapsam dışı (backlog):** aggregate, operator, cast, event trigger, base type ve multirange create, `BEGIN ATOMIC` gövde düzenleme, C fonksiyonları, `EXECUTE` yetkileri (GRANT matrisi), `SUPPORT` fonksiyonu, `TRANSFORM`, `WINDOW` create, typed table CASCADE otomasyonu, `ALTER TYPE … ADD VALUE IF NOT EXISTS` biçimi.

## Global Constraints

- Plan 10/12/14 Global Constraints aynen: PG **10–18** vanilla; tüm tanımlayıcı/değer yerleştirmeleri yalnızca `quote_ident`/`quote_literal`/`qualified`; ad alanlarında boş/NUL/denetim/görünmez biçim karakteri/63 bayt üstü reddedilir (`check_ident`), sistem şeması reddedilir (`check_schema_ident`); yüklü (eski) taraf yalnız kimlik alanlarıyla gevşek doğrulanır (`check_old_*`); alter yalnızca **değişen ya da ifadeye yazılan** alanları doğrular; `Statement.sql` sonda `;` taşımaz ve tek ifadedir.
- **Serbest SQL alanları (bu plan):** argüman `default`, trigger `when`, domain `default`, domain kısıt `expression` → `check_free_sql(path, …)` + `wrap()` + nihai ifadede `composed_is_clean`; sıradan `'…'` içinde `\` reddedilir (`E'…'`). Fonksiyon `body` → Karar 1 (dolar etiketi; içerik taranmaz).
- **Tip alanları:** argüman tipi, dönüş tipi, `TABLE` kolon tipi, composite öznitelik tipi, domain temel tipi, range alt tipi → `type_syntax::validate_type`; alter "değişti mi" karşılaştırması `type_canon::canonical_type` ile (yüklü `format_type` çıktısına karşı). `SETOF` ve `TABLE` modelde ayrı alanlardır; tip metninde kabul edilmez.
- **Diğer değer alanları:** dil adı → `check_ident` + lookup kümesine üyelik **değil** (sunucu doğrular; `c`/`internal` reddedilir); `cost` > 0, `rows` > 0 (yalnız set dönen), sayısal metin; trigger argümanları → her biri `quote_literal` (NUL yok, ≤ 4096 bayt); enum etiketleri → ≤ 63 bayt, NUL/denetim yok, `quote_literal`; `SET` listesi → mevcut `settings::validate_settings`/`guc_assignment_sql`.
- **Sürüm kapıları (doğrulama hatası, sessiz atlama yok):** Procedure PG 11+; `CREATE OR REPLACE TRIGGER` PG 14+ (aksi hâlde Karar 5 yeniden kurma); `REFERENCING` transition tabloları PG 10+ ve yalnız `AFTER … FOR EACH ROW/STATEMENT` (PG kuralları: INSTEAD OF ve constraint trigger'da yok); range `multirange_type_name` PG 14+; enum `RENAME VALUE` PG 10+ (tabanda var); `PARALLEL` PG 9.6+ (tabanda var).
- **Risk sınıfları:** spec §3 tablosu aynen; `SECURITY DEFINER` açılması plan uyarısı ("Fonksiyon sahibinin yetkileriyle çalışır; `SET search_path = …` önerilir."); `LEAKPROOF` yalnız süper kullanıcı verebilir (PG hatası ipucu).
- **Plan sırası (alter):**
  - Function/Procedure: (1) gövde/öznitelik değiştiyse `CREATE OR REPLACE …` **eski ad ve şemayla** (yeni tanımın diğer alanlarıyla), ya da `recreate` ise `DROP …(eski imza)` + `CREATE …(yeni ad/şema)`; (2) `ALTER … RENAME TO`; (3) `ALTER … SET SCHEMA`; (4) `COMMENT ON …(yeni ad/şema, yeni imza)`; (5) `ALTER … OWNER TO` (**en sonda**).
  - Trigger: (1) `ALTER TRIGGER … RENAME TO`; (2) yeniden kurma (Karar 5; yeni adla); (3) etkinlik; (4) `COMMENT ON TRIGGER`.
  - Type: (1) `RENAME TO`; (2) `SET SCHEMA`; (3) enum `RENAME VALUE`'lar sonra `ADD VALUE`'lar / composite `RENAME ATTRIBUTE`, `DROP ATTRIBUTE`, `ALTER ATTRIBUTE … TYPE`, `ADD ATTRIBUTE` (bu sırayla); (4) `COMMENT`; (5) `OWNER TO`.
  - Domain: (1) `RENAME TO`; (2) `SET SCHEMA`; (3) `SET/DROP DEFAULT`; (4) `DROP NOT NULL`; (5) kısıt `RENAME`, `DROP`, `ADD`, `VALIDATE`; (6) `SET NOT NULL`; (7) `COMMENT`; (8) `OWNER TO`.
- **Kapsam ve hedefler:** Function/Procedure/Type/Domain **şema kapsamlı**; Trigger **tablo kapsamlı** (create'te `DdlCreate.tableOid`, düzenlemede yüklenen tablo; tablo değiştirilemez). `target_of` yeni düğümler: `fn:N`, `proc:N`, `tfn:N` (→ `ObjectKind::Function`), `typ:N` (→ Type; base type düğümünde `object` dolu ama yükleyici salt okunur döner), `dom:N`, `trg:N` (ebeveyn `tbl:T`); koleksiyonlar `functions`, `procedures`, `trigfuncs` (create Function, dönüş `trigger` önden dolu — `DdlCreate.preset = "trigger_function"`), `types`, `domains`, `triggers`. `refreshNodeId`: create'te koleksiyon, düzenleme/silmede `parent.child(Coll)` (Trigger için `tbl:T/triggers`; `tfn` düğümü için `trigfuncs`).
- **Günlük/gizlilik:** gövde, SQL, ad, değer, ifade ve URL asla `tracing`'e, hata iletisine ya da diske yazılmaz.
- **Docker:** yalnızca `scripts/test-matrix.ps1` (`pgt-matrix-<v>` geçici kapsayıcılar; bitince silinir); başka kapsayıcılara dokunulmaz; `PGT_TEST_URLS` yalnızca test komutunun ortamında; sır adlı dosya yazılmaz. Test nesneleri benzersiz önekli şemada (`pgt_ddl_rt_<etiket>`) ve `guarded` temizliğiyle.
- **Ön yüz derleme uyumu (yalnızca Task 1):** yeni `ObjectKind`/`ObjectDef` değerleri TS tiplerini değiştirir; uyum **yalnızca tembel designer dosyalarında** sağlanır (`FORMS` `Partial` + geçici `formMissing` koruması, `KIND_LABELS`, `defName`, `FORM_PATHS` `Partial`, `SCHEMA_MOVE_COLLECTION`); `ddl-kinds.ts`'e yeni tür **eklenmez** (menü Plan 19'da). Ana `index-*.js` **tam 232,343 B** kalmalı; `index.html` yalnızca solid/web/Icon; ana `__vite__mapDeps` 21 girdi.
- `ts-rs`, `Box::pin`, git worktree yasağı, `src/ipc/bindings` satır sonu gürültüsü `git checkout -- src/ipc/bindings`; yeni binding dosyaları commit edilir. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Yorumlar Türkçe, çevredeki yoğunlukta. Ham NUL baytı kabuk üzerinden yazılmaz.

## Review Focus

- **Gövde ve tırnaklama:** gövdede `$pgt$`, `$pgt1$`, `$$`, `$_$`, `;`, `--`, `/*`, `'`, `\`, Unicode, çok satır, CRLF → ifade tek kalır ve gövde baytı baytına korunur (`prosrc` gidiş-dönüşü); seçilen etiket gövdede yok; hostile şema/ad/argüman adı/dil adı.
- **`OR REPLACE` doğruluğu:** imza/dönüş tipi/giriş argüman adı/varsayılan kaldırma/OUT değişimi `recreate`'siz alan hatası verir (PG'ye hiç gitmez); `recreate` DataLoss + üretimde ad yazma; aşırı yüklenmiş fonksiyonlarda yanlış nesne adreslenmez (eski imza); `COMMENT`/`OWNER` yeni imzayla.
- **Sabit nokta:** `alter(loaded, loaded)` her türde **boş plan** (argüman varsayılanlarının `pg_get_function_arg_default` biçimi, tip adlarının `format_type` biçimi, `proconfig` biçimi, trigger `pg_get_triggerdef` alanları, domain kısıt `pg_get_constraintdef` ifadesi, enum sırası, composite öznitelik sırası) — normalleştirme kuralları belgeli.
- **Sürüm davranışları:** PG 10'da Procedure create reddi; PG < 14 trigger yeniden kurma (DROP+CREATE aynı tx) ve etkinlik geri yazımı; enum `ADD VALUE` PG < 12'de transaction dışı ve kısmi uygulamada `committed` doğru; PG 13+ klonlanmış trigger salt okunur; PG 14+ `prosqlbody` salt okunur.
- **Kapılar ve hedefler:** salt okunur nesnelerde (eklenti, sistem, C dili, iç trigger) hiçbir değişiklik ve drop yapılmaz; `target_of` yeni düğümleri doğru `ObjectRef`/`DdlCreate`/`refreshNodeId` ile eşler; kolon/kısıt düğümü odak kuralı trigger'ı etkilemez; 2BP01 bağımlı nesne ipucu (fonksiyon/type/domain drop).

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-ddl/src/model.rs` | `ObjectKind::{Function, Procedure, Trigger, Type, Domain}`, `FunctionDef`, `TriggerDef`, `TypeDef`, `DomainDef` ve yardımcı tipler |
| `crates/pgt-ddl/src/build/routine.rs` (+ `snapshots/`) | Function/Procedure create/alter/drop, `dollar_tag`, imza, `OR REPLACE` sınırları |
| `crates/pgt-ddl/src/build/trigger.rs` | Trigger create/alter/drop, yeniden kurma, etkinlik |
| `crates/pgt-ddl/src/build/pgtype.rs` | Type (enum/composite/range) create/alter/drop |
| `crates/pgt-ddl/src/build/domain.rs` | Domain create/alter/drop |
| `crates/pgt-ddl/src/build/mod.rs` | Dispatch, `validate`, `validate_identity` |
| `crates/pgt-ddl/src/load.rs`, `src/sql/load_function*.sql`, `load_function_args*.sql`, `load_trigger*.sql`, `load_type*.sql`, `load_enum_values.sql`, `load_composite_attrs.sql`, `load_range*.sql`, `load_domain*.sql` | Yükleyiciler (`VersionedSql`) |
| `crates/pgt-ddl/src/lookup.rs`, `src/sql/lookup_languages.sql`, `lookup_trigger_functions.sql` | Yeni açılır listeler |
| `crates/pgt-ddl/src/target.rs` | Yeni düğüm/koleksiyon eşlemeleri, `refreshNodeId`, `DdlCreate.preset` |
| `crates/pgt-ddl/src/manager.rs` | `find_created`, `defaults`, `identity_changed`, `hint_for`, salt okunur kapısı |
| `crates/pgt-ddl/tests/routine_roundtrip.rs`, `type_roundtrip.rs` (+ `common/mod.rs` yardımcıları), `json.rs`, `manager.rs` | Docker gidiş-dönüş ve kapılar |
| `src/features/designer/*` (yalnız tembel, Task 1) | Derleme uyumu |
| `docs/smoke-test.md`, spec'ler | Notlar, durum |

---

### Task 1: Model, hedefler, açılır listeler ve tembel derleme uyumu

**Dosyalar:**
- Değiştir: `crates/pgt-ddl/src/model.rs`, `build/mod.rs` (geçici `Invalid("henüz desteklenmiyor")` kolları), `load.rs` (geçici kollar), `manager.rs` (geçici kollar), `target.rs`, `lookup.rs`, `src/sql/lookup_languages.sql`, `lookup_trigger_functions.sql`; `src/features/designer/{DesignerView.tsx, forms-model.ts, designer-model.ts, refresh-model.ts}` (yalnız tembel uyum), ilgili testler
- Test: `model.rs` serde testleri, `target.rs` birim testleri, `lookup.rs` birim testleri, `tests/json.rs`

**Arayüzler:**
- Üretir (`model.rs`; `ObjectKind` `#[serde(rename_all = "snake_case")]`):

```rust
pub enum ObjectKind { /* mevcutlar */, Function, Procedure, Trigger, Type, Domain }
pub enum ArgMode { In, Out, Inout, Variadic }                         // serde snake_case
pub struct FunctionArg { pub mode: ArgMode, pub name: Option<String>, pub type_name: String, pub default: Option<String> }
pub enum FunctionReturn {                                             // serde tag = "kind", snake_case
    Type { type_name: String, setof: bool },
    Table { columns: Vec<ReturnColumn> },                             // ReturnColumn { name, type_name }
    Trigger,
    Void,
}
pub enum Volatility { Volatile, Stable, Immutable }
pub enum Parallel { Unsafe, Restricted, Safe }
pub struct FunctionDef {
    pub schema: String, pub name: String, pub owner: Option<String>, pub comment: Option<String>,
    pub language: String, pub args: Vec<FunctionArg>, pub returns: Option<FunctionReturn>,   // Procedure'de None
    pub body: String, pub volatility: Volatility, pub strict: bool, pub security_definer: bool,
    pub leakproof: bool, pub parallel: Parallel, pub cost: Option<String>, pub rows: Option<String>,
    pub settings: Vec<ConfigSetting>,
    #[serde(default)] pub recreate: bool,                             // alter-only; yüklemede false
    pub editable: bool,                                               // yükleyici: Karar 4
    pub unsupported: Vec<String>,                                     // salt okunur gerekçeler
}
pub enum TriggerTiming { Before, After, InsteadOf }
pub enum TriggerLevel { Row, Statement }
pub enum TriggerEnabled { Origin, Replica, Always, Disabled }
pub struct TriggerEvents { pub insert: bool, pub update: bool, pub update_columns: Vec<String>, pub delete: bool, pub truncate: bool }
pub struct TriggerDef {
    pub schema: String, pub table: String, pub name: String,
    pub timing: TriggerTiming, pub events: TriggerEvents, pub level: TriggerLevel,
    pub when: Option<String>, pub function_schema: String, pub function_name: String,   // trigger fonksiyonu
    pub args: Vec<String>, pub old_table: Option<String>, pub new_table: Option<String>,
    pub constraint: bool, pub deferrable: bool, pub initially_deferred: bool,
    pub enabled: TriggerEnabled, pub comment: Option<String>, pub editable: bool, pub unsupported: Vec<String>,
}
pub enum TypeBody {                                                   // serde tag = "variant", snake_case
    Enum { values: Vec<EnumValue> },                                  // EnumValue { label: String, original: Option<String> } — original: yüklü etiket (yeniden adlandırma için)
    Composite { attributes: Vec<CompositeAttr> },                     // CompositeAttr { name, type_name, collation: Option<String>, original: Option<String> }
    Range { subtype: String, subtype_opclass: Option<String>, collation: Option<String>, canonical: Option<String>, subtype_diff: Option<String>, multirange_type_name: Option<String> },
}
pub struct TypeDef { pub schema: String, pub name: String, pub owner: Option<String>, pub comment: Option<String>, pub body: TypeBody, pub editable: bool, pub unsupported: Vec<String> }
pub struct DomainCheck { pub name: Option<String>, pub expression: String, pub not_valid: bool, pub validated: bool, pub original: Option<String>, #[serde(default)] pub validate: bool }
pub struct DomainDef { pub schema: String, pub name: String, pub owner: Option<String>, pub comment: Option<String>, pub base_type: String, pub collation: Option<String>, pub default: Option<String>, pub not_null: bool, pub constraints: Vec<DomainCheck>, pub editable: bool, pub unsupported: Vec<String> }
// ObjectDef'e: Function(FunctionDef) — kind "function" ve "procedure" iki ayrı ObjectDef varyantıdır: Function(FunctionDef), Procedure(FunctionDef); Trigger(TriggerDef), Type(TypeDef), Domain(DomainDef)
```
  (Alan adları ts-rs ile camelCase; `type_name` → `typeName`.)
- Üretir (`target.rs`): Global Constraints "Kapsam ve hedefler" eşlemeleri; `DdlCreate`'e `preset: Option<CreatePreset>` (`CreatePreset::TriggerFunction`; serde snake_case) — `trigfuncs` koleksiyonundan create'te dolu; `defaults(Function, preset)` dönüşü `Trigger` ve dili `plpgsql` olan tanım verir (Task 8).
- Üretir (`lookup.rs`): `LookupRequest::Languages`, `LookupRequest::TriggerFunctions { schema: Option<String> }` (veritabanı kapsamlı; `server_wide() = false`).
- Ön yüz (yalnız tembel): `FORMS` ve `FORM_PATHS` yeni türler için `Partial` (formMissing koruması Plan 14 kalıbı), `KIND_LABELS`'e etiketler, `defName` yeni türleri (`schema` + `name`) kapsar; Trigger için tablo bağlamı. `ddl-kinds.ts` **değişmez**.

- [ ] **Adım 1: Başarısız testler.** Serde: her yeni tipin camelCase/etiket şekli (`FunctionReturn` dört biçim, `TypeBody` üç biçim, `ObjectKind` yeni değerler), `tests/json.rs` tür listesine beş tür; `target.rs`: `…/sch:S/fn:N` → `ObjectRef{Function, N}`, `proc:N` → Procedure, `tfn:N` → Function, `typ:N`, `dom:N`, `…/tbl:T/trg:N` → Trigger (`refreshNodeId = …/tbl:T/triggers`), koleksiyonlar → `DdlCreate` (Trigger'da `tableOid = T`; `trigfuncs` → `preset = trigger_function`), bölüm tablosundaki `trg` de eşlenir; `other_nodes_are_empty` testinden bu düğümler çıkarılır; `lookup.rs`: iki yeni istek veritabanı ister (`None` → `NEEDS_DATABASE`).
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula (üretici/yükleyici/yönetici kollarında geçici `Invalid`; derleme her yerde geçer).
- [ ] **Adım 4:** `cargo test -p pgt-ddl` (Docker'sız), `cargo clippy --workspace --all-targets -- -D warnings`, bindingler üretilir; `bun run typecheck`, `bun run test`, `bun run build` (**ana 232,343 B**, mapDeps 21, preload solid/web/Icon); `git checkout -- src/ipc/bindings` gürültüsü; yeni binding dosyaları commit.
- [ ] **Adım 5: Commit** (`feat(ddl): Function/Procedure/Trigger/Type/Domain modelleri, hedef eşlemeleri ve açılır listeler`).

---

### Task 2: Function / Procedure üreticisi

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/build/routine.rs` (+ `snapshots/`)
- Değiştir: `build/mod.rs`

**Arayüzler:**
- Üretir:

```rust
pub fn dollar_tag(body: &str) -> String;                                   // Karar 1
pub fn signature_sql(schema: &str, name: &str, args: &[FunctionArg]) -> String;   // qualified(..)(giriş tipleri)
pub fn create(ctx: &BuildCtx, kind: ObjectKind /* Function|Procedure */, d: &FunctionDef) -> Result<Plan, Vec<FieldError>>;
pub fn alter(ctx: &BuildCtx, kind: ObjectKind, old: &FunctionDef, new: &FunctionDef) -> Result<Plan, Vec<FieldError>>;
pub fn drop(kind: ObjectKind, d: &FunctionDef, cascade: bool) -> Result<Plan, Vec<FieldError>>;
/// OR REPLACE ile yapılamayan değişiklik var mı (Karar 2); alan yollarıyla.
pub fn needs_recreate(kind: ObjectKind, old: &FunctionDef, new: &FunctionDef) -> Vec<FieldError>;
```
- `CREATE [OR REPLACE] FUNCTION <q>(<arg>, …) RETURNS <dönüş> LANGUAGE <q> <VOLATILE|STABLE|IMMUTABLE> [STRICT|CALLED ON NULL INPUT] [SECURITY DEFINER|INVOKER] [LEAKPROOF|NOT LEAKPROOF] PARALLEL <…> [COST n] [ROWS n] [SET a = b …] AS <tag><gövde><tag>`; argüman: `[IN|OUT|INOUT|VARIADIC] [<q ad>] <tip> [DEFAULT (<ifade>\n)]` (`IN` yazılmaz); Procedure: `CREATE [OR REPLACE] PROCEDURE <q>(…) LANGUAGE <q> [SECURITY DEFINER] [SET …] AS …` (dönüş/volatility/strict/leakproof/parallel/cost/rows yazılmaz; doluysa alan hatası). `RETURNS SETOF t`, `RETURNS TABLE (<q ad> <tip>, …)`, `RETURNS trigger`, `RETURNS void`. Trigger fonksiyonu argüman alamaz (alan hatası `args`).
- Doğrulama: ad/şema/argüman adları `check_ident`; argüman adları benzersiz; DEFAULT'lu argümandan sonraki giriş argümanları da DEFAULT'lu olmalı (PG kuralı); `VARIADIC` yalnız son giriş argümanı ve dizi tipi (dilbilgisel kontrol: `[]` ile biten); `OUT` argümanı + `RETURNS TABLE` birlikte olamaz; `ROWS` yalnız set dönen; `cost`/`rows` pozitif sayı; dil `c`/`internal` reddi; gövde Karar 1; sürüm kapısı Procedure PG 11+; `editable = false` eski tanım → `object` hatası.
- Alter: Karar 2 + Global Constraints sırası; `recreate` yalnız `needs_recreate` doluysa anlamlıdır (değilse yok sayılır ve uyarı yok); `recreate` DROP'u **DataLoss**; yeniden adlandırma/şema/sahip/yorum ayrı ifadeler; `SECURITY DEFINER` false→true plan uyarısı; `settings` farkı `OR REPLACE`'i tetikler (tam liste yazılır; PG `proconfig`'i değiştirir).
- [ ] **Adım 1: Başarısız testler (insta + birim):** `dollar_tag` (`$pgt$` yok → `$pgt$`; varsa `$pgt1$`; ikisi de varsa `$pgt2$`), create anlık görüntüleri (skaler, SETOF, TABLE, trigger fonksiyonu, void, procedure, OUT/INOUT/VARIADIC, DEFAULT'lu, SET listeli, SECURITY DEFINER + uyarı, LEAKPROOF, PARALLEL SAFE, COST/ROWS), alter (yalnız gövde → tek OR REPLACE; ad → RENAME; şema; sahip en sonda; yorum yeni imzayla; gövde + ad birlikte → OR REPLACE eski adla, sonra RENAME), `needs_recreate` tablosu (imza tipi, dönüş tipi, giriş argüman adı, DEFAULT kaldırma, OUT değişimi; DEFAULT ekleme/değiştirme `OR REPLACE` ile serbest), recreate planı (DROP eski imza DataLoss + CREATE + yorum + sahip), drop (CASCADE DataLoss), hostile gövde (`$pgt$`, `$$`, `;`, `'`, `\`, CRLF) tek ifade ve baytı baytına, hostile adlar, Procedure PG 10 reddi, trigger fonksiyonunda argüman reddi.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula. **Adım 4:** `cargo test -p pgt-ddl`, clippy. **Adım 5: Commit** (`feat(ddl): Function/Procedure üreticisi — dolar tırnaklı gövde, imzalı kimlik, OR REPLACE sınırları`).

---

### Task 3: Trigger üreticisi

**Dosyalar:** Oluştur `crates/pgt-ddl/src/build/trigger.rs` (+ snapshots); değiştir `build/mod.rs`.

**Arayüzler:** `create(ctx, &TriggerDef)`, `alter(ctx, old, new)`, `drop(d, cascade)` (aynı imza deseni).
- `CREATE [CONSTRAINT] TRIGGER <q> {BEFORE|AFTER|INSTEAD OF} <olay [OR olay …]> ON <qualified tablo> [FROM …] [NOT DEFERRABLE | DEFERRABLE [INITIALLY DEFERRED]] [REFERENCING {OLD|NEW} TABLE AS <q> …] FOR EACH {ROW|STATEMENT} [WHEN (<ifade>\n)] EXECUTE FUNCTION <qualified fn>(<'arg'>, …)` (PG 10'da `EXECUTE PROCEDURE` — sürüm kapısı: PG < 11 `PROCEDURE` anahtar sözcüğü), olay `UPDATE OF <q kolon>, …`.
- Doğrulama (PG kuralları önden): en az bir olay; `TRUNCATE` yalnız `STATEMENT`; `INSTEAD OF` yalnız `ROW` ve view'da (bu plan tablo trigger'ı sunduğu için `INSTEAD OF` reddi — view trigger'ları backlog); `WHEN` `INSTEAD OF`'ta yok; constraint trigger yalnız `AFTER … FOR EACH ROW`; `REFERENCING` yalnız `AFTER`, constraint trigger'da yok, `UPDATE OF` ile birlikte yok; `DEFERRABLE` yalnız constraint; tablo değişimi alan hatası.
- Alter: Karar 5 + sıra; etkinlik ifadesi `ALTER TABLE <tablo> {ENABLE|DISABLE|ENABLE REPLICA|ENABLE ALWAYS} TRIGGER <q>`.
- [ ] **Adım 1: Başarısız testler (insta + birim):** create varyantları (BEFORE ROW INSERT OR UPDATE OF, AFTER STATEMENT TRUNCATE, WHEN, argümanlı, REFERENCING, constraint DEFERRABLE INITIALLY DEFERRED), PG 10 `EXECUTE PROCEDURE`; alter: ad → RENAME; etkinlik; yorum; PG 14+ yapı değişimi → `CREATE OR REPLACE TRIGGER` (Safe); PG 13 → DROP + CREATE aynı tx (Destructive + not) ve etkinlik yeniden yazımı; constraint trigger PG 14+ da DROP+CREATE; kural ihlalleri tablosu; hostile argüman/ad/WHEN.
- [ ] **Adım 2–5:** uygula, `cargo test -p pgt-ddl`, clippy, commit (`feat(ddl): Trigger üreticisi — zamanlama/olay/düzey kuralları, OR REPLACE ya da aynı transaction'da yeniden kurma`).

---

### Task 4: Type üreticisi (enum / composite / range)

**Dosyalar:** Oluştur `crates/pgt-ddl/src/build/pgtype.rs` (+ snapshots); değiştir `build/mod.rs`.

- Create: `CREATE TYPE <q> AS ENUM ('a', 'b')`, `CREATE TYPE <q> AS (<q> <tip> [COLLATE <q>], …)`, `CREATE TYPE <q> AS RANGE (SUBTYPE = <tip> [, SUBTYPE_OPCLASS = <q>] [, COLLATION = <q>] [, CANONICAL = <q>] [, SUBTYPE_DIFF = <q>] [, MULTIRANGE_TYPE_NAME = <q>])`; yorum; sahip en sonda.
- Alter: Karar 6–8 + sıra; enum konum çıkarımı; PG < 12 `ADD VALUE` `stmt_nt`; composite DataLoss sınıfları; range yalnız kimlik alanları.
- Drop: `DROP TYPE <q>` Destructive, CASCADE DataLoss.
- [ ] **Adım 1: Başarısız testler (insta + birim):** enum create, enum alter (ekle başa/ortaya/sona → BEFORE/AFTER doğru; yeniden adlandır; ikisi birlikte sırası; silme/yeniden sıralama hatası; PG 11'de ADD VALUE `transactional = false`, PG 12'de `true`), composite (ekle/yeniden adlandır Safe; tip değiştir/sil DataLoss; aynı adlı silip-ekle = DROP + ADD), range create (PG 13'te `multirange_type_name` reddi; PG 14'te serbest), range alter alt tip değişimi hatası, hostile etiketler (`'`, `\`, Unicode, 63/64 bayt).
- [ ] **Adım 2–5:** uygula, test, clippy, commit (`feat(ddl): Type üreticisi — enum (sürüme bağlı ADD VALUE), composite, range`).

---

### Task 5: Domain üreticisi

**Dosyalar:** Oluştur `crates/pgt-ddl/src/build/domain.rs` (+ snapshots); değiştir `build/mod.rs`.

- Create: `CREATE DOMAIN <q> AS <tip> [COLLATE <q>] [DEFAULT (<ifade>\n)] [NOT NULL] [CONSTRAINT <q> CHECK (<ifade>\n)] …`; `NOT VALID` create'te yok (alan hatası); yorum; sahip.
- Alter: Karar 9 + sıra.
- [ ] **Adım 1: Başarısız testler (insta + birim):** create (collation, default, not null, iki adlı + bir adsız kısıt), alter (default set/drop; not null aç Destructive / kapat Safe; kısıt ekle NOT VALID; kısıt yeniden adlandır; ifade değişimi DROP+ADD Destructive; doğrula; temel tip/collation değişimi hatası; adsız yüklü kısıt — yükleyici adı doldurur, burada adı zorunlu kıl), hostile ifadeler (`;`, `)`, `--`), drop.
- [ ] **Adım 2–5:** uygula, test, clippy, commit (`feat(ddl): Domain üreticisi — default, not null, CHECK kısıtları`).

---

### Task 6: Function / Procedure yükleyicisi ve Docker gidiş-dönüşü

**Dosyalar:** Değiştir `load.rs`; oluştur `src/sql/load_function.sql`, `load_function.pg10.sql`, `load_function.pg14.sql` (`prosqlbody`), `load_function_args.sql`; oluştur `crates/pgt-ddl/tests/routine_roundtrip.rs`; değiştir `tests/common/mod.rs` (`proc_oid(schema, name, args)` yardımcısı).

- Yükleme: `pg_proc` + `pg_namespace` + `pg_language`; `prokind` (PG 11+; PG 10'da `proisagg`/`proiswindow`), argümanlar `proargmodes`/`proargnames`/`proallargtypes`/`proargtypes` + her tip `format_type`, varsayılanlar `pg_get_function_arg_default(oid, n)` (PG 9.4+; giriş argümanı sırasıyla), dönüş (`proretset`, `prorettype` = trigger/void, `TABLE` = `proargmodes` içinde `t`), `prosrc`, `provolatile`, `proisstrict`, `prosecdef`, `proleakproof`, `proparallel`, `procost`/`prorows` (metne çevir; varsayılanlar 100 / 1000 / 0 — **alter sabit noktası**: yüklenen değer yazılır, `None` yalnızca create'te varsayılan anlamına gelir; ayrıntı rapora), `proconfig` (`name=value` bölünür; `settings` modeline), sahip, yorum; `editable`/`unsupported` Karar 4 (`lanname IN ('c','internal')`, `prosqlbody IS NOT NULL`, `prokind IN ('a','w')`, `pg_depend` deptype `e`, sistem şeması).
- [ ] **Adım 1: Başarısız testler (Docker, her sunucuda):** create → load → eşit (skaler sql, plpgsql set dönen, TABLE dönen, trigger fonksiyonu, OUT/INOUT/VARIADIC, DEFAULT'lu, SET listeli, SECURITY DEFINER, procedure PG 11+), `alter(loaded, loaded)` boş plan, gövde baytı baytına (CRLF, `$pgt$`, Unicode), `alter` gövde → `OR REPLACE` uygulanır ve GRANT korunur (`has_function_privilege` ile), yeniden adlandırma + şema + sahip + yorum birlikte, `needs_recreate` örneği PG'ye gitmeden hata, `recreate` uygulanır (yeni imza), bağımlı trigger varken `recreate` → 2BP01 ve hiçbir şey değişmez, aşırı yüklenmiş iki fonksiyondan doğru olan değişir, salt okunur nesneler (`plpgsql_call_handler` gibi C dili; PG 14+ `BEGIN ATOMIC`; eklentiye ait — test kurulumunda eklenti gerektirmeyen bir yol yoksa raporla) `editable = false`.
- [ ] **Adım 2–4:** uygula; `cargo test --workspace` (Docker'sız), clippy, matris (`scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` ya da filtrelenmiş kopya; kapsayıcılar silinir).
- [ ] **Adım 5: Commit** (`feat(ddl): Function/Procedure yükleyicisi ve PG 10–18 gidiş-dönüş testleri`).

---

### Task 7: Trigger / Type / Domain yükleyicileri ve Docker gidiş-dönüşü

**Dosyalar:** Değiştir `load.rs`; oluştur `src/sql/load_trigger.sql` (+ `.pg13` `tgparentid`), `load_type.sql`, `load_enum_values.sql`, `load_composite_attrs.sql`, `load_range.sql` (+ `.pg14` multirange), `load_domain.sql`, `load_domain_checks.sql`; oluştur `crates/pgt-ddl/tests/type_roundtrip.rs` (trigger, type, domain).

- Trigger yükleme: `pg_trigger` (`tgtype` bitleri → zamanlama/olay/düzey; `tgattr` → `update_columns` adları; `tgargs` → bayt dizisi `\000` ayrılmış metinler; `tgqual` → `pg_get_expr`; `tgoldtable`/`tgnewtable`; `tgconstraint <> 0` → constraint + `pg_constraint.condeferrable/condeferred`; `tgenabled` `O|R|A|D`; `tgisinternal`; PG 13+ `tgparentid`), fonksiyon şema+adı, yorum.
- Type: `typtype` (`e`, `c` (+ `relkind = 'c'`), `r`; diğerleri `editable = false`), enum değerleri `enumsortorder` sırasıyla (`original = Some(label)`), composite öznitelikler `attnum` sırası (`attisdropped` hariç; `original = Some(name)`), range `pg_range` (+ PG 14 `rngmultitypid`).
- Domain: `typbasetype` `format_type`, `typcollation` (temel tipin varsayılanından farklıysa), `typdefault` (`pg_get_expr(typdefaultbin)`), `typnotnull`, kısıtlar `pg_constraint contype = 'c'` (`pg_get_constraintdef` → `CHECK (…)` gövdesi ayıklanır, `convalidated`), sıra kısıt adıyla.
- [ ] **Adım 1: Başarısız testler (Docker, her sunucuda):** her tür için create → load → eşit ve `alter(loaded, loaded)` boş; trigger: PG 13'te yapı değişimi DROP+CREATE uygulanır ve etkinlik korunur, PG 14+ OR REPLACE, constraint trigger, REFERENCING, iç (FK) trigger salt okunur, bölümlenmiş tablodaki trigger'ın bölüm klonu salt okunur (PG 13+); type: enum ekle (PG 11'de ayrı transaction — plan kısmi uygulamada `committed` doğru), yeniden adlandır, composite işlemleri, typed table kullanırken öznitelik ekleme PG hatası + ipucu (Task 8 ipucu yoksa yalnız hata), range + multirange PG 14+; domain: kısıt işlemleri, `SET NOT NULL` mevcut NULL'da hata ve geri alma.
- [ ] **Adım 2–5:** uygula, test, clippy, matris, commit (`feat(ddl): Trigger/Type/Domain yükleyicileri ve PG 10–18 gidiş-dönüş testleri`).

---

### Task 8: Yönetici entegrasyonu ve kapılar

**Dosyalar:** Değiştir `manager.rs` (+ `tests/manager.rs`, `tests/logging.rs`).

- `find_created` (yeni türler için `FIND_*_SQL`: fonksiyon `schema + name + imza` ile — `to_regprocedure` kullanmadan `pg_proc` + `pg_get_function_identity_arguments` eşleşmesi; trigger `table oid + name`; type/domain `schema + name`), `defaults(kind, preset)` (Function: `plpgsql`, `returns = Void`, gövde şablonu `BEGIN\n\nEND;`; preset `trigger_function` → `returns = Trigger`, gövde `BEGIN\n  RETURN NEW;\nEND;`; Procedure: `plpgsql`; Trigger: tablo bağlamı, `AFTER INSERT FOR EACH ROW`, fonksiyon boş; Type: `Enum { values: [] }`; Domain: `text`), `identity_changed`, nesne denetimi (yeniden yükleme eşitliği), **salt okunur kapısı** (`editable = false` nesnede alter/drop → kodsuz `invalid` "Bu nesne buradan düzenlenemez: <gerekçe>"), `hint_for` (fonksiyon/type/domain drop ve fonksiyon recreate 2BP01 → bağımlılar ipucu; 42P13 → "OR REPLACE bu değişikliği yapamaz; 'Yeniden oluştur' seçin"; 42809 → "Function ↔ Procedure dönüşümü yeniden oluşturma ister"; typed table 2BP01/0A000 → "Bu tip tablolarda kullanılıyor; CASCADE gerekir (desteklenmiyor)"; 42501 LEAKPROOF → "LEAKPROOF yalnızca süper kullanıcı verebilir").
- [ ] **Adım 1: Başarısız testler (Docker):** `load_and_apply_targets_agree_for_every_kind` yeni türlerle; create → `find_created` doğru oid (aşırı yüklemede doğru imza), alter'da ad değişimi sonrası `created`; kilitli sunucuda beş tür `locked`; DataLoss (recreate, composite drop attr) üretimde `production_confirmation_required`; Destructive (drop) `confirmation_required`; bayat `old` → `plan_changed`; salt okunur nesnede alter/drop reddi; ipuçları; günlükte gövde/ad/değer yok (`logging.rs` kalıbı, ayırt edici sır dizgeleri).
- [ ] **Adım 2–4:** uygula; `cargo test --workspace`, clippy, matris. **Adım 5: Commit** (`feat(ddl): yeni türler için yönetici entegrasyonu — find_created, varsayılanlar, salt okunur kapısı, ipuçları`).

---

### Task 9: Belgeler, matris ve bütünleşik doğrulama

**Dosyalar:** Değiştir `docs/smoke-test.md`, `docs/superpowers/specs/2026-10-16-object-dialogs-routines-types-design.md`, `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (§1.3'ten beş türü çıkar, yeni spec'e işaret), `docs/testing.md`.

- [ ] **Adım 1:** Tam Docker matrisi `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` — yeşil; sürüm kapılı atlamalar rapora.
- [ ] **Adım 2: `docs/smoke-test.md`** — "Nesne diyalogları: Function/Procedure/Trigger/Type/Domain arka ucu (Plan 18)" bölümü (GUI maddeleri Plan 19 bölümünde); bilinen sınırlar (OR REPLACE sınırları, enum silinemez, typed table CASCADE yok, `BEGIN ATOMIC` salt okunur, PG < 14 trigger yeniden kurma, PG < 12 enum ADD VALUE transaction dışı).
- [ ] **Adım 3: Spec** — durum "Plan 18 arka ucu uygulandı; Plan 19 ön yüzü kalan"; **Plan 18 sözleşme notları (Plan 19 için)**: model alanları ve salt okunur/`unsupported`, `recreate` anlamı ve hangi değişikliklerin onu gerektirdiği (`needs_recreate` alan yolları), `defaults(…, preset)`, alan yolları (`args[i].name|typeName|default`, `returns.typeName`, `returns.columns[i].*`, `body`, `language`, `settings[i].*`, `events.updateColumns[i]`, `when`, `args[i]` (trigger), `body.values[i].label`, `body.attributes[i].*`, `constraints[i].*`, …), lookup'lar (`languages`, `triggerFunctions`), ipucu kodları, `SECURITY DEFINER` uyarısı, ön yüz bütçe notu (ana 232,343 B; `ddl-kinds.ts` + telafi).
- [ ] **Adım 4: Bütünleşik doğrulama.** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `bun run test`, `bun run typecheck`, `bun run build` (ana **232,343 B**, mapDeps 21, preload solid/web/Icon).
- [ ] **Adım 5: Kapanış greps:** `rg "tracing::" crates/pgt-ddl/src/build crates/pgt-ddl/src/manager.rs` yeni çağrılarda değer yok; `rg "format!\(.*\{.*body" crates/pgt-ddl/src` yalnız üreticide; `rg "\\$\\$" crates/pgt-ddl/src/build/routine.rs` sabit `$$` yok (yalnız üretilmiş etiket).
- [ ] **Adım 6: Commit** (`docs(ddl): Plan 18 sözleşme notları, smoke ve bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 18 bitişi)

- Beş tür için create/alter/drop/load PG 10–18'de yeşil; sabit nokta her türde; sürüm kapıları ve sürüme bağlı transaction davranışları testli.
- Gövde tek ifade ve baytı baytına; salt okunur nesnelerde değişiklik yok; günlükte gövde/ad/değer yok.
- Ana paket **232,343 B** (değişmedi); yeni TS yalnızca bindingler + tembel derleme uyumu.
- Elle kontrol maddeleri Plan 19'dadır; final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §1 kararlar (gövde → T2; düzenleme stratejisi/OR REPLACE → T2; kimlik → T2/T8; desteklenmeyenler → T6/T8; trigger → T3/T7; type → T4/T7; domain → T5/T7; serbest SQL → T2–T5; tip ifadeleri → T2–T5; SET → T2; sıra → Global Constraints; eklentiye ait → T6/T8), §2 alanlar (T1 modeli), §3 risk (T2–T5), §5 güvenlik (Global Constraints + T8 günlük), §6 test (T2–T8), §4 ön yüz Plan 19.
- **Bilinçli kararlar:** `OR REPLACE` sınırları üreticide öngörülür (PG'ye gitmeden alan hatası); PG < 14 trigger yeniden kurma Destructive; enum değer silme yok; view trigger'ları (`INSTEAD OF`) backlog.
- **Yer tutucu taraması:** Task 1'deki geçici `Invalid` kolları Task 2–8'de kalkar (açıkça belirtilmiş ara durum).
- **Tip tutarlılığı:** `FunctionDef` (T1) → `routine.rs` (T2) → yükleyici (T6) → `defaults`/`find_created` (T8); `TriggerDef.function_schema/function_name` (T1) ↔ `lookup_trigger_functions` (T1) ↔ üretici (T3); `TypeBody` varyantları (T1) ↔ `pgtype.rs` (T4) ↔ yükleyici (T7); `DomainCheck.original/validate` (T1) ↔ `domain.rs` (T5) ↔ yükleyici (T7); `DdlCreate.preset` (T1) ↔ `defaults` (T8).
