# Nesne Diyalogları — Function / Procedure / Trigger / Type / Domain Arayüzü (Plan 19) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Plan 18 arka ucunun üstüne beş türün **arayüzü**: Explorer'dan **Function** (trigger fonksiyonları dahil), **Procedure**, **Trigger**, **Type** (enum/composite/range) ve **Domain** için **Oluştur… / Düzenle… / Yeniden adlandır… / Sil…**; tembel formlar (`RoutineForm` — Function + Procedure tek parça, `TriggerForm`, `TypeForm`, `DomainForm`), düzenlenebilir **gövde düzenleyicisi** (paylaşılan CodeMirror parçasıyla), satır düzenleyicileri (argümanlar, TABLE kolonları, enum değerleri, composite öznitelikleri, domain kısıtları, trigger argümanları), açık **"Yeniden oluştur"** seçimi, salt okunur nesnelerin gerekçeleriyle gösterimi ve elle kontrol listesi. Rust değişikliği **yoktur** (sözleşme boşluğu bulunursa düzeltilmez, raporlanır).

**Bağlantı:** Plan 18 (`docs/superpowers/plans/2026-10-16-object-dialogs-p18-routines-types-arka-uc.md`) `main`'dedir; spec `docs/superpowers/specs/2026-10-16-object-dialogs-routines-types-design.md` **§8 "Plan 18 sözleşme notları" bağlayıcıdır**. Plan 18 tembel designer dosyalarında `FORMS`/`FORM_PATHS` için `Partial` + geçici "form yok" korumasını bırakmıştır; bu plan formları ekler ve korumayı kaldırır.

