# pg-tools — Alt Proje 5: Nesne Oluşturma / Düzenleme Diyalogları Tasarımı

- **Tarih:** 2026-10-06
- **Durum:** Plan 10–11 uygulandı; Plan 12–13 kalan
- **Kapsam:** Object Explorer'dan nesne **oluşturma, düzenleme, yeniden adlandırma, silme** (Schema, Sequence, Extension, Index, Table/Column/Constraint, View/Materialized View, Database, Role); önizlemeli, korumalı DDL uygulaması
- **Önceki alt projeler:** Object Explorer (Plan 1–4), Query Tool (5–6), Editör zekâsı (7), Dashboard (8–9) — hepsi `main`'de.
- **Teslim planları:** Plan 10 (arka uç çerçevesi + Schema, Sequence, Extension, Index) → Plan 11 (ön yüz çerçevesi + aynı dört tür) → Plan 12 (Table/Column/Constraint) → Plan 13 (View/MatView, Database, Role).

---

## 1. Amaç ve bağlam

Object Explorer salt okunurdu; Query Tool SQL yazdırır ama nesne tanımlamak için DDL'i elle bilmek gerekir. pgAdmin'in "Create/Properties" diyaloglarının hafif karşılığı: alanları doldur, **üretilen SQL'i önizle**, onayla, uygula. Temel ilkeler: **hiçbir DDL önizlenmeden ve onaylanmadan çalışmaz**, üretim sunucularında yıkıcı değişiklik ek onay ister, kilitli sunucuda hiçbir değişiklik yapılamaz.

### 1.1 Kullanıcıdan gelen kararlar

| Konu | Karar |
|---|---|
| Nesne türleri | Tablo + kolon + kısıtlar; Index; Schema, Sequence, View/MatView; Database, Role, Extension |
| Teslim sırası | Çerçeve + basit nesneler → Tablo → Geri kalan (3 plan) |
| Uygulama | **SQL önizleme + onay**; isteğe bağlı "Query Tool'da aç" |
| Korumalar | Query Tool ile aynı: kilitli sunucuda tümü kapalı; üretimde yıkıcı işlemlerde sunucu adını yazarak onay; manuel transaction modu yok |
| Düzenleme | Mevcut tanım katalogdan yüklenir, **fark `ALTER …` olarak üretilir** |
| Alanlar | Her diyalogda **Sahip** ve **Yorum**; GRANT/REVOKE (yetki matrisi) sonraki iş |
| Giriş noktası | Sağ tık menüsü ve koleksiyonda "Oluştur…"; diyalog **sağ panelde ayrı sekme** (modal değil) |
| Teknoloji | Mevcut yığın (Rust, Tauri 2, SolidJS, Kobalte, Tailwind 4, CodeMirror 6) |

### 1.2 Başarı ölçütleri

| Ölçüt | Hedef |
|---|---|
| Güvenlik | Önizlenmemiş/onaylanmamış hiçbir DDL çalışmaz; kilitli sunucuda arka uç reddeder (yalnızca arayüz engeli yetmez) |
| Doğruluk | Her nesne türü için PG 10–18 matrisinde **gidiş-dönüş** testi: oluştur → katalogdan oku → tanım eşit; düzenle (fark) → uygula → katalog yeni tanıma eşit |
| Önizleme gecikmesi | Alan değişikliğinden sonra önizleme < 150 ms (saf Rust üretici, kısa debounce) |
| Ana paket | Diyalog kodu tembel parça; ana paket ≤ +3 KB |
| Sunucu yükü | Uygulama kısa ömürlü ayrı bağlantıda, `lock_timeout` ile; kuyrukta bekleyip kilit yığılması yaratmaz |

### 1.3 Kapsam dışı

