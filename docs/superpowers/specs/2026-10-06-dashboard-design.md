# pg-tools — Alt Proje 4: Dashboard Tasarımı

- **Tarih:** 2026-10-06
- **Durum:** Taslak (onay bekliyor)
- **Kapsam:** Sunucu/veritabanı düzeyinde canlı izleme: oturumlar (`pg_stat_activity`), kilitler ve engelleme zinciri, canlı grafikler, yapılandırma, hazır transaction'lar, `pg_stat_statements`; oturum iptal/sonlandırma
- **Önceki alt projeler:** Object Explorer (Plan 1–4), Query Tool (Plan 5–6), Editör zekâsı (Plan 7) — hepsi `main`'de.

---

## 1. Amaç ve bağlam

pgAdmin4'ün Dashboard'u yavaş açılır, kendi yenilemesiyle sunucuyu gereksiz yorar ve kilit/engelleme sorunlarında yetersiz kalır. Burada amaç: "şu an sunucuda ne oluyor, kim kimi bekliyor, hangi oturumu durdurmalıyım" sorularını hızlı yanıtlayan hafif bir ekran. Üç temel hedef aynı: **açılış hızı** (ana pakete yük yok), **bellek**, **sunucuya düşük yük**.

### 1.1 Kullanıcıdan gelen kararlar

| Konu | Karar |
|---|---|
| Bölümler | Oturumlar, Kilitler/engellemeler, Canlı grafikler, Yapılandırma (`pg_settings`) + hazır transaction'lar, `pg_stat_statements` (kuruluysa) |
| Eylemler | `pg_cancel_backend` ve `pg_terminate_backend`, onay diyaloğuyla; kilitli (salt okunur) sunucuda da serbest (veri değiştirmezler) |
| Düzey | Sunucu seçiliyse tüm sunucu; veritabanı seçiliyse o veritabanına süzülmüş görünüm |
| Yenileme | Otomatik ve ayarlanabilir; varsayılan grafik 2 sn, liste 5 sn; duraklat düğmesi; sekme görünmezken durur |
| Yer | Query sekmelerinin yanında ayrı sekme türü (`dashboard`) |
| Grafik | Kendi hafif Canvas çizimimiz; yeni bağımlılık yok |
| `pg_stat_statements` | Eklenti varsa göster, yoksa "kurulu değil" notu |

### 1.2 Başarı ölçütleri

