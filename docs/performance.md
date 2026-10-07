# Performans ve bellek ölçümleri

## Yöntem

Birimler: `measure.ps1` ile ölçülen değerler ve bayt sayısıyla verilen boyutlar ikili birimdir — 1 KiB = 1024 bayt,
1 MiB = 1.048.576 bayt (betik PowerShell'in `1MB` sabitiyle böler; bu da 1 MiB'dir). Plan 1 değerleri ve pgAdmin
tahmini birimi belirsiz kaynaklardan geldiği için MB olarak bırakıldı. Spec hedefleri spec'teki gibi MB yazılır;
ölçülen her değer hedefin hem MB hem MiB okumasıyla çok altındadır.

    bun tauri build --no-bundle        # ya da bun tauri build (kurulum paketiyle)
    ./scripts/measure.ps1 -Runs 5      # varsayılan exe: target\release\pg-tools.exe

`scripts/measure.ps1` her çalıştırmada:

- ayrı bir geçici `APPDATA` dizini kullanır (gerçek `%APPDATA%\pg-tools` yapılandırmasına dokunmaz; sunucu
  listesi boş, hiçbir sunucuya bağlanılmaz),
- ana pencerenin (`Tauri Window` sınıfı) **görünür** olmasına kadar geçen süreyi **pencere süresi** olarak ölçer.
  Beyaz kare düzeltmesinden (aşağıda) beri pencere gizli oluşturulur ve ön yüz tema + ilk çizimden sonra gösterir;
  bu süre artık ön yüz paketinin çalışmasını kapsar. Önceki ölçümler `MainWindowHandle`'ı bekliyordu (aşağıya bakın),
- 8 sn boşta bekler, sonra uygulama sürecini ve tüm alt süreçlerini (WebView2 tarayıcı, GPU, yardımcı ve
  çizici süreçleri) bulur ve `Win32_PerfFormattedData_PerfProc_Process` sayaçlarından okur:
  - **toplam özel çalışma kümesi** (`WorkingSetPrivate` toplamı — Görev Yöneticisi "Bellek" sütununun toplamı),
  - toplam çalışma kümesi (`WorkingSet` toplamı; bilgi amaçlı),
  - Rust (`pg-tools.exe`) sürecinin özel çalışma kümesi,
- başlattığı tüm süreçleri kapatır ve geçici dizini siler.

Medyan, 5 çalıştırmanın ortadaki değeridir. İlk çalıştırmanın pencere süresi soğuk disk önbelleği yüzünden
genellikle 100+ ms'dir; sonrakiler 20–45 ms.

### Metrik neden değişti

Plan 1 "toplam bellek" olarak çalışma kümesi toplamını raporladı (~380 MB). Çalışma kümesi, WebView2
süreçlerinin paylaştığı DLL ve bellek eşlemeli sayfaları her süreçte yeniden sayar; 7 süreçte bu ~280 MB'lık
şişmeye yol açar. Kullanıcının makinesinden gerçekten "aldığımız" bellek özel çalışma kümesidir; spec §1.3
hedefi Plan 2'de bu metrikle yeniden tanımlandı.

## Ortam

| | |
|---|---|
| CPU | AMD Ryzen 9 5950X 16-Core Processor |
| RAM | 32 GB |
| GPU | AMD Radeon RX 7700 XT |
| İşletim sistemi | Windows 11 Pro 10.0.26300 |
| WebView2 Runtime | 154.0.4258.53 |
| Ekran | 75 Hz |

## Taban çizgisi (2026-10-05, Plan 2 sonu, ayar yok)

5 çalıştırma medyanı: pencere **27 ms**, toplam özel WS **110,2 MiB**, toplam WS 392,3 MiB,
Rust özel WS **8,5 MiB**, 7 süreç.

Plan 1'deki ~191 MB "özel bellek" değeri büyük olasılıkla farklı bir sayaçtı (ör. Private Bytes; RAM'de
olmayan taahhüt edilmiş sayfaları da içerir); bu betikle ölçülen özel çalışma kümesi ayarsız hâliyle 110 MiB'dır.
Rust sürecinin Plan 1'deki 30–40 MB değeri de çalışma kümesiydi; özel çalışma kümesi 8,5 MiB.

> Not: ölçüme başlarken release exe açılıştan hemen sonra `STATUS_STACK_OVERFLOW` ile çıkıyordu. Plan 2'de
> büyüyen katalog gelecekleri (DDL geleceği ~70 KiB) Tauri'nin invoke işleyicisinde ana iş parçacığının
> 1 MiB'lık yığınına gömülüyordu. `get_ddl`/`get_properties`/`list_children` komutlarında gelecekler
> `Box::pin` ile kutulanarak giderildi (ayrı commit); taban çizgisi bu düzeltmeden sonra ölçüldü.

## Denenen ayarlar

Her satır ayrı bir release derlemesi, 5 çalıştırma medyanıdır. WebView2 argümanları
`app.windows[0].additionalBrowserArgs` ile verildi; bu alan Tauri'nin varsayılan argümanlarının yerine geçtiği
için varsayılan `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` her denemede korundu.

| Deneme | Ayar | Pencere (ms) | Toplam özel WS (MiB) | Toplam WS (MiB) | Rust özel WS (MiB) | Sonuç |
|---|---|---|---|---|---|---|
| Taban | — | 27 | 110,2 | 392,3 | 8,5 | |
| A | `--renderer-process-limit=1` | 41 | 110,3 | 392,8 | 8,5 | etkisiz (tek sayfa zaten tek çizici süreç kullanıyor) |
| B | A + `--disable-gpu --disable-gpu-compositing` | 37 | **74,3** | 355,0 | 8,5 | **−36 MiB**; kullanılabilirlik doğrulandı (aşağıda) |
| C | A + `--js-flags=--max-old-space-size=64` | 39 | 110,4 | 391,5 | 8,5 | etkisiz; büyük ağaçlarda yığın sınırı riski — seçilmedi |
| R1 | tokio çalışma zamanı 2 işçi iş parçacığı (`main.rs`) | 32 | 110,5 | 393,9 | 7,6 | Rust −0,9 MiB; güvenli |
| R2 | `[profile.release]` `panic = "abort"`, `opt-level = "s"` | 39 | 111,3 | 394,1 | 8,4 | bellek etkisiz; exe 12,4 → 5,8 MiB. Seçilmedi: `panic = "abort"` bir komuttaki panik yüzünden tüm uygulamayı kapatır (şu an tokio görevi panikte yalnızca o komut başarısız olur), `opt-level = "s"` DDL/ağaç yolunu yavaşlatabilir; kazanç bellek hedefiyle ilgisiz |
| **B + R1** | seçilen birleşim | 40 | **76,1** | 356,9 | **7,5** | seçildi |

