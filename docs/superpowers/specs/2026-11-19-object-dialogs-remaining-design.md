# pg-tools — Alt Proje 5c: Kalan Nesne Diyalogları Tasarımı (Policy, Tablespace, Bölümler, FDW, Publication, Subscription)

- **Tarih:** 2026-11-19
- **Durum:** Plan 34–37 uygulandı (Grup A–B: Policy, Tablespace ve Bölümler); Grup C kalan
- **Kapsam:** Explorer'da bulunup diyaloğu olmayan **RLS politikası** ve **tablespace**; bölümlü tabloya **bölüm oluşturma / bağlama / ayırma**; ağaçta henüz bulunmayan **Foreign Data Wrapper, Foreign Server, User Mapping, Foreign Table, Publication, Subscription** (önce Explorer düğümleri, sonra diyaloglar).
- **Temel:** `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (mimari §2, risk modeli §3, arayüz §4, güvenlik §6, test §7 ve tüm sözleşme notları) ile `docs/superpowers/specs/2026-10-16-object-dialogs-routines-types-design.md` (R5 salt okunur kuralı, `editable`/`unsupported`, eklentiye ait nesneler, `OWNER TO` en sonda) **aynen geçerlidir**. Bu belge yalnızca yeni türleri, kurallarını ve kararlarını ekler; ana spec'in §1.3 "kapsam dışı" listesinden RLS politikaları, FDW/publication/subscription diyalogları, partition oluşturma/bağlama ve tablespace oluşturma çıkarılır.
- **Kullanıcı kararı (2026-11-19):** dört grubun tamamı kapsamda. Plan 33 CSV içe aktarmaya ayrılmıştır; bu alt proje Plan 34'ten başlar.

---

## 0. Teslim grupları ve sıra

| Grup | Planlar | İçerik |
|---|---|---|
| A | **Plan 34** (arka uç) → **Plan 35** (ön yüz) | Policy + tablo RLS bayrakları; Tablespace |
| B | **Plan 36** (arka uç) → **Plan 37** (ön yüz) | Bölüm oluştur / bağla / ayır / sınır değiştir |
| C | **Plan 38** (Explorer düğümleri) → **Plan 39** (arka uç) → **Plan 40** (ön yüz) | FDW, Foreign Server, User Mapping, Foreign Table, Publication, Subscription |

Her grup kendi dalında biter ve `main`'e ayrı birleşir; sonraki grup bir öncekinin birleşmesini bekler (ortak dosyalar: `model.rs`, `target.rs`, `manager.rs`, `ddl-kinds.ts`).

---

## 1. Kararlar (tüm gruplar)

| Konu | Karar |
|---|---|
| Ana paket | Bütçe tavanları **yükseltilmez**. Her ön yüz planının menü/ikon kancası, aynı görevde **adı konmuş bir telafiyle** (yalnız tembel kullanılan komutları `src/ipc/commands.ts`'ten tembel modüle taşımak) karşılanır; tavanlar ölçülen değere indirilir. |
| Yeni türler | `ObjectKind`'e eklenir: `policy`, `tablespace`, `partition`, `foreign_data_wrapper`, `foreign_server`, `user_mapping`, `foreign_table`, `publication`, `subscription`. |
| Sunucu kapsamı | `Tablespace` sunucu kapsamlıdır (`server_scoped() = true`; bakım veritabanında yüklenir/uygulanır; `databaseOid = null`). Diğer yeni türler veritabanı kapsamlıdır (Subscription dahil: `pg_subscription.subdbid`). |
| Gizli değerler | Subscription `CONNECTION` dizgesindeki `password` ve User Mapping / Foreign Server `OPTIONS` içindeki **`password`** anahtarı gizlidir: `Secret` tipinde taşınır, önizlemede `********`, `tracing`'e ve hata iletisine yazılmaz; yükleyici bu değeri **hiç döndürmez** (yerine `passwordSet: bool`). Değiştirilmeyen gizli değer ifadeye yazılmaz. |
| Genel `OPTIONS` | FDW / Server / User Mapping / Foreign Table / foreign kolon seçenekleri ortak `GenericOption { name, value, original }` listesidir; alter farkı `OPTIONS (ADD a 'x', SET b 'y', DROP c)` olarak tek ifadede yazılır. Ad: boş olmayan, NUL/denetim karakteri içermeyen tanımlayıcı (tırnaklanır); değer: `quote_literal`. |
| Serbest SQL | Policy `USING` / `WITH CHECK`, bölüm sınır değerleri, Publication satır süzgeci (PG 15+) mevcut kapıdan geçer (`check_free_sql` + `wrap` + `composed_is_clean`; sıradan `'…'` içinde `\` → `E'…'`). |
| Transaction dışı | `CREATE/DROP TABLESPACE`, `CREATE SUBSCRIPTION` (slot oluşturuluyorsa), `DROP SUBSCRIPTION` (slot bağlıysa), `ALTER SUBSCRIPTION … REFRESH PUBLICATION`, `DETACH PARTITION … CONCURRENTLY` (PG 14+) `stmt_nt` ile; kısmi uygulama kuralı (ana spec `failed.created`) geçerlidir. |
| Salt okunur | R5 kuralı: `unsupported` doluysa `editable = false`. Ek gerekçeler türlere göre aşağıda. Eklentiye ait (`from_extension`) FDW/server/foreign table salt okunurdur. |
| Sürüm kapıları | Arayüzde gizlenmez; ipucu + arka uç alan hatası (Plan 13 ilkesi; arayüz modelinde sunucu sürümü yok). |

---

## 2. Grup A — Policy ve Tablespace

### 2.1 Policy (RLS politikası)

- **Explorer:** mevcut `…/tbl:T/policies` koleksiyonu ve `pol:P` düğümü (yalnız sıradan ve bölümlü tablo). Koleksiyonda **Oluştur…**, düğümde **Düzenle… / Yeniden adlandır… / Sil…**. Oluşturma bağlamı: `schemaOid`, `tableOid`.
- **Alanlar (`PolicyDef`):** `schema`, `table` (bağlamdan; değiştirilemez), `name`, `command` (`all|select|insert|update|delete`), `permissive` (`true` = PERMISSIVE, `false` = RESTRICTIVE; PG 10+), `roles` (boş liste = `PUBLIC`; öğeler rol adı, `public` ayrı öğe değil), `using` (serbest SQL, isteğe bağlı), `withCheck` (serbest SQL, isteğe bağlı), `comment`, `editable`, `unsupported`.
- **Doğrulama:** `insert` için `using` yasak; `select` ve `delete` için `withCheck` yasak (PG kuralı, alan hatası). `all`/`update` her ikisini kabul eder. En az biri olmayan politika geçerlidir (PG varsayılanı) ama önizleme uyarısı verir ("ifadesiz politika: PERMISSIVE → tüm satırlar, RESTRICTIVE → etkisiz").
- **Düzenleme:** `ALTER POLICY … RENAME TO`, `ALTER POLICY … TO roller`, `USING (…)`, `WITH CHECK (…)` (Safe; tek `ALTER POLICY` ifadesinde birleştirilir). `command` ya da `permissive` değişimi PG'de yapılamaz → aynı transaction'da `DROP POLICY` + `CREATE POLICY` (**Destructive** + not "aynı transaction içinde yeniden kurulur"; Plan 18 trigger kalıbı). Bir ifadeyi **kaldırmak** (`using` doluydu → boş) `ALTER POLICY` ile yapılamaz → aynı yeniden kurma.
- **RLS durumu:** yükleyici tablonun `relrowsecurity` / `relforcerowsecurity` değerlerini salt okunur `tableRls: { enabled, forced }` olarak döndürür; tabloda RLS kapalıyken önizleme uyarısı: "Tabloda satır güvenliği kapalı; politika etkili olmaz." 
- **Tablo diyaloğuna ek (Table):** `TableDef.rowSecurity: { enabled, forced }` (`#[serde(default)]`; yeni tabloda `false/false`). Alter: `ALTER TABLE … ENABLE/DISABLE ROW LEVEL SECURITY`, `FORCE/NO FORCE ROW LEVEL SECURITY`. Risk: ENABLE ve FORCE **Destructive** (politika yoksa sahip dışındaki herkes için tablo boş görünür: önizleme notu), DISABLE ve NO FORCE Safe + uyarı ("politikalar devre dışı kalır"). Bölüm tablosunda da düzenlenebilir.
- **Silme:** `DROP POLICY name ON tablo` — **Destructive** (erişimi genişletebilir ya da daraltabilir). CASCADE yoktur.
- **Salt okunur:** yok (tüm politikalar modellenebilir); tablo eklentiye aitse politika da salt okunurdur.
- **Kimlik:** politika adı tablo içinde tekildir; `created` yeni oid ile döner (yeniden kurmada oid değişir → sekme `object`'i güncellenir, ana spec kuralı).
- **Açılır listeler:** roller (`roles`, mevcut).

### 2.2 Tablespace

- **Explorer:** sunucu altındaki `tablespaces` koleksiyonu ve `ts:N` düğümü. Koleksiyonda **Oluştur…**, düğümde **Düzenle… / Yeniden adlandır… / Sil…** ve **Yetkiler…** (bkz. ACL).
- **Alanlar (`TablespaceDef`):** `name`, `owner`, `location` (sunucudaki mutlak dizin; yalnız oluşturmada; boş dizge = yerinde (`allow_in_place_tablespaces`) **desteklenmez** → alan hatası), `options` (`seq_page_cost`, `random_page_cost`, `effective_io_concurrency`, `maintenance_io_concurrency` (PG 13+); sayısal **metin**, boş = ayarsız), `comment`, `editable`, `unsupported`.
- **Doğrulama:** `location` mutlak yol olmalı (Windows sürücü harfi `C:\…` ya da `/` ile başlayan); NUL ve tek tırnak dışı karakter serbest, `quote_literal` ile yazılır; yükleyici `pg_tablespace_location()` döndürür. Ad `pg_` ile başlayamaz (PG kuralı, alan hatası). Seçenek değerleri: `seq_page_cost`/`random_page_cost` sonlu ≥ 0 ondalık; `*_io_concurrency` 0–1000 tamsayı.
- **İfadeler:** `CREATE TABLESPACE … OWNER … LOCATION '…' [WITH (…)]` (**transaction dışı**; ardından `COMMENT` ayrı ifade — kısmi uygulama kuralı), `ALTER TABLESPACE … RENAME TO`, `OWNER TO` (en sonda), `SET (…)` / `RESET (…)`, `COMMENT ON TABLESPACE`. `DROP TABLESPACE` **transaction dışı**, **Destructive** (boş değilse PG 55000 ile reddeder → ipucu: "Tablespace boş değil; önce içindeki nesneleri başka tablespace'e taşıyın (bu tablespace başka veritabanlarında da nesne içerebilir).").
- **Salt okunur:** `pg_default` ve `pg_global` (gerekçe "sistem tablespace'i"); yeniden adlandır/sil açık ret.
- **İzinler:** `CREATE TABLESPACE` süper kullanıcı ister → PG 42501 iletisi + ipucu "yalnızca süper kullanıcı". Arayüz önceden engellemez.
- **ACL:** Plan 20/22 yetki matrisi `tablespace` türüyle genişletilir (tek ayrıcalık `CREATE`; `GRANT … ON TABLESPACE`). Varsayılan yetkiler kapsam dışıdır (PG desteklemez).

### 2.3 Risk özeti (Grup A)

| Risk | İfadeler |
|---|---|
| Safe | `CREATE POLICY`, `ALTER POLICY` (ad/roller/ifadeler), `COMMENT`, `CREATE TABLESPACE`, tablespace `RENAME`/`OWNER TO`/`SET`/`RESET`, tablo `DISABLE`/`NO FORCE ROW LEVEL SECURITY` (+ uyarı) |
| Destructive | `DROP POLICY`, politika yeniden kurma, tablo `ENABLE`/`FORCE ROW LEVEL SECURITY`, `DROP TABLESPACE` |
| DataLoss | — |

---

## 3. Grup B — Bölümler (Partition)

### 3.1 Model

- **Yeni tür `partition`** yalnızca bölüm **sınırı ve üyeliği** içindir; bölüm tablosunun kendi özellikleri (ad, sahip, tablespace, yorum, fillfactor) mevcut **Table** diyaloğunda kalır (Plan 12/13 `isPartition` kuralı değişmez).
- **`PartitionDef`:** `parentSchema`, `parentTable` (bağlamdan; değiştirilemez), `strategy` (üst tablonun `range|list|hash`'i; yükleyici doldurur, salt okunur), `keyCount` (salt okunur), `mode` (`create` | `attach`; yalnız oluşturmada anlamlı), `schema`, `name` (create: yeni tablo; attach: bağlanacak **var olan** tablo), `bound` (aşağıda), `partitionBy` (yalnız create; isteğe bağlı alt bölümleme, mevcut `PartitionByDef`), `tablespace`, `owner`, `comment` (yalnız create; attach'te yok sayılır), `editable`, `unsupported`.
- **`PartitionBound`** (etiket `kind`): `range { from: BoundValue[], to: BoundValue[] }`, `list { values: string[] }`, `hash { modulus, remainder }`, `default` (PG 11+). `BoundValue` (etiket `kind`): `value { sql }` | `minvalue` | `maxvalue`. Değer ifadeleri serbest SQL'dir (sabit ifade; PG 10–11 yalnız literal kabul eder → PG hatası olduğu gibi). `list.values` içinde `NULL` serbesttir.
- **Doğrulama:** `range` için `from`/`to` uzunluğu = `keyCount`; `MINVALUE`/`MAXVALUE` sonrasında aynı tarafta yalnız aynı anahtar sözcük (PG kuralı); `list` en az bir değer; `hash` `modulus ≥ 1`, `0 ≤ remainder < modulus`; strateji uyuşmazlığı alan hatası (`bound`). `default` PG < 11 alan hatası; `hash` PG < 11 alan hatası.

### 3.2 Explorer ve eylemler

- **Bölümlü tablonun `partitions` koleksiyonu:** **Bölüm oluştur…** (`mode = create`) ve **Bölüm bağla…** (`mode = attach`; `name` seçimi için açılır liste: aynı veritabanındaki, bölüm olmayan, kalıtımsız sıradan/bölümlü tablolar — `attachable_tables` lookup'ı, en çok 5000).
- **Bölüm düğümü (`…/tbl:P/tbl:C`):** mevcut **Düzenle…** (Table) korunur; ek olarak **Bölüm sınırı…** (Partition diyaloğu yüklenir: `bound` düzenlenebilir) ve **Bölümü ayır…** (ayırma diyaloğu).
- **İfadeler:**
  - create: `CREATE TABLE s.n PARTITION OF ps.pt FOR VALUES … | DEFAULT [PARTITION BY …] [TABLESPACE …]`, sonra `ALTER TABLE … OWNER TO`, `COMMENT` (Safe).
  - attach: `ALTER TABLE ps.pt ATTACH PARTITION s.n FOR VALUES … | DEFAULT` (**Destructive**: tablo taranır, kısıt doğrulanır; üst tabloda PG 12+ `SHARE UPDATE EXCLUSIVE`, öncesinde `ACCESS EXCLUSIVE` — önizleme notu sürüme göre).
  - sınır değiştir: aynı transaction'da `DETACH` + `ATTACH` (**Destructive** + not "aynı transaction içinde yeniden bağlanır; tablo yeniden taranır").
  - ayır: `ALTER TABLE ps.pt DETACH PARTITION s.n` (**Destructive**: satırlar üst tablodan görünmez olur); PG 14+ seçenekli `CONCURRENTLY` (transaction dışı, DEFAULT bölüm varken PG reddeder → ipucu) ve yarım kalmış ayırmada `FINALIZE` (yükleyici `pg_inherits.inhdetachpending` PG 14+ → `unsupported` değil, `detachPending: true`; ayırma diyaloğu yalnız FINALIZE sunar).
- **Yükleme:** sınır `pg_get_expr(relpartbound)` metninden ayrıştırılır (`FOR VALUES FROM (…) TO (…)`, `IN (…)`, `WITH (modulus m, remainder r)`, `DEFAULT`); değer listeleri mevcut `sqlscan` ile üst düzey virgüllerden bölünür. Ayrıştırılamayan metin → `unsupported` (+ salt okunur). Sabit nokta: `alter(loaded, loaded)` boş plan.
- **Hedef:** `partitions` koleksiyonu create bağlamı `schemaOid` + `tableOid` (üst tablo); bölüm düğümü `object = { kind: partition, oid: C }` yalnız **Bölüm sınırı…/Bölümü ayır…** eylemlerinde (Düzenle Table'ı açmaya devam eder) — `ddl_target_of` yanında `ddl_partition_target_of` ya da `DdlTarget.partition: Option<ObjectRef>` alanı (Plan 36 seçer; arayüz kimlik ayrıştırmaz).
- **Yenileme:** create/attach/detach sonrası üst tablonun `partitions` koleksiyonu ve (attach/detach'te) kaynak tablonun eski koleksiyonu yenilenir.

### 3.3 Risk özeti (Grup B)

| Risk | İfadeler |
|---|---|
| Safe | `CREATE TABLE … PARTITION OF`, sahip/yorum |
| Destructive | `ATTACH PARTITION`, `DETACH PARTITION [CONCURRENTLY|FINALIZE]`, sınır değiştirme |
| DataLoss | — (bölüm silme mevcut Table `DROP` akışıdır: DataLoss) |

---

## 4. Grup C — FDW, Foreign Server, User Mapping, Foreign Table, Publication, Subscription

### 4.1 Explorer düğümleri (Plan 38)

Yeni `NodeKind` / `Seg` / `Coll` (etiketler):

| Düğüm | Yol | Liste kaynağı | Not |
|---|---|---|---|
| Foreign Data Wrappers (`fdws`) → `fdw:N` | `db:D/fdws/fdw:N` | `pg_foreign_data_wrapper` | detay: handler adı |
| Foreign Servers (`fservers`) → `fsrv:N` | `…/fdw:N/fservers/fsrv:S` | `pg_foreign_server` (`srvfdw`) | detay: tür/sürüm |
| User Mappings (`umaps`) → `um:N` | `…/fsrv:S/umaps/um:U` | `pg_user_mappings` (seçenekler **okunmaz**) | etiket: rol adı ya da `PUBLIC` |
| Foreign Tables (`ftables`) → `ftbl:N` | `…/sch:S/ftables/ftbl:T` (+ `columns`) | `pg_class relkind='f'` + `pg_foreign_table` | detay: sunucu adı; kolonlar mevcut kolon listesi |
| Publications (`pubs`) → `pub:N` | `db:D/pubs/pub:N` | `pg_publication` | detay: `all tables` ya da tablo sayısı |
| Subscriptions (`subs`) → `sub:N` | `db:D/subs/sub:N` | `pg_subscription` (`subdbid` = bu DB) | detay: etkin/devre dışı |

- Her yeni tür için ana spec'teki düğüm eylemleri: **Özellikler** (props), **DDL** (Inspector), arama (Plan 32 `FRAGMENTS`'e ekleme: foreign table, publication, subscription, server), ikon. `Tables` koleksiyonu foreign table'ları listelemez (bugünkü davranış korunur; bölümlü tablonun yabancı bölümleri Plan 32'deki gibi `partitions` altında kalır).
- **Subscription okuma izni:** `pg_subscription` yalnız süper kullanıcıya (PG 16+: `pg_create_subscription` üyelerine sütun düzeyinde) açıktır; okunamazsa koleksiyon boş döner ve **Özellikler**'de "Görüntülemek için yetki gerekir" notu; hata toast'u yoktur. `subconninfo` **hiçbir zaman** Özellikler/DDL'e ham yazılmaz: `password=` değeri `********` ile maskelenir (Inspector DDL dahil).
- **User mapping seçenekleri:** Özellikler ve DDL'de `password` seçeneğinin değeri maskelenir; diğer seçenekler (`user`) görünür.

### 4.2 Diyaloglar (Plan 39 arka uç, Plan 40 ön yüz)

- **FDW (`ForeignDataWrapperDef`):** `name`, `owner`, `handler` (fonksiyon; isteğe bağlı), `validator` (fonksiyon; isteğe bağlı), `options`, `comment`. Create/alter `HANDLER`/`NO HANDLER`, `VALIDATOR`/`NO VALIDATOR` (Destructive: mevcut seçenekleri geçersiz kılabilir), `OPTIONS`, `RENAME`, `OWNER TO`. Süper kullanıcı gerekir (ipucu). Eklentiye ait FDW (ör. `postgres_fdw`) salt okunur; altındaki server/mapping düzenlenebilir. Açılır liste: `fdw_handlers` (`fdw_handler` dönen), `fdw_validators` (`(text[], oid)` imzalı).
- **Foreign Server (`ForeignServerDef`):** `name`, `fdw` (bağlamdan; değiştirilemez), `serverType`, `version`, `owner`, `options` (gizli `password` kuralı), `comment`. Alter: `VERSION`, `OPTIONS`, `RENAME`, `OWNER TO`; `TYPE` değişimi yeniden kurma gerektirir → desteklenmez (alan hatası). Drop: Destructive, CASCADE seçeneği DataLoss (mapping ve foreign table'lar düşer).
- **User Mapping (`UserMappingDef`):** `server` (bağlamdan), `role` (rol adı ya da `PUBLIC`; `CURRENT_USER` yazılmaz, rol adına çözülür), `options` (`password` gizli; yükleyici `passwordSet`), kimlik (`role`) değiştirilemez. Ad yoktur: Yeniden adlandır menüsü gösterilmez. Drop: Destructive. Yükleme: `pg_user_mappings` görünümü; seçenekleri okuma yetkisi yoksa (`umoptions` NULL) `unsupported` + salt okunur.
- **Foreign Table (`ForeignTableDef`):** `schema`, `name`, `owner`, `server` (açılır liste), `columns[{name, typeName, notNull, default, collation, options, original}]`, `options`, `comment`, `editable`, `unsupported`. Alter: `ADD/DROP/RENAME COLUMN`, `ALTER COLUMN TYPE` (Destructive değil, yabancı veri değişmez → Safe + not), `SET/DROP NOT NULL`, `SET/DROP DEFAULT`, `ALTER COLUMN … OPTIONS`, tablo `OPTIONS`, `RENAME`, `SET SCHEMA`, `OWNER TO`. `server` değişimi yeniden kurma → alan hatası (PG desteklemez). CHECK kısıtları, kalıtım ve `PARTITION OF` → `unsupported` (salt okunur). Drop: Destructive (yerel veri yoktur). **IMPORT FOREIGN SCHEMA** bu planda yok (backlog).
- **Publication (`PublicationDef`):** `name`, `owner`, `allTables` (yalnız create; alter'da değiştirilemez → alan hatası), `tables[{schema, name, columns?: string[] (PG 15+), where?: sql (PG 15+)}]`, `schemas: string[]` (PG 15+, `TABLES IN SCHEMA`), `publish {insert, update, delete, truncate (PG 11+)}`, `viaPartitionRoot` (PG 13+), `comment`. Alter: `ADD/DROP TABLE`, değişen süzgeç/kolon listesi için `SET TABLE` yerine ilgili tabloyu `DROP` + `ADD` (aynı transaction; Safe + not), `ADD/DROP TABLES IN SCHEMA`, `SET (publish = …, publish_via_partition_root = …)`, `RENAME`, `OWNER TO`. Drop: Destructive (abonelikler kopar). Açılır liste: tablolar (mevcut referans verilebilir tablolar lookup'ı), şemalar. `FOR ALL TABLES` süper kullanıcı ister (ipucu). PG 18'deki `publish_generated_columns` → `unsupported` değil, okunur ve korunur alan (`publishGeneratedColumns: Option`; yalnız PG 18+ yazılır).
- **Subscription (`SubscriptionDef`):** `name`, `owner`, `connection` (gizli; yükleyici `password=` hariç bağlantı dizgesini `connectionMasked` olarak, parola varlığını `passwordSet` olarak döndürür; düzenlemede kullanıcı tam dizgeyi yeniden girer — kısmi düzenleme yok), `publications: string[]`, `enabled`, `slotName` (boş = abonelik adı; `NONE` ayrı bayrak `noSlot`), yalnız create: `createSlot`, `copyData`, `connect`; her ikisinde: `synchronousCommit`, `binary` (PG 14+), `streaming` (`off|on`, PG 16+ `parallel`), `twoPhase` (PG 15+, yalnız create), `disableOnError` (PG 15+), `origin` (PG 16+ `any|none`), `passwordRequired`/`runAsOwner` (PG 16+), `failover` (PG 17+), `comment`.
  - Create: `CREATE SUBSCRIPTION … CONNECTION '…' PUBLICATION … WITH (…)`; `createSlot = true` (varsayılan) → **transaction dışı**; önizlemede bağlantı `CONNECTION '********'` (yalnız `password` maskeli değil, **tüm dizge** maskelenir — kullanıcı adı/ana makine de sırdır kabul edilir).
  - Alter: `CONNECTION`, `SET PUBLICATION … WITH (refresh = false)` + ayrı isteğe bağlı **REFRESH PUBLICATION** eylemi (transaction dışı, Destructive: `copy_data` ile yeni tablolar kopyalanır), `ENABLE`/`DISABLE` (DISABLE Destructive: çoğaltma durur), `SET (…)`, `RENAME`, `OWNER TO`.
  - Drop: `DROP SUBSCRIPTION` — slot bağlıysa **transaction dışı**, **Destructive** (uzak slot silinir); bağlantı kurulamıyorsa PG hatası → ipucu: "Önce DISABLE edip `slot_name = NONE` ayarlayın (uzak slotu elle silmeniz gerekir)" ve diyalogda **"Slotu bırakarak sil"** seçeneği (aynı akışın üç ifadesi: `DISABLE`, `SET (slot_name = NONE)`, `DROP`).
  - Süper kullanıcı (PG 16+: `pg_create_subscription`) gerekir; 42501 ipucu.
- **Gizlilik testi:** her gizli alan için insta anlık görüntüsü önizlemede değerin bulunmadığını, `Debug`/`tracing` biçiminde bulunmadığını ve hata iletisinde bulunmadığını doğrular (Secret kalıbı).

### 4.3 Risk özeti (Grup C)

| Risk | İfadeler |
|---|---|
| Safe | Tüm `CREATE`'ler, `OPTIONS` değişikliği, `RENAME`, `OWNER TO`, `COMMENT`, foreign table kolon değişiklikleri, publication tablo/şema ekleme, subscription `ENABLE`, `SET (…)`, `CONNECTION` |
| Destructive | FDW `HANDLER`/`VALIDATOR` değişimi, tüm `DROP`'lar (CASCADE'siz), publication tablo/şema çıkarma, subscription `DISABLE`, `REFRESH PUBLICATION`, `SET PUBLICATION` |
| DataLoss | `DROP … CASCADE` (FDW/server) |

---

## 5. Arayüz

- Ana spec §4 aynen: sağ panelde sekme, SQL önizleme, risk rozeti, onay, kilit/üretim koruması, "Query Tool'da aç" (gizli değer içeren planlarda kapalı — Plan 15 parola kuralı).
- Yeni formlar tembel parçalardadır (`PolicyForm`, `TablespaceForm`, `PartitionForm`, `DetachPartitionDialog`, `FdwForm`, `ForeignServerForm`, `UserMappingForm`, `ForeignTableForm`, `PublicationForm`, `SubscriptionForm`); metinler `src/app/strings-*.ts` tembel modüllerinde. Ortak `OptionsEditor` (genel seçenek satır düzeyi; gizli anahtar için parola alanı).
- Serbest SQL alanları (Policy `USING`/`WITH CHECK`, bölüm sınır değerleri, publication `WHERE`) tek satır ya da küçük `textarea`; CodeMirror zorunlu değil.
- Gizli alanlar: parola alanı `type=password`, değer yalnız form yerel sinyali + denetleyici `secrets` haritası (Plan 15 kalıbı); sekme kapanınca silinir; `localStorage`'a yazılmaz.
- Ana paket: her ön yüz planı `ddl-kinds.ts` ve Explorer menü/ikon değişikliğini **aynı görevde** adı konmuş telafiyle karşılar; `__vite__mapDeps` ve preload listesi ölçülür.

## 6. Güvenlik

- Tanımlayıcılar `quote_ident`/`qualified`, değerler `quote_literal`; serbest SQL kapısı; gizli değerler `Secret`.
- Politika ifadeleri, sınır değerleri, seçenek değerleri, bağlantı dizgesi ve konumlar `tracing`'e yazılmaz; yalnız işlem/tür/ifade sayısı/süre/kod.
- Tablespace `location` ve subscription bağlantısı gibi sunucu tarafı yollar/adresler hata toast'unda gösterilmez (PG iletisi bunları içerirse ileti olduğu gibi yalnız sekmedeki hata alanında gösterilir; toast'ta özet).

## 7. Test stratejisi

- Saf üreticiler: insta anlık görüntüleri (create, alter varyantları, yeniden kurma, drop, hostile adlar/değerler, gizli değer maskeleme, sürüm kapıları).
- Docker PG 10–18 gidiş-dönüş (`scripts/test-matrix.ps1 -Versions 10,…,18 -SkipPerf`): `create → load → eşit`, `alter(loaded, loaded)` boş plan, `alter(old, new) → load → new`.
  - Policy: RESTRICTIVE, rol listesi, ifade kaldırma → yeniden kurma, tablo RLS bayrakları.
  - Tablespace: konteyner içinde `postgres` kullanıcısına ait geçici dizin (test önce `mkdir`); transaction dışı create + comment; boş olmayan drop → 55000 ipucu.
  - Partition: range/list/hash/default (11+), MINVALUE/MAXVALUE, alt bölümleme, attach/detach, sınır değiştirme, `CONCURRENTLY` (14+), sınır ayrıştırma sabit noktası.
  - FDW: `postgres_fdw` ile (eklentiye ait FDW salt okunur; kendi server/mapping/foreign table'ımız loopback'e), user mapping parolasının yükte dönmediği.
  - Publication: tablo/şema (15+), kolon listesi ve `WHERE` (15+), `publish` seçenekleri.
  - Subscription: aynı konteynerde ikinci veritabanına (`connect = false`, `create_slot = false` ile transaction içi create; slotlu varyant `wal_level = logical` gerektirir → matris betiği bu sürümlerde konteyneri `-c wal_level=logical` ile başlatır; Plan 39 betiği genişletir).
- Explorer düğümleri (Plan 38): her yeni koleksiyon için liste/props/DDL testleri ve Plan 32 arama zinciri testi (`search.rs`) genişletilir.
- Ön yüz: vitest model testleri (doğrulama, fark, gizli değer yaşam döngüsü), reaktivite testi (`designer-reactivity.test.ts` kalıbı); GUI elle kontrol `docs/smoke-test.md` `[—]` maddeleri.

## 8. Kapsam dışı (backlog)

- `IMPORT FOREIGN SCHEMA` diyaloğu; foreign table CHECK kısıtları ve kalıtım; FDW `HANDLER` için yeni C fonksiyonu yazımı.
- Subscription'da bağlantı dizgesinin parça parça (ana makine/port/kullanıcı) düzenlenmesi; `ALTER SUBSCRIPTION … SKIP` (PG 15+); çoğaltma durum izleme (Dashboard işi).
- Yerinde tablespace (`allow_in_place_tablespaces`); tablespace'teki nesneleri toplu taşıma (`ALTER … SET TABLESPACE` / `ALTER TABLESPACE … MOVE` yoktur, PG'de `ALTER TABLE ALL IN TABLESPACE … SET TABLESPACE`).
- Bölüm bölme/birleştirme (`SPLIT/MERGE PARTITION`; PG 17 geliştirme sürecinde geri çekildi); kalıtım (`INHERITS`) düzenleme; EXCLUDE kısıtları.
- Partition (Plan 36 sonrası, ileride açılabilir): ayrıştırılamayan sınırlı ya da bilinmeyen stratejili bölüm salt okunurdur ve ayırma da kapalıdır (gerekçede yalnız ad yazdığı hâlde); foreign table ve bölümü olan bölümlü tablo `attachable_tables` listesinde yer almaz; ATTACH/DETACH ipucu eşleşmesi ifade metni alt dizgesiyledir (tırnaklı adda yanlış ipucu çıkabilir, kozmetik).
- Event trigger, rule, collation, cast, operator, FTS nesneleri, language, aggregate (Explorer'da yok).

---

## 9. Plan 34 sözleşme notları (Plan 35 için)

Aşağıdaki maddeler **gönderilen** davranışı anlatır (kod ve Docker PG 10–18 testleriyle doğrulandı).

### 9.1 Model alanları (camelCase) ve salt okunurluk

- `ObjectDef` etiketi `kind`: `policy`, `tablespace`. `ObjectKind::Tablespace.server_scoped() = true` (ön yüzde `isServerScoped("tablespace") = true`).
- `PolicyDef`: `schema, table, name, command (all|select|insert|update|delete), permissive (true = PERMISSIVE), roles[] (boş = PUBLIC), using, withCheck (serbest SQL, isteğe bağlı), comment, editable, unsupported, tableRls`. `tableRls` **salt okunur** `{enabled, forced}` bilgisidir: üretici yok sayar, yalnız yükleyici ve `defaults` doldurur (`#[serde(default)]`; gönderilmezse `null`). Yükleyici `roles`'u **ada göre sıralı** döndürür; sıra farkı değişiklik sayılmaz.
- `TablespaceDef`: `name, owner, location, options{seqPageCost, randomPageCost, effectiveIoConcurrency, maintenanceIoConcurrency} (her biri sayısal metin ya da null), comment, editable, unsupported`.
- `TableDef.rowSecurity: {enabled, forced}` (`#[serde(default)]`; yeni tabloda `false/false`). Yükleyici `relrowsecurity`/`relforcerowsecurity` okur; bölüm tablosunda ve INHERITS çocuğunda da düzenlenebilir.
- **`editable` ve `unsupported` için serde varsayılanı YOKTUR** (iki yeni tür dahil): yeni tanım `editable: true, unsupported: []` ile gönderilmelidir (`ddl_defaults` bunu doldurur; arayüz alanları düşürmemeli).
- **R5:** `unsupported` doluysa yükleyici `editable = false` döndürür. Politika: eklentiye ait tablo, sistem şeması, sıradan/bölümlü dışı ilişki, bilinmeyen komut, geçersiz UTF-8. Tablespace: `pg_default`/`pg_global` (`["Sistem tablespace'i"]`, `location = ""`), bilinmeyen `spcoptions` anahtarı (`seçenek <anahtar>`), geçersiz UTF-8. Salt okunur kapısı `alter`/`drop`'u bağlantıdan önce reddeder (kodsuz `invalid`, ad/konum içermeyen sabit gerekçe); `create` etkilenmez. Arayüz `editable = false` nesneyi salt okunur gösterir.
- **Tablespace ACL'sinde yalnız `pg_global` salt okunurdur** (`pg_default`'a yetki verilebilir); DDL için ikisi de salt okunurdur.

