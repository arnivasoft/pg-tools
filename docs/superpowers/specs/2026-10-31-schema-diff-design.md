# pg-tools — Şema Karşılaştırma (Schema Diff) Tasarımı

- **Tarih:** 2026-10-31
- **Durum:** Plan 29 arka ucu tamam; Plan 30 ön yüzü uygulandı (feat/schema-diff-ui); elle kontrol bekliyor (`docs/smoke-test.md` "Şema karşılaştırma (Plan 30)")
- **Kapsam:**
  - İki şemanın yapısı karşılaştırılır: kaynak (sunucu → veritabanı → şema) ve hedef (sunucu → veritabanı → şema). Sunucular, veritabanları ve şema adları farklı olabilir.
  - Farklar nesne nesne listelenir ve her nesnenin iki taraftaki DDL'i satır satır gösterilir.
  - Seçilen farklardan, hedefi kaynağa eşitleyen bir SQL betiği üretilir. Betik önizlenir, kopyalanır, dosyaya kaydedilir ya da hedefe bağlı yeni bir Query Tool sekmesinde açılır.
- **Kullanıcı kararları (2026-10-31):**
  - Kapsam şema ↔ şema'dır. Veritabanı düzeyi nesneler (uzantı, rol, veritabanının kendisi, tablespace) kapsam dışıdır.
  - Sonuç betiktir ve Query Tool'a aktarılır. Diyalogdan doğrudan uygulama **yoktur**; çalıştırma, Query Tool'un mevcut risk ve kilit kapılarından geçer.
  - İlk sürüm `pgt-ddl`'in modellediği şema kapsamlı türlerin hepsini kapsar: tablo (kolon ve kısıtlarla), indeks, trigger, görünüm, MV, sequence, fonksiyon, prosedür, type (enum/composite/range) ve domain.
  - Yetkiler (GRANT) ve sahiplik bir seçenektir, varsayılanı kapalıdır.
- **Teslim:**
  - Plan 29 (arka uç): yeni `pgt-diff` crate'i. İçinde toplu yükleme, eşleştirme, normalizasyon, betik sıralama, satır farkı, Tauri komutları ve Docker testleri var.
  - Plan 30 (ön yüz): "Şema karşılaştırma" sekmesi, Explorer menüsü ve betik önizleme.

---

## 1. Mevcut altyapı (yeniden kullanılır)

- `pgt-ddl::load::load(mgr, server, db_oid, ObjectRef)` tek bir nesneyi `ObjectDef` modeline yükler. PG 10–18 sürüm farkları `VersionedSql` ile çözülür.
- `pgt-ddl::build::{create, alter, drop_with}` saf üreticilerdir (IO yok, deterministik). Bir `Plan { statements, warnings }` döndürürler ya da `Vec<FieldError>` ile reddederler. Her `Statement` `risk` (`Safe < Destructive < DataLoss`) ve `transactional` bilgisi taşır.
- Görünüm sorgusu değişince üretici `recreate` bayrağıyla DROP + CREATE üretir. Fonksiyonda yeniden kurma gerekip gerekmediğini `routine::needs_recreate` söyler.
- Modelin kapsamadığı özellikler, yüklemede `unsupported: Vec<String>` listesine yazılır (tablo, görünüm, MV, fonksiyon/prosedür, trigger, type, domain).

Şema karşılaştırma bu katmanın **üstüne** kurulur. Yeni SQL üretici yazılmaz; her ifade bu üreticilerden gelir.

## 2. Kararlar

### 2.1 Yükleme

- **Liste sorgusu:** taraf başına tek bir katalog sorgusu, şemadaki nesneleri `(kind, oid, ad, kimlik imzası)` olarak döndürür.
  - Kapsam: tablo (bölümler dahil işaretli), görünüm, MV, sequence, indeks (kısıt indeksleri hariç; onlar kısıtla gelir), trigger (iç ve bölüm klonu triggerlar hariç), fonksiyon, prosedür, type ve domain.
  - Kimlik imzası fonksiyon ve prosedürde `oidvectortypes(proargtypes)`'tır (yalnız giriş tipleri; `pg_get_function_identity_arguments` argüman adlarını da içerdiğinden ad değişikliği eşleşmeyi bozardı). İndeks ve triggerda ise bağlı tablonun adıdır.
  - **Hariç tutulanlar:**
    - Uzantıya ait nesneler (`pg_depend.deptype = 'e'`).
    - Sistem şemaları: karşılaştırma kaynağı ya da hedefi olarak seçilemezler.
    - Tablo satır tipleri, dizi tipleri ve sequence'lerin satır tipleri.
    - Kimlik (identity) dizileri, bölüm indeks klonları, multirange tipleri ve aggregate'ler.
- **Tek bağlantı, tutarlı anlık görüntü:** her taraf, havuz dışı ayrılmış bir katalog bağlantısında (havuzdaki bağlantı başka görevlerce paylaşılır) tek bir `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` transaction'ı içinde yüklenir. Transaction'da `SET LOCAL search_path TO pg_catalog, <şema>` uygulanır (`pg_catalog` önde: yükleyici sorgularındaki niteliksiz katalog adları kullanıcı nesnelerine çözülmesin).
  - Bunun nedeni, `pg_get_viewdef`, `pg_get_expr` ve `format_type` çıktısında aynı şemadaki nesnelerin **niteliksiz** yazılmasıdır. Böylece şema adları farklı olsa da gövdeler ve tip adları karşılaştırılabilir hâle gelir.
  - Bunun için `load.rs`'deki tür yükleyicileri, istemci alan iç fonksiyonlara ayrılır. Mevcut `load` bu iç fonksiyonları `mgr.run` içinde çağırmaya devam eder; Designer ve Inspector davranışı değişmez.
- **İlerleme ve iptal:** yükleme bir iş olarak yürür. Taraf başına `(yüklenen, toplam)` ilerleme olayı yayılır. İptal, ayrılmış bağlantıyı kapatır; sunucu transaction'ı geri alır.
- **Sürüm:** iki tarafın `server_version_num` değeri ayrı okunur. Üreticiler **hedefin** sürümüyle (`BuildCtx.version_num`) çağrılır; hedefte desteklenmeyen bir özellik, öğenin hatası olarak döner (§2.5).
- **Aynı şema reddi:** sunucu, veritabanı ve şemanın üçü de aynıysa karşılaştırma başlamaz.

### 2.2 Eşleştirme