Pencere süresi hiçbir denemede 1000 ms sınırına yaklaşmadı.

### B (yazılım çizimi) kullanılabilirlik doğrulaması

GPU kapatmak WebView2 süreçlerinin özel belleğini ~36 MiB azaltır (GPU sürecinin sürücü/D3D kaynakları); çizim
WARP (`Microsoft Basic Render Driver`) ile CPU'da yapılır. Kaydırma akıcılığı CDP ile ölçüldü (exe
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223` ile açılıp playwright-core
`connectOverCDP` ile bağlanıldı):

- 3.000 tablolu bir sınama veritabanında Server → Databases → … → Tables açıldı (ağaç kaydırma yüksekliği
  72.480 px, sanal liste).
- Programatik kaydırma (180 kare boyunca her karede 60 px) ve fare tekerleği kaydırması (60 × 100 px)
  sırasında `requestAnimationFrame` aralıkları ölçüldü.

| Mod | WebGL çizici | Ortalama fps | p95 kare (ms) | En uzun kare (ms) | 25 ms üstü kare |
|---|---|---|---|---|---|
| GPU açık (taban) | AMD Radeon RX 7700 XT (D3D11) | 75 | 13,5 | 13,7 | 0 |
| GPU kapalı (B) | Microsoft Basic Render Driver (WARP) | 75 | 13,4 | 13,5 | 0 |

Her iki modda ekran tazeleme hızı (75 Hz) tutturuldu, hiç kare atlanmadı. Aynı oturumda tablo seçilip DDL
sekmesi açıldı (CodeMirror sözdizimi renkli çizildi), tema düğmesi sistem → açık → koyu döndürüldü
(arka plan `rgb(251,251,252)` ↔ `rgb(22,24,29)`), ekran görüntüleri incelendi; konsol hatası yok.

Sınır: ölçüm güçlü bir masaüstü CPU'sunda ve 1280×800 pencerede yapıldı. Yazılım çizimi CPU'ya yük bindirir;
zayıf dizüstü bilgisayarlarda ya da yüksek DPI'lı büyük pencerelerde kaydırma ölçülmedi. Sorun görülürse
`tauri.conf.json`'daki `--disable-gpu --disable-gpu-compositing` kaldırılarak GPU çizimine dönülür
(bellek ~110 MiB'a çıkar, yine hedefin altında).

## Seçilen ayar

- `src-tauri/tauri.conf.json`: `"additionalBrowserArgs": "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --renderer-process-limit=1 --disable-gpu --disable-gpu-compositing"`
- `src-tauri/src/main.rs`: tokio çalışma zamanı 2 işçi iş parçacığı (`tauri::async_runtime::set`).
- `Cargo.toml` `[profile.release]` değişmedi.

Sonuç (B + R1, 5 çalıştırma medyanı): pencere 40 ms, **toplam özel WS 76,1 MiB**, toplam WS 356,9 MiB,
**Rust özel WS 7,5 MiB**.
Testlerden sonra yeniden derlenip `-Runs 3` ile tekrarlanan doğrulama: pencere 39 ms, toplam özel WS 73,6 MiB,
toplam WS 354,5 MiB, Rust özel WS 7,4 MiB.

## Hedef

Karar kuralı (Plan 2, Task 10): seçilen ayarla toplam özel çalışma kümesi < 150 MB ise spec hedefi aynen
kalır. 76,1 MiB < 150 MB olduğundan hedef değerleri değişmedi; yalnızca metrik açıkça "özel çalışma kümesi"
olarak tanımlandı:

- Boşta toplam bellek (uygulama + WebView2 süreçleri, özel çalışma kümesi): **< 150 MB** (ölçülen 76 MiB;
  GPU çizimiyle 110 MiB)
- Rust süreci (özel çalışma kümesi): **< 30 MB** (ölçülen 7,5 MiB)

Karşılaştırma: pgAdmin4 masaüstü (Electron/NW.js kabuğu + Python sunucusu) Plan 1 notlarına göre boşta
~400 MB+ kullanır.

## Plan 3 sonu (2026-10-05)

Plan 3 (pgAdmin içe aktarma, filtre, sağ tık menüsü, hata rozeti, sistem nesneleri anahtarı, durum çubuğu,
tembel yüklenen diyaloglar) sonrasında aynı makinede, Plan 2'de seçilen ayarlarla (B + R1) yeniden ölçüldü:

    bun run build
    cargo build --release -p pg-tools
    powershell -NoProfile -File scripts/smoke-release.ps1     # BAŞARILI, çıkış kodu 0 (pencere 192 ms, 6 sn ayakta)
    powershell -NoProfile -File scripts/measure.ps1 -Runs 5

> **Not (sonradan eklendi):** bu bölümün exe'si düz `cargo build --release` ile derlendi; ön yüzü gömmeyen (devUrl'ye
> yönelen) bir exe olabilir (bkz. Plan 6 bölümündeki not ve `docs/testing.md` "Yayın exe duman testi"). Rakamlar
> değiştirilmedi; arayüzsüz WebView ile ölçülmüş olabilirler.

| Çalıştırma | Pencere (ms) | Toplam özel WS (MiB) | Toplam WS (MiB) | Rust özel WS (MiB) | Süreç |
|---|---|---|---|---|---|
| 1 | 121 | 73,7 | 348,2 | 7,2 | 7 |
| 2 | 43 | 73,4 | 347,7 | 7,3 | 7 |
| 3 | 46 | 73,7 | 348,4 | 7,2 | 7 |
| 4 | 30 | 73,3 | 347,9 | 7,2 | 7 |
| 5 | 29 | 73,2 | 347,1 | 7,2 | 7 |
| **Medyan** | **43** | **73,4** | 347,9 | **7,2** | 7 |

Hedeflerin hepsi karşılandı (pencere < 1 sn, toplam özel WS < 150 MB, Rust < 30 MB); karar kuralının
uygulanmasına gerek kalmadı. Plan 2 sonuna göre (76,1 / 7,5 MiB) fark ölçüm gürültüsü düzeyindedir. `ImportDialog` ve
`ServerDialog` Plan 3'te tembel yüklenen ayrı parçalara ayrıldı (`DdlView` Plan 1'den beri ayrı); yeni
özellikler açılıştaki boşta belleği artırmadı.

Boyutlar:

| | Boyut |
|---|---|
| `dist` (ön yüz derlemesi, toplam) | 526 KiB (538.567 bayt); en büyük parçalar `DdlView` 288 KiB, `index` 184 KiB, CSS 17 KiB |
| `target\release\pg-tools.exe` (`cargo build --release -p pg-tools`) | 12,4 MiB (12.964.352 bayt) |
| `target\release\pg-tools.exe` (`bun run tauri build`) | 12,5 MiB (13.130.752 bayt) |
| NSIS kurulum paketi `pg-tools_0.1.0_x64-setup.exe` (WebView2 gömülü değil) | **3,24 MiB** (3.402.548 bayt) |

NSIS paketi < 15 MB hedefinin (spec §1.3) altında. `tauri build` exe'si de `smoke-release.ps1` ile sınandı
(BAŞARILI, pencere 186 ms).

Filtre (Ctrl+F): `tree-model.test.ts` "filters 10,000 loaded nodes in under 50 ms" testi — 10.000 yüklenmiş
tablo düğümünde filtreli düzleştirmenin 5 denemedeki en kısa süresi **0,93 ms** (3 ayrı çalıştırmada
0,94 / 0,93 / 0,93 ms; bütçe 50 ms; vitest, Node, aynı makine).

## Plan 5 sonu: Query Tool arka ucu (2026-10-06)

Plan 5 arka uç ekler (`pgt-query` kutusu, `qt_*` komutları, `futures-util` bağımlılığı); Query Tool arayüzü henüz
yok (Plan 6), bu yüzden boşta bellek değişmemelidir. Aynı makinede, seçilen ayarlarla (B + R1):

    bun run build
    cargo build --release -p pg-tools
    powershell -NoProfile -File scripts/smoke-release.ps1     # BAŞARILI, çıkış kodu 0 (pencere 158 ms, 6 sn ayakta)
    powershell -NoProfile -File scripts/measure.ps1 -Runs 5

> **Not (sonradan eklendi):** bu bölümün exe'si düz `cargo build --release` ile derlendi; ön yüzü gömmeyen (devUrl'ye
> yönelen) bir exe olabilir (bkz. Plan 6 bölümündeki not ve `docs/testing.md` "Yayın exe duman testi"). Rakamlar
> değiştirilmedi; arayüzsüz WebView ile ölçülmüş olabilirler.

| Çalıştırma | Pencere (ms) | Toplam özel WS (MiB) | Toplam WS (MiB) | Rust özel WS (MiB) | Süreç |
|---|---|---|---|---|---|
| 1 | 34 | 72,8 | 347,0 | 7,2 | 7 |
| 2 | 40 | 73,2 | 349,5 | 7,3 | 7 |
| 3 | 41 | 73,1 | 347,2 | 7,2 | 7 |
| 4 | 40 | 76,0 | 351,8 | 7,2 | 7 |
| 5 | 40 | 72,5 | 348,9 | 7,2 | 7 |
| **Medyan** | **40** | **73,1** | 348,9 | **7,2** | 7 |

Hedeflerin hepsi karşılandı (pencere < 1 sn, toplam özel WS < 150 MB, Rust < 30 MB); karar kuralı uygulanmadı.
Plan 3 sonuna göre (73,4 / 7,2 MiB) fark yok.

### Exe boyutu (öncesi/sonrası)

`cargo build --release -p pg-tools`, aynı `dist` ile, temiz hedef dizini:
(Exe boyutu karşılaştırması için geçerli; bu exe ön yüzü gömmez — gömülü exe ~0,2 MiB daha büyüktür.)

| Commit | `pg-tools.exe` |
|---|---|
| `abd97da` (Plan 5 öncesi, `main`) | 12,45 MiB (13.059.584 bayt) |
| Plan 5 sonu (HEAD) | 13,34 MiB (13.992.448 bayt) |
| Fark | **+0,89 MiB (+932.864 bayt, +%7,1)** |

Fark `pgt-query` (lexer/sınıflandırıcı/çalıştırıcı/oturum yöneticisi/CSV), `qt_*` komutları ve `futures-util` +
`pgt-core` değişikliklerinin toplamıdır; `futures-util` ayrı ölçülmedi (yeni bağımlılık yalnızca bu; ayrı bir
derleme gerektirirdi). NSIS paketi yeniden üretilmedi; exe farkı kadar büyümesi beklenir (sıkıştırılmış < 1 MiB),
15 MB hedefinin çok altında.

### Akış, gecikme ve iptal ölçümleri (bilgi amaçlı)

PG 18 (Docker, 127.0.0.1), **debug** test derlemesi, `PGT_QUERY_BIG=1 cargo test -p pgt-query --test memory -- --nocapture`:

| Ölçüm | Değer |
|---|---|
| 1M satır `generate_series`, `limit=10.000`, parça 1.000 | tam 10.000 satır; hiçbir `Rows` olayı > 1.000; `more()` x3 -> 3 x 1.000; tepe tampon <= 1.000 satır; kapatınca `idle in transaction` yok (test assert eder) |
| 1M satır CSV dışa aktarma | 1,82 sn (~550.000 satır/sn), 7.888.913 bayt, 200 ilerleme olayı (5.000'lik parçalar, monoton), tepe tampon 5.000 satır |
| Tamamlama: şemalar (soğuk / önbellekli) | 2,6 ms / 9,8 us |
| Tamamlama: `pg_catalog` ilişkileri (144 öğe, soğuk) | 3,0 ms |
| İptal: `select pg_sleep(30)` -> `57014` (iptal isteğinden `run` dönene dek) | 4,3 ms |

Süreler tek çalıştırmadır, assert edilmez; CSV hızı debug derlemede ve yerel Docker'da ölçüldü (yayın derlemesinde
daha yüksek olması beklenir).

Sınır: akış sayaçları yalnızca **satır** sayısını sınırlar; hücre başına bayt sınırı yoktur (1 GB'lık tek bir `bytea`/`text` hücresi bütün olarak bellekte tutulur ve gönderilir). Geniş satırlarda tepe bellek `satır x genişlik` ile büyür; ölçümlerdeki 1M satır dar (tek `int`) satırlardır.

## Plan 6 sonu: Query Tool arayüzü (2026-10-06)

Plan 6 arayüzü ekler (sekmeli sağ panel, CodeMirror editörü, sanal ızgara, geçmiş, taslaklar). Editör, ızgara ve
CodeMirror tembel yüklenen parçalardadır (`QueryTab`, `codemirror-setup`); ana pakete yalnızca sekme çubuğu,
denetleyici ve saf modeller girer. Boşta bellek bu yüzden Plan 5'ten anlamlı farklı olmamalıdır. Aynı makinede,
seçilen ayarlarla (B + R1), karar kuralı uygulanmadı (hiçbir hedef aşılmadı).

> **Önemli (ölçüm yöntemi düzeltmesi):** düz `cargo build --release -p pg-tools` çıktısı `custom-protocol` özelliği
> olmadan derlendiği için ön yüzü gömmez, `devUrl`'ye (`http://localhost:1420`) yönelir; dev sunucusu yokken pencerede
> bağlantı hatası sayfası görünür. Bu çalıştırmada bu doğrudan görüldü (CDP hedef URL'si `http://localhost:1420/`;
> gömülü exe'de `http://tauri.localhost/`). Gerçek uygulamanın ölçümü için `bun run tauri build` (ya da
> `bun tauri build --no-bundle`) çıktısı kullanılmalıdır. Aşağıda **ikisi de** verilir; Plan 3/Plan 5 bölümlerinde
> `cargo build --release` exe'si ölçülmüşse (73 MiB satırları) o değerler arayüzsüz WebView ile ölçülmüştür. Geçmiş
> rakamlar yeniden etiketlenmedi. 73 → 76 MiB kayması **ortamdandır** (büyük olasılıkla WebView2 çalışma zamanı
> güncellemesi): bugün aynı türden (arayüzsüz) `cargo build` exe'si de 75,9 MiB ölçüyor, Plan 5'te aynı tür 73,1 MiB'dı.

    bun run build
    cargo build --release -p pg-tools                        # exe: ön yüz gömülü DEĞİL (devUrl)
    powershell -NoProfile -File scripts/smoke-release.ps1    # BAŞARILI, çıkış kodu 0 (pencere 140 ms)
    powershell -NoProfile -File scripts/measure.ps1 -Runs 5  # cargo exe: medyan pencere 30 ms, toplam özel WS 75,9 MiB, Rust 7,1 MiB
    bun run tauri build                                      # gerçek exe + NSIS
    powershell -NoProfile -File scripts/smoke-release.ps1    # BAŞARILI, çıkış kodu 0 (pencere 236 ms)
    powershell -NoProfile -File scripts/measure.ps1 -Runs 5

