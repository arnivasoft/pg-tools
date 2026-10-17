# Ana Paket Bölme (Plan 21) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Ana JS paketinden (`dist/assets/index-*.js`, şu an 231,724 B) Kobalte, floating-ui ve bağımlılıklarını (≈ 101 KB) çıkarmak. Paket bütçesini betikle denetlemek. Görünür davranış değişmez.

**Mimari:** Açılışta görünmeyen her Kobalte kullanımı tembel bileşene taşınır: Explorer sağ tık menüsü (sanal çapalı menü), hata kartı (HoverCard), diyaloglar. Açılışta görünen tek kullanım olan Inspector sekmeleri hafif, ARIA uyumlu `TabStrip` ile değiştirilir. Tembel parçalar ilk boyamadan sonra boşta ısıtılır. Yeni `scripts/bundle-check.mjs` kaynak haritasıyla ana paketi denetler.

**Teknoloji:** SolidJS 1.9, Kobalte 0.13, Vite 8 (rolldown), vitest 5, Tailwind 4, bun.

**Spec:** `docs/superpowers/specs/2026-10-17-main-bundle-split-design.md` (§1 ölçüm, §2 kararlar, §3 riskler).

## Global Constraints

- Yalnız ön yüz (`src/`, `scripts/`, `package.json`, `docs/`). Rust/Tauri değişmez.
- **Davranış eşdeğerliği:** metinler (`t.*`), erişilebilirlik adları, kısayollar, odak davranışı, görünüm (Tailwind sınıfları) değişmez. Mevcut vitest testleri geçer. Test silinmez, yalnızca taşınan koda göre güncellenir.
- **Paket kuralları (Plan 21 boyunca):**
  - `index.html` yalnız solid/web/Icon preload eder.
  - Her görev ana paket boyutunu ölçer ve rapora yazar. Ana paket hiçbir görevde büyümez (≤ önceki görevin değeri).
  - Task 1'den sonra `bun run bundle:check` her görevde geçer.
  - `__vite__mapDeps` sayısı artabilir; yeni değer rapora yazılır.