### 9.2 Alan yolları (`FieldError.path`)

- **Policy:** `name`, `schema`, `table` (alter'da tablo değişimi), `roles[i]` (çıplak `roles` ve `command` yolu üretilmez; kural ihlalleri `using`/`withCheck` yoluna düşer), `using`, `withCheck`, `comment`, `object` (salt okunur), `old.*` (bayat/geçersiz yüklü tanım). Kurallar: `insert` → `using` yasak; `select`/`delete` → `withCheck` yasak; rol öğeleri geçerli ad, yinelenen yok, `public` öğe olarak reddedilir ("PUBLIC için listeyi boş bırakın"); `cascade` yok sayılır (drop modeli `cascade: false`, Plan 35 kutuyu göstermez).
- **Tablespace:** `name` (`pg_` öneki dahil), `location` (boş değil, NUL ve tek tırnak (`'`) yok, mutlak yol, ≤ 1024 bayt; alter'da değişim; PG tek tırnağı her zaman reddeder), `options.seqPageCost`, `options.randomPageCost` (sonlu ondalık ≥ 0), `options.effectiveIoConcurrency`, `options.maintenanceIoConcurrency` (0–1000 tamsayı; maintenance PG < 13'te sürüm kapısı alan hatası), `owner`, `comment`, `object`, `old.*`.
- **Tablo RLS:** `rowSecurity.enabled`/`rowSecurity.forced` iki boolean'dır ve doğrulanmaz; bu yollarda alan hatası **üretilmez** (Plan 35 tablo formu yollarına eklemesi gerekmez).
- **ACL (Tablespace):** ayrıcalık kümesi yalnız `{Create}`, nesne ifadesi `GRANT … ON TABLESPACE <ad>`; şema alanı yok. **Varsayılan yetkiler yapısal olarak tablespace'i hedefleyemez** (istek biçimi tür taşımaz; Plan 35 menüde göstermemeli).

### 9.3 Kurallar: yeniden kurma, risk, kapılar

- **Politika alter:** (1) ad değiştiyse `ALTER POLICY … RENAME TO`; (2) `command`/`permissive` değişti **ya da** `using`/`withCheck` `Some → None` oldu → aynı transaction'da `DROP POLICY` + `CREATE POLICY` (**Destructive**, not "Politika aynı transaction içinde yeniden kurulur.", yorum yeniden yazılır), aksi hâlde tek `ALTER POLICY … [TO] [USING] [WITH CHECK]` (Safe); (3) `COMMENT ON POLICY`. Yalnız ad değişimi tek `RENAME`; rol sırası farkı boş plan. `comment: Some("")` yorumsuz sayılır (`None` ile eşdeğer; Tablespace/Table ile tutarlı).
- **Bayat denetimi (policy):** nesne denetimi (`plan_changed`) yüklü tanımın **tamamını** karşılaştırır; salt okunur `tableRls` dahildir. Tablonun RLS bayrakları değişirse (ör. tablo sekmesinden `ENABLE`/`DISABLE`/`FORCE`) o tablonun açık policy sekmelerinin sonraki alter/drop'u `plan_changed` döner; **Plan 35 tablo RLS değişiminden sonra policy sekmesini yeniden yüklemelidir.**
- **Oid:** yeniden kurma yeni oid üretir; `created` yeni `pg_policy` oid'iyle döner (sekme `object`'i güncellenmeli). **Yeniden adlandırma da `created` döndürür.** Yalnız yorum/ifade değişimi `created` döndürmez.
- **Risk:** Safe: create, `ALTER POLICY`, rename, comment, tablespace create/rename/owner/SET/RESET/comment, tablo `DISABLE`/`NO FORCE ROW LEVEL SECURITY`. **Destructive:** policy drop ve yeniden kurma, tablo `ENABLE`/`FORCE ROW LEVEL SECURITY` (+ not "Politikası olmayan tabloda sahibi dışındakiler hiçbir satır görmez."), `DROP TABLESPACE`. DataLoss yok.
- **Onay kapısı (ledger kararı):** Destructive planlar yalnız `confirmation_required` ister. **Üretimde ek ad onayı yoktur**; `production_confirmation_required` mevcut kapıda yalnız DataLoss içindir (Policy/RLS/Tablespace hiçbiri DataLoss değil). Arayüz Destructive için yalnız onay kutusunu göstermelidir.
- **Transaction:** Policy ve tablo RLS ifadeleri transaction içinde. **Tüm Tablespace ifadeleri transaction dışıdır** (`stmt_nt`): create + comment arasında kısmi uygulama olabilir (`failed.created` dolu, `committed = 1`; "Önceki ifadeler uygulandı…" ipucu). Yerinde (boş `location`) tablespace desteklenmez. Alter sırası: `RENAME`, `SET`, `RESET`, `COMMENT`, `OWNER TO` (en sonda); `location` create-only.
- **Uyarılar:** politika `using` ve `withCheck` ikisi de boşsa "İfadesiz politika: PERMISSIVE tüm satırlara izin verir, RESTRICTIVE etkisizdir."; `tableRls.enabled = false` ise "Tabloda satır güvenliği kapalı; politika etkili olmaz."; tablo `DISABLE`/`NO FORCE` → "Tablonun politikaları devre dışı kalır.". **Alter'da politika uyarıları yalnız plan boş değilse** eklenir (sabit nokta: boş plan, boş uyarı). `tableRls` create'te `defaults`'tan, alter'da yükleyiciden gelir; üretici bunu yazmaz.