### Boşta (gerçek exe, `bun run tauri build` çıktısı)

| Çalıştırma | Pencere (ms) | Toplam özel WS (MiB) | Toplam WS (MiB) | Rust özel WS (MiB) | Süreç |
|---|---|---|---|---|---|
| 1 | 143 | 76,1 | 357,1 | 7,3 | 7 |
| 2 | 43 | 76,2 | 356,9 | 7,4 | 7 |
| 3 | 45 | 75,9 | 357,1 | 7,4 | 7 |
| 4 | 45 | 76,1 | 357,4 | 7,4 | 7 |
| 5 | 45 | 76,2 | 357,4 | 7,5 | 7 |
| **Medyan** | **45** | **76,1** | 357,1 | **7,4** | 7 |

< 150 MB hedefi karşılandı. Query Tool boşta bellek eklemedi: aynı tür karşılaştırma (arayüzsüz `cargo build` exe'si)
bugün 75,9 / 7,1 MiB, Plan 5'te 73,1 / 7,2 MiB — +2,8 MiB'lık fark Plan 6 kodu yüklenmeden de görüldüğü için ortamdandır;
gerçek (gömülü) exe 76,1 MiB, Plan 2'nin `tauri build` sonucu 76,1 MiB ile aynı. Plan 2 (satır 10) doğru yöntemi
(`bun tauri build --no-bundle`) kullanmıştı; Plan 3 ve Plan 5 `cargo build`'e geçmişti.

### Query Tool senaryosu (elle, CDP ile)

`measure.ps1`'e `-Scenario query` eklenmedi: senaryo bir PostgreSQL ve oturum açmış bir pencere gerektirir (betik
bugün sunucusuz çalışır); basit olmadığı için elle, betikle ölçüldü:

