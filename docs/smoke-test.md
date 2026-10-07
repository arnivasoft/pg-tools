# pg-tools — Duman Testi (Plan 1–9)

Her sürümden önce elle uygulanır. Ortam: Windows 10/11 veya Server 2019/2022, PostgreSQL 10–18 sunucusu.

## Sunucu yönetimi
- [ ] "Sunucu ekle" → zorunlu alanlar boşken Kaydet hata gösterir, doğru sekmeyi açar
- [ ] "Bağlantıyı test et" doğru parolada sürümü, yanlış parolada Türkçe kimlik doğrulama hatasını gösterir
- [ ] "Parolayı kaydet" ile kaydedilen sunucu bir sonraki açılışta parola sormadan bağlanır
- [ ] Parolası kaydedilmemiş sunucu bağlanırken parola ister; yanlış parola istemi yeniden açar
- [ ] Grup adı verilen sunucular grup altında, diğerleri kökte alfabetik listelenir
- [ ] Bağlı sunucuyu düzenleyip kaydetmek bağlantıyı keser ve alt ağacı kapatır

## Gezinme
- [ ] Server → Databases → Database → Schemas → Schema → Tables → Table → Columns açılır
- [ ] Koleksiyon etiketleri sayı gösterir: "Tables (1.243)"
- [ ] Partition'lar "Tables" listesinde görünmez; partitioned tablo farklı ikonla görünür
- [ ] `datallowconn = false` veritabanı genişletilemez
- [ ] Klavye: ↑ ↓ ← → Enter Home End F5 çalışır
- [ ] 1.000+ tablolu şemada kaydırma akıcıdır

## Inspector
- [ ] Properties: Database, Schema, Table, Column için bölümler doluyor
- [ ] DDL: Database, Schema, Table, Column; tırnaklı adlar (`"Order Items"`) doğru
- [ ] DDL "Kopyala" panoya yazar
- [ ] Sunucu/koleksiyon seçiliyken "DDL yok" mesajı

## Dayanıklılık
- [ ] PostgreSQL servisini yeniden başlat → sonraki işlem hatasız sonuç verir (sessiz yeniden bağlanma)
- [ ] Sunucu tamamen kapalıyken işlem → sunucu düğümünde hata rozeti, uygulamanın geri kalanı çalışır
- [ ] `%APPDATA%\pg-tools\servers.json` bozulduğunda uygulama açılır, durum çubuğunda yedek uyarısı görünür
- [ ] `pg_stat_activity`'de `application_name = pg-tools` ve oturum salt okunur
- [ ] `%LOCALAPPDATA%\pg-tools\logs` altında log var; içinde parola yok

## Görünüm
- [ ] Tema düğmesi sistem/açık/koyu arasında döner; tercih kalıcı
- [ ] Sol panel genişliği sürüklenebilir ve kalıcı

## Plan 3 — Kullanım (elle kontrol listesi)

Her satırda beklenen sonuç `→` ile verilir. Yayın exe'sinin otomatik açılış denetimi
(`scripts/smoke-release.ps1`) için `docs/testing.md`'ye, ölçümler için `docs/performance.md`'ye bakın.

### pgAdmin içe aktarma
- [ ] Araç çubuğu "İçe aktar" → pgAdmin `servers.json` seçilir → önizlemede "N kayıt · N aktarılabilir · N zaten var · N hatalı" özeti ve tablo (Ad, Grup, Host:Port, Kullanıcı, Durum) görünür
- [ ] Aynı dosyayı ikinci kez içe aktar → kayıtlı sunucularla eşleşenler (host + port + kullanıcı + ad) "zaten var" olarak işaretli ve varsayılan olarak seçili değil; aktarımda kopya oluşmaz
- [ ] Host'u ya da kullanıcı adı olmayan / geçersiz portlu kayıt → satır "hatalı" olarak nedeniyle görünür ve seçilemez; diğer kayıtlar aktarılabilir
- [ ] `UseSSHTunnel: 1` olan kayıt → "⚠" uyarı rozeti, uyarılarda "SSH tüneli ayarları yok sayıldı."; kayıt tünelsiz aktarılır
- [ ] "İçe aktar (N)" → "N sunucu içe aktarıldı. Parolalar ilk bağlantıda sorulacak." bildirimi; sunucular gruplarıyla ağaçta görünür, ilk bağlantıda parola istenir
- [ ] İçe aktarma sürerken diyalog Esc / dış tıklama ile kapanmaz

### Filtre (Ctrl+F)
- [ ] Ctrl+F → filtre kutusu odaklanır; metin yalnızca **yüklenmiş** düğümlerde aranır, eşleşmeler ataları ile birlikte görünür
- [ ] Eşleşme yoksa → "Eşleşme yok. Filtre yalnızca yüklenmiş düğümlerde arar."
- [ ] Filtre kutusunda ↓ → odak ağaca geçer; Esc / "Filtreyi temizle" → filtre boşalır, tam ağaç geri gelir
- [ ] 1.000+ tablolu açık şemada filtre yazarken gecikme hissedilmez

### Sağ tık menüsü
- [ ] Sunucu (bağlı değil) → Bağlan, Sunucuyu düzenle…, Sunucuyu sil…, Adı kopyala
- [ ] Sunucu (bağlı) → Yenile, Bağlantıyı kes, Sunucuyu düzenle…, Sunucuyu sil…, Adı kopyala; "Bağlantıyı kes" → alt ağaç kapanır, "Bağlantı kesildi." bildirimi
- [ ] Tablo → Yenile, Adı kopyala, Nitelikli adı kopyala (`şema.tablo`, gerekirse tırnaklı: `app."Order Items"`), DDL'i kopyala (panoda `CREATE TABLE ...`)
- [ ] Kolon → Adı kopyala, Nitelikli adı kopyala (`şema.tablo.kolon`), DDL'i kopyala
- [ ] Fonksiyon → Nitelikli ad imzayla kopyalanır (`şema.ad(argüman tipleri)`); DDL'i kopyala `CREATE OR REPLACE FUNCTION ...` verir
- [ ] Koleksiyon düğümü (ör. "Tables") → yalnızca Yenile ve Adı kopyala
- [ ] Shift+F10 / Menü tuşu → seçili düğümün menüsü açılır; Esc ile kapanır

### Sunucu silme
- [ ] **Bağlıyken** "Sunucuyu sil…" → "Sunucu silinsin mi?" onayı; "Sil" → bağlantı kapanır, düğüm ve kayıtlı parolası silinir, "‘ad’ silindi." bildirimi; PostgreSQL sunucusunda hiçbir şey değişmez
- [ ] Onayda "Vazgeç" → hiçbir şey değişmez, sunucu bağlı kalır

### Hata rozeti, Tekrar dene, NotFound
- [ ] Sunucu kapalıyken genişlet → düğümde hata rozeti; rozetin **üzerine gelince** hata kartı (Türkçe ileti, ayrıntı, "Tekrar dene"); rozete **tıklama** doğrudan yeniden dener ve odak ağaca döner (ok tuşları hemen çalışır)
- [ ] Sunucu yeniden açıldıktan sonra "Tekrar dene" → düğüm yüklenir, rozet kaybolur
- [ ] Başka bir oturumda tabloyu `DROP` et → bu tabloyu yenile/DDL'ini kopyala → "Nesne artık mevcut değil: …" bildirimi görünür, satır hatalı işaretlenir; rozet kartında "Nesne artık mevcut değil." ve "Üst düğümü yenile"; üst düğüm yenilenince tablo listeden çıkar. ("Bu öğe artık yok." yalnızca menü açıkken düğüm ağaçtan kalktıysa görünür.)
- [ ] Rozet düğmesi odaklıyken ok tuşları/Enter ağaç gezintisini tetiklemez

### Sistem nesneleri anahtarı
- [ ] Varsayılan kapalı → `pg_catalog`, `information_schema`, `pg_toast*` / `pg_temp_*` şemaları ve `pg_*` roller görünmez
- [ ] Araç çubuğundan açılır → bağlı sunucuların ağacı yenilenir ve sistem nesneleri görünür; tercih yeniden açılışta korunur

### Durum çubuğu
- [ ] Hiçbir şey seçili değilken → "Hazır"
- [ ] Bağlı sunucunun altındaki bir düğüm seçiliyken → "sunucu · PostgreSQL <sürüm> · <veritabanı> · bağlı" (veritabanı yalnızca veritabanı düğümü ve altında)
- [ ] Bağlı olmayan sunucu seçiliyken → "sunucu · bağlı değil"

## Query Tool (Plan 6)

Ön koşul: atılabilir bir PostgreSQL (ör. `docker run … postgres:18`; kapsayıcıyı işiniz bitince `docker rm -f` ile silin) ve
bir test sunucusu. Gerçek sunucu kaydınıza dokunmamak için `%APPDATA%`/`%LOCALAPPDATA%` dizinlerini geçici bir yere
yönlendirerek başlatın (`scripts/measure.ps1` aynı yöntemi kullanır). Her maddenin sonundaki not, o maddenin hangi
çalıştırmada doğrulandığını gösterir: **[T8]** = Task 8 elle duman testi (`tauri dev`, WebView2 uzak hata ayıklama/CDP ile
betikle sürüldü, yerel Windows diyalogları UIAutomation + SendKeys ile), **[T9]** = Task 9 yayın exe'si CDP çalıştırması,
**[—]** = henüz elle çalıştırılmadı (birim/DB testleriyle kapsanır; sürüm öncesi işaretlenmeli).

