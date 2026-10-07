# Veri Düzenleyici — Ön Yüz (Plan 17) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Plan 16 arka ucunun üstüne veri düzenleyicinin **arayüzü**: Explorer'da table/view/materializedView düğümünden **"Verileri Görüntüle/Düzenle"**; yeni sekme türü `data` (nesne başına tek sekme); kendi **salt okunur Query Tool oturumundan** akan satırlar; hücre düzenleme (satır içi), NULL/DEFAULT, satır ekleme, silme işareti; değişikliklerin `data_preview` ile **kaydetmeden önce gösterilmesi**; kapılı `data_apply` (onay kutusu, üretimde onay, iptal); çakışma / `outcomeUnknown` / `plan_changed` / kilit durumları; sıralama ve filtre; kaydedilmemiş değişiklikle kapatma/yeniden yükleme onayı. Rust değişikliği **yalnızca Task 2** (okuma oturumunun açıkça salt okunur açılabilmesi); bunun dışında sözleşme boşluğu bulunursa düzeltilmez, raporlanır.

**Mimari:** Plan 11/13/15 kalıbı: istemci SQL üretmez (SQL'i `data_select` verir; değişikliği `DataChangeSet` olarak `data_preview/apply`'a göndeririz), tüm mantık saf modüllerde (`data-edit-model.ts`, `data-select-model.ts`, `data-apply-model.ts`, `data-errors.ts`, `data-grid-model.ts`), denetleyici ince ve test edilebilir (sahte `Api`), bileşenler ince. Düzenleme katmanı **yalnızca değişen hücreleri** tutar (satır başına kopya yok; `StmtResult` değiştirilmez). Okuma: sekmeye özel `qt_open_readonly` oturumu + `qt_run`/`qt_more`; yazma: `data_*` komutları. Ana pakete yalnızca sekme türü, eylem kimliği, menü metni ve tembel yükleyici girer (Task 1) ve **aynı görevde adı konmuş azaltmayla** telafi edilir.

**Teknoloji:** SolidJS, Tailwind 4, TanStack Virtual, vitest (env `node`), mevcut Query Tool oturumu, tembel parçalar.

**Spec:** `docs/superpowers/specs/2026-10-07-data-editor-design.md` (§1.1 kararlar, §3 arayüz, §7 **Plan 16 sözleşme notları — bağlayıcı**: okuma oturumu, `ONLY`/kalıtım, değişiklik anahtarı/sürüm, uygulama sonucu, önizleme (bidi/denetim karakterleri görünür, uzun değer kısaltılır), hata kodları ve alan yolları, `outcomeUnknown`, `row_conflict` anlamı). Önceki arayüz planları: `2026-10-07-object-dialogs-p15-view-database-role-on-yuz.md`, `2026-10-07-object-dialogs-p13-tablo-on-yuz.md` — burada tekrarlanmayan tüm Global Constraints ve Review Focus maddeleri geçerlidir.

## Kararlar