1. `docker run -d --name pgt-qt-ui9 -e POSTGRES_PASSWORD=<rastgele> -e POSTGRES_INITDB_ARGS=--auth-host=scram-sha-256 -e POSTGRES_HOST_AUTH_METHOD=scram-sha-256 -p 127.0.0.1:55499:5432 postgres:18` (iş bitince `docker rm -f`).
2. Yalıtılmış `APPDATA`/`LOCALAPPDATA` (geçici dizin; gerçek `servers.json`/taslaklara dokunulmaz) ve
   `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="<tauri.conf.json'daki ayarlar> --remote-debugging-port=9224"` ile
   `target\release\pg-tools.exe` (tauri build) başlatıldı. **Dikkat:** bu ortam değişkeni `tauri.conf.json`'daki
   `additionalBrowserArgs`'ın yerine geçer; yalnız `--remote-debugging-port` verilirse GPU kapatma/çizici sınırı
   kaybolur ve bellek yükselir. Tam ayar kümesi verilince boşta 76,3 MiB (yukarıdakiyle aynı) ölçüldü.
3. Node (yerleşik `WebSocket`, bağımlılık yok) ile CDP (`http://127.0.0.1:9224/json`): sunucu `window.__TAURI_INTERNALS__.invoke("save_server", …)`
   ile kaydedildi (etiket "Test"), sayfa yenilendi, ağaçta çift tıklama + parola istemi + "Bağlan" (arayüz üzerinden),
   8 sn beklendi; "Yeni sorgu" düğmesine `mousedown` ile `.cm-content`'in DOM'a girişi arasındaki fark `performance.now`
   (sayfa içi `mousedown` yakalama dinleyicisi + `MutationObserver`) ile ölçüldü; ardından editöre
   `select g as id, md5(g::text) as txt, now() as ts, g * 1.5 as val from generate_series(1,10000) g;` yazılıp
   Ctrl+Enter ile çalıştırıldı.
