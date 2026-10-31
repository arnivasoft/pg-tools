# pg-tools — Şema Karşılaştırma (Schema Diff) Tasarımı

- **Tarih:** 2026-10-31
- **Durum:** Tasarım; Plan 29 (arka uç) ve Plan 30 (ön yüz) yazılacak
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
  - Görünümde sorgu değişikliği `recreate` ile yapılır.
  - Fonksiyonda dönüş tipi ya da argüman adı değişikliği, `needs_recreate` doluysa DROP + CREATE olur. Risk en az `Destructive`'dir ve öğede "yeniden kurulacak" notu yer alır.
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
   5. sequence `OWNED BY`
   6. FK kısıtları: yeni tablolarınki ve değişen tablolarda eklenenler. Bu ifadeler, FK'sız model eski, tam model yeni alınarak `table_alter::alter` ile üretilir.
   7. görünüm ve MV (kaynaktaki `pg_depend` sırasıyla)
   8. indeks
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