### 9.4 `defaults(kind)` ve hedefler

- `defaults(Policy)`: bağlam `tableOid`'inden `schema`/`table`, `command = all`, `permissive = true`, `roles = []`, ifadeler `null`, `tableRls` = tablonun **güncel** bayrakları. `tableOid` yoksa kodsuz `invalid`; tablo yok/görünümse `not_found`. `defaults(Tablespace)`: `owner = null`, `location = ""`, seçenekler boş; bağlam verilirse `invalid`.
- Hedef: `…/tbl:T/pol:P` → `object(Policy, P)`, `refreshNodeId = …/tbl:T/policies`; `…/tbl:T/policies` → `create(Policy, schemaOid, tableOid)` (bölüm tablosu dahil). `tablespaces` → `create(Tablespace)`, `ts:N` → `object(Tablespace, N)`; `databaseOid = null`, `refreshNodeId = srv:U/tablespaces`. Tablespace yükleme/uygulama/ACL bakım veritabanında çalışır.
- `identity_changed`: Policy için ad değişimi **ya da** yeniden kurma, Tablespace için ad.

### 9.5 İpuçları (SQLSTATE → ipucu)

- `55000` + `DROP TABLESPACE`: "Tablespace boş değil; önce içindeki nesneleri başka tablespace'e taşıyın (başka veritabanlarındaki nesneler de sayılır)." Hiçbir şey değişmez.
- `42501` + `CREATE TABLESPACE`: "Tablespace oluşturmak süper kullanıcı yetkisi ister; konum dizini de PostgreSQL hizmet kullanıcısına ait olmalı." (genel yetki ipucunun yerine; 42501 dizin sahipliğinden de gelebilir).
- `58P01`, `42P17` + `CREATE TABLESPACE`: "Konum sunucuda var olan, PostgreSQL hizmet kullanıcısına ait boş bir dizin olmalı."
- `55006` + `CREATE TABLESPACE`: "Bu konum zaten bir tablespace tarafından kullanılıyor; boş ve kullanılmayan bir dizin seçin."
- `22023` + `CREATE TABLESPACE` (çoğunlukla `WITH` seçenek sınırları): kural ipucu **yoktur**, PG iletisi olduğu gibi kalır.
- `42710` + `CREATE POLICY` (aynı ad): özel ipucu yoktur, PG iletisi geçerlidir.
- **Kural ipuçları yalnız PG kendi ipucunu vermediyse uygulanır**; PG ipucu varsa olduğu gibi kalır. Transaction dışı ifadelerden sonra hata → kısmi uygulama ipucu (yalnız başka ipucu yoksa).

