# Şema Karşılaştırma — Ön Yüz (Plan 30) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Explorer'dan **"Şema karşılaştır…"** (şema ve veritabanı düğümleri) ile açılan tembel **"Şema karşılaştırma"** sekmesi. Sekmenin içeriği:
- kaynak ve hedef seçicileri (sunucu → veritabanı → şema, "↔" ile yer değiştirme),
- seçenekler, "Karşılaştır" / "İptal" ve taraf başına ilerleme,
- sanallaştırılmış fark listesi (durum filtreleri ve sayılar, arama, seçim),
- seçili öğenin satır farkı, notları ve hataları,
- "Betik oluştur" önizlemesi: salt okunur CodeMirror, risk, uyarılar, hedef üretim/kilit bilgisi; "Query Tool'da aç", "Kopyala", "Kaydet…".

Plan 29 arka ucunu uçtan uca kullanılabilir yapar.

**Mimari:**
- **Ana pakette** yalnız şunlar bulunur:
  - menü eylemi `schemaDiff` ve tek menü metni,
  - `tabs-store`'da `DiffTab` kaydı,
  - `QueryTabs`'ta tembel başlık/panel bağlantısı,
  - App'te tembel yükleyici.
  - Komut sarıcıları ana pakete **girmez** (`diff-commands.ts`, `data-commands.ts` emsali).
- **Tembel parçalar** (üç kat):
  1. `diff-open.ts`: bağımlılıksız giriş (`data-open.ts` emsali). App ilk "Şema karşılaştır…"ta yükler; denetleyiciyi `./DiffTabs` ile paralel indirir.
  2. `DiffTabs.tsx`: başlıklar, panel kabuğu ve `startDiff` (denetleyici + açıcı). `QueryTabs` bunu tembel içe aktarır. Yalnız kendi özel modüllerini statik içe aktarır.
  3. `SchemaDiffView.tsx`: `DiffTabs` içinden tembel yüklenen ağır görünüm. Onun içinden de tembel `ScriptDialog.tsx` (CodeMirror, `ui/Dialog`, `@tauri-apps/plugin-dialog`) gelir.
- **Durum:** `diff-controller.ts` (enjekte api, sekme başına durum, saf testli). Kurallar ve metin üreticileri saf `diff-model.ts`'tedir. Metinler `src/app/strings-diff.ts`'tedir (`tsd`; yalnız tembel parça içe aktarır).
- **İş akışı:** `schema_diff_databases` + `ddlLookup({kind:"schemas"})` (seçiciler) → `schema_diff_start` (`Channel<DiffEvent>`) → `schema_diff_detail` (öğe odaklanınca) → `schema_diff_script` → `query.openScript` / `copyText` / `schema_diff_save`. Sekme kapanınca `schema_diff_close`.

**Teknoloji:** SolidJS, `@tanstack/solid-virtual` (mevcut), CodeMirror (mevcut `inspector/codemirror-setup`), Kobalte (yalnız tembel; `ui/Dialog`), `@tauri-apps/plugin-dialog` (`save`), vitest. Rust değişikliği yok.

**Spec:** `docs/superpowers/specs/2026-10-31-schema-diff-design.md`. Bağlayıcı olanlar: §2.7–2.8 ve **§6 Plan 29 sözleşme notları** (komutlar, hata kodları, alan anlamları). Paket kuralları: `docs/testing.md` "Ana paket bütçesi (Plan 21)". Emsaller: Plan 17 (veri sekmesi: `src/features/data/{data-open.ts,DataTabs.tsx,data-commands.ts}`), Plan 27 (`docs/superpowers/plans/2026-10-31-backup-p27-on-yuz.md`).

## Kararlar

1. **Menü:**
   - Eylem kimliği `schemaDiff`. Adı `Object` ile bitmez (designer kipi değildir). Grup 3.
   - Görünürlük: `schema` düğümü (`flags.system` değilse) ve `database` düğümü.
   - Karşılaştırma salt okunurdur, kilitli sunucuda da **açıktır** (`disabledReason` → `null`).
   - Sıra: `…, maintenance, backup, restore, schemaDiff, dropObject`.
   - Metin: `t.menu.schemaDiff = "Şema karşılaştır…"`.
2. **Sekme:**
   - Her menü çağrısı yeni bir sekme açar (tekilleştirme yok).
   - Kimlik öneki `s` (`isDiffTabId = (id) => id[0] === "s"`); kayıt `DiffTab { id, title }`.
   - Başlık tembel parçada üretilir: ilk başlık "Şema karşılaştırma". Karşılaştırma bitince `renameTab` ile "‹kaynakDb›.‹kaynakŞema› ↔ ‹hedefDb›.‹hedefŞema›" olur.
   - Sekmeler kalıcı değildir (yeniden açılışta geri gelmez); sayfa yenilenince arka uç kaydı da temizlenir.
3. **Seçiciler:**
   - Sunucu listesi `deps.query.servers()`'tır (tepkisel `ServerConfig[]`; ad, renk, üretim, kilit).
   - Sunucu seçilince `deps.connect(serverId)` (gerekirse parola istemi), ardından `schemaDiffDatabases(serverId)`.
   - Veritabanı seçilince `deps.api.ddlLookup(serverId, oid, { kind: "schemas" })` (sistem şemaları zaten hariç).
   - Seçici yanıtları kuşak sayacıyla korunur; eski yanıt durum yazmaz.
   - Sunucu değişince veritabanı ve şema sıfırlanır; veritabanı değişince şema sıfırlanır.
4. **Giriş noktası:** `deps.api.ddlTargetOf(nodeId)`:
   - Şema düğümünde `{ serverId, databaseOid, object: { kind: "schema" } }` → kaynak `{serverId, databaseOid, schema: düğüm etiketi}`.
   - Veritabanı düğümünde `{ databaseOid: null, object: { kind: "database", oid } }` → kaynak `{serverId, databaseOid: oid, schema: null}`.
   - Hedef varsayılanı: aynı sunucu ve veritabanı, şema seçilmemiş.
   - Seçicilerin listeleri arka planda yüklenir.
5. **"Aynı şema" reddi:** sunucu, veritabanı ve şema eşitse "Karşılaştır" kapalıdır ve gerekçe gösterilir. Arka ucun `same_schema` kodu da aynı metne eşlenir.
6. **Seçenekler:** "Sahiplik ve yetkiler" (varsayılan kapalı), "Tablespace" (kapalı), "Yorumlar" (açık). Sonuç varken seçici ya da seçenek değişirse sonuç **bayat** olur:
   - Liste görünür kalır.
   - Üst şeritte "Seçimler değişti; yeniden karşılaştırın." görünür.
   - "Betik oluştur" kapanır.
7. **Çalıştırma:**
   - `diffId = crypto.randomUUID()`. Önceki `diffId` varsa önce `schemaDiffClose` çağrılır (hatası yutulur).
   - İlerleme taraf başına çubuktur (`loaded / total`; `total = 0` iken belirsiz).
   - "İptal" → `schemaDiffCancel`. `cancelled` kodu "İptal edildi." olarak gösterilir; kısmi sonuç yoktur.
   - Sekme kapanırsa ya da yeni karşılaştırma başlarsa önceki işin yanıtı ve olayları yok sayılır (kuşak sayacı).
8. **Liste:**
   - Her öğe tek kategoridedir: `problem` doluysa "Sorunlu", değilse durumu ("Yalnız kaynakta", "Yalnız hedefte", "Farklı", "Aynı"). Sayılar toplamı öğe sayısıdır.
   - Varsayılan filtre: "Aynı" dışındakiler.
   - Arama tür etiketi, tablo, ad ve imzada alt dize arar. İki taraf `toLocaleLowerCase("tr-TR")` ile katlanır.
   - Satır içeriği: seçim kutusu, tür etiketi, `tablo.ad` ya da `ad(imza)`, durum, risk rozeti (Designer `RiskBadge`), not/hata simgesi (sayı + `title`).
   - Sıra arka ucun sırasıdır.
9. **Seçim (spec §2.4, §2.8):**
   - İlk seçim `defaultSelected` olanlardır.
   - `selectable = false` öğelerin kutusu devre dışıdır; gerekçe `title` ve `aria-describedby` ile gösterilir.
   - İki toplu eylem: "Tümü" (tüm seçilebilir öğeler) ve "Görünenler" (yalnız filtredeki seçilebilirler). İkisi de seç/kaldır üç durumlu kutudur.
   - Gizlenen seçili öğeler betiğe girer. Özet: "N seçili (M görünmüyor)".
   - Yeni karşılaştırma seçimi sıfırlar.
10. **Ayrıntı:**
    - Odaklanan öğe 120 ms gecikmeyle `schemaDiffDetail` ile istenir. Sonuç `diffId` başına anahtar önbelleğinde tutulur.
    - Gösterim tek sütunlu birleşik farktır: hedef no / kaynak no / işaret / metin. `removed` kırmızı, `added` yeşil zeminli, `same` düz. Satırlar sanallaştırılır.
    - `lines === null` ise "Fark çok büyük (20.000 satır üstü); metinler yan yana gösteriliyor." bilgisi ve iki sütunlu düz `<pre>` (hedef | kaynak) gösterilir.
    - Notlar ve hata iletileri farkın üstündedir.
