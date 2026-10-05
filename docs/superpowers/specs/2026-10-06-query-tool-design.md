# pg-tools — Alt Proje 2: Query Tool Çekirdeği Tasarımı

- **Tarih:** 2026-10-06
- **Durum:** İnceleme bekliyor
- **Kapsam:** Çoklu sekmeli SQL editörü, sorgu çalıştırma (yazma dahil, korumalarla), sanal ızgarada sonuçlar, iptal, geçmiş, CSV dışa aktarma
- **Önceki alt proje:** Object Explorer (`2026-10-05-object-explorer-design.md`) — Plan 1–4 ile tamamlandı ve `main`'de.
- **Sonraki alt proje:** Gelişmiş editör zekâsı (alias çözümleme, join önerisi, formatter, LSP benzeri hata çizimi) — ayrı spec.

---

## 1. Amaç ve bağlam

Object Explorer salt okunurdu. Query Tool, aynı sunucu kayıt defteri ve bağlantı altyapısı üzerinde SQL yazıp çalıştırmayı ekler. pgAdmin4'ün en çok kullanılan ikinci parçasıdır ve aynı üç sorun burada da belirleyicidir: **açılış hızı**, **bellek**, **büyük sonuçlarda akıcılık**.

### 1.1 Kullanıcıdan gelen kararlar

| Konu | Karar |
|---|---|
| Yazma | **Tam yazma** (varsayılan yazılabilir, autocommit) — korumalarla (§6) |
| Korumalar | Sunucu başına **salt okunur kilit**, **tehlikeli ifade onayı**, **sunucu rengi/etiketi**, **manuel transaction modu** seçeneği |
| Sonuçlar | **Sanal ızgara + sayfalama** (TanStack Virtual, imleçle parça parça çekme, satır sınırı, iptal, CSV) |
| Editör modeli | **Çoklu sekme**, sekme başına ayrı oturum; taslaklar oturumlar arası saklanır |
| Editör zekâsı (bu alt proje) | Vurgulama + **temel** otomatik tamamlama (anahtar kelime, katalogdan şema/tablo/kolon), Ctrl+Enter, sorgu geçmişi |
| Gelişmiş zekâ | Ayrı alt proje (alias çözümleme, join önerisi, formatter, hata çizimi) |
| Teknoloji | Object Explorer ile aynı (Rust, Tauri 2, SolidJS, Kobalte, Tailwind 4, CodeMirror 6, TanStack Virtual) |

### 1.2 Varsayımlar

- Aynı istemci/sunucu hedefleri: Windows 10/11, Server 2019/2022; PostgreSQL 10–18 vanilla.
- Aynı anda ~10 açık sekme; sekme başına tek ayrı bağlantı.
- Tipik sonuçlar < 10.000 satır; üst uç 1M satırlık tabloyu `SELECT *` etmek hatasına karşı korunur.

### 1.3 Başarı ölçütleri

| Ölçüt | Hedef |
|---|---|
| Boşta toplam özel çalışma kümesi (Query Tool açılmamışken) | < 150 MB (Object Explorer hedefi korunur) |
| Query Tool sekmesi açma (CodeMirror dahil, ilk kez) | < 500 ms |
| Basit sorguda (LAN, < 1.000 satır) ilk satırın ekranda görünmesi | < 150 ms |
| 100.000 satırlık sonucu kaydırma | 60 fps |
| 1.000.000 satırlık sorgu | Bellekte en fazla satır sınırı (varsayılan 10.000) + kullanıcı "daha fazla yükle" dedikçe; hiçbir durumda sınırsız birikim yok |
| İptal düğmesi → sorgu durdu | < 1 sn (LAN) |
| Açık Query Tool + 10.000 satır, toplam özel çalışma kümesi | < 250 MB |

### 1.4 Kapsam dışı (bu alt proje)