4. Her aşamadan sonra `measure.ps1`'in süreç ağacı yöntemiyle (uygulama + tüm alt süreçler, `WorkingSetPrivate` toplamı; MiB) bellek okundu.
5. Sunucu `delete_server` ile silindi (`servers.json` boş doğrulandı), uygulama kapatıldı, geçici dizin ve kapsayıcı silindi.

3 bağımsız çalıştırma (her biri sıfır profil + yeni uygulama süreci):

| Çalıştırma | A: bağlı, sorgu sekmesi yok (MiB) | İlk sorgu sekmesi açılış (ms) | B: sekme açık, boş (MiB) | İlk satırlar görünür (ms)¹ | C: 10.000 satır, ~10 sn sonra (MiB) | D: sonuna kaydırıldıktan sonra (MiB) |
|---|---|---|---|---|---|---|
| 1 | 85,5 | 45,2 | 91,6 | 115 | 107,1 | 101,8 |
| 2 | 85,1 | 46,5 | 90,7 | 107 | 109,1 | 102,3 |
| 3 | 85,8 | 38,2 | 90,5 | 114 | 106,3 | 101,3 |
| (ön deneme) | 80,9 | 46,6 | 85,6 | 116 | 96,7 | 97,1 |

¹ Ctrl+Enter'den ilk `gridcell`'in görünmesine kadar, 100 ms yoklamayla (≈ ±100 ms); ilk çalıştırma oturumu açmayı
(`qt_open`) içerir. Spec §1.3 hedefi: basit sorguda (LAN, < 1.000 satır) ilk satır < 150 ms. Bu ölçüm 10.000 satır,
`qt_open` dahil ve ±100 ms yoklama çözünürlüğüyle yapıldığından hedefi **ne doğrular ne çürütür**; açık iş olarak
`docs/smoke-test.md`'de listelenir.

- **İlk sorgu sekmesi açılış süresi: 38–47 ms** (hedef < 500 ms). Süre lazy `QueryTab` + `codemirror-setup` parçalarının
  yüklenmesini, `qt_target_of`/`listServers` çağrılarını ve editörün kurulmasını içerir; ilk açılıştır (parçalar önbellekte değil).
  (`window` parçası sorgu sekmesinde değil, uygulama açılışında `QueryTabs.onMount` içinde yüklenir.)
- **10.000 satırlı sekmeyle toplam özel WS: en çok 109,1 MiB** (hedef < 250 MB); sonuna kaydırınca 101–102 MiB.
  Sunucuya bağlanıp ağaç açmak ~+9 MiB (85 vs 76); boş sekme ~+5 MiB; 10.000 satır ~+16–18 MiB.
- Izgara DOM'u: 10.000 satır yüklüyken ve sonuna kaydırınca `gridcell` sayısı 92–96, toplam DOM düğümü 369 (sanal liste).

### Paket boyutları

`bun run build` (vite), `dist/assets`:

