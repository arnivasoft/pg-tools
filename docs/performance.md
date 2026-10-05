# Performans ve bellek ölçümleri

## Yöntem

Birimler: bu belgedeki tüm bellek ve dosya boyutları ikili birimdir — 1 KiB = 1024 bayt, 1 MiB = 1.048.576 bayt
(`measure.ps1` PowerShell'in `1MB` sabitiyle böler; bu da 1 MiB'dir). Spec hedefleri de aynı birimle okunur;
ondalık MB ile aradaki ~%5 fark, buradaki hiçbir sonucu değiştirmez.

    bun tauri build --no-bundle        # ya da bun tauri build (kurulum paketiyle)
    ./scripts/measure.ps1 -Runs 5      # varsayılan exe: target\release\pg-tools.exe

`scripts/measure.ps1` her çalıştırmada:

- ayrı bir geçici `APPDATA` dizini kullanır (gerçek `%APPDATA%\pg-tools` yapılandırmasına dokunmaz; sunucu
  listesi boş, hiçbir sunucuya bağlanılmaz),
- `MainWindowHandle` oluşana kadar geçen süreyi **pencere süresi** olarak ölçer (içerik çizimini kapsamaz),
- 8 sn boşta bekler, sonra uygulama sürecini ve tüm alt süreçlerini (WebView2 tarayıcı, GPU, yardımcı ve
  çizici süreçleri) bulur ve `Win32_PerfFormattedData_PerfProc_Process` sayaçlarından okur:
  - **toplam özel çalışma kümesi** (`WorkingSetPrivate` toplamı — Görev Yöneticisi "Bellek" sütununun toplamı),
  - toplam çalışma kümesi (`WorkingSet` toplamı; bilgi amaçlı),
  - Rust (`pg-tools.exe`) sürecinin özel çalışma kümesi,
- başlattığı tüm süreçleri kapatır ve geçici dizini siler.

Medyan, 5 çalıştırmanın ortadaki değeridir. İlk çalıştırmanın pencere süresi soğuk disk önbelleği yüzünden
genellikle 100+ ms'dir; sonrakiler 20–45 ms.

### Metrik neden değişti

Plan 1 "toplam bellek" olarak çalışma kümesi toplamını raporladı (~380 MiB). Çalışma kümesi, WebView2
süreçlerinin paylaştığı DLL ve bellek eşlemeli sayfaları her süreçte yeniden sayar; 7 süreçte bu ~280 MiB'lık
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

Plan 1'deki ~191 MiB "özel bellek" değeri büyük olasılıkla farklı bir sayaçtı (ör. Private Bytes; RAM'de
olmayan taahhüt edilmiş sayfaları da içerir); bu betikle ölçülen özel çalışma kümesi ayarsız hâliyle 110 MiB'dır.
Rust sürecinin Plan 1'deki 30–40 MiB değeri de çalışma kümesiydi; özel çalışma kümesi 8,5 MiB.

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

Karar kuralı (Plan 2, Task 10): seçilen ayarla toplam özel çalışma kümesi < 150 MiB ise spec hedefi aynen
kalır. 76,1 MiB < 150 MiB olduğundan hedef değerleri değişmedi; yalnızca metrik açıkça "özel çalışma kümesi"
olarak tanımlandı:

- Boşta toplam bellek (uygulama + WebView2 süreçleri, özel çalışma kümesi): **< 150 MiB** (ölçülen 76 MiB;
  GPU çizimiyle 110 MiB)
- Rust süreci (özel çalışma kümesi): **< 30 MiB** (ölçülen 7,5 MiB)

Karşılaştırma: pgAdmin4 masaüstü (Electron/NW.js kabuğu + Python sunucusu) Plan 1 notlarına göre boşta
~400 MiB+ kullanır.

## Plan 3 sonu (2026-10-05)

Plan 3 (pgAdmin içe aktarma, filtre, sağ tık menüsü, hata rozeti, sistem nesneleri anahtarı, durum çubuğu,
tembel yüklenen diyaloglar) sonrasında aynı makinede, Plan 2'de seçilen ayarlarla (B + R1) yeniden ölçüldü:

    bun run build
    cargo build --release -p pg-tools
    powershell -NoProfile -File scripts/smoke-release.ps1     # BAŞARILI, çıkış kodu 0 (pencere 192 ms, 6 sn ayakta)
    powershell -NoProfile -File scripts/measure.ps1 -Runs 5

| Çalıştırma | Pencere (ms) | Toplam özel WS (MiB) | Toplam WS (MiB) | Rust özel WS (MiB) | Süreç |
|---|---|---|---|---|---|
| 1 | 121 | 73,7 | 348,2 | 7,2 | 7 |
| 2 | 43 | 73,4 | 347,7 | 7,3 | 7 |
| 3 | 46 | 73,7 | 348,4 | 7,2 | 7 |
| 4 | 30 | 73,3 | 347,9 | 7,2 | 7 |
| 5 | 29 | 73,2 | 347,1 | 7,2 | 7 |
| **Medyan** | **43** | **73,4** | 347,9 | **7,2** | 7 |

Hedeflerin hepsi karşılandı (pencere < 1 sn, toplam özel WS < 150 MiB, Rust < 30 MiB); karar kuralının
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

NSIS paketi < 15 MiB hedefinin (spec §1.3) altında. `tauri build` exe'si de `smoke-release.ps1` ile sınandı
(BAŞARILI, pencere 186 ms).

Filtre (Ctrl+F): `tree-model.test.ts` "filters 10,000 loaded nodes in under 50 ms" testi — 10.000 yüklenmiş
tablo düğümünde filtreli düzleştirmenin 5 denemedeki en kısa süresi **0,93 ms** (3 ayrı çalıştırmada
0,94 / 0,93 / 0,93 ms; bütçe 50 ms; vitest, Node, aynı makine).