- Gelişmiş editör zekâsı (ayrı spec), SQL biçimlendirici.
- `psql` meta-komutları (`\d`, `\copy`…), `COPY ... FROM STDIN` / `COPY TO STDOUT` akışı.
- Sonuç ızgarasında hücre düzenleme (veri düzenleyici ayrı alt proje), `EXPLAIN` görselleştirme (düz metin gösterilir), grafikler.
- Sorgu planı karşılaştırma, bildirim (`LISTEN/NOTIFY`) dinleme, büyük nesne (large object) işleme.
- Birden çok sonuç kümesini aynı anda sekmeli gösterme: komut dizisinde (script) her ifadenin sonucu sırayla bir "sonuç şeridinde" listelenir (§5.4), karşılaştırmalı görünüm yok.

---

## 2. Mimari

### 2.1 Yeni bileşenler

```
crates/pgt-query/         (yeni)  ifade ayırıcı, sınıflandırıcı, çalıştırıcı (oturum + imleç + iptal)
crates/pgt-core/                  ServerConfig'e color/label/read_only_lock; ayrı oturum açma API'si
src-tauri/src/query.rs    (yeni)  Tauri komutları + Channel olay akışı
src/features/query/               sekmeler, editör, ızgara, geçmiş (tembel yüklenir)
```

### 2.2 İlkeler

- **Object Explorer'a dokunulmaz:** katalog sorguları hâlâ salt okunur havuz bağlantılarını kullanır. Query Tool'un oturumları havuzdan **ayrıdır**; kullanıcı yazarken veya transaction açıkken ağaç etkilenmez.
- **Tembel yükleme:** CodeMirror, ızgara ve Query Tool arayüzü ilk sekme açılana kadar yüklenmez; yoksa boşta bellek hedefi bozulur.
- **Bellek sınırı sözleşmesi:** arka uç hiçbir zaman istemciye sınırsız satır göndermez; ızgara en fazla yüklenmiş satır sayısını tutar.
- **Parola ve bağlantı bilgisi** hiçbir günlüğe, geçmişe veya hata metnine yazılmaz; sorgu metni **günlüğe yazılmaz** (yalnızca süre ve sonuç sayısı).

---

## 3. Oturumlar ve bağlantı (`pgt-core` + `pgt-query`)

