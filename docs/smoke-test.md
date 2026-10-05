# pg-tools — Duman Testi (Plan 1)

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

## Ölçümler

| Tarih | Makine | Paket (MB) | Açılış (ms) | Toplam bellek (MB) | Rust süreci (MB) |
|---|---|---|---|---|---|
| 2026-10-05 | AMD Ryzen 9 5950X (16 çekirdek), 32 GB RAM, Windows 11 | 3,2 (NSIS kurulum paketi) | 131 / 63 / 31 (3 çalıştırma) | 390 / 377 / 376 (çalışma kümesi; özel bellek ≈191) | 40 / 30 / 30 |
Notlar: Ölçüm `target/release/pg-tools.exe` doğrudan çalıştırılarak yapıldı (kurulum yapılmadı); boşta 5 sn, sunucu bağlı değil. Toplam bellek, WebView2 alt süreçleri dahil 7 sürecin çalışma kümesi toplamıdır (paylaşılan sayfalar tekrar sayıldığından özel bellek ≈191 MB). Açılış süresi `MainWindowHandle` oluşana kadar geçen süredir; içerik çizimi dahil olmayabilir. Yukarıdaki kutucuklar GUI üzerinden elle uygulanmadı; sürüm öncesi işaretlenmelidir.