### 9.6 Güvenlik, günlük ve test notları

- İfade, ad, konum (`location`) ve SQL `tracing`'e, hata iletisine ya da diske yazılmaz (yakalama testi: `logging.rs`, Policy `USING` + tablespace adı/konumu). Salt okunur gerekçeleri ad/konum içermez.
- Serbest SQL (`using`, `withCheck`) `check_free_sql` + `wrap` + `composed_is_clean` kapısından geçer; sıradan `'…'` içinde `\` → `E'…'`. Yüklenen ifade düz `\` içeriyorsa yalnız o ifadeyi yazan düzenleme reddedilir.
- Tablespace Docker testleri `PGT_TEST_TS_ROOT` ister (matris betiği sağlar; yoksa atlanır): ts1, ts2, ts3, ts5 `policy_roundtrip`, ts6 `acl_roundtrip`, ts7 `manager`, ts8 `logging`.
- Şema karşılaştırması tablo RLS bayraklarını **karşılaştırır**, politikaları karşılaştırmaz (backlog).

### 9.7 Ön yüz bütçe notu

- Plan 34 sonunda ana paket **126,582 B**, başlangıç **154,255 B** (tavanlarla birebir, değişmedi). Plan 35'in `ddl-kinds.ts` ve Explorer menü/ikon kancası ek bayt getirir; **adı konmuş bir telafi gerekir** (yalnız tembel kullanılan komutları `src/ipc/commands.ts`'ten tembel modüle taşımak). Tavanlar yükseltilmez.
- Plan 34'te yalnızca tembel dosyalar uyum için değişti (`src/features/schema-diff` **değişmedi**): geçici `formMissing` bileşeni (`DesignerView.tsx`) ve metni (`src/app/strings-designer.ts`; Plan 35 gerçek formlarla değiştirir), boş `POLICY_PATHS`/`TABLESPACE_PATHS` (`forms-model.ts`; FORM_PATHS bütünlük denetimi Plan 35'te yeni yollarla tamamlanmalı), `drop-model.ts` (`cascade=false`, `force=false`), `designer-model.ts` (`KIND_LABELS`, `isServerScoped` Tablespace'i de kapsar, `defName`).

---

## 10. Plan 35 notları (Grup A ön yüzü)

- **Bütçe telafisi ve ölçümler:** `ddl-kinds.ts` ek baytı yalnız `qtMore` taşımasıyla karşılanamadı (10 B eksik); `qtMore` ve `qtCancel` birlikte tembel `src/ipc/qt-more-commands.ts`'e taşındı. Tavanlar yalnız düştü: ana paket 126582 → 126553 B, başlangıç 154255 → 154226 B. PolicyForm 12514 B, TablespaceForm 6147 B; tablo RLS metinleri ortak tembel TypeInput parçasında.
- **Komut kuralı aynalaması:** `insert` → USING kapalı, `select`/`delete` → WITH CHECK kapalı; komut değişince yasak hâle gelen ifade temizlenir. Diğer kurallar arka ucun alan hatasıyla gösterilir.
- **Yeniden kurma notu:** komut/tür değişimi ya da ifade kaldırma istemcide hesaplanır ve yalnız bilgi notu olarak gösterilir; seçim kutusu yoktur (arka uç otomatik yeniden kurar, Yıkıcı onay mevcut kapıdan gelir).
- **Politika sekmesi yenileme:** tablo alter'ı RLS/ad/şemayı değiştirirse aynı sunucu + veritabanındaki o tablonun politika sekmeleri yenilenir; kirli ya da uygulamadaki sekmede taslak ezilmez, `needsReload` işaretlenir.
- **FORCE notu:** FORCE açılırken taslakta RLS kapalıysa (ve ENABLE açılmıyorsa) Yıkıcı notu yerine bilgi notu "FORCE yalnız satır güvenliği açıkken etkilidir." (`forceWithoutEnable`) gösterilir; arka uç risk sınıflaması değişmez.
- **Biçim uyarıları engellemez:** tablespace seçenek uyarıları (sayı değil, negatif, 0–1000 dışı) gönderimi durdurmaz; yetkili doğrulama arka uç alan hatasıdır. Konum yalnız create'te düzenlenir.
- **Backlog:** politika ifadeleri için CodeMirror (şimdilik `textarea`); şema karşılaştırmada politikalar.

---

## 11. Plan 36 sözleşme notları (Plan 37 için)

Aşağıdaki maddeler **gönderilen** davranışı anlatır (kod ve Docker PG 10–18 testleriyle doğrulandı); §3'teki taslaktan ayrıldığı yerde bu bölüm geçerlidir.

### 11.1 Model alanları (camelCase) ve salt okunurluk

- `ObjectDef` etiketi `kind`: `partition`. `PartitionDef`: `parentSchema, parentTable, strategy (range|list|hash), keyCount, mode (create|attach), schema, name, bound, partitionBy, tablespace, owner, comment, detachPending, editable, unsupported`.
- **Salt okunurlar:** `strategy`, `keyCount`, `detachPending` üst tablodan/katalogdan gelir; istemcinin gönderdiği değerler yetkili değildir (alter, sınırı **yüklü (`old`)** `strategy`/`keyCount` ile doğrular). `detachPending` için `#[serde(default)]` vardır.
- **`editable` ve `unsupported` için serde varsayılanı YOKTUR:** yeni tanım `editable: true, unsupported: []` ile gönderilmelidir (`ddl_defaults` doldurur; arayüz alanları düşürmemeli).
- `PartitionBound` (etiket `kind`): `range { from: BoundValue[], to: BoundValue[] }`, `list { values: string[] }`, `hash { modulus, remainder }`, `default`. `BoundValue` (etiket `kind`): `value { sql }` | `minvalue` | `maxvalue`.
- **R5:** yükleyici `unsupported` doluysa `editable = false` döndürür: eklentiye ait tablo (`EXTENSION … (eklentiye ait)`), bilinmeyen bölümleme stratejisi, ayrıştırılamayan sınır ifadesi, geçersiz UTF-8. Salt okunur kapısı `alter`/`drop`'u bağlantıdan önce reddeder (kodsuz `invalid`, ad/sınır içermeyen sabit gerekçe). **Yükleyici `mode = create` ve `partitionBy`/`tablespace`/`owner`/`comment = null` döndürür** (bu özellikler Tablo diyaloğundadır); alter'da bunlar **yazılamaz**.
- `detachPending` (PG 14+ `pg_inherits.inhdetachpending`) `unsupported` **değildir**; ayırma yalnız `FINALIZE` olarak yapılır.
- Hedef eşlemesi bölüm düğümünü hâlâ `Table` olarak verir (`object`); `Partition` yalnız `partition` alanındadır (§11.4).