- GRANT/REVOKE matrisi, varsayılan yetkiler (`ALTER DEFAULT PRIVILEGES`), RLS politikaları, trigger/fonksiyon/procedure/type/domain/FDW/publication/subscription diyalogları.
- Partition oluşturma/bağlama (tablo diyaloğu yalnızca partitioned tablo **tanımlar**), inheritance, EXCLUDE kısıtları, tablespace oluşturma.
- Kolon sırası değiştirme (PostgreSQL desteklemez; arayüz uyarır), toplu/şablonlu iş akışları, DDL geçmişi/geri alma.
- Rol üyeliklerinde PG 16+ seçenekleri (`INHERIT`/`SET` per grant).

---

## 2. Mimari

- Yeni crate **`pgt-ddl`** (`pgt-core` + `pgt-catalog` yardımcıları: `quote`, `PgVersion`, `VersionedSql`). Üç katman:
  1. **Model** (`model.rs`): nesne türü başına yapılandırılmış tanım (`SchemaDef`, `SequenceDef`, `IndexDef`, `TableDef { columns, constraints, … }`, …), `serde` + `ts-rs`. Arayüz formları bu tipleri düzenler.
  2. **Üretici** (`build/*.rs`): **saf işlevler**, ağ/IO yok. `create(def) -> Plan`, `alter(old, new) -> Plan`, `drop(target, options) -> Plan`, `rename(target, new_name) -> Plan`. `Plan { statements: Vec<Statement> }`, `Statement { sql, risk, transactional, note }` (bkz. §3). Sürüme duyarlı (`PgVersion`): ör. `INCLUDE` PG 11+, `GENERATED … STORED` PG 12+, `DROP DATABASE … WITH (FORCE)` PG 13+, ICU `locale_provider` PG 15+. Sürümün desteklemediği alan arayüzde gizlenir ve üreticide reddedilir.
  3. **Yükleyici** (`load/*.rs`, sürüm varyantlı `.sql`): havuzdaki **salt okunur** katalog bağlantısıyla (`ConnectionManager::run`) mevcut tanımı okur → `*Def`. Yükleyicinin döndürdüğü tanım, üreticinin `create` girdisiyle **aynı tip**tir (gidiş-dönüş testinin dayanağı).