| Parça | main (`ba40d78`, Plan 5 ön yüzü) | Plan 6 sonu |
|---|---|---|
| `index` (ana) | 191,70 kB (gz 61,61) | **228,09 kB (gz 73,83)** — +36,4 kB (gz +12,2) |
| `DdlView` (lazy) | 295,23 kB (gz 97,35) | 0,38 kB |
| `codemirror-setup` (lazy, DdlView + QueryTab ortak) | — (DdlView içindeydi) | 351,57 kB (gz 115,94) |
| `QueryTab` (lazy) | — | 78,37 kB (gz 26,30) |
| `window` (lazy, Tauri pencere API'si) | — | 14,34 kB (gz 3,49) |
| `ServerDialog` (lazy) | 5,77 kB | 8,22 kB |
| `ImportDialog` (lazy) | 5,45 kB | 5,47 kB |
| `web` (lazy) | 27,02 kB | 27,02 kB |
| CSS | 17,90 kB | 22,35 kB |

Ana paket Plan 6'da 36,4 kB büyüdü (sekme denetleyicisi, sekme çubuğu + diyaloglar, saf modeller, metinler);
CodeMirror, ızgara ve editör kodu **ana pakete girmez** (`QueryTab`/`codemirror-setup` ayrı ve tembel). Yalnızca
Query Tool ya da DDL sekmesi açılınca yüklenirler. Main (Plan 5) ölçümü: `ba40d78`'in geçici worktree'sinde `bun run build`
(Plan 3 sayısı 184 KiB `index` ≈ 188 kB ile uyumlu).

Exe / kurulum paketi:

| | Boyut |
|---|---|
| `target\release\pg-tools.exe` (`cargo build --release`, ön yüz gömülü değil) | 13,49 MiB (14.146.560 bayt) |
| `target\release\pg-tools.exe` (`bun run tauri build`, ön yüz gömülü) | 13,70 MiB (14.369.792 bayt) |
| NSIS kurulum paketi `pg-tools_0.1.0_x64-setup.exe` | **3,56 MiB** (3.737.668 bayt; hedef < 15 MB) |

Plan 3'e göre (3,24 MiB) NSIS +0,32 MiB; exe Plan 5'in 13,34 MiB'ine göre +0,15 MiB (cargo exe karşılaştırması).

## Plan 7 sonu: editör zekâsı (2026-10-06)

Plan 7 alias/CTE/alt sorgu tamamlaması, yazarken hata denetimi (sunucuda yalnızca Parse) ve SQL biçimlendiriciyi
ekler. Yeni kod `pgt-query`'dedir (`scope`, `check`, `format`, `parse_guard`; yeni bağımlılık yok); ön yüzde
lint katmanı ve biçimlendir komutu tembel yüklenen `QueryTab`/`codemirror-setup` parçalarındadır. Boşta bellek
bu yüzden Plan 6'dan anlamlı farklı olmamalıdır. Aynı makinede (bkz. Ortam), seçilen ayarlarla (B + R1), **gerçek
exe** (`bun run tauri build --no-bundle`; betikler `cargo build` exe'sini reddeder). Karar kuralı uygulanmadı.

    bun run tauri build --no-bundle
    powershell -NoProfile -File scripts/smoke-release.ps1    # BAŞARILI, çıkış kodu 0 (pencere 141 ms, 6 sn ayakta)
    powershell -NoProfile -File scripts/measure.ps1 -Runs 5

### Boşta (gerçek exe)

| Çalıştırma | Pencere (ms) | Toplam özel WS (MiB) | Toplam WS (MiB) | Rust özel WS (MiB) | Süreç |
|---|---|---|---|---|---|
| 1 | 70 | 77,5 | 358,1 | 7,4 | 7 |
| 2 | 43 | 78,6 | 359,8 | 7,3 | 7 |
| 3 | 44 | 78,8 | 360,1 | 7,4 | 7 |
| 4 | 30 | 76,2 | 357,4 | 7,4 | 7 |
| 5 | 29 | 75,9 | 356,7 | 7,4 | 7 |
| **Medyan** | **43** | **77,5** | 358,1 | **7,4** | 7 |

Hedefler karşılandı (pencere < 1 sn, toplam özel WS < 150 MB, Rust < 30 MB). Plan 6'ya göre (76,1 / 7,4 MiB) +1,4 MiB
fark ölçüm gürültüsü düzeyindedir (aynı exe türünde çalıştırmalar arası 75,9–78,8 MiB). Smoke'tan hemen sonraki ilk
5 çalıştırmalık denemede 1. çalıştırma 475,2 MiB / 17 süreç ölçtü: bu sırada Docker Desktop yeniden başlatılıyordu ve
betiğin süreç ağacı yöntemi dışarıdan gelen alt süreçleri de saymış görünüyor (süreç sayısı 7 yerine 17); bu
çalıştırma hariç medyan 77,1 MiB'dı. Temiz yeniden çalıştırma yukarıdaki tablodur.

### Yazarken denetim ve tamamlama gecikmeleri (CDP, gerçek exe)

Yöntem: Plan 6 CDP yöntemi (`docs/testing.md` "GUI'nin CDP ile sürülmesi"): yalıtılmış `APPDATA`/`LOCALAPPDATA`,
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` = `tauri.conf.json` ayarları + `--remote-debugging-port`, atılabilir PG 18.6
(Docker, 127.0.0.1, `app` şemasında 30 tablo: `orders(id,customer,total)`, `customers`, `items`, `t4…t30`), sunucu arayüzden
"Sunucu ekle" ile kaydedildi (parola kaydedilmedi, parola istemi arayüzden dolduruldu), iş sonunda arayüzden silindi
(`servers.json` boş doğrulandı). Süreler sayfa içinde `performance.now()` ile ölçüldü: son tuş vuruşunun
`beforeinput` olayından, ilgili DOM öğesinin (`.cm-tooltip-autocomplete li`, `.cm-lint-error`) görünmesini yakalayan
`MutationObserver`'a kadar; IPC süreleri `window.fetch` (`ipc.localhost/qt_*`) sarmalayıcısıyla. Tuş vuruşları
`Input.insertText` / `Input.dispatchKeyEvent` ile gönderildi. Aralıklar tek çalıştırmanın (n örnek) en küçük–en büyük değeridir.

| Ölçüm | n | Tuştan görünüre (ms) | `qt_complete` / `qt_check` IPC süresi (ms) | Spec §1.2 hedefi |
|---|---|---|---|---|
| Alias kolon tamamlaması `select o.` (`from app.orders o`), **önbellek sıcak** | 10 | 258–275 | 1,3–3,3 | < 150 ms |
| Aynısı, **soğuk** (her biri o oturumda ilk kez istenen farklı tablo) | 10 | 263–274 | 3,0–3,8 | < 400 ms |
| CTE sütunları `with x as (select …) select x.` — soğuk (gövde Parse edilir) / sıcak | 6 / 5 | 262–276 / 259–273 | 4,4–5,8 / 1,3–2,0 | (aynı) |
| Alt sorgu sütunları `from (select …) s` + `s.` — soğuk | 4 | 261–275 | 4,4–4,9 | (aynı) |
| Hata denetimi: son tuş → kırmızı işaret (`select * from no_such_table_n;`) | 12 | **613–628** | 4,5–5,4 (istek tuştan ~610–623 ms sonra çıkar) | ≤ 700 ms |
| **Son düzeltme dalgası** (`.` sonrası debounce yok, `activateOnTypingDelay` 50 ms): alias `o.` (`from app.orders o where o.`), **sıcak** | 10 | **54–69** | 1,3–1,8 (istek tuştan 51–66 ms sonra) | < 150 ms |
| Aynısı, **soğuk** (`app.t4…t13`, her biri oturumda ilk kez) | 10 | **56–70** | 3,0–3,9 | < 400 ms |
| Kısmi ad (`o.` + `c`; 150 ms debounce korunur) | 3 | 209–223 | 1,4–1,5 | — |

- **Denetim: hedef karşılandı.** 600 ms debounce'a ~20 ms eklenir; `qt_check` (Parse) 5 ms sürer. Ölçümün büyük
  kısmı debounce'tur.
- **Tamamlama (son düzeltme dalgası sonrası): hedefler karşılandı.** `.`'nın hemen ardında (üye bağlamı, kısmi ad boş)
  uygulama debounce'u kaldırıldı ve CodeMirror'ın yazarken etkinleştirme gecikmesi 50 ms'ye indirildi: tuştan öneri listesine
  sıcak 54–69 ms, soğuk 56–70 ms (yayın exe'si, aynı CDP yöntemi, 2026-10-06). Kısmi ad ve select listesi 150 ms debounce'lu
  kalır (~210 ms). CodeMirror kaynak başına aynı anda tek sorgu çalıştırdığından `.` başına en çok bir istek gider. Aşağıdaki
  madde düzeltme öncesi ölçümün kaydıdır.
- **(Düzeltme öncesi) Tamamlama: arka uç hedefi çok aşar; tuştan-görünüre ölçümü sıcak hedefi (< 150 ms) aşar.** Arka uç yanıtı 1–6 ms'dir
  (soğuk/sıcak, CTE/alt sorgu dahil), ama `qt_complete` isteği tuştan **~270 ms sonra** çıkar ve ölçüm sıcak/soğuk/CTE
  için aynıdır: Plan 6'dan kalan tamamlama yolunda 150 ms'lik uygulama debounce'u (`completion.ts` `debounceMs`) ile
  CodeMirror'ın yazarken etkinleştirme gecikmesi (varsayılan 100 ms) üst üste biner. Bu yüzden sıcak/soğuk farkı
  ölçülemez; soğuk hedef (< 400 ms) karşılanır; sıcak hedef, "tuştan öneri listesine" okunursa **karşılanmaz** (270 ms),
  isteğin yanıtı olarak okunursa (1–6 ms) karşılanır. Son düzeltme dalgasında giderildi (yukarıdaki madde; beklenen ~110 ms yerine ölçülen
  54–70 ms).
- Soğuk önbellek yalnızca ilk istekte ek ~2 ms (Parse yolunda ek ~3 ms) getirir; LAN gecikmesi ölçülmedi (127.0.0.1).

### Biçimlendirme (CDP, gerçek exe)

10.000 satırlık betikler (ifade + yorum satırı çiftleri), `qt_format` varsayılan seçeneklerle, 7 çalıştırma.

| Girdi | Bayt | Çıktı satırı | `qt_format` gidiş-dönüş (ms) |
|---|---|---|---|
| 10.000 satır, kısa satırlar (~11 B/satır) | 111.555 | — | **23,7–24,7** |
| 10.000 satır, orta (~74 B/satır) | 736.669 | 35.000 | **139,9–144,4** |
| 10.000 satır, uzun (~120 B/satır, 3 JOIN'li ifade) | 1.195.559 | 50.000 | 221,7–230,1 |

Saf Rust süresi (IPC ve sayfa olmadan): `cargo test --release -p pgt-query --test format ten_thousand -- --nocapture` (`ten_thousand_lines_are_fast`, 10.000 satır, ~58 B/satır, ifade/yorum/`DO`/emoji karışık) tek çalıştırmada **47 ms** (bütçe: release 200 ms, debug 2 s). Gidiş-dönüş farkı IPC dizge kopyalarıdır.

Gidiş-dönüş = `window.__TAURI_INTERNALS__.invoke('qt_format')` çağrısından yanıta (1 MB'lık dizgelerin IPC
serileştirmesi dahil; Rust tarafı `spawn_blocking`). Hedef < 200 ms: kısa ve orta satırlı betikte karşılanır;
satır başına 120 B'lık 10.000 satırda (1,2 MB; gerçekçi betikten büyük) aşılır (~225 ms) — sınır olarak kayda geçti.
Arayüzden (Ctrl+Shift+F, 10.000 satır / 737 KB belge, 3 deneme): tuş vuruşundan belgenin değişmesine **170–174 ms**;
bu sırada en uzun `requestAnimationFrame` aralığı 32–38 ms, `longtask` (> 50 ms) yok — arayüz donmaz. Aynı belgede
(10.000 satır, denetim açık) tek karakter yazmak sonraki kareye **9–12 ms** sürer; belgenin tek işlemle yerleştirilmesi 28 ms.

> Not: CDP `Input.insertText` çok satırlı metni **satır satır** işlediğinden büyük belge yapıştırmak belge boyutunun
> karesiyle yavaşlar (36 KB: 0,5 sn, 72 KB: 1,8 sn, 145 KB: 6,3 sn, 293 KB: 23 sn). Bu CDP'nin artefaktıdır
> (`EditorView.dispatch` ile tek işlemde 10.000 satır 28 ms); uygulama davranışı olarak okunmamalı.

### Bellek: Query Tool açık + denetim

Taze yalıtılmış profille, sunucu bağlı, tek sorgu sekmesi, denetim açık (`measure.ps1` süreç ağacı yöntemi; MiB):

| Durum | Toplam özel WS (MiB) | Rust özel WS (MiB) |
|---|---|---|
| Bağlı, sekme açık, bir sorgu çalıştırıldı, alias tamamlama + denetim kullanıldı | 95,2 | 7,6 |
| + 10.000 satırlık (737 KB) betik editörde, denetim açık, ~10 sn sonra | 113,2 | 8,3 |

Hedef (Query Tool açık + denetim < 250 MB) karşılandı. (Aynı oturumda defalarca 35.000 satırlık belgeler yüklenip
biçimlendirilip sayfada 1,2 MB'lık dizgeler tutulan önceki çalıştırmada özel WS 583,7 MiB'a çıktı; temsilî olmadığından
tabloya alınmadı, yalnızca aşırı kullanımın WebView2 yığınını büyüttüğünün kaydıdır.)

### Paket boyutları

`bun run build` (vite), `dist/assets`:

| Parça | Plan 6 sonu | Plan 7 sonu |
|---|---|---|
| `index` (ana) | 228,09 kB (gz 73,83) | **229,04 kB (gz 74,21)** — +0,95 kB (dizgeler/saf modeller; CodeMirror ana pakete girmedi) |
| `QueryTab` (lazy) | 78,37 kB (gz 26,30) | 90,59 kB (gz 30,02) |
| `codemirror-setup` (lazy) | 351,57 kB (gz 115,94) | 357,07 kB (gz 117,53) |
| CSS | 22,35 kB | 22,46 kB |

Son düzeltme dalgasından sonra (aynı makine): `index` 229,15 kB (gz 74,26), `QueryTab` 91,88 kB (gz 30,49),
`codemirror-setup` ve CSS değişmedi; exe 14.884.864 bayt (14,19 MiB).

| | Boyut |
|---|---|
| `target\release\pg-tools.exe` (`bun run tauri build --no-bundle`, ön yüz gömülü) | 14,19 MiB (14.884.352 bayt); Plan 6'ya göre +0,49 MiB (13,70 → 14,19) |

NSIS paketi bu çalıştırmada üretilmedi (`--no-bundle`); exe farkı kadar büyümesi beklenir (sıkıştırılmış < 0,5 MiB),
15 MB hedefinin çok altında.

## Açılışta beyaz kare düzeltmesi (2026-10-06)

Belirti: uygulama açılırken pencere bir an beyaz görünüyordu (koyu temada belirgin). Neden: pencere görünür
oluşturuluyordu ve WebView2, ön yüz paketi çalışıp `data-theme` uygulanana kadar varsayılan beyaz arka planı
çiziyordu (~300 ms).

Çözüm (`src-tauri/src/window.rs`):

- `tauri.conf.json` → ana pencere `"visible": false`.
- `setup`: pencere + WebView arka planı sistem temasına göre `--bg` rengine ayarlanır (açık `#fbfbfc`, koyu
  `#16181d`; birim test `styles.css` ile eşleşmeyi denetler).
- Ön yüz (`main.tsx`) temayı uygulayıp `render` ettikten hemen sonra `app_ready(dark)` komutunu çağırır: arka plan
  çözülmüş temaya (saklanan tercih sistemden farklı olabilir) ayarlanır ve pencere gösterilir. Özel komut,
  `core:window:allow-show` yetkisi ve pencere API'sinin ana pakete alınması gerekmesin diye seçildi (ana paket
  +0,12 kB).