### 11.2 Alan yolları (`FieldError.path`)

- `parentSchema`, `parentTable`, `schema`, `name` (geçerli ad), `bound`, `bound.from`, `bound.to` (uzunluk ≠ `keyCount`), `bound.from[i]`, `bound.to[i]`, `bound.values`, `bound.values[i]`, `bound.modulus`, `bound.remainder`, `partitionBy.strategy`, `partitionBy.keys`, `partitionBy.keys[j]`, `tablespace`, `owner`, `comment`, `concurrently`, `cascade`, `object` (salt okunur, yarım kalmış ayırma, sürüm kapısı), `old.*` (bayat/geçersiz yüklü tanım: `old.parentSchema`, `old.parentTable`, `old.schema`, `old.name`).
- **Sınır kuralları:** `range` için `from`/`to` uzunluğu = `keyCount`; `MINVALUE`/`MAXVALUE` sonrasında aynı tarafta yalnız aynı anahtar sözcük (`bound.<taraf>[i]`); `list` en az bir değer (`NULL` serbest); `hash` `modulus ≥ 1`, `remainder < modulus`; strateji–sınır uyuşmazlığı `bound` yolunda. **HASH üst tabloda `default` sınırı da `bound` uyuşmazlığıdır** (§3.1 taslağında yazılı değildi; PG zaten reddeder). `default` PG < 11 → `bound` sürüm hatası; `hash` PG < 11 → `bound` sürüm hatası; `partitionBy` HASH PG < 11 → `partitionBy.strategy`; PG < 10 → `object` sürüm hatası. Serbest SQL değerler `check_free_sql` ve nihai sınır metni `composed_is_clean` kapısından geçer.
- **PG 10–11 sınır değerleri ham yazılır:** bu sürümlerin sınır dilbilgisi yalnız sabit kabul eder (parantezli ifade yok). PG ≥ 12'de değer `(<sql>\n)` ile sarılır; PG < 12'de kırpılmış ham metin yazılır ve değer **üst düzeyde virgül içeremez** (`bound.from[i]` / `bound.to[i]` / `bound.values[i]`: "PG 12 öncesinde sınır değeri tek bir sabit olmalı."). `--` ile biten değer her sürümde reddedilir. **PG < 12'de değer yorum da içeremez** (`--` ve `/*`; tırnak içindeki `--` serbest): "PG 12 öncesinde sınır değeri yorum içeremez." (yorum, sonraki değeri ya da `)`'i yutabilir). Doğrulama, yazılacak kırpılmış (`trim()`) metne yapılır; doğrulanan ve yazılan metin birebir aynıdır. Nihai denetim sınır metninde değil **tam CREATE/ATTACH ifadesinde** yapılır (`composed_is_clean` + satır yorumuyla bitmeme; yol `bound`). **`MINVALUE`/`MAXVALUE`** (kırpılmış, büyük/küçük harf duyarsız) serbest SQL değeri olarak verilemez, o değerin yolunda alan hatası: range'de "MINVALUE/MAXVALUE için anahtar sözcük seçeneğini kullanın.", list'te "MINVALUE/MAXVALUE yalnız RANGE sınırında kullanılır.". `alter(loaded, loaded)` her sürümde boş plandır (sabit nokta).
- **Alter yasakları:** kimlik (`parentSchema`, `parentTable`, `schema`, `name`) değişimi → o yolda alan hatası ("Bölümün adı, şeması ve üst tablosu değiştirilemez; yeniden adlandırma Tablo diyaloğundadır."); `owner`, `comment`, `tablespace`, `partitionBy` dolu ya da değişmişse → o yolda alan hatası ("Bölüm tablosunun özellikleri Tablo diyaloğunda düzenlenir."). Yani **alter'a bu dört alan `null` gönderilmelidir**; arayüz Partition sekmesinde bunları alter'da göstermemeli.