- **Uygulayıcı** (`apply.rs`): her uygulama için **kısa ömürlü ayrı bağlantı** (`SessionProfile::Ddl`: `application_name = 'pg-tools ddl'`, `default_transaction_read_only = off`, `statement_timeout = 0`, `lock_timeout` çağrıdan (varsayılan 5 sn)). Transaction'a alınabilen ifadeler tek `BEGIN … COMMIT`; alınamayanlar (`CREATE INDEX CONCURRENTLY`, `CREATE/DROP DATABASE`, bazı `ALTER DATABASE`) **ayrı, sırayla** çalışır ve plan bunu açıkça işaretler. Hata → transaction `ROLLBACK`, hangi ifadenin (indeks + PG konumu) başarısız olduğu döner; transaction-dışı ifadelerden önceki kısmi uygulama kullanıcıya bildirilir. İptal desteklenir (`CancelHandle`).
- **Korumalar (arka uçta zorlanır):**
  - `ServerConfig.read_only_lock` → `ddl_apply` `locked` koduyla reddedilir (Query Tool'un `locked` kapısı gibi); `ddl_preview`/`ddl_load` serbesttir.
  - Plan'da en az bir `Destructive`/`DataLoss` ifade varsa `ddl_apply` `confirmed: true` ister (`confirmation_required`). Üretim ek onayı yalnızca **`DataLoss`** içeren planlar içindir: sunucu etiketi **üretim** ve plan `DataLoss` ise ek olarak `confirmed_production: true` (istemci sunucu adını yazmıştır) gerekir (`production_confirmation_required`); yalnızca `Destructive` plan üretimde de yalnız `confirmed` ister. Arka uç bayrakları yalnızca varlık olarak denetler, adın yazılması arayüz sorumluluğudur.
  - Arka uç ifadeleri istemciden **almaz**: istemci tanımı (`def`/`old`/`new`) gönderir, arka uç planı yeniden üretir ve uygular (istemcide üretilmiş/değiştirilmiş SQL çalıştırılamaz). Önizlenen planın özeti (`plan_hash`) uygulama isteğine eklenir; hash uyuşmazsa (sürüm/def değişti) `plan_changed` ile reddedilir.
- **Tauri komutları** (`ddl_*`, `Box::pin`, ts-rs bağlamaları): `ddl_load(node_id) → LoadedDef`, `ddl_preview(request) → PlanView`, `ddl_apply(request, lock_timeout_ms, confirmed, confirmed_production) → ApplyResult`, `ddl_cancel(apply_id)`, `ddl_lookups(kind, context) → Lookups` (açılır listeler: sahipler/roller, tablespace'ler, şemalar, tipler, extension'lar (`pg_available_extensions`), index yöntemleri, collation'lar, tablo kolonları, referans verilebilir tablolar).
- Günlük: **SQL metni, parola ve tanım değerleri günlüğe yazılmaz**; yalnızca nesne türü, ifade sayısı, süre, sonuç kodu.
- **Sözleşme notları (Plan 10 arka ucu, arayüz Plan 11 için):**
  - `PlanRequest::Alter` ve `PlanRequest::Drop` değiştirilen/silinen nesnenin `object: ObjectRef` kimliğini taşır; `ddl_apply` nesneyi yeniden yükler ve istemcinin `old`/`def` tanımının sunucudaki güncel tanıma **birebir** eşit olmasını ister (fark → `plan_changed`; yok/sistem nesnesi/identity dizisi hataları olduğu gibi geçer). `ddl_preview` bu denetimi yapmaz. Arayüz her zaman `ddl_load`'un döndürdüğü tanımı geri gönderir.
  - `SequenceDef`'in sayısal alanları (`increment`, `minValue`, `maxValue`, `start`, `cache`) JSON'da ondalık **dizge** olarak gider (JS `number` bigint aralığında kesinlik kaybeder); girişte dizge de tamsayı da kabul edilir. "Sayılar `number`" kuralının bilinçli tek istisnasıdır.
  - `FieldError.path` sözlüğü: alan adı (`name`, `schema`, `table`, `method`, `unique`, `include`, `tablespace`, `fillfactor`, `constraint`, `minValue`…), liste öğeleri `columns[i].key` / `columns[i].opclass` / `include[i]`, alter'da eski tanım `old.<alan>`, tür uyuşmazlığı `kind`.
  - Geçersiz tanım: `PlanView.empty = true` ve `errors` boş değil; `ddl_apply` `invalid_def` kodunu döndürür. Boş plan (değişiklik yok) uygulanmaz: kodsuz `invalid` ("Değişiklik yok").
  - Tablespace kaldırılamaz (yalnızca değiştirilebilir): varsayılana dönmek için `pg_default` açıkça seçilir.
  - `ApplyOutcome::Applied.created` yeni nesnenin kimliğini (en iyi çaba), `ApplyOutcome::Failed.position` PG hata konumunu (başarısız ifadenin metninde 1 tabanlı karakter sapması) taşır; `DdlTarget.refreshNodeId` uygulamadan sonra yenilenecek ağaç düğümüdür; yüklenen `IndexDef.valid` geçersiz (INVALID) indeksi işaretler.
  - İndeks yapısal değişikliği tek transaction'da `DROP INDEX` + `CREATE INDEX`'tir (tablo, işlem bitene kadar okuma/yazmaya kapalı kalır); kesintisiz yeniden kurma ileri bir plana ertelendi.

## 3. Plan ve risk modeli

`Statement.risk`:

| Risk | Örnek | Onay |
|---|---|---|
| `Safe` | `CREATE …`, nullable `ADD COLUMN`, `RENAME`, `COMMENT ON`, `ALTER … SET DEFAULT` | Yalnızca "Uygula" |
| `Destructive` | `DROP …` (schema/table/index/sequence/view/role/extension/database), `DROP CONSTRAINT`, `REFRESH MATERIALIZED VIEW` | Onay kutusu ("Etkisini anladım") |
| `DataLoss` | `DROP COLUMN`, `ALTER COLUMN TYPE`, `DROP … CASCADE`, `ALTER … SET NOT NULL` (mevcut veriyle başarısız olabilir → uyarı) | Onay kutusu + **üretimde sunucu adını yazma** |

Plan görünümü (`PlanView`): ifade listesi (sıra numarası, SQL, risk rozeti, `transactional` bayrağı, not), uyarılar (ör. "Kolon sırası değiştirilemez", "Tür dönüşümü `USING` gerektirir", "Veritabanı yeniden adlandırma için açık oturum olmamalı"), `plan_hash`, toplam risk. Boş plan (değişiklik yok) `Uygula`yı kapatır.

## 4. Arayüz

- **Sekme türü** `designer` (`tabs-store` kalıbıyla; dashboard gibi tembel parça, `DesignerTabs`/`DesignerView` ana pakete ince kancayla bağlanır). Başlık: `Oluştur: Table · <sunucu>/<db>` / `Düzenle: <nesne>`. Sekme kapanırken kaydedilmemiş değişiklik varsa onay sorar.
- **Giriş noktaları** (`ExplorerMenu`/`actions.ts`): koleksiyon düğümünde **Oluştur…**; nesne düğümünde **Düzenle…**, **Yeniden adlandır…**, **Sil…**. Kilitli sunucuda değiştiren eylemler devre dışı + gerekçe (`title`). Eylem kümeleri `actionsFor` içinde türe göre.
- **Yerleşim:** solda form (bölüm sekmeleri: Genel, tür özel, "SQL" önizleme), sağda/altta **canlı SQL önizleme** (salt okunur CodeMirror, `DdlView` ile aynı kurulum; hata veren ifade vurgulanır); altta **uygulama çubuğu**: risk özeti, `lock_timeout` seçici (1 sn / 5 sn / 30 sn / sınırsız), **Uygula**, **Query Tool'da aç**, **Vazgeç**.
- **Önizleme akışı:** form değişir → 150 ms debounce → `ddl_preview` (en son istek kazanır) → plan görünür. Doğrulama hataları (boş ad, 63 bayt aşımı, aynı kolon adı, eksik zorunlu alan) form üzerinde alan altında; geçersiz formda önizleme "formu tamamlayın" der ve Uygula kapalıdır.
- **Uygulama sonrası:** başarı toast'ı; Explorer'da ilgili düğüm yenilenir (oluşturma → üst koleksiyon; düzenleme → düğüm; silme → düğüm kaldırılır); düzenleme sekmesi yeni tanımı yeniden yükler. Hata: PG'nin `message`/`detail`/`hint` metni + başarısız ifadenin vurgulanması; form korunur.
- **Silme** ayrı küçük diyalogtur (sekme açmaz): nesne adı, `CASCADE` onay kutusu (varsayılan kapalı), plan önizleme, risk onayı. `CASCADE` kapalıyken PG bağımlılık hatası verirse `detail` (bağımlı nesneler) gösterilir.
- **Erişilebilirlik / tutarlılık:** Kobalte bileşenleri, tüm alanlarda etiket, hata alanları `aria-describedby`, odak sırası; Türkçe metinler `strings-designer.ts` (yalnızca tembel parçada).

## 5. Nesne türleri (ilk sürüm alanları)

Her türde ortak: **Ad, Sahip (rol listesi), Yorum**; düzenlemede **Yeniden adlandır**.

- **Schema:** ad, sahip, yorum. Sil (`DROP SCHEMA [CASCADE]`).
- **Sequence:** şema, veri tipi (smallint/integer/bigint, PG10+), artış, min, maks, başlangıç, cache, döngü, `OWNED BY tablo.kolon`; düzenleme: `ALTER SEQUENCE` farkı + `RESTART` seçeneği.
- **Extension:** `pg_available_extensions`'tan seçim, şema, sürüm, `CASCADE`; düzenleme: `ALTER EXTENSION … UPDATE TO`, `SET SCHEMA`; sil.
- **Index:** tablo, ad, yöntem (btree/hash/gist/gin/brin/spgist), kolon/ifade listesi (opclass, collation, sıralama, NULLS), `INCLUDE` (PG11+), `UNIQUE`, `WHERE`, tablespace, `fillfactor`, `CONCURRENTLY` (transaction dışı, uyarılı); düzenleme: yeniden adlandır, tablespace, `fillfactor`; **tanım değişikliği drop + create olarak planlanır ve bu açıkça not edilir**; `REINDEX` eylemi.
- **Table (Plan 11):** şema, ad, sahip, tablespace, `UNLOGGED`, `WITH (fillfactor …)`, partitioned (`RANGE`/`LIST`/`HASH` anahtarları; yalnızca oluşturmada); kolonlar (ad, tip + uzunluk/hassasiyet/dizi, NOT NULL, default, identity (`ALWAYS`/`BY DEFAULT` + sequence seçenekleri), `GENERATED … STORED` (PG12+), collation, yorum); kısıtlar: **PK, UNIQUE, CHECK, FK** (hedef tablo/kolonlar, `MATCH`, `ON UPDATE/DELETE`, `DEFERRABLE`, `NOT VALID`); düzenleme farkı: ad değişikliği, kolon ekle/sil/yeniden adlandır, `ALTER COLUMN TYPE … USING` (kullanıcı ifadesi), NOT NULL/default/identity aç-kapa, kısıt ekle/sil/yeniden adlandır/`VALIDATE`; kolon sırası değişikliği uyarı.
- **View / Materialized View (Plan 12):** şema, ad, sahip, sorgu gövdesi (SQL editörü), seçenekler (`security_barrier`, `check_option`), MatView: `WITH [NO] DATA`, tablespace; düzenleme: `CREATE OR REPLACE VIEW` (kolon uyumsuzluğunda drop+create'e yükseltilir ve not edilir), yeniden adlandır, MatView `REFRESH`.
- **Database (Plan 12):** ad, sahip, template, encoding, locale/collation (PG15+ ICU sağlayıcısı), tablespace, bağlantı limiti, `is_template`, `allow_connections`; düzenleme: yeniden adlandır (açık oturum uyarısı), sahip, tablespace, limit, bayraklar; sil (`WITH (FORCE)` PG13+ onay kutusu). **Tüm ifadeler transaction dışı.** Dashboard'daki oturum sonlandırmaya bağlantı noktası (ilgili uyarı metninde ipucu).
- **Role (Plan 12):** ad, `LOGIN`, `SUPERUSER`, `CREATEDB`, `CREATEROLE`, `REPLICATION`, `BYPASSRLS`, `INHERIT`, bağlantı limiti, `VALID UNTIL`, **parola (yalnızca yazılır, asla yüklenmez/gösterilmez)**, üyelikler (`GRANT rol TO rol` ekle/çıkar). **Parola önizlemede `********` olarak maskelenir**; uygulamada gerçek değer kullanılır, günlüğe/hata iletisine/geçmişe girmez. Parola düz metin gönderilir ve sunucu `password_encryption` ayarıyla özetler; `log_statement = 'ddl'` açık sunucularda sunucu günlüğüne düşebilir — diyalog bu uyarıyı gösterir. (İstemci tarafı SCRAM özeti üretimi bu alt projede yok.)

## 6. Güvenlik ve gizlilik

- Önizleme/yükleme **salt okunur** havuz bağlantısından; yazan yol yalnızca `ddl_apply` (ayrı bağlantı, ayrı profil, kilit kapısı).
- Arka uç istemci SQL'i çalıştırmaz (§2): SQL enjeksiyonu yüzeyi, `quote` ile tırnaklanan tanımlayıcılar ve `quote_literal` ile bağlanan değerlerden ibarettir; **tüm tanımlayıcı/değer yerleştirmeleri tek bir yardımcıdan** geçer ve birim + özellik tabanlı testlerle (rastgele/Unicode/tırnaklı adlar) doğrulanır. Serbest SQL alanları (view gövdesi, CHECK/default/WHERE ifadesi, `USING`) kullanıcı SQL'idir ve **olduğu gibi** yerleştirilir; bu alanlar için arayüz "ifade, SQL olarak çalıştırılır" notunu gösterir ve sunucu tarafında ayrıştırılarak hatası önizlemede (`PREPARE`/Parse yoluyla, Query Tool'daki `guarded_parse` kalıbıyla; uygulama öncesi **çalıştırmadan**) gösterilebilir.
- Parola, SQL metni ve tanım içeriği günlüğe, hata iletisine, `localStorage`'a, geçmişe yazılmaz.
- Kilit: arka uç zorlar; etiket/üretim onayı bayrakları arka uçta varlık olarak denetlenir.

## 7. Test stratejisi

- **Üretici (Rust, saf):** tablo odaklı birim testleri + `insta` anlık görüntüleri (nesne türü × sürüm), tırnaklama özellik testleri (Unicode, çift tırnak, 63 bayt sınırı, ayrılmış sözcükler), fark üretimi (eski/yeni → beklenen ifade dizisi, no-op → boş plan, yeniden adlandırma + değişiklik birleşimi, sürüm kapıları).
- **Gidiş-dönüş (Docker, PG 10–18 matrisi, `scripts/test-matrix.ps1`):** her tür için `create(def)` uygula → `load` → `def` eşit; `alter(old,new)` uygula → `load` → `new` eşit; yıkıcı planlar (drop/type değişimi) ve geri alma (hata ortası → hiçbir değişiklik yok); transaction dışı ifadeler (`CONCURRENTLY`, `CREATE DATABASE`); `lock_timeout` (kilit tutan oturumla): uygulama zaman aşımıyla düşer ve sorguyu kuyrukta bırakmaz; kilitli sunucuda `locked`; onaysız yıkıcı plan `confirmation_required`; değişen `plan_hash` `plan_changed`; yetkisiz rol (42501) hatası aktarımı; parola günlüğe girmez (günlük yakalama testi).
- **Ön yüz (vitest, saf):** form modelleri (kolon/kısıt liste işlemleri, doğrulama kuralları, bağımlı alan gizleme sürüme göre), önizleme debouncer'ı (en son istek kazanır, kapanan sekme yanıtı yok sayılır), risk onayı modeli (hangi planda hangi onay), sekme yaşam döngüsü (kaydedilmemiş değişiklik uyarısı), Explorer yenileme kararı.
- **Ölçüm/elle:** ana paket boyutu aynı eşik; elle kontrol listesi `docs/smoke-test.md`'ye eklenir.

## 8. Planlama notu

- **Plan 10 (çerçeve + basit nesneler):** `pgt-ddl` iskeleti, `quote`/`Plan`/risk modeli, `SessionProfile::Ddl`, uygulayıcı + korumalar + `ddl_*` komutları, **Schema/Sequence/Extension/Index** üreticileri + yükleyicileri + Docker gidiş-dönüş testleri; ön yüz: `designer` sekmesi, form kabuğu, önizleme, uygulama çubuğu, silme diyaloğu, Explorer menü bağlantısı ve bu dört tür.
- **Plan 11 (Tablo):** kolon/kısıt model ve üreticileri (fark → ALTER), form (kolon ve kısıt liste düzenleyicileri), tür kümesi/`lookups`, `USING` ifadesi akışı.
- **Plan 12 (geri kalan):** View/MatView, Database (transaction dışı hat), Role (parola, üyelik).
- Her plan kendi başına çalışır durumda `main`'e girer; bu spec üçünün ortak otoritesidir.
