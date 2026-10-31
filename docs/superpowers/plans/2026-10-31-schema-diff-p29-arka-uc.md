# Şema Karşılaştırma — Arka Uç (Plan 29) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Yeni `crates/pgt-diff` crate'i ve Tauri komutları. Kapsadıkları:
- `pgt-ddl` yükleyicilerinin davranış değişmeden istemci alan iç fonksiyonlara ayrılması (`load_in`, `load_acl_in`),
- taraf başına liste sorgusu ve tek bağlantılı, tutarlı anlık görüntüyle toplu yükleme (ilerleme, iptal),
- eşleştirme, normalizasyon, durum kararı, aşamalı ifade üretimi, betik sıralaması,
- doğrusal bellekli Myers satır farkı,
- fark kaydı (registry) ve `schema_diff_*` komutları, ts-rs bindingleri, Docker matris testleri.

**Ön yüz bu planda değil** (Plan 30). Yalnız TS derlemesinin gerektirdiği değişiklik yapılır (beklenen: yok).

**Mimari:**
- `pgt-ddl` yükleyicileri ikiye bölünür: `fetch_<tür>` (yalnız sorgular, `tokio_postgres::Error`) ve `parse_<tür>` (satır → `ObjectDef`, saf). `load` bunları eskisi gibi tek `mgr.run` içinde çağırır; `load_in` aynı ikiliyi verilen istemcide çağırır.
- `pgt-diff` katmanları:
  - **IO:** `list` (liste sorgusu) ve `snapshot` (havuz dışı katalog bağlantısı, `REPEATABLE READ READ ONLY` transaction, `search_path`, ardışık düzenli yükleme). Yalnız bu iki modül ve `run` sunucuya dokunur.
  - **Saf:** `key` (anahtar/eşleştirme), `normalize` (şema adı, seçenekler, kimlik eşlemesi, durum kararı), `generate` (öğe başına aşamalı ifadeler), `script` (sıralama, başlık, transaction), `linediff` (Myers).
  - **Durum:** `registry` (fark kimliğine göre sonuç ve iptal belirteci), `run` (iki tarafı eşzamanlı yükler, hepsini birleştirir).
- Tauri katmanı (`src-tauri/src/schema_diff.rs`) yalnız doğrulama, hata eşlemesi ve kayıt defteri işlemlerini yapar. İş, backup desenindeki gibi istemcinin verdiği kimlikle (`diffId`) başlar; komut iş bitince sonucu döndürür, ilerleme `Channel<DiffEvent>` ile akar.

**Teknoloji:** Rust 2024, tokio, tokio-postgres (ardışık düzen: aynı istemcide `futures_util::stream::buffered`), ts-rs, insta, Tauri 2 Channel. Yeni dış bağımlılık yok.

**Spec:** `docs/superpowers/specs/2026-10-31-schema-diff-design.md` (§2 kararlar bağlayıcı, §3 test, §5 bilinen sınırlar). Emsal: Plan 26 (`docs/superpowers/plans/2026-10-18-backup-p26-arka-uc.md`, `crates/pgt-backup/src/{registry.rs,events.rs}`, `src-tauri/src/backup.rs`).

## Kararlar

1. **Spec düzeltmesi — ayrılmış bağlantı:** havuzdaki katalog bağlantısı (`Arc<PgConn>`) Explorer, Inspector ve Designer yükleyicileri tarafından eşzamanlı paylaşılır; onda `BEGIN` açmak başka görevlerin sorgularını bizim transaction'ımıza sokar. Her taraf `ConnectionManager::open_dedicated(server, Target::Database(oid), SessionProfile::Catalog)` ile havuz dışı bir bağlantı açar (oturum ayarları katalog oturumuyla aynı: salt okunur, `statement_timeout = 30000`, `search_path = pg_catalog`). İş bitince ya da iptalde bağlantı düşürülür (kapanır); havuza iade yoktur. Spec §2.1 buna göre düzeltildi.
2. **Spec düzeltmesi — `search_path` sırası:** `SET LOCAL search_path TO pg_catalog, <şema>` (spec'te `<şema>, pg_catalog` idi). Gerekçe: yükleyici SQL'leri katalog tablolarını ve işlevlerini niteliksiz kullanır (`pg_class`, `left(...)`, `'pg_class'::regclass`); şema önde olsaydı kullanıcı şemasındaki aynı adlı nesne yükleyici sorgusunu ele geçirebilirdi. `format_type`/`pg_get_viewdef`/`pg_get_expr` için şemadaki adlar yine yolda göründüğünden niteliksiz çıkar. Betiğin `SET search_path` satırı da aynı sırayı kullanır (yükleme ve uygulama aynı çözümleme kuralını görür). Değer `set_config('search_path', $1, true)` ile parametreli verilir; `$1 = format!("pg_catalog, {}", quote_ident(şema))`.
3. **Spec düzeltmesi — rutin imzası:** eşleştirme anahtarındaki imza `pg_catalog.oidvectortypes(p.proargtypes)`'tır (yalnız giriş tipleri; PG'nin kendi kimlik kuralı). `pg_get_function_identity_arguments` argüman adlarını da içerir; ad değişikliği anahtarı değiştirip §2.4'teki `needs_recreate` yolunu hiç çalıştırmazdı (yalnız kaynakta CREATE hedefte "zaten var" hatası verirdi). Listede gösterilen imza da budur.
4. **Spec düzeltmesi — liste dışlamaları (ek):** kimlik (identity) dizileri (`pg_depend.deptype = 'i'`; `load_sequence` bunları zaten `Invalid` ile reddeder), bölüm indeks klonları (PG 11+, `pg_inherits.inhrelid` = indeks), multirange tipleri (`typtype = 'm'`), aggregate'ler (modellenmez; bilinen sınır) ve uzantı üyesi tabloların indeks/trigger'ları.
5. **Spec düzeltmesi — bölümlenmiş üst tablo:** yükleyici `partition_by`'ı hiç doldurmaz, bölümleme yalnız salt okunur `partition_text`'te taşınır. `build::create` bu tanımla bölümsüz tablo üretirdi (sessiz anlam kaybı). Bu yüzden:
   - `partition_text` yok sayılmaz; iki taraf farklıysa öğe "elle gerekiyor" olur.
   - Yalnız kaynakta olan bölümlenmiş üst tablo da "elle gerekiyor" olur.
   - Spec §2.3 ve §5 düzeltildi.
6. **Spec düzeltmesi — SQL hatası:** transaction içinde bir yükleme sorgusu sunucu hatası verirse (`Error::Query`) transaction bozulur ve sonraki sorgular yürümez. Bu durumda o tarafın işi biter (spec §2.5 "bağlantı kopması ya da iptal" ile aynı sonuç). "Yüklenemedi" öğesi yalnız yükleyicinin Rust düzeyi retleridir (`Invalid`, `NotFound`; ör. sistem nesnesi, geçici tablo). Spec §2.5 düzeltildi.
7. **Spec düzeltmesi — sahiplik ve yorumlar betikte satır içidir:** üreticiler `OWNER TO` ve `COMMENT ON` ifadelerini nesnenin kendi planında üretir; spec "her öğenin ifadeleri kendi üreticisinin sırasını korur" der. Bu yüzden §2.6 adım 4.10 yalnız `GRANT`/`REVOKE` (yetki üreticisi) içerir. Spec düzeltildi.
8. **Spec düzeltmesi — IPC:** backup deseniyle aynı biçim:
   - `schema_diff_start(diffId, source, target, options, onEvent) -> DiffResult`. Kimliği istemci üretir (`crypto.randomUUID()`), komut iş bitince sonucu döndürür. Böylece iptal kimliği başlatmadan önce bilinir.
   - Ek komutlar: `schema_diff_save(diffId, path)` (uygulamada dosya yazan genel bir eklenti yok; son üretilen betik arka uçta yazılır) ve `schema_diff_databases(serverId)` (seçici için bağlanılabilir, şablon olmayan veritabanları).
   - Spec §2.7 düzeltildi.
9. **Model kimlik eşlemesi:** üreticiler eski ↔ yeni eşlemesini yüklemede dolan kimliklerle yapar: kolon `attnum`, kısıt `oid`, enum değeri / composite özniteliği / domain kısıtı `original`. Kaynağın kimlikleri başka veritabanına aittir. Bu yüzden kaynak model, hedefe göre **adla** yeniden eşlenir (`align_to_target`). Eşleşmeyen öğenin kimliği `None` olur (yeni).
10. **FK aşaması:** `create(TableDef)` FK'ları satır içi yazar. Tablolar FK'sız oluşturulur ya da değiştirilir. FK'lar ayrı aşamada eklenir: `alter(eski, yeni)` ile, eski = FK'sız tanım ve yeni = aynı tanım + eklenecek FK'lar. İki tarafın kolonlarına ve kısıtlarına aynı **yapay** kimlikler (`attnum = i+1`, `oid = i+1`) verilir; böylece üretici yalnız `ADD CONSTRAINT` üretir. Yapısı değişen FK birinci aşamada düşer (eski adla `DROP CONSTRAINT`), ikinci aşamada yeniden eklenir.
11. **OWNED BY aşaması ve otomatik silinme:**
    - `OWNED BY` değişiyorsa sequence aşamasında önce bağ çözülür (`OWNED BY NONE`); yeni bağ 4.5'te kurulur. Gerekçe: tablo değişikliği bağlı kolonu düşürürse PG diziyi de siler.
    - Yalnız hedefte olan ve bağlı olduğu tablo da silinen ya da kolonu seçili tablo değişikliğiyle düşen dizi için `DROP SEQUENCE` yazılmaz; betikte açıklama satırı kalır.