- Her Query Tool sekmesi bir **QuerySession**'dır: `(sunucu, veritabanı)` çifti için **ayrı, kalıcı** bir bağlantı (havuza dönmez). Sekme kapanınca bağlantı kapanır; 30 dk boşta kalan oturum "uyku" durumuna alınıp bağlantı kapatılır, sonraki çalıştırmada sessizce yeniden açılır (açık transaction yoksa; varsa kullanıcıya bildirilir ve sekme "bağlantı koptu" durumuna geçer).
- Oturum ayarları (Object Explorer'dan farklı): `application_name = 'pg-tools query'`; `statement_timeout` **varsayılan 0 (sınırsız)**, sekme başına ayarlanabilir (30 sn / 5 dk / sınırsız); `default_transaction_read_only` **sunucunun kilidine göre** (`on` kilitliyse, değilse `off`); `search_path` **dokunulmaz** (kullanıcının kendi yolu geçerli).
- **Kilitli sunucuda** oturum `default_transaction_read_only = on` ile açılır ve istemci taraf da yazma ifadelerini önceden reddeder (§6). Kilit, sekme açıkken değiştirilirse sekme yeniden bağlanmaya zorlanır ve kullanıcı bilgilendirilir.
- Sürüm, bağlantıda gelen `server_version_num`'dan okunur; veritabanı bazlı bağlantı Object Explorer ile aynı OID çözümlemesini kullanır.
- **İptal:** `tokio_postgres::Client::cancel_token()` ile ayrı bağlantı üzerinden `CancelRequest`; TLS'li bağlantılarda aynı `MakeRustlsConnect`. İptal sonrası oturum kullanılabilir kalır (aktif transaction `aborted` durumuna düşebilir; arayüz bunu gösterir).

## 4. Çalıştırma motoru (`pgt-query`)

### 4.1 İfade ayırıcı

- Rust'ta yazılmış, **sözlüksel** (parser değil) ifade ayırıcı: `;` ile böler; tek tırnak (`'..'`, `E'..'` kaçışları), çift tırnaklı tanımlayıcılar, `$tag$..$tag$` dolar tırnağı, `--` satır yorumu, `/* */` iç içe blok yorumu ve `BEGIN ATOMIC ... END` (PG 14+ SQL gövdeli fonksiyonlar) içindeki `;`'leri bölmez. Her ifade için bayt aralığı (editörde vurgulama/imleçteki ifade için) döner.
- İmleçteki ifade: Ctrl+Enter, seçim varsa seçimi, yoksa imleci içeren ifadeyi; "Tümünü çalıştır" (F5) bütün metni sırayla çalıştırır.

### 4.2 Sınıflandırıcı

İfadenin ilk anlamlı belirteci (yorumlar atlanır) ve basit tarama ile: `Select-benzeri` (SELECT/WITH…SELECT/VALUES/TABLE/SHOW/EXPLAIN [ANALYZE yoksa]), `Write` (INSERT/UPDATE/DELETE/MERGE/COPY…FROM), `Ddl` (CREATE/ALTER/DROP/TRUNCATE/COMMENT/GRANT/REVOKE…), `Tx` (BEGIN/COMMIT/ROLLBACK/SAVEPOINT…), `Other`. `WITH … INSERT/UPDATE/DELETE` yazma sayılır (sözlüksel tarama; kesin değil — sınıflandırıcı yanılırsa kilitli sunucuda sunucu zaten reddeder, çünkü oturum salt okunurdur). `EXPLAIN ANALYZE` yazma ifadesi için `Write` sayılır.

### 4.3 Çalıştırıcı

Bir ifade için:
1. **Hazırla:** `client.prepare(sql)` → sütun adları ve tür OID/adları (ızgara başlığı, hizalama, hücre biçimi için), parametre yok.
2. **Satır döndüren** ifade (hazırlanan ifadenin sütunu varsa): kullanıcı autocommit modundaysa açık bir transaction yoksa kısa süreli `BEGIN` ile; `DECLARE pgt_c NO SCROLL CURSOR FOR <sql>`; `FETCH FORWARD <parça> FROM pgt_c` (parça = 1.000 satır) `simple_query` protokolüyle **metin biçiminde** — her tür (numeric, jsonb, dizi, aralık, bytea, geometrik…) için ek kod gerekmeden gösterilir; `NULL` ile boş dize ayrılır (`Option<&str>`). Satır sınırına (varsayılan 10.000) ulaşınca durulur, imleç açık tutulur ve "daha fazla yükle" ile devam edilir; sekme kapanır/yeni ifade çalışırsa `CLOSE`/`ROLLBACK`.
   - `DECLARE … CURSOR` satır döndüren ama imleçle açılamayan ifadelerde (`INSERT … RETURNING`, `EXPLAIN`, `SHOW`, `VALUES`'ın bazı biçimleri; SQLSTATE `42P11` veya sözdizimi) **doğrudan** `simple_query` ile çalıştırılır ve sonuç, satır sınırında kesilerek toplanır (uyarı: yalnızca bu yolda tüm satırlar sunucudan istemciye gelir, bellekte kesilir). Bu iki yolun hangi ifadeler için kullanıldığı test edilir (§9).
3. **Satır döndürmeyen** ifade: `simple_query` → etkilenen satır sayısı (`CommandComplete` etiketi) ve süre.
4. **Notice/Warning** mesajları (`RAISE NOTICE`, uyarılar) bağlantı mesaj akışından toplanır ve ifadenin sonucuna eklenir.
5. Süre, sunucu istemci tarafında ölçülür (gönderimden ilk satıra / tamamlanmaya).

**Komut dizisi (birden çok ifade):** sırayla çalışır; ilk hatada **durur** (autocommit'te önceki ifadeler uygulanmıştır, kullanıcıya açıkça gösterilir); her ifade bir sonuç öğesi üretir.

### 4.4 Olay akışı (Tauri `Channel`)

`qt_run(session, sql, scope, limit)` bir `Channel<QueryEvent>` ile akıtır:
`Started{stmt_index, kind, columns?}` → `Rows{stmt_index, rows: string|null[][], more: bool}` (parça başına) → `Done{stmt_index, tag, rows_affected?, elapsed_ms, notices[]}` | `Error{stmt_index, ErrorPayload + position}` → `ScriptDone`. `qt_more(session, stmt_index)` imleçten sonraki parçayı ister; `qt_cancel(session)`; `qt_tx(session, begin|commit|rollback)`; `qt_close(session)`. Hata `position` alanı (PG'nin 1 tabanlı karakter konumu) editörde hata konumunu işaretler.

### 4.5 Transaction modları

- **Autocommit** (varsayılan): her ifade kendi transaction'ında; yukarıdaki imleç için geçici `BEGIN` ifade bitince `COMMIT` ile kapatılır (okuma) — yazma ifadelerinde sarmalama yapılmaz.
- **Manuel:** ilk ifadede otomatik `BEGIN`; arayüzde **Commit**/**Rollback** düğmeleri ve "açık transaction" rozeti (süre sayacı). Sekme/uygulama kapanırken açık transaction varsa onay sorulur (kaydet = COMMIT, atma = ROLLBACK, vazgeç). Hatada transaction `aborted` durumuna düşer; arayüz uyarır ve yalnızca Rollback'e izin verir.
- Kullanıcı elle `BEGIN`/`COMMIT` yazarsa durum **ifade sınıflandırmasından** türetilir (tokio-postgres 0.7.18 `ReadyForQuery` durumunu dışarı vermez; kaynak doğrulandı). Durum makinesi `Idle`/`InTransaction`/`Failed`; her `Tx` sınıfı ifade ve her hata sonrası tek bir yeniden eşitleme sorgusu (`SELECT statement_timestamp() <> transaction_timestamp()`; `25P02` ise `Failed`) çalışır ve çelişkide **sunucudan gelen cevap kazanır**. Sınır: fonksiyon/`DO` içi dinamik `COMMIT` sınıflandırmada görünmez; bir sonraki `Tx`/hata eşitlemesine kadar durum eskimiş olabilir.

---

## 5. Arayüz

### 5.1 Düzen

Sağ panel bir **sekme çubuğu** olur: sabit ilk sekme **"Nesne"** (mevcut Inspector: Properties/DDL), ardından Query Tool sekmeleri. Yeni sekme: Object Explorer'da bir sunucu/veritabanı/şema/tablo üzerinde bağlam menüsü → **"Query Tool'da aç"** (Ctrl+Alt+Q): sekme o düğümün `(sunucu, veritabanı)`'na bağlanır; tabloda `SELECT * FROM şema.tablo LIMIT 100;` taslağı önerilir (nitelikli ad `quote_identifiers` ile). Araç çubuğunda "Yeni sorgu".

Sekme içi (üstten alta): **bağlam şeridi** (sunucu adı + renk/etiket, veritabanı, kilit simgesi, transaction modu seçici, statement_timeout seçici, çalıştır/iptal/commit/rollback düğmeleri) → **editör** → bölücü → **sonuç alanı** (şerit: ifade sonuçları listesi; altında ızgara/Mesajlar/Geçmiş sekmeleri).

### 5.2 Editör

CodeMirror 6 (Object Explorer'ın DDL görünümüyle aynı tembel yükleme kalıbı): PostgreSQL vurgulaması (`@codemirror/lang-sql` PostgreSQL lehçesi), satır numaraları, parantez eşleme, çoklu imleç, `Ctrl+Enter` (imleçteki/seçili ifade), `F5` (tümü), `Ctrl+.` (iptal), `Ctrl+/` yorum, `Ctrl+Shift+F` yok (formatter sonraki alt proje). Hata konumu `position`'dan çizilir (kırmızı alt çizgi + gutter işareti).
**Temel otomatik tamamlama:** (a) PostgreSQL anahtar kelimeleri ve ifade kalıpları; (b) **katalogdan** şema, tablo/görünüm, kolon adları — kaynak, Object Explorer'ın yüklü ağacı değil, ayrı hafif sorgulardır: `qt_complete(session, kind, schema?, relation?)` salt okunur sorgusu (`pg_class`/`pg_attribute`, şema başına önbellek, 60 sn tazelik). Bağlamsal olarak yalnızca `schema.` ve `tablo.` önekleri ve `FROM`/`JOIN` sonrası tablo adı; alias çözümlemesi **yok** (sonraki alt proje). Tamamlama listesi kapalı/yüklenmemiş durumda sessizce anahtar kelimelere düşer.

### 5.3 Izgara

TanStack Virtual ile satır **ve** sütun sanallaştırma (geniş sonuçlarda yatay akıcılık); sabit satır yüksekliği 24 px; sütun başlığında ad + tür; tür bazlı hizalama (sayısal sağa); `NULL` soluk italik `[NULL]`; uzun metin kesilir, hücre seçilince altta tam değer paneli (JSON güzel yazdırma, çok satırlı metin); sütun genişliği sürükleme/otomatik sığdırma (ilk 200 satıra göre); satır numarası sütunu; seçim (hücre/satır/sütun) ve `Ctrl+C` (seçimi TSV kopyalar). Yüklenen satır sayısı ve "Daha fazla yükle (10.000)" düğmesi; sınıra gelince açık uyarı. **CSV'ye aktar:** Tauri dialog ile dosya yolu seçilir; **yüklenmiş satırlar değil, sorgu yeniden çalıştırılarak** akıtılır (arka uç imleçle parça parça yazar, bellek şişmez; ilerleme ve iptal) — tam sonuç dışa aktarma ayrı bir bağlantıda değil, aynı oturumda ikinci imleçle yapılır, açık transaction'ı bozmaz.

### 5.4 Sonuç şeridi ve Mesajlar

Komut dizisi çalışınca ifade başına bir satır: `#`, sınıf, durum (✓/✗/⏹), etkilenen satır/dönen satır, süre; satıra tıklayınca ızgara o ifadenin sonucunu gösterir (yalnızca **son çalıştırmanın** sonuçları tutulur; sekme başına bellek sınırı yüklenmiş toplam 50.000 satır — aşılırsa en eski ifadenin sonucu bırakılır ve işaretlenir). **Mesajlar** sekmesi: NOTICE/WARNING, hata ayrıntısı (`detail`, `hint`, `position`), komut etiketi.

### 5.5 Geçmiş

Çalıştırılan her ifade: metin (en çok 10 KB, fazlası kesilir), sunucu/veritabanı adı, zaman, süre, durum. `%APPDATA%\pg-tools\query-history.json`, **en çok 500 kayıt** (en eski silinir), atomik yazım; **Ayarlar'dan kapatılabilir** ("sorgu geçmişini kaydet") çünkü SQL metni hassas veri taşıyabilir (parola içeren `ALTER ROLE … PASSWORD` benzeri ifadeler için sınıflandırıcı `Role/Password` ifadelerini geçmişe **yazmaz**). Geçmiş panelinde arama, tıklayınca editöre ekleme.

### 5.6 Sekme kalıcılığı

Açık sekmelerin taslakları (metin, sunucu/veritabanı kimliği, mod ayarları) `%APPDATA%\pg-tools\query\<uuid>.sql` + `tabs.json`; kapanışta ve 5 sn'lik sessizlikte yazılır; açılışta **bağlantı kurulmadan** (tembel) geri yüklenir, ilk çalıştırmada bağlanılır. Sunucu silinmişse sekme "sunucu yok" durumunda salt-okunur taslak olarak açılır.

---

## 6. Korumalar

- **Salt okunur kilit** (`ServerConfig.read_only_lock: bool`): sunucu diyaloğunda (SSL sekmesinin yanında "Güvenlik") ve içe aktarmada ayarlanır. Kilitliyse oturum salt okunur açılır, `Write/Ddl` ifadeleri çalıştırılmadan önce **istemcide reddedilir** (açıklayıcı mesaj) ve sunucu da zaten reddeder (kemer-askı). Yeni eklenen veya içe aktarılan sunucularda varsayılan **kilitsiz**; ancak sunucu **etiketi `Üretim` seçilirse kilit varsayılan açık gelir** (kullanıcı kapatabilir).
- **Sunucu rengi/etiketi** (`color: Option<String>` — önceden tanımlı 8 renk, `label: Option<String>` — serbest metin, öneriler: Üretim, Test, Geliştirme): Object Explorer'da sunucu satırında renkli nokta; Query Tool sekmesi ve bağlam şeridinde renkli üst çizgi + etiket. pgAdmin içe aktarmada `BGColor` varsa renge en yakın önceden tanımlı renk atanır.
- **Tehlikeli ifade onayı:** çalıştırmadan önce istemci tarafı sınıflandırma ile şunlar için **onay diyaloğu** (ifade metni gösterilir, varsayılan odak "Vazgeç"): `UPDATE`/`DELETE` ifadesi **üst düzeyde `WHERE` belirteci içermiyorsa**; `TRUNCATE`; `DROP` (herhangi bir nesne); `ALTER TABLE … DROP COLUMN`; `DELETE`/`UPDATE` ve diğer yazma ifadeleri **etiketi `Üretim` olan** sunucuda her zaman onaylı (tek tıkla "Bu sekmede bir daha sorma" yok). Sınıflandırma sözlüksel ve **kasıtlı olarak tutucudur** (yanlış pozitif onay, yanlış negatif veri kaybından iyidir); `WHERE true` gibi hileler yakalanmaz — belgelenen sınır.
- **Manuel transaction** (§4.5) ayrı bir güvenlik ağıdır; üretim etiketli sunucuda yeni sekmenin varsayılan modu **Manuel**'dir.

---

## 7. Hata yönetimi

- Hata modeli Object Explorer ile aynıdır (`ErrorPayload`); Query Tool için `position` (karakter konumu) eklenir ve `Error::Query`'de taşınır.
- Sorgu hataları **Mesajlar** sekmesinde ve editörde işaretli konumda gösterilir; modal açılmaz. Bağlantı kopması: sekme "bağlantı koptu" durumuna geçer, yeniden bağlan düğmesi; açık transaction kaybı açıkça bildirilir.
- Panik/yarış: arka uç sekme kapanırken akan olay kanalı kapanır; geç gelen olaylar yok sayılır (sekme kimliği + çalıştırma kimliği eşleşmesi).
- Günlük: yalnızca ifade sayısı, süre, sonuç (ok/hata kodu); SQL metni ve parametre **yazılmaz**.

## 8. Güvenlik notları

- Query Tool kullanıcının yazdığı SQL'i çalıştırır — bu alt projenin amacı budur; Object Explorer'ın "arayüze rastgele SQL açılmaz" ilkesi **yalnızca katalog katmanı için** geçerli kalır ve Query Tool bunun **istisnasıdır** (ayrı crate, ayrı oturumlar, ayrı komutlar).
- Dışa aktarma dosyaları kullanıcının seçtiği yola yazılır; üzerine yazma onaylanır.
- Geçmiş ve taslak dosyaları yerel düz metindir (şifrelenmez); Ayarlar'da geçmiş kapatılabilir ve "geçmişi/taslakları temizle" düğmesi vardır.

## 9. Test stratejisi

- **Ifade ayırıcı/sınıflandırıcı:** tablo odaklı birim testleri (dolar tırnağı, iç içe yorum, `E''` kaçışı, `BEGIN ATOMIC`, boş ifade, sonda `;` yok, Unicode, çok uzun ifade, `WHERE` içeren/içermeyen UPDATE, CTE içi DML) + özellik tabanlı (rastgele birleştirme ile ayırıcı yeniden birleştirince girdiyi verir).
- **Çalıştırıcı (PG 10–18 matrisi):** her ifade türü için sonuç sözleşmesi (SELECT, INSERT…RETURNING, EXPLAIN, SHOW, VALUES, DDL, boş sonuç, çok sütunlu/geniş, 100K satır sınır ve "daha fazla", NULL vs boş dize, `bytea`/`jsonb`/dizi/aralık metin biçimi, NOTICE toplama, hata `position`), iptal (uzun `pg_sleep` → < 1 sn), manuel transaction (BEGIN/COMMIT/ROLLBACK, aborted durum), kilitli sunucuda yazma reddi (istemci + sunucu), `statement_timeout`.
- **Bellek sözleşmesi testi:** 1M satırlık `generate_series` sorgusu çalıştırılır; arka uç sürecinin ve olay kanalının tuttuğu satır sayısı sınırı aşmaz (sayaç testi).
- **Ön yüz:** ızgara model mantığı (sütun genişliği, seçim, TSV serileştirme, satır sınırı/daha fazla), sekme durum makinesi (autocommit/manuel/aborted/bağlantı koptu), tehlikeli ifade onay kararı, geçmiş (500 sınırı, hassas ifade hariç tutma), taslak kalıcılığı — saf modüllerde Vitest.
- **Yayın exe duman testi** (`scripts/smoke-release.ps1`) Query Tool açık biçimde de çalışır (tembel yükleme çökmesin).
- **Elle kontrol listesi** (`docs/smoke-test.md` genişler): renkli üretim etiketi, tehlikeli ifade onayı, manuel transaction ile kapanış onayı, iptal, CSV.

## 10. Açık riskler

1. **İmleç ile metin sonuçları:** `DECLARE … CURSOR` + `simple_query` her ifade türünde çalışmaz (`INSERT … RETURNING`, `EXPLAIN`, `SHOW`); doğrudan yoldaki sonuç tümü gelip bellekte kesildiği için dev bir `INSERT … RETURNING` bellek şişirebilir — sınır olarak sunucu tarafı `RETURNING` boyutu kullanıcı sorumluluğundadır (belgelenir).
2. **Transaction durumu:** tokio-postgres durumu vermediği için sınıflandırma tabanlı durum makinesi + hedefli yeniden eşitleme kullanılır (§4.5); karar Plan 5'te verildi. Kalan risk: dinamik `COMMIT`/`ROLLBACK` içeren fonksiyonlar.
3. **Tehlikeli ifade sınıflandırması sözlüksel:** parser değildir; atlatma mümkündür. Kilit ve etiket mekanizmaları asıl güvence, onay ikincil güvencedir.
4. **Tam yazma varsayılan:** üretim sunucusuna yanlış sekmeden yazma riski yüksek; rengi/etiket/kilit/manuel mod varsayılanları bu nedenle üretim etiketinde sıkıdır. Etiket atanmamış sunucular korunmasızdır — ilk Query Tool açılışında "bu sunucunun ortamını seçin" istemi (Üretim/Test/Geliştirme/Atla) gösterilir.
5. **Bellek:** CodeMirror + ızgara + çoklu sekme boşta bellek hedefini etkilemez ama açıkken artar; ölçüt 250 MB (§1.3) planda ölçülür, aşılırsa sekme başına yüklenmiş satır sınırı düşürülür.
6. **`search_path` dokunulmaz:** katalog tamamlama sorguları kendi `pg_catalog` yolunu açıkça nitelendirmeli (oturum yolundan bağımsız).