- **Anahtar:** `(tür sınıfı, ad, kimlik imzası)`.
  - İlişkiler (tablo, görünüm, MV, sequence) aynı ad alanını paylaşır. Kaynakta tablo, hedefte aynı adda görünüm varsa öğe "tür değişti" olur ve hedefteki DROP ile kaynaktaki CREATE'e açılır.
  - Fonksiyon ve prosedür ad + kimlik argümanlarıyla eşleşir; aşırı yüklemeler ayrı öğelerdir.
  - İndeks ve trigger, tablo adı + kendi adıyla eşleşir.
- **Yeniden adlandırma algılanmaz.** Yeni adlı nesne "yalnız kaynakta", eski adlı nesne "yalnız hedefte" görünür. Kolonlarda da aynısı geçerlidir: ad değişikliği DROP COLUMN + ADD COLUMN olur ve risk DataLoss olarak işaretlenir.

### 2.3 Normalizasyon (karşılaştırmadan önce)

- **Şema adı:** kaynak modeldeki `schema` alanları hedef şema adına çevrilir. Böylece üretilen ifadeler hedef şemayı nitelendirir. Serbest SQL gövdelerine (görünüm sorgusu, fonksiyon gövdesi, varsayılan değer, CHECK ifadesi) **dokunulmaz**.
  - §2.1'deki `search_path` sayesinde aynı şemadaki adlar zaten niteliksizdir.
  - Bir gövde kaynak şemayı **açıkça** nitelendiriyorsa (ör. `src.t`) metin olduğu gibi kalır. Şema adları farklıysa sonuç ekranında bir uyarı gösterilir: "Gövdelerdeki nitelikli adları gözden geçirin".
- **Yok sayılanlar (her zaman):**
  - Oid'ler ve yalnız yüklemede dolan salt okunur alanlar (`columns` gösterimi vb.). `partition_text` yok sayılmaz: yükleyici `partition_by`'ı doldurmaz, bölümleme anahtarı yalnız orada taşınır; farkı "elle gerekiyor"dur.
  - Sequence'lerin anlık değeri (`last_value`, `is_called`).
  - MV'nin dolu olup olmadığı (`with_data`): bu bir veri durumudur.
- **Yok sayılanlar (seçenekle):**
  - **Sahiplik ve yetkiler** (varsayılan: yok sayılır). Açıkken `owner` ve ACL karşılaştırılır; betikte `ALTER … OWNER TO` ile mevcut ACL üreticisinin GRANT/REVOKE ifadeleri yer alır.
  - **Tablespace** (varsayılan: yok sayılır).
  - **Yorumlar** (varsayılan: karşılaştırılır).
- **Kolon sırası:** sıra farkı tek başına fark sayılmaz, çünkü ALTER ile değiştirilemez. Öğede bir not olarak gösterilir ("kolon sırası farklı"). Yeni kolonlar tablonun sonuna eklenir.

### 2.4 Öğe durumu ve üretim

| Durum | Üretim | Varsayılan seçim |
|---|---|---|
| Yalnız kaynakta | `build::create(kaynak)` | Seçili |
| Yalnız hedefte | `build::drop_with(hedef, cascade: false)` | **Seçili değil** |
| Farklı | `build::alter(hedef, kaynak)` | Seçili |
| Aynı | — | Listede gizli (filtreyle görünür) |

- **Yeniden kurma gerekirse:**
  - Görünümde sorgu değişikliği `recreate` ile yapılır. **İstisna (Plan 29 son inceleme):** hedef görünümün `unsupported` listesinde `TRIGGER …` (INSTEAD OF trigger), `RULE …` ya da `DEFAULT …` (kolon varsayılanı) varsa DROP VIEW bunları sessizce düşüreceği için öğe yeniden kurulmaz, "elle gerekiyor" olur. Not kaybolacakları adlarıyla sayar: "Sorgu değişti; görünüm yeniden kurulursa (DROP + CREATE) hedefteki şu ekler kaybolur, elle uygulayın: TRIGGER t, RULE r". Yalnız kolon yorumu (`COMMENT …`) kaybolacaksa yeniden kurma sürer (üreticinin "Kaybolacaklar" uyarısıyla).
  - Fonksiyonda dönüş tipi ya da argüman adı değişikliği, `needs_recreate` doluysa DROP + CREATE olur. Risk en az `Destructive`'dir ve öğede "yeniden kurulacak" notu yer alır.
  - MV'de sorgu değişikliği DROP + CREATE'tir (üretici kuralı). DROP MATERIALIZED VIEW indeksleri de düşürdüğü için, MV yeniden kurulduğunda (sorgu farkı ya da tür değişimi) o MV'nin **değişmeyen** (`same`) indeksleri kaynak tanımlarıyla MV öğesinin indeks aşamasında (§2.6 4.8) yeniden oluşturulur; yeniden karşılaştırma temiz çıkar.
  - **Hedef sahibi ve yorumu korunur:** sahiplik ya da yorum seçeneği kapalıyken normalizasyon bu alanları boşaltır; nesne baştan kuruluyorsa (görünüm / fonksiyon / MV DROP + CREATE, tür değişimi) yeni nesneye **hedefteki** sahip (`OWNER TO`) ve yorum (`COMMENT ON`) geri verilir. Böylece ör. `app_owner`'a ait bir SECURITY DEFINER fonksiyon, betiği çalıştıran rolün sahipliğine geçmez. Seçenek açıksa kaynak değerleri kullanılır (değişiklik yok). Yerinde değişiklikler (`CREATE OR REPLACE`, `ALTER`) sahibi ve yorumu zaten korur.
- **Tablolar yeniden kurulmaz.** `alter` bir tablo farkını ifade edemiyorsa (bölümleme anahtarı, persistence vb.), öğe "elle gerekiyor" olarak işaretlenir. Bu öğeler betiğe girmez; gerekçe öğede gösterilir. Yalnız kaynakta olan bölümlenmiş üst tablo da "elle gerekiyor"dur (`create` bölümsüz tablo üretirdi).
- **Fark kaynağı:** "farklı" kararı normalize edilmiş modellerin eşitliğiyle verilir. Modeller aynı olsa bile iki taraftaki `unsupported` listeleri farklıysa öğe "modelin dışında fark var" uyarısıyla **farklı** listelenir. Böyle bir öğe için betik üretilmez ve öğe "elle gerekiyor" olur.
- **Satır farkı:** her öğenin iki tarafı için normalize modelden `build::create` metni üretilir. Bu metinler, sabit şema adıyla iki tarafta da aynı biçimde çıkar. Sonra iki metin satır satır karşılaştırılır.
  - Fark Rust'ta Myers algoritmasıyla hesaplanır (yeni npm bağımlılığı yok).
  - Satır sayısının toplamı 20.000'i aşarsa fark hesaplanmaz; iki metin yan yana düz gösterilir.

