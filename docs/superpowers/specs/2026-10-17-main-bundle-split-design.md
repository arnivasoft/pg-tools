# pg-tools — Ana Paket Bölme Tasarımı

- **Tarih:** 2026-10-17
- **Durum:** Uygulandı (feat/main-bundle-split; `main`'e birleştirme kullanıcı onayı bekliyor, elle UI kontrolü bekliyor)
- **Kapsam:** Açılışta yüklenen ana JS paketini (`dist/assets/index-*.js`) küçültmek. Kobalte ve floating-ui'yi tembel parçalara taşımak. Paket bütçesini elle ölçmek yerine betikle denetlemek. Görünür davranış değişmez.
- **Gerekçe:** proje hedefi "açılış hızı, bellek" (bkz. `project-goal`). Her ön yüz planı ana pakete eklenen her baytı başka yerden telafi etmek zorunda kaldı (231,724 B sınırı); bu, Plan 21+ (yetki ekranları vb.) için de engel.
- **Teslim:** tek plan, Plan 21 (yalnız ön yüz). Yetki ekranları Plan 22 olur.

---

## 1. Ölçüm (main, ddfdc73, kaynak haritasıyla)

Ana paket 230,541 B (eşlenen bayt). En büyük katkılar:

| Kaynak | Bayt | Neden açılışta |
|---|---|---|
| `@kobalte/core` | 71,005 | context-menu (Explorer), hover-card (ErrorBadge), dialog (ConfirmDialog, PasswordPrompt, QueryTabs diyalogları), tabs (Inspector) |
| `@tanstack/virtual-core` | 23,350 | Explorer ağacı (gerekli) |
| `lucide-solid` | 10,693 | ikonlar (gerekli) |
| `@floating-ui/{dom,core,utils}` | 18,193 | Kobalte popper (menü, hover-card) |
| `@kobalte/utils`, `@corvu/utils`, `solid-prevent-scroll`, `solid-presence`, `@solid-primitives/*` | ≈ 11,800 | Kobalte bağımlılıkları |
| uygulama kodu (QueryTabs, query-controller, strings, App, explorer, inspector …) | ≈ 90,000 | kabuk |

Kobalte ve bağımlılıkları toplam ≈ **101 KB**, yani paketin %44'ü. Bunların hiçbiri ilk boyamada görünmez; yalnız Inspector sekme şeridi görünür ama o da Kobalte'siz yazılabilir.

## 2. Kararlar

| Konu | Karar |
|---|---|
| Hedef | Ana pakette **sıfır** `@kobalte/*`, `@floating-ui/*`, `@corvu/*`, `solid-prevent-scroll`, `solid-presence` modülü (kaynak haritasıyla denetlenir). Beklenen ana paket ≈ 125–135 KB. |
| Inspector sekmeleri | Yeni hafif `src/ui/TabStrip.tsx` (ana paket): WAI-ARIA tabs deseni — `role=tablist/tab/tabpanel`, `aria-selected`, `aria-controls`, gezgin tabindex, ←/→/Home/End, otomatik etkinleştirme (Kobalte varsayılanıyla aynı), `muted`/`mutedHint` desteği (mevcut `ui/Tabs` API'siyle aynı görünüm). `ui/Tabs` (Kobalte) tembel formlar için kalır. |
| Diyaloglar | `ConfirmDialog` (`createDeleteConfirm`), `PasswordPrompt`, QueryTabs'ın `CloseTxDialog`/`BulkTxDialog`/`EnvDialog`'u: açık durum/model ana pakette kalır, **diyalog bileşeni** `lazy()` ile yalnız açıkken (`<Show when={open}>` + `Suspense`) yüklenir. Model fonksiyonlarının imzaları değişmez. |
| Hata kartı | `ErrorBadge`: rozet (ikon + erişilebilir ad) ana pakette; HoverCard içeriği tembel bileşen, fallback = aynı rozet (yerleşim kayması yok). |
| Explorer menüsü | Menü içeriği tembel. Ana pakette yalnız ağacı saran bir `div` ve `onContextMenu`/klavye (ContextMenu tuşu, Shift+F10) işleyicisi kalır; tıklanan satır seçilir, işaretçi (ya da klavyede seçili satırın) konumu kaydedilir, tembel menü bileşeni bu konuma **sanal çapa** ile açılır (Kobalte `DropdownMenu`/`Menu` `getAnchorRect`). Klavye gezinmesi, odak geri dönüşü, Esc, devre dışı öğe gerekçeleri ve öğe grupları mevcutla aynı. Ağaç asla yeniden bağlanmaz (kaydırma/odak korunur). Kobalte sanal çapası beklenen gibi çalışmazsa yedek: Kobalte ContextMenu'yu tembel parçada tutup ağaç dışında görünmez bir tetikleyiciye yönlendirmek — karar uygulama görevinde ölçülerek verilir ve deftere yazılır. |
| Önceden yükleme | İlk boyamadan sonra boşta (`requestIdleCallback`, yoksa `setTimeout(…, 1500)`) menü, diyalog ve hata kartı parçaları `import()` ile ısıtılır; ilk sağ tık/diyalog gecikmesiz. Hata yutulur (sessiz; sonraki gerçek açılışta yeniden denenir). |
| Paylaşılan parça | Tembel parçaların ortak Kobalte kodu tek bir paylaşılan parçaya düşer. `__vite__mapDeps` girdi sayısı artabilir; bu kabul edilir. Kural yalnızca **`index.html` preload'ları solid/web/Icon** olarak kalır (Kobalte parçası preload edilmez). |
| Bütçe denetimi | `scripts/bundle-check.mjs` (+ `bun run bundle:check`): derlemeden sonra ana paket boyutu, `__vite__mapDeps` sayısı, `index.html` preload listesi ve (kaynak haritasıyla) ana paketteki paket/kaynak dağılımı; **yasak paketler** (`@kobalte/`, `@floating-ui/`, `@corvu/`, `solid-prevent-scroll`, `solid-presence`) ana pakette varsa hata; `bundle-budget.json`'daki tavan (bayt) aşılırsa hata. Sonraki planlar "tam N bayt" yerine bu tavana ve betiğin çıktısına bakar; tavan her planın sonunda ölçülen değere çekilir. |
| Davranış | Görünür arayüz, kısayollar, metinler, erişilebilirlik adları değişmez. Davranış testleri (vitest) mevcut modelleri korur; yeni TabStrip ve menü konum/klavye modeli için saf model testleri. |
| Kapsam dışı | strings.ts'in kalan tembel blokları (isteğe bağlı, Plan 21 son görevinde fırsat varsa), QueryTabs/query-controller'ın ertelenmesi, lucide ikon azaltma, CodeMirror parçası (zaten tembel). |

## 3. Riskler

- **Sağ tık ilk açılış gecikmesi:** önceden yükleme ile kapanır; yüklenmeden sağ tıklanırsa menü yüklenince açılır (tek seferlik, yerel diskten ms düzeyinde).
- **Kobalte sanal çapa ve odak:** menü kapanınca odak ağaca (seçili satıra) döner; mevcut davranış testle ve elle kontrol listesiyle doğrulanır.
- **Sınama:** GUI ajanlarca test edilemez; `docs/smoke-test.md`'ye "Ana paket bölme (Plan 21)" elle kontrol bölümü eklenir (sağ tık fare/klavye, hata kartı, diyaloglar, Inspector sekmeleri klavye, tema).

## 4. Sonuçlar (Plan 21 sonu)

| Ölçüm | Başlangıç | Sonuç |
| --- | --- | --- |
| Ana paket (`index-*.js`) | 231,724 B | **130,090 B** (-101,634 B, -%43,9) |
| Açılış kümesi (index + solid + web) | 255,213 B (index+solid+web+Icon) | **157,763 B** (3 parça) |
| `__vite__mapDeps` girdisi | 21 | 34 |
| `index.html` preload | solid, web, Icon | solid, web |
| Ana pakette Kobalte/floating-ui/corvu | ≈101 KB | 0 (betik denetler, `enforceForbidden: true`) |

Tavanlar (`bundle-budget.json`) bu değerlere çekildi. Gerçek açılış süresi yalnız `tauri build` exe ile ölçülür (elle).

Kararlar ve hükümler:
- **R3a:** Bütçe metriği açılış kümesidir (girişin statik içe aktarma kapanışı), yalnız giriş parçası değil.
- **R3b:** Diyalog metinleri `dialog-strings.ts` tek kaynağındadır (ana paket ile tembel parça aynı metni paylaşır).
- **R4b:** Hata kartı tembel; yüklenene dek ya da ret halinde çalışan fallback rozet, bildirim ret başına bir kez.
- **R5a:** `index.html` preload'ları ⊆ solid/web/Icon; Icon ana pakete katlandı, son durumda yalnız solid/web.
- **R5b:** Seçili düğüm yokken boş alana sağ tık eski davranışı korur (menü açmaz); seçili düğüm varken seçili düğümün menüsü işaretçide açılır.
- **Sanal çapa seçildi (yedek yok):** menü çapası konumdan türetilen sanal çapadır; ağaç asla yeniden bağlanmaz.
- **`createLoader` birleştirildi:** tek paylaşılan yükleyici (tek uçuş, ret önbelleğe alınmaz, `onFail` ret başına bir kez, `load()`'ın eşzamanlı fırlatması da ret sayılır).
- **Boşta ısıtma:** `prefetchIdle` (`requestIdleCallback ?? setTimeout 1500 ms`) beş tembel parçayı sessizce ısıtır; yükleyicileri `App.tsx` verir, `prefetch.ts` içe aktarma içermez. Maliyet: ana pakete +261 B (App.tsx içe aktarmaları + zamanlayıcı). Açılış kümesine parça eklemez (bundle:check doğruladı).