### 11.3 Kurallar: eylemler, risk, kapılar

- **create (`mode = create`):** `CREATE TABLE s.n PARTITION OF ps.pt FOR VALUES … | DEFAULT [PARTITION BY …] [TABLESPACE …]` (Safe), sonra `COMMENT ON TABLE` (`Some("")` yorumsuz sayılır), `ALTER TABLE … OWNER TO` (**en sonda**). `partitionBy`/`tablespace`/`owner`/`comment` yalnız `create` modunda doğrulanır ve yazılır; `attach`'te yok sayılır.
- **attach (`mode = attach`):** tek `ALTER TABLE ps.pt ATTACH PARTITION s.n …` (**Destructive**). Not sürüme göre: PG ≥ 12 "Tablo taranır ve sınır doğrulanır; üst tabloda SHARE UPDATE EXCLUSIVE kilidi alınır.", PG < 12 aynı metin `ACCESS EXCLUSIVE` ile. `name` **var olan** tablodur; `schema` onun şemasıdır.
- **alter (sınır değiştirme):** yalnız `bound` farkı → aynı transaction'da `DETACH PARTITION` (Destructive, not "Bölüm aynı transaction içinde yeniden bağlanır; tablo yeniden taranır.") + `ATTACH PARTITION` (Destructive, sürüme göre kilit notu). Sınır aynıysa **boş plan** — **yalnız üretici düzeyinde** (salt okunur ya da `detachPending` eski tanımda da). Yönetici üzerinden: `editable = false` eski tanımda Alter/Drop salt okunur kapısında reddedilir (boş plan dönmez); boş plan yönetici üzerinden yalnız `detachPending` (düzenlenebilir) eski tanım için geçerlidir. Salt okunur eski tanım → `object` hatası ("Bu bölüm salt okunurdur; değiştirilemez ve ayrılamaz."); `detachPending` → `object` hatası ("Yarım kalmış ayırma var; önce ayırmayı tamamlayın (FINALIZE).").
- **drop (= ayırma; tablo silinmez):** `PlanRequest::Drop`'un partition anlamıdır. **`PlanRequest::Drop.concurrently`** (`serde(default)`, TS'de isteğe bağlı) yalnız Partition'da geçerlidir; diğer türlerde `concurrently` alan hatası ("CONCURRENTLY yalnızca bölüm ayırmada kullanılır."). `cascade` Partition'da `cascade` alan hatası ("Bölüm ayırmada CASCADE yoktur."); `force` `force` alan hatası. Normal ayırma: `ALTER TABLE ps.pt DETACH PARTITION s.n` (transaction içinde, Destructive, not "Satırlar üst tablodan görünmez olur; tablo silinmez."). `concurrently = true`: `… CONCURRENTLY` (**transaction dışı**; PG < 14 → `concurrently` sürüm hatası). `detachPending = true`: `… FINALIZE` (transaction dışı; `concurrently` yok sayılır; PG < 14 → `object` sürüm hatası). Tablo ve satırlar korunur; DataLoss yoktur.
- **Onay kapısı:** tüm Partition planları en çok Destructive'dir → yalnız `confirmation_required`; **üretimde ek ad onayı yoktur** (`production_confirmation_required` yalnız DataLoss içindir). Arayüz yalnız onay kutusunu göstermelidir.
- **Bayat denetimi:** alter/drop `old` tanımın **tamamını** karşılaştırır (`plan_changed`); `identity_changed(Partition, Partition) = false` (alter ad/şema değiştirmez).
- **`created`:** create ve attach `created = Some(çocuk tablonun oid'i)` döndürür (`find_created`: şema + ad, `relispartition`); alter ve drop `created` döndürmez.
- **Transaction dışı ifadeler** (`CONCURRENTLY`/`FINALIZE`) tek ifadedir; hata verirse `committed = 0`, hiçbir şey değişmemiştir.
- **Uyarılar:** Partition planları uyarı üretmez (`warnings` boş).