- **Ana pakete giren dosyalar:** `src/app/*` (tembel `strings-*.ts` hariç), `src/features/explorer/*`, `query/{QueryTabs,tabs-store,query-controller,workspace-model,…}`, `src/ipc/commands.ts`, `designer/ddl-kinds.ts`, `src/ui/{Button,Field,…}`. Yeni tembel dosyalar yalnız `lazy()`/`import()` ile içe aktarılır. Statik içe aktarma parçayı ana pakete geri çeker; her görevde `bundle:check` bunu yakalar.
- **vitest:** ortam `node`. Solid bileşen testleri gerekirse `vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));` kalıbıyla. Mantık saf modellere ayrılır (`*-model.ts`) ve orada test edilir. GUI ajanlarca test edilemez; elle kontrol maddeleri `docs/smoke-test.md`'ye `[—]` olarak eklenir.
- **Yükleyici sözleşmesi (T3–T5):** her tembel görünüm için ana pakette tek bir yükleyici dışa aktarılır (`export const loadX = () => import("./X")`); `lazy(loadX)` ve T6 `prefetchIdle` aynı fonksiyonu kullanır.
- IME koruması (`ev.isComposing || ev.keyCode === 229`) klavye işleyicilerinde korunur.
- Yorumlar Türkçe, çevredeki yoğunlukta. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Dal: `feat/main-bundle-split` (main'den). `src/ipc/bindings` satır sonu gürültüsü `git checkout -- src/ipc/bindings`.
- Doğrulama komutları: `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`.

## Review Focus

- **Sağ tık menüsü eşdeğerliği:**
  - Fareyle satıra sağ tık: satır seçilir ve menü işaretçide açılır.
  - Boş alana sağ tık: menü açılmaz ve tarayıcı menüsü de çıkmaz (mevcut `preventDefault`).
  - ContextMenu tuşu / Shift+F10: menü seçili satırın yanında açılır.
  - Ok tuşları/Enter/Esc çalışır; kapanınca odak ağaçtaki seçili satıra döner.
  - Devre dışı öğeler ve gerekçe metinleri, öğe grupları ve ayırıcılar aynı kalır.
  - Menü açıkken ikinci bir sağ tık menüyü yeni konumda yeniden açar.
- **İlk etkileşim yarışı:** parça yüklenmeden sağ tık ya da diyalog açılışı olay kaybetmez. Yükleme bitince menü/diyalog açılır. Yükleme hatası sessizce yutulmaz: toast + sonraki denemede yeniden yükleme.
- **Inspector TabStrip erişilebilirliği:**
  - Roller ve `aria-selected`/`aria-controls` doğru.
  - Klavyede ←/→ kaydırır ve uçlarda sarar; Home/End çalışır.
  - Seçili olmayan sekme `tabindex=-1` alır.
  - `muted` sekmenin `sr-only` gerekçesi korunur.
  - Panel `hidden` olur (bağlanmış DdlView durumu korunur mu: mevcut Kobalte `Tabs.Content` davranışıyla aynı — bağlı tutma/kaldırma rapora).
- **Diyaloglar:** `createDeleteConfirm` ve `createPasswordPrompt` sözleşmeleri (Promise dönüşleri, iptal/Esc → aynı sonuç) değişmez. Ardışık iki istek sırası korunur. Ana pakette `Dialog` statik içe aktarılmaz.
- **Bütçe betiği güvenilirliği:**
  - Kaynak haritası yoksa ya da eşleşmezse betik açık hatayla düşer (yanlış "temiz" yok).
  - Windows yol ayırıcıları doğru işlenir.
  - Derleme çıktısı geçici dizine yazılır ve `dist/` bozulmaz (ya da derleme `dist/`'e yapılıp harita silinir: karar Task 1'de, rapora).

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `scripts/bundle-check.mjs`, `scripts/bundle-budget.json`, `package.json` (`bundle:check`) | Bütçe denetimi |
| `src/ui/TabStrip.tsx`, `src/ui/tabstrip-model.ts` (+ test) | Hafif ARIA sekmeleri (ana paket) |
| `src/features/inspector/Inspector.tsx` | TabStrip'e geçiş |
| `src/ui/ConfirmDialog.tsx` → model ana pakette, görünüm `src/ui/ConfirmDialogView.tsx` (tembel) | Silme onayı |
| `src/features/servers/PasswordPrompt.tsx` → model + `PasswordPromptView.tsx` (tembel) | Parola istemi |
| `src/features/query/QueryTabs.tsx` → `query/tx-dialogs.tsx` (tembel; CloseTx/BulkTx/Env) | Sorgu diyalogları |
| `src/features/explorer/ErrorBadge.tsx` + `ErrorBadgeCard.tsx` (tembel) | Hata kartı |
| `src/features/explorer/ExplorerMenu.tsx` (ana; sarıcı + olaylar) + `ExplorerMenuPopup.tsx` (tembel; menü) + `menu-anchor-model.ts` (+ test) | Sağ tık menüsü |
| `src/app/prefetch.ts` (+ test) | Boşta ısıtma |
| `docs/smoke-test.md`, spec, `docs/testing.md` | Notlar |

---

### Task 1: Bütçe denetim betiği

**Dosyalar:** Oluştur `scripts/bundle-check.mjs`, `scripts/bundle-budget.json`; değiştir `package.json` (`"bundle:check": "node scripts/bundle-check.mjs"`), `docs/testing.md`.

**Arayüzler — Üretir:**
- `bundle-budget.json`:
  ```json
  { "mainMaxBytes": 231724, "forbiddenInMain": ["@kobalte/", "@floating-ui/", "@corvu/", "solid-prevent-scroll", "solid-presence"], "enforceForbidden": false, "htmlPreloads": ["solid", "web", "Icon"] }
  ```
  `enforceForbidden` Task 5'te `true` olur.
- `bundle-check.mjs` adımları:
  1. Geçici dizine `vite build --sourcemap --outDir <tmp>` çalıştırır (`dist/`'e dokunmaz).
  2. `index.html`'den giriş parçasını bulur.
  3. Haritadaki `//# sourceMappingURL` yorumunu boyuttan düşerek ana paket baytını ölçer. Sonuç `bun run build` çıktısındaki ana dosya boyutuyla aynı olmalı; bunu birim testinde ya da raporda doğrulayın.
  4. `__vite__mapDeps` dizisinin girdi sayısını sayar.
  5. `index.html` `modulepreload` adlarını önek eşleşmesiyle kontrol eder.
  6. Kaynak haritasını VLQ çözerek kaynak ve paket başına bayt tablosu basar (ilk 25). Yollar Windows `\` ve `/` ile normalize edilir.
  7. Yasak paketleri listeler.
  8. Çıkış kodu: tavan aşımı, `enforceForbidden && yasak > 0`, preload ihlali, harita yok ya da eşlenen toplam < ana bayt × 0.9 → `1`.
  - VLQ çözücü ve gruplama saf fonksiyonlardır, `scripts/bundle-check-lib.mjs`'te tutulur. Betik dosyası çalıştırıcıdır.
- Testler: `scripts/bundle-check-lib.test.mjs` (vitest `include`'a eklenir ya da `src/` altına taşınmadan çalışacak biçimde yapılandırılır). Kapsam: VLQ çözümü (bilinen örnek), Windows yolu → paket adı (`node_modules\@kobalte\core\dist\…` → `@kobalte/core`), mapDeps sayımı, preload ayrıştırma.

- [ ] **Adım 1:** Başarısız birim testleri. **Adım 2–3:** uygula. **Adım 4:** `bun run bundle:check` → geçer, çıktıda ana 231,724 B, mapDeps 21, preload solid/web/Icon, Kobalte ≈ 71 KB görünür (rapora tablo). `bun run test`, `bun run typecheck`.
- [ ] **Adım 5: Commit** (`build: ana paket bütçe denetimi (bundle:check)`).

---

### Task 2: Inspector için hafif TabStrip

**Dosyalar:** Oluştur `src/ui/TabStrip.tsx`, `src/ui/tabstrip-model.ts`, `src/ui/tabstrip-model.test.ts`; değiştir `src/features/inspector/Inspector.tsx`.

**Arayüzler — Üretir:**

```ts
// tabstrip-model.ts (saf)
export type TabKey = "ArrowLeft" | "ArrowRight" | "Home" | "End";
export function nextTabIndex(current: number, count: number, key: TabKey): number;   // sarar
export function tabId(base: string, value: string): string;      // `${base}-tab-${value}`
export function panelId(base: string, value: string): string;    // `${base}-panel-${value}`
// TabStrip.tsx — ui/Tabs ile aynı props + panel bileşeni
export function TabStrip(props: { value: string; onChange: (v: string) => void; tabs: { value: string; label: string; muted?: boolean; mutedHint?: string }[]; children: JSX.Element; class?: string }): JSX.Element;
export function TabStripPanel(props: { value: string; class?: string; children: JSX.Element }): JSX.Element;   // bağlamdan seçili değeri okur
```
- Görünüm `ui/Tabs` ile birebir aynı sınıflar. `data-[selected]` yerine `aria-selected` + `classList`; seçili sekmede `border-accent text-fg`.
- Panel davranışı: Kobalte `Tabs.Content` varsayılanı (`forceMount` yok) seçili olmayan paneli DOM'dan **kaldırır**. Inspector bunu kullanıyorsa aynısı yapılır (`<Show>`). Değilse mevcut davranış korunur. Mevcut davranışı kodla doğrulayın ve rapora yazın.
- Klavye: odak sekmedeyken ←/→/Home/End `nextTabIndex` ile seçer ve odağı taşır (otomatik etkinleştirme). IME koruması.

- [ ] **Adım 1:** Başarısız model testleri: sarma, Home/End, tek sekme, id'ler. **Adım 2–3:** uygula, Inspector geçişi. **Adım 4:** typecheck, test, build, `bundle:check` → ana paket küçülmüş olabilir ya da Kobalte tabs başka kullanımlarla kalmış olabilir (rapora); ana paket ≤ 231,724 B.
- [ ] **Adım 5:** `docs/smoke-test.md` maddeleri (Inspector sekmeleri fare/klavye, ekran okuyucu adı). **Commit** (`refactor(ui): Inspector için Kobalte'siz TabStrip`).

---

### Task 3: Diyalogları tembel yükleme

**Dosyalar:** Değiştir `src/ui/ConfirmDialog.tsx` (model), oluştur `src/ui/ConfirmDialogView.tsx`; değiştir `src/features/servers/PasswordPrompt.tsx`, oluştur `PasswordPromptView.tsx`; değiştir `src/features/query/QueryTabs.tsx`, oluştur `src/features/query/tx-dialogs.tsx`; değiştir `src/app/App.tsx` (yalnız gerekirse).

- **Sözleşme korunur:** `createDeleteConfirm()` ve `createPasswordPrompt()` aynı dönüş değerini verir (Promise tabanlı istek, `view`/bileşen). Bileşen öğesi artık `lazy()` görünümü `<Show when={open()}>` içinde `Suspense` ile render eder. Fallback `null` olur; parça yüklenene dek istek bekler, kaybolmaz.
- QueryTabs: `CloseTxDialog`, `BulkTxDialog`, `EnvDialog` `tx-dialogs.tsx`'e taşınır. QueryTabs bunları `lazy()` ile ve yalnız ilgili `ctl` durumu açıkken render eder. Ana pakette `ui/Dialog` statik içe aktarılmaz; bunu `grep` ile doğrulayın.
- Yükleme hatası (`import()` reddi): toast (`t.app.chunkLoadFailed` gibi yeni kısa metin, `strings.ts`'e; telafi gerekmez, paket küçülüyor). İstek iptal sonucu döner (ör. onay → `false`, parola → iptal).

- [ ] **Adım 1:** Başarısız testler: diyalog modelleri (mevcut testler varsa genişlet). (a) Açık durumda görünüm yüklenmeden iptal → doğru sonuç. (b) Ardışık iki istek sırası. (c) Yükleme hatası → iptal sonucu + toast çağrısı (bağımlılık enjeksiyonuyla).
- [ ] **Adım 2–3:** uygula. **Adım 4:** typecheck, test, build, `bundle:check` → ana paket küçüldü (rapora). `rg "ui/Dialog" src/app src/features/explorer src/features/query/QueryTabs.tsx src/ui/ConfirmDialog.tsx src/features/servers/PasswordPrompt.tsx` → yalnız `import type` ya da hiç.
- [ ] **Adım 5:** smoke maddeleri (silme onayı, parola istemi, işlem kapatma/toplu/ortam diyalogları; Esc/odak). **Commit** (`perf(ui): diyaloglar tembel yüklenir`).

---

### Task 4: Hata kartı tembel

**Dosyalar:** Değiştir `src/features/explorer/ErrorBadge.tsx`; oluştur `ErrorBadgeCard.tsx`.

- `ErrorBadge` ana pakette rozeti (ikon, `aria-label`/`title`, mevcut sınıflar) render eder. HoverCard'lı tam bileşen `lazy()` olur ve fallback birebir aynı rozettir; kart içeriği yüklenince aynı rozet HoverCard tetikleyicisi olur. İçerik ve metinler aynı kalır.
- Klavye: odaklanınca kart açılıyorsa (mevcut davranış) korunur.

- [ ] **Adım 1:** Saf mantık varsa (ileti biçimleme) test. Yoksa bileşen ayrımını belgeleyen küçük test (fallback ile tam rozet aynı `aria-label`'ı üretir: ortak fonksiyon `badgeLabel(err)`).
- [ ] **Adım 2–4:** uygula, doğrulamalar, `bundle:check`. **Adım 5:** smoke maddesi (hatalı düğüm rozeti: fareyle üzerine gelme/odak → kart). **Commit** (`perf(explorer): hata kartı tembel yüklenir`).

---

### Task 5: Explorer sağ tık menüsü tembel (sanal çapa)

**Dosyalar:** Değiştir `src/features/explorer/ExplorerMenu.tsx`; oluştur `ExplorerMenuPopup.tsx`, `menu-anchor-model.ts`, `menu-anchor-model.test.ts`; değiştir `scripts/bundle-budget.json` (`enforceForbidden: true`, `mainMaxBytes` = ölçülen değer).

**Arayüzler — Üretir:**

```ts
// menu-anchor-model.ts (saf)
export type MenuAnchor = { x: number; y: number; nodeId: string; source: "pointer" | "keyboard" };
/** Olaydan çapa: fare → istemci koordinatı; klavye (ContextMenu tuşu / Shift+F10) → satır öğesinin sol-alt köşesi. Satır yok + seçili yok → null (menü açılmaz). */
export function anchorFromEvent(e: { clientX: number; clientY: number; button?: number; detail?: number }, rowId: string | null, selected: string | null, rowRect: (id: string) => DOMRect | null, keyboard: boolean): MenuAnchor | null;
```
- `ExplorerMenu` (ana):
  - Ağacı saran `div` mevcut sınıflarla render edilir. `onContextMenu` her zaman `preventDefault` yapar, çünkü tarayıcı menüsü hiçbir zaman çıkmamalı (mevcut Kobalte davranışı).
  - Çapa `anchorFromEvent` ile alınır. `null` ise hiçbir şey yapılmaz.
  - Çapa varsa `controller.select(id)` çağrılır ve `setAnchor(anchor)` ayarlanır.
  - `onKeyDown`: ContextMenu tuşu ve Shift+F10 klavye çapası verir. Fare `contextmenu` olayını ayırmak için Kobalte'nin kullandığı ölçütü inceleyin. IME koruması uygulanır.
  - `Show when={anchor()}` içinde `lazy(ExplorerMenuPopup)` render edilir; menü `anchor`, `items`, `onClose` alır.
  - Kapanınca `setAnchor(null)` olur ve odak `treeEl` içindeki seçili satıra döner (mevcut `onCloseAutoFocus` davranışını taşıyın).
- `ExplorerMenuPopup` (tembel):
  - Kobalte `DropdownMenu` (ya da `Menu`) kontrollü `open = true` ve `getAnchorRect = () => ({ x, y, width: 0, height: 0 })` ile kurulur. Öğe listesi, gruplar, ayırıcılar, devre dışı gerekçeleri ve `executeAction` çağrıları mevcut `ExplorerMenu.tsx`'ten birebir taşınır.
  - Menü açıkken yeni bir sağ tık çapayı günceller ve menü yeni konuma taşınır ya da kapanıp yeniden açılır. Seçilen davranışı rapora yazın.
  - **Yedek** (sanal çapa odak/konum sorunları çıkarırsa): Kobalte `ContextMenu` tembel parçada, ağacın üstüne yerleştirilmiş tam boyutlu görünmez bir tetikleyiciyle kullanılır ve olay yeniden gönderilir. Bu bir **karar noktasıdır**: ölçüm ve gerekçeyle rapora yazın. Ağaç asla yeniden bağlanmaz.
- `bundle-budget.json`: `enforceForbidden: true`. `mainMaxBytes` ölçülen ana paket baytı olur; tavan tam bu değerdir.

- [ ] **Adım 1:** Başarısız testler (`menu-anchor-model`):
  - fare satırda → satır çapası;
  - fare boş alanda + seçili var → seçili düğüm, fare koordinatı (mevcut: "satır yoksa seçili düğüme düş");
  - hiçbiri yoksa → `null`;
  - klavye → satır dikdörtgeni;
  - satır dikdörtgeni yok (sanallaştırma dışında) → ağaç kapsayıcısının sol üstü (ya da `null`; kararı rapora yazın).
- [ ] **Adım 2–3:** uygula. **Adım 4:** typecheck, test, build, `bundle:check` (artık yasak paket 0 olmalı) → ana paket baytı ve kaynak dağılımı rapora.
- [ ] **Adım 5:** smoke maddeleri (Review Focus'taki sağ tık listesinin tamamı). **Commit** (`perf(explorer): sağ tık menüsü tembel yüklenir (sanal çapa)`).

---

### Task 6: Boşta ısıtma, belgeler ve bütünleşik doğrulama

**Dosyalar:** Oluştur `src/app/prefetch.ts` (+ test); değiştir `src/app/App.tsx` (ilk boyamadan sonra `prefetchIdle()`); değiştir `docs/smoke-test.md`, spec (durum + ölçüm sonuçları), `docs/testing.md` (`bundle:check` kullanımı; sonraki planlar için kural: "ana paket ≤ `bundle-budget.json` tavanı; tavan plan sonunda ölçülen değere çekilir").

**Arayüzler — Üretir:**

```ts
export function prefetchIdle(loaders: (() => Promise<unknown>)[], schedule?: (cb: () => void) => void): void;   // varsayılan: requestIdleCallback ?? setTimeout(cb, 1500); hatalar yutulur (yalnız ısıtma)
```
- Isıtılacaklar: `ExplorerMenuPopup`, `ConfirmDialogView`, `PasswordPromptView`, `tx-dialogs`, `ErrorBadgeCard`. Bunlar aynı `lazy()` yükleyici fonksiyonlarıdır; modül önbelleği sayesinde ikinci `import()` aynı sözü döner. Yükleyiciler ilgili modüllerden dışa aktarılır, böylece ikinci bir dinamik içe aktarma yolu oluşmaz.
- [ ] **Adım 1:** Başarısız test: `schedule` enjekte edilir. Yükleyiciler zamanlayıcı çalışınca çağrılır. Reddeden yükleyici diğerlerini engellemez. Test çıktısı temiz kalır, yakalanmamış ret uyarısı olmaz.
- [ ] **Adım 2–3:** uygula. **Adım 4: Bütünleşik doğrulama:**
  - `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`.
  - Ana paket baytı, mapDeps ve `index.html` preload'ları rapora ve spec'e yazılır.
  - `cargo test --workspace` gerekmez (Rust değişmedi); `cargo build -p pg-tools` derleme uyumu için bir kez çalıştırılır.
- [ ] **Adım 5:** Kapanış taramaları:
  - `rg "@kobalte/core" src/app src/features/explorer src/features/inspector src/features/query/QueryTabs.tsx src/ui/ConfirmDialog.tsx src/features/servers/PasswordPrompt.tsx` → yalnız tembel dosyalarda.
  - `rg "import\(" src/app/prefetch.ts` → yok (yükleyiciler dışarıdan gelir).
- [ ] **Adım 6: Commit** (`perf(app): tembel parçalar boşta ısıtılır; Plan 21 belgeleri ve ölçümler`).

---

## Kapanış kontrolü (Plan 21 bitişi)

- Ana pakette sıfır Kobalte/floating-ui/corvu (betik denetler). Ana paket baytı spec'te kayıtlı; `bundle-budget.json` tavanı bu değere çekildi.
- Davranış eşdeğerliği: vitest yeşil; elle kontrol maddeleri `docs/smoke-test.md` "Ana paket bölme (Plan 21)" bölümünde.
- Gerçek açılış süresi yalnız `tauri build` exe ile ölçülür (kullanıcının elle kontrolü; proje kuralı).
- `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme

- **Spec kapsamı:**
  - §2 Hedef → T5 (`enforceForbidden`).
  - Inspector → T2.
  - Diyaloglar → T3.
  - Hata kartı → T4.
  - Menü + yedek → T5.
  - Önceden yükleme → T6.
  - Paylaşılan parça/mapDeps kuralı → Global Constraints.
  - Bütçe denetimi → T1 (+ T5 tavanı, T6 belgeler).
  - Davranış → Global Constraints + smoke.
  - §3 riskler → Review Focus + T3/T5 hata yolu.
- **Tip tutarlılığı:**
  - `TabStrip` props `ui/Tabs` ile aynı (T2).
  - `MenuAnchor`/`anchorFromEvent` (T5) yalnız `ExplorerMenu` ↔ `ExplorerMenuPopup` arasında kullanılır.
  - `prefetchIdle` (T6), T3–T5'in dışa aktardığı yükleyicileri kullanır. T3–T5 yükleyici fonksiyonlarını dışa aktarır (`loadConfirmDialogView` vb.) ve T6 bunlara güvenir; bu, her görevin Arayüz notudur.
- **Yer tutucu yok:** T1'deki `enforceForbidden: false` T5'te açıkça `true` yapılır.