| Ölçüt | Hedef |
|---|---|
| Ana paket | Dashboard kodu tembel parça; ana paket boyutu **artmaz** (ölçüm betiği aynı eşikle geçer) |
| Açılış | Sekme açılıp ilk veri görünene kadar < 500 ms (yerel ağ) |
| Sunucu yükü | Grafik örneği tek hafif sorgu (≈ 2 katalog görünümü); aynı anda tek istek uçuşta |
| Bellek | Grafik geçmişi yalnızca bellekte, en fazla 300 örnek (2 sn'de 10 dk); sekme kapanınca serbest |
| Doğruluk | PG 10–18'in hepsinde her sorgu çalışır (test matrisi) |

### 1.3 Kapsam dışı

- Geçmişin diske yazılması, uyarı/alarm, dışa aktarma.
- Replikasyon (`pg_stat_replication`), vacuum/bloat, WAL, tablo bazlı istatistik.
- `pg_settings` değiştirme (`ALTER SYSTEM`), sorgu planı, `pg_stat_statements_reset`.
- Birden çok sunucuyu tek ekranda karşılaştırma.

---

## 2. Mimari

- Yeni crate **`pgt-dash`**; `pgt-core` bağlantı katmanını kullanır. Object Explorer havuzunu ve Query Tool oturumlarını **kullanmaz**.
- Dashboard sekmesi başına tek **ayrı bağlantı** (`open_dedicated`, yeni `SessionProfile::Dashboard`): `application_name = 'pg-tools dash'`, `statement_timeout = 10s`, `default_transaction_read_only = on`, `lock_timeout` kısa. Sekme kapanınca bağlantı kapanır. Veritabanı düzeyinde bağlantı o veritabanına açılır (OID çözümlemesi Object Explorer ile aynı).
- Sorgu metinleri **sabit** SQL dosyalarıdır (`crates/pgt-dash/src/sql/*.sql`, sürüme göre `.pg10` / `.pg13` sonekleri). Kullanıcıdan SQL alınmaz; tek parametre `pid` (`i32`) ve süzgeç değerleridir (bağlı parametre).
- Oturumlar kendi bağlantısını (`pg_backend_pid()`) tanır; kendi pid'i listede işaretlenir ve eylemleri kapalıdır.
- Tauri komutları (`dash_*`, `Box::pin`, ts-rs bağlamaları): `dash_open(server_id, db_oid?) → DashId`, `dash_close`, `dash_sample`, `dash_activity`, `dash_locks`, `dash_settings`, `dash_prepared`, `dash_statements`, `dash_cancel(pid)`, `dash_terminate(pid)`.
- Bağlantı koparsa: son veri soluk gösterilir, üstte "bağlantı koptu · yeniden dene" şeridi; otomatik yeniden bağlanma üstel bekleme ile (en çok 30 sn).

## 3. Veri kaynakları

| Bölüm | Kaynak | Not |
|---|---|---|
| Örnek (grafik) | `pg_stat_database` toplamı/satırı + `pg_stat_activity` durum sayıları | `xact_commit`, `xact_rollback`, `tup_*`, `blks_read`, `blks_hit`; sunucu düzeyinde tüm veritabanları, veritabanı düzeyinde `datname` süzgeci |
| Oturumlar | `pg_stat_activity` | `left(query, 4096)`; `backend_type` (PG10+), `leader_pid` (PG13+, yoksa `NULL`), `wait_event*` (PG10+). Yetkisiz kullanıcıda başkalarının sorgusu `NULL` gelir; arayüz "yetki yok" der |
| Kilitler | `pg_locks` ⨝ `pg_stat_activity`, `pg_blocking_pids(pid)` | Engelleme zinciri istemcide ağaca çevrilir (kök engelleyici → beklerler); `relation` yalnızca bağlı veritabanında `regclass` ile çözülür |
| Yapılandırma | `pg_settings` | Talep üzerine (yenile düğmesi); arama + "değiştirilmiş" (`source <> 'default'`) + "yeniden başlatma bekliyor" (`pending_restart`, PG10+) süzgeçleri |
| Hazır tx | `pg_prepared_xacts` | Talep üzerine; 1 saatten eskiler uyarıyla işaretlenir (donmuş vacuum riski) |
| Sorgu istatistikleri | `pg_stat_statements` | Bağlı veritabanında `pg_extension` ile varlık denetimi. PG13+ `total_exec_time`/`mean_exec_time`, PG10–12 `total_time`/`mean_time` (iki sorgu dosyası). İlk 50, toplam süreye göre; sıralama değiştirilebilir (ortalama, çağrı sayısı). Sunucu düzeyinde eklenti yalnızca bakım veritabanında aranır; yoksa not gösterilir |

**Hız hesabı (istemci):** saniyelik oranlar ardışık örnek farkından `Δ / Δt` ile bulunur. Sayaç geriye giderse (`pg_stat_reset`, veritabanı yeniden oluşturma) o örnek atlanır, çizgide boşluk bırakılır. İlk örnek yalnızca taban çizgisidir.

## 4. Arayüz

Sekme: `Dashboard · <sunucu>[/<veritabanı>]` (sunucu rengi/etiketi Query sekmeleriyle aynı). Üst çubukta: yenileme aralığı (1/2/5/10/30 sn), **Duraklat**, **Yenile**, bağlantı durumu.

- **Genel:** 5 grafik (Oturumlar — etkin/boşta/boşta-tx yığılı; İşlem/sn — commit/rollback; Satır girişi — insert/update/delete; Satır çıkışı — returned/fetched; Blok G/Ç — read/hit) + özet kartları (sürüm, çalışma süresi, oturum/`max_connections`, en eski açık transaction).
- **Oturumlar:** sanal tablo; sütunlar pid, veritabanı, kullanıcı, uygulama, istemci, durum, bekleme, süre (sorgu/transaction/durum), sorgu (tek satır, tıklayınca altta tam metin + kopyala). Süzgeçler: durum, veritabanı, "yalnızca etkin", "boşta-tx ≥ N sn", arama. Satır eylemleri: **İptal** (sorguyu), **Sonlandır** (oturumu).
- **Kilitler:** engelleme zinciri ağacı (kimi kim bekliyor, bekleme süresi, kilit modu, nesne) + düz `pg_locks` listesi. Ağaçtan kök engelleyiciyi iptal/sonlandır kısayolu.
- **Yapılandırma:** `pg_settings` tablosu ve altında **Hazır transaction'lar** bölümü.
- **Sorgu istatistikleri:** yalnızca eklenti varsa sekme görünür; yoksa pasif ve "pg_stat_statements kurulu değil" notu.

**Onay diyaloğu (iptal/sonlandır):** pid, kullanıcı, uygulama ve sorgunun ilk satırları gösterilir. Ek uyarılar: `application_name = 'pg-tools query'` ise "bu pg-tools'un kendi sorgu sekmesi"; oturum açık transaction içindeyse (`state` = `idle in transaction*`) "geri alınacak"; `Sonlandır` için "oturum kapanır". Sonuç `false` ya da yetki hatası ise toast ile açıklanır ("yalnızca kendi oturumlarınızı ya da `pg_signal_backend` üyeliğiyle başkalarınınkini durdurabilirsiniz").

**Yenileme kuralları:** tek uçuşta istek (öncekisi bitmeden yeni tick atlanır); sekme etkin değilse veya pencere gizliyse duraklar, dönünce hemen bir yenileme yapar; sorgu hatasında o bölümde şerit gösterilir, son veri korunur. Liste istekleri yalnızca açık olan bölüm için yapılır (Genel açıkken yalnızca örnek sorgusu).

**Grafik çizimi:** tek bileşen, `<canvas>`, `devicePixelRatio`'ya duyarlı, `requestAnimationFrame` ile yalnızca veri değişince çizer; tema renkleri CSS değişkenlerinden; imleç üstünde değer ipucu; ekran okuyucu için son değerleri özetleyen `aria-label`/gizli tablo.

## 5. Güvenlik ve gizlilik

- Sabit SQL; kullanıcı girdisi yalnızca bağlı parametre. Dashboard bağlantısı salt okunur oturumdur; `pg_cancel/terminate_backend` bunun içinde çalışır.
- Sorgu metinleri (oturumlar, `pg_stat_statements`) **günlüğe yazılmaz, diske kaydedilmez, hata iletilerine konmaz**; yalnızca ekranda ve panoya (kullanıcı kopyalarsa) bulunur.
- Parola/URL günlüğe girmez (mevcut kural).
- Yetkisiz kullanıcı: başkalarının sorgusu görünmez, eylem hata verir — arayüz bunu "yetki yok" olarak açıklar, sessizce boş bırakmaz.

## 6. Test stratejisi

- **Rust (Docker, PG 10–18 matrisi, `scripts/test-matrix.ps1`):** her sorgu her sürümde çalışır ve beklenen sütunları döndürür; sürüme özel dosyaların seçimi; engelleme zinciri gerçek iki oturumla (biri `LOCK TABLE`, diğeri bekleyen); `cancel`/`terminate` gerçek oturumda; yetkisiz rolle (`pg_monitor`'suz) sorgunun `NULL` gelmesi ve `terminate` hatası; `pg_stat_statements` varken/yokken.
- **Birim (Rust/TS):** hız hesabı ve sayaç sıfırlama, halka tampon, engelleme ağacı kurma (döngü/ortak engelleyici), süzgeçler, onay diyaloğu uyarı mantığı.
- **Ön yüz:** sekme yaşam döngüsü (açma/kapama/bağlantı kopması), yenileme duraklatma/görünürlük, tek uçuşta kuralı, tembel yükleme (ana pakette dashboard modülü yok).
- **Ölçüm:** `measure.ps1` ile ana paket ve açılış değişmez; elle kontrol maddeleri `docs/smoke-test.md`'ye eklenir.

## 7. Planlama notu

Önerilen plan bölümü: **Plan 8** (arka uç: `pgt-dash`, SQL dosyaları, komutlar, Docker testleri) ve **Plan 9** (ön yüz: sekme türü, grafik bileşeni, tablolar, eylemler). Plan 8, Plan 9 başlamadan birleştirilir.