### 2.5 Hatalar

- Bir nesne yüklenemezse karşılaştırma durmaz. Öğe "yüklenemedi" durumunda listelenir, ileti `pgt-core` hata eşlemesinden gelir. Bu yalnız yükleyicinin nesne düzeyi retleri içindir (sistem nesnesi, geçici tablo vb.); bir yükleme sorgusu sunucu hatası verirse transaction bozulur ve o tarafın işi biter.
- Üreticiler `FieldError` döndürürse öğe "üretilemedi" olur. Örnek: hedefte desteklenmeyen bir özellik, ya da kaynakta PG 15'in `NULLS NOT DISTINCT`'i varken hedef PG 13 ise. Alan iletileri öğede gösterilir.
- Bağlantı kopması ya da iptal, bütün işi bitirir; kısmi sonuç gösterilmez.

### 2.6 Betik sıralaması

Sıralama, seçili öğeler üzerinde yapılır. Her öğenin ifadeleri kendi üreticisinin sırasını korur.

1. Başlık yorumu: kaynak ve hedef olarak `veritabanı.şema` adları, hedef sürümü ve zaman. Sunucu adresi, kullanıcı adı ya da parola **yazılmaz**.
2. `SET check_function_bodies = off;` ve `SET search_path TO pg_catalog, <hedef şema>;` (transaction'lı betikte `SET LOCAL`)
3. **Silmeler** (bağımlılığın tersi sırasıyla): trigger → indeks → görünüm/MV (hedefteki `pg_depend` sırasının tersi) → fonksiyon/prosedür → tablo → sequence → domain → type.
4. **Oluşturma ve değişiklikler:**
   1. type (enum/composite/range; composite'ler bağımlılık sırasıyla) ve domain
   2. sequence (OWNED BY hariç)
   3. fonksiyon ve prosedür
   4. tablo oluşturma (FK kısıtları **hariç**) ve tablo değişiklikleri (FK ekleme hariç)
      - ardından, imzası (argüman tipleri ya da `SETOF` / `TABLE` kolonları dahil dönüş tipi) **bu betikte oluşturulan ya da yeniden kurulan** bir tablonun satır tipine başvuran fonksiyon/prosedürler. Başvuru, yüklenen tip adlarından (`format_type`; şema niteleyicisi kaynak/hedef şemaysa atılır) seçili tablo/görünüm/MV öğeleriyle eşlenerek bulunur; gövde metnine bakılmaz.
   5. sequence `OWNED BY`
   6. FK kısıtları: yeni tablolarınki ve değişen tablolarda eklenenler. Bu ifadeler, FK'sız model eski, tam model yeni alınarak `table_alter::alter` ile üretilir.
   7. görünüm ve MV (kaynaktaki `pg_depend` sırasıyla)
      - ardından, imzası bu betikte oluşturulan ya da yeniden kurulan bir görünümün / MV'nin satır tipine başvuran fonksiyon/prosedürler (hem tabloya hem görünüme başvuran rutin en geç olanın, yani bu adımın arkasına düşer). Diğer rutinler 4.3'te kalır; aşama içi sıra deterministiktir (tür, ad).
   8. indeks (yeniden kurulan MV'nin yeniden oluşturulan indeksleri dahil)
   9. trigger
   10. yetkiler: GRANT/REVOKE (seçenek açıksa). Sahiplik (`OWNER TO`) ve yorumlar üreticinin kendi planında, nesnenin ifadeleriyle birlikte kalır.
5. **Transaction:**
   - Tüm ifadeler `transactional` ise betik `BEGIN; … COMMIT;` ile sarılır.
   - Değilse (`CREATE INDEX CONCURRENTLY`, PG 12 öncesinde `ALTER TYPE … ADD VALUE` vb.) sarılmaz. Başlıkta "Betik transaction'sız; hata olursa kısmen uygulanmış kalır" uyarısı yer alır.
- **Bağımlılık tamamlama yoktur.** Seçilmemiş bir nesneye bağımlı bir öğe seçilirse betik hedefte hata verir. Bu hatayı Query Tool gösterir. Bilinen bir sınırdır (§5).

### 2.7 IPC ve durum

- **Komutlar** (`src-tauri`):
  - `schema_diff_databases(server_id) -> [{ oid, name }]`: seçici için bağlanılabilir, şablon olmayan veritabanları.
  - `schema_diff_start(diff_id, source, target, options, channel) -> sonuç`: yükleme ve karşılaştırma işini başlatır; kimliği istemci üretir (yedekleme deseni), komut iş bitince sonucu döndürür. `source` ve `target` alanlarının her biri `{ serverId, databaseOid, schema }` biçimindedir. İlerleme olayları `channel` üzerinden gelir.
  - `schema_diff_cancel(diff_id)`
  - `schema_diff_detail(diff_id, item_key) -> { sourceText, targetText, lines }`: satır farkı, liste şişmesin diye öğe seçilince ayrıca istenir.
  - `schema_diff_script(diff_id, item_keys) -> { sql, transactional, warnings, risk }`
  - `schema_diff_save(diff_id, path)`: son üretilen betiği dosyaya yazar (uygulamada dosya yazan genel bir eklenti yoktur).
  - `schema_diff_close(diff_id)`
- **Sonuç listesi:** her öğe için `{ key, kind, schema-içi ad, imza, status, risk, notes, error }` tutulur. Modeller arka uçta, `diff_id` ile bir kayıt defterinde saklanır. Sekme kapanınca ya da yeni karşılaştırma başlayınca bu kayıt silinir.
- **Bağlantı:** iki taraf da Explorer'daki sunucu kayıtlarının bağlantılarını kullanır. Bağlı olmayan sunucu için mevcut parola isteme akışı işler.
- **Günlük:** SQL metni, nesne adları ve değerler günlüğe yazılmaz. Yalnızca sayımlar ve süreler yazılır.

### 2.8 Ön yüz

- **Giriş noktaları:**
  - Explorer'da şema düğümünün menüsünde "Şema karşılaştır…" var; o şema kaynak olarak seçilir.
  - Veritabanı düğümünde de aynı giriş var; kaynak veritabanı seçili gelir, şema seçilmemiş olur.
- **Sekme** (tembel parça `schema-diff`, dashboard ve veri düzenleyicideki `*-open` deseniyle):
  - **Üst bölüm:**
    - kaynak ve hedef seçiciler (sunucu → veritabanı → şema; "↔" ile yer değiştirme)
    - seçenekler (sahiplik/yetkiler, tablespace, yorumlar)
    - "Karşılaştır" ve iş sürerken "İptal"
    - ilerleme çubuğu
  - **Liste:**
    - Sanallaştırılmıştır (TanStack Virtual).
    - Durum filtresi vardır (yalnız kaynakta / yalnız hedefte / farklı / aynı / sorunlu) ve her durumun sayısı gösterilir.
    - Tür ve ad araması vardır.
    - Seçim kutuları: tümü, filtredekiler ve tek tek.
    - Her satırda risk rozeti (mevcut Designer rozetleri) ve not ya da uyarı simgesi bulunur.
  - **Ayrıntı paneli:**
    - Seçili öğenin satır farkı (eklenen/silinen satırlar renkli, salt okunur) gösterilir.
    - Öğenin notları ve hata iletileri gösterilir.
  - **"Betik oluştur":**
    - Önizleme diyaloğunda CodeMirror salt okunur SQL gösterilir.
    - En yüksek risk ve uyarılar da gösterilir.
    - Eylemler: "Query Tool'da aç" (hedef sunucu ve veritabanında yeni sekme, metin taslak olarak gelir, çalıştırılmaz), "Kopyala" ve "Kaydet…" (.sql).
- **Kilit:** hedef sunucu üretim ya da kilitli etiketliyse, önizlemede bu bilgi belirgin biçimde gösterilir. Çalıştırma kapısı Query Tool'dadır; sekme ek bir kapı koymaz.
- **Paket bütçesi:** ana paket tavanı **yükseltilmez** (kullanıcı kararı 2026-10-31). Menü girişleri ve sekme kaydının maliyeti, yalnız tembel parçaların kullandığı kodun ana paketten çıkarılmasıyla telafi edilir. Telafi yetmezse durulup sorulur.

## 3. Test

- **Saf birimler (Rust):**
  - eşleştirme
  - normalizasyon: şema adı, yok sayılan alanlar, seçenekler
  - durum kararı
  - betik sıralaması: FK sonrası, görünüm bağımlılık sırası, silme sırası, transaction kararı
  - Myers satır farkı
  - başlıkta gizli bilgi olmaması
  - Snapshot testleri `insta` ile yazılır.
- **Docker (pgt-matrix, PG 10 / 13 / 16 / 18):**
  - İki şema kurulur ve karşılaştırılır.
  - Üretilen betik hedefe uygulanır, sonra yeniden karşılaştırılır. Beklenen sonuç: "aynı" dışında öğe kalmaması. Seçilmeyen ya da "elle gerekiyor" öğeler hariç tutulur.
  - Çapraz sürüm denenir (kaynak PG 16 → hedef PG 13): üretilemeyen öğeler doğru iletiyle dönmelidir.
  - Uzantı nesneleri listede görünmemelidir.
  - Kaptanlar `pgt-matrix-*` adıyla kurulur ve her zaman silinir.
- **Ön yüz (vitest):** liste modeli (filtre, sayım, seçim, varsayılan seçim kuralı), seçici durumu ve "aynı şema" reddi test edilir.
- **Elle:** `docs/smoke-test.md` dosyasında "Şema karşılaştırma (Plan 30)" başlığı altında `[—]` maddeleri yer alır.

## 4. Performans hedefi

Yerel ağda taraf başına 500 tablo ve 2.000 nesne için yükleme ve karşılaştırma 10 saniyenin altında bitmelidir. Yükleme nesne başına sorgu yaptığı için bu bir hedeftir, test değildir. Aşılırsa liste sorgularının toplu yüklemeye çevrilmesi sonraki iştir.

## 5. Bilinen sınırlar (ilk sürüm)

- Veri karşılaştırması yoktur; yalnız yapı karşılaştırılır.
- Yeniden adlandırma algılanmaz.
- Bağımlılık tamamlama yoktur (§2.6). Seçilmemiş bağımlılık, betik çalışırken hata verir.
- Bölüm alt tabloları ve kalıtım çocukları listelenir ama "elle gerekiyor" olarak işaretlenir (üretici ATTACH/INHERITS kurmaz). Yalnız kaynakta olan bölümlenmiş üst tablo da elle oluşturulur.
- Policy, rule ve tablo düzeyi `unsupported` özellikler modellenmez. Fark varsa uyarı verilir, betik üretilmez.
- Değişen bir kolon tipine bağımlı görünüm varsa `ALTER COLUMN TYPE` hedefte hata verir. Betik bağımlı görünümü kendiliğinden düşürüp yeniden kurmaz.
- Gövdelerde kaynak şemayı açıkça niteleyen adlar çevrilmez (§2.3).
- Veritabanı düzeyi nesneler (uzantı, rol, tablespace) kapsam dışıdır.

## 6. Plan 29 sözleşme notları (Plan 30 için)

Plan 30'un tükettiği yüzey yalnız bu bölümdeki komutlar ve `src/ipc/bindings/Diff*.ts` türleridir (alanlar camelCase, enum değerleri snake_case). Arka uç `crates/pgt-diff`, komutlar `src-tauri/src/schema_diff.rs`.

### 6.1 Komutlar

Tauri'de bağımsız değişkenler camelCase yazılır (`diffId`, `serverId`, `itemKey`, `itemKeys`, `onEvent`). `diffId` istemcinin ürettiği UUID'dir.

| Komut | Bağımsız değişkenler | Dönüş |
|---|---|---|
| `schema_diff_databases` | `serverId` | `DiffDatabase[]` (`{ oid, name }`; bağlanılabilir, şablon olmayan, ada göre C sırası) |
| `schema_diff_start` | `diffId`, `source: DiffSide`, `target: DiffSide`, `options: DiffOptions`, `onEvent: Channel<DiffEvent>` | `DiffResult` (iş bitince; sonuç kayıt defterine konur) |
| `schema_diff_cancel` | `diffId` | `void` (çalışan işi iptal eder; hazır kayda ya da bilinmeyen kimliğe dokunmaz, hata vermez) |
| `schema_diff_detail` | `diffId`, `itemKey` | `DiffDetail` |
| `schema_diff_script` | `diffId`, `itemKeys: string[]` | `DiffScript` |
| `schema_diff_save` | `diffId`, `path` | `void` (son üretilen betiği yazar) |
| `schema_diff_close` | `diffId` | `void` (iptal + kayıttan sil; bilinmeyen kimlik hata değildir) |

- `DiffSide = { serverId, databaseOid, schema }`; `DiffOptions = { ownership, tablespace, comments }` (varsayılan: `false`, `false`, `true`).
- `schema_diff_start` iş bitene kadar döner; ilerleme `onEvent` üzerinden akar. İptal edilen iş `cancelled` hatasıyla döner (kısmi sonuç yok).
- Bağlantı: iki taraf da bağlı sunucu kayıtlarını kullanır; bağlı olmayan sunucu `not_connected` döner (parola isteme akışı Plan 30'da mevcut desenle).

### 6.2 Hata kodları

Hata yükü `ErrorPayload { code, message, detail?, hint? }`; iletiler ad, SQL ya da yol içermez.

| `code` | Ne zaman | `detail` |
|---|---|---|
| `same_schema` | Sunucu, veritabanı oid'i ve şema adı (bayt düzeyinde, büyük/küçük harf duyarlı) aynı; iş başlamaz, olay yok | - |
| `schema_not_found` | Şema yok | `source` veya `target` |
| `system_schema` | `information_schema` ya da adı `pg_` ile başlayan şema (`pg_catalog`, `pg_toast`, geçici şemalar) | `source` veya `target` |
| `cancelled` | `schema_diff_cancel` / `close` / sayfa yenileme ya da kanalın kapanması | - |
| `busy` | Aynı `diffId` ile çalışan ya da hazır kayıt var | - |
| `diff_not_found` | `detail` / `script` / `save` için kayıt yok (çalışıyor, kapatılmış, `MAX_READY` ile düşmüş ya da sayfa yenilenmiş) | - |
| `item_not_found` | `itemKey`/`itemKeys` sonuçta yok (`detail`, `script`) | - |
| `not_selectable` | Seçilen öğelerden biri betiğe alınamaz (`selectable == false`) | - |
| `empty_selection` | `itemKeys` boş | - |
| `no_script` | `save`: önce başarılı bir `script` gerekir | - |
| `invalid_path` | `save`: yol boş / denetim karakteri içeriyor / Windows'ta mutlak değil / uzantısı `.sql` değil (büyük/küçük harf duyarsız; "Dosya uzantısı .sql olmalı.") | - |
| `write_failed` | `save`: dosya yazılamadı (yol iletiye yazılmaz) | - |
| `not_connected`, `connection_lost`, ... | Bağlantı hataları `pgt-core` eşlemesiyle | - |

### 6.3 `DiffEvent`

Tek varyant: `{ "event": "progress", "side": "source" | "target", "loaded": n, "total": m }`.

- Yükleme taraf başına bağımsızdır; iki taraf eşzamanlı yüklenir, olaylar iç içe gelir.
- Olay her `PROGRESS_EVERY = 25` nesnede ve sonda gelir; her tarafta sayaçlar azalmaz ve son olayda `loaded == total`. Boş şema tek `(0, 0)` olayı verir.
- Toplam, o tarafın liste sorgusu bittikten sonra bilinir; ilk olaydan önce çubuk belirsiz (indeterminate) gösterilmelidir.
- Kanal gönderimi başarısız olursa (pencere gitti) iş iptal edilir.

### 6.4 `DiffItem` alanları

- `key`: opak; yalnız eşitlik ve komutlara geri gönderme için. Biçimine güvenilmez.
- `kind` (`ObjectKind`): kaynakta varsa kaynağın, yoksa hedefin türü. `targetKind`: yalnız tür değiştiyse (ilişki türü ya da tip gövdesi: ör. kaynakta tablo, hedefte görünüm) hedefteki tür, aksi halde `null`.
- `name`: şema-içi ad (şema niteleyicisi yok). `table`: indeks ve trigger için bağlı tablo adı. `signature`: fonksiyon / prosedür için yalnız giriş tipleri (`oidvectortypes`), diğerlerinde `null`.
- `status`: `only_source` (hedefte oluşturulur), `only_target` (hedeften silinir), `different` (hedef değiştirilir), `same` (varsayılan olarak gizlenir; filtreyle görünür).
- `problem`: `null` ya da `manual` (üreticiyle ifade edilemiyor), `load_failed` (nesne yüklenemedi), `build_failed` (hedef sürümde üretilemedi).
- `risk`: `safe` < `destructive` < `data_loss`; `same` öğede her zaman `safe`.
- `notes`: bilgi / uyarı iletileri; `errors`: yalnız `load_failed` ve `build_failed` için hata iletileri.
- `selectable = problem == null && status != same`. `defaultSelected = selectable && status ∈ {only_source, different}`; yani yalnız hedefte olan (DROP) öğeler seçilebilir ama varsayılan seçili değildir.

### 6.5 `problem` ve `warnings` iletileri

- `manual` (`notes`): "Fark üreticiyle ifade edilemiyor."; "Bölüm alt tablosu (ATTACH modellenmez)"; "Kalıtım çocuğu (INHERITS modellenmez)"; "Bölümlenmiş tablo oluşturma modellenmez"; "Bölümleme anahtarı farklı"; "Modelin dışında özellikler var: …" (yalnız kaynak ya da tür değişimi); "Modelin dışında fark var: …" (iki tarafta `unsupported` listeleri farklı); "Gizli değer içeren ifade betiğe alınamaz."; "Sorgu değişti; görünüm yeniden kurulursa (DROP + CREATE) hedefteki şu ekler kaybolur, elle uygulayın: …" (hedef görünümde INSTEAD OF trigger / kural / kolon varsayılanı, §2.4).
- `load_failed` (`errors`): "Kaynak: <ileti>" / "Hedef: <ileti>" (`pgt-core` hata eşlemesi; yalnız nesne düzeyi retleri, ör. sistem nesnesi). Durum, öğenin varlığına göre `only_source` / `only_target` / `different` kalır.
- `build_failed` (`errors`): üretici alan iletileri, ör. "… PG 15+ gerektirir." (kaynak PG 16, hedef PG 13 ve `UNIQUE NULLS NOT DISTINCT`).
- `notes` ayrıca şunları taşır: "Tür değişti: hedefteki nesne silinip yeniden oluşturulur", "Yeniden kurulacak (DROP + CREATE)", "Kolon sırası farklı (ALTER ile değiştirilemez; yeni kolonlar sona eklenir).", "Gövdede kaynak şema adı açıkça geçiyor; gözden geçirin." ve üreticilerin uyarıları (ör. yetkilerin yeniden verilmesi).
- `DiffResult.warnings`: şema adları farklıysa "Gövdelerdeki nitelikli adları gözden geçirin: …"; ana sürümler farklıysa "Kaynak PostgreSQL X, hedef Y: ifadeler hedef sürüme göre üretilir.". `DiffScript.warnings` ayrıca transaction'sız betik uyarısını ve üretici uyarılarını taşır.

### 6.6 Kayıt defteri, ayrıntı ve betik

- `MAX_READY = 8`: bellekte en çok 8 hazır karşılaştırma tutulur; en eskisi düşer ve ona yapılan istekler `diff_not_found` döner. Plan 30 sekme kapanınca ve yeni karşılaştırma başlayınca `schema_diff_close` çağırmalıdır.
- Sayfa yenilenince (ve uygulama kapanırken) kayıt defteri temizlenir, çalışan işler iptal edilir; yenileme sonrası eski `diffId`'ler `diff_not_found` verir.
- `DiffDetail.lines`: iki metnin satır toplamı `MAX_DIFF_LINES = 20_000`'i aşarsa `null`; arayüz `sourceText` ve `targetText`'i yan yana düz göstermelidir. Metinler hedef sürüm bağlamıyla üretilir ve iki yanda aynı biçimdedir.
- `DiffScript`: `sql`, `transactional`, `warnings`, `risk` (en yüksek), `statements`. Betik yalnız seçime göre üretilir; seçim sırası önemsizdir, sıra §2.6'ya göre belirlenir (gidiş-dönüş testi anahtarları ters sırada verir). Başlıkta yalnız `veritabanı.şema` adları, hedef sürümü ve UTC zaman damgası vardır. `schema_diff_save` son başarıyla üretilen betiği yazar.
- Karşılaştırma salt okunurdur; betik uygulanmaz. Uygulama Query Tool'un mevcut kapılarından geçer.

### 6.7 Bilinen sınırlar (§5'e ek)

- §5'teki sınırlar geçerlidir (bağımlılık tamamlama yok, yeniden adlandırma yok, bölüm / kalıtım çocuğu elle, `ALTER COLUMN TYPE` + bağımlı görünüm, gövdelerde açık şema adı, vb.).
- **Yeniden kurma DROP'ları kendi oluşturma aşamasında kalır** (bağımlılık tamamlama yok, §2.6). Bu yüzden bağımlısı olan bir nesnenin yeniden kurulması uygulanırken hata verebilir: ör. dönüş tipi değişen bir fonksiyona bağlı görünüm, ya da bir FK'nın kaldırıldığı durumda o FK'nın başvurduğu yalnız-hedefteki tablonun silinmesi. Docker gidiş-dönüş fikstürü bu birleşimlerden kaçınır (yaprak görünüm farkı, bağımlısız dönüş tipi farkı).
- **PG 12 öncesinde transaction'sız betik** (`ALTER TYPE … ADD VALUE`) tek çok-ifadeli basit sorgu olarak gönderilemez (sunucu örtük transaction bloğu açar ve "cannot run inside a transaction block" der). Gidiş-dönüş testi bu yüzden enum değeri farkını yalnız PG 12+'ta kurar.
- **PG 12+'ta yeni enum değeri aynı betikte kullanılamaz.** `ALTER TYPE … ADD VALUE` transaction'lı betikte (`BEGIN; … COMMIT;`) çalışır, ama eklenen değer COMMIT'ten önce kullanılırsa (ör. aynı betikteki bir kolon varsayılanı, CHECK ya da görünüm sorgusu bu değeri anıyorsa) sunucu "unsafe use of new value" hatası verir. Betik bu kullanımları ayırmaz; böyle bir betik iki parçada (önce enum değişikliği) uygulanmalıdır.
- **Silme aşaması yalnız görünüm/MV ve tablo bağımlılıklarını sıralar** (`DropView` hedefteki `pg_depend` sırasının tersiyle, `DropTable` FK'larla). Tip/domain arası bağımlılıklar (ör. bir domain'in temel aldığı tipin silinmesi) ve rutin ile tablo varsayılanı arasındaki bağımlılıklar (varsayılanı silinen fonksiyonu çağıran tablo) silmelerde sıralanmaz; silmeler varsayılan olarak seçili olmadığı için bu, kullanıcının bilerek seçtiği birleşimlerde hataya yol açabilir.
- **Satır tipine bağlı rutinin taşınması tek yönlüdür** (§2.6 4.4 / 4.7). Taşınan rutini kullanan bir tablo varsayılanı ya da CHECK ifadesi (aynı betikte oluşturulan başka bir tabloda) tablolar aşamasında rutin henüz yokken çalışır ve hata verir; bu döngüsel birleşim elle sıralanmalıdır.
- **Yeniden kurulan MV'nin yalnız değişmeyen indeksleri yeniden oluşturulur.** Aynı MV'nin farklı (`different`) indeks öğesi kendi üreticisiyle (`DROP INDEX` + `CREATE INDEX`, `ALTER INDEX` ya da `COMMENT`) değiştirilmeye çalışılır; DROP MATERIALIZED VIEW indeksi zaten düşürdüğü için bu öğe seçiliyse betik uygulanırken hata verir ve işlem geri alınır (takip işi: farklı MV indekslerini de kaynaktan yeniden oluşturmak). Yalnız kaynakta olan (`only_source`) indeks kendi öğesiyle MV'den sonra oluşturulur.

### 6.8 Plan 30 için sözleşme notu: Query Tool'a aktarım

`DiffScript.transactional == false` olduğunda (PG < 12 hedefte `ALTER TYPE … ADD VALUE` ya da başka transaction'sız ifade) betik tek çok-ifadeli sorgu olarak gönderilmemelidir: Query Tool bu ifadeleri **ayrı ayrı** (ifade başına) çalıştırmalıdır; betik başlığındaki uyarı ("Betik transaction'sız; hata olursa kısmen uygulanmış kalır.") önizlemede görünür tutulur. Transaction'lı betik (`BEGIN; … COMMIT;`) olduğu gibi çalıştırılabilir. Betik metni sekmeye çalıştırılmadan, taslak olarak konur.

**İptal sırası:** `schema_diff_start` işi kayıt defterine ilk `await`'ten önce kaydeder ve iş bitene kadar dönmez; kayıttan **önce** gelen `schema_diff_cancel` (henüz bilinmeyen kimlik) hiçbir şey yapmaz ve sonra kaydolan iş iptal edilmeden sürer. İlk `onEvent` (ilerleme) bildirimi kaydın tamamlandığını kanıtlar. Plan 30 bu yüzden: ilk bildirim geldikten sonra iptali doğrudan `schema_diff_cancel` ile gönderir; ilk bildirimden önce istenen iptali bekletir ve ilk bildirim gelir gelmez gönderir; hiç bildirim gelmeden `start` dönerse (ör. `same_schema`, bağlantı hatası) sonucu yok sayar. `start` dönmeden kapatılan sekmede istemci aynı kuralla iptal eder, dönen sonucu yok sayıp `schema_diff_close` çağırır.

---

## 7. Plan 30 uygulama notları

Plan: `docs/superpowers/plans/2026-10-31-schema-diff-p30-on-yuz.md` (dal `feat/schema-diff-ui`). Elle kontrol: `docs/smoke-test.md` "Şema karşılaştırma (Plan 30)"; testler: `docs/testing.md` "Şema karşılaştırma ön yüzü testleri (Plan 30)".

### 7.1 Kararların özeti

1. **Menü:** "Şema karşılaştır…" (`schemaDiff`, grup 3) sistem dışı şema ve veritabanı düğümlerinde görünür; karşılaştırma salt okunur olduğundan kilitli sunucuda da açıktır. Sıra: `…, backup, restore, schemaDiff, dropObject`.
2. **Sekme:** her menü çağrısı yeni sekme açar (`s` önekli kimlik, kayıt `DiffTab { id, title }`). Başlık tembel parçada üretilir ve karşılaştırma bitince "‹kaynakDb›.‹kaynakŞema› ↔ ‹hedefDb›.‹hedefŞema›" olur. Sekmeler kalıcı değildir.
3. **Tembel üç kat:** ana pakette yalnız menü eylemi, sekme kaydı ve `diff-open` yükleyicisi (`once(() => import(…))`, boşta ısıtma listesinde değil) vardır. `DiffTabs` (sekme başlıkları + denetleyici + açıcı; `solid-js`, `solid-js/store`, `strings-diff`, fark modülleri ve yalnız ana pakette zaten bulunan `server-form`, `ipc/errors`, `app/clipboard` (pano, `diff-opener.ts`) ve `@tauri-apps/api/core` (`Channel`)) → `SchemaDiffView` (`lazy`; `ui/*`, `RiskBadge`, `lucide-solid`, sanallaştırma) → `ScriptDialog` (`lazy`; `Dialog`, CodeMirror, `plugin-dialog`). Böylece `QueryTabs`'taki `import()`'ın `__vite__mapDeps` listesi yalnız `DiffTabs` ile rolldown çalışma zamanı kadar büyür.
4. **Seçiciler ve giriş noktası:** sunucu listesi denetleyicinin `servers()` erişimcisidir (görünümler yalnız fark denetleyicisini alır). Sunucu seçimi gerekirse parola ister; seçici yanıtları kuşak sayacıyla korunur; üst seçici değişince alttakiler sıfırlanır. Şema düğümünden açılışta kaynak dolu, hedef aynı veritabanında şemasız; veritabanı düğümünden açılışta kaynak şeması boştur.
5. **Aynı şema:** sunucu, veritabanı ve şema eşitse "Karşılaştır" kapalıdır; arka ucun `same_schema` kodu aynı metne eşlenir.
6. **Bayat sonuç:** sonuç varken (ya da karşılaştırma sürerken) seçici veya seçenek değişirse sonuç bayattır (`stale = true`; çalışırken değişiklik de sonucu bayat getirir, çünkü sonuç artık seçimlerle eşleşmez). Liste görünür kalır, "Seçimler değişti; yeniden karşılaştırın." şeridi görünür, "Betik oluştur" kapanır.
7. **Çalıştırma ve iptal:** `diffId = crypto.randomUUID()`; önceki `diffId` önce kapatılır. `start`'tan önce iki tarafın sunucusuna bağlanılır (7.4). İptal ve kapatma §6.8 iptal sırasına uyar: ilk `onEvent`'ten önce istenen iptal hemen bir kez gönderilir, ayrıca bekletilir ve ilk olayda yeniden gönderilir (en çok iki çağrı); olaydan sonra doğrudan gönderilir; olay gelmeden `start` dönerse bekleyen iptal düşer. Çalışırken kapatılan sekmede `start` dönünce `schemaDiffClose` çağrılır. Kısmi sonuç yoktur.
8. **Liste:** her öğe tek kategoridedir ("Sorunlu" durumdan baskın); varsayılan filtre "Aynı" dışındakilerdir; arama `tr-TR` katlamasıyla tür etiketi, tablo, ad ve imzada arar; sıra arka ucun sırasıdır.
9. **Seçim:** ilk seçim `defaultSelected`'dır; "Tümü" ve "Görünenler" üç durumlu kutulardır. **Gizlenen seçili öğeler betiğe girer**; özet "N seçili (M görünmüyor)" olarak bunu gösterir.
10. **Ayrıntı:** odaklanan öğe 120 ms gecikmeyle istenir ve `diffId` başına önbelleğe alınır; birleşik satır farkında renk tek başına bilgi değildir (`+`/`−` işaret sütunu); 20.000 satır üstünde iki sütunlu düz görünüm.
11. **Betik önizleme:** risk rozeti, ifade sayısı, transaction bilgisi, uyarılar ve salt okunur CodeMirror. Üretim ve kilit hedef bantları yalnız bilgidir; sekme ek kapı koymaz, çalıştırma Query Tool'un risk/kilit kapılarından geçer. "Query Tool'da aç" betiği hedefte yeni sekmeye çalıştırmadan koyar.
12. **Kayıttan düşmüş sonuç:** ayrıntı, betik ya da kaydetme `diff_not_found` alırsa sonuç temizlenir ve "Sonuç artık yok (en fazla 8 karşılaştırma tutulur ya da sayfa yenilendi); yeniden karşılaştırın." şeridi görünür.
13. **Kapsam dışı:** betiğin sekmeden uygulanması, sekme kalıcılığı, sonucun dışa aktarılması, öğe başına "yalnız bunu betikle" kısayolu, F5 ile yeniden karşılaştırma.

### 7.2 Uygulama sırasında verilen kararlar

- **İptal sırası:** plan metni yalnız `schemaDiffCancel` diyordu; denetleyici bağlayıcı §6.8 iptal sırasını uygular (7.1 madde 7).
- **Çalışırken değişiklik:** karşılaştırma sürerken seçim ya da seçenek değişirse dönen sonuç `stale = true` gelir (plan özeti `false` diyordu).
- **`servers()`:** `DiffTabCtl`'e `servers(): ServerConfig[]` erişimcisi eklendi; görünümler yalnız fark denetleyicisini alır (tembel parçada, ana pakete maliyeti yok).
- **`riskTone` kaldırıldı:** plan arayüzünde vardı ama kullanılmıyordu (ölü kod).
- **`createStore` kaldı:** sekme başına store hataya daha az açıktır; yeni dışa aktarma adının ana paketteki ~8 B'lık bedeli 7.3'te telafi edildi.
- **Transaction'sız betik (§6.8):** Query Tool ifadeleri zaten birer birer çalıştırır (`crates/pgt-query/src/exec.rs` başlığı); `transactional == false` betik için ek iş gerekmedi, betik başlığındaki uyarı önizlemede görünür.

### 7.3 Paket ölçümleri ve telafi kaydı

`bun run bundle:check` ile ölçüldü (bayt):

| Ölçüm | Plan 30 öncesi tavan | Task 1–4 sonu | Telafi sonrası (yeni tavan) |
|---|---|---|---|
| Ana paket | 127,333 | 128,666 (+1,333) | 127,314 |
| Açılış kümesi | 155,006 | 156,339 (+1,333) | 154,987 |
| `__vite__mapDeps` girdisi | — | 39 | 39 |

Artışın kaynağı: Task 1 +1,301 (menü eylemi, `tabs-store` fark listesi / `openSchemaDiff` / `isDiffTabId`, `QueryTabs` bağlantısı, `App` yükleyicisi, `t.menu.schemaDiff`); Task 3 +16 (`createStore` yeni dışa aktarma adı, `Channel` ve `diff-commands`'ın `DiffTabs` parçasına çekilmesi); Task 4 +16 (rolldown yükleyicisi `.then(e => e.t)` ve `__vite__mapDeps`'te `rolldown-runtime`, çünkü tembel `SchemaDiffView` `DiffTabs` parçasından içe aktarır).

Tembel parçalar: `DiffTabs` 18,012 B, `SchemaDiffView` 22,536 B, `ScriptDialog` 3,572 B, `diff-open` 1,464 B.

Telafi (yalnız tembel parçaların kullandığı kod ana paket modüllerinden o parçaya özel modüllere taşındı; davranış ve metinler aynı, testler yeni modüllerden içe aktarır):

| Adım | Taşınan / değişen | Ana paket | Fark |
|---|---|---|---|
| 1 | `workspace-model.ts` → yeni `env-model.ts` (yalnız `tx-dialogs`): `ENV_CHOICES`, `envLockDefault`, `canCommitAny` (`envLabel` `applyEnvChoice`'ta kullanıldığı için kaldı) | 128,474 | −192 |
| 2 | `workspace-model.ts` → yeni `querytab-model.ts` (yalnız QueryTab parçası): `TIMEOUT_CHOICES`, `formatElapsed`, `sameScope`, `swapEditorState`/`SavedView` | 128,007 | −467 |
| 3 | `run-model.ts` `canCancel`, `canCommit`, `canRollback`, `canLoadMoreRows` ve `result-model.ts` `canLoadMore` (yalnız `canLoadMoreRows` kullanıyordu), `LOAD_MORE_CHUNK` → `querytab-model.ts` | 127,525 | −482 |
| 4 | `tabs-store.ts`: beş açma işlevindeki kimlik döngüsü tek `freeId` yardımcısına toplandı | 127,457 | −68 |
| 5 | `tabs-store.ts`: store sarmalayıcısındaki açma yöntemleri `opener`/`reopener` yardımcılarıyla kuruldu | 127,328 | −129 |
| 6 | `QueryTabs.tsx`: `closeDiff` `requestClose` içine alındı | 127,314 | −14 |

Taşınmayanlar: `isUserCancel` (`result-model.ts` içinde kullanılıyor), `MAX_TAB_ROWS` (dosya içinde kullanılıyor), `t.query.ctx` anahtarları (`openTx`, `failedTx`, `locked`, `running` `QueryTabs.tsx`'te, `noServer` `workspace-model.ts`'te kullanılıyor; taşınacak anahtar yok), `errorText` (son çare; gerek kalmadı). Tembel parçalar büyüdü: QueryTab 117,511 → 118,349 B, `tx-dialogs` 5,110 → 5,263 B. Tavanlar ölçülen değerlere indirildi (sıfır pay).

### 7.4 Son inceleme düzeltme turu

- **Karşılaştırmada bağlantı (§2.7):** `compare` `phase = "running"`'i yazıp (kapı çift tıklamayı engeller) `schemaDiffStart`'tan önce kaynak ve hedef sunucusu için `deps.connect`'i (sunucu başına bir kez, sırayla; gerekirse parola istemi) çağırır. `false` ya da hata → "Sunucuya bağlanılamadı.", `start` yok. Her `await` sonrası sekme kapandıysa `start` yok (iptal/kapatma IPC'si de yok); bağlanırken iptal istendiyse `start` yok ve "İptal edildi.". `not_connected`, `connection_lost`, `unknown_server` sabit Türkçe metinlere eşlenir.
- **İptal:** `start` çağrıldıktan sonra, ilk olaydan önce istenen iptal hemen de bir kez gönderilir (bilinmeyen kimlikte etkisiz); bekleyen iptal ilk olayda yine gönderilir (en çok iki çağrı).
- **Karşılaştırılan hedef:** modelde `comparedTarget` (başlarken alınan anlık görüntü); betik diyaloğunun üretim/kilit bandı seçiciyi değil bunu okur.
- **"Query Tool'da aç":** `QueryController.openScript` `Promise<void>` döner ve başarıyı bildirmez; dönüş değeri eklemek ana pakete bayt eklerdi (sıfır pay). Ölü `catch` kaldırıldı, diyalog yine kapanır; sınır denetleyicide belgelendi ve elle kontrol listesinde izlenir.
- **Silinmiş sunucu:** `compareBlocked` bir `known` yüklemi alır; `servers()`'ta olmayan sunucuyu tutan taraf eksik sayılır (görünüm ve denetleyici aynı kapıyı kullanır).
- **Erişilebilirlik:** Karşılaştır/İptal tek düğmedir (etiket değişir, odak kaybolmaz; İptal `aria-disabled`, çift tıklamanın ikinci tıklaması iptal etmez); ayrıntı başlığındaki hata bloğu rolsüz, ayrıntı yükleme hatası `role="status"`.
- **Parça yükleme hatası:** `diff-open` ham tarayıcı iletisi yerine `t.app.chunkLoadFailed` gösterir.
- **Dosya adı:** `suggestScriptName` kesimden sonra sondaki boşluk ve noktaları atar.
- **Paket:** ana paket 127,314 B, açılış kümesi 154,987 B, `__vite__mapDeps` 39 (değişmedi; tavanlar aynı).
