# pg-tools — Duman Testi (Plan 1–6)

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
- [ ] Etiketsiz sunucuda ilk açılışta ortam istemi (Üretim / Test / Atla): "Üretim" kilit kutusunu işaretli getirir; "Atla" seçimi hatırlanır; X ile kapatma sekmeyi açmaz. **[T8]** (Üretim/Test; "Atla" ve hatırlanması **[—]**)
- [ ] Parolası kaydedilmemiş sunucuda ilk çalıştırma parola ister, sonra çalışır; tabloyu içeren ağaç etiket kaydından sonra daralmaz. **[T8]**

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

## Ölçümler

| Tarih | Makine | Paket (MB) | Açılış (ms) | Toplam özel WS (MB) | Rust özel WS (MB) |
|---|---|---|---|---|---|
| 2026-10-05 | AMD Ryzen 9 5950X (16 çekirdek), 32 GB RAM, Windows 11 | 3,2 (NSIS kurulum paketi) | 131 / 63 / 31 (3 çalıştırma) | ≈191 (Plan 1 "özel bellek"; çalışma kümesi 390 / 377 / 376) | 40 / 30 / 30 (çalışma kümesi) |
| 2026-10-05 (Plan 2) | aynı makine | — (paket üretilmedi; exe 12,4) | 40 (5 çalıştırma medyanı) | 76,1 (toplam WS 356,9) | 7,5 |
| 2026-10-06 (Plan 6) | aynı makine | 3,56 (NSIS; ana parça `index` 228 kB, Query Tool tembel) | 45 (5 çalıştırma medyanı; Query Tool sekmesi ilk açılış 38–47 ms) | 76,1 boşta; 10.000 satırlı sorgu sekmesiyle en çok 109,1 | 7,4 |
Notlar: Birimler — Plan 2 satırının bellek ve exe değerleri `measure.ps1`/bayt sayısından geldiği için başlıktaki "MB" orada MiB'dir (1 MiB = 1.048.576 bayt); Plan 1 satırının birimi belirsiz kaynaktandır (bkz. `docs/performance.md` "Birimler"). Plan 2 satırı `scripts/measure.ps1 -Runs 5` ile ölçüldü (8 sn boşta, sunucu yok, seçilen WebView2/tokio ayarlarıyla); yöntem ve denenen ayarlar `docs/performance.md`'de. Plan 1 satırı: ölçüm `target/release/pg-tools.exe` doğrudan çalıştırılarak yapıldı (kurulum yapılmadı); boşta 5 sn, sunucu bağlı değil. Toplam bellek, WebView2 alt süreçleri dahil 7 sürecin çalışma kümesi toplamıdır (paylaşılan sayfalar tekrar sayıldığından özel bellek ≈191 MB). Açılış süresi `MainWindowHandle` oluşana kadar geçen süredir; içerik çizimi dahil olmayabilir. Yukarıdaki kutucuklar GUI üzerinden elle uygulanmadı; sürüm öncesi işaretlenmelidir.
