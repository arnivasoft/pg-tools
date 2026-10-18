# Bakım İşlemleri — Ön Yüz (Plan 25) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Explorer'dan **"Bakım…"** diyaloğu. Diyalog işlem ve seçenek seçimi, SQL önizlemesi, risk ve kilit notları, lock timeout, onay, uygula ve iptal, canlı ilerleme, `VERBOSE` iletileri ile WARNING vurgusu ve sonuç özeti içerir. Plan 24 arka ucunu uçtan uca kullanılabilir yapar.

**Mimari:**
- Ana pakette yalnız şunlar bulunur: menü eylemi (`maintenance`), tür kümesi, menü metni, iki `commands.ts` sarıcısı ve tembel yükleyici.
- Diyalog tembel bir parçadır (`MaintenanceDialog.tsx` + saf `maintenance-model.ts` + `strings-maintenance.ts`). Önizleme ve uygulama mevcut `ddl_preview`/`ddl_apply`/`ddl_cancel` ile yapılır. İstek biçimi `{op:"maintenance", object, target, operation}`.
- İlerleme, uygulama sürerken ~1 sn aralıkla `ddl_maintenance_progress(applyId)` ile sorulur. Başarılı uygulamadan sonra hedef yeniden yüklenir.
- Kod tasarımcı (designer) sekmesine değil, `DropDialog` benzeri bağımsız bir diyaloğa dayanır.

**Teknoloji:** SolidJS, Kobalte (yalnız tembel; mevcut `ui/Dialog` tembel), vitest.

**Spec:** `docs/superpowers/specs/2026-10-18-maintenance-design.md`. Bağlayıcı olanlar: §3 arayüz özeti ve **§6 Plan 24 sözleşme notları**. Paket kuralları: `docs/testing.md` "Ana paket bütçesi" (`bun run bundle:check`; sıfır pay). Rolldown dersi: tembel görünüm ana modülleri içe aktarmaz, metin ve işleyicileri props ile alır.

## Kararlar