**Mimari:** Plan 11/13/15 kalıbı aynen: istemci SQL üretmez, `ObjectDef` düzenler; mantık saf modüllerde (`routine-model.ts`, `trigger-model.ts`, `type-model.ts`, `domain-model.ts`), Solid bileşenleri ince; her formun metni kendi tembel modülünde (`strings-routine.ts`, `strings-trigger.ts`, `strings-type.ts`, `strings-domain.ts`). Ana pakete yalnızca menü görünürlüğü (`ddl-kinds.ts`'e altı tür) ve `ddlDefaults`'un `preset` gönderimi girer (Task 1) ve **aynı görevde adı konmuş azaltmayla** telafi edilir.

**Teknoloji:** SolidJS, Tailwind 4, vitest (env `node`), CodeMirror 6 (paylaşılan `inspector/codemirror-setup` parçası), mevcut designer çerçevesi (`designer-controller`, `SqlPreview`, `ApplyBar`, `FormParts`, `DropDialog`, `SettingsEditor`, `TypeInput`).

**Spec:** `docs/superpowers/specs/2026-10-16-object-dialogs-routines-types-design.md` (§1 kararlar, §2 alanlar, §4 arayüz, §8 sözleşme notları); temel spec `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (§3 risk modeli, §4 arayüz). Önceki arayüz planları `2026-10-07-object-dialogs-p13-tablo-on-yuz.md`, `2026-10-07-object-dialogs-p15-view-database-role-on-yuz.md` — burada tekrarlanmayan tüm Global Constraints ve Review Focus maddeleri geçerlidir.

## Kararlar

1. **Gövde düzenleyicisi = düzenlenebilir CodeMirror (koşullu):** `SqlBodyEditor.tsx` (tembel) `inspector/codemirror-setup`'tan `sqlLookExtensions()`'ı ve `@codemirror/state`/`@codemirror/view`'u içe aktararak kendi `EditorView`'unu kurar (`updateListener` → `onChange`, satır numaraları, sekme girintisi, `history` + varsayılan kısayollar yalnızca bu parçada). **Kabul koşulu:** ana `index-*.js` bayt sayısı ve ana `__vite__mapDeps` girdi sayısı **değişmez** (paylaşılan `codemirror-setup` parçası zaten designer'dan iniyor). Koşul sağlanamazsa düzenleyici `textarea`'ya düşer (Plan 15 Karar 1) ve ledger'a yazılır.
2. **"Yeniden oluştur" görünürlüğü istemcide hesaplanır:** `routine-model.ts` `recreateNeeds(old, new): string[]` spec §8.2'deki `needs_recreate` kurallarını **aynalar** (giriş tipi kanonik değişimi — istemcide yalnız metin karşılaştırması; `int` ↔ `int4` gibi eşdeğerlikler arka uca bırakılır ve kutu gereksiz görünebilir; zararsızdır çünkü arka uç gereksiz `recreate`'i yok sayar), var olan giriş adı değişimi/kaldırılması, var olan varsayılanın kaldırılması, giriş sayısı ya da OUT/INOUT listesi değişimi, dönüş değişimi. Liste doluysa formda **"Yeniden oluştur (DROP + CREATE)"** kutusu ve kaybolacaklar notu (GRANT'lar, bağımlı nesneler) görünür; arka uç yetkilidir (`recreate` seçilmeden gelen alan hatası kutuya odaklanır).
3. **Salt okunurluk:** `editable = false` olan her yüklü tanım formda **tamamen salt okunur** gösterilir; üstte `unsupported` gerekçeleri listelenir (`ReadOnlyReasons.tsx`, ortak); Yeniden adlandır / Sil menüleri görünse de arka uç kodsuz `invalid` ile reddeder ve ileti şeritte okunur. Base/array/pseudo/shell/multirange tipler boş composite düzenleyicisi olarak **değil**, "bu tip türü düzenlenemez" gerekçesiyle gösterilir (§8.1).
4. **Yeni tanım alanları:** `ddl_defaults` döndürdüğü `editable: true, unsupported: []` alanları istemcide korunur (`cloneDef`/`defsEqual` kalıbı); `recreate` ve `validate` alanları gönderilmezse `false`.
5. **Trigger fonksiyonu seçimi:** `triggerFunctions` lookup'ı `value` = ad, `detail` = şema döner (R1); seçim `functionName = value`, `functionSchema = detail` yazar; serbest yazım da serbesttir (iki ayrı alan). "Trigger Functions" koleksiyonundan Oluştur `preset = "trigger_function"` ile açılır.
6. **Kural aynalama sınırlı:** form, açıkça geçersiz birleşimleri **kapatarak** önler (ör. `TRUNCATE` seçiliyken `ROW` kapalı; `INSTEAD OF` hiç sunulmaz; `REFERENCING` yalnız `AFTER` + tek olayda açık; `DEFERRABLE` yalnız constraint trigger'da; enum'da yüklü değer için Sil/Taşı kapalı; composite'te yüklü öznitelik için Taşı kapalı, yeni öznitelik yalnız sona; range alanları alter'da salt okunur; domain temel tip/collation alter'da salt okunur). Diğer her kural arka ucun alan hatasıyla gösterilir (istemci ayrıca doğrulamaz).
7. **Sürüm bağımlı alanlar gizlenmez** (Plan 13/15 kararı): Procedure (PG 11+), `CREATE OR REPLACE TRIGGER` (PG 14+), multirange adı (PG 14+), procedure OUT argümanı (PG 14+) ipucuyla sunulur; eski sunucuda arka uç alan hatası gösterilir.
8. **Kapsam dışı (backlog):** view/`INSTEAD OF` trigger'ları, aggregate/operator/event trigger, base type create, `EXECUTE` yetkileri, typed table CASCADE, fonksiyon gövdesinde dil-özel tamamlama, gövde için Query Tool'da "çalıştır".

## Global Constraints

- **Paket bütçesi (bağlayıcı):** ana `index-*.js` bu planın başında **tam 232,343 B**'dir (pay yok), `__vite__mapDeps` **21** girdi, `index.html` yalnızca solid/web/Icon önyükler. Plan 19 **yalnızca tembel**dir; tek istisna Task 1: `src/features/designer/ddl-kinds.ts`'e altı tür (`function`, `procedure`, `triggerFunction`, `type`, `domain`, `trigger`) ve `src/ipc/commands.ts` `ddlDefaults`'a `preset` parametresi — **aynı görevde** şu adı konmuş azaltmayla telafi edilir: `t.query.grid` ve `t.query.cell` metin blokları (`src/app/strings.ts`; yalnızca `ResultGrid.tsx` ve `CellPanel.tsx` kullanır — QueryTab parçası) yeni tembel `src/features/query/strings-grid.ts`'e (`tgrid`) taşınır; yetmezse sıradaki adaylar (her biri `rg` ile "yalnız tembel tüketici" doğrulanarak): `t.query.strip`, `t.query.results` (`sessionLost` anahtarı hariç — `query-controller.ts` ana pakette kullanır), `t.import` (`title` ve `button` hariç), `t.server` (ana paketin kullandığı `cancel`, `close`, `lockedTitle`, `labelSuggestions` hariç). **Kabul ölçütü:** Task 1 sonunda ve sonraki **her** UI görevinde ana `index-*.js` ≤ **232,343 B** (Task 1'de ölçülen değer sonraki görevlerin tavanıdır); `__vite__mapDeps` **21**; preload solid/web/Icon.
- **Boyut doğrulaması (her UI görevinin son adımı):**
  ```powershell
  bun run build
  (Get-Item dist/assets/index-*.js).Length                       # ≤ Task 1 değeri (≤ 232,343)
  Select-String -Path dist/index.html -Pattern 'modulepreload'   # yalnızca solid / web / Icon
  Select-String -Path dist/assets/index-*.js -SimpleMatch "Yeniden oluştur (DROP + CREATE)","Trigger fonksiyonu","Enum değerleri","Domain kısıtları"   # BOŞ
  Get-ChildItem dist/assets -Filter "*Form-*.js" | Select-Object Name, Length
  ```
  (Uygulayıcı her görevde gerçekten eklenen, başka yerde geçmeyen dizgeleri seçip kontrolü günceller ve ledger'a yazar.)
- **Tembel kural:** ana paket modülleri (`src/app/*` — `strings-*.ts` tembel modüller hariç —, `src/features/explorer/*`, `src/features/query/{QueryTabs,tabs-store,query-controller}.ts(x)`, `src/ipc/commands.ts`, `ddl-kinds.ts`) yeni form/model/metin modüllerini **statik içe aktarmaz**. Tembel formlarda Kobalte yok, `preview.ts`'in `createPreviewer`'ı içe aktarılmaz; lucide simgelerinden yalnızca `Trash2` ve `Undo`. Formlar yalnızca `DesignerView`'ın `lazy(() => import(…))` girdileriyle erişilir; aynı modülü iki tembel parça içe aktarırsa paylaşılan parça doğar — **ana** `__vite__mapDeps` değişmediği sürece kabul (ölçülür).
- **İstemci SQL üretmez:** gövde, WHEN, varsayılan ve CHECK ifadeleri **olduğu gibi** gönderilir; serbest SQL alanlarının altında `FreeSqlNote` + `E'…'` ipucu; gövdenin dolar tırnaklaması arka uçtadır (formda gövdeye `$$` yazılmaz).
- **Sözleşme (spec §8):** yeni tanımlar `editable: true, unsupported: []` ile gider; `recreate`/`validate` alter-only ve yüklemede `false`; trigger `table` değiştirilemez; range alanları ve domain `baseType`/`collation` alter'da salt okunur; enum yüklü değer silme/yeniden sıralama ve composite yüklü öznitelik yeniden sıralama sunulmaz; `settings` listesi yükleyicinin sırasıyla tutulur (yeniden sıralama değişiklik sayılmaz ama form yine de sıralamayı bozmaz); COST/ROWS yüklenen metin olduğu gibi gösterilir.
- **FieldError yolları** spec §8.5 listesidir; satır indeksleri taslaktaki sıradır; `old.` önekliler ve bilinmeyen yollar genel şeride (`groupIssues`).
- **Uygulama güvenliği** Plan 11 ile aynı ve **değişmez** (`applyGate`, onay kutusu, üretimde ad yazma, kilit, tek uçuş, `plan_changed` sonrası otomatik yükleme yok); DataLoss sınıfları (fonksiyon yeniden oluşturma, composite öznitelik silme/tip değiştirme, CASCADE) üretimde ad yazma ister.
- **Düğüm kimlikleri ayrıştırılmaz:** hedef `ddl_target_of`; `DdlTarget.create.preset` `ddlDefaults`'a aynen geçer.
- **Tepkisellik dersleri:** denetleyicide okunan her yeni `Map` mevcut `registry` sinyaline bağlanır; Solid sinyali kullanan her test dosyası başında `vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));`; saf model testleri Solid içe aktarmaz.
- **Gizlilik/erişilebilirlik/tema:** SQL/gövde/tanım/yorum `console.*`'a ve depolamaya yazılmaz; arayüz metinleri Türkçe, PostgreSQL terimleri İngilizce; renkler yalnızca CSS değişkenleri; her alan etiketli, hatalar `aria-describedby`; satır düzenleyicileri klavyeyle tam kullanılır (ekle/sil/taşı düğmeleri Tab ile erişilir, odak taşınan satırda kalır); gövde düzenleyicisinde Tab girinti yapar, **Esc ardından Tab** odaktan çıkar (klavye tuzağı yok; ipucu metni).
- Test: `bun run test`, `bun run typecheck`, `bun run build`; `cargo test --workspace` (Docker'sız) yeşil kalır; git worktree **kullanılmaz**; `src/ipc/bindings` satır sonu gürültüsü `git checkout -- src/ipc/bindings`. Ajanlar GUI'yi elle denemez: elle kontrol maddeleri `docs/smoke-test.md`'ye `[—]` olarak yazılır. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- **Paket bütçesi:** her görevde ana `index-*.js` ≤ Task 1 değeri, `__vite__mapDeps` 21; CodeMirror düzenleyicisi ana pakete hiçbir şey eklemez; telafi taşımaları davranışı değiştirmez (QueryTab ızgarası ve hücre paneli metinleri aynı).
- **Yeniden oluşturma:** `recreateNeeds` spec §8.2 ile tutarlı (giriş adı değişimi, varsayılan kaldırma, OUT/INOUT, dönüş); kutu yalnızca alter'da ve liste doluyken; DataLoss onayı + üretimde ad yazma; arka uçtan gelen "yeniden oluşturmayı gerektirir" alan hatası kutuya odaklanır.
- **Salt okunurluk:** `editable = false` nesnede hiçbir denetim düzenlenemez; gerekçeler listelenir; base/multirange tipler boş composite olarak gösterilmez; yeni tanımlarda `editable`/`unsupported` alanları düşmez.
- **Satır düzenleyicileri:** enum yüklü değer silme/taşıma kapalı, yeni değerin konumu taslak sırası; composite yüklü öznitelik taşıma kapalı, yeni yalnız sona; argüman satırlarında mod/ad/tip/varsayılan; domain alter'da yeni kısıt adı zorunlu; hata yolları doğru satıra düşer (silme/ekleme sonrası kayma yok).
- **Trigger:** fonksiyon seçimi ad + şemayı doğru yazar; geçersiz birleşimler kapalı; `UPDATE OF` kolonları tablonun kolonlarından; etkinlik durumu ve yorum.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `src/features/designer/ddl-kinds.ts`, `src/ipc/commands.ts`, `src/app/strings.ts`, `src/features/query/strings-grid.ts` (yeni), `ResultGrid.tsx`, `CellPanel.tsx` (+ `actions.test.ts`) | **Ana paket (Task 1):** menü kancası, `preset`, telafi |
| `src/features/designer/designer-controller.ts`, `designer-open.ts`, `ReadOnlyReasons.tsx`, `SqlBodyEditor.tsx`, `RowList.tsx` (ortak satır düzenleyicisi), `src/app/strings-designer.ts` (+ testler) | Designer çekirdeği: `preset`, salt okunur gerekçeler, gövde düzenleyicisi, satır listesi |
| `src/features/designer/routine-model.ts`, `RoutineForm.tsx`, `src/app/strings-routine.ts` (+ test) | Function / Procedure |
| `src/features/designer/trigger-model.ts`, `TriggerForm.tsx`, `src/app/strings-trigger.ts` (+ test) | Trigger |
| `src/features/designer/type-model.ts`, `TypeForm.tsx`, `src/app/strings-type.ts` (+ test) | Type |
| `src/features/designer/domain-model.ts`, `DomainForm.tsx`, `src/app/strings-domain.ts` (+ test) | Domain |
| `src/features/designer/{forms-model.ts, DesignerView.tsx}` | `FORMS`/`FORM_PATHS` tam `Record`, geçici koruma kaldırılır (Task 6) |
| `docs/smoke-test.md`, spec | Elle kontrol listesi, durum |

---

### Task 1: Ana paket — menü kancası, `preset` ve adı konmuş telafi

**Dosyalar:** Değiştir `src/features/designer/ddl-kinds.ts`, `src/ipc/commands.ts`, `src/app/strings.ts`, `src/features/query/ResultGrid.tsx`, `src/features/query/CellPanel.tsx`, `src/features/explorer/actions.test.ts`, designer testlerindeki `ddlDefaults` sahteleri; Oluştur `src/features/query/strings-grid.ts`.

**Arayüzler:**
- `ddl-kinds.ts`: `DDL_OBJECT_KINDS`'e `"function", "procedure", "triggerFunction", "type", "domain", "trigger"` (NodeKind adları — bindings'ten doğrula); `DDL_CREATE_COLLECTIONS` türetilmiş kalır (`functionCollection`, `procedureCollection`, `triggerFunctionCollection`, `typeCollection`, `domainCollection`, `triggerCollection`); dosya bağımlılıksız kalır.
- `commands.ts`: `ddlDefaults: (serverId, databaseOid, kind, ctx, preset: CreatePreset | null = null) => call<ObjectDef>("ddl_defaults", { serverId, databaseOid, kind, ctx, preset })` (`CreatePreset` yalnız tip içe aktarımı).
- Telafi: `t.query.grid` ve `t.query.cell` → `strings-grid.ts` (`export const tgrid = { grid: {…}, cell: {…} } as const;` birebir kopya); `ResultGrid.tsx`/`CellPanel.tsx` `tgrid` kullanır; `rg "t\.query\.(grid|cell)" src` → 0. Yetmezse Global Constraints'teki sıradaki adaylar (her biri `rg` doğrulamalı; seçim ledger'a).

- [ ] **Adım 1: Başlangıç ölçümü.** `bun run build`; ana `index-*.js` **232343**, mapDeps 21 (değilse dur ve raporla).
- [ ] **Adım 2: Başarısız testler.** `actions.test.ts`: `functionCollection`, `procedureCollection`, `triggerFunctionCollection`, `typeCollection`, `domainCollection`, `triggerCollection` koleksiyonlarında `createObject`; `function`, `procedure`, `triggerFunction`, `type`, `domain`, `trigger` düğümlerinde `editObject`/`renameObject`/`dropObject`; kilitli sunucuda gerekçe; kolon/kısıt düğümleri değişmez (yalnız Düzenle). `designer-controller.test.ts`'teki `ddlDefaults` beklentileri beşinci argüman `null` ile güncellenir (Task 2 gerçek değeri geçirir).
- [ ] **Adım 3–4:** Başarısızlığı gör; uygula; telafi.
- [ ] **Adım 5: Doğrulama.** `bun run test`, `bun run typecheck`, **Boyut doğrulaması**: ana ≤ 232,343 B, mapDeps 21; ölçülen değeri commit mesajına ve `docs/smoke-test.md` "Ölçümler" altına yaz ("Plan 19 Task 1: index-*.js = N B"). Aşılamıyorsa **dur ve raporla**.
- [ ] **Adım 6: Commit** (`feat(explorer): Function/Procedure/Trigger/Type/Domain menü kancası ve ddl_defaults preset; ızgara/hücre metinleri tembel parçaya (ana paket telafisi, index N B)`).

---

### Task 2: Designer çekirdeği — `preset`, salt okunur gerekçeler, gövde düzenleyicisi, satır listesi

**Dosyalar:** Değiştir `designer-controller.ts` (+ test), `designer-open.ts` (+ test), `src/app/strings-designer.ts`; Oluştur `ReadOnlyReasons.tsx`, `SqlBodyEditor.tsx`, `RowList.tsx`, `row-list-model.ts` (+ test).

**Arayüzler:**
- Denetleyici: create girişine `preset: CreatePreset | null` (`DdlTarget.create.preset`'ten `designer-open.ts` geçirir); `api.ddlDefaults(…, c.kind, c.context, c.preset)`; `defsEqual`/`cloneDef` `editable`/`unsupported` alanlarını korur (test: `ddl_defaults` sonucu düzenlenip gönderildiğinde iki alan `PlanRequest`'te var).
- `ReadOnlyReasons.tsx`: `props.def: { editable: boolean; unsupported: string[] }` → `editable = false` ise `role="note"` şerit ("Bu nesne buradan düzenlenemez" + gerekçe listesi); mevcut "Düzenlenmeyenler" listeleriyle görsel uyumlu.
- `SqlBodyEditor.tsx`: `props: { id, value, onChange(v), readOnly, describedBy, label }`; Karar 1 (CodeMirror; `EditorView` `onCleanup`'ta yok edilir; dışarıdan gelen değer değişimi imleç konumunu korur; Tab girinti, Esc → sonraki Tab odaktan çıkar; satır sonu korunur; değer `\r\n` içerirse olduğu gibi kalır). Koşul sağlanmazsa `textarea` (ledger).
- `row-list-model.ts` (saf): `moveRow<T>(list, i, dir)`, `removeRow`, `insertRow(list, i, item)` + kısıtlar (`canMove(i, dir, locked)`); `RowList.tsx`: genel satır kabı (ekle / sil / yukarı / aşağı düğmeleri, `Trash2` dışında simgesiz metin düğmeler, odak taşınan satırda kalır, `aria-label`'lar satır adıyla).
- [ ] **Adım 1: Başarısız testler.** `designer-controller.test.ts`: `preset` `ddlDefaults`'a geçer (`trigger_function`), `editable/unsupported` gidiş-dönüşü; `designer-open.test.ts`: `trigfuncs` hedefinden açılış `preset` taşır; `row-list-model.test.ts`: taşıma sınırları, kilitli satır, değişmezlik.
- [ ] **Adım 2–3:** uygula. **Adım 4:** `bun run test`, `typecheck`, **Boyut doğrulaması** (ana ≤ Task 1; mapDeps 21; `SqlBodyEditor` henüz bir forma bağlı değilse bile derlemede ana pakete girmez).
- [ ] **Adım 5: Commit** (`feat(designer): create preset, salt okunur gerekçe şeridi, gövde düzenleyicisi ve satır listesi`).

---

### Task 3: Function / Procedure formu

**Dosyalar:** Oluştur `routine-model.ts` (+ test), `RoutineForm.tsx`, `src/app/strings-routine.ts`; Değiştir `forms-model.ts` (`ROUTINE_PATHS`), `DesignerView.tsx` (`function`, `procedure` → aynı `lazy(() => import("./RoutineForm"))`).

**Arayüzler (`routine-model.ts`, saf):**

```ts
export function asRoutine(d: ObjectDef | null): (FunctionDef & { kind: "function" | "procedure" }) | null;
export function routinePatch(d: ObjectDef, patch: Partial<FunctionDef>): ObjectDef;
export function recreateNeeds(old: FunctionDef | null, d: FunctionDef): string[];   // Karar 2; create → []
export function withReturns(d: FunctionDef, kind: "type" | "setof" | "table" | "trigger" | "void"): FunctionDef;  // TABLE'a geçişte OUT/INOUT argüman uyarısı; trigger'a geçişte argüman listesi korunur (arka uç hata verir)
export function argModes(kind: "function" | "procedure"): ArgMode[];              // procedure'de out PG 14+ ipucuyla
export function securityDefinerWarning(old: FunctionDef | null, d: FunctionDef): boolean;
export function recreateLoss(): string[];                                       // "GRANT yetkileri", "bağımlı trigger/view'lar (varsa uygulama reddedilir)"
```
- `ROUTINE_PATHS`: spec §8.5 Function/Procedure yolları (kök anahtarlar: `schema, name, owner, comment, language, body, cost, rows, volatility, strict, leakproof, parallel, kind, object, args, returns, settings, recreate`).
- **Form:** Genel (şema + `schemas` datalist, ad + `createFocusOnRequest("name")`, sahip, yorum, dil `select` — `languages` lookup; detay "güvenilmez" ise ipucu), **Argümanlar** (`RowList`: mod `select`, ad, tip `TypeInput`, varsayılan; trigger dönüşünde liste kapalı + ipucu; procedure'de `OUT` PG 14+ ipucu), **Dönüş** (Function: `select` tip/SETOF tip/TABLE/trigger/void; TABLE kolon `RowList`; Procedure'de bölüm yok), **Gövde** (`SqlBodyEditor`, `FreeSqlNote`, "dolar tırnaklaması otomatik" notu), **Seçenekler** (volatility `select`, STRICT, SECURITY DEFINER + uyarı, LEAKPROOF + "süper kullanıcı" ipucu, PARALLEL `select`, COST, ROWS (yalnız set dönen), Procedure'de yalnız SECURITY DEFINER), **Ayarlar** (`SettingsEditor`, sıra korunur), **Yeniden oluştur** (Karar 2: `recreateNeeds` doluysa kutu + kayıp listesi + DataLoss notu), `ReadOnlyReasons`. Bölümler basit başlıklı `section`; tüm denetimler `readOnly` ya da `editable=false` iken `disabled`.
- [ ] **Adım 1: Başarısız testler** (`routine-model.test.ts`): `recreateNeeds` tablosu (giriş tipi metin değişimi, var olan ad değişimi/kaldırma, adsıza ad verme serbest, varsayılan kaldırma, varsayılan ekleme serbest, OUT ekleme, IN→INOUT, dönüş değişimi, create → []), `withReturns` geçişleri, `securityDefinerWarning` (false→true), `routinePatch` tür uyuşmazlığında aynı nesne. `forms-model.test.ts`: `FORM_PATHS.function === FORM_PATHS.procedure === ROUTINE_PATHS`; `groupIssues` örnekleri (`args[1].typeName` alan, `old.args[0].typeName` genel).
- [ ] **Adım 2–3:** uygula. **Adım 4:** `bun run test`, `typecheck`, **Boyut doğrulaması** (ana ≤ Task 1; mapDeps 21; `RoutineForm-*.js` ayrı parça; CodeMirror paylaşılan parçadan; `rg "strings-routine" src` yalnız form/model).
- [ ] **Adım 5: Commit** (`feat(designer): Function/Procedure formu (argümanlar, dönüş, gövde düzenleyicisi, seçenekler, SET, yeniden oluştur)`).

---

### Task 4: Trigger formu

**Dosyalar:** Oluştur `trigger-model.ts` (+ test), `TriggerForm.tsx`, `src/app/strings-trigger.ts`; Değiştir `forms-model.ts` (`TRIGGER_PATHS`), `DesignerView.tsx`.

**Arayüzler (`trigger-model.ts`, saf):** `asTrigger`, `triggerPatch`, `availability(d): { row: boolean; statement: boolean; referencing: boolean; deferrable: boolean; updateColumns: boolean }` (Karar 6 kuralları: TRUNCATE → yalnız STATEMENT; constraint → yalnız AFTER + ROW; REFERENCING → AFTER, constraint değil, `UPDATE OF` yok, tek olay; OLD TABLE yalnız UPDATE/DELETE, NEW TABLE yalnız INSERT/UPDATE), `normalizeTrigger(d)` (kapanan seçeneğin değerini temizler: ör. TRUNCATE seçilince `level = statement`), `functionFromLookup(item) → { functionSchema, functionName }`, `rebuildNote(old, d, versionHint)` ("PG 14+ OR REPLACE; daha eskide DROP + CREATE (Yıkıcı)" bilgi metni — sürüm arayüzde bilinmez, iki durum da anlatılır).
- **Form:** Genel (tablo salt okunur metin, ad, yorum, etkinlik `select` — "Etkin / Yalnız replika / Her zaman / Devre dışı"), **Zamanlama ve olaylar** (BEFORE/AFTER `radio`; INSERT/UPDATE/DELETE/TRUNCATE onay kutuları; UPDATE OF kolon çipleri — `tableColumns` lookup), **Düzey** (ROW/STATEMENT `radio`, kapalı seçenek gerekçe `title`'lı), **WHEN** (`textarea` font-mono + `FreeSqlNote` + "STATEMENT trigger'da NEW/OLD kullanılamaz" ipucu), **Fonksiyon** (`triggerFunctions` lookup datalist/`select`; şema + ad alanları; "Trigger fonksiyonu oluştur" bağlantısı yok — backlog), **Argümanlar** (`RowList`, metin), **Geçiş tabloları** (OLD/NEW TABLE ad alanları; kapalıyken ipucu), **Constraint trigger** (kutu; DEFERRABLE / INITIALLY DEFERRED), `ReadOnlyReasons`.
- [ ] **Adım 1: Başarısız testler** (`trigger-model.test.ts`): `availability` tablosu (her kural), `normalizeTrigger` temizlikleri (TRUNCATE → statement; constraint kapanınca deferrable false; olay sayısı artınca transition tablolar temizlenir; `UPDATE` kapanınca `updateColumns` temizlenir), `functionFromLookup` (value/detail eşlemesi R1). `forms-model.test.ts`: `TRIGGER_PATHS`.
- [ ] **Adım 2–5:** uygula; test; **Boyut doğrulaması**; commit (`feat(designer): Trigger formu (zamanlama/olay/düzey, WHEN, fonksiyon seçimi, geçiş tabloları, constraint trigger)`).

---

### Task 5: Type formu (enum / composite / range)

**Dosyalar:** Oluştur `type-model.ts` (+ test), `TypeForm.tsx`, `src/app/strings-type.ts`; Değiştir `forms-model.ts` (`TYPE_PATHS`), `DesignerView.tsx`.

**Arayüzler (`type-model.ts`, saf):** `asType`, `typePatch`, `switchVariant(d, v)` (yalnız create; alter'da yok), `enumAdd(d, at, label)` (yeni değer `original = null`), `enumRename(d, i, label)`, `enumRemove(d, i)` (yalnız `original = null` değerlerde), `enumMove(d, i, dir)` (yalnız yeni değerler kendi aralarında ve yüklü değerlerin arasına; yüklü değerlerin göreli sırası **değişmez** — model yasak hamleyi reddeder), `attrAdd(d, attr)` (yalnız sona), `attrRemove(d, i)` (yüklü öznitelikte DataLoss notu), `attrMoveAllowed(d, i, dir)` (yüklü öznitelik → false), `isReadOnlyVariant(d)` (yükleyicinin boş composite + unsupported döndürdüğü base/multirange vb. → true; Karar 3).
- **Form:** Genel (şema, ad + odak, sahip, yorum), **Tür** (create: `radio` Enum/Composite/Range; alter: salt okunur rozet), **Enum değerleri** (`RowList`: etiket; yeni satır "yeni" rozeti; yüklü satırda Sil/Taşı kapalı + "PostgreSQL silmeyi/yeniden sıralamayı desteklemez" ipucu; ekleme konumu imleçteki satırın altına), **Composite öznitelikleri** (`RowList`: ad, tip `TypeInput`, collation; yüklü satırda Taşı kapalı; Sil/tip değişimi DataLoss notu; "tablo kolonunda kullanılıyorsa PG reddeder" ipucu), **Range** (alt tip, opclass, collation, canonical, subtype_diff, multirange adı (PG 14+ ipucu); alter'da salt okunur), `ReadOnlyReasons`.
- [ ] **Adım 1: Başarısız testler** (`type-model.test.ts`): enum işlemleri (yüklü silme/taşıma reddi, yeni değer ekleme/taşıma, yeniden adlandırma `original` korur), composite (sona ekleme, yüklü taşıma reddi, silme), `switchVariant` alter'da no-op, `isReadOnlyVariant`. `forms-model.test.ts`: `TYPE_PATHS`; `body.values[2].label` yolu doğru satıra.
- [ ] **Adım 2–5:** uygula; test; **Boyut doğrulaması**; commit (`feat(designer): Type formu (enum değerleri, composite öznitelikleri, range)`).

---

### Task 6: Domain formu ve korumanın kaldırılması

**Dosyalar:** Oluştur `domain-model.ts` (+ test), `DomainForm.tsx`, `src/app/strings-domain.ts`; Değiştir `forms-model.ts` (`DOMAIN_PATHS`; `FORM_PATHS` tam `Record<ObjectKind, …>`), `DesignerView.tsx` (`FORMS` tam `Record`; geçici "form yok" koruması ve sabit metin **silinir**: `rg "FORM_MISSING|formMissing" src` → 0).

**Arayüzler (`domain-model.ts`, saf):** `asDomain`, `domainPatch`, `checkAdd(d)` (`{ name: "", expression: "", notValid: false, validated: false, original: null, validate: false }`), `checkRemove`, `checkRename`, `needsName(mode, c)` (alter'da `original = null` kısıtta ad zorunlu → yerel ön uyarı; asıl hata arka uçtan), `canValidate(c)` (yüklü + `validated = false` + ifade değişmemiş), `notValidAllowed(c)` (yeni ya da ifadesi değişmiş).
- **Form:** Genel (şema, ad, sahip, yorum), **Tanım** (temel tip `TypeInput` — alter'da salt okunur; collation — alter'da salt okunur; DEFAULT ifadesi + `FreeSqlNote`; NOT NULL kutusu + "mevcut veride NULL varsa uygulama başarısız olur (Yıkıcı)" ipucu), **CHECK kısıtları** (`RowList`: ad, ifade (font-mono `textarea`), NOT VALID (yalnız izinliyse), "Doğrula" (yalnız `canValidate`), "doğrulanmamış" rozeti), `ReadOnlyReasons`, PG 17+ adlı NOT NULL notu (spec §8.7).
- [ ] **Adım 1: Başarısız testler** (`domain-model.test.ts`): `checkAdd`/`checkRemove`/`checkRename` değişmezliği, `needsName`, `canValidate`, `notValidAllowed`. `forms-model.test.ts`: `DOMAIN_PATHS`; **`FORM_PATHS` her `ObjectKind` için tanımlı** (14 tür; `Partial` yok).
- [ ] **Adım 2–5:** uygula; test; **Boyut doğrulaması**; `rg "FORM_MISSING|formMissing" src` → 0; commit (`feat(designer): Domain formu; form yok koruması kaldırıldı`).

---

### Task 7: Elle kontrol listesi, spec durumu ve bütünleşik doğrulama

**Dosyalar:** Değiştir `docs/smoke-test.md`, `docs/superpowers/specs/2026-10-16-object-dialogs-routines-types-design.md`.

- [ ] **Adım 1: `docs/smoke-test.md`** — Plan 18 bölümündeki "GUI maddeleri Plan 19 bölümündedir" cümlesinin ardına **"## Nesne diyalogları: Function/Procedure/Trigger/Type/Domain arayüzü (Plan 19)"** (ön koşul: Docker PG 11, 13, 14+ ve 17+; bir şema, tablo, trigger fonksiyonu, enum ve composite tip, domain; üretim etiketli sunucu). Maddeler (hepsi `- [ ] … **[—]**`): menü (altı koleksiyon/düğüm; eklentiye ait ve sistem nesneleri salt okunur açılır); Function oluştur (argümanlar, dönüş türleri, gövde düzenleyicisi — Tab girinti, Esc+Tab çıkış, satır sonları, büyük gövde, `$$` içeren gövde), SECURITY DEFINER uyarısı, LEAKPROOF yetki hatası ipucu; Function düzenle (gövde → OR REPLACE Güvenli; argüman tipi/adı değişimi → "Yeniden oluştur" kutusu + kayıp listesi + Veri kaybı onayı + üretimde ad yazma; bağımlı trigger varken yeniden oluştur → bağımlılar ipucu, değişiklik yok; aşırı yüklenmiş iki fonksiyondan doğru olan); Procedure (PG 11+; PG 10'da öğretici hata; OUT PG 14+); Trigger Functions koleksiyonundan oluştur (dönüş `trigger` ve şablon gövde); Trigger oluştur/düzenle (olay/düzey kuralları kapalı seçeneklerle, UPDATE OF kolonları, WHEN, fonksiyon seçimi, geçiş tabloları, constraint trigger, etkinlik durumu; PG 13'te yapı değişimi Yıkıcı onayı; PG 14+ Güvenli); Type (enum değer ekle/yeniden adlandır, yüklü değerde Sil/Taşı kapalı; composite öznitelik ekle/sil/tip değiştir Veri kaybı; tablo kolonunda kullanılan composite'te tip değişimi ipucu; range oluştur, alter'da salt okunur; base/multirange tip salt okunur gerekçeli); Domain (temel tip/collation alter'da kilitli, DEFAULT, NOT NULL → mevcut NULL'da hata ve geri alma, kısıt ekle NOT VALID, Doğrula, ad değişimi); salt okunur nesnede Yeniden adlandır/Sil → açık ret iletisi; klavye/erişilebilirlik/tema (satır düzenleyicileri klavyeyle, gövde düzenleyicisi klavye tuzağı yok, açık/koyu tema, dar pencere ≈1100 px); "Ölçümler" altına son ölçüm.
- [ ] **Adım 2: Spec** — durum "Plan 18–19 uygulandı"; §4'e kısa **"Plan 19 notları"** (CodeMirror kararı ve ölçüm sonucu, `recreateNeeds` aynalama sınırı, salt okunur gösterimi, telafi taşıması, backlog).
- [ ] **Adım 3: Bütünleşik doğrulama.** `bun run test`, `bun run typecheck`, `bun run build` + **Boyut doğrulaması** (ana ≤ Task 1 değeri; mapDeps 21; tembel parça boyutları rapora); `cargo test --workspace` (Docker'sız), `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Adım 4: Kapanış greps:** `console\.` / `localStorage|sessionStorage` / `innerHTML` `src/features/designer` → boş; `strings-(routine|trigger|type|domain)` yalnız ilgili form/model; `t\.query\.(grid|cell)` → 0; `FORM_MISSING|formMissing` → 0.
- [ ] **Adım 5: Commit** (`docs(designer): Function/Procedure/Trigger/Type/Domain arayüzü elle kontrol listesi, spec durumu; bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 19 bitişi)

- Beş tür için tam akış: menü → açma (preset dahil) → düzenleme → önizleme → kapılı uygulama → başlık + ağaç yenileme; salt okunur nesneler gerekçeli; yeniden oluşturma açık seçim.
- Ana paket ≤ Task 1 değeri (≤ 232,343 B), mapDeps 21, preload solid/web/Icon; yeni kod/metin yalnız tembel parçalarda.
- Elle kontrol maddeleri `[—]`; final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §4 menü (Task 1), gövde düzenleyicisi (Task 2–3), satır düzenleyicileri (Task 2–6), `SET` listesi (Task 3), bütçe (Task 1 + her görev); §8.1 salt okunurluk/yeni tanım alanları (Task 2, 3–6), §8.2 yeniden oluşturma (Task 3), §8.3 preset (Task 1–2), §8.4 lookup'lar (Task 3–4), §8.5 alan yolları (Task 3–6 `*_PATHS`), §8.6 ipuçları (mevcut hata şeridi; yeni metin yok), §8.7 sınırlar (formdaki ipuçları + smoke).
- **Bilinçli kararlar:** CodeMirror koşullu (Karar 1); `recreateNeeds` istemci aynası arka uç yetkili (Karar 2); kural aynalama yalnız açık geçersizlikleri kapatır (Karar 6).
- **Yer tutucu taraması:** "TBD/TODO" yok; geçici koruma Task 6'da kalkar.
- **Tip tutarlılığı:** `CreatePreset` (Plan 18 bindings) → `commands.ts` (Task 1) → denetleyici (Task 2); `RowList`/`row-list-model` (Task 2) → Task 3–6; `ReadOnlyReasons` (Task 2) → Task 3–6; `*_PATHS` (Task 3–6) → `FORM_PATHS` tam `Record` (Task 6).
