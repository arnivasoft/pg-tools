# Nesne Diyalogları Arka Uç (Plan 10) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Nesne oluşturma/düzenleme/silme diyaloglarının Rust arka ucu: yeni `pgt-ddl` crate'i (yapılandırılmış nesne tanımları, saf DDL üreticisi, katalogdan yükleyici, korumalı uygulayıcı) ve `ddl_*` Tauri komutları; ilk dört tür **Schema, Sequence, Extension, Index**.

**Mimari:** İstemci SQL göndermez, **tanım** gönderir (`ObjectDef`); arka uç planı (`Plan`) saf Rust işlevleriyle üretir (`create` / `alter(old,new)` / `drop`), önizleme (`PlanView`) döner, `apply` aynı planı yeniden üretip özet (`hash`) denetimiyle uygular. Uygulama kısa ömürlü ayrı bağlantıda (`SessionProfile::Ddl`), transaction'a alınabilen ifadeler tek transaction'da, alınamayanlar ayrı çalışır. Kilit/onay kapıları arka uçta zorlanır. Ön yüz Plan 11'dedir.

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, serde, insta (anlık görüntü testleri; mevcut workspace bağımlılığı), mevcut Docker matrisi.

**Spec:** `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (§2 mimari, §3 risk modeli, §5 türler, §6 güvenlik, §7 test).

## Global Constraints

- PostgreSQL **10–18** vanilla. Sürüm kapıları: `INCLUDE` PG 11+; sequence `AS tür` PG 10+ (`pg_sequence` PG 10+); `GENERATED` bu planda yok. Desteklenmeyen alan üreticide **doğrulama hatası**dır (sessizce atlanmaz).
- **Tanımlayıcılar Rust'ta tırnaklanır** (`quote_ident`/`quote_literal` saf işlevleri, `crates/pgt-ddl/src/quote.rs`); **tüm** tanımlayıcı/değer yerleştirmeleri yalnızca bu yardımcılardan geçer (üreticilerde `format!` ile ham ad yerleştirmek yasak). Aşırı tırnaklama geçerlidir, eksik tırnaklama hatadır: PG'nin `pg_get_keywords()` listesindeki **her** anahtar sözcük (kategori ne olursa olsun) ve `^[a-z_][a-z0-9_$]*$` dışındaki her ad tırnaklanır. Doğrulama: Docker testi her anahtar sözcüğü ve rastgele/Unicode/tırnaklı adları sunucuya `SELECT 1 AS <ours>` ile gönderip sütun adının geri geldiğini denetler.
- Serbest SQL alanları (`IndexDef.where_clause`, ifade anahtarları) **olduğu gibi** yerleştirilir (kullanıcı SQL'i, parantez içinde); başka hiçbir alan SQL olarak yorumlanmaz.
- Arka uç istemciden SQL **almaz**; `apply` planı yeniden üretir ve `expected_hash` ile karşılaştırır.
- Korumalar (arka uçta): `ServerConfig.read_only_lock` → `locked` (`ddl_apply` reddedilir; `preview/load/lookup` serbest); `Destructive`/`DataLoss` içeren plan `confirmed = true` ister (`confirmation_required`); `DataLoss` içeren plan ve sunucu **üretim** etiketliyse ek olarak `confirmed_production = true` (`production_confirmation_required`).
- Uygulama bağlantısı: `application_name = 'pg-tools ddl'`, `default_transaction_read_only = off`, `statement_timeout = 0`, `lock_timeout` çağrıdan (ms; `0` = sınırsız; varsayılan 5000).
- **Günlük:** SQL metni, tanım değerleri, parola/URL asla `tracing`'e, hata iletisine ya da diske yazılmaz; yalnızca nesne türü, ifade sayısı, süre, sonuç kodu.
- ts-rs: dışa verilen tüm tipler `#[derive(Serialize, Deserialize?, TS)]`, `#[serde(rename_all = "camelCase")]` (enum'lar `snake_case`/`tag = "kind"` aşağıda belirtildiği gibi), `#[ts(export)]`, sayılar `#[ts(type = "number")]`; `src/ipc/bindings`'e cargo test sonrası düşen salt satır sonu gürültüsü `git checkout -- src/ipc/bindings` ile geri alınır; yeni binding dosyaları commit edilir.
- Tauri komutları gelecekleri `Box::pin` ile kutular (commands.rs'deki 1 MB ana yığın gerekçesi); hata dönüşümü `ErrorPayload::from`/`ddl_payload`.
- Test sunucuları: Docker'la geçici; `PGT_TEST_URLS` yalnızca test komutunun ortamında; URL/parola asla yazdırılmaz; biten kapsayıcılar silinir (başkalarının kapsayıcılarına dokunulmaz). `scripts/test-matrix.ps1` `cargo test --workspace` çalıştırdığı için yeni crate otomatik girer.
- Commit eki: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Git worktree **kullanılmaz**. Yorumlar Türkçe, çevredeki yoğunlukta.

## Review Focus

- **Enjeksiyon / tırnaklama:** `;`, `"`, `'`, `\`, `--`, `/*`, NUL, çok uzun (>63 bayt), Unicode ve anahtar sözcük adlarıyla her türün create/alter/drop çıktısı tek bir ifade kalır ve ad olarak okunur; `CHECK`/`WHERE` gibi serbest alanlar parantezli yerleştirilir.
- **Kilit/onay kapıları atlanamaz:** istemci `confirmed` bayrağını göndermese de yıkıcı plan çalışmaz; hash uyuşmazlığı (tanım değişti/sürüm değişti) uygulamayı durdurur; kilitli sunucuda hiçbir yazma çalışmaz (preview serbest).
- **Kısmi uygulama:** transaction dışı ifade karışımında hata ortası durum doğru raporlanır (kaç ifade kalıcı); tüm-transactional planlar hata halinde **hiçbir şey** bırakmaz; `lock_timeout` ile kilit bekleyen uygulama düşer ve arkasında bekleyen yan etki bırakmaz; iptal çalışır.
- **Gidiş-dönüş doğruluğu PG 10–18:** `create → load → eşit` ve `alter(old,new) → load → new`; sürüme özel kolonlar (INCLUDE, `pg_sequence`) her sürümde.
- **Kısıt-bağlı indeks:** PK/UNIQUE kısıtının indeksi indeks diyalogundan düzenlenemez/silinemez (açık hata); başka tür/koleksiyondaki düğümler için `ddl_target_of` boş döner (arayüz menü açmaz).
- **Sistem nesneleri:** `pg_catalog`/`information_schema`/`pg_toast` şemaları ve sistem nesneleri düzenlenemez/silinemez (açık hata).

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-core/src/connection.rs` | `SessionProfile::Ddl { lock_timeout_ms }` |
| `crates/pgt-ddl/Cargo.toml`, `src/lib.rs` | crate iskeleti |
| `crates/pgt-ddl/src/quote.rs` | `quote_ident`, `quote_literal`, anahtar sözcük kümesi |
| `crates/pgt-ddl/src/model.rs` | `ObjectKind`, `*Def`, `ObjectDef`, `ObjectRef`, `FieldError` |
| `crates/pgt-ddl/src/plan.rs` | `Statement`, `Risk`, `Plan`, `PlanView`, `plan_hash` |
| `crates/pgt-ddl/src/build/{mod,schema,sequence,extension,index}.rs` | Saf üreticiler + doğrulama |
| `crates/pgt-ddl/src/load.rs`, `sql/*.sql`, `lookup.rs` | Katalogdan tanım yükleme ve açılır liste verileri |
| `crates/pgt-ddl/src/apply.rs`, `manager.rs`, `error_codes.rs` | Uygulayıcı, `DdlManager`, hata kodları |
| `crates/pgt-ddl/tests/*.rs` | Docker entegrasyon testleri |
| `src-tauri/src/ddl.rs`, `state.rs`, `main.rs` | `ddl_*` komutları ve bağlama |

---

### Task 1: `SessionProfile::Ddl` (pgt-core)

**Dosyalar:**
- Değiştir: `crates/pgt-core/src/connection.rs` (profil, `application_name`, `profile_session_options`, `profile_session_sets`, `validate_profile`, `settings_ok_for` ve birim testleri)

**Arayüzler:**
- Üretir: `SessionProfile::Ddl { lock_timeout_ms: u32 }`; `ConnectionManager::open_dedicated(server, target, SessionProfile::Ddl{..})` bu profille çalışır.

- [ ] **Adım 1: Başarısız birim testleri** (mevcut `Query`/`Dashboard` profil testlerinin yanında):

```rust
#[test]
fn ddl_profile_options_and_sets_agree() {
    let p = SessionProfile::Ddl { lock_timeout_ms: 5000 };
    let opts = profile_session_options(p);
    assert!(opts.contains("default_transaction_read_only=off"));
    assert!(opts.contains("statement_timeout=0"));
    assert!(opts.contains("lock_timeout=5000"));
    assert!(!opts.contains("search_path"));
    let sets: Vec<String> = profile_session_sets(p).iter().map(|s| s.to_string()).collect();
    assert!(sets.contains(&"SET default_transaction_read_only = off".to_string()));
    assert!(sets.contains(&"SET statement_timeout = 0".to_string()));
    assert!(sets.contains(&"SET lock_timeout = 5000".to_string()));
}
#[test]
fn ddl_profile_application_name_and_validation() {
    assert_eq!(SessionProfile::Ddl { lock_timeout_ms: 0 }.application_name(), "pg-tools ddl");
    assert!(validate_profile(SessionProfile::Ddl { lock_timeout_ms: 0 }).is_ok());
    assert!(validate_profile(SessionProfile::Ddl { lock_timeout_ms: MAX_STATEMENT_TIMEOUT_MS + 1 }).is_err());
}
#[test]
fn ddl_profile_settings_check() {
    let p = SessionProfile::Ddl { lock_timeout_ms: 5000 };
    assert!(settings_ok_for(p, "off", "0", "public"));
    assert!(!settings_ok_for(p, "on", "0", "public"));
    assert!(!settings_ok_for(p, "off", "30s", "public"));
}
```

- [ ] **Adım 2:** `cargo test -p pgt-core ddl_profile` → derleme hatasıyla başarısız.
- [ ] **Adım 3: Uygula.** `DDL_APPLICATION_NAME = "pg-tools ddl"`; seçenekler `-c default_transaction_read_only=off -c statement_timeout=0 -c lock_timeout={ms}`; `SET` yedeği aynı üç değer; `validate_profile` `lock_timeout_ms > MAX_STATEMENT_TIMEOUT_MS` ise `Error::Invalid`; `settings_ok_for` Ddl kolu: `read_only.trim() == "off" && timeout_ms(statement_timeout) == Some(0)`. Derleyicinin işaret ettiği tüm tam kapsamlı `match`'leri güncelle.
- [ ] **Adım 4:** `cargo test -p pgt-core`, `cargo clippy -p pgt-core -- -D warnings` yeşil.
- [ ] **Adım 5: Commit** (`feat(core): Ddl oturum profili (yazılabilir, lock_timeout parametreli)`).

---

### Task 2: `pgt-ddl` iskeleti — tırnaklama, model, plan ve hata kodları

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/{Cargo.toml,src/lib.rs,src/quote.rs,src/model.rs,src/plan.rs,src/error_codes.rs}` ve testleri
- Değiştir: kök `Cargo.toml` (`members` + `pgt-ddl = { path = "crates/pgt-ddl" }`)

**Arayüzler:**
- Üretir (`quote.rs`): `pub fn quote_ident(name: &str) -> String`, `pub fn quote_literal(s: &str) -> String` (PG `quote_literal` ile aynı: `'…'` tekrarlı tırnak; `\` içeriyorsa `E'…'` ve `\\`), `pub fn qualified(schema: &str, name: &str) -> String`. `KEYWORDS: &[&str]` = sürümden bağımsız tüm PG anahtar sözcükleri (kaynak: `SELECT word FROM pg_get_keywords()` PG 18 çıktısı; sıralı, küçük harf; testte `binary_search`).
- Üretir (`model.rs`; hepsi `Serialize + Deserialize + TS`, camelCase, `#[ts(export)]`):

```rust
#[serde(rename_all = "snake_case")]
pub enum ObjectKind { Schema, Sequence, Extension, Index }
pub struct SchemaDef { pub name: String, pub owner: Option<String>, pub comment: Option<String> }
#[serde(rename_all = "snake_case")] pub enum SeqType { Smallint, Integer, Bigint }
pub struct ColumnRef { pub schema: String, pub table: String, pub column: String }
pub struct SequenceDef {
    pub schema: String, pub name: String, pub owner: Option<String>,
    pub data_type: SeqType,
    #[ts(type = "number")] pub increment: i64,
    #[ts(type = "number | null")] pub min_value: Option<i64>, #[ts(type = "number | null")] pub max_value: Option<i64>,
    #[ts(type = "number")] pub start: i64, #[ts(type = "number")] pub cache: i64, pub cycle: bool,
    pub owned_by: Option<ColumnRef>, pub comment: Option<String>,
}
pub struct ExtensionDef { pub name: String, pub schema: Option<String>, pub version: Option<String>, pub cascade: bool }
#[serde(tag = "kind", rename_all = "snake_case")] pub enum IndexKey { Column { name: String }, Expression { sql: String } }
pub struct IndexColumn { pub key: IndexKey, pub opclass: Option<String>, pub collation: Option<String>, pub descending: bool, pub nulls_first: Option<bool> }
pub struct IndexDef {
    pub schema: String, pub table: String, pub name: String, pub method: String, pub unique: bool,
    pub columns: Vec<IndexColumn>, pub include: Vec<String>, pub where_clause: Option<String>,
    pub tablespace: Option<String>, #[ts(type = "number | null")] pub fillfactor: Option<u32>,
    pub comment: Option<String>,
    /// Yalnızca yüklemede dolar: indeks bir kısıta (PK/UNIQUE/EXCLUDE) aitse kısıt adı. Üreticiler bu indeksi değiştirmez/silmez.
    pub constraint: Option<String>,
}
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectDef { Schema(SchemaDef), Sequence(SequenceDef), Extension(ExtensionDef), Index(IndexDef) }
pub struct FieldError { pub path: String /* ör. "columns[1].key" */, pub message: String }
```

`ObjectDef::kind()`, `ObjectDef::name()` yardımcıları.
- Üretir (`plan.rs`):

```rust
#[serde(rename_all = "snake_case")] #[derive(PartialOrd, Ord, ...)] pub enum Risk { Safe, Destructive, DataLoss }
pub struct Statement { pub sql: String, pub risk: Risk, pub transactional: bool, pub note: Option<String> }
pub struct Plan { pub statements: Vec<Statement>, pub warnings: Vec<String> }
pub fn plan_hash(statements: &[Statement]) -> String;                 // std::hash::DefaultHasher (SipHash, sabit anahtar) → 16 haneli hex; yalnızca bütünlük denetimi
pub struct PlanView { pub statements: Vec<Statement>, pub warnings: Vec<String>, pub errors: Vec<FieldError>, pub hash: String, pub risk: Risk, pub empty: bool }
impl Plan { pub fn risk(&self) -> Risk /* en yüksek */ }
```

- Üretir (`error_codes.rs`, `pgt-query` kalıbı): işaret sabitleri `LOCKED_MESSAGE`, `CONFIRMATION_REQUIRED`, `PRODUCTION_CONFIRMATION_REQUIRED`, `PLAN_CHANGED`, `INVALID_DEF`; `DdlErrorCode { Locked, ConfirmationRequired, ProductionConfirmationRequired, PlanChanged, InvalidDef }` (`ts(export)`), `pub fn ddl_payload(e: &Error) -> ErrorPayload` (işaretli `Invalid` → ayrı kod; gerisi `ErrorPayload::from`). `DDL_ERROR_CODES` listesi.

- [ ] **Adım 1: Başarısız testler** (`quote.rs`):

```rust
#[test] fn plain_lowercase_names_stay_unquoted() { assert_eq!(quote_ident("orders"), "orders"); assert_eq!(quote_ident("_t1$x"), "_t1$x"); }
#[test] fn uppercase_space_digit_start_and_empty_are_quoted() {
    assert_eq!(quote_ident("Order Items"), "\"Order Items\""); assert_eq!(quote_ident("1abc"), "\"1abc\"");
    assert_eq!(quote_ident("Ünï"), "\"Ünï\""); assert_eq!(quote_ident(""), "\"\"");
}
#[test] fn embedded_double_quotes_are_doubled() { assert_eq!(quote_ident("a\"b"), "\"a\"\"b\""); }
#[test] fn every_keyword_is_quoted() { for k in KEYWORDS { assert!(quote_ident(k).starts_with('"'), "{k}"); } assert!(KEYWORDS.windows(2).all(|w| w[0] < w[1])); }
#[test] fn literals() {
    assert_eq!(quote_literal("it's"), "'it''s'"); assert_eq!(quote_literal("a\\b"), "E'a\\\\b'");
    assert_eq!(quote_literal(""), "''");
}
#[test] fn qualified_names() { assert_eq!(qualified("public", "Order"), "public.\"Order\""); }
```

`plan.rs`: `risk()` en yüksek; aynı ifadeler aynı hash, farklı sıra/metin farklı hash; `empty` = ifade yok. `error_codes.rs`: her işaret ayrı koda (pgt-query'deki test kalıbı).

- [ ] **Adım 2:** `cargo test -p pgt-ddl` → başarısız. **Adım 3: Uygula.** `KEYWORDS`: elle üretilmiş liste (PG 18 `pg_get_keywords()`; makul eksiksizlik için PG 10–18 birleşimi); Docker testi (Task 5) sunucu listesiyle karşılaştırıp **sunucuda olup listede olmayan** anahtar sözcük bulursa başarısız olur. Kimlik karakter kuralı: ilk karakter `a-z` ya da `_`, devamı `a-z 0-9 _ $`; aksi tırnaklanır. NUL içeren ad üreticilerde doğrulama hatasıdır (`quote_ident` bunu kabul etmez: `debug_assert`).
- [ ] **Adım 4:** `cargo test -p pgt-ddl`, `cargo clippy -p pgt-ddl --all-targets -- -D warnings`; yeni `src/ipc/bindings/*.ts` dosyalarını ekle.
- [ ] **Adım 5: Commit** (`feat(ddl): pgt-ddl iskeleti — tırnaklama, tanım modeli, plan/risk, hata kodları`).

---

### Task 3: Schema, Sequence, Extension üreticileri

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/build/mod.rs`, `build/schema.rs`, `build/sequence.rs`, `build/extension.rs` (+ `crates/pgt-ddl/src/build/snapshots/` insta dosyaları)

**Arayüzler:**
- Üretir (`build/mod.rs`):

```rust
pub struct BuildCtx { pub version_num: u32 }                          // server_version_num
pub fn validate(def: &ObjectDef, ctx: &BuildCtx) -> Vec<FieldError>;
pub fn create(def: &ObjectDef, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>>;
pub fn alter(old: &ObjectDef, new: &ObjectDef, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>>;   // aynı tür; ad değişimi = RENAME
pub fn drop(def: &ObjectDef, cascade: bool, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>>;
```

`create/alter/drop` önce `validate`'i çalıştırır; hata varsa `Err`. `alter` farklı türler için `Err` (`INVALID_DEF`). Sistem şemaları (`pg_*`, `information_schema`) ve boş/NUL/63 bayt üstü adlar doğrulama hatasıdır. 63 sınırı **bayt** (UTF-8) cinsindendir.

**Kurallar (üretilecek ifadeler):**
- **Schema** — create: `CREATE SCHEMA <q> [AUTHORIZATION <owner>]` + varsa `COMMENT ON SCHEMA <q> IS '<c>'` (Safe). alter: ad → `ALTER SCHEMA <old> RENAME TO <new>` (Safe), sahip → `ALTER SCHEMA <q> OWNER TO <o>` (Safe), yorum → `COMMENT ON …` (`NULL` yorum = `IS NULL`). Birleşik değişiklikte sıra: rename, owner, comment (sonraki ifadeler **yeni** adı kullanır). drop: `DROP SCHEMA <q>` (Destructive) / `CASCADE` (DataLoss, uyarı: "Şemadaki tüm nesneler silinir").
- **Sequence** — create: `CREATE SEQUENCE <schema.name> AS <type> INCREMENT BY n [MINVALUE n | NO MINVALUE] [MAXVALUE n | NO MAXVALUE] START WITH n CACHE n [NO] CYCLE [OWNED BY <s.t.c> | OWNED BY NONE]` (varsayılan olmayanlar yine yazılır — deterministik çıktı), `OWNER`, `COMMENT`. alter: `ALTER SEQUENCE <q> AS/INCREMENT/MINVALUE/…`(değişenler tek ifade), rename `ALTER SEQUENCE … RENAME TO`, `SET SCHEMA`, owner, comment, `OWNED BY`; **başlangıç değeri (`start`) değişimi `ALTER SEQUENCE … START WITH n`** (mevcut sayacı **değiştirmez**; plan notu bunu söyler); mevcut sayacı sıfırlamak ayrı eylemdir (kapsam dışı). drop: `DROP SEQUENCE <q>` (Destructive; `OWNED BY` ya da kolon default'u bağlıysa PG hata verir — `cascade` ile DataLoss).
  Doğrulama: `increment != 0`; `min_value < max_value`; `start` aralıkta; `cache >= 1`; `data_type` sınırları (smallint ±32767, integer ±2147483647, bigint i64); `owned_by` üç parçası dolu.
- **Extension** — create: `CREATE EXTENSION <q> [SCHEMA <s>] [VERSION '<v>'] [CASCADE]` (Safe). alter: `version` değişimi `ALTER EXTENSION <q> UPDATE TO '<v>'` (Safe, not: "Eklentinin güncelleme betiği çalışır"), `schema` değişimi `ALTER EXTENSION <q> SET SCHEMA <s>`; ad değişimi **doğrulama hatası** (PG desteklemez). drop: `DROP EXTENSION <q>` (Destructive) / `CASCADE` (DataLoss).

- [ ] **Adım 1: Başarısız testler.** Tablo odaklı: her tür için create/alter/drop beklenen ifade dizileri (`insta::assert_snapshot!` ile okunabilir anlık görüntü + kritik olanlarda açık `assert_eq!`), no-op alter → boş plan, birleşik değişiklik sırası, risk sınıfları, doğrulama hataları (boş ad, 64 bayt ad, NUL, sistem şeması, `increment = 0`, `min >= max`, `start` aralık dışı, `owned_by` eksik parça, eklenti yeniden adlandırma, tür uyuşmazlığı), tırnaklama (adlarda `"`, `;`, boşluk, anahtar sözcük, Unicode; çıktının **tek ifade** olduğunu — `;` yalnızca tırnaklı ad/literal içinde — saf bir "dış `;` sayacı" yardımcısıyla doğrula).
- [ ] **Adım 2:** Başarısızlığı gör; **Adım 3: Uygula** (her üretici saf; yalnızca `quote.rs` yardımcılarıyla). **Adım 4:** `cargo test -p pgt-ddl`, clippy temiz; `cargo insta` anlık görüntüleri commit edilir (`.snap` dosyaları; `.snap.new` kalmaz).
- [ ] **Adım 5: Commit** (`feat(ddl): Schema, Sequence ve Extension üreticileri`).

---

### Task 4: Index üreticisi

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/build/index.rs` (+ anlık görüntüler); Değiştir: `build/mod.rs`

**Kurallar:**
- **create:** `CREATE [UNIQUE] INDEX <name> ON <schema.table> USING <method> (<anahtarlar>) [INCLUDE (<kolonlar>)] [WITH (fillfactor = n)] [TABLESPACE <ts>] [WHERE (<sql>)]`; anahtar: `Column` → `quote_ident`, `Expression` → `(<sql>)` (parantezli, olduğu gibi), ardından `COLLATE <q>`/opclass (`<opclass>` tırnaklı, şemalı olabilir: `schema.opclass` ayrıştırılmadan **`quote_ident` parça başına** — arayüz iki ayrı alan vermediği için opclass/collation `Option<String>` tek ad kabul edilir ve **tek tanımlayıcı** olarak tırnaklanır; şema nitelikli opclass bu planın kapsamı dışı), `DESC`, `NULLS FIRST|LAST`. `INCLUDE` PG 11+ (aksi halde doğrulama hatası: "INCLUDE PG 11+ gerektirir"). Yöntem kümesi: `btree, hash, gist, gin, brin, spgist` (başka değer doğrulama hatası; kullanıcı eklenti yöntemi için SQL'e gider). `hash` + `unique`/`include`/çok kolon → doğrulama hatası. `fillfactor` 10–100. `CONCURRENTLY` bir **uygulama seçeneğidir**, tanımın parçası değildir: `BuildCtx.concurrently: bool` (üretici `CREATE INDEX CONCURRENTLY` yazar ve `Statement.transactional = false`, not: "Transaction dışında çalışır; hata olursa geçersiz (INVALID) indeks kalabilir"); `BuildCtx`'e bu bayrak eklenir (varsayılan `false`). `COMMENT ON INDEX`. Kolonsuz indeks doğrulama hatası.
- **alter(old,new):** `constraint.is_some()` ise her yapısal/yeniden adlandırma/silme isteği doğrulama hatası: "Bu indeks '<kısıt>' kısıtına aittir; kısıtı tablo düzenleyicisinden değiştirin". Yeniden adlandırma: `ALTER INDEX <old> RENAME TO <new>` (Safe); tablespace: `ALTER INDEX … SET TABLESPACE` (Safe, not: "İndeks yeniden yazılır, kilit alır"); `fillfactor`: `ALTER INDEX … SET (fillfactor = n)` / `RESET (fillfactor)` (Safe, not: "Etkisi için REINDEX gerekir"); yorum; **diğer her yapısal değişiklik** (yöntem, unique, kolonlar, include, where, tablo, şema) → `DROP INDEX <old>` (Destructive) + `CREATE …` (Safe) ve uyarı: "İndeks yeniden oluşturulur; oluşturma süresince sorgular yavaşlayabilir". Aynı planda önce yapısal drop+create, sonra (yeni ada göre) rename değil — rename + yapısal birlikte ise yeni ad doğrudan `CREATE`'te kullanılır ve eski indeks `DROP` edilir.
- **drop:** `DROP INDEX <schema.name>` (Destructive) / `CASCADE` (DataLoss); kısıt-bağlıysa doğrulama hatası. `REINDEX` bu planda ifade değil (Plan 11'de eylem).

- [ ] **Adım 1: Başarısız testler.** Tablo + anlık görüntü: yöntem × seçenek kombinasyonları, ifade anahtarı parantezleme (`lower(email)` → `(lower(email))`), `WHERE` parantezi, `INCLUDE` sürüm kapısı (`version_num = 100000` → hata; `110000` → tamam), `hash` kısıtları, `fillfactor` sınırları, `CONCURRENTLY` → `transactional = false` ve notu, alter senaryoları (yalnız rename; yalnız tablespace; yalnız fillfactor; yapısal → drop+create+uyarı; rename+yapısal; kısıt-bağlı → hata; no-op → boş), tırnaklama (tablo/şema/indeks adlarında özel karakterler, anahtar sözcük kolon adları), enjeksiyon: `name = "x\"; DROP TABLE t; --"` → tek ifade, ad tırnaklı; `WHERE` alanı kullanıcı SQL'i olarak parantez içinde kalır (içindeki `;` testle gösterilir: **bilinen davranış** — kullanıcı SQL'i çok-ifade olabilir mi? **Hayır:** üretici `where_clause`/`Expression.sql` içinde dış `;` bulursa doğrulama hatası verir; `;` yalnızca tırnaklı dize/tırnaklı ad/yorum içinde serbesttir — Task 3'teki "dış `;` sayacı" yardımcısı bunu kullanır).
- [ ] **Adım 2–4:** Başarısızlığı gör, uygula, `cargo test -p pgt-ddl` + clippy temiz.
- [ ] **Adım 5: Commit** (`feat(ddl): Index üreticisi (sürüm kapıları, drop+create yükseltme, CONCURRENTLY)`).

---

### Task 5: Yükleyiciler, açılır liste verileri ve gidiş-dönüş testleri (Docker)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/load.rs`, `crates/pgt-ddl/src/lookup.rs`, `crates/pgt-ddl/src/sql/{load_schema,load_sequence,load_extension,load_index}.sql` (+ `.pg10`/`.pg11` varyantları gerektiğinde), `crates/pgt-ddl/src/sql/lookup_*.sql`, `crates/pgt-ddl/tests/roundtrip.rs`, `crates/pgt-ddl/tests/quote.rs`
- Değiştir: `lib.rs`

**Arayüzler:**
- Tüketir: `ConnectionManager::run(server, Target::Database(oid), …)` (havuzdaki **salt okunur** katalog bağlantısı); `pgt_catalog::version::{PgVersion, VersionedSql}` (bağımlılık: `pgt-catalog`).
- Üretir (`load.rs`):

```rust
pub struct ObjectRef { pub kind: ObjectKind, pub oid: u32 }                 // ts(export)
/// Katalogdan tanım okur; yok ise `Error::NotFound`. Okuma `ObjectDef` döner (üreticinin `create` girdisiyle aynı tip).
pub async fn load(mgr: &ConnectionManager, server: Uuid, database_oid: u32, obj: ObjectRef) -> Result<ObjectDef>;
pub async fn server_version(mgr: &ConnectionManager, server: Uuid) -> Result<u32>;   // server_version_num
```

- Üretir (`lookup.rs`):

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LookupRequest { Roles, Schemas, Tablespaces, AvailableExtensions, IndexMethods, Tables { schema: Option<String> }, TableColumns { schema: String, table: String } }
pub struct LookupItem { pub value: String, pub detail: Option<String> }    // ts(export)
pub async fn lookup(mgr: &ConnectionManager, server: Uuid, database_oid: u32, req: LookupRequest) -> Result<Vec<LookupItem>>;
```

`Roles`: `pg_roles` (sistem rolleri `pg_` önekli hariç), `Schemas`: sistem şemaları hariç, `Tablespaces`: `pg_default` dahil, `AvailableExtensions`: `pg_available_extensions` (`value=name`, `detail="default <v> · yüklü <v|—> · <comment>"`), `IndexMethods`: sabit altı yöntem (sunucuda var olanlarla kesişir), `Tables`: ilişki türleri tablo/partitioned (şema süzgeçli, en çok 5000), `TableColumns`: kolon adları sırayla (`detail=tip`).

- Yükleyici ayrıntıları: **Schema** (`pg_namespace` + sahip + yorum); **Sequence** (`pg_sequence` PG 10+: `seqtypid`, `seqstart`, `seqincrement`, `seqmax`, `seqmin`, `seqcache`, `seqcycle`; `OWNED BY` için `pg_depend` `deptype='a'` ve `refobjsubid > 0`; varsayılan min/max (tipin doğal sınırları ve artışın işaretine göre) `None` döner — üretici `NO MINVALUE`/`NO MAXVALUE` yazar); **Extension** (`pg_extension`, şema, sürüm; `cascade = false`); **Index** (`pg_index` + `pg_class` + `pg_am`; kolon anahtarı `indkey` 0 ise `pg_get_indexdef(indexrelid, k, true)` ifade olarak; opclass yalnızca varsayılan **değilse**; collation yalnızca varsayılandan **farklıysa**; `indoption` bitleriyle `DESC`/`NULLS FIRST`; `indnkeyatts` ile INCLUDE kolonları (PG 11+; PG 10 varyantı INCLUDE'suz); `pg_get_expr(indpred, indrelid)` WHERE; `reloptions` içinden `fillfactor`; `reltablespace`; `constraint` = `pg_constraint.conindid = indexrelid` varsa kısıt adı). Sistem nesnesi (`pg_catalog` vb.) isteği `Error::Invalid("sistem nesnesi düzenlenemez")`.

- [ ] **Adım 1: Başarısız Docker testleri.** `tests/roundtrip.rs` (her sunucu için; `PGT_TEST_URLS` yoksa atlanır; `servers()`/`admin_client` kalıbı `crates/pgt-dash/tests/*.rs` ile aynı; geçici veritabanı `pgt_ddl_rt_<ver>` ve `guarded` temizlik deseni — başarısız assertion sonrası da temizlenir; sabit adlı artıklar başta temizlenir):

```rust
// Her tür için: planı üret (build::create), ifadeleri admin_client ile ÇALIŞTIR (apply henüz yok), nesne oid'ini bul, load() ile oku,
// ve def'e eşit olduğunu doğrula (normalize: yüklemenin doldurduğu alanlar — owner, varsayılan min/max, constraint=None).
async fn create_then_load_equals_def()      // Schema, Sequence (smallint/integer/bigint, OWNED BY, cycle), Extension (ör. pgcrypto/citext benzeri contrib), Index (btree/hash/gin; ifade; DESC NULLS FIRST; INCLUDE sürüm kapısı; WHERE; fillfactor; opclass)
async fn alter_then_load_equals_new()       // her tür için old→new: rename, owner, yorum, sequence parametreleri, extension schema, index tablespace/fillfactor, index yapısal (drop+create)
async fn constraint_backed_index_is_flagged() // PRIMARY KEY indeksi: load.constraint = Some(adı); build::alter/drop → hata
async fn system_objects_are_refused()        // pg_catalog şeması/pg_class_oid_index → load Invalid
async fn lookups_return_expected_shapes()    // roller, şemalar, tablespace (pg_default), available extensions, index methods, tables, columns
```

`tests/quote.rs`: (1) `KEYWORDS` ⊇ `SELECT word FROM pg_get_keywords()` (sunucuda olup listede olmayan sözcük → hata, mesajda sözcük); (2) her `KEYWORDS` öğesi ve 300 rastgele ad (boşluk, `"`, `;`, `'`, `\`, `--`, Unicode, 63 bayt sınırı, çok baytlı karakterle kesilen uzun ad) için `CREATE TABLE t (<quote_ident(name)> int)` yerine hafifçe: `SELECT 1 AS <quote_ident(name)>` çalıştırılır ve sütun adının `name` olduğu denetlenir (rastgele ürete­ç sabit tohumlu, yeniden üretilebilir); (3) `quote_literal` için `SELECT <quote_literal(s)>` sonucu `s`'e eşit (`standard_conforming_strings` açık/kapalı iki oturumda; arka uç oturumu varsayılanı açıktır).

- [ ] **Adım 2: Uygula** yükleyiciler, lookup ve SQL dosyaları. SQL'lerde `$1` OID parametresi (`u32` → `oid`); sütunlar adıyla okunur; `<insufficient privilege>` gibi maskelemeler bu tablolarda yok.
- [ ] **Adım 3:** Docker'lı: tek PG 18 ile `cargo test -p pgt-ddl`; ardından `scripts/test-matrix.ps1 -Versions 10,11,13,18 -SkipPerf` (PG 10 INCLUDE'suz yolu ve PG 11+ INCLUDE'lu yolu birlikte sınanır). Kapsayıcılar silinir.
- [ ] **Adım 4:** `cargo clippy -p pgt-ddl --all-targets -- -D warnings`.
- [ ] **Adım 5: Commit** (`feat(ddl): katalogdan tanım yükleyiciler, açılır liste verileri, gidiş-dönüş ve tırnaklama testleri`).

---

### Task 6: `DdlManager` — önizleme, kapılı uygulama, iptal

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/manager.rs`, `crates/pgt-ddl/src/apply.rs`, `crates/pgt-ddl/tests/manager.rs`
- Değiştir: `lib.rs`, `plan.rs` (`PlanView` üretimi), `error_codes.rs`

**Arayüzler:**
- Tüketir: Task 1 `SessionProfile::Ddl`, Task 3–5 üreticiler/yükleyiciler, `Arc<ConnectionManager>`, `Arc<RwLock<Registry>>` (`ServerConfig.read_only_lock`, `is_production()`).
- Üretir:

```rust
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PlanRequest { Create { def: ObjectDef, concurrently: bool }, Alter { old: ObjectDef, new: ObjectDef }, Drop { def: ObjectDef, cascade: bool } }
pub struct ApplyOptions {
    pub apply_id: Uuid,                       // iptal için istemci üretir
    #[ts(type = "number")] pub lock_timeout_ms: u32,
    pub expected_hash: String, pub confirmed: bool, pub confirmed_production: bool,
}
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ApplyOutcome {
    Applied { statements: u32, #[ts(type = "number")] elapsed_ms: u64 },
    /// `failed_index`: başarısız ifadenin 0 tabanlı sırası; `committed`: kalıcılaşmış (transaction dışı ya da önceki blok) ifade sayısı.
    Failed { failed_index: u32, committed: u32, error: ErrorPayload },
}
impl DdlManager {
    pub fn new(mgr: Arc<ConnectionManager>, registry: Arc<RwLock<Registry>>) -> Self;
    pub async fn preview(&self, server: Uuid, database_oid: u32, req: &PlanRequest) -> Result<PlanView>;   // sürümü sunucudan okur; doğrulama hataları PlanView.errors'ta (Err değil)
    pub async fn apply(&self, server: Uuid, database_oid: u32, req: &PlanRequest, opts: ApplyOptions) -> Result<ApplyOutcome>;
    pub async fn cancel(&self, apply_id: Uuid) -> Result<()>;            // yoksa Ok (idempotent)
    pub fn cancel_all(&self);
    pub async fn load(&self, server: Uuid, database_oid: u32, obj: ObjectRef) -> Result<ObjectDef>;
    pub async fn defaults(&self, server: Uuid, database_oid: u32, kind: ObjectKind, ctx: CreateContext) -> Result<ObjectDef>;
    pub async fn lookup(&self, server: Uuid, database_oid: u32, req: LookupRequest) -> Result<Vec<LookupItem>>;
}
pub struct CreateContext { pub schema_oid: Option<u32>, pub table_oid: Option<u32> }    // koleksiyon düğümünden
```

**Kapı sırası (`apply`):** (1) sunucu kaydı yok → `UnknownServer`; (2) `read_only_lock` → `Invalid(LOCKED_MESSAGE)`; (3) plan üret (`preview` mantığı); doğrulama hatası → `Invalid(INVALID_DEF)`; boş plan → `Invalid("Değişiklik yok")` (kodlu değil, düz `invalid`); (4) `hash != expected_hash` → `Invalid(PLAN_CHANGED)`; (5) `risk >= Destructive && !confirmed` → `Invalid(CONFIRMATION_REQUIRED)`; (6) `risk == DataLoss && production && !confirmed_production` → `Invalid(PRODUCTION_CONFIRMATION_REQUIRED)`; (7) `lock_timeout_ms` doğrulaması (`validate_profile`). Kapılar **bağlantı açmadan önce** değerlendirilir.

**Çalıştırma:** `open_dedicated(server, Target::Database(oid), SessionProfile::Ddl { lock_timeout_ms })`; ardışık `transactional` ifadeler tek blokta `BEGIN … COMMIT` (blok içinde hata → `ROLLBACK`; `committed` önceki bloklar + transaction dışı ifadeler), `transactional = false` ifadeler tek başına `batch_execute`. `CancelHandle` `apply_id` ile bir haritada saklanır (std Mutex, `.await` boyunca tutulmaz); sonuçta kaldırılır; iptal edilen ifade `57014` ile `Failed` olur. `ApplyOutcome::Failed.error` = `ErrorPayload::from(Error::from_query(..))` (PG `message`/`detail`/`hint`; **SQL metni hata iletisine eklenmez**). Günlük: yalnızca tür, ifade sayısı, süre, sonuç.

- [ ] **Adım 1: Başarısız testler (Docker, `tests/manager.rs`):**

```rust
async fn preview_reports_validation_errors_without_error()   // geçersiz def → PlanView.errors dolu, statements boş, hash boş-plan değeri
async fn locked_server_refuses_apply_but_allows_preview()    // read_only_lock → apply Err(code "locked"); preview/load/lookup Ok
async fn destructive_plan_needs_confirmation()               // DROP → confirmed=false → "confirmation_required"; true → uygulanır
async fn production_datalloss_needs_name_confirmation()      // üretim etiketli kayıt + DROP … CASCADE: confirmed_production=false → "production_confirmation_required"
async fn stale_hash_is_refused()                             // preview hash'i ile uygula; arada def değiştir → "plan_changed"
async fn all_transactional_failure_leaves_nothing()          // 2 ifadeli plan, ikincisi hata (ör. var olan ad) → Failed{failed_index:1, committed:0}; ilk ifade de geri alınmış
async fn lock_timeout_aborts_waiting_apply()                 // başka oturum tabloda ACCESS EXCLUSIVE tutarken CREATE INDEX (lock_timeout 500 ms) → ≤ ~2 sn içinde Failed (55P03), indeks yok, yan etki yok; kilit bırakılınca aynı plan başarılı
async fn non_transactional_statement_runs_outside_tx()       // concurrently=true → CREATE INDEX CONCURRENTLY başarılı; plan.statements[0].transactional==false
async fn mixed_plan_reports_partial_commit()                 // [CREATE TABLE benzeri transactional, CONCURRENTLY (hata verecek: aynı ad)] → committed ≥ 1, failed_index doğru (yapay bir karışık plan: PlanRequest üretmek yerine manager iç yardımcısını `#[cfg(test)]` ile statements listesiyle çağır)
async fn cancel_stops_a_running_apply()                      // pg_sleep gibi uzun süren ifade yerine kilit bekleyen uygulama (lock_timeout 0) + cancel(apply_id) → Failed(57014); cancel bilinmeyen id → Ok
async fn password_free_logging()                             // (tracing yakalama) log çıktısı SQL metni/ad içermez
```

- [ ] **Adım 2: Uygula** (`manager.rs` kapılar + plan üretimi; `apply.rs` blok çalıştırıcı + iptal haritası). `preview` sürüm için `server_version_num`'ı havuz bağlantısından okur (`load::server_version`). `defaults`: `Schema` → boş ad, `Sequence/Index` → bağlama göre şema adı/tablo adı çözülür (OID → ad), `Extension` → boş; makul varsayılanlar (`sequence`: `bigint`, artış 1, cache 1, start 1; `index`: `btree`).
- [ ] **Adım 3:** Docker'lı `cargo test -p pgt-ddl` (PG 18) + `scripts/test-matrix.ps1 -Versions 10,13,18 -SkipPerf`; clippy temiz.
- [ ] **Adım 4: Commit** (`feat(ddl): DdlManager — önizleme, kapılı uygulama (kilit/onay/hash), lock_timeout, iptal`).

---

### Task 7: Tauri `ddl_*` komutları ve entegrasyon

**Dosyalar:**
- Oluştur: `src-tauri/src/ddl.rs`
- Değiştir: `src-tauri/Cargo.toml` (`pgt-ddl`), `src-tauri/src/state.rs` (`ddl: Arc<DdlManager>`), `src-tauri/src/main.rs` (modül, `invoke_handler`, çıkışta `cancel_all`), gerekirse `src-tauri/src/commands.rs`

**Arayüzler:**
- Üretir (`dash.rs` kalıbında; her gövde `Box::pin(state.ddl.<yöntem>(…)).await.map_err(|e| ddl_payload(&e))`):

```rust
#[tauri::command] pub async fn ddl_target_of(state: State<'_, AppState>, node_id: String) -> CmdResult<DdlTarget>;
#[tauri::command] pub async fn ddl_defaults(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, kind: ObjectKind, ctx: CreateContext) -> CmdResult<ObjectDef>;
#[tauri::command] pub async fn ddl_load(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, object: ObjectRef) -> CmdResult<ObjectDef>;
#[tauri::command] pub async fn ddl_lookup(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, request: LookupRequest) -> CmdResult<Vec<LookupItem>>;
#[tauri::command] pub async fn ddl_preview(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, request: PlanRequest) -> CmdResult<PlanView>;
#[tauri::command] pub async fn ddl_apply(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, request: PlanRequest, options: ApplyOptions) -> CmdResult<ApplyOutcome>;
#[tauri::command] pub async fn ddl_cancel(state: State<'_, AppState>, apply_id: Uuid) -> CmdResult<()>;
```

`DdlTarget { server_id, database_oid: Option<u32>, object: Option<ObjectRef>, create: Option<DdlCreate> }` (`ts(export)`; `DdlCreate { kind: ObjectKind, context: CreateContext }`). `ddl_target_of(node_id)` düğüm kimliğini `pgt_catalog::node::NodeId` ile çözer (arayüz kimliği ayrıştırmaz — `qt_target_of` ilkesi): `Seg::Schema|Sequence|Extension|Index` düğümü → `object`; `Coll::Schemas|Sequences|Extensions` koleksiyonu → `create` (`schema_oid` ile), `Coll::Indexes` (tablo altında) → `create{kind:Index, table_oid, schema_oid}`; diğer her düğüm → ikisi de `None`. `database_oid`: yoldaki `Seg::Db`; sunucu düzeyindeki düğümler için `None`. Hata: bilinmeyen/bozuk kimlik → `invalid`.
- `ddl_payload` (`pgt-ddl::error_codes`) tek hata eşlemesidir; `ddl_apply` kapı hataları `Err(ErrorPayload)` (kodlu), yürütme hataları `Ok(ApplyOutcome::Failed{..})`.

- [ ] **Adım 1: Başarısız testler.** `ddl_target_of`: schema/sequence/extension/index düğümleri → `object`; koleksiyonlar → `create`; tablo/kolon/rol düğümü → ikisi de `None`; bozuk kimlik → `invalid`. `AppState` kurulunca `ddl` boş (`cancel_all` panik etmez). Docker'lı (kalıp: `commands.rs` testleri): kayıtlı sunucuda `ddl_preview` → `ddl_apply` (schema create) → `ddl_load` eşit; kilitli kayıtta `ddl_apply` kodu `locked`; bağlı olmayan sunucuda `not_connected`.
- [ ] **Adım 2: Uygula.** `main.rs`: komutları `invoke_handler`'a ekle; `RunEvent::Exit`'te `ddl.cancel_all()`; webview yeniden yüklemesinde (`on_page_load` Started) `ddl.cancel_all()` (uçuştaki uygulamanın sahibi arayüz gitti; sunucu tarafı tx zaten bağlantı düşünce geri alınır).
- [ ] **Adım 3: Doğrulama.** `cargo test --workspace` (DB testleri `PGT_TEST_URLS` olmadan atlanır), `cargo test -p pg-tools`, `cargo clippy --workspace --all-targets -- -D warnings`, `bun run typecheck`, `bun run test` (469+ yeşil); Docker'lı: PG 18 ile workspace testleri. `src/ipc/bindings` gürültüsü geri alınır, yeni binding dosyaları commit edilir.
- [ ] **Adım 4: Commit** (`feat(app): ddl_* komutları; çıkış/yeniden yüklemede uygulamaları iptal et`).

---

## Kapanış kontrolü (Plan 10 bitişi)

- Matris: `scripts/test-matrix.ps1` PG 10–18 tümü geçer (kapsayıcılar silinir).
- `rg "tracing::" crates/pgt-ddl/src` çağrıları SQL/ad/parola içermez; `rg "format!" crates/pgt-ddl/src/build` her kullanımı `quote_*` çıktısı ya da sabit anahtar sözcük birleştirir (incelemede tek tek doğrulanır).
- Final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.