- Yedek: ön yüz 3 sn içinde çağırmazsa Rust pencereyi yine gösterir ve logda `fallback-show` uyarısı yazar;
  `smoke-release.ps1` bu uyarıyı başarısızlık sayar.
- `styles.css`: `data-theme` yokken (betik çalışmadan) zemin `prefers-color-scheme`'i izler; `index.html`'e
  `<meta name="color-scheme" content="light dark">` eklendi. Satır içi `<style>` kullanılmadı: Tauri satır içi
  stillere nonce ekler, bu da CSP'deki `style-src 'unsafe-inline'`i geçersiz kılar.

Doğrulama (gerçek exe, `PrintWindow` ile ~25 ms aralıklı kare yakalama, açılıştan 2,5 sn): koyu temada ilk görünür
kare tam arayüzlü ve koyu, açık (>240) piksel oranı tüm karelerde %0; sistem koyu + saklanan tercih açık iken
ilk kare açık temalı; betikler kapalıyken (`--blink-settings=scriptEnabled=false`) pencere 3,35 sn'de yedekle
koyu zeminle göründü.

### Ölçüm yöntemi değişikliği

`Process.MainWindowHandle` tao'nun `Tao Thread Event Target` yardımcı penceresini (16x16, `WS_VISIBLE`) döndürür;
bu pencere süreç başlar başlamaz oluşur. Yani Plan 1–7'deki 27–43 ms "pencere" değerleri ana pencerenin değil bu
yardımcı pencerenin oluşma süresiydi (ana pencere o zamanlar ~70 ms'de görünür oluşuyor, içerik ~300 ms sonra
çiziliyordu). `measure.ps1` ve `smoke-release.ps1` artık `EnumWindows` ile `Tauri Window` sınıfındaki pencerenin
görünür olmasını bekler.

    bun run tauri build --no-bundle
    powershell -NoProfile -File scripts/smoke-release.ps1    # BAŞARILI, çıkış kodu 0 (pencere 394 ms'de göründü)
    powershell -NoProfile -File scripts/measure.ps1 -Runs 5