1. **Salt okunur okuma oturumu (Rust, Task 2):** `SessionOptions` değişmez (çok sayıda Rust/TS çağıranı bozar). Yeni `QueryManager::open_read_only(server, target, opts)` ve Tauri komutu `qt_open_readonly(serverId, databaseOid, options)`; oturum yuvası `force_read_only` taşır ve **yeniden açılışta/`set_options`'ta korunur** (`slot.profile()` → `read_only = guard.locked || force_read_only`). TS sarmalayıcısı `commands.ts`'e değil tembel `data-commands.ts`'e girer.
2. **Kendi ızgara bileşeni (`DataGrid.tsx`):** `ResultGrid`/`grid-model` QueryTab parçasının içindedir; veri parçası onları içe aktarırsa paylaşılan yeni parça doğar ve ana paketin `__vite__mapDeps` listesi büyür (Plan 13/15 dersi). Dashboard `DataTable.tsx` emsali: sanallaştırıcı kalıbı kopyalanır, küçük saf yardımcılar (`displayCell`, seçim, sütun genişliği) `data-grid-model.ts`'e **kopyalanır** (kopya kabul edilir; ortak modüle çıkarma backlog). `designer/*` modülleri (`designer-model`, `RiskBadge`, `strings-designer`, `preview.ts`, `errors.ts`) de veri parçasından **içe aktarılmaz** (aynı gerekçe): ihtiyaç duyulan küçük kapı/rozet/önizleyici mantığı veri modüllerinde yeniden yazılır.
3. **Satır kimliği = yüklenen dizideki indeks:** satırlar yalnız eklenir (sayfalama); düzenleme durumu `rowIndex` anahtarlıdır. Yeniden yükleme (sıralama/filtre/F5/çakışma sonrası) düzenleme durumunu **sıfırlar** (kirli ise onay ister). Başarılı uygulamadan sonra satırlar `returned` değerleriyle yerinde tazelenir (UPDATE: satır değerleri + yeni sürüm; INSERT: yeni satır eklenir; DELETE: satır diziden çıkarılır) ve düzenleme durumu sıfırlanır.
4. **Değişiklik üretimi (`toChangeSet`):** Update `set` yalnızca **özgün değerden farklı** hücreleri içerir (düzenleyip eski değere döndürülen hücre kaldırılır); Update/Delete `key` = **özgün** satırdaki `DataKey.columns` değerleri (sırayla), `version` = özgün sürüm kolonu; Insert `values` yalnızca kullanıcının dokunduğu kolonlar (diğerleri sunucu varsayılanı); NULL ≠ `''` ≠ `'NULL'`; silinmiş satır düzenlenemez; yeni satır silinirse listeden çıkar (Delete üretilmez); yeni satırda `generated` kolonlar kilitli, `identity = always` yalnızca `Default`.
5. **Kapı (`data-apply-model.ts`):** `empty` → kapalı; `invalid` (PlanView.errors) → kapalı; kilitli sunucu → kapalı; bekleyen önizleme/yükleme → kapalı; `outcomeUnknown` sonrası yeniden yükleme yapılana dek kapalı; risk ≥ Destructive (DELETE) → onay kutusu; **üretim sunucusunda boş olmayan her plan** onay kutusu ister (ad yazma yok; Plan 16 Karar 3). Seçenekler: `{ applyId, lockTimeoutMs: 5000, expectedHash: view.hash, confirmed }`.
6. **Çakışma anlamı ifade türüne göre:** UPDATE/DELETE → "satır başka oturumca değiştirilmiş/silinmiş (ya da tetikleyici/RLS)"; INSERT → "satır bir tetikleyici/kural/RLS tarafından düşürüldü" (spec §7). `failedIndex` → değişiklik indeksi → satır vurgusu. Çakışma sonrası otomatik yeniden deneme **yoktur**; "Yeniden yükle" (düzenlemeleri atar, onayla) sunulur.
7. **Okuma sınırları:** ilk yükleme 1000 satır (`qt_run` limit), "Daha fazla" ile `qt_more` (1000'lik); sekme başına en çok **50 000** yüklü satır (sonra düğme kapanır, ipucu: filtre/sıralama kullanın). İmleç 5 dk boşta kapanır (Query Tool kuralı) → `qt_more` hatası "Yeniden yükle" ipucu verir. Hücre gösterimi 1000 karakterde kırpılır; düzenleme yalnızca tam değeri olan hücrede (kırpılmış gösterim düzenleme açıldığında tam değeri yükler — değer zaten bellektedir).
8. **Kısayollar (kayıt defteri yok, ızgara içi):** `Enter`/`F2`/çift tık düzenle; düzenlerken `Enter` onayla + aşağı, `Tab` onayla + sağa, `Esc` vazgeç; `Delete` satırı silinecek işaretle/geri al; `Insert` yeni satır; `Alt+N` hücreyi NULL, `Alt+D` hücreyi DEFAULT; `Ctrl+Z` etkin hücrenin düzenlemesini geri al; `Ctrl+C` etkin hücre değerini kopyala; `F5` yeniden yükle. (AltGr = Ctrl+Alt olduğundan Ctrl+Alt kombinasyonları kullanılmaz.) Aynı komutlar araç çubuğunda düğmedir.
9. **Önizleme gösterimi:** her ifade en çok **4000 karakter** gösterilir (`…(+N karakter)` ile kısaltılır; tam metin "Kopyala"da); bidi biçimlendirme/denetim karakterleri (`U+202A–202E`, `U+2066–2069`, `U+200B–200F`, `U+061C`, C0/C1 denetimleri) `⟦U+XXXX⟧` olarak **görünür** çizilir; hücre gösterimi (ızgara) bidi geçersiz kılmalarını da aynı kaçışla gösterir.
10. **Kapsam dışı (backlog):** ortak `ResultGrid` ile kod paylaşımı, kolon yeniden sıralama/gizleme, çoklu hücre yapıştırma, büyük nesne (bytea/text > 1 MiB) düzenleyici, kopyala-yapıştır satır, CSV içe aktarma, kullanıcı seçimli anahtar.

## Global Constraints

- **Paket bütçesi (bağlayıcı):** ana `index-*.js` bu planın başında **tam 232,805 B**'dir (Plan 16 tavanı; pay yok). Plan 17 **yalnızca tembel**dir; tek istisna Task 1'in ana paket kancaları (`tabs-store.ts`, `QueryTabs.tsx`, `App.tsx`, `explorer/actions.ts`, `src/app/strings.ts` menü metni, bağımlılıksız `features/data/open-kinds.ts`) — **aynı görevde** şu adı konmuş azaltmayla telafi edilir: `t.query.history` metin bloğu (`src/app/strings.ts`; yalnızca `HistoryView.tsx` kullanır, bkz. adım) yeni tembel `src/features/query/strings-history.ts`'e (`thist`) taşınır; bu yetmezse sıradaki adaylar (yalnızca QueryTab parçası kullanıyorsa): `t.query.messages`, `t.query.check`, `t.query.format`, `t.query.confirm` — uygulayıcı her aday için "yalnızca tembel dosya kullanıyor" doğrulamasını `rg` ile yapar ve seçimi ledger'a yazar. **Kabul ölçütü:** Task 1 sonunda ve sonraki **her** UI görevinde ana `index-*.js` ≤ **232,805 B** (Task 1'de ölçülen değer sonraki görevlerin tavanıdır); `index.html` yalnızca solid/web/Icon parçalarını önyükler; ana `__vite__mapDeps` listesine **yalnızca veri parçasının yükleyicisi için gereken girdi** eklenebilir (Task 1'de ölçülüp ledger'a yazılır) — sonraki görevlerde **sabit** kalır.
- **Boyut doğrulaması (her UI görevinin son adımı):**
  ```powershell
  bun run build
  (Get-Item dist/assets/index-*.js).Length                       # ≤ Task 1 değeri (≤ 232,805)
  Select-String -Path dist/index.html -Pattern 'modulepreload'   # yalnızca solid / web / Icon
  Select-String -Path dist/assets/index-*.js -SimpleMatch "Verileri Görüntüle","Değişiklik paneli","Satırı sil","Yeniden yükle (düzenlemeler atılır)"   # YALNIZCA menü metni "Verileri Görüntüle" izinli; diğerleri BOŞ
  Get-ChildItem dist/assets -Filter "*Data*-*.js" | Select-Object Name, Length
  ```
  (Uygulayıcı `strings-data.ts`'e gerçekten taşınan, başka yerde geçmeyen dizgeleri seçip kontrolü gerçek metinlerle günceller ve ledger'a yazar.)
- **Tembel kural:** ana paket modülleri (`src/app/*` — `strings-*.ts` hariç —, `src/features/explorer/*`, `src/features/query/{QueryTabs,tabs-store,query-controller}.ts(x)`, `src/ipc/commands.ts`) veri modüllerini **statik içe aktarmaz** (tek yol `App.tsx`'in `once(() => import("../features/data/data-open"))` yükleyicisi ve `QueryTabs.tsx`'in `lazy(() => import(...))` çağrıları). Veri parçası `designer/*`, `query/ResultGrid`, `query/grid-model`, `query/result-model`'i **içe aktarmaz** (Karar 2; yalnızca `import type` serbest). Tembel dosyalarda Kobalte yok; lucide simgelerinden yalnızca ana pakette zaten olanlar (`Icon` parçası) — emin değilsen simgesiz düğme/metin kullan; `createPreviewer` içe aktarılmaz.
- **İstemci SQL üretmez:** SQL yalnızca `data_select` çıktısıdır; filtre metni kullanıcıdan gelir ve olduğu gibi gönderilir (doğrulama arka uçtadır); hücre değerleri `CellValue` olarak gider.
- **Gizlilik:** hücre değerleri, anahtarlar, sürümler, filtre metni ve SQL `console.*`'a, `localStorage`/`sessionStorage`'a, toast/hata metnine (PG'nin kendi iletisi yalnız hata şeridinde gösterilir, günlüğe gitmez), sekme kaydına ve geçmişe **girmez**; düzenleme durumu yalnızca bellekte (denetleyici). Test fikstürlerinde gerçek veri yoktur.
- **Sözleşme (Plan 16 §7):** `databaseOid` `null` ise açma reddedilir (`tdata.open.none`); `DataSelect.versionIndex` sürüm kolonudur (son), `hasVersion=false` (view/MV) ise düzenleme zaten kapalıdır; `fingerprint` süreç içidir (kalıcılaştırılmaz); alan yolları `object, changes, changes[i], changes[i].key, changes[i].key[j], changes[i].version, changes[i].values[j], changes[i].set[j]` (+ `data_select` için `filter, order, order[i].column`) eşlenir; bilinmeyen yollar genel şeride.
- **Tepkisellik dersleri:** denetleyicide okunan her `Map` bir `registry` sinyaline bağlıdır; Solid sinyali kullanan her test dosyası başında `vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));`; saf model testleri Solid içe aktarmaz.
- **Erişilebilirlik/tema:** ızgara `role="grid"`, `aria-activedescendant`, hücre/satır durumları (değişti, silinecek, yeni, hata) renk + metin/`sr-only` ile; satır içi düzenleyici etiketli; uygulama sonucu `aria-live` durum satırı; renkler yalnızca CSS değişkenleri; seçili/durum rozetleri zemin rengiyle.
- Test: `bun run test`, `bun run typecheck`, `bun run build`; `cargo test --workspace` (Docker'sız) ve Task 2 Docker testi; git worktree **kullanılmaz**; `src/ipc/bindings` satır sonu gürültüsü `git checkout -- src/ipc/bindings`. Ajanlar GUI'yi elle denemez: elle kontrol maddeleri `docs/smoke-test.md`'ye `[—]` olarak yazılır. Docker: `PGT_TEST_URLS` yalnızca test komutunun ortamında (asla yazdırılmaz; sır adlı dosya yazılmaz); yalnızca `pgt-matrix-*` geçici kapsayıcıları; diğer kapsayıcılara (hub-broker, arniva-report) dokunulmaz. Commit eki: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Yorumlar Türkçe, çevredeki yoğunlukta.

## Review Focus

- **Paket bütçesi:** her görevde ana `index-*.js` ≤ Task 1 değeri; yeni metinler ana pakette yok; `__vite__mapDeps` yalnızca Task 1'de eklenen veri girdisini içerir; veri parçası `designer/*`/`ResultGrid` içe aktarmaz (yeni paylaşılan parça yok).
- **Veri bütünlüğü:** `toChangeSet` anahtar ve sürümü **özgün** satırdan alır (düzenlenen anahtar kolonu bile eski değerle `key`'e girer); no-op düzenleme değişiklik üretmez; NULL/boş/`'NULL'`/`'DEFAULT'` ayrımı; silinmiş+düzenlenmiş satır; yeni satır silme; Insert yalnız dokunulan kolonlar; başarılı uygulama sonrası satır tazeleme (returned) doğru satıra gider (indeks kayması yok); çakışma sonrası düzenlemeler kullanıcı onayı olmadan atılmaz.
- **Güvenlik/gizlilik:** değerler günlükte/depoda/toast'ta yok; salt okunur kaynakta (kilit, anahtar yok, yetki yok, view/MV, sistem) hiçbir düzenleme komutu çalışmaz ve uygulama düğmesi kapalı; üretimde Safe plan bile onay ister; DELETE onay kutusu; `outcomeUnknown` sonrası otomatik yeniden deneme yok; okuma oturumu salt okunur (Docker testi), kullanıcı SQL'i çalıştırmaz.
- **Yaşam döngüsü:** sekme kapanışında oturum kapanır ve uçuştaki uygulama iptal edilir; sunucu bağlantısı kesilince/oturum kaybında yeniden bağlanma veya açık hata; kirli sekme kapatma/F5/sıralama/filtre onayı; bayat `plan_changed` → kaynak+satır yeniden yükleme; hızlı ardışık düzenlemelerde önizleme yarışı (en son istek kazanır).
- **Gösterim:** bidi/denetim karakterleri görünür; uzun değerler kısaltılır; hücre 1000 karakter kırpma; INSERT çakışması farklı metin; hata yolları doğru hücreye/şeride.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `src/features/query/tabs-store.ts` (+ test), `QueryTabs.tsx`, `src/app/App.tsx`, `src/features/explorer/actions.ts` (+ test), `src/app/strings.ts`, `src/features/data/open-kinds.ts`, `src/features/query/strings-history.ts` (yeni), `HistoryView.tsx` | **Ana paket (Task 1):** `data` sekme türü, `openData` eylemi, yükleyici, telafi |
| `crates/pgt-query/src/session.rs`, `src-tauri/src/query.rs`, `src-tauri/src/main.rs` | **Rust (Task 2):** `open_read_only` + `qt_open_readonly` |
| `src/features/data/data-commands.ts`, `data-errors.ts`, `data-select-model.ts`, `src/app/strings-data.ts` (+ testler) | Komut sarmalayıcıları, hata eşleme, sıralama/filtre modeli, metinler |
| `src/features/data/data-edit-model.ts` (+ test) | Düzenleme katmanı (saf): hücre/satır durumu, `toChangeSet`, yeni sonuçların birleştirilmesi |
| `src/features/data/data-apply-model.ts` (+ test) | Kapı, onay, seçenekler, önizleme metni (kısaltma/görünür kaçış) |
| `src/features/data/data-controller.ts`, `data-open.ts` (+ testler) | Sekme denetleyicisi: oturum, yükleme, düzenleme, önizleme, uygulama, yaşam döngüsü |
| `src/features/data/data-grid-model.ts`, `DataGrid.tsx` | Sanallaştırılmış, düzenlenebilir ızgara |
| `src/features/data/DataTabs.tsx`, `DataToolbar.tsx`, `ChangePanel.tsx`, `DataBanner.tsx` | Sekme başlıkları/panelleri, araç çubuğu, değişiklik paneli, şeritler |
| `docs/smoke-test.md`, `docs/performance.md`, spec | Elle kontrol listesi, ölçümler, durum |

---

### Task 1: Ana paket — `data` sekme türü, eylem, yükleyici ve adı konmuş telafi

**Dosyalar:**
- Değiştir: `src/features/query/tabs-store.ts` (+ `tabs-store.test.ts`), `src/features/query/QueryTabs.tsx`, `src/app/App.tsx`, `src/features/explorer/actions.ts` (+ `actions.test.ts`), `src/app/strings.ts`, `src/features/query/HistoryView.tsx`
- Oluştur: `src/features/data/open-kinds.ts`, `src/features/query/strings-history.ts`, tembel **taslaklar** `src/features/data/data-open.ts` ve `src/features/data/DataTabs.tsx` (sonraki görevler doldurur)

**Arayüzler:**
- Üretir (`tabs-store.ts`; tür/anahtar kuralı ve metin **yok**, `designerTabKey` emsali):

```ts
export interface DataTab { id: string; serverId: string; databaseOid: number; object: ObjectRefLite; title: string; key: string; where: string }
// ObjectRefLite = { kind: "table" | "view" | "materialized_view"; oid: number }  (bindings'ten yalnız tip: ObjectRef)
export function isDataTabId(id: string): boolean;                       // "g" öneki
export function openData(state: TabsState, input: { serverId: string; databaseOid: number; object: ObjectRef; title: string; key: string; where: string }): { state: TabsState; id: string; created: boolean };  // key ile tekilleştirir
// taken, closeTab (sıra: sorgu, dash, designer, data), renameTab, activateTab, initialTabs, createTabsStore.openData güncellenir
```
- Üretir (`features/data/open-kinds.ts`, bağımlılıksız): `export const DATA_KINDS: NodeKind[] = ["table", "view", "materializedView"]; export function canOpenData(kind: NodeKind): boolean;` (bölümler ve bölümlü ana tablo `table` düğümüdür; kolon/kısıt/koleksiyon düğümleri değil).
- Üretir (`actions.ts`): `ActionId`'e `"openData"`; `ActionDeps.openData?(id: string): void`; `actionsFor`: `canOpenData(kind)` ise `openQueryTool`'un yanında `"openData"`; `groupOf("openData") = -1`; `disabledReason` **değişmez** (kilitli sunucuda salt okunur açılır); `executeAction`: `case "openData": deps.openData?.(id)`. Eylem adı `...Object` ile **bitmez** (designer eylemleriyle karışmaz).
- Üretir (`strings.ts`): `t.menu.openData = "Verileri Görüntüle/Düzenle"`.
- Üretir (`App.tsx`): `loadData = once(() => import("../features/data/data-open").then(m => setData(m.createData(lazyDeps()))))`, `data` sinyali; `actionDeps.openData: id => void loadData().then(d => d.open(id), loadFailed)`; `sessionsClosed` döngüsüne `data()`; `<QueryTabs data={data} …/>`.
- Üretir (`QueryTabs.tsx`): `lazy` `DataTabHeads`/`DataPanels` (`designer` emsali, `Suspense`); `props.data?: () => DataController | null`; `requestClose`: veri sekmesi kirli (`props.data?.()?.isDirty(id)`) ise **mevcut `designerClosing` onay mekanizması genelleştirilir** (`{ kind: "designer" | "data"; id }`; metinler tembel taraftan gelir) ya da ayrı sinyal — hangisi daha az bayt ise; F5 yakalama dalına veri sekmesi (`reloadNow(id)`); `closeData` → `controller.close(id)` + `ctl.tabs.close(id)`.
- Üretir (taslaklar): `data-open.ts` → `export function createData(deps): DataController` (`{ ctl: { isDirty(id): boolean; close(id): void; reloadNow(id): void; serverSessionsClosed(serverId): void }, open(nodeId: string): void }` — taslakta gövdeler boş/no-op); `DataTabs.tsx` → `DataTabHeads`, `DataPanels` (taslakta başlıklar ve "Yükleniyor…" yer tutucu paneli).
- Telafi: `t.query.history` bloğu `strings-history.ts`'e (`export const thist = { …birebir kopya… } as const`); `strings.ts`'ten silinir; `HistoryView.tsx` `thist` kullanır; `rg "t\.query\.history" src` → 0. Yetmezse Global Constraints'teki sıradaki aday bloklar.

- [ ] **Adım 1: Başlangıç ölçümü.** `bun run build`; ana `index-*.js` **232805** olmalı (değilse dur ve raporla); `__vite__mapDeps` girdi sayısını ve `index.html` önyüklemelerini ledger'a yaz.
- [ ] **Adım 2: Başarısız testler.** `tabs-store.test.ts`: `openData` aynı `key` için tek sekme (ikinci çağrı `created=false`, aynı id), farklı key yeni sekme; id öneki `g` ve `isDataTabId`; `taken` ve `activateTab` veri sekmesini tanır; `closeTab` etkin sekmeyi kapatınca sıradaki etkin sekme sırası (sorgu → dash → designer → data); `renameTab` veri sekmesini de etkiler. `actions.test.ts`: `table`/`view`/`materializedView` düğümünde `openData` var ve grup −1; `database`, `schema`, kolon, kısıt, koleksiyon düğümlerinde yok; kilitli sunucuda `disabledReason(openData)` **boş** (devre dışı değil); `executeAction("openData")` `deps.openData`'yı düğüm kimliğiyle çağırır.
- [ ] **Adım 3–4:** Başarısızlığı gör; uygula (ana paket + taslaklar + telafi).
- [ ] **Adım 5: Doğrulama.** `bun run test`, `bun run typecheck`, **Boyut doğrulaması**: ana ≤ 232,805 B (aşıyorsa sıradaki telafi adayını da taşı); ölçülen değeri ve `__vite__mapDeps` girdi sayısını commit mesajına ve `docs/smoke-test.md` "Ölçümler" altına yaz ("Plan 17 Task 1: index-*.js = N B, mapDeps = M girdi"). Aşılamıyorsa **dur ve raporla**.
- [ ] **Adım 6: Commit** (`feat(data): veri sekmesi türü, Verileri Görüntüle eylemi ve tembel yükleyici; geçmiş metinleri tembel parçaya (ana paket telafisi, index N B)`).

---

### Task 2: Rust — açıkça salt okunur okuma oturumu (`qt_open_readonly`)

**Dosyalar:**
- Değiştir: `crates/pgt-query/src/session.rs`, `src-tauri/src/query.rs`, `src-tauri/src/main.rs`
- Test: `crates/pgt-query/tests/session.rs` (Docker), `session.rs` birim testleri

**Arayüzler:**
- Üretir (`session.rs`): `QueryManager::open_read_only(&self, server: Uuid, target: Target, opts: SessionOptions) -> Result<SessionId>` — `open`'ın aynısı ama `SessionProfile::Query { read_only: true, .. }`; oturum yuvasına `force_read_only: bool` alanı eklenir ve `slot.profile(&opts)` `read_only = guard.locked || self.force_read_only` hesaplar (yeniden açılış `:1155` ve `set_options` yolları dahil). `open` davranışı değişmez (`force_read_only = false`).
- Üretir (`src-tauri/src/query.rs`): `#[tauri::command] pub async fn qt_open_readonly(state, server_id: Uuid, database_oid: Option<u32>, options: SessionOptions) -> CmdResult<String>` (`qt_open` kalıbı: hedef çözümü, `Box::pin`, aynı hata eşlemesi); `main.rs` `invoke_handler`'a kayıt.
- `SessionOptions` ve bindingleri **değişmez**; `commands.ts` **değişmez**.

- [ ] **Adım 1: Başarısız testler.** Birim: `force_read_only` yuvası kilitsiz sunucuda `profile()` `read_only = true` verir; `open` ile açılan yuva `false`. Docker (`crates/pgt-query/tests/session.rs`, `PGT_TEST_URLS` yoksa atlanır): (a) `open_read_only` oturumunda `INSERT`/`CREATE TABLE` PG `25006` (read-only transaction) ile başarısız olur, `SELECT` çalışır, `SHOW default_transaction_read_only = on`; (b) `set_options` (mod/zaman aşımı değişimi) sonrası oturum hâlâ salt okunur; (c) oturum düşürülüp yeniden açıldığında (yeniden bağlanma yolu) hâlâ salt okunur; (d) normal `open` oturumu kilitsiz sunucuda yazabilir (regresyon); (e) kilitli sunucuda iki yol da salt okunur.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula.
- [ ] **Adım 4:** `cargo test --workspace` (Docker'sız), `cargo clippy --workspace --all-targets -- -D warnings`, `bun run typecheck`, `bun run build` (ana paket **Task 1 değeri**), Docker: `pgt-matrix-*` geçici kapsayıcılarla PG 10..18 `session` testleri (filtrelenmiş `test-matrix` kopyası ya da tam betik; kapsayıcılar silinir).
- [ ] **Adım 5: Commit** (`feat(query): açıkça salt okunur oturum (open_read_only / qt_open_readonly) — veri düzenleyici okuma yolu`).

---

### Task 3: Komut sarmalayıcıları, hata eşleme, sıralama/filtre modeli, metinler

**Dosyalar:**
- Oluştur: `src/features/data/data-commands.ts`, `data-errors.ts`, `data-select-model.ts` (+ testler), `src/app/strings-data.ts`

**Arayüzler:**
- Üretir (`data-commands.ts`): `commands.ts`'in `call` yardımcısını kullanan (yoksa `export` eklenir — bayt değişmez) ince sarmalayıcılar: `dataLoad(serverId, databaseOid, object)`, `dataSelect(serverId, databaseOid, object, fingerprint, request)`, `dataPreview(serverId, databaseOid, changeSet)`, `dataApply(serverId, databaseOid, changeSet, options)`, `dataCancel(applyId)`, `qtOpenReadonly(serverId, databaseOid, options)`; `DataApi` arayüzü (bu fonksiyonlar + `ddlTargetOf`, `qtRun`, `qtMore`, `qtCancel`, `qtClose`, `refreshServers`, `connect`) — denetleyici testlerinde sahtelenir.
- Üretir (`data-errors.ts`, saf): `classifyDataError(payload): "disconnected" | "gone" | "locked" | "plan_changed" | "confirmation_required" | "invalid_def" | "other"` (`not_connected`/`connection_lost` → disconnected; `unknown_server`/`not_found` → gone; …); `isConnectionCode`; `outcomeText(out: DataOutcome-Failed, ctx: { changeKind: "insert" | "update" | "delete" | null }): { text: string; kind: "conflict" | "unknown" | "failed"; hint?: string }` — `conflict` ve INSERT → "satır bir tetikleyici/kural/RLS tarafından düşürüldü", UPDATE/DELETE → "satır başka bir oturumca değiştirilmiş ya da silinmiş (ya da tetikleyici/RLS süzdü)"; `outcomeUnknown` → "Sonuç belirsiz; kalıcılaşmış olabilir. Yeniden denemeden önce yeniden yükleyin."; diğer → PG iletisi (+ `55P03` için satır kilidi ipucu: "Satır başka bir oturumca kilitli; sonra yeniden deneyin."). `fieldIssues(errors: FieldError[], changeSet): { cell: Map<string /* `${changeIndex}:${columnName}` */, string[]>; row: Map<number, string[]>; general: string[] }` — `changes[i].set[j]`/`values[j]` yolu, değişiklik listesindeki `i`. değişikliğin `j`. hücresinin kolonuna; `changes[i]`, `changes[i].key*`, `changes[i].version` satıra; `object`, `changes`, bilinmeyen → genel.
- Üretir (`data-select-model.ts`, saf):

```ts
export interface SelectState { order: OrderBy[]; filter: string }              // order en çok 8
export const EMPTY_SELECT: SelectState;
export function cycleOrder(order: readonly OrderBy[], column: string, additive: boolean): OrderBy[];  // artan → azalan → kaldır; additive (Shift) mevcut sıralamaya ekler; en çok 8
export function toRequest(s: SelectState): SelectRequest;                      // filter: boş/boşluk → null
export function selectErrors(errors: FieldError[]): { filter: string[]; order: string[]; general: string[] };   // filter | order | order[i].column
```
- Üretir (`strings-data.ts`, `tdata`): menü dışı tüm metinler: sekme başlığı (`tdata.tab.title(name)`), açma (`open.none` "Bu nesne için veri açılamıyor", `open.locked`), şeritler (`ro.noKey`, `ro.noPrivilege`, `ro.systemObject`, `ro.unsupportedKind`, `ro.locked`, `ro.noVersion`), araç çubuğu (Yeniden yükle, Satır ekle, Satırı sil/geri al, Kaydet, Vazgeç, NULL, DEFAULT, filtre etiketi/yer tutucu/örnek, sıralama), durum satırları (`rows(n, more)`, `changes(n)`), değişiklik paneli ("Değişiklik paneli", risk etiketleri, "Kopyala", `…(+N karakter)`), onaylar (`confirm.destroy`, `confirm.production`), kirli sekme kapatma/yeniden yükle ("Yeniden yükle (düzenlemeler atılır)"), çakışma/`outcomeUnknown`/`plan_changed`/kilit metinleri, erişilebilirlik (`sr.changed`, `sr.deleted`, `sr.new`, `sr.error`).

- [ ] **Adım 1: Başarısız testler.** `data-errors.test.ts`: `classifyDataError` tablosu; `outcomeText` — UPDATE çakışma, INSERT çakışma (farklı metin), `outcomeUnknown`, `55P03`, PG hatası iletisi aynen geçer; `fieldIssues` — `changes[1].set[0]` ikinci değişikliğin ilk `set` kolonuna, `changes[0].values[2]` Insert'in üçüncü hücresine, `changes[2].key[0]`/`version` satıra, `object` genel. `data-select-model.test.ts`: `cycleOrder` döngüsü (yok → asc → desc → yok), Shift ile ekleme, 9. anahtar eklenmez; `toRequest` boş filtre `null`; `selectErrors` yolları. `data-commands` için yalnızca tür/uyum testi (`DataApi` sahte oluşturulabilir).
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula.
- [ ] **Adım 4:** `bun run test`, `typecheck`, **Boyut doğrulaması** (ana ≤ Task 1 değeri; `tdata` metinleri ana pakette yok).
- [ ] **Adım 5: Commit** (`feat(data): komut sarmalayıcıları, hata/alan yolu eşleme, sıralama-filtre modeli ve metinler`).

---

### Task 4: Düzenleme katmanı (saf model) ve `toChangeSet`

**Dosyalar:**
- Oluştur: `src/features/data/data-edit-model.ts` (+ `data-edit-model.test.ts`)

**Arayüzler:**
- Tüketir: `DataSource`, `DataColumn`, `DataKey`, `CellValue`, `DataChangeSet`, `RowChange`, `RowResult`.
- Üretir (saf, değişmez güncellemeler; Solid yok):

```ts
export type Row = (string | null)[];                  // kolonlar kaynak sırasıyla + (hasVersion ise) son eleman sürüm
export interface Loaded { source: DataSource; rows: Row[]; versionIndex: number; hasVersion: boolean }
export interface EditState {
  cells: ReadonlyMap<number, ReadonlyMap<string /* kolon adı */, CellValue>>;   // rowIndex → kolon → yeni değer (Text|Null|Default)
  inserted: readonly NewRow[];                        // { id: number; values: ReadonlyMap<string, CellValue> }
  deleted: ReadonlySet<number>;                       // rowIndex
}
export const EMPTY_EDIT: EditState;
export function isDirty(e: EditState): boolean;
export function changeCount(e: EditState): number;   // gerçek (no-op olmayan) değişiklik sayısı
/** Hücre düzenlenebilir mi ve değilse neden. */
export function cellState(l: Loaded, e: EditState, row: number | { newId: number }, column: string): { editable: boolean; reason?: "readOnly" | "generated" | "identityAlways" | "notUpdatable" | "deleted" };
export function setCell(l: Loaded, e: EditState, row: number, column: string, value: CellValue): EditState;   // özgün değere eşitse girdi kaldırılır; silinmiş/uygunsuz hücrede aynı durum
export function newRowSetCell(l: Loaded, e: EditState, id: number, column: string, value: CellValue): EditState;
export function revertCell(e: EditState, row: number, column: string): EditState;
export function toggleDelete(l: Loaded, e: EditState, row: number): EditState;   // silinecek ↔ geri al; silinen satırın düzenlemeleri tutulur ama gönderilmez
export function addRow(e: EditState): EditState;                                  // boş yeni satır (id artan)
export function removeNewRow(e: EditState, id: number): EditState;
export function cellValueAt(l: Loaded, e: EditState, row: number, column: string): { value: CellValue | { kind: "orig"; value: string | null }; changed: boolean };
export function toChangeSet(l: Loaded, e: EditState): { changes: RowChange[]; origin: ChangeOrigin[] };   // origin[i] = { kind: "insert"; newId } | { kind: "update" | "delete"; rowIndex }
/** Başarılı uygulama sonrası yükleneni tazele: UPDATE → satır yerinde (returned), INSERT → sona ekle, DELETE → satırı çıkar; düzenleme sıfırlanır. */
export function applyResults(l: Loaded, origin: ChangeOrigin[], results: RowResult[]): Loaded;
export function keyOf(l: Loaded, row: number): string[];                          // DataKey.columns sırasıyla özgün değerler (null yok; anahtar kolonları NOT NULL)
```
- Kurallar (Karar 4): Update `set` yalnız özgün değerden farklı hücreler (`Text` aynı metin ise no-op; `Null` ↔ özgün `null` no-op; `Default` her zaman farklı sayılır); Update/Delete `key`/`version` **özgün satırdan**; silinecek satırın düzenlemeleri `toChangeSet`'e girmez (yalnız Delete); Insert `values` yalnız dokunulan kolonlar, sıfır dokunulmuş boş yeni satır **atlanır** (`changeCount`'a girmez); `insertable=false`/`generated` kolon yeni satırda reddedilir; `identity=always` kolon Insert'te yalnız `Default`; Update'te `updatable=false` kolon reddedilir; kaynak `editable=false` ise tüm düzenleme işlevleri durumu değiştirmez. `applyResults`: `returned` kolonlar + sürüm (son) → satır; Insert `returned` yoksa (olmamalı) bile satır eklenir ve kullanıcıya "yeniden yükle" önerilir (`Loaded` değişmez; denetleyici işler); DELETE satırı çıkarılırken sonraki satır indeksleri kayar — işlem **büyükten küçüğe** yapılır.

- [ ] **Adım 1: Başarısız testler** (`data-edit-model.test.ts`; Solid yok; fikstür kaynağı: PK'lı `id int`, `name text`, `note text` varsayılanlı, `gen` üretilmiş, `ident` ALWAYS kimlik, `bd` BY DEFAULT kimlik, çok kolonlu anahtarlı ikinci kaynak):

```ts
it("setCell özgün değere dönünce girdi kalkar; changeCount no-op saymaz", () => {});
it("Null/Text/Default ayrımı: '' ≠ null ≠ 'NULL' ≠ 'DEFAULT'; özgün null'a Null yazmak no-op", () => {});
it("toChangeSet Update: set yalnız değişen kolonlar; key ve version özgün satırdan; anahtar kolonu düzenlense bile key eski değer", () => {});
it("toChangeSet Delete: key+version; silinmiş satırın düzenlemeleri gönderilmez; geri alınca Update'e döner", () => {});
it("toChangeSet Insert: yalnız dokunulan kolonlar; hiç dokunulmamış yeni satır atlanır; sırası değişiklik sırasıyla (insert/update/delete karışık) korunur ve origin eşler", () => {});
it("cellState: generated, identity always (Insert'te yalnız Default), updatable=false, silinmiş satır, salt okunur kaynak, view/MV (editable=false)", () => {});
it("çok kolonlu anahtar sırası DataKey.columns ile", () => {});
it("applyResults: UPDATE satırı returned ile yerinde (yeni sürüm), INSERT sona eklenir, DELETE'ler büyükten küçüğe çıkarılır ve indeks kayması yanlış satırı silmez; düzenleme sıfırlanır", () => {});
it("modül durumsuz: aynı girdi aynı çıktı; girdi nesneleri değiştirilmez (donmuş fikstürle)", () => {});
```
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula.
- [ ] **Adım 4:** `bun run test`, `typecheck`, **Boyut doğrulaması**.
- [ ] **Adım 5: Commit** (`feat(data): düzenleme katmanı (saf model) — hücre/satır durumu, toChangeSet, sonuçların birleştirilmesi`).

---

### Task 5: Kapı modeli ve önizleme gösterimi (saf)

**Dosyalar:**
- Oluştur: `src/features/data/data-apply-model.ts` (+ test)

**Arayüzler:**
- Üretir (saf):

```ts
export type DataGate =
  | { kind: "ready" }
  | { kind: "needsConfirm"; production: boolean; destructive: boolean }
  | { kind: "blocked"; reason: "locked" | "readOnly" | "empty" | "invalid" | "pending" | "applying" | "disconnected" | "loading" | "stale" | "unknown" };
export interface DataGateInput { locked: boolean; production: boolean; editable: boolean; view: PlanView | null; viewIsCurrent: boolean; pending: boolean; applying: boolean; connected: boolean; loading: boolean; needsReload: boolean; outcomeUnknown: boolean; dirty: boolean }
export function dataGate(i: DataGateInput): DataGate;     // öncelik: disconnected > loading > locked > readOnly > unknown (outcomeUnknown) > stale (needsReload) > empty(!dirty ya da view.empty) > invalid > pending > applying > needsConfirm > ready
export function canSubmit(g: DataGate, checked: boolean): boolean;
export function applyOptions(applyId: string, view: PlanView, checked: boolean, lockTimeoutMs?: number): DataApplyOptions;   // confirmed = checked; lockTimeoutMs varsayılan 5000
export function riskNeedsConfirm(view: PlanView, production: boolean): boolean;   // risk ≥ destructive || (production && !view.empty)
/** Önizleme/hücre gösterimi: bidi biçimlendirme ve denetim karakterlerini görünür kaçışa çevirir. */
export function visibleControls(text: string): string;                    // U+202A–202E, U+2066–2069, U+200B–200F, U+2028/2029, U+061C, C0/C1 (\t\n\r hariç) → ⟦U+XXXX⟧
export function shortenStatement(sql: string, max?: number): { text: string; omitted: number };   // 4000 karakter; vekil çift güvenli (emoji ortadan kesilmez); omitted = kalan karakter sayısı
export function statementDisplay(sql: string): { text: string; omitted: number };               // shorten + visibleControls (kaçışlar sınırı aşmadan önce uygulanır)
```
- [ ] **Adım 1: Başarısız testler.** Kapı önceliği tablosu (her çift); `riskNeedsConfirm` (Safe + üretim dışı → false; Safe + üretim → true; Destructive → true; boş plan + üretim → false); `applyOptions` (`confirmed`, `expectedHash`, `lockTimeoutMs` 5000, `applyId` aynen); `visibleControls`: `U+202E` `⟦U+202E⟧`, sıfır genişlikli boşluk, satır içi `\n` korunur, düz metin değişmez, vekil çiftler korunur; `shortenStatement`: sınırda değişmez, aşımda `omitted` doğru, emoji (vekil çift) ortadan kesilmez, boş dizge; `statementDisplay` kaçışlı metin uzunluğu sınırı aşmaz.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula. **Adım 4:** `bun run test`, `typecheck`, **Boyut doğrulaması**.
- [ ] **Adım 5: Commit** (`feat(data): uygulama kapısı, onay kuralı ve önizleme gösterimi (görünür kaçışlar, kısaltma)`).

---

### Task 6: Sekme denetleyicisi (`data-controller.ts`, `data-open.ts`)

**Dosyalar:**
- Oluştur: `src/features/data/data-controller.ts`, `data-controller.test.ts`, `data-reactivity.test.ts`; doldur: `data-open.ts` (+ `data-open.test.ts`)

**Arayüzler:**
- Tüketir: Task 3–5 modülleri, `DataApi`.
- Üretir (`data-controller.ts`; `designer-controller.ts` kalıbı: `entries` Map + `registry` sinyali + `bump()`, `gen` yarış sayacı, `alive(tabId, e, gen)`):

```ts
export interface DataModel {
  tabId: string; serverId: string; databaseOid: number; object: ObjectRef;
  status: "loading" | "ready" | "error" | "gone" | "disconnected";
  source: DataSource | null; loaded: Loaded | null; edit: EditState;
  more: boolean; truncatedAtCap: boolean; select: SelectState; selectErrors: { filter: string[]; order: string[]; general: string[] };
  view: PlanView | null; viewIsCurrent: boolean; pending: boolean; applying: boolean;
  gate: DataGate; confirm: { checked: boolean };
  issues: ReturnType<typeof fieldIssues> | null;            // son önizleme/uygulama alan hataları
  conflictRow: number | null; outcome: ReturnType<typeof outcomeText> | null;
  needsReload: boolean; outcomeUnknown: boolean; loadError: string | null; postApplyNote: string | null;
}
export interface DataTabCtl {
  model(tabId): DataModel | null;                           // registry'ye bağlı okuma
  isDirty(tabId): boolean;
  openTab(input): string;                                   // sekme girdisi + yükleme başlatır
  reload(tabId, opts?: { discard: boolean }): Promise<void>;   // kirli ise çağıran onay almıştır (discard=true), değilse reddeder
  loadMore(tabId): Promise<void>;
  setSelect(tabId, next: SelectState): Promise<void>;       // sıralama/filtre → yeniden yükleme (kirli ise onay bayrağı gerekir)
  edit(tabId, fn: (l: Loaded, e: EditState) => EditState): void;   // düzenleme + önizleme zamanlaması
  setConfirm(tabId, checked: boolean): void;
  apply(tabId): Promise<void>; cancelApply(tabId): void;
  close(tabId): void;                                       // oturumu kapatır, uçuştaki uygulamayı data_cancel eder, girdiyi siler
  serverSessionsClosed(serverId): void; serverChanged(serverId): void;   // kilit/üretim/ad tazelemesi
}
```
- Davranış:
  - **Açma** (`data-open.ts`'te `open(nodeId)`; `designer-open.ts:76-157` kalıbı: tek uçuşlu `opening` bayrağı; `Promise.all([ddlTargetOf(nodeId), refreshServers()])`; `target.object == null` ya da `target.focus != null` ya da `databaseOid == null` ya da tür ∉ {table, view, materialized_view} → `tdata.open.none` toast; sunucu bağlı değilse `connect`; `key = "data:<serverId>:<databaseOid>:<oid>"` (aynı nesne tek sekme, mevcutsa etkinleştir); sekme başlığı düğüm adı + `where`: `<sunucu>/<veritabanı>.<şema>`; kilitli sunucuda **açılır** (salt okunur şeridi)).
  - **Yükleme**: `dataLoad` → `source`; `source.hasVersion`/`editable` salt okunur nedenini şeride taşır; `qtOpenReadonly(serverId, databaseOid, { mode: "autocommit", statementTimeoutMs: 30000 })` → oturum (denetleyiciye özel; **kullanıcı SQL'i çalıştırılmaz**); `dataSelect(…, source.fingerprint, toRequest(select))` → `Ok{select}` ise `qtRun(session, select.sql, {type:"all"}, 1000, false, onEvent)`; `Invalid{errors}` ise `selectErrors` doldurulur ve satırlar değişmez. Olay işleme: `started` sütunlar (sayı `columns.length`, sürüm son kolon, `versionIndex` doğrulaması: uyuşmazsa `loadError`), `rows` satırları `Loaded.rows`'a ekler (yalnız ekleme), `failed` `loadError`; **50 000** sınırında `more` kapanır, `truncatedAtCap = true`. `loadMore` `qtMore(session, 0, …)`; `qt_more` hatası (imleç kapandı) → `loadError` + "Yeniden yükle" ipucu.
  - **Oturum yaşam döngüsü**: `not_connected`/`session_reset` → `deps.connect` ile yeniden bağlan + oturumu yeniden aç + bir kez yeniden dene (`query-controller.ts` `ensureSession` kalıbı); `connection_lost` → `status = "disconnected"`; `serverSessionsClosed(serverId)` ilgili sekmeleri `disconnected` yapar (düzenleme durumu **korunur**, yeniden bağlanınca devam).
  - **Düzenleme/önizleme**: `edit()` `EditState`'i günceller; `toChangeSet` boş değilse 150 ms debounce'lu (kendi küçük zamanlayıcı; `createPreviewer` içe aktarılmaz) `dataPreview({object, fingerprint: source.fingerprint, changes})`; en son istek kazanır (`gen`/`requestId`); sonuç `view` + `issues = fieldIssues(view.errors, changeSet)`; `plan_changed`/parmak izi uyuşmazlığı (`Err plan_changed`) → `needsReload = true`; `gone`/`disconnected` kodları durumu günceller. Kirli değilse `view = null`.
  - **Uygulama** (`apply`): kapı `dataGate(...)` (`peek` ile yeniden hesaplanır; `viewIsCurrent` değilse önce önizleme); `applyId = crypto.randomUUID()`; `dataApply(…, applyOptions(applyId, view, confirm.checked))`; `Applied` → `applyResults` ile `loaded` tazelenir, `edit = EMPTY_EDIT`, `view = null`, `confirm.checked = false`, `postApplyNote = tdata.status.applied(n)` (`aria-live`); `Failed`: `conflict` → `conflictRow = origin[failedIndex]` satırı, `outcomeText` (`changeKind` = ilgili değişiklik türü), düzenlemeler **korunur** (kullanıcı "Yeniden yükle (düzenlemeler atılır)" der); `outcomeUnknown` → `outcomeUnknown = true` (kapı kapanır, yeniden yükleme zorunlu, otomatik deneme yok); diğer hata (kısıt vb.) → `outcome` (PG iletisi) ve önizleme yeniden tetiklenir, düzenleme korunur; `Err plan_changed` → `needsReload`; `Err locked` → `locked = true` (kapı kapanır); `Err confirmation_required` (kapı bayatlığı) → onay kutusunu sıfırla + önizleme tazele; bağlantı kodları `disconnected`. `cancelApply` `dataCancel(applyId)`. Aynı anda tek uygulama (`applying`).
  - **Kapanış/yenileme**: `close` oturumu kapatır (`qtClose` `ClosePolicy` ile; en iyi çaba), uçuştaki uygulamayı iptal eder, `gen` artırır; `reload(discard:false)` kirliyse reddeder (`false` döner; bileşen onay sorar); `serverChanged` kilit/üretim/ad tazeler.
- [ ] **Adım 1: Başarısız testler** (`data-controller.test.ts`; `designer-controller.test.ts` sahte `Api` kalıbı; `data-reactivity.test.ts` başında `vi.mock("solid-js", …)`):

```ts
it("açma: databaseOid null, focus dolu, tür uygunsuz → none toast ve sekme yok; kilitli sunucuda açılır", () => {});
it("yükleme: dataLoad → qtOpenReadonly(autocommit) → dataSelect → qtRun; sürüm kolonu son; started/rows olayları satırları ekler; 50 000 sınırında more kapanır", () => {});
it("select Invalid → selectErrors dolar, satırlar değişmez; setSelect kirli iken onaysız reddedilir", () => {});
it("düzenleme → debounce'lu önizleme tek istek (yarış: eski sonuç yok sayılır); no-op düzenleme önizleme istemez", () => {});
it("hata yolları fieldIssues ile doğru hücreye/satıra eşlenir", () => {});
it("apply başarı: loaded tazelenir (UPDATE yerinde, INSERT sona, DELETE çıkar), edit sıfırlanır, aria-live notu", () => {});
it("apply conflict: conflictRow = origin[failedIndex], düzenleme korunur, INSERT için farklı metin, otomatik yeniden deneme YOK (dataApply tek çağrı)", () => {});
it("apply outcomeUnknown: kapı kapanır, yeniden yükleme zorunlu, ikinci dataApply çağrılmaz", () => {});
it("apply Err: plan_changed → needsReload; locked → kilit; confirmation_required → onay sıfırlanır + önizleme; bağlantı kodu → disconnected", () => {});
it("üretimde Safe plan onay kutusu ister; DELETE onay ister; onaysız dataApply çağrılmaz", () => {});
it("close: qtClose çağrılır, uçuştaki apply için dataCancel; reload kirli iken discard olmadan false", () => {});
it("oturum kaybı: bir kez yeniden bağlan+aç+dene; connection_lost → disconnected; serverSessionsClosed düzenlemeyi korur", () => {});
it("değer sızıntısı yok: console.* çağrılmaz; toast/notify argümanlarında hücre/anahtar/sürüm/SQL yok", () => {});
```
  `data-reactivity.test.ts`: `model(tabId)` `registry` bağlı (düzenleme sonrası memo güncellenir); `isDirty`. `data-open.test.ts`: tek uçuş; aynı nesne ikinci açılış mevcut sekmeyi etkinleştirir; `ddlTargetOf` hatası toast.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula.
- [ ] **Adım 4:** `bun run test`, `typecheck`, **Boyut doğrulaması** (ana ≤ Task 1 değeri; denetleyici `designer/*`'ı içe aktarmaz: `rg "from \"\.\./designer" src/features/data` boş).
- [ ] **Adım 5: Commit** (`feat(data): veri sekmesi denetleyicisi — salt okunur oturum, yükleme, düzenleme, önizleme, kapılı uygulama, çakışma/belirsiz sonuç/oturum kaybı`).

---

### Task 7: Izgara ve arayüz bileşenleri

**Dosyalar:**
- Oluştur: `src/features/data/data-grid-model.ts` (+ test), `DataGrid.tsx`, `DataToolbar.tsx`, `ChangePanel.tsx`, `DataBanner.tsx`; doldur: `DataTabs.tsx`

**Arayüzler:**
- Üretir (`data-grid-model.ts`, saf; `grid-model.ts`'ten **kopya**, bağımsız): `ROW_HEIGHT = 24`, `MIN_COL_WIDTH`, `MAX_COL_WIDTH`, `MAX_CELL_CHARS = 1000`; `initialWidths(columns, sampleRows)`; `isNumericType(typeName)`; `displayCell(value)` → `{ text, null, truncated }` (bidi/denetim `visibleControls` ile kaçışlı, satır sonu `⏎`); `Pos`, `moveFocus(pos, key, rows, cols, pageSize)`; `cellKind(l, e, row, column)` → `"normal" | "changed" | "null" | "default" | "readonly" | "error"` (stil/`sr-only` metni için).
- Üretir (`DataGrid.tsx`): `role="grid"`, çift sanallaştırıcı (satır `ROW_HEIGHT`, sütun), yapışkan başlık; satırlar = `loaded.rows` + yeni satırlar (alta, "yeni" rozetli) ; **tek hücre odağı** (`aria-activedescendant`); stil: değişen hücre `bg-selected`+sol işaret yerine zemin tonu (CSS değişkeni) + `sr-only` "değişti"; silinecek satır üstü çizili + zemin tonu + `sr-only`; yeni satır zemin tonu; salt okunur kolon soluk; hata hücresi `text-danger` çerçeve (zemin rengiyle) + `aria-invalid` + hata metni `aria-describedby`; `conflictRow` satırı vurgulu. **Satır içi düzenleyici:** `Enter`/`F2`/çift tık → etkin hücre üzerinde mutlak konumlu `<input>` (değerde `\n` varsa ya da > 120 karakterse `<textarea>`), etiketli (`aria-label` = kolon adı), tam değeri yükler; `Enter` onayla+aşağı (textarea'da `Ctrl+Enter`), `Tab` onayla+sağa, `Esc` vazgeç; NULL/DEFAULT düzenleyicide yazılamaz (yalnız komutla); `Alt+N`/`Alt+D`, `Delete`, `Insert`, `Ctrl+Z`, `Ctrl+C` (etkin hücre) Karar 8; başlık tıklaması → `cycleOrder` (Shift ile ekle) ve `setSelect`; sıralama göstergesi başlıkta (`aria-sort`); sütun genişliği sürükleme (`ResultGrid` kalıbı); altbilgi: yüklü satır sayısı + "Daha fazla" (`more` ve < 50 000) + sınır notu.
- Üretir (`DataToolbar.tsx`): Yeniden yükle, Satır ekle, Satırı sil/geri al, NULL, DEFAULT, Vazgeç (tüm değişiklikler; onaylı), Kaydet (kapı + onay kutusu bilgisi `ChangePanel`'de), filtre girdisi (etiketli, Enter uygular, hata `aria-describedby`, `tdata` yer tutucu), sıralama özeti (temizle). Salt okunur durumda ilgili düğmeler `disabled` + gerekçe `title`.
- Üretir (`ChangePanel.tsx`): "Değişiklik paneli" başlığı + `changes(n)`; önizleme ifadeleri `statementDisplay` ile (kısaltılmış, kaçışlı; her ifadede "Kopyala" tam metni kopyalar; risk rozeti — **küçük yerel rozet**, `RiskBadge` içe aktarılmaz); `view.warnings`; genel alan hataları + `issues.general`; onay kutusu (`riskNeedsConfirm`) metni `confirm.destroy` / `confirm.production`; **Kaydet** / **İptal** (uygulama sırasında `cancelApply`); `outcome` şeridi (`role="alert"`: çakışma, belirsiz sonuç, hata; çakışmada "Yeniden yükle (düzenlemeler atılır)"; `outcomeUnknown`'da yalnız yeniden yükleme); `postApplyNote` `role="status" aria-live="polite"`.
- Üretir (`DataBanner.tsx`): salt okunur gerekçe şeridi (`readOnlyReason` → `tdata.ro.*`; `locked`; `hasVersion=false`), `loadError`, `disconnected`/`gone` durumları, `needsReload` ("Nesne değişti, yeniden yükleyin") — `role="status"`/`alert` uygun yerde.
- Üretir (`DataTabs.tsx`): `DataTabHeads` (ortak `TabHead` kabuğu `Head` olarak alınır; başlık + kirli noktası `bg-accent` + `sr-only` "değişti"), `DataPanels` (mevcut paneller `invisible pointer-events-none` ile bağlı kalır; üstte `DataBanner`, `DataToolbar`, orta `DataGrid`, altta `ChangePanel`; kirli kapatma onayı `ConfirmDialog` — `tdata` metinleri, odak Vazgeç'te, `danger`).

- [ ] **Adım 1: Başarısız testler** (`data-grid-model.test.ts`; Solid yok): `displayCell` null/boş/uzun/çok satırlı/bidi kaçışı; `isNumericType`; `moveFocus` sınırlar ve sayfa; `cellKind` tablosu (değişti/NULL/DEFAULT/salt okunur/hata); `initialWidths` sınırları. Bileşenler için otomatik test yok (env node): davranışları `docs/smoke-test.md`'de `[—]`.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula.
- [ ] **Adım 4:** `bun run test`, `typecheck`, **Boyut doğrulaması** (ana ≤ Task 1 değeri; `rg "ResultGrid|grid-model|designer/" src/features/data` yalnız `import type`/yorum; veri parçaları listesi ledger'a; `index.html` yalnız solid/web/Icon).
- [ ] **Adım 5: Commit** (`feat(data): düzenlenebilir ızgara, araç çubuğu, değişiklik paneli ve sekme bileşenleri`).

---

### Task 8: Elle kontrol listesi, belgeler ve bütünleşik doğrulama

**Dosyalar:**
- Değiştir: `docs/smoke-test.md`, `docs/performance.md`, `docs/superpowers/specs/2026-10-07-data-editor-design.md`

- [ ] **Adım 1: `docs/smoke-test.md`** — Plan 16 bölümündeki "GUI maddeleri Plan 17 bölümündedir" cümlesinin ardına **"## Veri düzenleyici arayüzü (Plan 17)"** (ön koşul: Docker PG 13+ ve 17+; PK'lı tablo, PK'sız tablo, birleşik anahtarlı tablo, bölümlü tablo + bölümler, kalıtım üst/alt tablo, view, MV, üretilmiş+kimlik kolonlu tablo, tetikleyicili tablo, 100k satırlı tablo; üretim etiketli sunucu; kilitli sunucu; yetkisiz rol). Maddeler (hepsi `- [ ] … **[—]**`):
  - Menü: table/view/MV düğümünde **Verileri Görüntüle/Düzenle** (üstte, Sorgu Aracı yanında); bölüm düğümünde de; kolon/kısıt/veritabanı/şema düğümünde yok; kilitli sunucuda etkin (salt okunur açılır); aynı nesne ikinci kez tek sekme.
  - Okuma: ilk 1000 satır hızlı, "Daha fazla", 50 000 sınırı notu; sıralama (başlık tıklaması, Shift ile çoklu), filtre (`status = 'a'`, hata: `;`, `\` → E'…' ipucu); salt okunur şerit nedenleri (anahtar yok, view, MV, sistem tablosu, yetkisiz rol, kilitli); kalıtım üst tablosunda çocuk satırları görünmez; bölümlü tabloda bölüm satırları görünür; imleç 5 dk sonra "Daha fazla" hatası → yeniden yükle.
  - Düzenleme: çift tık/F2/Enter ile düzenle, Enter/Tab/Esc; NULL (`Alt+N`) ve DEFAULT (`Alt+D`) — `''` ile ayrımı önizlemede; `Ctrl+Z` hücre geri alma; üretilmiş/ALWAYS kimlik hücreleri kilitli, BY DEFAULT kimlik düzenlenir; anahtar kolonunu değiştirme (uyarı + uygulama sonrası yeni anahtarla tekrar düzenleme); `Insert` yeni satır (yalnız dokunulan kolonlar önizlemede), yeni satır silme; `Delete` işaretle/geri al; çok satırlı/uzun değer `<textarea>`; Ctrl+C hücre kopyalama.
  - Önizleme/kaydet: "Değişiklik paneli" ifadeleri anlık (150 ms), uzun değer kısaltması + "Kopyala", bidi karakter içeren değer `⟦U+202E⟧` görünür; Safe plan (dev), Destructive (DELETE) onay kutusu, **üretim sunucusunda** Safe plan bile onay; Kaydet → satırlar yerinde tazelenir (varsayılan/serial/tetikleyici sonucu görünür), durum satırı okunur (`aria-live`); alan hataları (yanlış tipte değer 22P02 → PG iletisi ve hücre vurgusu).
  - Çakışma: ikinci istemcide (psql) aynı satırı güncelle → Kaydet → "satır değişmiş" şeridi + satır vurgusu, **hiçbir değişiklik yazılmaz**, düzenlemeler korunur; "Yeniden yükle (düzenlemeler atılır)" onayı; silinmiş satır; `BEFORE` tetikleyicisi `RETURN NULL` → INSERT çakışma metni ("tetikleyici/kural/RLS düşürdü"); ortada kısıt ihlali (UNIQUE) → tüm değişiklikler geri alınır, PG iletisi gösterilir; satır kilidi (psql `SELECT … FOR UPDATE`) → 55P03 ipucu ve **İptal** düğmesi çalışır.
  - `outcomeUnknown` (zor tetiklenir; birim testle kapsanır) metni ve yalnız yeniden yükleme.
  - Yaşam döngüsü: kirli sekme kapatma / F5 / sıralama değişimi / filtre uygulama onayı; sekme kapanınca sunucuda `idle in transaction` kalmaz (`pg_stat_activity`); sunucu bağlantısı kesilince "bağlantı kesildi" ve yeniden bağlanınca düzenlemeler korunur; sunucu kilidi açılıp kapanınca (sunucu düzenleme) kilit şeridi güncellenir; nesne `ALTER TABLE ADD COLUMN` ile değişince `plan_changed` → "yeniden yükleyin".
  - Klavye/erişilebilirlik/tema: ızgara oklar/Home/End/PageUp/PageDown; tüm düzenleme komutları klavyeyle; ekran okuyucuda hücre durumu (değişti/silinecek/yeni/hata); açık/koyu tema; dar pencerede (≈1100 px) araç çubuğu taşmaz; 100k satırda kaydırma akıcı (DOM'da ~100 hücre).
  - Gizlilik: geliştirici konsolunda ve günlükte hücre değeri/SQL yok; `localStorage`'da veri yok.
  - "Ölçümler" altına son ölçüm: "Plan 17 sonu: index-*.js = N B; Data*-*.js parçaları = …; DataGrid = …".
- [ ] **Adım 2: `docs/performance.md`** — veri sekmesi ölçümü (10k satırlı sekme bellek, 100k tabloda ilk açılış süresi, kaydırma) `measure.ps1` yöntemiyle **elle** (ajan çalıştırmaz; `[—]` ve talimat).
- [ ] **Adım 3: Spec** — Durum satırı "Plan 16–17 uygulandı"; §3 sonuna kısa **"Plan 17 notları"**: salt okunur okuma oturumu (`qt_open_readonly`), kendi ızgara bileşeni ve kopya gerekçesi (Karar 2), `toChangeSet` kuralları, kısayollar, sınırlar (50 000 satır, 4000 karakter önizleme), ana paket telafisi (`t.query.history` → `strings-history.ts`), backlog (ortak ızgara, yapıştırma, büyük nesne düzenleyici, kullanıcı seçimli anahtar, kolon gizleme/sıralama).
- [ ] **Adım 4: Bütünleşik doğrulama.** `bun run test`, `bun run typecheck`, `bun run build` + **Boyut doğrulaması** (ana ≤ Task 1 değeri; parça boyutları rapora); `cargo test --workspace` (Docker'sız), `cargo clippy --workspace --all-targets -- -D warnings`; Task 2 Docker testlerinin PG 10..18 tam matris çalıştırması (`scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf`).
- [ ] **Adım 5: Kapanış greps:** `console\.` / `localStorage|sessionStorage` / `innerHTML` `src/features/data` → boş; `from "\.\./designer|from "\.\./query/(ResultGrid|grid-model|result-model)` `src/features/data` → boş (yalnız `import type` istisnası ayrı incelenir); `strings-data` yalnız `src/features/data` ve `src/app/strings-data.ts`'te; `t\.query\.history` → 0; `rg "setItem|getItem" src/features/data` boş.
- [ ] **Adım 6: Commit** (`docs(data): veri düzenleyici arayüzü elle kontrol listesi, performans notu, spec durumu; bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 17 bitişi)

- Tam akış: menü → açma (tek sekme) → salt okunur oturumdan okuma (sıralama/filtre/sayfalama) → hücre/satır düzenleme → önizleme → kapılı uygulama (onay kuralları) → satırların yerinde tazelenmesi; çakışma/`outcomeUnknown`/`plan_changed`/kilit/bağlantı kaybı durumları; kirli sekme onayları.
- Ana paket ≤ Task 1 değeri (≤ 232,805 B); yeni kod ve metinler yalnızca tembel parçalarda; `index.html` önyüklemeleri yalnızca solid/web/Icon; veri parçası `designer/*` ve `ResultGrid`'i içe aktarmaz.
- Elle kontrol maddeleri `[—]`; final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §3 giriş (Task 1), ızgara/düzenleme (Task 4, 7), değişiklik paneli (Task 5, 7), salt okunur durumlar (Task 6, 7), sıralama/filtre (Task 3, 6, 7); §7 sözleşme: okuma oturumu (Task 2, 6), `ONLY`/kalıtım (arka uç; smoke), değişiklik anahtarı/sürüm (Task 4), uygulama sonucu/`returned` (Task 4, 6), önizleme bidi/kısaltma (Task 5, 7), hata kodları/alan yolları (Task 3, 6), `outcomeUnknown` (Task 3, 6, 7), INSERT çakışma metni (Task 3), kilit zaman aşımı varsayılanı 5000 ms (Task 5, 6).
- **Bilinçli kararlar:** `SessionOptions` değişmez, yeni `qt_open_readonly` (Karar 1); `ResultGrid`/`designer` kodunun **kopyalanması** (Karar 2; paylaşım backlog); satır kimliği = dizi indeksi (Karar 3); onay kutusu tek (ad yazma yok, Karar 5).
- **Yer tutucu taraması:** "TBD/TODO" yok; Task 1 taslak dosyaları Task 6–7'de doldurulur (açıkça belirtilmiş ara durum); `QueryTabs.tsx` kapatma onayının genelleştirilmesi/ayrı sinyal seçimi uygulayıcıya bırakılmıştır ve **bayt ölçümüyle** bağlanmıştır.
- **Tip tutarlılığı:** `Loaded`/`EditState`/`ChangeOrigin` (Task 4) → `DataModel` (Task 6) → bileşenler (Task 7); `DataGate`/`applyOptions` (Task 5) ↔ denetleyici `apply` (Task 6); `fieldIssues` anahtarı `"${changeIndex}:${columnName}"` (Task 3) ↔ `origin` eşlemesi (Task 4) ↔ ızgara hücre hatası (Task 7); `DataApi` (Task 3) ↔ sahte API testleri (Task 6).