### Açılış ve bağlam
- [ ] Ağaçta sunucu/veritabanı/şema/tablo sağ tık → "Query Tool'da aç" (ya da ağaçta Ctrl+Alt+Q) → o veritabanına bağlı yeni sekme açılır; tabloda başlangıç metni `SELECT * FROM şema.tablo LIMIT 100;`; sekme başlığı `<sunucu>/<db> #n`. **[T8]**
- [ ] Araç çubuğunda "Yeni sorgu" → seçili düğümün veritabanında (yoksa sunucuda) boş sekme; ilk açılış (CodeMirror dahil) < 500 ms. **[T9]** (38–47 ms, bkz. `docs/performance.md`)
- [ ] Etiketsiz sunucuda ilk açılışta ortam istemi (Üretim / Test / Geliştirme + Atla): "Üretim" kilit kutusunu işaretli getirir; "Atla" seçimi hatırlanır; X ile kapatma sekmeyi açmaz. **[T8]** (Üretim/Test; "Atla" ve hatırlanması **[—]**)
- [ ] Parolası kaydedilmemiş sunucuda ilk çalıştırma parola ister, sonra çalışır; tabloyu içeren ağaç etiket kaydından sonra daralmaz. **[T8]**
- [ ] Sunucu düzenle → parolayı yeniden yaz → kaydet: o sunucuda açık transaction'lı sekme varsa toplu diyalog (Tümünü kaydet / Tümünü at / Vazgeç) çıkar; kayıttan sonra sekmeler "Oturum kapandı" + "Yeniden bağlan" gösterir. **[—]** (birim testli: `saveClosesSessions(prev, next, typed)`)

### Renkli üretim etiketi ve kilit
- [ ] "Üretim"/"prod" etiketli sunucunun sekmesinde sunucu rengi (renk yoksa kırmızı) 3 px üst çizgi + etiket rozeti + kilit simgesi; varsayılan mod Manuel. **[T8]** (çizgi/rozet/Manuel görüldü)
- [ ] Kilitli sunucuda `INSERT/UPDATE/DDL` → çalışmaz, Mesajlar'da ifade başına neden; `set_config(` ve `U&"…"` de reddedilir; kilit ve etiket değişimi açık transaction'ı olan sekmeler için uyarı verir, sonra sekme "Oturum kapandı" + "Yeniden bağlan" olur. **[T8]** (etiket/kilit değişimi + yeniden bağlanma; `locked` notları **[—]**)
- [ ] Üretim sunucusunda yazan ifade onay diyaloğu "Üretim sunucusu — …" bandı gösterir; `DO`/`CALL` de onay ister. **[T8]** (yazma onayı; `DO`/`CALL` **[—]**)

