# pg-tools — Duman Testi (Plan 1–3)

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
- [ ] Sunucu kapalıyken genişlet → düğümde hata rozeti; rozete tıklama → hata kartı (Türkçe ileti) ve "Tekrar dene"
- [ ] Sunucu yeniden açıldıktan sonra "Tekrar dene" → düğüm yüklenir, rozet kaybolur
- [ ] Başka bir oturumda tabloyu `DROP` et → bu tabloyu yenile/DDL'ini kopyala → "Nesne artık mevcut değil." kartı ve "Üst düğümü yenile"; menü eyleminde "Bu öğe artık yok." bildirimi; üst düğüm yenilenince tablo listeden çıkar
- [ ] Rozet düğmesi odaklıyken ok tuşları/Enter ağaç gezintisini tetiklemez

### Sistem nesneleri anahtarı
- [ ] Varsayılan kapalı → `pg_catalog`, `information_schema`, `pg_toast*` / `pg_temp_*` şemaları ve `pg_*` roller görünmez
- [ ] Araç çubuğundan açılır → bağlı sunucuların ağacı yenilenir ve sistem nesneleri görünür; tercih yeniden açılışta korunur

### Durum çubuğu
- [ ] Hiçbir şey seçili değilken → "Hazır"
- [ ] Bağlı sunucunun altındaki bir düğüm seçiliyken → "sunucu · PostgreSQL <sürüm> · <veritabanı> · bağlı" (veritabanı yalnızca veritabanı düğümü ve altında)
- [ ] Bağlı olmayan sunucu seçiliyken → "sunucu · bağlı değil"

## Ölçümler

| Tarih | Makine | Paket (MB) | Açılış (ms) | Toplam özel WS (MB) | Rust özel WS (MB) |
|---|---|---|---|---|---|
| 2026-10-05 | AMD Ryzen 9 5950X (16 çekirdek), 32 GB RAM, Windows 11 | 3,2 (NSIS kurulum paketi) | 131 / 63 / 31 (3 çalıştırma) | ≈191 (Plan 1 "özel bellek"; çalışma kümesi 390 / 377 / 376) | 40 / 30 / 30 (çalışma kümesi) |
| 2026-10-05 (Plan 2) | aynı makine | — (paket üretilmedi; exe 12,4) | 40 (5 çalıştırma medyanı) | 76,1 (toplam WS 356,9) | 7,5 |
Notlar: Plan 2 satırı `scripts/measure.ps1 -Runs 5` ile ölçüldü (8 sn boşta, sunucu yok, seçilen WebView2/tokio ayarlarıyla); yöntem ve denenen ayarlar `docs/performance.md`'de. Plan 1 satırı: ölçüm `target/release/pg-tools.exe` doğrudan çalıştırılarak yapıldı (kurulum yapılmadı); boşta 5 sn, sunucu bağlı değil. Toplam bellek, WebView2 alt süreçleri dahil 7 sürecin çalışma kümesi toplamıdır (paylaşılan sayfalar tekrar sayıldığından özel bellek ≈191 MB). Açılış süresi `MainWindowHandle` oluşana kadar geçen süredir; içerik çizimi dahil olmayabilir. Yukarıdaki kutucuklar GUI üzerinden elle uygulanmadı; sürüm öncesi işaretlenmelidir.