1. **Menü:**
   - Eylem kimliği `maintenance`. Adı `Object` ile bitmez, çünkü designer kipi değildir.
   - Grup: nesne eylemleri grubu (3). `groupOf`'a açık kural eklenir.
   - Kilitli sunucuda devre dışıdır ve aynı gerekçe metnini gösterir (`disabledReason`'a açık kural).
   - Görünür olduğu türler (`NodeKind`): `table`, `materializedView`, `index`, `schema`, `database`. Bölümler tablodur.
   - Sıra: `editObject`, `renameObject`, `privilegesObject`, `defaultPrivilegesObject`, **`maintenance`**, `dropObject`.
   - Koleksiyonlar, kolon ve kısıt satırlarında görünmez.
2. **Diyalog akışı:**
   1. **Hedef yükleme:** açılışta `ddlTargetOf(nodeId)` ile `ObjectRef` ve `databaseOid` (veritabanı düğümünde `object.oid`) alınır, ardından `ddlLoadMaintenanceTarget` çağrılır. Hata olursa diyalog içinde ileti ve "Yeniden dene" gösterilir.
   2. **İşlem seçimi:** `opsFor(kind, partitioned, versionNum)`. Bu, Rust `ops_for`'un TS aynasıdır (spec §6.1) ve Rust tablosuyla birebir test edilir. İşlem yoksa (ör. bölümlenmiş indeks PG < 14 ya da `editable = false`) `unsupported` gerekçesi gösterilir; Uygula kapalıdır.
   3. **Seçenekler:**
      - İşleme göre seçenekler görünür; sürüm kapısının altındaki seçenekler gizlenmez, **devre dışı + "PG N+ gerektirir"** olarak gösterilir.
      - Birbirini dışlayanlar devre dışıdır: `FULL` → `PARALLEL`/`DISABLE_PAGE_SKIPPING`; sistem nesnesinde `CONCURRENTLY`.
      - `INDEX_CLEANUP` seçimi: varsayılan (yok) / AUTO (PG 14+) / ON / OFF. `PARALLEL` sayısı 0–1024 arasındadır.
      - CLUSTER `USING` seçimi `target.indexes`'ten gelir. Varsayılanı `clusterIndex`'tir; o da yoksa ilk indeks seçilir; liste boşsa CLUSTER devre dışıdır ve gerekçe gösterilir.
      - Varsayılanlar: VACUUM `ANALYZE` açık, diğerleri kapalı; REINDEX `CONCURRENTLY` PG 12+ ve sistem nesnesi değilse açık; diğer tüm boolean'lar kapalı.
   4. **Önizleme:**
      - Her değişiklikte gecikmeli (debounced) `ddl_preview` çağrılır. Gösterilenler: tek ifade (salt okunur `<pre>`; CodeMirror yok, parça büyümesin), risk rozeti, `Statement.note` ve `PlanView.warnings`.
      - Alan hataları ilgili seçeneğin altında, `object`/`op`/eşlenmeyen yollar üst şeritte gösterilir.
   5. **Uygulama:**
      - Lock timeout seçimi (designer'daki seçeneklerle aynı değerler) yapılır.
      - Risk Destructive ise onay kutusu gerekir. DataLoss olmadığından üretimde ad yazma yoktur; kural yine `applyGate` ile aynı kalır.
      - Uygula'ya basılınca `applyId` üretilir ve `ddl_apply` çağrılır.
   6. **Çalışırken:**
      - İlerleme çubuğu: `total` varsa yüzde, yoksa belirsiz çubuk + sayı.
      - Aşama adı bilinen PG aşama metinleri için Türkçeye çevrilir, bilinmeyen olduğu gibi gösterilir.
      - Geçen süre (mm:ss) gösterilir. **İptal** düğmesi `ddl_cancel` çağırır.
      - Seçenekler kilitlenir.
      - Diyalog kapatılmak istenirse "İşlem sürüyor; iptal edilsin mi?" onayı sorulur (Evet → iptal + kapanış beklenir; Hayır → açık kalır).
   7. **Sonuç:**
      - Başarıda: "Tamamlandı (süre)", iletiler listesi ve WARNING sayısı (spec §6.6: WARNING satırları belirgin, özet "N uyarı").
      - Başarısızlıkta: PG hata iletisi + ipucu (`error.hint`) + iletiler. İptalde (57014) "İptal edildi".
      - REINDEX CONCURRENTLY başarısız ya da iptal edilirse `*_ccnew` uyarısı yinelenir ve elle temizleme önerisi gösterilir (spec §6.6).
      - Başarıdan sonra hedef yeniden yüklenir (spec §6.6); hatada yeniden yükleme yapılmaz.
      - "Yeniden çalıştır" ile aynı seçeneklerle yeni önizleme yapılabilir.
3. **Sürüm bilgisi:** `versionNum` ön yüzde mevcut bir kaynaktan okunur (sunucu bağlantı bilgisi; ör. `ServerInfo` ya da Explorer düğüm bayrakları — uygulayıcı mevcut kaynağı bulur ve rapora yazar). Kaynak yoksa seçenekler sürüm kapısız gösterilir; arka uç doğrulaması hatayı alan altında gösterir. Bu yedek davranış da testlidir.
4. **Metinler:** menü metni `t.menu.maintenance` ("Bakım…") ana pakettedir. Diğer metinler `src/app/strings-maintenance.ts` dosyasındadır (yalnız diyalog içe aktarır): işlem adları, seçenek etiketleri ve açıklamaları, aşama çevirileri, durum metinleri, onay ve iptal metinleri. Diyalog `ui/Dialog`, `ui/Button` ve `ui/Field` kullanabilir (zaten tembel diyaloglarda kullanılıyorlar). Ana pakete yeni bağımlılık eklemez; `bundle:check` yeni preload yokluğunu doğrular.
5. **Ana paket bütçesi:** menü kancası ve `commands.ts` sarıcıları ana pakete bayt ekler; bu bayt aynı görevde geri ödenir (`strings.ts`'ten yalnız tembel parçaların kullandığı bir blok tembel modüle taşınır). Geri ödeme yapılamazsa tavan, görev raporunda gerekçe ve ölçülen değerle yükseltilir; plan toplamı en çok +500 B.
6. **Kapsam dışı:** birden çok nesne seçimi, zamanlama, `vacuumdb` benzeri paralel iş, bölümlenmiş indeks bakımı, ilerlemenin Dashboard'da gösterimi.

## Global Constraints

- Dal `feat/maintenance-ui` (main'den). main'e birleştirme yalnızca kullanıcı onayıyla.
- Yalnız ön yüz (`src/`, `docs/`; gerekirse `scripts/bundle-budget.json`). Rust değişmez.
- **Paket:**
  - `bun run bundle:check` her görevde geçer.
  - Ana paket ≤ `mainMaxBytes`, açılış kümesi ≤ `startupMaxBytes`.
  - Preload tam `["solid","web"]` olmalı; yasak paket olmamalı.
  - Tembel diyalog, ana modülleri (`app/strings`, `explorer/actions`, `tree-model`…) içe aktarmaz.
  - Yükleyici sözleşmesi: `export const loadMaintenanceDialog = () => import("./MaintenanceDialog")` (ana tarafta). `prefetchIdle` listesine eklenir.
- **Erişilebilirlik:**
  - Diyalog başlığı ve odak tuzağı `ui/Dialog` ile sağlanır.
  - Seçenekler etiketli olur; devre dışı gerekçeleri `aria-describedby` ile bağlanır.
  - İlerleme `role="progressbar"` ile verilir: `aria-valuenow/max` ya da belirsizse `aria-valuetext` (aşama + sayı).
  - Sonuç ve hata `role="status"`/`role="alert"` ile duyurulur.
  - Klavye kısayolları: Enter uygular (onay tamamsa), Esc kapatır (çalışırken iptal onayı sorar).
  - IME koruması uygulanır.
- **vitest:** ortam `node`. Mantık `maintenance-model.ts` içinde saf fonksiyonlardır ve orada test edilir. GUI ajanlarca test edilemez; elle kontrol maddeleri `docs/smoke-test.md` "Bakım işlemleri arayüzü (Plan 25)" bölümüne `[—]` olarak eklenir.
- **Gizlilik:** iletiler, SQL ve nesne adları `console.*`'a ve depolamaya yazılmaz.
- Yorumlar Türkçe, metinler Türkçe. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Doğrulama:** `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`.

## Review Focus

- **İstek biçimi:**
  - `operation` alanı kullanılır.
  - `target` yüklenen haliyle değiştirilmeden gönderilir.
  - `applyId` hem `ApplyOptions`'ta hem ilerleme sorgusunda aynıdır.
  - Önizleme hash'i ile uygulama hash'i tutarlıdır: önizleme bitmeden Uygula kapalıdır, seçenek değişince eski önizleme bayatlar.
- **İlerleme yoklaması:**
  - Yalnız uygulama sürerken çalışır.
  - Sonuç gelince, iptalde ya da diyalog kapanınca durur (sızıntı yok).
  - Üst üste istek yığmaz; önceki yanıt beklenir.
  - `null` yanıtta belirsiz çubuk gösterilir.
  - Hata olursa sessizce yoklamaya devam edilmez; birkaç ardışık hatada durulur.
- **Kapatma ve yarış:** çalışırken kapatma onayı gösterilir. Diyalog kapalıyken gelen sonuç durumu bozmaz. Aynı nesnede ikinci diyalog açılabilir; aynı anda iki uygulama birbirinin ilerlemesini karıştırmaz, çünkü her birinin kendi `applyId`'si vardır.
- **Seçenek kuralları:** sürüm kapıları, karşılıklı dışlamalar, CLUSTER `USING` varsayılanı ve boş liste, sistem nesnesi kuralları, `opsFor` ile Rust `ops_for` eşdeğerliği.
- **Sonuç:** WARNING sayımı ve vurgusu; taşma iletisinin ("N ileti daha gösterilmedi") olduğu gibi gösterilmesi; başarıdan sonra hedefin yeniden yüklenmesi (CLUSTER sonrası ikinci çalıştırma `plan_changed` vermez); `*_ccnew` uyarısı.

---

### Task 1: Ana paket kancası (menü, komutlar, yükleyici) ve geri ödeme

**Dosyalar:** `src/ipc/commands.ts`, `src/features/explorer/actions.ts` (+ test), `src/app/strings.ts` (`t.menu.maintenance`), `src/features/explorer/ExplorerMenu.tsx` (etiket/işleyici prop), `src/app/App.tsx` (`openMaintenance` bağımlılığı, tembel yükleyici, prefetch), geri ödeme için bir `strings.ts` bloğu ve onun tembel modülü, gerekirse `scripts/bundle-budget.json`.

**Arayüzler — Üretir:**

```ts
// commands.ts
ddlLoadMaintenanceTarget: (serverId: string, databaseOid: number, object: ObjectRef) => call<MaintenanceTarget>("ddl_load_maintenance_target", { serverId, databaseOid, object }),
ddlMaintenanceProgress: (serverId: string, databaseOid: number, applyId: string) => call<MaintenanceProgress | null>("ddl_maintenance_progress", { serverId, databaseOid, applyId }),
// actions.ts
type ActionId = … | "maintenance";
interface ActionDeps { …; openMaintenance?: (id: string) => void }
// ddl-kinds.ts ya da yeni küçük ana modül
export const MAINTENANCE_KINDS: ReadonlySet<NodeKind>;   // table, materializedView, index, schema, database
// App.tsx (ana)
export const loadMaintenanceDialog = () => import("../features/maintenance/MaintenanceDialog");  // yol uygulayıcıya; yükleyici sözleşmesi
```
- Bu görevde diyalog yalnız geçici bir iskelettir: "Hazırlanıyor" + kapat. Task 3 gerçeğini yazar. İskelet ayrı dosyada tembeldir.
- [ ] **Adım 1: Başarısız testler (`actions.test.ts`):**
  - Beş türde `maintenance` var; koleksiyon, kolon, kısıt, view, sequence, function, role ve trigger türlerinde yok.
  - Sıra ve grup doğru.
  - Kilitli sunucuda gerekçe gösteriliyor.
  - `executeAction` `openMaintenance(id)` çağırıyor.
  - Menü metni doğru.
- [ ] **Adım 2–3:** uygula; geri ödeme. **Adım 4:** doğrulamalar + `bundle:check` (sayılar rapora). **Adım 5: Commit** (`feat(maintenance): Explorer "Bakım…" menüsü, komutlar ve tembel yükleyici`).

### Task 2: Saf model

**Dosyalar:** Oluştur `src/features/maintenance/maintenance-model.ts` (+ `maintenance-model.test.ts`), `src/app/strings-maintenance.ts`.

**Arayüzler — Üretir:**

```ts
export type OpName = "vacuum" | "analyze" | "reindex" | "cluster";
export function opsFor(kind: MaintenanceTargetKind, partitioned: boolean, versionNum: number | null): OpName[];   // §6.1; versionNum null → en geniş küme (arka uç doğrular)
export function defaultOperation(op: OpName, target: MaintenanceTarget, versionNum: number | null): MaintenanceOp;
export type OptionState = { visible: boolean; enabled: boolean; reason: string | null };   // reason: "PG 12+ gerektirir" | "FULL ile birlikte kullanılamaz" | …
export function optionStates(op: MaintenanceOp, target: MaintenanceTarget, versionNum: number | null): Record<string, OptionState>;
export function normalize(op: MaintenanceOp, target: MaintenanceTarget, versionNum: number | null): MaintenanceOp;   // devre dışı kalan seçenekleri temizler (ör. FULL açılınca parallel=null, disablePageSkipping=false)
export function maintenanceRequest(object: ObjectRef, target: MaintenanceTarget, op: MaintenanceOp): PlanRequest;   // {op:"maintenance", object, target, operation: op}
export function progressView(p: MaintenanceProgress | null, phaseLabel: (phase: string) => string): { percent: number | null; text: string };
export function messageSummary(messages: ServerMessage[] | undefined): { warnings: number; total: number };
export function needsCcnewNotice(op: MaintenanceOp, outcome: "applied" | "failed" | "cancelled"): boolean;
export function elapsedLabel(ms: number): string;   // mm:ss
// Yoklama denetleyicisi (zamanlayıcı ve api enjekte)
export function createProgressPoller(deps: { fetch: () => Promise<MaintenanceProgress | null>; schedule: (cb: () => void, ms: number) => () => void; onUpdate: (p: MaintenanceProgress | null) => void; onStop: (reason: "errors") => void }): { start(): void; stop(): void };
```
- [ ] **Adım 1: Başarısız testler:**
  - `opsFor`: Rust tablosuyla birebir; sürümler 100000, 130000, 140000, 150000 ve null.
  - Varsayılanlar.
  - `optionStates`: her sürüm kapısı (kapı altı devre dışı + gerekçe), `FULL` dışlamaları, sistem nesnesinde `CONCURRENTLY`, CLUSTER `USING` boş liste.
  - `normalize`.
  - İstek biçimi (`operation`).
  - `progressView`: total var / yok / null; aşama çevirisi bilinen ve bilinmeyen.
  - `messageSummary`: WARNING sayımı; taşma iletisi sayılır mı (INFO → hayır).
  - `needsCcnewNotice`.
  - Yoklama: sırayla çalışır (önceki bitmeden yenisi yok); `stop` sonrası çağrı yok; 3 ardışık hata → `onStop`.
- [ ] **Adım 2–4:** uygula, doğrulamalar. **Adım 5: Commit** (`feat(maintenance): bakım diyaloğu saf modeli`).

### Task 3: Diyalog

**Dosyalar:** Oluştur `src/features/maintenance/MaintenanceDialog.tsx`, gerekirse `maintenance-controller.ts` (+ test; durum makinesi: loading → ready ⇄ previewing → applying → done/failed/cancelled; kapatma onayı); iskeleti değiştir.

- Kararlar 2–4. Önizleme gecikmesi mevcut designer/drop diyaloglarıyla aynıdır (`createPreviewer` ya da DropDialog deseni; ana modüllere bağımlılık eklemeden).
- Kontrol katmanı api'yi enjeksiyonla alır: `ddlTargetOf`, `ddlLoadMaintenanceTarget`, `ddlPreview`, `ddlApply`, `ddlCancel`, `ddlMaintenanceProgress`. Durum geçişleri saf testlidir.
- [ ] **Adım 1: Başarısız testler (denetleyici, sahte api):**
  - Yükleme hatası → yeniden dene.
  - Seçenek değişince önizleme yenilenir ve eski hash kullanılmaz.
  - Destructive için onaysız uygulama kapalı.
  - Uygulama sırasında yoklama başlar, sonuçta durur.
  - İptal → `ddlCancel(applyId)` ve "İptal edildi".
  - Başarıdan sonra hedef yeniden yüklenir.
  - `plan_changed` reddi → hedefi yeniden yükle ve bildir.
  - Kapatma isteği çalışırken onay ister.
  - Kapalıyken gelen sonuç yok sayılır.
  - WARNING özeti doğru.
- [ ] **Adım 2–4:** uygula, doğrulamalar, `bundle:check`.
- [ ] **Adım 5:** smoke maddeleri. Kapsanacaklar:
  - her hedef türü ve işlem;
  - sürüm kapıları (PG 10 ve 17 sunucu);
  - VACUUM FULL onayı;
  - CLUSTER `USING` seçimi ve ikinci çalıştırma;
  - REINDEX CONCURRENTLY ve iptal sonrası `*_ccnew` uyarısı;
  - VERBOSE iletileri ve WARNING (sahibi olunmayan tablo);
  - lock timeout (başka oturumda kilit);
  - uzun VACUUM'da ilerleme çubuğu;
  - çalışırken kapatma;
  - klavye ve ekran okuyucu;
  - kilitli sunucu.

  **Commit** (`feat(maintenance): "Bakım…" diyaloğu`).

### Task 4: Belgeler ve bütünleşik doğrulama

**Dosyalar:** spec (durum: "Plan 25 ön yüzü uygulandı; elle kontrol bekliyor" + uygulama notları), `docs/smoke-test.md` (bölümü topla), `docs/testing.md`.

- [ ] **Adım 1:** `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`; ana paket ve açılış kümesi sayıları spec'e yazılır.
- [ ] **Adım 2: Kapanış taramaları:**
  - `rg "strings-maintenance" src` → yalnız bakım diyaloğu dosyaları.
  - `rg "@kobalte" src/app src/features/explorer src/ipc` → yok.
  - `rg "console\." src/features/maintenance` → yok.
- [ ] **Adım 3: Commit** (`docs(maintenance): Plan 25 smoke ve notlar`).

## Öz inceleme

- **Spec kapsamı:**
  - §3 arayüz özeti → T1/T3.
  - §6.1 istek ve `ops_for` → T2.
  - §6.2 alan yolları → T3 hata eşlemesi.
  - §6.3 ipuçları → T3 sonuç.
  - §6.4 iletiler → T2/T3.
  - §6.5 ilerleme → T2/T3.
  - §6.6 yeniden yükleme, `*_ccnew`, disk notu, WARNING → T2/T3.
- **Tip tutarlılığı:** `maintenanceRequest` (T2) → denetleyici (T3); `createProgressPoller` (T2) → T3; `loadMaintenanceDialog` (T1) → prefetch.