### Tehlikeli ifade onayı
- [ ] `UPDATE t SET v=1` (WHERE'siz) / `DELETE FROM t` / `DROP` / `TRUNCATE` → onay diyaloğu, odak "Vazgeç"te; Vazgeç hiçbir şey çalıştırmaz, onay sonrası çalışır. **[T8]**

### Transaction
- [ ] Manuel moda geç → `INSERT` sonrası "açık transaction" rozeti + süre sayacı; Commit → satır kalıcı, Rollback → satır yok. **[T8]**
- [ ] Hatalı ifade sonrası "Transaction hatalı — yalnızca Rollback" rozeti; Commit etkin değil. **[T8]**
- [ ] Manuel modda `VACUUM` → hata 25001 + "autocommit moduna geçin" ipucu (Mesajlar). **[T8]**
- [ ] Açık transaction'lı sekmeyi kapat → Kaydet / At / Vazgeç; Vazgeç sekmeyi korur. **[T8]**
- [ ] Açık transaction varken pencereyi kapat → tek diyalog (Tümünü kaydet / Tümünü at / Vazgeç); Vazgeç uygulamayı açık bırakır; "Tümünü at" kapatır ve yeniden açılışta değişiklik geri alınmıştır; hatalı sekmeler için "Tümünü kaydet" yoktur. **[T8]**
- [ ] Sunucu sil / bağlantıyı kes menüsünde açık transaction varsa aynı toplu diyalog. **[T8]** (silme; bağlantıyı kes menüsü **[—]**, birim testli)

### İptal
- [ ] `select pg_sleep(30)` → İptal → ~100 ms içinde "İptal edildi"; PostgreSQL'de aktif sorgu kalmaz. **[T8]**
- [ ] İptal ilk basışta etkisiz kalırsa (ifade ortası, çok kısa pencere) **tekrar basılabilir**; düğme çalışırken etkin kalır. **[—]** (iptal her basışta yeniden gönderilir — arka uç testli)
- [ ] Meşgul sekmeyi kapatma (sorgu sürerken) → bilgi bildirimi, sekme birkaç yüz ms içinde kapanır, PostgreSQL'de yetim sorgu yok. **[T8]**

### CSV dışa aktarma
- [ ] Tek bir SELECT sonucu → "CSV'ye aktar" → Windows kaydet diyaloğu; dosya BOM + başlık + satırlar. **[T8]**
- [ ] Aynı dosya adına ikinci dışa aktarma → Windows "Farklı Kaydetmeyi Onayla — … zaten var" istemi; Evet dosyayı yeniden yazar. **[T8]**
- [ ] Kaydet diyaloğunu iptal et → dışa aktarma başlamaz, hata bildirimi yok. **[—]**
- [ ] Dışa aktarma sürerken Çalıştır/F5 tetikleyicileri devre dışı. **[T8]**
- [ ] Hedef Excel'de açıkken yazma → hata tüm dışa aktarma bittikten sonra görünür (bilinen sınır). **[—]**

### Emoji içeren betikte seçim çalıştırma ve hata işareti
- [ ] `select '😀' as e; select nope_kolon;` betiğinde ikinci ifadeyi seçip Ctrl+Enter → yalnızca seçim çalışır (emojiden sonra kayma yok); hata işareti `nope_kolon` üstünde (dalgalı alt çizgi). **[—]** (UTF-16 ↔ Unicode scalar dönüşümü `offsets.test.ts`/`history-model.test.ts` ile; hata işareti emojisiz betikle **[T8]**)
- [ ] Ctrl+Enter imleçteki ifadeyi, F5 / "Tümünü çalıştır" tüm betiği çalıştırır; sayfa yenilenmez (Ctrl+R / Ctrl+Shift+R da engellenir). **[T8]** (F5/Ctrl+Enter; Ctrl+R **[—]**)

### İlk satır gecikmesi
- [ ] Basit sorguda (LAN, < 1.000 satır, oturum açık) Ctrl+Enter'den ilk satırın görünmesine kadar < 150 ms (spec §1.3). **[—]** (Plan 6 ölçümü 10.000 satır + `qt_open` dahil, ±100 ms çözünürlük: hedefi doğrulamaz; bkz. `docs/performance.md`)

### Büyük sonuç ve kaydırma
- [ ] `generate_series(1,100000)` — ilk 10.000 satır gelir; "Daha fazla yükle (1.000)" ile 50.000 satıra kadar yüklenir; kaydırma akıcı, DOM'daki `gridcell` sayısı < 2.000 (ölçülen 69–99, toplam DOM düğümü 323). **[T8]** (10.000 satır **[T9]**: 92–96 hücre)
- [ ] Izgarada Ctrl+C başka yerde eski DOM seçimi varken hücre değerini kopyalar. **[T8]**

### Taslaklar
- [ ] Sekmelerde metin yazıp (5 sn bekleyerek ya da hemen uygulamayı kapatarak) çık → yeniden açılışta sekmeler metin, mod ve zaman aşımıyla gelir ("Bağlı değil — ilk çalıştırmada bağlanır"); CodeMirror sekme seçilene kadar yüklenmez. **[T8]**
- [ ] Sunucusu silinmiş sekme salt okunur "Sunucu kaydı yok" taslağıdır; kapatınca silinir. **[T8]**

### Geçmiş
- [ ] Geçmiş bölmesi son çalıştırmaları (başarısızlar işaretli) gösterir; girdiye tıklamak metni imlece ekler. **[T8]**
- [ ] Geçmişte arama kutusu girdileri süzer; Geçmiş ayarlarında kaydı kapatma/temizleme çalışır; hassas ifadeler (parola içerenler) kaydedilmez. **[—]**

### session_reset
- [ ] Manuel moda geç, `set search_path = pg_catalog;` çalıştır; sunucuyu yeniden başlat (`docker restart`) → sonraki çalıştırma "oturum sıfırlandı" ipucu verir ve ifade çalışmaz; yeniden çalıştırınca yeni bağlantıda çalışır; açık transaction kaybolduysa ayrı "bağlantı koptu; geri alındı" hatası. **[—]** (PG yeniden başlatma gerektirir; arka uç `session.rs` testli)

## Editör zekâsı (Plan 7)

Ön koşul: Query Tool bölümündeki gibi atılabilir bir PostgreSQL ve yalıtılmış `%APPDATA%`/`%LOCALAPPDATA%`. Sınama
şeması: `create schema app; create table app.orders(id int primary key, customer text, total numeric); create table
app.customers(id int primary key, name text, email text);`. Doğrulama notları: **[T5]** = Task 5 (`tauri dev` +
WebView2 CDP, PG 18.6), **[T7]** = Task 6 / Plan 7 sonu doğrulaması ve son düzeltme dalgası (yayın exe'si `bun run tauri build --no-bundle`, CDP; sayılar
`docs/performance.md`), **[—]** = elle çalıştırılmadı (birim/DB testleriyle kapsanır; sürüm öncesi işaretlenmeli).

### Alias çözümlemeli tamamlama
- [ ] `select o. from app.orders o` ile imleç `o.`'dan sonra → `id`, `customer`, `total` (tür bilgisiyle). **[T5] [T7]**
- [ ] `with x as (select id, total+1 as tt from app.orders) select x. from x` → `id`, `tt` (CTE gövdesi sunucuda Parse edilir). **[T7]**
- [ ] `select s. from (select id, customer || 'a' as cc from app.orders) s` → `id`, `cc` (alt sorgu). **[T7]**
- [ ] CTE'de açık kolon listesi (`with x(a,b) as (…)`), `join` ile birden çok alias, `update t a … from`, `insert … returning`
  ve `from generate_series(1,3) g` alias'ı. **[—]** (birim testli: `scope.rs`, `tests/complete.rs`)
- [ ] `FROM`'dan önce yazılan `select ` sonrası kapsamdaki ilişkilerin kolonları `alias.kolon` önerilir (en çok 500). **[—]**
- [ ] Alias tamamlaması oturum meşgulken (sorgu sürerken) hiçbir şey göstermez, hata vermez. **[—]**
- [ ] Gecikme (`.` tuşundan öneri listesine): sıcak 54–69 ms, soğuk 56–70 ms (`.` sonrası debounce yok, CodeMirror 50 ms);
  kısmi adda (`o.c`) ~210 ms (150 ms debounce korunur); arka uç yanıtı 1–4 ms. **[T7]** (bkz. performans notu)

### Hata ve bilgi işaretleri (yazarken denetim)
- [ ] `select * from no_such_table;` yazıp ~0,6 sn bekleyince `no_such_table` altı dalgalı kırmızı, gutter'da kırmızı nokta,
  üzerine gelince "Hata: … (42P01)" ipucu; ifade çalışmaz (hiçbir şey çalıştırılmaz, yalnızca Parse). **[T5] [T7]**
  (denetim gecikmesi 613–628 ms)
- [ ] Betikte önceki bir `create table t_new …;` varken `select a from t_new;` → `t_new` **gri noktalı** (bilgi), kırmızı değil;
  tablo oluşmaz. **[T5]**
- [ ] Metni düzenleyince ilgili işaretler hemen kalkar, durup bekleyince yeniden gelir; hızlı yazarken eski sonuç uygulanmaz. **[T5]**
- [ ] Emojili betikte (`select '😀'; select nope_kolon;`) işaret `nope_kolon` üstünde. **[—]** (UTF-16 eşleme `check-model.test.ts`'te)
- [ ] Salt okunur kilitli / üretim etiketli sunucuda da çalışır (yalnızca Parse). **[—]**
- [ ] Bağlı olmayan ya da uyuyan oturumda işaret yok, hata yok. **[—]**

### Açık transaction güvenliği
- [ ] Manuel modda `insert …` ile transaction aç; denetim sürerken: transaction `idle in transaction` kalır, `orders` üzerinde
  yalnız `RowExclusiveLock` (Parse'ın kilitleri bırakılmış), aynı transaction'da `count(*)` satırı görür, Rollback sonrası satır yok. **[T5]**
- [ ] `select 1/0` ile transaction `Failed` olunca denetim istek atmaz, işaret yok; Rollback sonrası yeniden denetler. **[T5]**
- [ ] Başka oturum tabloyu ACCESS EXCLUSIVE kilitliyken denetim ~200 ms'de (lock_timeout) sessizce vazgeçer, sunucuda yetim bekleme yok. **[T5]**

### Çalıştırma denetimden önce gelir (BUSY değil)
- [ ] Denetim uçuştayken Ctrl+Enter: denetim kesilir (46 ms), çalıştırma `busy` toast'u vermeden başlar ve biter; denetim sonucu uygulanmaz. **[T5]**
- [ ] Aynı şeyi `more`/Commit/Rollback/seçenek değişikliği/CSV için dene (kullanıcı işlemleri en çok 3 sn bekler). **[—]** (arka uç testli)

### Biçimlendirme (Ctrl+Shift+F)
- [ ] Seçim yokken tüm metin biçimlenir (BÜYÜK anahtar kelime, ana yan tümceler yeni satırda); imleç en yakın belirtecin önünde kalır; **tek Ctrl+Z** özgün metne döner. **[T5] [T7]**
- [ ] Seçim varken yalnızca seçim biçimlenir, seçimin baş/son boşlukları korunur. **[T5]** (boşluk korunması `format-edit.test.ts`)
- [ ] Seçim bir dize/yorum/dolar gövdesi/tırnaklı ad içinde ya da belirteç ortasında başlıyor/bitiyorsa çevreleyen ifadeye
  genişletilir; dize içeriği değişmez (`insert … values ('select  a  from b')` içindeki metni seçip Ctrl+Shift+F). **[T7]**
  (`format-edit.test.ts` `safeFormatSelection`)
- [ ] 10.000 satırlık betikte tuştan belgenin değişmesine ~170 ms; arayüz donmaz (en uzun kare 38 ms). **[T7]**
- [ ] Dize, dolar tırnaklı gövde, yorum, çift tırnaklı ad ve operatörler değişmez. **[—]** (`tests/format.rs`, fuzz)
- [ ] Biçimlendirme başarısız olursa (belirteç eşitliği/eşkuvvetlilik bozulur, 5 MB üstü) "biçimlendirilemedi" toast'u çıkar ve metin değişmez. **[—]** (kod yolu basit; biçimlendiriciyi bozacak girdi bulunamadı)

### Ayarlar ve sekme düğmesi
- [ ] Ayarlar → Editör: "Yazarken hata denetimi" kapatınca işaretler kalkar, istek gitmez; kalıcıdır (`settings.json` `checkEnabled`). **[T5]**
- [ ] Biçim seçenekleri (harf büyüklüğü / girinti 2–4 / virgül konumu / satır uzunluğu) hemen uygulanır ve `settings.json`'a yazılır; geçersiz
  değerli `format` yüklemede yalnızca biçim varsayılanına döner. **[T5]** (kalıcılık); geçersiz dosya yüklemesi **[—]** (birim testli)
- [ ] Sekme bağlam şeridindeki geçici düğme yalnız o sekmeyi kapatır/açar; kapatınca işaretler kalkar, düzenlemede istek gitmez; açınca bir istek gider. **[T5]**
- [ ] Plan 6 dönemi `settings.json` (`{"historyEnabled":false}`) yeni sürümde açılır, varsayılanlar gelir. **[—]** (birim testli)

### Kapanan iş
- [x] Tamamlama gecikmesi (Plan 7 son düzeltme dalgası): `.` sonrası debounce kaldırıldı, CodeMirror `activateOnTypingDelay` 50 ms;
  tuştan öneriye ~270 ms → 54–70 ms (sıcak/soğuk); spec §1.2 sıcak hedefi (< 150 ms) karşılanır (`docs/performance.md`).

## Dashboard (Plan 8–9)

Ön koşul: Query Tool bölümündeki gibi atılabilir bir PostgreSQL ve geçici `%APPDATA%`/`%LOCALAPPDATA%`. Aşağıdaki maddelerin
hepsi **[—]** = elle çalıştırılmadı (ajanlar GUI'yi denemez; birim testleri yalnızca modelleri/denetleyiciyi kapsar). Sürüm öncesi
kullanıcı işaretlemeli. Dashboard bağlantıları sunucuda `application_name = 'pg-tools dash'` ile görünür; doğrulamalarda şu sorgu kullanılır:
`select pid, state, query_start, state_change from pg_stat_activity where application_name = 'pg-tools dash';`

### Açılış, sekme ve görünüm
- [ ] Ağaçta sunucu düğümü sağ tık → "Dashboard'u aç": yeni sekme `Dashboard · <sunucu>` açılır; veritabanı düğümünden açılınca `Dashboard · <sunucu>/<db>`. Aynı sunucu/veritabanı için ikinci kez açmak mevcut sekmeye geçer (çoğaltmaz). **[—]**
- [ ] Sekme rengi sunucu rengini, etiketi (Üretim/Test/Geliştirme) ve kilit simgesini sorgu sekmeleriyle aynı biçimde gösterir; sunucu rengi/etiketi bağlam şeridinde okunur. **[—]**
- [ ] İlk açılışta (tembel parça yüklenirken) sekme gövdesi "Yükleniyor…" gösterir; titreme/boş kare, yerleşim sıçraması yok; ikinci açılışta anında açılır. **[—]**
- [ ] Bölüm sekmeleri (Genel / Oturumlar / Kilitler / Yapılandırma / Sorgu istatistikleri): ok tuşlarıyla gezilir; seçili bölüm vurgulu. `pg_stat_statements` yokken "Sorgu istatistikleri" sekmesi soluk (muted) görünür ama hem açık hem koyu temada okunur (kontrast yeterli) ve tıklanabilir. **[—]**
- [ ] Açık ve koyu temada tüm bölümler okunur; kalın yan çizgi vurgusu yok, rozetler kullanılır. **[—]**
- [ ] Dashboard açıkken temayı değiştir (araç çubuğundaki Tema düğmesi): grafik renkleri (seri çizgileri, ızgara, eksen yazıları) bir sonraki çizimde yeni temaya göre okunur; sayfa yenilemeye gerek kalmaz, yeni renkler eski temanın izini taşımaz. **[—]**

### Genel (grafikler)
- [ ] 5 grafik (Oturumlar, İşlem/sn, Satır girişi/sn, Satır çıkışı/sn, Blok G/Ç/sn) 2 sn'de bir sağa doğru akar; ilk örnek yalnızca taban çizgisidir (oranlı grafiklerde ilk birkaç saniye "Veri bekleniyor"). **[—]**
- [ ] Duraklat → grafikler ve sayılar durur, durum "Duraklatıldı"; sunucuda dashboard bağlantısının `query_start` değeri ilerlemez. Sürdür → hemen bir yenileme, durum "Canlı". **[—]**
- [ ] Aralık seçici 1/2/5/10/30 sn: değişince tek zamanlayıcı kalır (`query_start` farkları yeni aralığa yakın; çift sıklık yok). **[—]**
- [ ] Grafik üzerinde fare: ipucu (tooltip) imleçle birlikte gezer, grafiğin sağ/alt kenarında taşmaz (kenara yaslanıp içeride kalır), serilerin değerleri ve zaman doğru; fare çıkınca kaybolur. **[—]**
- [ ] `pg_stat_reset()` (veya sunucu yeniden bağlanması) sonrası oran grafiklerinde negatif değer çıkmaz; çizgide boşluk olur. Bilgisayarı uyutup uyandırınca (aralığın 5 katından uzun ara) çizgide boşluk olur. **[—]** (mantık `chart-model.test.ts`)
- [ ] Özet kartları: sürüm, çalışma süresi, bağlantı sayısı `n / max_connections`, yüksek kullanımda "Yüksek kullanım" rozeti, en eski açık transaction yaşı ("Yok" yoksa); oturum dağılımı (Etkin / Boşta / Boşta (tx) / Diğer). **[—]**
- [ ] 300 örnekten fazlası belleği büyütmez (uzun süre açık bırakınca sekme belleği sabit kalır). **[—]**

### Arka plan ve istek durdurma
- [ ] Dashboard sekmesi arka plandayken (başka sekme/Query Tool seçili) sunucuda dashboard bağlantısının `query_start` değeri ilerlemez, `state` değişmez; sekmeye dönünce hemen bir yenileme olur ve akış sürer. **[—]**
- [ ] Pencere simge durumuna küçültülünce ya da gizlenince (`document.visibilityState = hidden`) aynı şekilde sıfır istek; geri getirince hemen bir yenileme. **[—]**
- [ ] Dashboard sekmesini kapatınca `application_name = 'pg-tools dash'` bağlantısı kısa sürede sunucudan düşer (satır sayısı 0). **[—]**
- [ ] F5 (ve araç çubuğundaki "Yenile (F5)") dashboard açıkken yalnızca verileri yeniler, sayfayı yeniden yüklemez (sekmeler, seçim, süzgeçler yerinde kalır). **[—]**

### Oturumlar
- [ ] Yerleşim: tablo gövdesi kalan yüksekliği doldurur (`h-full`), sayfa kaydırma çubuğu çıkmaz; tablo kendi içinde kaydırılır; üstteki süzgeç satırı sabit kalır. Ayrıntı paneli (Sorgu) açılınca tablo en çok kendi yüzde sınırına (`max-h`) kadar küçülür, panel taşmaz, ikisi de okunur; pencereyi küçültünce/büyütünce bozulmaz. **[—]**
- [ ] Süzgeçler: arama kutusu (pid, sorgu, kullanıcı, uygulama), "Yalnızca etkin", durum seçimi, "Boşta-tx ≥" eşiği, veritabanı seçici birleşik çalışır; sayaç "x / y oturum"; hiçbiri uymazsa "Süzgece uyan oturum yok." **[—]**
- [ ] Yukarı/Aşağı satır seçer, Enter ayrıntıyı açar; satır seçili kalır ve 5 sn'lik yenilemede seçim kaybolmaz; "Kopyala" sorgu metnini panoya alır ("Sorgu panoya kopyalandı."). **[—]**
- [ ] Başka bir uygulama (ör. `psql`) kendi sorgusunu uzun süre çalıştırırken (`select pg_sleep(60)`) satır görünür, "Sorgu süresi" ilerler; bitince durum değişir. **[—]**
- [ ] Yetkisiz rol (`pg_read_all_stats` üyesi olmayan, süper kullanıcı olmayan) ile bağlanınca başkalarının satırlarında sorgu sütunu soluk italik "yetki yok" gösterir; ayrıntıda "Yetki yok: başka bir rolün sorgusu görüntülenemez"; Genel'de "Bazı oturumlar görünmüyor (yetki)." notu çıkar. **[—]**
- [ ] 5000'den fazla oturumda "İlk 5000 oturum gösteriliyor; liste kesildi." **[—]** (büyük kurulum gerektirir)

### İptal ve sonlandırma
- [ ] Kendi dashboard bağlantısının satırında (`isSelf`) İptal/Sonlandır düğmeleri kapalıdır; ipucu "Bu dashboard'un kendi bağlantısı; durdurulamaz." **[—]**
- [ ] Çalışan başka oturumda `select pg_sleep(300)` iken "İptal" → diyalog o anki canlı satırı gösterir (pid, kullanıcı, uygulama, durum, sorgu), odak **Vazgeç**'te başlar; Enter yıkıcı eylemi onaylamaz; Delete tuşu satırda iptal isteği diyaloğunu açar. Onaylayınca "İptal isteği gönderildi." ve diğer oturum `canceling statement` hatası alır; oturum bağlı kalır. **[—]**
- [ ] "Sonlandır" → "Oturum kapanır; bağlı istemci hata alır." uyarısı; onaylayınca oturum `pg_stat_activity`'den kaybolur, istemci bağlantı hatası alır. **[—]**
- [ ] Açık transaction'lı (`idle in transaction`) oturum için ek uyarı "Açık transaction geri alınacak." **[—]**
- [ ] pg-tools'un kendi sorgu sekmesi (`application = pg-tools query`) satırı için "Bu, pg-tools'un kendi sorgu sekmesi." uyarısı; sonlandırınca o sorgu sekmesi "Oturum kapandı" gösterir. **[—]**
- [ ] Diyalog açıkken hedef oturum kapanırsa onayda "Oturum artık yok." çıkar ve işlem yapılmaz; pid başka kullanıcı/uygulamaya geçtiyse "Bu pid artık başka bir oturuma ait; işlem yapılmadı." **[—]** (zor tetiklenir; `signal-model.test.ts`)
- [ ] Çift tıklama çift sinyal göndermez (onay düğmesi tıklamadan sonra kapanır/kilitlenir; sunucu günlüğünde tek sinyal). **[—]**
- [ ] Yetkisiz rolle başka bir rolün oturumunu iptal etmeyi dene: hata iletisi PostgreSQL'in kendi ayrıntısını (yetki hatası, kod `42501`) gösteren Türkçe "Bu oturumu durdurma yetkiniz yok…" metnidir; ham İngilizce hata değil, ama sunucu ayrıntısı metnin yanında okunur. **[—]**
- [ ] Diyalog kapanınca (Vazgeç / Esc / onay) odak diyaloğu açan satıra/düğmeye döner; klavyeyle gezme kesintisiz sürer. **[—]**

### Kilitler
- [ ] Yerleşim: Engelleme ağacı üstte, Kilitler tablosu altta; ikisi de kendi yüksekliğini yüzde sınırlarıyla paylaşır (biri diğerini dışarı itmez), uzun listede her biri kendi içinde kaydırılır. **[—]**
- [ ] İki oturumla zincir: oturum A `begin; lock table t;` açık bırakır, B `select * from t;` (bekler), C de aynısını. Ağaçta kök "Engelleyen" = A, altında B ve C ("2 bekleyen" gibi), bekleme süreleri ("… bekliyor") ilerler; A `commit` edince ağaç "Engellenen oturum yok" olur. **[—]**
- [ ] Zincir A → B → C (B de bir şeyi tutup C'yi bloke eder) iç içe doğru gösterilir; döngü varsa "döngü" rozeti çıkar ve arayüz donmaz. **[—]** (`lock-tree.test.ts`; gerçek döngü deadlock detector'ı yüzünden zor)
- [ ] Hazır transaction ile engelleme (`max_prepared_transactions > 0` ve yeniden başlatma gerekir): `begin; lock table t; prepare transaction 'x';` sonra başka oturum `select * from t;`. Ağaçta engelleyen "Hazır transaction" (eylemsiz; "Hazırlanmış (prepared) transaction kilit tutuyor; buradan durdurulamaz.") görünür; `rollback prepared 'x'` ile temizlenir. **[—]**
- [ ] Kilit tablosunda bekleyen satırlarda "Verildi: Hayır" ve "Kilit bekleniyor" ipucu; tablo klavyeyle gezilir. **[—]**
- [ ] Ağaç ve tablo satırlarında bekleyen/engelleyen oturum listede yoksa "Oturum listede yok" yazar. **[—]**

### Yapılandırma ve hazır transaction'lar
- [ ] Yapılandırma: arama (ad, kategori, açıklama) çalışır; "Yalnızca değiştirilmiş" varsayılan dışı ayarları (kaynak ≠ default) gösterir; "Yeniden başlatma bekliyor" yalnızca `pending_restart` ayarlarını gösterir (ör. `max_connections`'ı `ALTER SYSTEM` + `pg_reload_conf()` ile değiştirip sonra süzgeci aç; satırda "yeniden başlatma bekliyor" rozeti). **[—]**
- [ ] Ayarlar otomatik yenilenmez; "Yenile" düğmesi yeniden yükler. **[—]**
- [ ] Uzun değerler (`search_path`, `shared_preload_libraries`, uzun dizinler) hücrede kesilir (üç nokta), satır yüksekliği büyümez, tam değer ipucu/ayrıntıda okunur; sütun taşması yatay kaydırma çubuğu yaratmaz. **[—]**
- [ ] Hazır transaction'lar tablosu (Yapılandırma altında): `max_prepared_transactions` 0 ise "Hazır transaction yok"; bir tane `prepare transaction` ile oluşturulunca gid, transaction, sahip, veritabanı, yaş görünür; 1 saatten eskiyse "uzun süredir açık" rozeti; "yalnızca bilgi" notu, sonlandırma düğmesi yok. **[—]**

### Sorgu istatistikleri (`pg_stat_statements`)
- [ ] `shared_preload_libraries = 'pg_stat_statements'` + `create extension pg_stat_statements` yapılmış veritabanında liste gelir: sıralama (Toplam süre / Ortalama / Çağrı) değişince sıra değişir; ayrıntıda tam sorgu, "Kopyala" çalışır; isabet oranı (Hit/Read) mantıklı. **[—]**
- [ ] Eklenti yokken (ya da yalnızca yüklenmemişken): bölümde "pg_stat_statements kullanılamıyor" durumu, hata iletisi/çökme yok; diğer bölümler çalışmaya devam eder; `pg_stat_statements_reset()` sonrası "Henüz kayıtlı sorgu istatistiği yok." **[—]**
- [ ] `shared_preload_libraries`'de olup `create extension` yapılmamış durumda da aynı "kullanılamıyor" durumu (eklenti yok ile ayırt edilmeyebilir, kabul). **[—]**

### Bağlantı yaşam döngüsü
- [ ] Dashboard açıkken ağaçta sunucuyu "Bağlantıyı kes": dashboard "Bağlantı kesildi" durumuna geçer ("Dashboard oturumu kapandı; yenileme durdu."), istekler durur (sunucuda `pg-tools dash` bağlantısı yok), **Yeniden aç** yeniden bağlanır ve "Canlı" olur; grafik geçmişi temizlenmiş başlar. **[—]**
- [ ] Sunucu tarafında bağlantıyı düşür (`select pg_terminate_backend(pid)` ile `pg-tools dash` bağlantısını): "Bağlantı kesildi" + **Yeniden aç** çıkar, ham `not_found` hatası görünmez. **[—]**
- [ ] Dashboard açıkken sunucuyu sil: "Sunucu silindi" ("Bu sunucunun kaydı silindi; dashboard yeniden açılamaz."), **Yeniden aç** yok/etkisiz, sekme kapatılabilir; sunucuda dashboard bağlantısı kalmaz. **[—]**
- [ ] Dashboard açıkken sunucuyu düzenleyip salt okunur kilidini ya da üretim etiketini değiştir (kaydet): dashboard "Bağlantı kesildi" durumuna geçer, **Yeniden aç** çalışır; her yeniden açmadan sonra `select count(*) from pg_stat_activity where application_name = 'pg-tools dash'` dashboard sekmesi sayısı kadar (sekme başına 1) kalır; fazladan/yetim bağlantı birikmez. **[—]**
- [ ] Aynı sunucuda iki dashboard sekmesi (sunucu + veritabanı) açıkken biri kapanınca diğeri etkilenmez. **[—]**
- [ ] İki dashboard sekmesini art arda aç (ör. iki farklı sunucu ya da sunucu + veritabanı) → ikinci sekme de yüklenir ve "Canlı" olur ("Yükleniyor…"da kalmaz). **[—]**
- [ ] Uygulama açıkken F5/Ctrl+R sayfayı yeniden yüklemez (Plan 6 ile aynı davranış); kapatma penceresi dashboard bağlantılarını temizler (uygulamayı kapattıktan sonra sunucuda `pg-tools dash` bağlantısı kalmaz). **[—]**

## Nesne diyalogları (Plan 11)

Ön koşul: bağlı bir sunucu ve veritabanı; designer sekmesi Explorer menüsünden açılır (Task 4). Aşağıdaki kutucuklar GUI üzerinden elle uygulanmadı; sürüm öncesi işaretlenmelidir.

### Sekme kabuğu
- [ ] Designer sekmesi sorgu ve dashboard sekmelerinden sonra çizilir (kalem-cetvel simgesi); sunucu renk çizgisi, üretim rozeti ve kilit simgesi sorgu sekmesindekiyle aynıdır. **[—]**
- [ ] Formda değişiklik yapılınca sekmede nokta görünür (ekran okuyucu "kaydedilmemiş değişiklikler" okur); bağlantı kesilince/sunucu silinince başlık soluklaşır ("gone"da üstü çizili). **[—]**
- [ ] Kirli designer sekmesini kapat (×, orta tık, Delete): "Kaydedilmemiş değişiklikler atılsın mı?" diyaloğu, odak **Vazgeç**'te, Enter sekmeyi kapatmaz; "Değişiklikleri at ve kapat" kapatır. Temiz sekme onaysız kapanır. **[—]**
- [ ] Designer etkinken F5 sayfayı yeniden yüklemez, sorgu çalıştırmaz, önizlemeyi hemen yeniler; Ctrl+R/Ctrl+Shift+R sayfayı yeniden yüklemez. Ctrl+F davranışı değişmez. **[—]**
- [ ] Designer etkinken sorgu paneli ve "Nesne" paneli görünmez/tıklanamaz; designer'dan sorgu sekmesine geçince editör/ızgara kaydırma konumu korunur, designer'a dönünce form ve önizleme olduğu gibi durur. **[—]**
- [ ] **Bilinçli karar:** pencere kapatılırken kirli designer sekmeleri için uyarı **gösterilmez** (açık transaction toplu onayına eklenmez); kaydedilmemiş taslaklar kaybolur, uçuştaki uygulama arka uçta iptal edilir. Kapatma sonrası sunucuda `pg-tools` DDL oturumu kalmadığını doğrula. **[—]**

### Önizleme ve uygulama çubuğu
- [ ] Önizleme salt okunur; metin yalnızca arka ucun ifadeleri (`;` ile, aralarında boş satır). İfade listesinde sıra, risk rozeti (Güvenli / Yıkıcı / Veri kaybı olabilir), "transaction dışı" rozeti ve not görünür; uyarılar şeridi okunur. **[—]**
- [ ] Boş durumlar: hatalı form → "Formu tamamlayın"; değişiklik yok → "Değişiklik yok"; ilk önizleme beklerken → "Önizleme hazırlanıyor…"; yazarken eski önizleme soluk kalır ("Yenileniyor…"), titreme yok. **[—]**
- [ ] **Kopyala** betiği panoya koyar ("SQL panoya kopyalandı."); pano hatasında hata bildirimi. **[—]**
- [ ] Yıkıcı planda "Etkisini anladım" işaretlenmeden **Uygula** kapalı; Uygula kırmızıdır. Üretim etiketli sunucuda veri kaybı planında ayrıca "Onaylamak için sunucu adını yazın: <ad>" alanı (büyük/küçük harf duyarlı); ad alanında Enter gönderim yapmaz. **[—]**
- [ ] Uygulama sürerken Uygula "Uygulanıyor…" ve kapalı, **İptal** görünür ve uçuştaki uygulamayı iptal eder; çift tık ikinci istek üretmez. **[—]**
- [ ] Kilitli sunucuda Uygula ve lock_timeout kapalı (`title`: "Sunucu salt okunur kilitli"), önizleme ve Kopyala serbest. **[—]**
- [ ] Başarısız uygulamada ilgili ifade satırları önizlemede kırmızı zeminle vurgulanır, listede "başarısız" rozeti; `committed > 0` ise "N ifade kalıcı oldu" uyarısı ve "Yeniden yükle" şeridi; `55P03` → "Kilit beklerken zaman aşımı". **[—]**
- [ ] lock_timeout seçici (1 sn / 5 sn / 30 sn / sınırsız) uygulamada kullanılır (başka oturumda `lock table … in access exclusive mode` tutarken 1 sn ile zaman aşımı). **[—]**
- [ ] **Yeniden yükle**: kirli formda "Kaydedilmemiş değişiklikler atılsın mı?" onayı (odak Vazgeç); temizken doğrudan yükler. İlk yükleme hatasında "Yeniden dene". **[—]**

### Gezgin menüsü ve açma (Task 4)
- [ ] Schemas / Sequences / Extensions / Indexes koleksiyonunda sağ tık → **Oluştur…**; schema / sequence / extension / index düğümünde **Düzenle…**, **Yeniden adlandır…**, **Sil…** (Query Tool eyleminden sonra, ayırıcıyla ayrı grup). Tablo, kolon, görünüm, fonksiyon, rol, veritabanı, sunucu düğümlerinde bu eylemler yok. **[—]**
- [ ] Salt okunur kilitli sunucuda bu dört eylem görünür ama soluk ve seçilemez; üzerine gelince `title` "Bu sunucu salt okunur kilitli."; kilit kaldırılıp ağaç yenilenince etkinleşir. **[—]**
- [ ] İlk "Oluştur/Düzenle" designer parçasını yükler (ağ sekmesinde `designer-open-*.js`); iki hızlı tıklama tek sekme / tek kurulum üretir. Bağlı olmayan sunucuda önce parola istenir; iptalde sekme açılmaz. **[—]**
- [ ] İki farklı nesnenin **Düzenle…** sekmelerini art arda aç → ikinci sekme de yüklenir ("Yükleniyor…"da kalmaz); formda değişiklik yapınca başlığında kirli noktası görünür, silinince/bağlantı kesilince başlık stili güncellenir. **[—]**
- [ ] Aynı nesnede ikinci **Düzenle…** mevcut sekmeyi etkinleştirir; **Yeniden adlandır…** (yeni ya da mevcut sekmede) ad alanına odaklanır. Sekme üst çubuğunda "<sunucu>/<veritabanı>". **[—]**
- [ ] **Sil…** sekme açmaz; silme diyaloğunu açar (bkz. "Silme diyaloğu"). Hiç designer sekmesi açık değilken de diyalog görünür (ilk kullanımda `DropDialog-*.js` iner). **[—]**
- [ ] Uygulama sonrası Explorer'da doğru koleksiyon yenilenir (yeni sequence görünür). Designer'dan "Query Tool'da aç" betiği yeni sorgu sekmesine **çalıştırmadan** koyar (başlık "<sunucu>/<veritabanı> #n"; etiketsiz sunucuda ortam sorusu; kilitli sunucuda da açılır, çalıştırma kilit kapısına takılır). **[—]**
- [ ] Sunucu bağlantısı kesilince/silinince açık designer sekmeleri "Bağlantı kesildi"/"Sunucu kaydı silindi" olur; sunucu adı/etiketi/kilidi düzenlenince açık designer sekmesi yeni adı/kilidi gösterir. **[—]**

### Formlar: Schema
- [ ] Schemas → **Oluştur…**: ad, sahip (OWNER; boş → bağlanan rol), yorum → önizleme `CREATE SCHEMA …` (+ `COMMENT ON SCHEMA`) → Uygula → Schemas yenilenir, sekme yeni şemanın Düzenle sekmesine dönüşür. **[—]**
- [ ] Var olan şemada yeniden adlandır / yorum değiştir / sahip değiştir → her biri ayrı `ALTER SCHEMA …`/`COMMENT ON` ifadesi; yorumu boşaltmak → `COMMENT … IS NULL`. **[—]**

### Formlar: Sequence
- [ ] Oluştur: veri türü, INCREMENT, MINVALUE/MAXVALUE (boş = varsayılan), START, CACHE, CYCLE → önizleme doğru; büyük sayılar (`9223372036854775807`) bozulmadan gider; harf/boşluk girilince "sayıları düzeltin" yerel engeli → istek gitmez. **[—]**
- [ ] Yön değişimi (INCREMENT pozitiften negatife) → START/MIN/MAX birlikte güncellenir, `start` hatası çıkmaz; uyarı şeridi okunur. **[—]**
- [ ] OWNED BY: tablo → kolon seçimi (tablo dizinin şemasında olmalı; dizinin şeması değişince OWNED BY şeması da izler); kaldırma → `OWNED BY NONE`. Tür değişimi (`bigint` → `integer`) sınır dışı değerde arka uç hatası alanın yanında görünür. **[—]**

### Formlar: Extension
- [ ] Oluştur: kullanılabilir eklenti listesi, sürüm (boş = varsayılan), şema → `CREATE EXTENSION …`; sürüm listesi yüklenirken "yükleniyor" gösterir, kurulamayan adda arka uç hatası alanın yanında. **[—]**
- [ ] Düzenle: sürüm güncelle (`ALTER EXTENSION … UPDATE TO`), şemayı taşı (`SET SCHEMA`); taşınamayan eklentide arka uç hatası okunur. Sil → silme diyaloğu. **[—]**

### Formlar: Index
- [ ] Oluştur: yöntem btree / hash / gin (yöntem değişince tanım korunur), kolon ve ifade anahtarları ("ifade SQL olarak çalıştırılır" notu), INCLUDE (desteklenmeyen sürümde öğretici hata), WHERE (kısmi indeks), UNIQUE, fillfactor, tablespace (`pg_default` seçilir; temizlenemez), CONCURRENTLY → önizlemede "transaction dışı" rozeti. **[—]**
- [ ] Kolon satırı klavyeyle taşınır (yukarı/aşağı düğmeleri Tab ile erişilir; odak taşınan satırda kalır); satır silme/ekleme sonrası `columns[i].key` hataları doğru satırda görünür. **[—]**
- [ ] Düzenle: yeniden adlandır, fillfactor, tablespace → `ALTER INDEX`; yapısal değişiklik (kolon/yöntem/WHERE) → "drop + create" uyarısı ve tablo kilidi notu, risk ≥ Yıkıcı. **[—]**
- [ ] Yapısal değişiklik uygulandıktan sonra (DROP+CREATE → yeni oid) sekme yeni indeks üzerinde `ready` kalır ("Nesne yok" olmaz); form yeni tanımı gösterir. **[—]**
- [ ] Kısıt-bağlı indeks (PRIMARY KEY/UNIQUE kısıtı) → banner + form salt okunur; geçersiz (`indisvalid = false`) indeks → banner. **[—]**
- [ ] Uzun CREATE INDEX (büyük tablo, `statement_timeout` olmadan) sırasında **İptal** → uygulama iptal edilir, sekme kullanılabilir kalır; CONCURRENTLY yarım kalırsa geçersiz indeks banner'ı. **[—]**

### Canlı önizleme ve yaşam döngüsü
- [ ] Yazarken önizleme ~150 ms gecikmeyle yenilenir (her tuşta istek yağmuru yok); eksik formda "Formu tamamlayın", değişmemiş formda "Değişiklik yok", yüklenirken "Yükleniyor". **[—]**
- [ ] `plan_changed`: sekme açıkken nesneyi başka oturumda değiştir (`ALTER SCHEMA … RENAME`), sonra Uygula → "Nesne değişti; yeniden yükleyin." şeridi + Yeniden yükle; kirli taslakta yeniden yükleme önce onay ister. **[—]**
- [ ] Hata ortası kısmi uygulama (çok ifadeli planda ikinci ifade başarısız) → "N ifade kalıcı oldu" uyarısı, yeniden yükleme şeridi; aynı plan yeniden gönderilmez. **[—]**
- [ ] Sunucu bağlantısını kes → açık designer sekmeleri "Bağlantı kesildi" salt okunur, istek gitmez; sunucuyu bağlayıp Yeniden yükle → sekme yeniden hazır. **[—]**
- [ ] Açık ve koyu temada formlar, rozetler, hata şeritleri, silme diyaloğu okunur (renkler yalnızca tema değişkenlerinden; kalın yan vurgu kenarlığı yok). **[—]**
- [ ] Klavye ile tam kullanım: Tab sırası form → önizleme → uygulama çubuğu; tüm denetimler odaklanabilir, odak halkası görünür. **[—]**
- [ ] Ekran okuyucu: her alan etiketli; alan hataları `aria-describedby` ile okunur; uygulama hatası `role="alert"` ile duyurulur; durum rozeti okunur. **[—]**
- [ ] Uzun ifade/yorum alanları (birkaç yüz karakterlik WHERE, çok satırlı yorum) yerleşimi bozmaz; önizleme satırları kaydırılır, yatay sayfa kaydırması yok. **[—]**

### Silme diyaloğu
- [ ] Nesne düğümünde **Sil…** → diyalog "Sil: <tür> <ad>": önce "Tanım yükleniyor…", sonra çalıştırılacak `DROP …` ifadesi risk rozetiyle (`<pre>`, salt okunur), varsa uyarılar. Sekme açılmaz. **[—]**
- [ ] Odak **Vazgeç**'te başlar; Enter diyaloğu kapatır, silmez. **Sil** kırmızıdır (`--danger-fg` metin) ve "Etkisini anladım" işaretlenmeden kapalıdır (`title` gerekçeyi söyler). **[—]**
- [ ] "Bağımlı nesnelerle birlikte sil (CASCADE)" işaretlenince önizleme hemen yenilenir (`… CASCADE`, risk "Veri kaybı olabilir"), onay kutusu sıfırlanır; hızlı aç/kapa son seçime göre önizleme gösterir. **[—]**
- [ ] Üretim etiketli sunucuda CASCADE işaretliyken kutuya ek olarak sunucu adının tam yazılması gerekir (büyük/küçük harf duyarlı); ad alanında Enter silmez. **[—]**
- [ ] Bağımlı nesnesi olan şemayı (içinde tablo) CASCADE olmadan sil → diyalog açık kalır, şerit "Bağımlı nesneler var; CASCADE ile silin ya da bağımlıları kaldırın." ve altında PG'nin bağımlı nesne listesi (`detail`); CASCADE işaretleyip onaylayınca silinir. **[—]**
- [ ] Bağımlısı olmayan nesneyi sil → "<ad> silindi." bildirimi, diyalog kapanır, Gezgin'de üst koleksiyon yenilenir ve düğüm kaybolur; aynı nesnenin açık Düzenle sekmesi "Nesne silindi" (gone) olur. **[—]**
- [ ] Diyalog açıkken nesneyi başka oturumda sil/değiştir, sonra Sil → "Nesne değişti ya da artık yok; Gezgini yenileyin."; Sil kapalı kalır, yalnızca Kapat. **[—]**
- [ ] Silme sürerken (başka oturum tabloyu kilitliyken) Sil "Siliniyor…" ve kapalı, Vazgeç yerine **İptal**; Esc diyaloğu kapatmaz; lock_timeout (5 sn) dolunca "Kilit beklerken zaman aşımı" şeridi. **[—]**
- [ ] Kilitli sunucuda Sil… menüde soluk; (kilit diyalog açıkken konursa) Sil kapalı ve "Sunucu salt okunur kilitli." notu. Diyalog açıkken sunucu bağlantısı kesilirse diyalog kapanır. **[—]**

## Ölçümler

| Tarih | Makine | Paket (MB) | Açılış (ms) | Toplam özel WS (MB) | Rust özel WS (MB) |
|---|---|---|---|---|---|
| 2026-10-05 | AMD Ryzen 9 5950X (16 çekirdek), 32 GB RAM, Windows 11 | 3,2 (NSIS kurulum paketi) | 131 / 63 / 31 (3 çalıştırma) | ≈191 (Plan 1 "özel bellek"; çalışma kümesi 390 / 377 / 376) | 40 / 30 / 30 (çalışma kümesi) |
| 2026-10-05 (Plan 2) | aynı makine | — (paket üretilmedi; exe 12,4) | 40 (5 çalıştırma medyanı) | 76,1 (toplam WS 356,9) | 7,5 |
| 2026-10-06 (Plan 6) | aynı makine | 3,56 (NSIS; ana parça `index` 228 kB, Query Tool tembel) | 45 (5 çalıştırma medyanı; Query Tool sekmesi ilk açılış 38–47 ms) | 76,1 boşta; 10.000 satırlı sorgu sekmesiyle en çok 109,1 | 7,4 |
| 2026-10-06 (Plan 7) | aynı makine | — (`--no-bundle`; exe 14,19) | 43 (5 çalıştırma medyanı) | 77,5 boşta; sorgu sekmesi + 10.000 satırlı betik + denetim 113,2 | 7,4 |

Plan 13 Task 1: index-*.js = 233088 B (taban 233733 B; tavan 233088 B)

Plan 13 sonu: index-*.js = 233.088 B; TableForm-*.js = 81.467 B (index.html yalnızca solid/web/Icon parçalarını önyükler; Table metinleri ana pakette yok).
Notlar: Birimler — Plan 2, Plan 6 ve Plan 7 satırlarının bellek ve exe/paket değerleri `measure.ps1`/bayt sayısından geldiği için başlıktaki "MB" orada MiB'dir (Plan 6: 3,56 / 76,1 / 109,1 / 7,4 MiB; Plan 7: exe 14,19 / 77,5 / 113,2 / 7,4 MiB) (1 MiB = 1.048.576 bayt); Plan 1 satırının birimi belirsiz kaynaktandır (bkz. `docs/performance.md` "Birimler"). Plan 2 satırı `scripts/measure.ps1 -Runs 5` ile ölçüldü (8 sn boşta, sunucu yok, seçilen WebView2/tokio ayarlarıyla); yöntem ve denenen ayarlar `docs/performance.md`'de. Plan 1 satırı: ölçüm `target/release/pg-tools.exe` doğrudan çalıştırılarak yapıldı (kurulum yapılmadı); boşta 5 sn, sunucu bağlı değil. Toplam bellek, WebView2 alt süreçleri dahil 7 sürecin çalışma kümesi toplamıdır (paylaşılan sayfalar tekrar sayıldığından özel bellek ≈191 MB). Açılış süresi `MainWindowHandle` oluşana kadar geçen süredir; içerik çizimi dahil olmayabilir. Yukarıdaki kutucuklar GUI üzerinden elle uygulanmadı; sürüm öncesi işaretlenmelidir.

## Nesne diyalogları: Table arka ucu (Plan 12)

Plan 12 yalnızca arka uçtur; GUI maddeleri aşağıdaki Plan 13 bölümündedir. Docker doğrulaması: `scripts/test-matrix.ps1 -SkipPerf` (varsayılan: PG 10–18 tümü; Table gidiş-dönüş + yönetici testleri dahil).

Not: serbest SQL alanlarında (CHECK, DEFAULT, indeks koşulu vb.) sıradan `'…'` dizesi içinde `\` reddedilir; kaçış gerekiyorsa `E'…'` yazın (regex CHECK'leri ve indeks koşullarını etkiler).

## Nesne diyalogları: Table arayüzü (Plan 13)

Ön koşul: Plan 11 smoke ortamı (Docker PG, birkaç tablo; biri bölümlenmiş + bölüm, biri INHERITS çocuğu, biri kendine-başvuran FK'li; PG 18'de ayrıca EXCLUDE kısıtlı tablo). Table arayüzü tembel parçadadır; ilk açılışta kısa yükleme normaldir.

### Menü ve açma
- [ ] Tables koleksiyonunda **Oluştur…**; tablo ve bölüm düğümünde **Düzenle… / Yeniden adlandır… / Sil…**; tablo kolonu ve kısıtı düğümünde **yalnızca Düzenle…**; görünüm/MV kolonunda ve Columns/Constraints/Partitions koleksiyonlarında designer eylemi yok; kilitli sunucuda eylemler soluk + gerekçe. **[—]**
- [ ] Kolon düğümünden Düzenle → tablo sekmesi **Kolonlar** bölümünde, satır "seçili" rozetli, görünür ve ad alanı odaklı; kısıt düğümünden → **Kısıtlar** bölümünde ilgili kart aynı biçimde. Aynı tablo zaten açıksa yeni sekme açılmaz, satıra geçer. **[—]**
- [ ] EXCLUDE kısıt düğümünden (veya silinmiş kısıttan) Düzenle → Kısıtlar sekmesi + "bulunamadı / düzenlenebilir kısıtlar arasında yok" şeridi + Düzenlenmeyenler listesi (ekran okuyucu listeyi `aria-describedby` ile okur). **[—]**
- [ ] Bölüm/INHERITS tablosunda kolon veya kısıt düğümünden Düzenle → her denetim kapalı, kilit notu görünür, satırın kendisi odak alır. **[—]**
- [ ] Kolon/kısıt düğümünde Yeniden adlandır/Sil menüde yok; (eski menü durumunda) "Kolon ve kısıtlar tablo diyaloğunun içinden…" bildirimi. **[—]**
- [ ] Table formunda önizleme paneli diğer türlerden dar (%38); Genel/Kolonlar/Kısıtlar/(create'te) Bölümleme sekmeleri; Tab ile sekme listesine girip ←/→/Home/End gezinti; hatalı bölümde kırmızı nokta + ekran okuyucuda "(hata var)". **[—]**

### Oluştur
- [ ] Kolon yokken "en az bir kolon" notu + Kolon ekle; yeni kolon eklenince odak yeni ad alanına gider. **[—]**
- [ ] Tip seçici: yazınca (~200 ms) öneriler ve ayrıntı etiketi (`int` → integer/int4…), `numeric(10,2)` serbest yazım; öneri araması hata verse de serbest yazılır; `serial` yazınca uyarı + "integer + IDENTITY kullan" düğmesi (tıklayınca integer + BY DEFAULT, NOT NULL işaretli ve kilitli). **[—]**
- [ ] Değer kaynakları: DEFAULT `now()`, IDENTITY ALWAYS/BY DEFAULT (NOT NULL kilitli), GENERATED STORED; VIRTUAL PG 18 öncesi sunucuda öğretici arka uç hatası; DEFAULT ↔ GENERATED arasında geçince ifade metni korunur. **[—]**
- [ ] Collation listesi ilk odakta yüklenir; yorum eklenince önizlemede `CREATE TABLE …` + `COMMENT ON …`. **[—]**
- [ ] Kısıtlar: PK / UNIQUE (+INCLUDE, NULLS NOT DISTINCT) / CHECK (NO INHERIT) / FK (başka tabloya ve kendine), DEFERRABLE / INITIALLY DEFERRED (DEFERRABLE kapalıyken INITIALLY DEFERRED kapalı ve işareti kalkar); NOT VALID create'te sunulmaz; bölümlenmiş create'te CHECK NO INHERIT kapalı + ipucu. **[—]**
- [ ] Kısıt ekleme çubuğu: PK varken PRIMARY KEY seçeneği tooltip'li kapalı, PK silinince yine seçilebilir; yeni kartın tür seçimi değişebilir; ekleme sonrası yeni kartın adı odaklanır; PK varken çubukta UNIQUE görünür ve PK silinince de UNIQUE kalır. **[—]**
- [ ] PK/UNIQUE kolon çipleri ekle/çıkar önizlemeyi günceller (çıkar düğmeleri etiketli); INCLUDE ile anahtar kolonlar birbirini dışlar; PK üyesi kolon Kolonlar'da NOT NULL işaretli + kilitli + ipucu. **[—]**
- [ ] FK hedefi: şema listesi odakta yüklenir, tablo listesi ~200 ms sonra o şemanın tablolarını önerir; hedef değişince hedef kolonlar temizlenir, hedef kolon önerileri hedef tablodan gelir; kendine işaret edince taslağın (kaydedilmemiş dahil) kolonları + "kendine başvuruyor" ipucu; sonradan tablo yeniden adlandırılınca yalnızca o FK'nin hedefi taşınır; çift ekle/çıkar, yerel kolon seçimi; MATCH / ON UPDATE / ON DELETE önizlemede. **[—]**
- [ ] Create'te FK hedefi `emp` yazıp ardından tabloya `emp` adı verilince hedef-kolon alanı "kaydedilmedi … yeniden yazın" ipucu ve sunucu önerileri gösterir; hedefi yeniden yazınca "kendine başvuruyor" olur. **[—]**
- [ ] UNLOGGED; fillfactor; tablespace; Bölümleme: RANGE/LIST/HASH, anahtar ekle/çıkar (odak taşınır), kolon seçimi taslak kolon adlarını listeler, ifade anahtarında serbest SQL + `E'…'` ipucu → önizleme `PARTITION BY …`. **[—]**
- [ ] Uygula → Tables yenilenir, sekme "Düzenle: <ad>" başlığına geçer; aynı tablo için Gezgin'den Düzenle ikinci sekme açmaz. **[—]**

### Düzenle: kolonlar
- [ ] Kolon ekle: nullable → Güvenli; NOT NULL varsayılansız → Yıkıcı notu; boş durum metni görünür. **[—]**
- [ ] Kolon yeniden adlandır → PK/UNIQUE/FK/bölüm anahtarı kolon listeleri birlikte güncellenir; önizlemede `RENAME COLUMN`, kısıt DROP+ADD yok; yazarken başka kolonun adına geçici denk gelmek (ve Tab ile çıkıp dönüp yazmaya devam etmek) o kolonun kısıt başvurularını taşımaz. **[—]**
- [ ] Tip değişimi → Ayrıntılar açılır, USING alanı + "tablo yeniden yazılır" notu, risk "Veri kaybı olabilir"; tip geri alınınca USING gizlenir ve temizlenir. Dokunulmamış yüklü egzotik tip (`geometry(Point,4326)`, tırnaklı tip) uyarı vermez; düzenlenince dilbilgisi uyarısı çıkar. **[—]**
- [ ] Kolon sil → "Silinecek kolonlar" + Geri ekle (özgün konumuna döner, önizleme "Değişiklik yok"; yeniden adlandırılıp silinip geri alınan kolonda da); üretimde ad yazma onayı. Silinen kolona başvuran kısıtta uyarı rengiyle sarkan başvuru metni (kolon geri gelince kalkar) + önizleme uyarısı. **[—]**
- [ ] Değer kaynağı `<select>` satır kayması: üretilmiş kolonu ve ardındaki sıradan kolonu olan tabloda üretilmiş kolonu sil → alttaki kolon yalnızca yok/default/identity seçeneklerini listeler ve doğru değeri gösterir; tersi (üstteki sıradan kolonu silmek) de. **[—]**
- [ ] Yüklü kolonda GENERATED seçeneği sunulmaz (ipucu); yüklü üretilmiş kolonda yalnızca "Yok" (DROP EXPRESSION notu); NOT NULL aç/kapa; DEFAULT/IDENTITY değişimleri. **[—]**
- [ ] Ayrıntılarda collation/yorum/USING hatası sunucudan gelince o satırın ayrıntıları kendiliğinden açılır. **[—]**

### Düzenle: kısıtlar
- [ ] Yüklü kartta tür sabit + ipucu. Kısıt ekle (Yıkıcı değil, "tabloyu tarar" notu); yeni FK/CHECK için NOT VALID + ipucu (işaretlenince tarama notu gizlenir). **[—]**
- [ ] Doğrulanmamış yüklü FK/CHECK: "doğrulanmamış" rozeti; Doğrula işaretlenince önizleme `VALIDATE CONSTRAINT`; yapısı da değiştiyse ipucu "yeniden eklenecek (işaretliyse doğrulanmış)". **[—]**
- [ ] Kısıt yeniden adlandır → `RENAME CONSTRAINT`; yapısal değişiklik → DROP+ADD. **[—]**
- [ ] Yüklü kısıt sil → "Silinecek kısıtlar", geri ekle → önizleme "Değişiklik yok"; yeni PK varken yüklü PK'yi geri ekle düğmesi tooltip'li kapalı; yüklü PK kolonu yeniden adlandırılıp PK silinip geri alınınca "Değişiklik yok" ve sarkan uyarı yok. **[—]**
- [ ] Sunucu hatası `constraints[i].columns[j]` ilgili çip/çiftin altında; satır silinince eski hatalar liste düzeyinde görünür. **[—]**
- [ ] PG 18'de özel NOT NULL kısıtı ve EXCLUDE "Düzenlenmeyen kısıt ve özellikler" listesinde açıklamalarıyla; başka değişiklik uygulanınca korunur. **[—]**

### Düzenle: genel
- [ ] Yeniden adlandır (başlık güncellenir, Tables yenilenir); kendine-başvuran FK'li tabloda yeniden adlandırma/şema taşıma FK hedefini önizlemede günceller (aradaki yazım adımları başka FK'nin hedefini değiştirmez). **[—]**
- [ ] Şema taşı: eski ve — yüklüyse — yeni şemanın Tables koleksiyonu yenilenir; sahip; UNLOGGED ↔ LOGGED (yeniden yazma uyarısı); fillfactor set/boş; yorum; alter'da PARTITION BY salt okunur metin. **[—]**
- [ ] Tablespace: yüklü değer yokken boş seçenek var, varken yok; `pg_default` seçilebilir; temizlenemez. **[—]**
- [ ] Uygulama sonrası tablonun açık Columns/Constraints/Indexes koleksiyonları yenilenir (yeni kolon Gezgin'de görünür). **[—]**

### Bölüm tablosu ve INHERITS
- [ ] Bölüm tablosu ve INHERITS çocuğu: rozet + "kolon ve kısıtlar üst tablodan yönetilir" şeridi; Kolonlar/Kısıtlar salt okunur (yalnızca kolon yorumları düzenlenebilir); ad/şema/sahip/kalıcılık/fillfactor/tablespace/yorum uygulanır; Sil… bölüm tablosunu siler (DROP TABLE, Veri kaybı onayı). **[—]**

### Hata ve kilit durumları
- [ ] `plan_changed`: sekme açıkken başka oturumda kolon ekle → Uygula → "Nesne değişti; yeniden yükleyin."; `lock table … access exclusive` + 1 sn lock_timeout → "Kilit beklerken zaman aşımı". **[—]**
- [ ] Kilitli sunucu: form ve Uygula kapalı, önizleme serbest; kısıtlarda ekle/sil/geri ekle gizli, Düzenlenmeyenler listesi görünür; bağlantı kesilince salt okunur. **[—]**

### Erişilebilirlik ve yerleşim
- [ ] Kolonlar ızgarasında Tab sırası (ad → tip → NOT NULL → kaynak → ifade → Ayrıntılar → Sil); Ayrıntılar `aria-expanded`; çip çıkar düğmeleri etiketli; ekran okuyucu satır/kart adlarını okur. **[—]**
- [ ] Açık/koyu tema okunur; uzun tip/ifade metinleri yatay sayfa kaydırması yaratmaz; dar pencerede (≈1100 px) kolon satırı sarılır, taşmaz. **[—]**