11. **Betik önizleme:**
    - "Betik oluştur" → `schemaDiffScript(diffId, seçili anahtarlar)`; diyalog (genişlik 880px) açılır.
    - İçerik:
      - risk rozeti;
      - ifade sayısı;
      - transaction bilgisi ("Betik BEGIN/COMMIT ile sarılı" ya da uyarı);
      - `warnings` listesi;
      - salt okunur CodeMirror (`createReadOnlyEditor`, yükseklik 60vh).
    - **Hedef bandı** (spec §2.8 "Kilit"):
      - hedef `ServerConfig` `isProductionLabel(label)` ise tehlike renginde "Hedef üretim sunucusu: ‹ad›";
      - `readOnlyLock` ise "Hedef sunucu salt okunur kilitli; Query Tool betiği çalıştırmaz.";
      - ikisi birden olabilir. Sekme ek kapı koymaz.
    - **Eylemler:**
      - "Query Tool'da aç": `deps.query.openScript({ serverId, databaseOid, database: hedefDbAdı, text: sql })`; çalıştırmaz, diyalog kapanır.
      - "Kopyala": `copyText(sql)`, bildirim "Kopyalandı.".
      - "Kaydet…": `save({ defaultPath: suggestScriptName(…), filters: [{ name: "SQL", extensions: ["sql"] }] })` → `schemaDiffSave(diffId, yol)`, bildirim "Betik kaydedildi." (yol yazılmaz).
      - "Kapat".
12. **Kayıttan düşmüş sonuç:** detay/betik/kaydet `diff_not_found` alırsa sonuç temizlenir ve üst şeritte "Sonuç artık yok (en fazla 8 karşılaştırma tutulur ya da sayfa yenilendi); yeniden karşılaştırın." görünür.
13. **Kapsam dışı:** betiğin sekmeden uygulanması (spec), sekme kalıcılığı, karşılaştırma sonucunun dışa aktarılması, öğe başına "yalnız bunu betikle" kısayolu, F5 ile yeniden karşılaştırma (ana pakete bayt eklerdi).

- **Kullanıcı kararı (2026-10-31):** ana paket tavanı **hiçbir zaman yükseltilmez**. Telafi yetmezse görev durur ve kullanıcıya sorulur.

## Global Constraints