12. **Görünüm sırası:** liste sorgusu her görünüm/MV için `pg_rewrite` → `pg_depend` üzerinden bağımlı olduğu ilişki oid'lerini döndürür. Oluşturma kaynak bağımlılık sırasıyla, silme hedef bağımlılığının tersiyle yapılır. Composite/domain/range sırası model başvurularından (öznitelik tipi, temel tip, alt tip adı) çıkarılır. Silinen tablolar hedef FK'larının tersine göre sıralanır.
13. **Transaction ve oturum satırları:** tüm ifadeler `transactional` ise `BEGIN;`, `SET LOCAL check_function_bodies = off;`, `SET LOCAL search_path TO pg_catalog, <hedef>;` … `COMMIT;` kullanılır. Değilse `SET` (LOCAL'sız) kullanılır ve `BEGIN` yazılmaz.
14. **Bilinen sınırlar (plan düzeyi; ön yüz notları için):**
    - Yeniden kurulan MV'nin indeksleri betikte yeniden oluşturulmaz (üretici uyarısı). Yeniden karşılaştırmada "yalnız kaynakta" görünürler.
    - `pg_get_*` işlevleri katalog önbelleğini okur; eşzamanlı DDL varsa anlık görüntüden küçük sapma olabilir.
    - Yabancı tablolar (`relkind 'f'`) ve aggregate'ler listelenmez.
    - Sahiplik/yetki seçeneğinde hedefin `ALTER DEFAULT PRIVILEGES` ayarları yeni nesnenin ilk ACL'sini değiştirebilir; temel alınan değer `acldefault`'tur.
    - Kolon düzeyi yetkiler ve sahipten başka verenlerin yetkileri karşılaştırılmaz (not verilir).

## Global Constraints

- **Gizlilik (en önemli):**
  - `tracing`'e yalnız sayımlar, süreler ve sonuç (`ok`/`cancelled`/`failed`) yazılır. SQL metni, şema/nesne/veritabanı adı, değer, yol ve ileti metni yazılmaz.
  - Betik başlığında sunucu adresi, kullanıcı adı ya da parola yoktur; başlık üreticisi bu bilgileri **almaz** (imza düzeyinde imkânsız).
  - Test URL'leri ve parolaları yazdırılmaz.
  - Testler günlük yakalamayla doğrular (Task 9).
- **Davranış koruması (Task 1):**
  - `load`/`load_acl` dışarıdan aynı kalır: aynı SQL dosyaları, aynı sorgu sırası ve koşulları, yükleyici başına tek `mgr.run` (yeniden deneme semantiği).
  - SQL dosyalarına dokunulmaz. Designer/Inspector testleri değişmeden geçer.
- **Salt okunurluk:**
  - Karşılaştırma hiçbir zaman yazmaz: transaction `READ ONLY`'dir, oturum katalog profilidir.
  - Betik uygulanmaz; yalnız metin üretilir (Plan 30 Query Tool'a aktarır).
- **Bağlantı:**
  - Havuzdaki bağlantıların `search_path`'i ve durumu değişmez.
  - Ayrılmış bağlantı her çıkış yolunda (başarı, hata, iptal, panik) düşer. `idle in transaction` oturum kalmaz.
- **Sürüm:**
  - Üreticiler **hedefin** sürümüyle çağrılır: `BuildCtx { version_num: hedef, concurrently: false }`.
  - Liste sorgusu `VersionedSql` ile PG 10 / 11 / 13 biçimlerine ayrılır.
- **Yeni SQL:** yalnız `pgt-diff`'in liste ve şema denetimi sorguları. Bütün katalog adları `pg_catalog.` ile nitelenir; değerler yalnız parametredir.
- **Docker testleri:**
  - `PGT_TEST_URLS` yoksa test atlanır ve bunu yazar. Matris yalnız `powershell -File scripts/test-matrix.ps1 -Versions <liste> -SkipPerf` ile çalıştırılır.
  - Kapsayıcılar geçici (`pgt-matrix-*`) ve her zaman silinir; başka kapsayıcıya dokunulmaz.
  - Test veritabanları `pgt_diff_<ana sürüm>_<etiket>` adını alır ve `guarded` temizliğiyle silinir.
- **Genel:**
  - Bindingler commit edilir; satır sonu gürültüsü `git checkout -- src/ipc/bindings` ile atılır.
  - Ön yüze dokunulmaz (`commands.ts` dahil).
  - Yorumlar Türkçe, çevredeki üslupla.
  - Crate genelinde `cargo fmt` çalıştırılmaz; dokunulan satırlar elle biçimlendirilir.
  - `graphify-out/` commit edilmez.
  - Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Doğrulama (her görev sonu):** `cargo test -p <crate>` (görevin crate'i), `cargo clippy --workspace --all-targets -- -D warnings`, `~/.bun/bin/bun run typecheck`. Task 8–9 ek olarak `bun run test`, `bun run build`, `bun run bundle:check`.

## Review Focus

- **Kimlik eşlemesi ve kolon sırası:**
  - Kaynak `attnum`/kısıt `oid`/`original` değerleri hedefe adla eşlenmezse üretici ya "tanım bayat" hatası verir ya da yanlış kolonu düşürür.
  - Sıra farkı fark sayılmaz: eşleşen kolonlar hedef sırasına dizilir, yeniler sona eklenir, öğeye not düşülür.
  - Testler:
    - Task 5: kaynakta `attnum` 1,2,3, hedefte farklı attnum'lu aynı adlar → "Aynı" + not;
    - Task 5: enum'a ortaya değer eklenir → `original` eşlemesi;
    - Task 6: yeniden adlandırılmış kolon → `DROP COLUMN` + `ADD COLUMN`, DataLoss.
- **Modelin taşımadığı yapı:**
  - Bölümlenmiş üst tablo bölümsüz oluşturulmamalı; bölüm ve kalıtım çocukları elle gerekiyor olmalı.
  - MV `unsupported`'taki `INDEX …` girdileri fark sayılmamalı (indeksler ayrı öğedir); `COMMENT …` girdileri yorum seçeneğine uymalı.
  - MV `with_data` farkı "Aynı" olmalı.
  - Enum ↔ composite gibi tip gövdesi değişimi tür değişimi gibi DROP + CREATE olmalı (üretici varyant değişimini reddeder).
  - Testler: Task 5 durum tablosu, Task 9 Docker (bölümlü tablo, MV indeksi).
- **Niteleme ve imza:**
  - `search_path` `pg_catalog, <şema>` iken görünüm gövdesi, `nextval('t_id_seq'::regclass)` varsayılanı, enum kolon tipi ve rutin argüman tipi niteliksiz çıkmalı. Şema adları farklı iki tarafta aynı metin oluşmalı.
  - Gövdede kaynak şemayı açıkça niteleyen ad (`src.t`, `"src".t`) korunmalı ve not almalı; `srcx.t` ve `a.src` not almamalı.
  - Aşırı yüklemeler `oidvectortypes` ile ayrı öğeler olmalı; argüman adı değişimi aynı öğe kalıp `needs_recreate` yolundan DROP + CREATE olmalı.
  - Testler: Task 1 ve Task 4 Docker, Task 5 `mentions_schema`, Task 9 farklı adlı şemalarla gidiş-dönüş.
- **Bağımlılık sırası ve otomatik silinmeler:**
  - Yeni tablolar arası FK'lar tüm tablolardan sonra gelmeli.
  - `OWNED BY` bağlı kolon düşmeden çözülmeli.
  - Tabloyla ya da kolonuyla birlikte silinen dizi için ikinci `DROP` yazılmamalı.
  - Görünüm zincirinde oluşturma ve silme sırası; composite içinde composite; birbirine FK'lı iki tablonun silinmesi.
  - Testler: Task 6, Task 7 insta senaryoları, Task 9 "betik uygula → yeniden karşılaştır → yalnız Aynı".
- **Betik ve iş güvenliği:**
  - Veritabanı ya da şema adında `\n`, `\r`, U+2028 varsa başlık yorumu satırdan taşmamalı (yorumdan SQL'e kaçış).
  - İptal ve bağlantı kopması kısmi sonuç bırakmamalı, `idle in transaction` oturum kalmamalı.
  - Sayfa yenilemede kayıt temizlenmeli; kayıt defterinden düşmüş `diffId`'ye detay/betik isteği `diff_not_found` almalı.
  - Testler: Task 7 `comment_safe`, Task 8 kayıt testleri, Task 9 iptal ve günlük yakalama.

---

### Task 1: `pgt-ddl` yükleyicilerini istemci alan iç fonksiyonlara ayır

**Dosyalar:**
- Değiştir: `crates/pgt-ddl/src/load.rs`, `crates/pgt-ddl/src/load_acl.rs`, `crates/pgt-ddl/src/lib.rs` (yeniden dışa aktarma).
- Oluştur: `crates/pgt-ddl/tests/load_in.rs` (Docker).

**Arayüzler — Üretir:**

```rust
// load.rs
pub const SERVER_SCOPED_MSG: &str = "Veritabanı ve rol bu yolla yüklenmez (bakım bağlantısı gerekir).";
/// Şema kapsamlı türleri (Schema, Sequence, Extension, Index, Table, View, MaterializedView, Function, Procedure,
/// Trigger, Type, Domain) verilen istemcide yükler. Sorgular, sıra ve normalleştirme `load` ile aynıdır.
/// Database/Role → `Error::Invalid(SERVER_SCOPED_MSG)`. Sorgu hatası `Error::from_query` ile eşlenir.
pub async fn load_in(client: &tokio_postgres::Client, version_num: u32, obj: ObjectRef) -> pgt_core::Result<ObjectDef>;
// load_acl.rs
/// `load_acl` ile aynı sonuç; Database → `Error::Invalid(SERVER_SCOPED_MSG)`; yetkisiz tür → `Invalid(UNSUPPORTED_KIND_MSG)`.
pub async fn load_acl_in(client: &tokio_postgres::Client, version_num: u32, obj: ObjectRef) -> pgt_core::Result<AclDef>;
// lib.rs
pub use load::{load_in, SERVER_SCOPED_MSG, …};  pub use load_acl::{load_acl_in, …};
```
- **İç yapı:**
  - Her şema kapsamlı tür için `async fn fetch_<tür>(c: &Client, v: PgVersion, oid: u32) -> Result<<Tür>Rows, tokio_postgres::Error>` yazılır (bugünkü `run` kapanışının gövdesi birebir). Yanında `fn parse_<tür>(rows: <Tür>Rows) -> pgt_core::Result<ObjectDef>` olur (bugünkü kapanış sonrası kod birebir).
  - `<Tür>Rows` küçük yerel yapılar ya da demetlerdir (ör. `(Vec<Row>, Vec<Row>)`).
  - `load_<tür>(mgr, …)` = `mgr.run(server, Target::Database(db), move |c| async move { fetch_<tür>(&c.client, PgVersion(c.version_num()), oid).await }).await?` + `parse_<tür>`.
  - `load_in` = `fetch_<tür>(client, PgVersion(version_num), oid).await.map_err(Error::from_query)?` + `parse_<tür>`.
  - Function/Procedure'da `kind` parse'a geçer. `load_acl` için de aynı ikili (`fetch_acl`, `parse_acl`); `version_num` parse'a parametre olarak girer (`available`).
  - Database/Role yükleyicilerine dokunulmaz.
- **Kapsam denetimi saf olur:** `fn in_scope(kind: ObjectKind) -> pgt_core::Result<()>` (sunucu kapsamlı → `Invalid(SERVER_SCOPED_MSG)`). `load_in` önce bunu çağırır.

- [ ] **Adım 1: Başarısız testler.**
  - Birim (`load.rs` testleri):
    - `in_scope(Database)` ve `in_scope(Role)` → `Invalid(SERVER_SCOPED_MSG)`;
    - diğer 12 tür → `Ok`.
  - Docker (`tests/load_in.rs`, her sürüm; Procedure PG 11+, multirange adımı PG 14+). Fikstür veritabanı `pgt_ddl_li_<ana>_all`, şema `s`:
    - `CREATE TYPE s.mood AS ENUM ('a','b')`, `CREATE TYPE s.pair AS (x int, y s.mood)`, `CREATE TYPE s.r AS RANGE (subtype = int4)`, `CREATE DOMAIN s.pos AS int CHECK (VALUE > 0)`;
    - `CREATE TABLE s.p (id serial PRIMARY KEY, m s.mood DEFAULT 'a', d s.pos)`, `CREATE TABLE s.c (id int PRIMARY KEY, p_id int REFERENCES s.p(id), u text UNIQUE, CHECK (id > 0))`, `CREATE INDEX c_u ON s.c (lower(u))`;
    - `CREATE FUNCTION s.f(a s.mood) RETURNS int LANGUAGE sql AS 'SELECT 1'`, trigger fonksiyonu ve `s.c` üzerinde trigger;
    - `CREATE VIEW s.v AS SELECT * FROM s.c`, `CREATE MATERIALIZED VIEW s.mv AS SELECT id FROM s.p`, `CREATE PROCEDURE s.pr() LANGUAGE sql AS ''` (PG 11+);
    - `GRANT SELECT ON s.c TO PUBLIC`.
    - Her nesne için `load(&mgr, server, db, obj)` == `load_in(&dedicated.client, v, obj)`; yetkili türlerde `load_acl` == `load_acl_in`. Ayrılmış bağlantı `mgr.open_dedicated(server, Target::Database(db), SessionProfile::Catalog)` ile açılır.
    - Sunucu kapsamlı tür → `Invalid`.
  - Docker — spec §2.1 öncülü (aynı fikstürde): ayrılmış bağlantıda `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` ve `SELECT pg_catalog.set_config('search_path', 'pg_catalog, s', true)` sonrası `load_in` ile:
    - `s.v`'nin `query`'si `FROM c` içerir, `s.c` içermez;
    - `s.p.m` kolon tipi `mood`, varsayılanı `'a'::mood`;
    - `s.p.id` varsayılanı `nextval('p_id_seq'::regclass)`;
    - `s.f` argüman tipi `mood`;
    - `s.pair` özniteliği `mood`.
    - Aynı nesneler `load` (havuz, `search_path = pg_catalog`) ile `s.` nitelemeli gelir.
    - Ardından havuzdaki bağlantıda `SHOW search_path` → `pg_catalog` (sızıntı yok).
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `cargo test -p pgt-ddl in_scope` → derleme hatası (`in_scope` yok).
- [ ] **Adım 3: Uygula.**
  - Bölme mekaniktir. Sorgu metinleri, parametre sırası, koşullu ikinci sorgular (başlık boşsa kolon sorgusu yok), `note_invalid_utf8`/`refuse_*` çağrı yerleri ve hata metinleri değişmez.
  - `load` belge yorumuna iki giriş noktası eklenir.
- [ ] **Adım 4: Doğrula.**
  - `cargo test -p pgt-ddl` PASS (birim + insta).
  - `cargo clippy --workspace --all-targets -- -D warnings`.
  - Matris: `powershell -File scripts/test-matrix.ps1 -Versions 10,11,12,13,14,15,16,17,18 -SkipPerf`. Mevcut tüm `pgt-ddl` Docker testleri (roundtrip, table/view/routine/type/acl) ve yeni `load_in` testleri PASS.
- [ ] **Adım 5: Commit.**
  ```bash
  git add crates/pgt-ddl/src/load.rs crates/pgt-ddl/src/load_acl.rs crates/pgt-ddl/src/lib.rs crates/pgt-ddl/tests/load_in.rs
  git commit -m "refactor(ddl): yükleyicileri istemci alan iç fonksiyonlara ayır (load_in, load_acl_in)"
  ```

### Task 2: `pgt-diff` iskeleti, IPC türleri, anahtar ve eşleştirme

**Dosyalar:**
- Oluştur: `crates/pgt-diff/{Cargo.toml, src/lib.rs, src/model.rs, src/key.rs}`.
- Değiştir: kök `Cargo.toml` (`members` + `pgt-diff = { path = "crates/pgt-diff" }`).
- Bindingler `src/ipc/bindings/` altına `cargo test -p pgt-diff` ile üretilir.

**Bağımlılıklar:** `pgt-core`, `pgt-catalog` (`VersionedSql`), `pgt-ddl`, `tokio`, `tokio-postgres`, `futures-util`, `serde`, `serde_json`, `ts-rs`, `tracing`, `uuid`. Dev: `pgt-core` (`testing`), `tokio` (`test-util`), `insta`, `tempfile`, `tracing-subscriber` (pgt-ddl'deki satırla aynı).

**Arayüzler — Üretir:**

```rust
// model.rs — hepsi Serialize/Deserialize(gereken)/TS, #[ts(export)], struct'lar camelCase, enum'lar snake_case
pub struct DiffSide { pub server_id: Uuid, #[ts(type = "number")] pub database_oid: u32, pub schema: String }
pub struct DiffOptions { pub ownership: bool, pub tablespace: bool, pub comments: bool }   // Default: false, false, true (elle impl)
pub enum SideTag { Source, Target }
#[serde(tag = "event", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DiffEvent { Progress { side: SideTag, loaded: u32, total: u32 } }
pub enum ItemStatus { OnlySource, OnlyTarget, Different, Same }
pub enum ItemProblem { Manual, LoadFailed, BuildFailed }
pub struct DiffItem {
    pub key: String, pub kind: ObjectKind, pub target_kind: Option<ObjectKind>, pub name: String,
    pub table: Option<String>, pub signature: Option<String>, pub status: ItemStatus, pub problem: Option<ItemProblem>,
    pub risk: Risk, pub notes: Vec<String>, pub errors: Vec<String>, pub selectable: bool, pub default_selected: bool,
}
pub struct DiffResult { pub items: Vec<DiffItem>, pub source_version: String, pub target_version: String, pub warnings: Vec<String>, #[ts(type = "number")] pub elapsed_ms: u64 }
pub enum LineKind { Same, Added, Removed }
pub struct DiffLine { pub kind: LineKind, pub text: String, pub source_no: Option<u32>, pub target_no: Option<u32> }
pub struct DiffDetail { pub source_text: String, pub target_text: String, pub lines: Option<Vec<DiffLine>> }
pub struct DiffScript { pub sql: String, pub transactional: bool, pub warnings: Vec<String>, pub risk: Risk, pub statements: u32 }
pub struct DiffDatabase { #[ts(type = "number")] pub oid: u32, pub name: String }
// key.rs
pub enum KeyClass { Relation, Type, Routine, Index, Trigger }               // sıralama da bu sırayla
pub fn class_of(kind: ObjectKind) -> Option<KeyClass>;                       // Table|View|MaterializedView|Sequence → Relation; Type|Domain → Type; Function|Procedure → Routine; Index; Trigger; diğerleri None
pub struct Listed { pub kind: ObjectKind, pub oid: u32, pub name: String, pub table: Option<String>, pub signature: Option<String>, pub is_partition: bool, pub deps: Vec<u32> }
pub fn item_key(l: &Listed) -> String;                                       // serde_json::to_string(&(sınıf harfi, tablo ?? "", ad, imza ?? "")); "r","t","f","i","g"
pub struct Pair<'a> { pub key: String, pub source: Option<&'a Listed>, pub target: Option<&'a Listed> }
pub fn match_objects<'a>(source: &'a [Listed], target: &'a [Listed]) -> Vec<Pair<'a>>;   // sıra: sınıf, tablo, ad, imza (bayt sırası)
```
- **Anlamlar:**
  - `Same`'de risk `Safe`'tir; `selectable = false`, `default_selected = false`.
  - `problem` doluysa `selectable = false`.
  - `default_selected = selectable && status ∈ {OnlySource, Different}` (spec §2.4).
- `DiffEvent::Progress.loaded ≤ total`. Son olay her tarafta `loaded == total`'dır.
- Anahtar opaktır. Ön yüz ayrıştırmaz; yalnız eşitlik ve geri gönderme için kullanır.

- [ ] **Adım 1: Başarısız testler (`key.rs`, `model.rs`).**
  - Kaynakta tablo `x`, hedefte görünüm `x` → tek `Pair` (iki taraf dolu).
  - `f(integer)` ve `f(text)` → iki ayrı çift.
  - `a` tablosunda indeks `i` ve `b` tablosunda indeks `i` → iki anahtar.
  - Tablo `x` ile tip `x` → iki ayrı çift (farklı sınıf).
  - Ad çakışma güvenliği: `("a\u{1f}b", tablo yok)` ile `("a", tablo "b")`, `"` ve `,` içeren adlar farklı anahtar verir.
  - Çiftler deterministik sırada gelir: tablolar, tipler, rutinler, indeksler, trigger'lar; içeride ad bayt sırası.
  - `DiffOptions::default()` == `{ownership: false, tablespace: false, comments: true}`.
  - Serileştirme: `DiffEvent::Progress` → `{"event":"progress","side":"source","loaded":1,"total":2}`; `ItemStatus::OnlySource` → `"only_source"`; `DiffItem` alanları camelCase (`targetKind`, `defaultSelected`).
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `cargo test -p pgt-diff` → derleme hatası.
- [ ] **Adım 3: Uygula.** `lib.rs` modülleri ve dışa aktarmaları tanımlar. Crate belge yorumu Mimari'yi özetler.
- [ ] **Adım 4: Doğrula.**
  - `cargo test -p pgt-diff`; bindingler üretilir: `DiffSide`, `DiffOptions`, `SideTag`, `DiffEvent`, `ItemStatus`, `ItemProblem`, `DiffItem`, `DiffResult`, `LineKind`, `DiffLine`, `DiffDetail`, `DiffScript`, `DiffDatabase`.
  - `ObjectKind` ve `Risk` mevcut bindinglerden içe aktarılır (yeni kopya yok).
  - `git status src/ipc/bindings` → yalnız yeni dosyalar; gürültü varsa `git checkout -- src/ipc/bindings`.
  - Clippy, `bun run typecheck`.
- [ ] **Adım 5: Commit.**
  ```bash
  git add Cargo.toml Cargo.lock crates/pgt-diff src/ipc/bindings
  git commit -m "feat(diff): pgt-diff crate'i, IPC türleri ve nesne eşleştirme"
  ```

### Task 3: Myers satır farkı ve DDL metni

**Dosyalar:** Oluştur `crates/pgt-diff/src/linediff.rs`, `crates/pgt-diff/src/script.rs` (yalnız `comment_safe`), `src/snapshots/`.

**Arayüzler — Üretir:**

```rust
pub const MAX_DIFF_LINES: usize = 20_000;
/// target → source dönüşümü: `Removed` yalnız hedefte (`target_no`), `Added` yalnız kaynakta (`source_no`), `Same` ikisinde.
/// Satırlar `\n` ile bölünür, satır sonundaki `\r` atılır, metin sonundaki tek `\n` boş satır üretmez.
/// İki metnin satır toplamı `MAX_DIFF_LINES`'ı aşarsa `None`.
pub fn line_diff(source: &str, target: &str) -> Option<Vec<DiffLine>>;
/// Ayrıntı metni: `build::create(def, ctx)` ifadeleri `";\n\n"` ile birleşir ve sonda `";"` olur.
/// Üretici reddederse `"-- DDL üretilemedi:"` satırı ve her ileti için `"-- <ileti>"` satırı (`comment_safe`, Task 7).
pub fn ddl_text(def: &ObjectDef, ctx: &BuildCtx) -> String;
```
- **Algoritma:** önce ortak ön ve son satırlar kırpılır. Ardından doğrusal bellekli Myers kullanılır (orta yılan + böl ve yönet). İz (trace) dizisi tutulmaz; bellek O(N+M) kalır.
- Satır karşılaştırması bayt eşitliğidir; çıktı minimaldir (düzenleme sayısı N + M − 2·LCS).
- `comment_safe` bu görevde `crates/pgt-diff/src/script.rs`'te tanımlanır (dosya bu görevde yalnız bu işlevi içerir). Task 7 aynı dosyayı genişletir ve bu işlevi değiştirmeden kullanır:

  ```rust
  pub fn comment_safe(s: &str) -> String;   // denetim karakterleri (\n, \r, \t dahil) ve U+2028/U+2029 → '?'
  ```

- [ ] **Adım 1: Başarısız testler.**
  - `line_diff("", "")` → `Some([])`.
  - `line_diff("a\n", "")` → `[Added "a" (source_no 1)]`.
  - Aynı üç satır → üç `Same`, numaralar 1–3.
  - Klasik örnek: kaynak `C B A B A C`, hedef `A B C A B B A` (her harf bir satır) → düzenleme sayısı `7 + 6 − 2·4 = 5`.
  - Özellik testi (yeni bağımlılık yok; LCG tohum 42): 300 rastgele çift, 4 harfli alfabe, uzunluk 0–40:
    - `Added + Removed` sayısı DP ile hesaplanan `N + M − 2·LCS`'ye eşittir;
    - `Same + Removed` satırları sırayla hedefi, `Same + Added` satırları sırayla kaynağı verir;
    - numaralar artan ve boşluksuzdur.
  - CRLF ve LF aynı metin → tümü `Same`.
  - 10.001 + 10.000 satır → `None`.
  - Tamamen farklı 10.000 + 10.000 satır → `Some`, süre < 10 sn (debug derlemede).
  - `ddl_text` (insta):
    - FK'lı tablo (FK satır içi);
    - enum tipi;
    - kayıplı UTF-8 gerekçeli tablo (`build::create` reddeder) → "-- DDL üretilemedi:" satırları ve SQL satırı yok.
  - `comment_safe("a\nb\r\u{2028}c\td")` == `"a?b??c?d"`.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `cargo test -p pgt-diff linediff` → FAIL.
- [ ] **Adım 3: Uygula.**
- [ ] **Adım 4: Doğrula.** `cargo test -p pgt-diff`, clippy.
- [ ] **Adım 5: Commit.**
  ```bash
  git add crates/pgt-diff/src
  git commit -m "feat(diff): doğrusal bellekli Myers satır farkı ve DDL metni"
  ```

### Task 4: Liste sorgusu ve taraf yükleyicisi

**Dosyalar:**
- Oluştur: `crates/pgt-diff/src/sql/{list_objects.pg10.sql, list_objects.pg11.sql, list_objects.sql, schema_check.sql}`, `src/list.rs`, `src/snapshot.rs`, `src/cancel.rs`.
- Oluştur: `crates/pgt-diff/tests/common/mod.rs` (`servers`, `guarded`, veritabanı adları; `crates/pgt-ddl/tests/common/mod.rs`'teki desenin kopyası, önek `pgt_diff`) ve `crates/pgt-diff/tests/list_snapshot.rs` (Docker).

**Arayüzler:**
- Tüketir: Task 1 `load_in`/`load_acl_in`; Task 2 `Listed`, `DiffSide`, `SideTag`.
- Üretir:

```rust
// list.rs
pub enum SchemaCheck { Ok, Missing, System }
pub async fn schema_check(c: &Client, schema: &str) -> Result<SchemaCheck, tokio_postgres::Error>;   // pg_namespace; pg_* / information_schema → System
pub async fn list_objects(c: &Client, version_num: u32, schema: &str) -> Result<Vec<Listed>, tokio_postgres::Error>;
// cancel.rs — pgt-backup CancelToken'ın kopyası (new, cancel, is_cancelled, cancelled().await); pgt-backup'a bağımlılık yok
pub struct CancelToken;
// snapshot.rs
pub const LOAD_PIPELINE: usize = 8;          // aynı istemcide eşzamanlı bekleyen yükleme sayısı (sıra korunur)
pub const PROGRESS_EVERY: u32 = 25;
pub struct Loaded { pub listed: Listed, pub def: Result<ObjectDef, String>, pub acl: Option<Result<AclDef, String>> }
pub struct SideSnapshot { pub version_num: u32, pub version: String, pub database: String, pub viewer: String, pub objects: Vec<Loaded> }
pub enum SnapshotError { Cancelled, Schema(SchemaCheck), Core(pgt_core::Error) }
/// `report(yüklenen, toplam)` false dönerse iş iptal sayılır.
pub async fn load_side(mgr: &ConnectionManager, side: &DiffSide, with_acl: bool,
                       report: &(dyn Fn(u32, u32) -> bool + Sync), cancel: &CancelToken) -> Result<SideSnapshot, SnapshotError>;
pub fn classify_load_error(e: &pgt_core::Error) -> LoadErrorClass;   // enum LoadErrorClass { Item, Fatal }: Invalid/NotFound → Item; Query/Connect/ConnectionLost/NotConnected/diğer → Fatal
```
- **Liste sorgusu** (`$1 = şema adı`; kolonlar `kind text, oid oid, name text, table_name text, signature text, is_partition bool, deps oid[]`; sıra `kind, table_name, name COLLATE "C"`):
  - **İlişkiler:**
    - `relkind` `r`/`p` → `table`, `v` → `view`, `m` → `materialized_view`, `S` → `sequence`.
    - Kimlik dizileri (`pg_depend deptype 'i'`) hariç.
    - `is_partition = relispartition`.
    - Görünüm/MV için `deps` = `pg_rewrite` (`ev_class = c.oid`) → `pg_depend` (`classid = 'pg_catalog.pg_rewrite'::pg_catalog.regclass`, `refclassid = pg_class`, `refobjid <> c.oid`) farklı `refobjid`'ler. Diğerlerinde `'{}'`.
  - **İndeksler:**
    - Tablo `relkind` `r`/`p`/`m` olmalı; `table_name` = tablo adı.
    - Kısıt indeksi hariç: `pg_constraint.conindid = indexrelid AND contype IN ('p','u','x')`.
    - PG 11+'da bölüm klonu hariç (`pg_inherits.inhrelid = indexrelid`).
  - **Trigger'lar:**
    - `NOT tgisinternal`; PG 13+'da `tgparentid = 0`.
    - Tablo `relkind` `r`/`p`; `table_name` = tablo adı.
  - **Rutinler:**
    - PG 10: `NOT proisagg` → `function`.
    - PG 11+: `prokind` `f`/`w` → `function`, `p` → `procedure`.
    - `signature = pg_catalog.oidvectortypes(proargtypes)`.
  - **Tipler:**
    - `typtype` `e`/`r` ve `c` yalnız `typrelid`'in `relkind`'i `c` ise → `type`; `d` → `domain`.
    - Diziler, multirange ve satır tipleri `typtype` süzgeciyle düşer.
  - **Uzantı üyeleri:** `pg_depend deptype 'e'` (`classid` sırasıyla `pg_class`/`pg_proc`/`pg_type`) hariç. Uzantı üyesi tabloların indeks ve trigger'ları da hariç.
  - Bütün katalog adları `pg_catalog.` nitelemelidir.
- **`load_side` akışı:**
  1. `open_dedicated(side.server_id, Target::Database(side.database_oid), SessionProfile::Catalog)`.
  2. `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY`.
  3. `SELECT pg_catalog.set_config('search_path', $1, true)` (Kararlar 2).
  4. `schema_check` (Ok değilse `Schema(..)`).
  5. `SELECT current_database()::text, current_user::text, current_setting('server_version_num')::int4`. Sürüm metni `DedicatedConn.info.version`'dan alınır.
  6. `list_objects`.
  7. `stream::iter(nesneler).map(|l| load_in(..)).buffered(LOAD_PIPELINE)` (yetki seçeneğinde `acl_supported` türler için ayrıca `load_acl_in`). Her `PROGRESS_EVERY` nesnede ve sonda `report` çağrılır.
  8. `COMMIT`.
  9. Bağlantı düşer.
- **Hata sınıfları:** `Item` hatası `Loaded.def = Err(ErrorPayload::from(&e).message)` olur. `Fatal` hata o tarafı bitirir (`Core`).
- **İptal:** `tokio::select!` ile `cancel.cancelled()` dinlenir. İptalde `DedicatedConn.cancel.cancel()` en çok 2 sn denenir, ardından gelecek düşürülür (istemci kapanır, sunucu geri alır) ve `Cancelled` döner. `report` false da aynıdır.

- [ ] **Adım 1: Başarısız testler.**
  - Birim:
    - `classify_load_error` tablosu: `Invalid`, `NotFound` → Item; `Query`, `ConnectionLost`, `NotConnected` → Fatal.
    - `CancelToken`: iptal kalıcıdır; sonradan bekleyen hemen döner.
  - Docker (`tests/list_snapshot.rs`, her sürüm; sürüme bağlı adımlar atlanır). Fikstür şema `s`:
    - `CREATE EXTENSION IF NOT EXISTS citext SCHEMA s`: `citext` tipi ve işlevleri listede **yok**.
    - `id int GENERATED ALWAYS AS IDENTITY` kolonlu tablo: kimlik dizisi yok. `serial` dizisi var.
    - Tablo satır tipi ve dizi tipleri yok; composite tip, enum, range ve domain var. PG 14+'da multirange yok.
    - PK/UNIQUE indeksleri yok; düz indeks var (`table_name` dolu).
    - Bölümlü tablo `s.pt` + bölüm `s.pt_1`: üst `is_partition = false`, bölüm `true`.
    - PG 11+: `s.pt` üzerindeki indeks var, bölümdeki klonu yok.
    - PG 13+: `s.pt` üzerindeki satır trigger'ının bölüm klonu yok, üst trigger var.
    - FK'nın iç trigger'ları yok.
    - Aggregate (`CREATE AGGREGATE s.agg(int) (sfunc = int4pl, stype = int)`) yok.
    - Procedure (PG 11+) var.
    - Aşırı yüklemeler `f(integer)` / `f(text)` → `signature` `"integer"` ve `"text"`. Argüman adı farklı iki tanım aynı imzayı verir.
    - `s.v2 AS SELECT * FROM s.v1` → `v2.deps` `v1`'in oid'ini içerir.
    - `schema_check`: yok → Missing; `pg_catalog` ve `information_schema` → System; `s` → Ok.
  - Docker (`load_side`):
    - Görünüm sorgusu niteliksiz.
    - `report` son çağrısı `(n, n)`.
    - `database`, `viewer` ve sürüm dolu.
    - Başlamadan iptal edilmiş belirteç → `Cancelled` (< 2 sn).
    - `report` ilk çağrıda false → `Cancelled`.
    - Dönüşten sonra `pg_stat_activity`'de bu veritabanı için `state = 'idle in transaction'` oturum yok.
    - `mgr.run` havuz sorgusu iş sürerken ve sonrasında çalışır; havuzda `SHOW search_path` → `pg_catalog`.
    - Item hatası yolu: iç işlev `load_objects`'e `pg_catalog.pg_class`'ın oid'iyle elle kurulmuş bir `Listed { kind: Table, … }` ve fikstürden iki nesne verilir → ilki `Loaded.def = Err(ileti)` (sistem nesnesi), diğer ikisi `Ok`; iş sürer ve transaction bozulmaz.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `cargo test -p pgt-diff classify` → FAIL.
- [ ] **Adım 3: Uygula.** İç işlev `load_objects(client, version, objects, with_acl, report, cancel)` testte doğrudan çağrılabilir olsun (`#[doc(hidden)] pub`).
- [ ] **Adım 4: Doğrula.** `cargo test -p pgt-diff`, clippy, matris `-Versions 10,11,13,14,16,18 -SkipPerf` (sürüm dalları: 10, 11, 13 SQL'i ve 14 multirange).
- [ ] **Adım 5: Commit.**
  ```bash
  git add crates/pgt-diff
  git commit -m "feat(diff): liste sorgusu ve tutarlı anlık görüntüyle taraf yükleyicisi"
  ```

### Task 5: Normalizasyon, kimlik eşlemesi ve durum kararı (saf)

**Dosyalar:** Oluştur `crates/pgt-diff/src/normalize.rs` (+ insta anlık görüntüleri).

**Arayüzler:**
- Tüketir: Task 2 türleri, Task 4 `Loaded`.
- Üretir:

```rust
pub struct NormCtx<'a> { pub source_schema: &'a str, pub target_schema: &'a str, pub options: DiffOptions }
pub fn normalize(def: &ObjectDef, side: SideTag, cx: &NormCtx) -> ObjectDef;
pub fn normalize_acl(acl: &AclDef, side: SideTag, cx: &NormCtx) -> AclDef;          // schema → hedef
pub fn align_to_target(source: &ObjectDef, target: &ObjectDef) -> (ObjectDef, Vec<String>);   // aynı tür; notlar
pub fn unsupported_for_compare(def: &ObjectDef, options: &DiffOptions) -> Vec<String>;
pub fn mentions_schema(def: &ObjectDef, schema: &str) -> bool;
pub fn acl_equal(source: &AclDef, target: &AclDef) -> bool;
pub struct Prepared {
    pub key: String, pub status: ItemStatus, pub problem: Option<ItemProblem>, pub notes: Vec<String>, pub errors: Vec<String>,
    pub kind_changed: bool,
    pub source: Option<ObjectDef>,   // normalize + align edilmiş istenen durum
    pub target: Option<ObjectDef>,   // normalize edilmiş mevcut durum
    pub source_acl: Option<AclDef>, pub target_acl: Option<AclDef>,
}
pub fn prepare(key: String, source: Option<&Loaded>, target: Option<&Loaded>, cx: &NormCtx) -> Prepared;
```
- **`normalize` kuralları (spec §2.3):**
  - **Şema** (yalnız kaynakta, kaynak şemaya eşitse hedefe):
    - `Table.schema` ve FK `ref_schema`;
    - `Index.schema`;
    - `Trigger.schema` ve `function_schema`;
    - `Sequence.schema` ve `owned_by.schema`;
    - `View`/`MatView`/`Function`/`Type`/`Domain` `schema`.
    - Başka şemaya başvuran FK/trigger fonksiyonu olduğu gibi kalır.
    - Serbest SQL'e dokunulmaz.
  - **Sahip:** `options.ownership` kapalıysa sahibi olan her türde `None`.
  - **Tablespace:** kapalıysa `Table`/`Index`/`MatView` için `None`.
  - **Yorum:** kapalıysa tablo, kolon, indeks, görünüm, MV, sequence, rutin, trigger, tip ve domain yorumları `None`.
  - **Yalnız yüklemede dolanlar:**
    - `View.columns = []`, `MatView.columns = []`;
    - `Index.valid = true`;
    - `ColumnDef.using = None`;
    - `View.recreate = false`, `Function.recreate = false`, `MatView.refresh = None`.
    - `Table.partition_text` **korunur** (Kararlar 5).
- **`unsupported_for_compare`:** `INDEX ` önekliler (MV) her zaman, `COMMENT ` önekliler (görünüm/MV) yorum seçeneği kapalıyken atılır.
- **`align_to_target` (Kararlar 9):**
  - **Tablo:**
    - kolon `attnum` := hedefte aynı adlı kolonunki, yoksa `None`;
    - kolonlar hedef sırasına dizilir, yeniler kaynak sırasıyla sona eklenir;
    - kaynağın kendi sırası farklıysa not: "Kolon sırası farklı (ALTER ile değiştirilemez; yeni kolonlar sona eklenir).";
    - kısıt `oid` := hedefte aynı adlı kısıtınki, yoksa `None`.
  - **Enum:** değer `original` := hedefte aynı etiket varsa `Some(etiket)`, yoksa `None`.
  - **Composite:** öznitelik için aynı kural.
  - **Domain:** kısıt `original` := hedefte aynı adlı kısıt varsa `Some(ad)`.
  - **MatView:** `with_data` := hedeften.
- **`mentions_schema`:**
  - Taranan alanlar: görünüm/MV sorgusu, rutin gövdesi ve argüman varsayılanları, kolon varsayılanı ve üretilmiş ifadesi, CHECK ifadesi, indeks ifadesi ve `WHERE`'i, trigger `WHEN`'i, domain varsayılanı ve CHECK'i.
  - Aranan biçimler: `<şema>.` ya da `quote_ident(şema).`.
  - Eşleşmenin önündeki karakter tanımlayıcı karakteri (harf, rakam, `_`, `$`) ya da `"` olmamalıdır.
- **`acl_equal`:** hedef satırlarında hedef sahibi kaynak sahibine çevrilir; aynı alıcı varsa yetkiler birleşir ve `grant_option` OR'lanır. Ardından `sort_grants(_, privileges_for(kind, 170000))` ve `grants` eşitliği bakılır. `foreign`/`columns` karşılaştırılmaz; farklıysa `prepare` not ekler: "Kolon düzeyi ve başka verenlerin yetkileri karşılaştırılmaz.".
- **`prepare` karar sırası:**
  1. Bir tarafın `def`'i ya da (seçenek açıkken) `acl`'si `Err` → `LoadFailed`; `errors` iletileri içerir. Durum varlığa göre `OnlySource`/`OnlyTarget`/`Different`.
  2. Yalnız kaynak → `OnlySource`. Kaynak "elle" koşulları varsa `Manual` ve gerekçeleri not olur:
     - `unsupported_for_compare` dolu → "Modelin dışında özellikler var: a, b";
     - tablo `is_partition` → "Bölüm alt tablosu (ATTACH modellenmez)";
     - tablo `inherits` → "Kalıtım çocuğu (INHERITS modellenmez)";
     - `partition_text` dolu → "Bölümlenmiş tablo oluşturma modellenmez".
  3. Yalnız hedef → `OnlyTarget` (sorunsuz).
  4. İki taraf, farklı tür ya da farklı tip gövdesi varyantı → `Different` + `kind_changed`. Kaynak tarafı (2)'deki "elle" koşullarına tabidir.
  5. İki taraf, aynı tür → `align_to_target`:
     - modeller eşit, `unsupported` eşit, ACL eşit (ya da seçenek kapalı) → `Same` (+ sıra notu);
     - modeller eşit ama `unsupported` farklı → `Different` + `Manual`, not: "Modelin dışında fark var: …";
     - aksi hâlde tablo için iki taraftan birinde `is_partition`/`inherits` → `Manual`; `partition_text` farklı → `Manual`, not: "Bölümleme anahtarı farklı";
     - kalan durumlar `Different`.
  6. Şema adları farklı ve `mentions_schema(kaynak, kaynak_şema)` ise not: "Gövdede kaynak şema adı açıkça geçiyor; gözden geçirin.".

- [ ] **Adım 1: Başarısız testler (kodla kurulan fikstürler; karmaşık çıktılar insta).**
  - Şema çevirisi: tüm yapısal alanlar, başka şemaya FK değişmez, `src.t` içeren görünüm sorgusu değişmez.
  - `mentions_schema` örnekleri:
    - `src.t` → true;
    - `"src".t` → true;
    - `srcx.t` → false;
    - `a.src` → false;
    - `"My Src".t` → `quote_ident` biçimiyle true;
    - argüman varsayılanında ve CHECK'te → true.
  - Seçenek matrisi:
    - sahiplik kapalı/açık;
    - tablespace;
    - yorum kapalıyken `COMMENT c` girdisi atılır.
  - `align_to_target`:
    - attnum ve kısıt oid eşlemesi;
    - yeni kolon `None`;
    - sıra notu;
    - enum'a ortaya değer → `original` dizisi `[Some(a), None, Some(b)]`;
    - composite ve domain eşlemesi.
  - Durum tablosu (her satır `(status, problem)`):
    - yalnız kaynak / yalnız hedef / aynı / farklı;
    - tablo ↔ görünüm tür değişimi;
    - enum ↔ composite;
    - yükleme hatası;
    - bölüm çocuğu, kalıtım çocuğu, bölümlü üst tablo (yalnız kaynakta → Manual);
    - `partition_text` farkı;
    - MV yalnız `with_data` farkı → Same;
    - MV yalnız `INDEX i` farkı → Same;
    - görünüm `TRIGGER t` farkı → Different + Manual;
    - yalnız kolon sırası farkı → Same + not;
    - sahiplik açıkken yalnız GRANT farkı → Different.
  - `acl_equal`: farklı sahip + yeniden adlandırılmış sahip satırı eşit; PUBLIC farkı eşit değil.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `cargo test -p pgt-diff normalize` → FAIL.
- [ ] **Adım 3: Uygula.**
- [ ] **Adım 4: Doğrula.** `cargo test -p pgt-diff`, clippy.
- [ ] **Adım 5: Commit.**
  ```bash
  git add crates/pgt-diff/src
  git commit -m "feat(diff): normalizasyon, kimlik eşlemesi ve durum kararı"
  ```

### Task 6: Öğe üretimi (aşamalı ifadeler)

**Dosyalar:** Oluştur `crates/pgt-diff/src/generate.rs` (+ insta).

**Arayüzler:**
- Tüketir: Task 5 `Prepared`; `pgt_ddl::build::{create, alter, drop_with, DropOptions, routine::needs_recreate, acl::privileges}`, `pgt_ddl::{privileges_for, sort_grants, acl_supported}`.
- Üretir:

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Phase {
    DropTrigger, DropIndex, DropView, DropRoutine, DropTable, DropSequence, DropDomain, DropType,   // spec §2.6 adım 3
    Types, Sequences, Routines, Tables, OwnedBy, ForeignKeys, Views, Indexes, Triggers, Privileges, // adım 4.1–4.10
}
pub struct Generated {
    pub parts: Vec<(Phase, Vec<Statement>)>, pub warnings: Vec<String>, pub risk: Risk, pub notes: Vec<String>,
    pub drops_table: bool,                          // öğe hedefteki tabloyu siler
    pub dropped_columns: Vec<String>,               // tablo değişikliğinin düşürdüğü hedef kolonları
    pub sequence_owner: Option<(String, String)>,   // yalnız hedefte dizi: OWNED BY (tablo, kolon)
}
pub struct GenFailure { pub problem: ItemProblem, pub messages: Vec<String> }   // BuildFailed (FieldError iletileri) ya da Manual
pub fn generate(p: &Prepared, ctx: &BuildCtx, target_viewer: &str) -> Result<Generated, GenFailure>;   // Same → boş Generated
pub fn default_grants(kind: ObjectKind, owner: &str, version_num: u32) -> Vec<AclGrant>;
pub fn fk_add_plan(base: &TableDef, fks: &[ConstraintDef], ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>>;
```
- **Yalnız kaynakta** (`create`):
  - **Tablo:** `D_noFK` (FK kısıtları çıkarılmış) `create` → `Tables`; FK'lar `fk_add_plan(D_noFK, fk'lar)` → `ForeignKeys`.
  - **Sequence:** `owned_by = None` ile `create` → `Sequences`; bağ varsa `alter(bağsız, tam)` → `OwnedBy`.
  - **Diğerleri:** Type/Domain → `Types`; rutin → `Routines`; görünüm/MV → `Views`; indeks → `Indexes`; trigger → `Triggers`.
  - **Yetki** (sahiplik açık + `acl_supported` + kaynak ACL var):
    - eski = taban ACL (`kind`, `schema` = hedef, `name`, `signature` = kaynak ACL'ninki, `owner` = kaynak sahibi, `viewer` = `target_viewer`, `explicit = false`, `grants = default_grants`, `foreign`/`columns` boş, `editable = true`, `available = privileges_for(kind, hedef)`);
    - yeni = eski + kaynak `grants`;
    - `privileges(ctx, &eski, &yeni, false)` → `Privileges`.
- **Yalnız hedefte:**
  - `drop_with(hedef, DropOptions { cascade: false, force: false })` → hedef türünün `Drop*` aşaması.
  - `drops_table` ve `sequence_owner` doldurulur.
- **Tür değişti:** hedef silmesi + kaynak oluşturması (yukarıdaki iki kural). `kind_changed` notu: "Tür değişti: hedefteki nesne silinip yeniden oluşturulur".
- **Farklı** (aynı tür):
  - **Tablo** (Kararlar 10):
    - `added_fks` = `D`'deki FK'lardan `oid` `None` olanlar ile hedefteki aynı adlı kısıttan (kind, deferrable, initially_deferred, not_valid) farklı olanlar;
    - `D_noFK` = `D` − `added_fks`;
    - `alter(hedef, D_noFK)` → `Tables`;
    - `fk_add_plan(D_noFK, added_fks (oid None))` → `ForeignKeys`;
    - `dropped_columns` = hedefte olup `D`'de olmayan kolonlar.
  - **Sequence** (Kararlar 11):
    - `ara` = `D`, ancak `owned_by` farklıysa `None`, aynıysa `D.owned_by`;
    - `alter(hedef, ara)` → `Sequences`;
    - `ara ≠ D` ise `alter(ara, D)` → `OwnedBy`.
  - **Görünüm:** sorgu farklıysa `D.recreate = true`; not "Yeniden kurulacak (DROP + CREATE)"; → `Views`.
  - **Rutin:** `needs_recreate(kind, hedef, D)` doluysa `D.recreate = true`; not "Yeniden kurulacak (DROP + CREATE)"; öğe riski en az `Destructive`; → `Routines`.
  - **Diğerleri:** MV → `Views`; Type/Domain → `Types`; indeks → `Indexes` (yapısal değişimi üretici DROP + CREATE yapar); trigger → `Triggers`.
  - **Yetki:**
    - eski = hedef ACL (sahibi kaynak sahibine çevrilmiş, `acl_equal` kuralıyla);
    - yeni = eski + kaynak `grants`;
    - `privileges` → `Privileges`.
- **Hatalar:**
  - Üretici `Err(FieldError)` → `GenFailure { BuildFailed, iletiler (yalnız `message`, tekrarsız) }`.
  - Model farklı ama birleşik ifade listesi boş → `GenFailure { Manual, ["Fark üreticiyle ifade edilemiyor."] }`.
  - `redacted` ifade → `BuildFailed` ("Gizli değer içeren ifade betiğe alınamaz.").
- `risk` = ifadelerin en yükseği (yeniden kurma kuralıyla). `warnings` = üretici uyarıları (tekrarsız).
- **`fk_add_plan`:**
  - eski = `base`, kolonlara `attnum = i+1`, kısıtlara `oid = i+1` verilerek;
  - yeni = eski + `fks` (`oid = None`);
  - `table_alter::alter(eski, yeni, ctx)`.
  - Çıktıda yalnız `ADD CONSTRAINT` ifadeleri bulunduğu testle sabitlenir.

- [ ] **Adım 1: Başarısız testler (insta; ctx sürümü belirtilen).**
  - Yeni iki tablo `a` → `b` FK: `Tables`'ta FK'sız iki `CREATE TABLE`, `ForeignKeys`'te tek `ALTER TABLE … ADD CONSTRAINT`.
  - Değişen tabloya FK eklenir → `Tables`'ta FK yok, `ForeignKeys`'te var.
  - FK hedef tablosu değişir → `Tables`'ta `DROP CONSTRAINT`, `ForeignKeys`'te `ADD`.
  - Kolon yeniden adlandırma → `DROP COLUMN` + `ADD COLUMN`, risk DataLoss, `dropped_columns = ["eski"]`.
  - Kolon tipi değişimi → `ALTER COLUMN … TYPE` (Destructive).
  - Sequence `owned_by`:
    - yok → var: `OwnedBy`'da bağ;
    - var → başka kolon: `Sequences`'ta `OWNED BY NONE`, `OwnedBy`'da yeni bağ;
    - var → yok: yalnız `Sequences`'ta `OWNED BY NONE`.
  - Görünüm sorgusu değişimi → `DROP VIEW` + `CREATE VIEW`, not.
  - Fonksiyon dönüş tipi değişimi → DROP + CREATE, risk ≥ Destructive, not.
  - Yalnız gövde değişimi → `CREATE OR REPLACE`.
  - MV sorgu değişimi → DataLoss + indeks uyarısı.
  - Tür değişimi tablo → görünüm: `DropTable` + `Views`.
  - Enum'a değer ekleme → `ALTER TYPE … ADD VALUE`; ctx PG 11'de `transactional = false`.
  - Enum'dan değer silme → `BuildFailed`, iletisi üreticininki.
  - Çapraz sürüm: `UNIQUE NULLS NOT DISTINCT` kaynak tablosu, ctx 130000 → `BuildFailed` (ileti PG 15 sürüm iletisi).
  - Yalnız hedefte tablo `DropTable` (`cascade` yok); `drops_table = true`.
  - Yalnız hedefte bağlı dizi → `sequence_owner = Some(("t","id"))`.
  - `default_grants`:
    - Table → yalnız sahip, `privileges_for(Table, v)`;
    - Function → sahip + PUBLIC EXECUTE;
    - Type/Domain → sahip + PUBLIC USAGE;
    - PG 17'de MAINTAIN dahil.
  - Yetki:
    - yeni tabloda `GRANT SELECT … TO PUBLIC` → `Privileges`'te tek GRANT;
    - farklı tabloda REVOKE + GRANT;
    - sahip farkı → nesne planında `OWNER TO`.
  - Boş birleşik plan → `Manual`.
  - `fk_add_plan` → yalnız `ADD CONSTRAINT` ifadeleri.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `cargo test -p pgt-diff generate` → FAIL.
- [ ] **Adım 3: Uygula.**
- [ ] **Adım 4: Doğrula.** `cargo test -p pgt-diff`, clippy.
- [ ] **Adım 5: Commit.**
  ```bash
  git add crates/pgt-diff/src
  git commit -m "feat(diff): öğe başına aşamalı DDL üretimi (FK, OWNED BY, yeniden kurma, yetkiler)"
  ```

### Task 7: Betik birleştirme ve sıralama

**Dosyalar:** Değiştir `crates/pgt-diff/src/script.rs` (Task 3'teki `comment_safe` korunur; + insta).

**Arayüzler:**
- Tüketir: Task 6 `Generated`/`Phase`, Task 2 `DiffScript`.
- Üretir:

```rust
pub struct ScriptItem<'a> {
    pub key: &'a str, pub kind: ObjectKind, pub target_kind: Option<ObjectKind>, pub name: &'a str, pub table: Option<&'a str>,
    pub generated: &'a Generated,
    pub source_deps: Vec<&'a str>,   // oluşturma için: kaynakta bağımlı olunan seçili öğe anahtarları (görünüm/MV, tip/domain)
    pub target_deps: Vec<&'a str>,   // silme için: hedefte bağımlı olunan seçili öğe anahtarları (görünüm/MV, tablo FK)
}
pub struct ScriptHeader<'a> { pub source_db: &'a str, pub source_schema: &'a str, pub target_db: &'a str, pub target_schema: &'a str, pub target_version: &'a str, pub generated_at: &'a str }
pub fn assemble(h: &ScriptHeader, items: &[ScriptItem]) -> DiffScript;
pub fn topo_order<'a>(keys: &[&'a str], deps: &dyn Fn(&str) -> Vec<&'a str>) -> Vec<&'a str>;   // bağımlılık önce; eşitlikte giriş sırası; döngüde kalanlar giriş sırasıyla sona
pub fn utc_timestamp(t: std::time::SystemTime) -> String;    // "2026-10-31 00:00:00 UTC"
pub fn type_refs(def: &ObjectDef) -> Vec<String>;            // composite öznitelik tipleri, domain temel tipi, range alt tipi; sondaki "[]" ve "(…)" atılmış, tırnak çözülmüş adlar
```
- **Sıra (spec §2.6, Kararlar 7, 11–13):**
  - Aşamalar `Phase` sırasıyla yazılır.
  - Aşama içi sıralar:
    - `DropView`: hedef bağımlılığının tersi (`topo_order` ters çevrilir);
    - `Views` ve `Types`: kaynak bağımlılığı (`topo_order`);
    - `DropTable`: hedef FK bağımlılığının tersi (başvuran önce silinir);
    - diğerleri: tür, sonra ad (bayt).
  - Bir öğenin her aşamadaki ifadeleri kendi sırasını korur.
- **Otomatik silinen dizi:** `sequence_owner = (t, c)` olan yalnız-hedef dizi için iki durumda `DROP SEQUENCE` yazılmaz: aynı betikte `t`'yi silen öğe (`drops_table`) varsa ya da `t` öğesinin `dropped_columns`'ı `c`'yi içeriyorsa. Yerine `-- <dizi>: bağlı olduğu kolonla birlikte silinir` satırı yazılır.
- **Metin:**
  1. Başlık (her satır `-- ` ile başlar; tüm değişkenler `comment_safe`):
     ```
     -- pg-tools şema karşılaştırma betiği
     -- Kaynak: <db>.<şema>
     -- Hedef: <db>.<şema> (PostgreSQL <sürüm>)
     -- Oluşturuldu: <zaman>
     -- Öğe: <n>, ifade: <m>, en yüksek risk: <Güvenli|Yıkıcı|Veri kaybı olabilir>
     -- Bağımlılıklar tamamlanmaz: seçilmeyen nesnelere bağlı öğeler hedefte hata verebilir.
     ```
     İfadelerden biri bile transaction dışıysa ek satır: `-- UYARI: Betik transaction'sız; hata olursa kısmen uygulanmış kalır.`
  2. Oturum satırları (Kararlar 13).
  3. Bölüm satırları: `-- ==== Silmeler ====`, `-- ==== Oluşturma ve değişiklikler ====`, `-- ==== Yetkiler ====` (boş bölüm yazılmaz).
  4. Her öğenin ilk ifadesinden önce `-- <tür etiketi> <tablo.>ad` satırı. Tür etiketleri sabit Türkçe adlardır: Tablo, Görünüm, Materialized view, Sequence, İndeks, Trigger, Fonksiyon, Prosedür, Tip, Domain.
  5. Notlu ifadeden önce `-- not: <not>` satırı.
  6. İfade metni + `;`.
  7. `COMMIT;` (transaction varsa).
- `DiffScript`:
  - `transactional` = tüm ifadeler transaction içinde çalışabilir;
  - `risk` = öğe risklerinin en yükseği;
  - `statements` = SQL ifade sayısı (oturum satırları hariç);
  - `warnings` = öğe uyarıları (tekrarsız, ilk görülme sırası) + transaction'sız uyarısı.
- Başlık üreticisi sunucu kaydı, adres ya da kullanıcı almaz.

- [ ] **Adım 1: Başarısız testler (insta).**
  - Tam senaryo (tüm aşamalardan birer öğe) → aşama ve bölüm sırası.
  - FK'lı iki yeni tablo → `CREATE TABLE`'lardan sonra `ADD CONSTRAINT`.
  - Görünüm zinciri `v1 ← v2`: oluşturmada v1, v2; silmede v2, v1.
  - Composite `a(x b)`, `b` enum → `b` önce. Domain temel tipi enum → enum önce.
  - Silme sırası: trigger → indeks → görünüm → rutin → tablo → sequence → domain → type.
  - Birbirine FK'lı iki tablo silmesi → başvuran önce.
  - `topo_order` döngü → giriş sırasıyla, kayıpsız.
  - Otomatik silinen dizi (iki koşul) → `DROP SEQUENCE` yok, açıklama satırı var.
  - Transaction'sız ifade (ctx PG 11 enum ADD VALUE) → `BEGIN` yok, `SET` LOCAL'sız, uyarı başlıkta ve `warnings`'te.
  - Transaction'lı → `BEGIN;`, `SET LOCAL …`, `COMMIT;`; `search_path` satırı `pg_catalog, "Hedef Şema"`.
  - Başlık güvenliği:
    - veritabanı adı `"a\nDROP TABLE x; --"` ve şema adı `"s\u{2028}x"` → ilk ifadeye kadar her satır `--` ile başlar ve satır sayısı sabittir;
    - başlık metni yalnız verilen alanları içerir (adres, kullanıcı alanı yok — imza testi: `ScriptHeader` alan listesi).
  - `utc_timestamp(UNIX_EPOCH + 1_793_404_800 sn)` == `"2026-10-31 00:00:00 UTC"`; `UNIX_EPOCH` → `"1970-01-01 00:00:00 UTC"`.
  - `type_refs`: `"b[]"`, `"\"My T\""`, `"numeric(10,2)"` örnekleri.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `cargo test -p pgt-diff script` → FAIL.
- [ ] **Adım 3: Uygula.**
- [ ] **Adım 4: Doğrula.** `cargo test -p pgt-diff`, clippy.
- [ ] **Adım 5: Commit.**
  ```bash
  git add crates/pgt-diff/src
  git commit -m "feat(diff): betik birleştirme, bağımlılık sıralaması ve güvenli başlık"
  ```

### Task 8: Kayıt defteri, yürütme ve Tauri komutları

**Dosyalar:**
- Oluştur: `crates/pgt-diff/src/{registry.rs, run.rs}`, `src-tauri/src/schema_diff.rs`.
- Değiştir: `src-tauri/Cargo.toml` (`pgt-diff.workspace = true`), `src-tauri/src/state.rs` (`pub diff: pgt_diff::DiffRegistry`), `src-tauri/src/main.rs` (komut kaydı; `on_page_load` ve `RunEvent::Exit`'te `state.diff.clear()`), `src/ipc/bindings` (gerekirse).

**Arayüzler — Üretir:**

```rust
// run.rs
pub trait DiffSink: Send { fn send(&mut self, e: DiffEvent) -> bool; }   // false → istemci gitti → iptal
pub enum RunError { SameSchema, Cancelled, Schema { side: SideTag, check: SchemaCheck }, Core(pgt_core::Error) }
pub fn same_side(a: &DiffSide, b: &DiffSide) -> bool;                    // sunucu, veritabanı oid'i ve şema adı (bayt) eşit
pub struct DiffData { … }   // sonuç, öğe girdileri (Prepared + Generated|GenFailure + bağımlılık anahtarları), başlık bilgisi, hedef BuildCtx, son betik
pub async fn run_diff(mgr: &ConnectionManager, source: &DiffSide, target: &DiffSide, options: DiffOptions,
                      sink: &mut dyn DiffSink, cancel: &CancelToken) -> Result<DiffData, RunError>;
impl DiffData {
    pub fn result(&self) -> &DiffResult;
    pub fn detail(&self, key: &str) -> Option<DiffDetail>;               // iki tarafın `ddl_text`'i (hedef ctx) + `line_diff`
    pub fn script(&self, keys: &[String], generated_at: &str) -> Result<DiffScript, ScriptError>;   // başarıda `last_script` güncellenir
    pub fn last_script(&self) -> Option<String>;
}
pub enum ScriptError { Empty, UnknownKey, NotSelectable }
// registry.rs
pub const MAX_READY: usize = 8;
pub enum RegisterError { DuplicateId }
pub struct DiffRegistry;   // Clone; iç Arc<Mutex<…>>
impl DiffRegistry {
    pub fn register(&self, id: Uuid) -> Result<(CancelToken, RunGuard), RegisterError>;
    pub fn complete(&self, id: Uuid, data: DiffData);    // kayıt hâlâ "çalışıyor"sa hazıra geçer; kapatılmışsa veri düşer. En eski hazır kayıtlar MAX_READY'ye kırpılır.
    pub fn get(&self, id: Uuid) -> Option<Arc<DiffData>>;
    pub fn cancel(&self, id: Uuid);
    pub fn close(&self, id: Uuid);                       // iptal + kaldır
    pub fn clear(&self) -> usize;                        // hepsini iptal et ve kaldır
}
```
```rust
// src-tauri/src/schema_diff.rs (Tauri; bağımsız değişkenler camelCase)
schema_diff_databases(server_id: Uuid) -> Vec<DiffDatabase>
schema_diff_start(diff_id: Uuid, source: DiffSide, target: DiffSide, options: DiffOptions, on_event: Channel<DiffEvent>) -> DiffResult
schema_diff_cancel(diff_id: Uuid) -> ()
schema_diff_detail(diff_id: Uuid, item_key: String) -> DiffDetail
schema_diff_script(diff_id: Uuid, item_keys: Vec<String>) -> DiffScript
schema_diff_save(diff_id: Uuid, path: String) -> ()
schema_diff_close(diff_id: Uuid) -> ()
```
- **`run_diff`:**
  1. `same_side` doğruysa → `SameSchema`.
  2. İki taraf `tokio::try_join!` ile eşzamanlı yüklenir. Sink `std::sync::Mutex` içinde paylaşılır; `report` olay gönderir.
  3. `match_objects` → `prepare` → `generate` (hedef `BuildCtx`, hedefin `viewer`'ı).
  4. `GenFailure` öğeye `problem` ve iletiler olarak yazılır.
  5. `DiffItem`'lar kurulur; `selectable`/`default_selected` Task 2 kuralıyla.
  6. Uyarılar:
     - şema adları farklıysa "Gövdelerdeki nitelikli adları gözden geçirin: kaynak şemayı açıkça niteleyen adlar çevrilmez.";
     - ana sürümler farklıysa "Kaynak PostgreSQL <s>, hedef <t>: ifadeler hedef sürüme göre üretilir.".
  7. `tracing::info!(job = "schema_diff", outcome, source_objects, target_objects, items, problems, elapsed_ms)`; başka alan yazılmaz.
- **Bağımlılık anahtarları:**
  - Görünüm/MV `deps` oid'leri aynı taraftaki listelenmiş görünüm/MV'lerin anahtarlarına çevrilir.
  - Tip/domain için `type_refs` adları aynı sınıftaki anahtarlara çevrilir.
  - Tablo FK'ları hedef modelindeki `ref_table` (aynı şema) üzerinden çevrilir.
- **Komut kuralları:**
  - **`schema_diff_databases`:** bakım bağlantısında `SELECT oid, datname::text FROM pg_catalog.pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname COLLATE "C"`.
  - **`schema_diff_start`:** `register(diff_id)` (çakışma → `busy`) → `Box::pin(run_diff(..))`. Başarıda `complete` ve `DiffResult` döner.
  - **`schema_diff_script`:** `generated_at = utc_timestamp(now)`.
  - **`schema_diff_save`:**
    - Yol doğrulaması saf `save_path_check(path) -> Result<(), &'static str>`: mutlak olmalı (Windows), denetim karakteri içermemeli.
    - Son betik yoksa `no_script`.
    - Yazma `spawn_blocking` içinde: `<yol>.pgt-partial`'a yazılır, `std::fs::rename` ile hedefe taşınır (var olan değiştirilir). Hata olursa geçici dosya silinir.
    - UTF-8, BOM yok. Yol günlüğe ve hata iletisine yazılmaz.
  - **`schema_diff_close`:** `close` (yoksa sessiz).
  - Gelecekler `Box::pin` ile kutulanır (ana iş parçacığı yığını; `ddl.rs`/`backup.rs` notu).
- **Hata kodları** (`ErrorPayload.code`; ileti Türkçe, ad/yol içermez):

| Kod | Durum |
|---|---|
| `same_schema` | `SameSchema` |
| `schema_not_found` | şema yok (`detail`: `"source"` \| `"target"`) |
| `system_schema` | sistem şeması (`detail`: `"source"` \| `"target"`) |
| `cancelled` | iptal |
| `busy` | aynı kimlikle çalışan/hazır kayıt |
| `diff_not_found` | kimlik kayıtta yok |
| `item_not_found` | `UnknownKey` |
| `not_selectable` | `NotSelectable` |
| `empty_selection` | `Empty` |
| `no_script` | kaydetmeden önce betik üretilmemiş |
| `invalid_path` | `save_path_check` reddi |
| `write_failed` | yazma/taşıma hatası |

  `Core` hataları `ErrorPayload::from` ile eşlenir (`not_connected`, `connection_lost` …).

- [ ] **Adım 1: Başarısız testler.**
  - `registry.rs`:
    - aynı kimlik ikinci kez → `DuplicateId`;
    - `close` sonra `complete` → `get` `None`;
    - 9 hazır kayıt → en eskisi düşer, 8 kalır;
    - `clear` çalışan belirteçleri iptal eder ve sayıyı döndürür;
    - `RunGuard` düşünce çalışan kayıt (tamamlanmadıysa) silinir.
  - `run.rs` (saf parçalar):
    - `same_side` (şema adı büyük/küçük harf duyarlı: `S` ≠ `s`);
    - `DiffData::script`: boş → `Empty`, bilinmeyen → `UnknownKey`, `selectable = false` → `NotSelectable`;
    - başarıda `last_script` dolar;
    - `detail` bilinmeyen anahtarda `None`.

    `DiffData`'yı sunucusuz kurmak için `#[doc(hidden)] pub fn from_parts(…)` kullanılır.
  - `src-tauri/src/schema_diff.rs` birim:
    - `run_error_payload` kod tablosu;
    - `save_path_check` (göreli → `"Tam (mutlak) yol gerekir."` yalnız `#[cfg(windows)]`; `\n` → `"Yol geçersiz karakter içeriyor."`);
    - `write_script` (tempfile): yeni dosya yazılır; var olan değiştirilir; hedef bir klasörse hata döner ve `.pgt-partial` kalmaz.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `cargo test -p pgt-diff registry` → FAIL.
- [ ] **Adım 3: Uygula.**
- [ ] **Adım 4: Doğrula.**
  - `cargo test -p pgt-diff`, `cargo test -p pg-tools schema_diff`, clippy, `cargo build -p pg-tools`.
  - Bindingler değişmediyse dokunulmaz.
  - `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check` (ön yüz değişmediği için baytlar aynı).
- [ ] **Adım 5: Commit.**
  ```bash
  git add crates/pgt-diff src-tauri/Cargo.toml src-tauri/src/schema_diff.rs src-tauri/src/state.rs src-tauri/src/main.rs Cargo.lock
  git commit -m "feat(diff): fark kaydı, yürütme ve schema_diff_* Tauri komutları"
  ```

### Task 9: Docker uçtan uca testler, belgeler ve sözleşme notları

**Dosyalar:**
- Oluştur: `crates/pgt-diff/tests/diff_roundtrip.rs`.
- Değiştir: `crates/pgt-diff/tests/common/mod.rs` (fikstür yardımcıları), `docs/testing.md`, `docs/smoke-test.md`, `docs/superpowers/specs/2026-10-31-schema-diff-design.md` (Durum + §6).

- [ ] **Adım 1: Başarısız testler (Docker; her sürüm, sürüme bağlı adımlar atlanır).**
  - **Gidiş-dönüş (iki biçim):** (a) aynı veritabanında `pgt_diff_src` → `pgt_diff_tgt`; (b) iki ayrı veritabanında aynı şema adı `app`.
    - Kaynak fikstür:
      - enum, composite (enum'a başvuran), range, domain;
      - `serial` PK'lı tablo ve ona FK'lı tablo;
      - CHECK, UNIQUE, ifade indeksi, kısmi indeks;
      - trigger fonksiyonu + trigger;
      - iki aşırı yüklü fonksiyon, procedure (PG 11+);
      - görünüm zinciri `v1 ← v2`, MV ve MV üzerinde indeks;
      - `OWNED BY`'lı bağımsız sequence;
      - yorumlar.
    - Hedef fikstür (farklar):
      - eksik tablo;
      - kolon tipi farkı, fazla kolon, yeniden adlandırılmış kolon;
      - eksik indeks;
      - farklı görünüm sorgusu, farklı fonksiyon gövdesi, farklı dönüş tipi;
      - enum'da eksik değer, farklı domain CHECK'i;
      - eksik trigger;
      - yalnız hedefte tablo (ve ona bağlı `serial` dizisi);
      - kolon sırası farklı ama eşit tablo.
    - Beklenen durumlar tablo olarak sabitlenir.
    - Seçilebilen tüm öğeler (yalnız-hedef dahil) seçilir → `script` → betik hedefe yönetici istemcisiyle `batch_execute` uygulanır → yeniden karşılaştırılır. Sonuçta `Same` dışında yalnız beklenen `Manual` öğeler kalır.
  - **Sahiplik/yetki seçeneği:** iki test rolü (`pgt_diff_rk_<etiket>`); sahip ve GRANT farkları → betik → yeniden karşılaştırma → `Same`. Roller temizlenir.
  - **Bölümleme:** bölümlü üst tablo yalnız kaynakta → `Manual` ve betikte yok; bölüm çocuğu → `Manual`.
  - **Çapraz sürüm:** `PGT_TEST_URLS`'te ana sürüm 16 ve 13 varsa kaynak 16'da `UNIQUE NULLS NOT DISTINCT`'li tablo, hedef 13 → `BuildFailed`, ileti PG 15 sürüm iletisi. Sürümler yoksa `eprintln!` ile atlanır.
  - **Uzantı:** kaynak şemada `citext` uzantısı → listede citext nesnesi yok.
  - **İptal:** 400 tablolu kaynakta iş başladıktan sonra `cancel` → `Err(Cancelled)`. `pg_stat_activity`'de bu veritabanlarında `idle in transaction` oturum yok.
  - **Eşzamanlılık:** iş sürerken `mgr.run` havuz sorgusu başarılı.
  - **Günlük yakalama** (`tracing-subscriber` yazıcısı, `pgt-ddl/tests/logging.rs` deseni): gidiş-dönüş boyunca günlük satırları şema, tablo, sütun ve veritabanı adlarından hiçbirini ve `CREATE`/`ALTER` sözcüklerini içermez.
  - **Performans** (`#[ignore]`, `diff_perf_500_tables`): taraf başına 500 tablo / 2.000 nesne. Süre yazdırılır, ölçülmez (spec §4).
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `cargo test -p pgt-diff --test diff_roundtrip` (PGT_TEST_URLS olmadan atlar). Matris ilk koşuda hatalı fikstür/beklentiyi gösterir.
- [ ] **Adım 3: Düzelt ve belgele.**
  - Bulunan üretim hataları ilgili görevin modülünde düzeltilir ve saf testi eklenir.
  - `docs/testing.md`'ye "## Şema karşılaştırma arka ucu testleri (Plan 29)" eklenir: dosya → kapsam tablosu (`key`, `linediff`, `normalize`, `generate`, `script`, `registry`, `list_snapshot`, `diff_roundtrip`, `pgt-ddl/tests/load_in.rs`), matris komutu, `#[ignore]` performans testinin elle çalıştırılması.
  - `docs/smoke-test.md`'de "## Görsel EXPLAIN arayüzü (Plan 28)" bölümünden sonra "## Şema karşılaştırma arka ucu (Plan 29)" eklenir. GUI maddesi yoktur (Plan 30'da). İki `[—]` maddesi:
    - performans hedefinin gerçek sunucuda ölçülmesi;
    - Explorer'ın karşılaştırma sürerken akıcı kalması.
  - Spec `Durum`: "Plan 29 arka ucu uygulandı; Plan 30 ön yüzü bekliyor". Yeni **"## 6. Plan 29 sözleşme notları (Plan 30 için)"** eklenir:
    - komut imzaları ve hata kodları tablosu;
    - `DiffEvent` biçimi;
    - `DiffItem` alan anlamları ve `selectable`/`default_selected` kuralı;
    - `problem` iletileri;
    - kayıt defteri sınırı (`MAX_READY = 8`, sayfa yenilemede temizlik);
    - ayrıntıda `lines = null` (20.000 satır üstü);
    - Kararlar 14'teki bilinen sınırlar.
- [ ] **Adım 4: Doğrula.**
  - Matris: `powershell -File scripts/test-matrix.ps1 -Versions 10,13,16,18 -SkipPerf` PASS.
  - `cargo test --workspace`, clippy.
  - `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`.
  - Kapanış taramaları:
    - `rg "tracing::" crates/pgt-diff` → alanlarda ad, SQL ya da yol yok;
    - `rg "open_dedicated" crates/pgt-diff` → yalnız `snapshot.rs`;
    - `git status` → `graphify-out/` ve binding gürültüsü sahnelenmemiş.
- [ ] **Adım 5: Commit.**
  ```bash
  git add crates/pgt-diff docs/testing.md docs/smoke-test.md docs/superpowers/specs/2026-10-31-schema-diff-design.md
  git commit -m "test(diff): Docker gidiş-dönüş, çapraz sürüm ve günlük testleri; Plan 29 sözleşme notları"
  ```

## Öz inceleme

- **Spec kapsamı:**
  - §2.1 yükleme → T1 (iç fonksiyonlar), T4 (liste, ayrılmış bağlantı, transaction, `search_path`, ilerleme, iptal), T8 (sürüm, aynı şema reddi).
  - §2.2 eşleştirme → T2.
  - §2.3 normalizasyon → T5.
  - §2.4 durum ve üretim → T5 (karar) ve T6 (üretim, yeniden kurma, elle gerekiyor); satır farkı → T3 ve T8 `detail`.
  - §2.5 hatalar → T4 (hata sınıfları), T6 (`BuildFailed`), T8 (kodlar).
  - §2.6 sıralama → T6 (aşamalar), T7.
  - §2.7 IPC → T8; günlük → Global Constraints + T9.
  - §3 test → T1–T9; §4 → T9 `#[ignore]`.
- **Tip tutarlılığı:**
  - `Listed` (T2) → `list_objects` (T4) → `match_objects` (T2) → `prepare` (T5) → `generate` (T6) → `ScriptItem` (T7) → `DiffData` (T8).
  - `DiffItem`/`DiffResult`/`DiffDetail`/`DiffScript` (T2) → komutlar (T8) → Plan 30.
  - `CancelToken` (T4) → `DiffRegistry` (T8).
- **Plan 30'un tükettiği yüzey:** yalnız T2 bindingleri ve T8 komut adları/hata kodları (spec §6'da da yazılır).