### 11.4 `defaults(kind)`, hedefler ve yenileme

- `defaults(Partition)`: bağlam `tableOid` (üst tablo) ister; yoksa kodsuz `invalid`; tablo yoksa `not_found`; bölümlenmemiş tabloda `invalid` ("Bölüm yalnızca bölümlenmiş bir tablo altında oluşturulabilir."). Dönen: `parentSchema`/`parentTable`, `schema = parentSchema`, `name = ""`, `mode = create`, `strategy`/`keyCount` üst tablodan, `bound` boş iskelet (`range` → `keyCount` adet boş `value`; `list` → `[]`; `hash` → `modulus 1, remainder 0`), `partitionBy/tablespace/owner/comment = null`, `detachPending = false`, `editable = true`, `unsupported = []`.
- **Hedef:** `…/tbl:P/partitions` → `create(Partition, schemaOid, tableOid = P)`, `refreshNodeId = …/tbl:P/partitions`. Bölüm düğümü `…/tbl:P/tbl:C` → `object = Table C` (Düzenle Tablo'yu açmaya devam eder) **ve** yeni `DdlTarget.partition = { kind: partition, oid: C }` ("Bölüm sınırı…" ve "Bölümü ayır…" eylemleri bunu kullanır); `refreshNodeId = …/tbl:P/partitions`. `partition` alanı yalnız gerçek bölüm düğümünde yazılır (aksi hâlde JSON'da yoktur; TS'de isteğe bağlı). Alt bölümde üst tablo o bölümdür. Arayüz kimlik ayrıştırmaz.
- **İkinci yenileme hedefi `tablesRefreshNodeId`** (isteğe bağlı): bölüm düğümünde ve `partitions` koleksiyonunda yoldaki en yakın şemanın `tables` koleksiyonudur. **Yenileme kuralı:** create/attach/detach sonrası `refreshNodeId` (üst tablonun `partitions` koleksiyonu) **ve** `tablesRefreshNodeId` yenilenir. **Bilinen sınır:** üst tablodan farklı şemaya bağlanan/ayrılan tablonun şema koleksiyonu yenilenmez (hedef yalnız üst tablonun şemasını bilir; kimlik ayrıştırma yasaktır).
- **`AttachableTables`:** `LookupRequest::AttachableTables { parent_oid }` (JSON `{"kind":"attachable_tables","parent_oid":N}`; `server_wide = false`). Dönüş `LookupItem` listesidir: `value` = tablo adı, `detail` = şema adı; şema ve ada göre sıralı, en çok 5000. Dahil: bölüm olmayan, kalıtım ilişkisi olmayan, sıradan ya da bölümlü tablolar (bölümsüz bölümlü tablo dahil). Hariç: üst tablonun kendisi ve ataları (alt bölümlemede), bölümler, `pg_inherits`'te çocuk ya da üst olanlar, `pg_*`/`information_schema` şemaları, eklentiye ait tablolar.
- **Yükleme:** sınır `pg_get_expr(relpartbound)` metninden ayrıştırılır (`FROM … TO …`, `IN (…)`, `WITH (modulus m, remainder r)`, `DEFAULT`); değerler `sqlscan` ile üst düzey virgüllerden bölünür. Yüklenen değerler **sunucunun kanonik sabit metnidir** (`pg_get_expr`; ör. `1`, `'2024-01-01'`), `wrap`'lı DEĞİL; Plan 37 wrap soymaya çalışmamalıdır. `wrap` yalnız üretilen ifadededir (PG ≥ 12: `(<sql>\n)`); MINVALUE/MAXVALUE, hash ve default tam sabit noktadır.

### 11.5 İpuçları (SQLSTATE → ipucu)

Sabitler `crates/pgt-ddl/src/manager.rs`'tedir (`PARTITION_*_HINT`); SQLSTATE'ler Docker'da PG 10–18'de gözlenip testte sabitlendi (tahmin yok).

- `23514` + ifadede ` ATTACH PARTITION ` (bağlanan tabloda sınır dışı satır; sınır değiştirmenin yeniden bağlamasında da): "Tablodaki bazı satırlar bölüm sınırının dışında; önce bu satırları taşıyın ya da sınırı genişletin."
- `42P17` + ` ATTACH PARTITION ` (çakışan sınır): "Sınır başka bir bölümün sınırıyla çakışıyor (ya da tablo bölüm sınırıyla uyumsuz)."
- `55000` + ` DETACH PARTITION ` ve ifade ` CONCURRENTLY` ile biter (üst tabloda DEFAULT bölüm varken): "CONCURRENTLY, üst tabloda DEFAULT bölüm varken kullanılamaz; normal ayırmayı seçin." Hiçbir şey değişmez.
- `42809`/`42P16` için özel ipucu **yoktur**; PG iletisi olduğu gibi kalır. Kural ipuçları yalnız PG kendi ipucunu vermediyse uygulanır. İpucu yalnız ATTACH ifadesine bağlıdır (`PARTITION OF` çakışması PG iletisiyle kalır).

### 11.6 Güvenlik, günlük ve test notları

- Sınır değeri, tablo adı, yorum ve SQL `tracing`'e, hata iletisine ya da diske yazılmaz (yakalama testi: `crates/pgt-ddl/tests/logging.rs`, `partition_logging_is_bound_name_free`; yalnız `23514` kodu günlükte). `build/partition.rs` içinde `tracing::` yoktur. Salt okunur gerekçeleri ad/sınır içermez.
- Docker testleri: `crates/pgt-ddl/tests/partition_roundtrip.rs` (5 test; ayrı `pgt_ddl_pt…` veritabanı), `tests/manager.rs` (`partition_defaults_created_gates_and_stale_old`), `tests/logging.rs`. `detachPending` testi katalogda `pg_inherits.inhdetachpending` elle kurularak yapılır (atılabilir veritabanı); eklentiye ait tablo yolu yalnız SQL'dedir (Docker testi yok).
- `pgt-diff` Bölüm türünü `normalize/script`'te tanır (kind_rank 16, etiket "Bölüm").

### 11.7 Ön yüz bütçe notu

- Plan 36 sonunda ana paket **126,553 B**, başlangıç **154,226 B** (tavanlarla birebir, değişmedi). Plan 37'nin `ddl-kinds.ts` ve Explorer menü/ikon kancası ek bayt getirir; **adı konmuş bir telafi gerekir** (tavanlar yükseltilmez). `PlanRequest.Drop.concurrently` TS'de isteğe bağlı olduğundan ana paket etkilenmez.
- Plan 36'da yalnızca tembel dosyalar uyum için değişti (`src/features/schema-diff` **değişmedi**): `DesignerView.tsx` geçici `formMissing` koruması (metin `tdes.view.formMissing`), `KIND_LABELS.partition`, boş `FORM_PATHS.partition` (Plan 37 yeni yollarla tamamlamalı; bütünlük denetimi 17 tür), `drop-model.ts` (Partition: `cascade`/`force` yok; `concurrently` seçeneğini Plan 37 ekler).

---

## 12. Plan 37 notları (Grup B ön yüzü)

- **Bütçe telafisi ve ölçümler:** Plan 37 menü kancası ana pakete bayt ekledi; adı konmuş telafi: `qtRun` → `src/ipc/qt-more-commands.ts`, `ddlTargetOf` → `src/ipc/ddl-commands.ts`, `settingsGet` → yeni `src/ipc/settings-commands.ts` (hepsi yalnız tembel tüketicili), hata kartı metinleri (`gone`/`retry`/`refreshParent`) → `src/features/explorer/error-badge-card-text.ts` (ErrorBadgeCard başlık/etiketi kendi hesaplar). `boundObject`/`detachObject` koşulu mevcut `underPartitions()` ile yazıldı. Tavanlar yalnız düştü: ana paket 126553 → 126415 B, başlangıç 154226 → 154088 B. PartitionForm 22712 B, DropDialog 7827 B.
- **Tek "Oluştur…" + kip seçimi (bilinçli sapma):** ayrı "Bölüm bağla…" menü öğesi yoktur; Partitions koleksiyonundaki "Oluştur…" formunda yeni oluştur / var olan tabloyu bağla radio grubu vardır (ana paket baytı gerekçesi). Bağlanabilir tablo listesi `attachable_tables` lookup'ındandır.
- **Eylemler:** `boundObject` (Bölüm sınırı…) `bound` kipiyle alter sekmesini açar (`findAlter` tekilliği); `detachObject` (Bölümü ayır…) `detach` kipiyle drop diyaloğunu açar. İkisi de `DdlTarget.partition` yoksa `tdes.open.none` döner; Düzenle Table'ı açmaya devam eder.
- **`createContext`:** designer-controller, create kipinde üst tablo/şema bağlamını (`schemaOid` + `tableOid`) formun attach lookup'ına ve oluşturma isteğine taşır.
- **İkinci yenileme:** uygulama sonrası Partitions koleksiyonuna ek olarak `tablesRefreshNodeId` (şemanın Tables koleksiyonu) yenilenir (`OpenContext.tablesNodeId`, drop'ta `extraRefreshNodeIds`; her biri ayrı, hata yutulur). Farklı şemaya bağlanan tablonun şema listesi yenilenmez (bilinen sınır).
- **Ayırma diyaloğu:** `concurrently` seçeneği yalnız Partition drop isteğinde yazılır (`dropRequest(..., concurrently)`; kind kapısı, `detachPending` iken no-op); `FINALIZE` notu; Yıkıcı onay mevcut kapıdan gelir; ayırma metinleri ana pakette yoktur.
- **Alt bölümleme:** form Tablo düzenleyicisine bağlı olmadığından minimal bağımsız strateji/anahtar düzenleyicisi vardır (`partition-model.ts`).
- **Backlog:** DropDialog bileşen testi; `PartitionForm` erişilebilirlik ince ayarları (`aria-describedby` kimlikleri, ListEditor `listAdd`/`listRemove` kullanımı, hash 007 imleç); `ErrorBadgeCard` başlık yorumu; şema karşılaştırmada bölüm sınırları; farklı şemaya bağlanan tablonun şema listesi yenilemesi.