- **Dal:** `feat/schema-diff-ui` (main'den). main'e birleştirme yalnız kullanıcı onayıyla.
- **Değişen alanlar:**
  - Ön yüz (`src/`), `docs/`, `scripts/bundle-budget.json` (yalnız tavan **indirme**).
  - Rust'a dokunulmaz. Bindingler değişmez; satır sonu gürültüsü `git checkout -- src/ipc/bindings` ile atılır.
- **Sözleşme:** komut ve alan adları spec §6 ile birebirdir (camelCase bağımsız değişkenler):

  | Sarıcı (`diff-commands.ts`) | Komut ve bağımsız değişkenler | Dönüş |
  |---|---|---|
  | `schemaDiffDatabases(serverId)` | `schema_diff_databases { serverId }` | `DiffDatabase[]` |
  | `schemaDiffStart(diffId, source, target, options, onEvent)` | `schema_diff_start { diffId, source, target, options, onEvent: Channel<DiffEvent> }` | `DiffResult` |
  | `schemaDiffCancel(diffId)` | `schema_diff_cancel { diffId }` | `null` |
  | `schemaDiffDetail(diffId, itemKey)` | `schema_diff_detail { diffId, itemKey }` | `DiffDetail` |
  | `schemaDiffScript(diffId, itemKeys)` | `schema_diff_script { diffId, itemKeys }` | `DiffScript` |
  | `schemaDiffSave(diffId, path)` | `schema_diff_save { diffId, path }` | `null` |
  | `schemaDiffClose(diffId)` | `schema_diff_close { diffId }` | `null` |

  Türler `src/ipc/bindings/`'ten (`DiffSide`, `DiffOptions`, `DiffEvent`, `DiffItem`, `DiffResult`, `DiffLine`, `DiffDetail`, `DiffScript`, `DiffDatabase`, `ItemStatus`, `ItemProblem`, `LineKind`, `SideTag`, `ObjectKind`, `Risk`). Arka uç tek doğruluk kaynağıdır: istemci kuralları yalnız görünürlük ve devre dışılık içindir.
- **Paket:**
  - Ana paket ≤ `mainMaxBytes` (şu an 127,364 B, sıfır pay), açılış kümesi ≤ `startupMaxBytes` (155,037 B). Preload tam `["solid","web"]`; açılış kümesinde yasak paket yok.
  - Task 1–4'te `bun run bundle:check` yalnız bu iki tavanın aşımı yüzünden düşebilir. Aşım rapora yazılır; başka her ihlal (preload, yasak paket, harita) görevi durdurur. Telafi Task 5'tedir. **Tavan yükseltilmez.**
  - **Tembel içe aktarma kuralları:**
    - `diff-open.ts` yalnız tür içe aktarır; çalışma zamanında yalnız `../../ipc/errors` `toPayload`'ı alır (`data-open.ts` ile aynı).
    - `DiffTabs.tsx` ve statik kapanışı yalnız `solid-js`, `src/app/strings-diff.ts`, `src/features/schema-diff/` altındaki özel modülleri ve ana paket modüllerinden aşağıdaki izinli adları içe aktarır. `src/ui/*`, `lucide-solid`, CodeMirror, `designer/*`, `inspector/*` yasaktır; bunlar `QueryTabs`'taki `import("../schema-diff/DiffTabs")`'ın `__vite__mapDeps` listesini (ana paket) büyütür.
    - `SchemaDiffView.tsx` (DiffTabs'tan `lazy`) `ui/*`, `designer/RiskBadge`, `lucide-solid`, `@tanstack/solid-virtual` kullanabilir. `ScriptDialog.tsx` (SchemaDiffView'dan `lazy`) `ui/Dialog`, `inspector/codemirror-setup`, `@tauri-apps/plugin-dialog`, `app/clipboard` kullanabilir.
    - Ana paket modüllerinden yalnız başka tembel parçaların zaten içe aktardığı adlar alınır: `toPayload`, `copyText`, `isProductionLabel`, `invoke`. Yeni ad ana paketin dışa aktarma listesini büyütür. Tek istisna `Channel`'dır (`@tauri-apps/api/core`); bedeli Task 5'te ölçülür ve telafi edilir.
    - `app/strings`, `explorer/*`, `query/*` (tür dışında) tembel parçadan içe aktarılmaz.
  - Yükleyici: `loadDiff = once(() => import("../features/schema-diff/diff-open").then((m) => setDiff(m.createSchemaDiff(lazyDeps()))))`. `prefetchIdle` listesine **eklenmez** (ilk kullanımda iner; liste girdisi ana pakete bayt ekler).
- **Gizlilik:** betik metni, nesne/şema/veritabanı adları ve yollar `console.*`'a, `localStorage`'a ya da bildirimlere yazılmaz. Bildirimler sabit metinlerdir.
- **Erişilebilirlik:**
  - Seçiciler `<label>`'lıdır; devre dışı gerekçeleri `aria-describedby` ile bağlanır.
  - İlerleme `role="progressbar"` (`aria-valuenow/max`; belirsizde `aria-valuetext`).
  - Liste `role="listbox"` + `aria-multiselectable="true"`; satırlar `role="option"` + `aria-selected` (odak) ve içinde etiketli kutu.
  - ↑/↓ odağı taşır, Boşluk seçimi değiştirir, Home/End uçlara gider.
  - Hata `role="alert"`, bayat/sonuç şeridi `role="status"`.
  - Satır farkında renk tek başına bilgi değildir (işaret sütunu `+`/`−`).
- **vitest:** ortam `node`. Mantık `diff-model.ts` ve `diff-controller.ts`'tedir ve orada test edilir. GUI ajanlarca test edilemez; elle kontrol maddeleri `docs/smoke-test.md` "## Şema karşılaştırma (Plan 30)" altına `[—]` olarak eklenir (Task 5).
- **Diğer:**
  - Yorumlar ve metinler Türkçe, doğru Türkçe karakterlerle. PostgreSQL terimleri (`BEGIN`, `OWNED BY`) olduğu gibi kalır.
  - `graphify-out/` commit edilmez.
  - Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Doğrulama (her görev sonu):** `~/.bun/bin/bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check` (Task 1–4'te yalnız tavan aşımı raporlanır).

## Review Focus

- **Yarışlar ve bayat yanıtlar:**
  - Peş peşe iki "Karşılaştır"; çalışırken sekmeyi kapatma; iptal ile sonucun aynı anda gelmesi.
  - Hızlı ↑/↓ ile ayrıntı isteklerinin sıra dışı dönmesi.
  - Sunucu seçimi değişirken eski veritabanı listesinin gelmesi.
  - Beklenen:
    - yalnız son kuşağın yanıtı yazılır;
    - kapanan sekmenin işi iptal edilir ve kaydı kapatılır (`schemaDiffClose` bir kez);
    - iş bittiyse iptal düğmesi sonucu değiştirmez;
    - ayrıntı yalnız odaktaki anahtar için gösterilir.
  - Test: Task 3.
- **Seçim semantiği:**
  - Filtre değişince gizlenen seçili öğe betikte kalır mı? "Görünenler" kutusu gizlileri etkiliyor mu? Seçilemeyen öğe toplu seçimde kutusu işaretli görünüyor mu?
  - Beklenen: gizli seçimler korunur ve özetle bildirilir; toplu eylemler yalnız `selectable` öğelere dokunur; üç durumlu kutu kısmi seçimde `indeterminate`.
  - Test: Task 2 `setMany`/`selectionSummary`/`selectAllState`.
- **Türkçe ve ASCII dışı adlar:**
  - `Şube` şeması, `İLÇE` tablosu, `"a b"` adı.
  - Arama `ilçe` → `İLÇE`, `şube` → `Şube` bulunmalı.
  - Önerilen dosya adı Windows'ta geçersiz karakterleri `_` yapmalı.
  - Başlıkta adlar olduğu gibi görünmeli.
  - Test: Task 2 `matches`/`suggestScriptName`/`tabTitle`.
- **Hedef bilgisi ve aktarım:**
  - Hedef sunucu üretim ya da kilitli; hedef sunucu kaydı silinmiş; büyük betik (birkaç MB); kayıt diyaloğunda vazgeçme; `write_failed`.
  - Beklenen:
    - bant doğru hedefe göre çizilir (kaynağa göre değil);
    - "Query Tool'da aç" hedef `serverId`/`databaseOid` ve hedef veritabanı adıyla çağrılır;
    - vazgeçmede komut çağrılmaz;
    - hata iletisi yol içermez.
  - Test: Task 2 `targetBanner`, Task 3 betik akışı.
- **Paket bütçesi:**
  - Menü, sekme kaydı ve QueryTabs bağlantısı ana pakete bayt ekler; tembel parçanın yanlış içe aktarması (ör. `DiffTabs.tsx`'te `ui/Button`) `__vite__mapDeps`'i sessizce büyütür.
  - Beklenen: Task 5 sonunda ana paket ≤ 127,364 B ve açılış kümesi ≤ 155,037 B; `rg` taramaları içe aktarma kurallarını doğrular.
  - Test: Task 1 ve Task 5 ölçümleri.

---

### Task 1: Ana paket girişleri ve tembel sekme iskeleti

**Dosyalar:**
- Değiştir: `src/features/explorer/actions.ts` (+ `actions.test.ts`), `src/app/strings.ts` (`t.menu.schemaDiff`), `src/features/query/tabs-store.ts` (+ `tabs-store.test.ts`), `src/features/query/QueryTabs.tsx`, `src/app/App.tsx`.
- Oluştur: `src/features/schema-diff/diff-open.ts`, `src/features/schema-diff/DiffTabs.tsx` (iskelet), `src/features/schema-diff/diff-opener.ts` (iskelet), `src/features/schema-diff/diff-controller.ts` (yalnız `DiffTabCtl` iskelet türü), `src/app/strings-diff.ts` (yalnız `tsd.title`, `tsd.preparing`).

**Arayüzler — Üretir:**

```ts
// actions.ts
type ActionId = … | "schemaDiff";
interface ActionDeps { …; /** "Şema karşılaştır…" (Plan 30; tembel sekme). */ openSchemaDiff?(id: string): void }
// tabs-store.ts
export interface DiffTab { id: string; title: string }
interface TabsState { …; /** Şema karşılaştırma sekmeleri; veri sekmelerinden sonra çizilir. */ diffs: DiffTab[] }
export const isDiffTabId = (id: string): boolean => id[0] === "s";
export function openSchemaDiff(state: TabsState, input: { title: string }): { state: TabsState; id: string };   // her çağrı yeni sekme, etkinleşir
// createTabsStore(): openSchemaDiff(input: { title: string }): string
// diff-open.ts
export interface DiffDeps { api: Api; tree: () => TreeState; tabs: TabsStore; connect: (serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void; query: Pick<QueryController, "servers" | "openScript"> }
export type DiffController = DiffTabCtl;          // `import type` (diff-controller.ts; Task 3 genişletir)
export function createSchemaDiff(deps: DiffDeps): { ctl: DiffController; open: (nodeId: string) => Promise<void> };
export type SchemaDiff = ReturnType<typeof createSchemaDiff>;
// DiffTabs.tsx
export { startDiff } from "./diff-opener";
export function DiffTabHeads(props: { diff: DiffController; tabs: DiffTab[]; Head: TabHeadComponent }): JSX.Element;
export function DiffPanels(props: { diff: DiffController; tabs: DiffTab[]; activeId: string }): JSX.Element;
// diff-opener.ts (Task 3'te gerçeği)
export function startDiff(deps: DiffDeps): { ctl: DiffController; open: (nodeId: string) => Promise<void> };
```
- **Bu görevde `DiffTabCtl` iskeleti:**
  ```ts
  { model(tabId): null; ids(): string[]; close(tabId): void }
  ```
  `startDiff().open(nodeId)` yalnız `deps.tabs.openSchemaDiff({ title: tsd.title.initial })` çağırır. Panel `tsd.preparing` ("Hazırlanıyor…") gösterir.
- **`QueryTabs.tsx`:**
  - Prop `diff?: () => DiffController | null`.
  - Tembel `const diffTabs = () => import("../schema-diff/DiffTabs")`; `DiffTabHeads` ve `DiffPanels` için `lazy`.
  - Başlıklar ve paneller veri sekmelerinden sonra `<Show when={props.diff?.()}>` + `Suspense` içinde çizilir.
  - `requestClose`'a dal: `else if (isDiffTabId(id)) closeDiff(id)`, burada `closeDiff = (id) => { props.diff?.()?.close(id); ctl.tabs.close(id); }`.
- **`tabs-store.ts`:**
  - `initialTabs`, `taken`, `closeTab` (sıra: sorgu → dashboard → designer → veri → fark) ve `renameTab` `diffs` listesini kapsar.
  - Kimlik `s<n>` (`nextId` paylaşılır).
- **`App.tsx`:**
  - Sinyal `diff`.
  - `loadDiff` (Global Constraints).
  - `actionDeps.openSchemaDiff = (id) => void loadDiff().then((d) => d.open(id), loadFailed)`.
  - `<QueryTabs … diff={() => diff()?.ctl ?? null} />`.
- **`diff-open.ts`:** `data-open.ts` desenini birebir izler. `import("./DiffTabs")` açılışla paralel başlar; yüklenene dek `ctl` güvenli varsayılanlar döner.

- [ ] **Adım 1: Başarısız testler.**
  - `actions.test.ts`:
    - `schemaDiff` şema ve veritabanı düğümünde var;
    - `flags.system` şemada, tabloda, sunucuda ve koleksiyonda yok;
    - sıra `…backup, restore, schemaDiff, dropObject` (veritabanı) ve `…backup, schemaDiff, dropObject` (şema);
    - `groupOf("schemaDiff") === 3`;
    - kilitli sunucuda `disabledReason("schemaDiff") === null`;
    - `executeAction(…, "schemaDiff", id)` → `openSchemaDiff(id)`;
    - `t.menu.schemaDiff === "Şema karşılaştır…"`.
  - `tabs-store.test.ts`:
    - `openSchemaDiff` iki çağrıda iki sekme (`s1`, `s2`) açar ve sonuncusu etkindir;
    - kimlik sorgu sekmeleriyle çakışmaz (`nextId` paylaşımı);
    - `isDiffTabId("s3") === true`, `isDiffTabId("q1") === false`;
    - `closeTab` etkin fark sekmesini kapatınca soldaki (veri ya da designer) sekme etkinleşir;
    - `renameTab` fark sekmesini yeniden adlandırır;
    - `initialTabs().diffs` boştur.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `bunx vitest run src/features/explorer/actions.test.ts src/features/query/tabs-store.test.ts` → FAIL.
- [ ] **Adım 3: Uygula.** Kararlar 1–2; yorumlar mevcut üslupta. `strings-diff.ts`:

  ```ts
  export const tsd = {
    title: { initial: "Şema karşılaştırma" },
    preparing: "Hazırlanıyor…",
  } as const;
  ```
- [ ] **Adım 4: Doğrula.**
  - Dört doğrulama komutu.
  - `bundle:check` ana paket ve açılış kümesi baytları, tavanla farkları ve `__vite__mapDeps` girdi sayısı rapora yazılır (beklenen: +1 girdi `DiffTabs`, +1 `diff-open`).
  - `bun run build` çıktısında `diff-open-*.js` ve `DiffTabs-*.js` parçaları görünür.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src/features/explorer/actions.ts src/features/explorer/actions.test.ts src/app/strings.ts src/app/strings-diff.ts src/features/query/tabs-store.ts src/features/query/tabs-store.test.ts src/features/query/QueryTabs.tsx src/app/App.tsx src/features/schema-diff/
  git commit -m "feat(diff): Explorer \"Şema karşılaştır…\" girişi ve tembel sekme iskeleti"
  ```

### Task 2: Komut sarıcıları, saf model ve metinler

**Dosyalar:** Oluştur `src/features/schema-diff/diff-commands.ts`, `src/features/schema-diff/diff-model.ts`, `src/features/schema-diff/diff-model.test.ts`; genişlet `src/app/strings-diff.ts`.

**Arayüzler — Üretir:**

```ts
// diff-commands.ts — data-commands.ts'teki yerel `call` deseni; Channel @tauri-apps/api/core'dan
export const schemaDiffDatabases = (serverId: string) => call<DiffDatabase[]>("schema_diff_databases", { serverId });
export const schemaDiffStart = (diffId: string, source: DiffSide, target: DiffSide, options: DiffOptions, onEvent: (e: DiffEvent) => void) =>
  call<DiffResult>("schema_diff_start", { diffId, source, target, options, onEvent: channel(onEvent) });
export const schemaDiffCancel = (diffId: string) => call<null>("schema_diff_cancel", { diffId });
export const schemaDiffDetail = (diffId: string, itemKey: string) => call<DiffDetail>("schema_diff_detail", { diffId, itemKey });
export const schemaDiffScript = (diffId: string, itemKeys: string[]) => call<DiffScript>("schema_diff_script", { diffId, itemKeys });
export const schemaDiffSave = (diffId: string, path: string) => call<null>("schema_diff_save", { diffId, path });
export const schemaDiffClose = (diffId: string) => call<null>("schema_diff_close", { diffId });
export type DiffApi = { schemaDiffDatabases: typeof schemaDiffDatabases; …(yedisi) };
export const diffApi: DiffApi;
// diff-model.ts
export type Category = "onlySource" | "onlyTarget" | "different" | "same" | "problem";
export const CATEGORIES: readonly Category[];                         // bu sırayla gösterilir
export function categoryOf(i: DiffItem): Category;
export function defaultFilters(): Record<Category, boolean>;          // same: false, diğerleri true
export function counts(items: readonly DiffItem[]): Record<Category, number>;
export function foldSearch(q: string): string;                        // trim + toLocaleLowerCase("tr-TR")
export function matches(i: DiffItem, folded: string): boolean;        // boş sorgu → true; kindLabel + table + name + signature
export function visibleItems(items: readonly DiffItem[], filters: Record<Category, boolean>, folded: string): DiffItem[];
export function initialSelection(items: readonly DiffItem[]): ReadonlySet<string>;
export function toggleKey(sel: ReadonlySet<string>, item: DiffItem): ReadonlySet<string>;         // seçilemez → aynı küme
export function setMany(sel: ReadonlySet<string>, items: readonly DiffItem[], on: boolean): ReadonlySet<string>;   // yalnız selectable
export function selectAllState(sel: ReadonlySet<string>, items: readonly DiffItem[]): "none" | "some" | "all";   // seçilebilirler üzerinden
export function selectionSummary(sel: ReadonlySet<string>, visible: readonly DiffItem[]): { selected: number; hidden: number };
export function selectedKeys(sel: ReadonlySet<string>, items: readonly DiffItem[]): string[];    // sonuç sırasıyla; artık olmayan anahtar atılır
export interface SideSel { serverId: string | null; databaseOid: number | null; schema: string | null }
export function emptySide(): SideSel;
export function sideComplete(s: SideSel): s is { serverId: string; databaseOid: number; schema: string };
export function sameSide(a: SideSel, b: SideSel): boolean;            // üçü de dolu ve eşit
export function swapSides<T>(a: T, b: T): [T, T];
export type Blocked = keyof typeof tsd.blocked;                       // "incomplete" | "same" | "running" | "stale" | "noSelection" | "noResult"
export function compareBlocked(src: SideSel, tgt: SideSel, running: boolean): Blocked | null;   // sıra: running > incomplete > same
export function scriptBlocked(i: { running: boolean; hasResult: boolean; stale: boolean; selected: number }): Blocked | null;   // sıra: running > noResult > stale > noSelection
export function entryFromTarget(t: DdlTarget, label: string): SideSel | null;   // Kararlar 4; başka nesne türü → null
export function tabTitle(src: { db: string; schema: string }, tgt: { db: string; schema: string }): string;   // "a.s ↔ b.t"
export function suggestScriptName(src: { db: string; schema: string }, tgt: { db: string; schema: string }, now: Date): string;
export function progressView(p: { loaded: number; total: number } | null): { percent: number | null; text: string };
export function kindLabel(kind: ObjectKind): string;                  // tsd.kind
export function itemLabel(i: DiffItem): string;                       // "tablo.ad" | "ad(imza)" | "ad"
export function problemText(i: DiffItem): string | null;              // tsd.problem[problem]
export function errorText(p: ErrorPayload): string;                   // tsd.errors[code] ?? message (+ hint)
export function targetBanner(cfg: ServerConfig | null): { production: boolean; locked: boolean; name: string | null };
export function gutter(l: DiffLine): { target: string; source: string; sign: "+" | "−" | " " };
```

- **`strings-diff.ts` (`tsd`) — sabit metinler:**
  - **Başlık ve bölümler:** `title.initial` "Şema karşılaştırma"; `source` "Kaynak"; `target` "Hedef"; `swap` "Kaynak ile hedefi değiştir"; `server` "Sunucu"; `database` "Veritabanı"; `schema` "Şema"; `choose` "Seçin…"; `loading` "Yükleniyor…".
  - **Seçenekler:** `options.ownership` "Sahiplik ve yetkiler"; `options.tablespace` "Tablespace"; `options.comments` "Yorumlar".
  - **Eylemler:** `compare` "Karşılaştır"; `cancel` "İptal"; `cancelling` "İptal ediliyor…"; `buildScript` "Betik oluştur"; `openInQuery` "Query Tool'da aç"; `copy` "Kopyala"; `save` "Kaydet…"; `close` "Kapat".
  - **Kategoriler:** `onlySource` "Yalnız kaynakta"; `onlyTarget` "Yalnız hedefte"; `different` "Farklı"; `same` "Aynı"; `problem` "Sorunlu".
  - **Sorunlar:** `problem.manual` "Elle gerekiyor"; `problem.load_failed` "Yüklenemedi"; `problem.build_failed` "Üretilemedi".
  - **Tür etiketleri:** `kind.table` "Tablo"; `view` "Görünüm"; `materialized_view` "Materialized view"; `sequence` "Sequence"; `index` "İndeks"; `trigger` "Trigger"; `function` "Fonksiyon"; `procedure` "Prosedür"; `type` "Tip"; `domain` "Domain". Diğer türler için `ObjectKind` metni olduğu gibi.
  - **Kapı gerekçeleri (`blocked`):** `incomplete` "Kaynak ve hedefte sunucu, veritabanı ve şema seçin."; `same` "Kaynak ve hedef aynı şema."; `running` "Karşılaştırma sürüyor."; `stale` "Seçimler değişti; yeniden karşılaştırın."; `noSelection` "Betik için en az bir öğe seçin."; `noResult` "Önce karşılaştırın.".
  - **Durum şeritleri:**
    - `cancelled` "İptal edildi.";
    - `gone` "Sonuç artık yok (en fazla 8 karşılaştırma tutulur ya da sayfa yenilendi); yeniden karşılaştırın.";
    - `summary(n, ms)` → `"${n} öğe · ${saniye} sn"` (saniye bir ondalıklı, virgüllü);
    - `selection(s, h)` → `h > 0 ? "${s} seçili (${h} görünmüyor)" : "${s} seçili"`;
    - `selectAll` "Tümü"; `selectVisible` "Görünenler";
    - `search` "Tür ya da ad ara…";
    - `notSelectable` "Bu öğe betiğe eklenemez: ";
    - `empty` "Fark yok." (filtre sonrası boşsa "Filtreye uyan öğe yok.").
  - **Ayrıntı:** `detail.pick` "Ayrıntı için listeden bir öğe seçin."; `detail.tooLarge` "Fark çok büyük (20.000 satır üstü); metinler yan yana gösteriliyor."; `detail.notes` "Notlar"; `detail.errors` "Hatalar"; `detail.targetCol` "Hedef"; `detail.sourceCol` "Kaynak".
  - **Betik diyaloğu:**
    - `script.title` "Betik önizleme";
    - `script.statements(n)` → `"${n} ifade"`;
    - `script.tx` "Betik BEGIN/COMMIT ile sarılı.";
    - `script.noTx` "Betik transaction'sız; hata olursa kısmen uygulanmış kalır.";
    - `script.production(name)` → `"Hedef üretim sunucusu: ${name}"`;
    - `script.locked` "Hedef sunucu salt okunur kilitli; Query Tool betiği çalıştırmaz.";
    - `script.copied` "Kopyalandı.";
    - `script.saved` "Betik kaydedildi.";
    - `script.filterName` "SQL".
  - **Hata kodları (`errors`):**
    - `same_schema` = `blocked.same`;
    - `schema_not_found` "Seçilen şema bulunamadı; listeyi yenileyin.";
    - `system_schema` "Sistem şemaları karşılaştırılamaz.";
    - `cancelled` = `cancelled`;
    - `busy` "Bu karşılaştırma zaten çalışıyor.";
    - `diff_not_found` = `gone`;
    - `item_not_found` "Seçili öğelerden biri sonuçta yok; yeniden karşılaştırın.";
    - `not_selectable` "Seçili öğelerden biri betiğe eklenemez.";
    - `empty_selection` = `blocked.noSelection`;
    - `no_script` "Önce betiği oluşturun.";
    - `invalid_path` "Kayıt yolu geçersiz.";
    - `write_failed` "Dosya yazılamadı.";
    - `connectFailed` "Sunucuya bağlanılamadı." (istemci tarafı; kod değil);
    - `password_required` için arka uç iletisi + "Sunucu bağlantısını kesip yeniden bağlanın; parola istenir.".
- **`suggestScriptName`:** `${srcDb}_${srcSchema}__${tgtDb}_${tgtSchema}_${YYYYMMDD}_${HHmm}.sql`. `<>:"/\|?*` ve denetim karakterleri `_` olur; toplam ad 120 karakterde kesilir (uzantı korunur).

- [ ] **Adım 1: Başarısız testler (`diff-model.test.ts`).**
  - `categoryOf`: `problem` doluyken durumdan bağımsız `problem`; durumların camelCase eşlemesi.
  - `counts` toplamı öğe sayısına eşit. `defaultFilters().same === false`.
  - `matches`:
    - `ilçe` ↔ `İLÇE`, `şube` ↔ `Şube`, `ı` ↔ `I` (tr katlama);
    - tablo adında ve imzada (`integer`) arama;
    - tür etiketinde (`indeks`) arama;
    - boş sorgu her şeyle eşleşir.
  - Seçim:
    - `initialSelection` yalnız `defaultSelected`;
    - `toggleKey` seçilemez öğede değişmez;
    - `setMany(on)` seçilemezleri eklemez;
    - `setMany(off)` yalnız verilen öğeleri çıkarır (gizlilere dokunmaz);
    - `selectAllState` none/some/all (seçilemezler sayılmaz);
    - `selectionSummary`: 3 seçili, 1'i filtre dışı → `{ selected: 3, hidden: 1 }`;
    - `selectedKeys` sonuç sırasını korur ve sonuçta olmayan anahtarı atar.
  - Seçici:
    - `sameSide`: eksik tarafta `false`;
    - `compareBlocked` sıra tablosu (`running` > `incomplete` > `same`);
    - `scriptBlocked` sıra tablosu;
    - `swapSides`.
  - `entryFromTarget`:
    - şema hedefi + etiket `Şube` → `{serverId, databaseOid: 7, schema: "Şube"}`;
    - veritabanı hedefi (`databaseOid: null`, `object: {kind: "database", oid: 9}`) → `{databaseOid: 9, schema: null}`;
    - tablo hedefi → `null`.
  - `tabTitle({db:"a", schema:"s"}, {db:"b", schema:"t"}) === "a.s ↔ b.t"`.
  - `suggestScriptName({db:"satış", schema:'a"b'}, {db:"x", schema:"y/z"}, new Date(2026, 9, 31, 14, 5)) === 'satış_a_b__x_y_z_20261031_1405.sql'`; 300 karakterlik adda uzunluk ≤ 120 ve `.sql` ile biter.
  - `progressView`: `null` → `{percent: null, text: ""}`; `{0,0}` → belirsiz; `{5,10}` → 50, `"5 / 10"`; `{12,10}` → 100.
  - `itemLabel`: indeks `t.i`; fonksiyon `f(integer, text)`; tablo `t`.
  - `problemText` üç sorun ve `null`.
  - `errorText`: tablodaki her kod; bilinmeyen kod → `message`; `password_required` ekli öneri.
  - `targetBanner`:
    - `label: "Üretim"` → `production`;
    - `readOnlyLock: true` → `locked`;
    - `null` → ikisi `false`, `name: null`.
  - `gutter`:
    - `{kind:"removed", targetNo: 4}` → `{target:"4", source:"", sign:"−"}`;
    - `added` → `+`;
    - `same` iki numara.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `bunx vitest run src/features/schema-diff/diff-model.test.ts` → FAIL (modül yok).
- [ ] **Adım 3: Uygula.** `diff-model.ts` yalnız `strings-diff` ve tür içe aktarır. `diff-commands.ts` `invoke`/`Channel`'ı `@tauri-apps/api/core`'dan, `toPayload`'ı `../../ipc/errors`'tan alır (hata `Object.assign(new Error(p.message), p)`; `data-commands.ts` yorumuyla aynı gerekçe).
- [ ] **Adım 4: Doğrula.** Odak testi PASS; dört doğrulama komutu.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src/features/schema-diff/diff-commands.ts src/features/schema-diff/diff-model.ts src/features/schema-diff/diff-model.test.ts src/app/strings-diff.ts
  git commit -m "feat(diff): şema karşılaştırma komut sarıcıları, saf model ve metinler"
  ```

### Task 3: Denetleyici

**Dosyalar:** Task 1 iskeletleri `src/features/schema-diff/diff-controller.ts` ve `diff-opener.ts`'i gerçeğiyle değiştir; `diff-open.ts` yüzünü genişlet; oluştur `src/features/schema-diff/diff-controller.test.ts`.

**Arayüzler:**
- Tüketir: Task 2'nin tamamı, Task 1 `DiffDeps`/`DiffTab`.
- Üretir:

```ts
export const DETAIL_DELAY_MS = 120;
export type Picker = { save(o: { defaultPath: string; filters: { name: string; extensions: string[] }[] }): Promise<string | null> };
export interface DiffControllerDeps {
  api: DiffApi & Pick<Api, "ddlTargetOf" | "ddlLookup">;
  servers: () => ServerConfig[];
  nodeLabel: (nodeId: string) => string | null;         // deps.tree().nodes.get(nodeId)?.label ?? null
  connect: (serverId: string) => Promise<boolean>;
  tabs: Pick<TabsStore, "openSchemaDiff" | "rename">;   // rename: mevcut renameTab sarıcısı
  openScript: QueryController["openScript"];
  copy: (text: string) => Promise<void>;
  notify: (text: string, kind: ToastKind) => void;
  pick: () => Promise<Picker>;                           // ScriptDialog parçasıyla tembel yüklenir
  newId(): string; now(): Date;
  setTimer(fn: () => void, ms: number): unknown; clearTimer(h: unknown): void;
}
export interface SideState extends SideSel {
  databases: DiffDatabase[] | null; databasesError: string | null;
  schemas: string[] | null; schemasError: string | null; loadingLists: boolean;
}
export type Phase = "idle" | "running" | "done";
export interface DiffModel {
  tabId: string; source: SideState; target: SideState; options: DiffOptions;
  phase: Phase; cancelRequested: boolean; diffId: string | null;
  progress: { source: { loaded: number; total: number } | null; target: { loaded: number; total: number } | null };
  result: DiffResult | null; stale: boolean; banner: string | null; error: string | null;   // banner: cancelled | gone; error: role=alert
  filters: Record<Category, boolean>; search: string; selected: ReadonlySet<string>; focused: string | null;
  detail: { key: string; data: DiffDetail | null; error: string | null } | null;
  script: { data: DiffScript | null; error: string | null; busy: boolean } | null;    // null = diyalog kapalı
}
export interface DiffTabCtl {
  model(tabId: string): DiffModel | null; ids(): string[];
  open(nodeId: string): Promise<void>; close(tabId: string): void;
  setServer(tabId: string, side: SideTag, serverId: string | null): Promise<void>;
  setDatabase(tabId: string, side: SideTag, oid: number | null): Promise<void>;
  setSchema(tabId: string, side: SideTag, schema: string | null): void;
  swap(tabId: string): void;
  setOption<K extends keyof DiffOptions>(tabId: string, key: K, value: DiffOptions[K]): void;
  compare(tabId: string): Promise<void>; cancel(tabId: string): void;
  setFilter(tabId: string, c: Category, on: boolean): void; setSearch(tabId: string, q: string): void;
  toggle(tabId: string, key: string): void; selectAll(tabId: string, on: boolean): void; selectVisible(tabId: string, on: boolean): void;
  focus(tabId: string, key: string | null): void; moveFocus(tabId: string, delta: number | "home" | "end"): void;
  buildScript(tabId: string): Promise<void>; closeScript(tabId: string): void;
  openInQueryTool(tabId: string): Promise<void>; copyScript(tabId: string): Promise<void>; saveScript(tabId: string): Promise<void>;
}
export function createDiffController(deps: DiffControllerDeps): DiffTabCtl;
// diff-opener.ts
export function startDiff(deps: DiffDeps): { ctl: DiffTabCtl; open: (nodeId: string) => Promise<void> };
// deps.api = { ...diffApi, ddlTargetOf, ddlLookup }, copy = copyText (app/clipboard),
// pick = () => import("./ScriptDialog").then(m => m.picker), newId = crypto.randomUUID
```
- **Durum ve kuşaklar:**
  - Sekme başına Solid store kullanılır (`createStore`; `model()` tepkiseldir).
  - Üç bağımsız kuşak sayacı vardır: seçici listeleri (taraf başına), karşılaştırma ve ayrıntı. Eski kuşağın yanıtı, olayı ve hatası durum yazmaz.
- **Açılış:**
  - `open(nodeId)` → `tabs.openSchemaDiff({ title: tsd.title.initial })` → model.
  - Ardından `ddlTargetOf(nodeId)` → `entryFromTarget(hedef, nodeLabel(nodeId) ?? "")`. Kaynak ve hedef aynı sunucu/veritabanıyla doldurulur, hedef şeması `null` olur.
  - Sonra `connect` → veritabanı listesi → kaynak şema listesi ve hedef şema listesi.
  - Hata `databasesError`/`schemasError`'a `errorText` ile yazılır; sekme açık kalır.
- **Seçim değişiklikleri:**
  - `setServer`: `connect(serverId)`; `false` ise `databasesError = tsd.errors.connectFailed` ("Sunucuya bağlanılamadı."). Veritabanı ve şema sıfırlanır.
  - `setDatabase`: şema sıfırlanır ve şema listesi yüklenir.
  - `swap`: iki `SideState` yer değiştirir.
  - Sonuç varken her seçici ya da seçenek değişikliği `stale = true` yapar.
- **`compare`:**
  1. `compareBlocked` yeniden denetlenir.
  2. Eski `diffId` kapatılır (`schemaDiffClose`, hata yutulur).
  3. `diffId = newId()`, `phase = "running"`; ilerleme, sonuç, seçim, ayrıntı ve şeritler sıfırlanır.
  4. `schemaDiffStart(…)` olayları `progress`'e yazar.
  5. **Başarıda:** `phase = "done"`, `result`, `selected = initialSelection`, `stale = false`, `focused` = ilk görünen öğe. Ardından `tabs.rename(tabId, tabTitle(…))`; veritabanı adları `databases` listesinden alınır.
  6. **`cancelled`:** `phase = "idle"`, `banner = tsd.cancelled`, `diffId = null`.
  7. **Diğer hata:** `phase = "idle"`, `error = errorText(p)`.
- **`cancel`:** `cancelRequested = true` + `schemaDiffCancel(diffId)` (hata yutulur). Sonuç arka uçtan beklenir; iş başarıyla bittiyse sonuç gösterilir.
- **`focus`:**
  - `focused = key`; önbellekte varsa hemen gösterilir.
  - Yoksa `DETAIL_DELAY_MS` sonra `schemaDiffDetail`; yalnız `focused` hâlâ aynıysa ve kuşak güncelse yazılır.
  - Önbellek `diffId` değişince temizlenir.
- **`moveFocus`** görünen öğeler üzerinde çalışır (`visibleItems`).
- **`buildScript`:**
  - `scriptBlocked` denetlenir.
  - `script = { busy: true }` → `schemaDiffScript(diffId, selectedKeys(…))` → `data`. Hata `script.error`'a yazılır.
- **`openInQueryTool`:**
  - `openScript({ serverId: target.serverId, databaseOid: target.databaseOid, database: hedefDbAdı, text: script.data.sql })`.
  - Ardından `closeScript`.
- **`copyScript`:** `copy(sql)` → bildirim `tsd.script.copied`.
- **`saveScript`:**
  - `(await pick()).save({ defaultPath: suggestScriptName(…, now()), filters: [{ name: tsd.script.filterName, extensions: ["sql"] }] })`.
  - `null` → hiçbir şey çağrılmaz.
  - Aksi hâlde `schemaDiffSave(diffId, path)` → bildirim `tsd.script.saved`. Hata `script.error`'a yazılır.
- **`diff_not_found`** (detay/betik/kaydet): `result = null`, `diffId = null`, `phase = "idle"`, `banner = tsd.gone`, `script = null`.
- **`close(tabId)`:** çalışıyorsa `schemaDiffCancel`, `diffId` varsa `schemaDiffClose` (bir kez), zamanlayıcılar temizlenir, model silinir.

- [ ] **Adım 1: Başarısız testler (`diff-controller.test.ts`; sahte api, sahte zamanlayıcı, `createRoot`).**
  - **Açılış:**
    - şema düğümü → kaynak `{s1, 7, "Şube"}`, hedef `{s1, 7, null}`;
    - veritabanı ve şema listeleri yüklenir;
    - `connect` false → `databasesError`, sekme açık;
    - `ddlTargetOf` hatası → `error`.
  - **Seçiciler:**
    - sunucu değişince veritabanı ve şema sıfırlanır;
    - eski sunucunun geç gelen veritabanı listesi yazılmaz;
    - `swap` tarafları değiştirir;
    - sonuç varken `setOption` → `stale`;
    - `compareBlocked === "same"` iken `compare` çağrı yapmaz.
  - **Karşılaştırma:**
    - olay sırası `progress(source 0/3) → progress(target 3/3) → progress(source 3/3)` → `progress` doğru;
    - sonuçta `selected === initialSelection`;
    - sekme başlığı yeniden adlandırılır;
    - ikinci `compare` önce eski `diffId`'yi kapatır;
    - ilk işin geç gelen sonucu ve olayı yazılmaz.
  - **İptal:**
    - `cancel` → `schemaDiffCancel(diffId)`;
    - `cancelled` hata kodu → `banner === "İptal edildi."`, `result === null`;
    - iptal isteğinden sonra başarı gelirse sonuç gösterilir.
  - **Ayrıntı:**
    - art arda üç `focus` → tek `schemaDiffDetail` (son anahtar);
    - sıra dışı dönen eski yanıt yazılmaz;
    - önbellekteki anahtar ikinci kez istenmez;
    - `moveFocus(+1)` görünen öğelerde ilerler ve filtre dışını atlar.
  - **Betik:**
    - `scriptBlocked` (`noSelection`, `stale`) iken çağrı yok;
    - gizli seçili anahtar istekte var;
    - `openInQueryTool` hedef `serverId`/`databaseOid`/veritabanı adıyla çağrılır (kaynakla değil) ve diyalog kapanır;
    - `saveScript`: vazgeç → `schemaDiffSave` çağrılmaz; yol → çağrılır ve bildirim metninde yol yok;
    - `write_failed` → `script.error === "Dosya yazılamadı."`.
  - **`diff_not_found`:** ayrıntıda → `banner === tsd.gone`, `result === null`.
  - **Kapatma:**
    - çalışırken `close` → `schemaDiffCancel` + `schemaDiffClose` birer kez;
    - sonra gelen sonuç ya da olay hata vermez ve model yoktur.
  - **Gizlilik:** testler boyunca `console.*` çağrılmaz (`vi.spyOn(console, …)`).
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `bunx vitest run src/features/schema-diff/diff-controller.test.ts` → FAIL.
- [ ] **Adım 3: Uygula.** `diff-opener.ts` gerçeği kurar. `diff-open.ts`'teki `ctl` yüzü yeni `DiffTabCtl` yöntemlerinin hepsini güvenli varsayılanlarla devreder (parça yüklenmeden çağrılırsa `model` → `null`, `ids` → `[]`, eylemler → yok ya da `Promise.resolve()`); `createSchemaDiff` ve `open` imzaları değişmez.
- [ ] **Adım 4: Doğrula.** Odak testi PASS; dört doğrulama komutu.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src/features/schema-diff/diff-controller.ts src/features/schema-diff/diff-controller.test.ts src/features/schema-diff/diff-opener.ts src/features/schema-diff/diff-open.ts
  git commit -m "feat(diff): şema karşılaştırma denetleyicisi"
  ```

### Task 4: Görünümler

**Dosyalar:**
- Oluştur: `src/features/schema-diff/SchemaDiffView.tsx`, `SidePicker.tsx`, `DiffList.tsx`, `LineDiffView.tsx`, `ScriptDialog.tsx`, `diff-view-model.ts` (+ `diff-view-model.test.ts`).
- Task 1 iskeleti `DiffTabs.tsx`'i gerçeğiyle değiştir.

**Arayüzler:**
- Tüketir: Task 3 `DiffTabCtl`/`DiffModel`/`Picker`, Task 2 model işlevleri ve `tsd`.
- Üretir:

```ts
// DiffTabs.tsx — imzalar Task 1 ile aynı
// SchemaDiffView.tsx
export default function SchemaDiffView(props: { diff: DiffController; tabId: string }): JSX.Element;
// ScriptDialog.tsx
export const picker: Picker;    // `save` → @tauri-apps/plugin-dialog `save`; dizi dönüşü yoktur
export default function ScriptDialog(props: { diff: DiffController; tabId: string }): JSX.Element;
// diff-view-model.ts (saf)
export const ROW_HEIGHT = 26;
export const LINE_HEIGHT = 18;
export function rowAria(i: DiffItem, selected: boolean, focused: boolean): { label: string; selected: boolean };   // "Tablo t, Farklı, Veri kaybı olabilir, seçili"
export function riskTone(r: Risk): "muted" | "warn" | "danger";
export function lineClass(k: LineKind): string;   // same "", added "bg-[color-mix(in_srgb,var(--srv-green)_14%,transparent)]", removed "bg-[color-mix(in_srgb,var(--danger)_14%,transparent)]"
export function noteBadge(i: DiffItem): { count: number; tone: "info" | "error"; title: string } | null;   // errors önce
```
- **`DiffTabs.tsx`:**
  - `DiffTabHeads` her `DiffTab` için `Head`'i başlıkla çizer (`serverId: null`).
  - `DiffPanels` etkin sekmede `const SchemaDiffView = lazy(() => import("./SchemaDiffView"))`'i `Suspense fallback={tsd.preparing}` ile çizer. Gizli paneller `invisible` sınıfıyla saklanır (QueryTabs deseni).
  - Statik içe aktarma kuralı Global Constraints'teki gibidir.
- **`SchemaDiffView.tsx`** (yerleşim: üst bölüm sabit, altında yatay bölünmüş liste | ayrıntı; bölme oranı sabit %45 / %55):
  - **Üst bölüm:**
    - iki `SidePicker` (Kaynak, Hedef) ve aralarında "↔" düğmesi (`aria-label = tsd.swap`);
    - seçenek kutuları;
    - "Karşılaştır" (kapı kapalıyken gerekçe `title`/`aria-describedby`) ve çalışırken "İptal" (`cancelRequested` iken devre dışı "İptal ediliyor…");
    - iki ilerleme çubuğu;
    - şeritler: `stale`/`banner` `role="status"`, `error` `role="alert"`, `result.warnings` amber liste;
    - özet `tsd.summary(n, elapsedMs)`.
  - **Liste araç çubuğu:**
    - kategori düğmeleri (basılı durum `aria-pressed`, metin "Etiket (sayı)");
    - arama kutusu;
    - "Tümü" ve "Görünenler" üç durumlu kutuları (`indeterminate` DOM özelliği);
    - seçim özeti;
    - "Betik oluştur" (kapı gerekçeli).
  - **Ayrıntı:**
    - başlık (`kindLabel` + `itemLabel` + durum + `problemText`);
    - notlar ve hatalar;
    - `LineDiffView`.
  - `model().script !== null` iken `const ScriptDialog = lazy(() => import("./ScriptDialog"))` çizilir.
- **`SidePicker.tsx`:**
  - Üç `<select>`: sunucu (`servers()` adlarıyla; üretim sunucusunda ad yanında "(Üretim)", kilitlide "(Kilitli)"), veritabanı, şema.
  - Yüklenirken "Yükleniyor…" seçeneği gösterilir. Hata altta `role="alert"`.
  - Liste yoksa seçici devre dışıdır.
- **`DiffList.tsx`:**
  - `createVirtualizer` (`estimateSize: ROW_HEIGHT`, `overscan: 12`).
  - Satır: kutu (seçilemezse `disabled` + `title = tsd.notSelectable + problemText`), tür etiketi, `itemLabel`, durum metni, `RiskBadge`, `noteBadge`.
  - Tıklama `focus`, kutu `toggle` çağırır. Klavye kuralları Global Constraints'teki gibidir.
  - Odak satırı görünür alana kaydırılır (`scrollToIndex`).
- **`LineDiffView.tsx`:**
  - `lines !== null`: sanallaştırılmış satırlar (`LINE_HEIGHT`, eş aralıklı yazı). Gutter iki numara sütunu + işaret, metin `whitespace-pre`, yatay kaydırma kabın içinde.
  - `lines === null`: `tsd.detail.tooLarge` + iki sütun `<pre>` (her biri kendi kaydırmasında).
  - Yüklenirken `tsd.loading`; seçim yoksa `tsd.detail.pick`.
- **`ScriptDialog.tsx`:**
  - `ui/Dialog` (`width = "880px"`, başlık `tsd.script.title`).
  - Gövde: hedef bandı (`targetBanner(servers().find(id === target.serverId))`), risk rozeti, ifade sayısı, transaction metni, uyarılar, CodeMirror kabı (`createReadOnlyEditor`, `onCleanup` ile `destroy`).
  - Yüklenirken "Yükleniyor…"; hata `role="alert"`.
  - Alt bar: "Kapat", "Kaydet…", "Kopyala", "Query Tool'da aç" (birincil). `data` yokken üçü de devre dışıdır.
  - Esc ve "Kapat" → `closeScript`.

- [ ] **Adım 1: Başarısız test (`diff-view-model.test.ts`).**
  - `rowAria`: seçili/odaklı metin birleşimi; seçilemez öğede ", betiğe eklenemez" eki.
  - `riskTone` üç değer.
  - `lineClass` üç tür (`same` boş).
  - `noteBadge`:
    - hata varsa `tone: "error"`, sayı hata sayısı, `title` iletileri `\n` ile birleştirir;
    - yalnız not → `info`;
    - ikisi de yok → `null`.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `bunx vitest run src/features/schema-diff/diff-view-model.test.ts` → FAIL.
- [ ] **Adım 3: Uygula.** Görünümler yalnız çizer; mantık denetleyicide kalır. Renkler mevcut tasarım belirteçleriyle (`--danger`, `--srv-green`, `--srv-amber`, `text-warn-text`) verilir; yeni CSS değişkeni eklenmez.
- [ ] **Adım 4: Doğrula.**
  - Dört doğrulama komutu.
  - `bun run build` çıktısında `DiffTabs-*.js`, `SchemaDiffView-*.js` ve `ScriptDialog-*.js` parçaları ve boyutları rapora yazılır.
  - İçe aktarma taramaları (sonuç yok beklenir):
    - `rg "from \"(\.\./)+ui/|lucide-solid|codemirror|designer/|inspector/" src/features/schema-diff/DiffTabs.tsx src/features/schema-diff/diff-opener.ts src/features/schema-diff/diff-controller.ts src/features/schema-diff/diff-model.ts src/features/schema-diff/diff-commands.ts`;
    - `rg "app/strings\"" src/features/schema-diff`.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src/features/schema-diff/
  git commit -m "feat(diff): şema karşılaştırma sekmesi, fark listesi, satır farkı ve betik önizleme"
  ```

### Task 5: Paket telafisi, smoke maddeleri ve belgeler

**Dosyalar:** telafi için seçilen ana paket modülleri ve tembel tüketicileri (aşağıda), `scripts/bundle-budget.json`, `docs/smoke-test.md`, `docs/testing.md`, `docs/superpowers/specs/2026-10-31-schema-diff-design.md`.

- [ ] **Adım 1: Ölç.** `bun run bundle:check`. Ana paket ve açılış kümesi baytları, tavanlarla farkları (Task 1–4 artışı) ve `__vite__mapDeps` girdi sayısı kaydedilir.
- [ ] **Adım 2: Telafi.** Yalnız tembel parçaların kullandığı kod ve metinler ana paket modüllerinden tembel modüllere taşınır. Rollup bir modülü tek parçaya koyar; ana paket modülünün **yalnız tembel parçaların içe aktardığı dışa aktarımları** da ana pakette kalır. Plan yazımı sırasında (2026-10-31, `src/main.tsx` statik kapanışı + içe aktarma taraması) bulunan adaylar, öncelik sırasıyla:
  1. **`src/features/query/workspace-model.ts`** (ana pakette 3,741 B):
     - yalnız `tx-dialogs.tsx`'in kullandıkları: `ENV_CHOICES`, `envLabel`, `envLockDefault`, `canCommitAny`;
     - yalnız QueryTab parçasının (`QueryTab.tsx`, `ContextBar.tsx`) kullandıkları: `TIMEOUT_CHOICES`, `formatElapsed`, `sameScope`, `swapEditorState`.
     - Hedefler: ilk grup yeni `src/features/query/env-model.ts`'e (yalnız `tx-dialogs.tsx` içe aktarır), ikinci grup yeni `src/features/query/querytab-model.ts`'e (yalnız QueryTab parçası içe aktarır).
     - Ön koşul: işlev `workspace-model.ts` içinde ana paketin kullandığı başka bir işlevce çağrılmıyor olmalı (`rg "\benvLabel\b" src/features/query/workspace-model.ts` gibi). Çağrılıyorsa taşınmaz.
  2. **`src/features/query/run-model.ts`** (1,537 B): `canCancel`, `canCommit`, `canRollback`, `canLoadMoreRows` (yalnız `ContextBar.tsx` ve `QueryTab.tsx`) → `querytab-model.ts`. Ön koşul aynıdır.
  3. **`src/features/query/result-model.ts`** (2,182 B): `LOAD_MORE_CHUNK` (yalnız `ResultGrid.tsx`) → `querytab-model.ts`; `isUserCancel` (yalnız `export.ts`, ama dosya içi kullanım var gibi görünüyor: önce denetlenir). `MAX_TAB_ROWS` dosya içinde kullanıldığından kalır.
  4. **`src/app/strings.ts` `t.query.ctx`:** `openTx`, `failedTx`, `locked`, `noServer`, `running` anahtarlarından ana paket dosyalarında (`QueryTabs.tsx`, `query-controller.ts`, `workspace-model.ts`) kullanılmayanlar `src/features/query/strings-querytab.ts`'e (`tqt.ctx`) taşınır.
  5. **Son çare:** `src/ipc/errors.ts` `errorText` (yalnız tembel parçalar: dashboard, data, designer). Yeni paylaşılan modül birden çok tembel parçanın statik bağımlılığı olur ve `QueryTabs`'taki `import()`'ların `__vite__mapDeps`'ini büyütebilir. Yalnız ölçülen net kazanç pozitifse yapılır.

  **Kurallar:**
  - Taşınan her ad için `rg` kanıtı (tüketici dosyalar) ve önce/sonra `bundle:check` baytı rapora yazılır.
  - Taşınan modül **tek bir** tembel parçanın modüllerince içe aktarılmalıdır. İki parçanın paylaştığı yeni modül paylaşılan parça doğurur ve ana `__vite__mapDeps`'i büyütür; o taşıma geri alınır.
  - Davranış ve metinler birebir korunur. İlgili testler (`workspace-model.test.ts`, `run-model.test.ts`, `result-model.test.ts`, `tx-dialogs`/`ContextBar` testleri) yeni modülden içe aktarır.
- [ ] **Adım 3: Tavan.**
  - Hedef: ana paket ≤ 127,364 B ve açılış kümesi ≤ 155,037 B.
  - Ölçülen değerler tavanın altındaysa `scripts/bundle-budget.json`'da `mainMaxBytes` ve `startupMaxBytes` ölçülen değerlere **indirilir** (sıfır pay).
  - Telafi yetmezse tavan **yükseltilmez**: görev BLOCKED raporlanır (bayt farkı, denenen taşımalar ve kalan adaylar yazılır); karar kullanıcıya aittir (kullanıcı kararı 2026-10-31).
  - `bun run bundle:check` PASS.
- [ ] **Adım 4: Smoke maddeleri.** `docs/smoke-test.md`'de "## Şema karşılaştırma arka ucu (Plan 29)" bölümünden sonra **"## Şema karşılaştırma (Plan 30)"** eklenir.
  - **Ön koşul paragrafı:**
    - Docker PG 13, 16 ve 18 sunucuları;
    - zengin iki şema (Plan 29 `diff_roundtrip` fikstürü elle kurulabilir), Türkçe adlı şema/tablo (`Şube`, `İLÇE`), bölümlü tablo, 2.000 nesneli büyük şema;
    - "Üretim" etiketli ve kilitli bir hedef sunucu kaydı;
    - sekme tembel parçadadır (`DiffTabs-*.js`, `SchemaDiffView-*.js`, `ScriptDialog-*.js`).
  - **`[—]` maddeleri:**
    - menü görünürlüğü (şema ve veritabanı; sistem şeması, tablo ve sunucuda yok; kilitli sunucuda açık);
    - şema düğümünden açılışta kaynak dolu, hedef şeması boş; veritabanı düğümünden açılışta kaynak şeması boş;
    - bağlı olmayan sunucu seçilince parola istemi;
    - sunucu/veritabanı değişince alt seçicilerin sıfırlanması;
    - "↔" yer değiştirme;
    - aynı şemada "Karşılaştır" kapalı ve gerekçe;
    - ilerleme çubukları (büyük şema) ve "İptal" (kısmi sonuç yok, Explorer bu sırada akıcı);
    - sonuç sayıları ve kategori filtreleri; "Aynı" varsayılan gizli;
    - arama (`ilçe` → `İLÇE`);
    - seçim: varsayılan kural (yalnız hedefte seçili değil), "Tümü"/"Görünenler" üç durumlu kutu, seçilemeyen öğe gerekçesi, "N seçili (M görünmüyor)";
    - seçenek değişince bayat şeridi ve kapalı "Betik oluştur";
    - ayrıntı: renkli satır farkı ve işaret sütunu, hızlı ↑/↓ ile doğru öğe, notlar ("kolon sırası farklı", "yeniden kurulacak", "gövdede kaynak şema adı"), hata iletileri (çapraz sürüm 16 → 13 "üretilemedi"), 20.000 satır üstü yan yana görünüm;
    - betik önizleme: risk rozeti, ifade sayısı, transaction metni, uyarılar, CodeMirror'da kaydırma;
    - üretim ve kilitli hedefte bantlar (kaynak üretimken bant yok);
    - "Query Tool'da aç": hedef sunucu/veritabanında yeni sekme, metin çalıştırılmamış; çalıştırınca mevcut risk/kilit kapıları;
    - "Kopyala";
    - "Kaydet…": önerilen ad, Türkçe ve ASCII dışı yol, vazgeçme, var olan dosyanın üzerine yazma sorusu (Windows);
    - betik Query Tool'da çalıştırılıp yeniden karşılaştırınca yalnız "Aynı" (ve seçilmeyen/elle öğeler) kalması;
    - sekmeyi çalışırken kapatma;
    - dokuzdan fazla karşılaştırmadan sonra eski sekmede "Sonuç artık yok" şeridi;
    - sayfa yenileme sonrası durum;
    - klavye (↑/↓, Boşluk, Home/End, Esc) ve ekran okuyucu (ilerleme, şeritler, satır etiketleri);
    - parola, adres ve betik metni hiçbir bildirimde ya da konsolda görünmez.
- [ ] **Adım 5: Spec ve test belgesi.**
  - Spec `Durum`: "Plan 29 arka ucu tamam; Plan 30 ön yüzü uygulandı (feat/schema-diff-ui); elle kontrol bekliyor (`docs/smoke-test.md` "Şema karşılaştırma (Plan 30)")".
  - Spec'e yeni **"## 7. Plan 30 uygulama notları"** eklenir: Kararlar 1–13'ün özeti (özellikle tembel üç kat, bayat sonuç, gizli seçimlerin betiğe girmesi, kayıttan düşmüş sonuç), paket ölçümleri (ana / açılış / tembel parçalar) ve telafi kaydı.
  - `docs/testing.md`'ye "## Şema karşılaştırma ön yüzü testleri (Plan 30)" eklenir: dosya → kapsam tablosu (`diff-model.test.ts`, `diff-controller.test.ts`, `diff-view-model.test.ts`, `actions.test.ts`, `tabs-store.test.ts`), paket tavanları ve elle kontrol bağlantısı.
- [ ] **Adım 6: Kapanış taramaları ve doğrulama.**
  - `rg "strings-diff" src` → yalnız `src/features/schema-diff/*`.
  - `rg "console\." src/features/schema-diff` → yok.
  - `rg "@kobalte|plugin-dialog|codemirror" src/app src/features/explorer src/ipc src/features/query/tabs-store.ts src/features/query/QueryTabs.tsx` → yok.
  - `rg "schema_diff_" src --glob "!src/features/schema-diff/diff-commands.ts"` → yok (komut adları tek yerde).
  - `git status` → `graphify-out/` ve binding satır sonu gürültüsü sahnelenmemiş.
  - Dört doğrulama komutu PASS.
- [ ] **Adım 7: Commit.**
  ```bash
  git add scripts/bundle-budget.json docs/smoke-test.md docs/testing.md docs/superpowers/specs/2026-10-31-schema-diff-design.md
  git add <Adım 2'de değişen/oluşan dosyalar>
  git commit -m "docs(diff): Plan 30 paket telafisi, smoke maddeleri ve notlar"
  ```

## Öz inceleme

- **Spec kapsamı:**
  - §2.8 giriş noktaları → T1 (menü), T3 (`entryFromTarget` ile ön seçim).
  - Sekme ve tembel parça → T1, T4.
  - Üst bölüm (seçiciler, ↔, seçenekler, Karşılaştır/İptal, ilerleme) → T2–T4.
  - Liste (sanallaştırma, filtreler, sayılar, arama, seçim, risk rozeti, not simgesi) → T2 model, T3 akış, T4 görünüm.
  - Ayrıntı (satır farkı, notlar, hatalar) → T3, T4.
  - "Betik oluştur" (CodeMirror, risk, uyarılar, Query Tool / Kopyala / Kaydet…) → T3, T4.
  - Kilit/üretim bilgisi → T2 `targetBanner`, T4.
  - Paket bütçesi → Global Constraints, T5.
  - §2.7 komutlar ve kapanışta kayıt silme → T2 sarıcılar, T3 `close`/`compare`.
  - §3 ön yüz testleri (liste modeli, seçici durumu, aynı şema reddi) → T2, T3.
  - §3 elle → T5 smoke.
- **Tip tutarlılığı:**
  - `DiffController`/`DiffTabCtl` imzası T1 (iskelet) = T3 (gerçek).
  - `DiffTabHeads`/`DiffPanels` T1 = T4.
  - `Picker` T3 → T4 `ScriptDialog.picker`.
  - `Category`/`Blocked` T2 → T3/T4.
  - Komut sarıcıları T2 = Plan 29 Task 8 komutları (Global Constraints tablosu).
- **Orantı:** kod gövdesi yok; yalnız imzalar, sabit metinler ve test beklentileri.