| Çalıştırma | Pencere görünür (ms) | Toplam özel WS (MiB) | Toplam WS (MiB) | Rust özel WS (MiB) | Süreç |
|---|---|---|---|---|---|
| 1 | 393 | 77,1 | 358,5 | 7,3 | 7 |
| 2 | 349 | 77,4 | 358,9 | 7,6 | 7 |
| 3 | 349 | 78,0 | 359,5 | 7,4 | 7 |
| 4 | 346 | 78,0 | 359,5 | 7,5 | 7 |
| 5 | 352 | 77,3 | 358,8 | 7,4 | 7 |
| **Medyan** | **349** | **77,4** | 358,9 | **7,4** | 7 |

Pencere süresi yeni anlamıyla (temalı ilk çizimle görünür) 349 ms; 1 sn hedefinin altında. Bellek Plan 7 ile aynı
(77,5 → 77,4 MiB). Paket: `index` 229,15 → 229,27 kB (gz 74,26 → 74,31), CSS 22,51 → 22,66 kB; exe 14.934.016 bayt.


## Plan 17 sonu: veri düzenleyici (elle ölçüm bekliyor)

Paket (ölçüldü, `bun run build`): ana `index-*.js` = 232.343 B (Plan 16 sonundan 462 B küçük; `t.query.history/messages/check/format/confirm` tembel dosyalara taşındı), `index.html` yalnızca solid/web/Icon önyükler, mapDeps 21 girdi. Veri sekmesi kodu ve metinleri `DataTabs-*.js` (68.672 B) ve `data-open-*.js` (1.281 B) tembel parçalarındadır; veri sekmesi açılana kadar yüklenmez, boşta bellek/açılış değişmez.

Aşağıdaki ölçümler **elle** yapılır (ajan çalıştırmaz); sonuçlar buraya işlenir. Hepsi `[—]`:

- [ ] 10.000 satırlı veri sekmesi belleği: `bun run tauri build --no-bundle`, ardından Docker PG'de 10k satırlı tabloyu bağlayıp "Verileri Görüntüle/Düzenle" ile aç, "Daha fazla" ile 10.000 satıra çık, 8 sn bekle; `powershell -NoProfile -File scripts/measure.ps1` yöntemindeki gibi toplam özel WS (MiB) oku (WebView2 alt süreçleri dahil; Plan 6 karşılaştırması: 10.000 satırlı sorgu sekmesi en çok 109,1 MiB). **[—]**
- [ ] 100k satırlı tabloda ilk açılış süresi: menü tıklamasından ilk 1000 satırın çizilmesine kadar (DevTools Performance ya da `performance.now()` ile, CDP); hedef: yüklenen sayfa 1000 satır, tabloyu tarayan sorgu yok (`LIMIT` + sıralama yoksa). **[—]**
- [ ] Kaydırma: 100k satırlı tabloda (50.000 satır sınırına kadar "Daha fazla" ile yükleyerek) fare tekerleği ve PageDown ile kaydırma akıcı; DOM'da yalnızca görünür pencere (~100 hücre) bulunur (Elements sayacı). **[—]**
- [ ] Sekme kapatınca bellek geri iner (10k satırlı sekme kapatıldıktan sonra toplam özel WS boşta değerine yaklaşır). **[—]**
