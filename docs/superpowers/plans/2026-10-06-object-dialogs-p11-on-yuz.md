# Nesne Diyalogları Ön Yüz (Plan 11) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Plan 10 arka ucunun üstüne nesne diyalogları arayüzü: Explorer'dan **Oluştur… / Düzenle… / Yeniden adlandır… / Sil…** (Schema, Sequence, Extension, Index), sağ panelde `designer` sekmesi, canlı SQL önizleme, risk onayı ve korumalı uygulama.

**Mimari:** Tüm mantık saf, test edilebilir modüllerdedir (`designer-model`, `preview` zamanlayıcı, `designer-controller`); Solid bileşenleri ince kalır. İstemci **hiç SQL üretmez**: formlar `ObjectDef` düzenler, `ddl_preview` planı döner, `ddl_apply` aynı tanımı gönderir. Dashboard kalıbı izlenir: `designer` sekmesi `tabs-store`'a üçüncü sekme türü olarak girer, tüm kod tembel parçadır (ana paket büyümez). Diyalog durumu denetleyicide tutulur; sekme kapanırken kaydedilmemiş değişiklik onay ister.

**Teknoloji:** SolidJS, Kobalte (Dialog/Select/Tabs), Tailwind 4, CodeMirror 6 (`createReadOnlyEditor` — `src/features/inspector/codemirror-setup.ts`), vitest.

**Spec:** `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (§1.2, §3 risk modeli, §4 arayüz, §5 türler, §6 güvenlik; **"Sözleşme notları"** paragrafı bağlayıcıdır). Arka uç sözleşmesi: `docs/superpowers/plans/2026-10-06-object-dialogs-p10-arka-uc.md` ve `src/ipc/bindings/*.ts` (`ObjectDef`, `PlanRequest`, `PlanView`, `ApplyOptions`, `ApplyOutcome`, `DdlTarget`, `LookupRequest`, `FieldError`, `DdlErrorCode`).

## Global Constraints

- **Ana paket büyümez:** designer modülleri yalnızca `lazy(() => import(...))` ile yüklenir; ana pakette (`src/app`, `QueryTabs.tsx`, `tabs-store.ts`, `explorer/actions.ts`) yalnızca ince kancalar ve tipler bulunur. Doğrulama: `bun run build` çıktısında ana `index-*.js` boyutu bu planın başındaki ölçüme göre en çok **+3 KB** artar (ilk görevde başlangıç boyutu not edilir); designer kodu ayrı parçalardır. Menü metinleri (`t.menu.*`) ana pakette kalır, diğer tüm metinler `src/app/strings-designer.ts` (yalnızca tembel parçada içe aktarılır).
- **İstemci SQL üretmez ve SQL'i değiştirmez:** önizleme metni yalnızca `PlanView.statements[].sql`'dir (salt okunur). "Query Tool'da aç" bu metni olduğu gibi yeni sorgu sekmesine koyar; çalıştırmaz.
- **Sözleşme (Plan 10 Sözleşme notları):** `PlanRequest::Alter/Drop` her zaman `object: ObjectRef` taşır ve `old`/`def` **tam olarak `ddl_load`'un döndürdüğü** tanımdır (istemci `old`'u asla düzenlemez/yeniden kurmaz); sequence sayıları (`increment`, `minValue`, `maxValue`, `start`, `cache`) JSON'da **string**'dir; geçersiz tanım → `PlanView.errors` dolu **ve** `empty = true` (ayırt etmek için `errors.length` bakılır); boş plan **asla uygulanmaz**; tablespace temizlenemez (`pg_default` seçilir); `FieldError.path` sözlüğü: `name`, `owner`, `comment`, `schema`, `version`, `dataType`, `increment`, `minValue`, `maxValue`, `start`, `cache`, `ownedBy.{schema,table,column}`, `method`, `unique`, `columns`, `columns[i]`, `columns[i].key|opclass|collation`, `include`, `include[i]`, `tablespace`, `fillfactor`, `whereClause`, `constraint`, `kind`; `old.` önekli yollar genel hata olarak gösterilir; `columns` hatası son-ifade taramasından da gelebilir (alan değil genel hata gibi göster).
- **Uygulama güvenliği:** (1) **Uygula**, yalnızca *mevcut taslağa karşılık gelen en son önizleme* geldiyse (`hash` taslağın sürümüyle eşleşir; önizleme bekliyorsa/bayatsa düğme kapalı), `errors.length === 0`, `!empty` iken etkindir; (2) `risk` ≥ `destructive` ise "Etkisini anladım" onay kutusu işaretlenmeden gönderilmez (`confirmed`); (3) `risk === data_loss` **ve** sunucu üretim etiketli ise kutuya ek olarak **sunucu adı yazılmadan** gönderilmez (`confirmedProduction`); yazılan ad büyük/küçük harf duyarlı, boşluk kırpılmış tam eşleşmedir; (4) sunucu kilitliyse (`readOnlyLock`) tüm değiştiren eylemler/düğmeler kapalıdır (gerekçe `title`), önizleme serbesttir; (5) uygulama uçuştayken tüm düğmeler kapalı, çift gönderim yok; yanıt gelince `expectedHash` taşıyan istek tekrar gönderilmez; (6) odak **Vazgeç/İptal**'te başlar, `Enter` yıkıcı eylemi onaylamaz.
- **Hata eşlemesi yalnızca `ErrorPayload.code` ile** (metinle asla): `locked`, `confirmation_required`, `production_confirmation_required`, `plan_changed` (→ "Nesne değişti; yeniden yükle" ve yeniden yükleme eylemi), `invalid_def` (→ önizlemeyi yenile), `not_connected`/`connection_lost`/`unknown_server` (→ sekme "bağlantı kesildi" salt okunur durumu), `not_found` (→ "nesne artık yok"), diğer her kod `errorText(p)` biçimiyle (`message — detail — hint`; `src/features/dashboard/dash-controller.ts`'teki `errorText` mantığı bu modüle taşınmadan kopyalanmaz: ortak yardımcı gerekiyorsa `src/ipc/errors.ts`'e taşı ve her iki kullanıcıyı güncelle). `Ok(Failed{failedIndex, committed, sqlstate, position, error})`: başarısız ifade önizlemede vurgulanır; `committed > 0` ise "N ifade kalıcı oldu" uyarısı; PG `position` yalnızca ifade metninde konum göstermek içindir; `sqlstate` `55P03` → "kilit beklerken zaman aşımı".
- Düğüm kimlikleri arayüzde **ayrıştırılmaz**: hedef `ddl_target_of(nodeId)` ile alınır (`object`, `create`, `refreshNodeId`); düğüm türü kümeleri yalnızca menü görünürlüğü içindir (arka uç yine reddeder).
- Önizleme: alan değişikliğinden sonra **150 ms** debounce, en son istek kazanır, kapanan sekme/uçuştaki eski yanıt durumu değiştirmez; uygulama sırasında yeni önizleme başlamaz.
- Gizlilik: SQL metni, tanım değerleri, yorumlar `console.*`'a, `localStorage`'a/`sessionStorage`'a, geçmişe yazılmaz; yalnızca ekranda ve kullanıcı kopyalarsa panoda. Serbest SQL alanlarında (indeks ifadesi, `WHERE`) "ifade SQL olarak çalıştırılır" notu gösterilir.
- Arayüz metinleri Türkçe, PostgreSQL terimleri İngilizce (`Schema`, `Sequence`, `Extension`, `Index`, `lock_timeout`, `OWNED BY`…); renkler yalnızca CSS değişkenlerinden (`--accent`, `--danger`, `--danger-fg`, `--warn-text`, `--muted`, `--border`…); açık/koyu temada okunur; kalın tek taraflı vurgu kenarlığı yok (rozet/etiket kullan); erişilebilirlik: her alanda etiket, hata metni `aria-describedby`, `role="alert"` ile uygulama hatası, odak sırası, klavye ile tam kullanım.
- Yeni çalışma zamanı bağımlılığı yok; `console.*` yok (testler hariç spy); commit eki: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- Test: `bun run test` (vitest), `bun run typecheck`, `bun run build`; cargo testleri bu plan boyunca değişmemelidir (`cargo test --workspace` yeşil kalır; Rust değişikliği **yok**, bir sözleşme boşluğu bulunursa raporlanır). Git worktree **kullanılmaz**; `src/ipc/bindings`'e satır sonu gürültüsü düşerse `git checkout -- src/ipc/bindings`. Ajan çalıştırıcılar GUI'yi elle denemez; doğrulama vitest + typecheck + build; elle kontrol maddeleri `docs/smoke-test.md`'ye yazılır.

## Review Focus

- **Bayat önizleme / çift gönderim:** taslak önizleme sonrası değiştiyse Uygula kapalı; yeni önizleme gelene kadar eski `hash` ile gönderilmez; uygulama uçuştayken ikinci tıklama/Enter ikinci istek üretmez; `apply` bittikten sonra gelen eski önizleme yanıtı durumu bozmaz.
- **Onay ve korumalar:** yıkıcı planda onay kutusu olmadan, üretim + veri kaybında ad yazılmadan gönderim yok (UI ve denetleyici düzeyinde); kilitli sunucuda tüm düğmeler/menü eylemleri kapalı; sunucu adı değişir/etiket kalkarsa (kayıt güncellenirse) onay gereksinimi yeniden hesaplanır.
- **Sözleşme uyumu:** her `Alter/Drop` `object` taşır ve `old` = yüklenen tanım (derin kopya, düzenlenen taslakla aynı nesne değil); sequence sayıları string; boş/geçersiz plan gönderilmez; `created` ile yeni nesneye geçiş; `plan_changed` sonrası yeniden yükleme taslağı ezmeden önce kullanıcıya sorar (kaydedilmemiş değişiklik varsa).
- **Yaşam döngüsü:** aynı nesne için ikinci "Düzenle" mevcut sekmeyi etkinleştirir; sunucu bağlantısı kesilir/silinirse designer sekmeleri "bağlantı kesildi" salt okunur olur ve istek atmaz; sekme kapanırken kaydedilmemiş değişiklik onayı (Vazgeç odaklı); pencere kapanırken uçuştaki uygulama varsa uyarı değil sessiz (arka uç iptal eder) — davranış belgelenir; uygulama sonrası Explorer'ın doğru düğümü yenilenir (`refreshNodeId`), silmede düğüm kaybolur.
- **Form doğruluğu:** `FieldError` yolları doğru alana eşlenir (`columns[1].key` → ikinci kolonun anahtar alanı); eşlenemeyen hatalar genel hata şeridinde görünür; `old.`-önekli hatalar genel; sürüme bağlı alanlar (INCLUDE vb.) arka uç hatasıyla birlikte öğretici metin gösterir; sequence yön değişiminde `start/min/max` birlikte güncellenir (aksi halde `start` hatası çıkar).

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `src/ipc/commands.ts` | `ddl*` komutları (7) |
| `src/features/designer/designer-model.ts` | Saf modeller: hata eşleme, onay modeli, kind/title yardımcıları, ifade aralıkları, sayı alanı doğrulama, indeks kolon işlemleri |
| `src/features/designer/preview.ts` | Debounce'lu, "son istek kazanır" önizleme zamanlayıcısı |
| `src/features/designer/designer-controller.ts` | Designer sekmelerinin durumu: açma/yükleme, taslak, önizleme, uygulama, iptal, yaşam döngüsü |
| `src/features/designer/{DesignerTabs,DesignerView,SqlPreview,ApplyBar,FormParts}.tsx` | Sekme başlığı/panel, kabuk, SQL önizleme, uygulama çubuğu, ortak form parçaları |
| `src/features/designer/{SchemaForm,ExtensionForm,SequenceForm,IndexForm}.tsx` | Tür formları |
| `src/features/designer/{DropDialog,designer-open}.tsx/.ts` | Silme diyaloğu; Explorer düğümünden açma akışı |
| `src/features/query/tabs-store.ts`, `QueryTabs.tsx` | Üçüncü sekme türü (`designers`) ve başlıklar/paneller |
| `src/features/explorer/actions.ts`, `ExplorerMenu.tsx`, `src/app/App.tsx`, `src/app/strings.ts` | Menü eylemleri (kilitliyse devre dışı), bağlama |
| `src/app/strings-designer.ts` | Tüm designer metinleri (yalnızca tembel parçada) |
| `docs/smoke-test.md` | Elle kontrol listesi |

---

### Task 1: IPC komutları ve saf modeller

**Dosyalar:**
- Değiştir: `src/ipc/commands.ts`
- Oluştur: `src/features/designer/designer-model.ts`, `preview.ts` ve her birinin `.test.ts` dosyası

**Arayüzler:**
- Tüketir: `src/ipc/bindings/{ObjectDef,ObjectKind,ObjectRef,PlanRequest,PlanView,Statement,Risk,FieldError,ApplyOptions,ApplyOutcome,DdlTarget,CreateContext,LookupRequest,LookupItem,SequenceDef,IndexDef,IndexColumn,IndexKey,ExtensionDef,SchemaDef}.ts`.
- Üretir (`commands.ts`, mevcut `call<T>` kalıbıyla; argüman adları camelCase — `src-tauri/src/ddl.rs` parametreleriyle eşleşmeli):

```ts
ddlTargetOf: (nodeId: string) => call<DdlTarget>("ddl_target_of", { nodeId }),
ddlDefaults: (serverId: string, databaseOid: number, kind: ObjectKind, ctx: CreateContext) => call<ObjectDef>("ddl_defaults", { serverId, databaseOid, kind, ctx }),
ddlLoad: (serverId: string, databaseOid: number, object: ObjectRef) => call<ObjectDef>("ddl_load", { serverId, databaseOid, object }),
ddlLookup: (serverId: string, databaseOid: number, request: LookupRequest) => call<LookupItem[]>("ddl_lookup", { serverId, databaseOid, request }),
ddlPreview: (serverId: string, databaseOid: number, request: PlanRequest) => call<PlanView>("ddl_preview", { serverId, databaseOid, request }),
ddlApply: (serverId: string, databaseOid: number, request: PlanRequest, options: ApplyOptions) => call<ApplyOutcome>("ddl_apply", { serverId, databaseOid, request, options }),
ddlCancel: (applyId: string) => call<null>("ddl_cancel", { applyId }),
```

**Önce** `src-tauri/src/ddl.rs`'te gerçek Tauri parametre adlarını doğrula (`ctx` mı `context` mı, `options` mı `opts` mı); farklıysa komutları ona göre yaz ve farkı raporla.

- Üretir (`designer-model.ts`, hepsi saf):

```ts
export type DesignerKind = ObjectKind;                                   // "schema" | "sequence" | "extension" | "index"
export function kindLabel(kind: ObjectKind): string;                      // "Schema" …
export function defName(def: ObjectDef): string;                          // başlıkta/mesajlarda görünen ad
export function cloneDef<T extends ObjectDef>(def: T): T;                 // derin kopya (structuredClone)
export function defsEqual(a: ObjectDef, b: ObjectDef): boolean;           // taslak == yüklenen mi (kirli bayrağı)
export interface FieldIssues { byPath: Map<string, string[]>; general: string[] }
/** FieldError[] → alan yoluna göre gruplar; `old.` önekliler ve bilinmeyen/üst-düzey `columns` hataları general'a düşer. */
export function groupIssues(errors: readonly FieldError[], knownPaths?: ReadonlySet<string>): FieldIssues;
export function issuesFor(issues: FieldIssues, path: string): string[];   // tam yol + `path.` altı (ör. columns[1] → columns[1].key dahil)
export type ApplyGate =
  | { kind: "ready" }
  | { kind: "blocked"; reason: "locked" | "invalid" | "empty" | "stale" | "pending" | "busy" | "disconnected" }
  | { kind: "needsConfirm" }                                              // checkbox gerekli
  | { kind: "needsProductionName"; expected: string };                    // checkbox + ad gerekli
export interface GateInput { locked: boolean; production: boolean; serverName: string; view: PlanView | null; viewIsCurrent: boolean; pending: boolean; applying: boolean; connected: boolean }
export function applyGate(i: GateInput): ApplyGate;
export interface ConfirmState { checked: boolean; typedName: string }
export function canSubmit(g: ApplyGate, c: ConfirmState): boolean;        // ready → true; needsConfirm → checked; needsProductionName → checked && typedName.trim() === expected
export function confirmFlags(g: ApplyGate, c: ConfirmState): { confirmed: boolean; confirmedProduction: boolean };
/** Birleşik betik + her ifadenin [başlangıç, bitiş) karakter aralığı (başarısız ifadeyi vurgulamak için). Her ifade sonuna `;` eklenir, ifadeler arası boş satır. */
export function scriptWithRanges(statements: readonly Statement[]): { text: string; ranges: Array<[number, number]> };
export function isSafeIntegerString(s: string): boolean;                  // sequence sayı alanları: /^[+-]?\d+$/ ve i64 aralığı (BigInt ile)
export function applyOptionsFor(applyId: string, lockTimeoutMs: number, view: PlanView, gate: ApplyGate, c: ConfirmState): ApplyOptions;
// İndeks kolon listesi işlemleri (değişmez): addColumn/removeColumn/moveColumn/updateColumn/addInclude/removeInclude
```

- Üretir (`preview.ts`) — zaman enjekte edilir (test sahte zamanlayıcıyla):

```ts
export interface PreviewDeps { setTimer(fn: () => void, ms: number): unknown; clearTimer(h: unknown): void }
export interface Previewer<R> {
  /** Her çağrı öncekini geçersiz kılar; 150 ms sonra `run`; yalnızca EN SON çağrının yanıtı `onResult`a gider. */
  schedule(run: () => Promise<R>): void;
  cancel(): void;                         // bekleyen zamanlayıcıyı ve uçuştaki yanıtı geçersiz kılar
  dispose(): void;
}
export function createPreviewer<R>(deps: PreviewDeps, onResult: (r: R) => void, onError: (e: unknown) => void, delayMs?: number): Previewer<R>;
```

- [ ] **Adım 1: Başarısız testler yaz** (hepsi saf). Kritik vakalar:

```ts
// designer-model.test.ts
it("groupIssues: columns[1].key alan yoluna, old.* ve bilinmeyenler general'a", () => {});
it("issuesFor: 'columns' isteği columns[0].key hatalarını da kapsar; 'name' yalnızca name", () => {});
it("applyGate: kilitli → blocked locked; errors var → invalid; empty → empty; viewIsCurrent=false → stale; pending → pending; applying → busy; bağlı değil → disconnected", () => {});
it("applyGate: risk safe → ready; destructive → needsConfirm; data_loss+production → needsProductionName(expected=serverName); data_loss+production değil → needsConfirm", () => {});
it("canSubmit/confirmFlags: onay kutusu olmadan gönderilemez; ad tam eşleşmeli (büyük/küçük harf duyarlı, kenar boşlukları kırpılır); confirmedProduction yalnızca needsProductionName'de true", () => {});
it("scriptWithRanges: aralıklar ifade metnini tam kapsar, ';' ve boş satır ayırıcılar aralık dışı", () => {});
it("isSafeIntegerString: '+5','-0','007' tamam; '', ' 5', '1e3', '9223372036854775808' (aralık dışı) hayır", () => {});
it("defsEqual/cloneDef: derin kopya bağımsız; alan sırasından bağımsız eşitlik", () => {});
it("indeks kolon işlemleri değişmezdir ve sınır dışı indeksleri yok sayar", () => {});

// preview.test.ts
it("150 ms debounce: üç hızlı schedule → tek run (sonuncusu)", () => {});
it("eski yanıt geç gelirse (yeni schedule sonrası) onResult çağrılmaz", async () => {});
it("cancel/dispose bekleyen zamanlayıcıyı ve uçuştaki yanıtı yok sayar", async () => {});
it("run reddederse onError (yalnızca son çağrı için) ve sonraki schedule çalışır", async () => {});
```

- [ ] **Adım 2:** `bun run test src/features/designer` — başarısızlığı gör. **Adım 3: Uygula.** **Adım 4:** `bun run test`, `bun run typecheck`, `bun run build` (ana `index-*.js` boyutunu **başlangıç ölçümü olarak** not et — sonraki görevlerde bütçe bu değere göredir).
- [ ] **Adım 5: Commit** (`feat(designer): ddl* komutları; saf modeller (hata eşleme, uygulama kapısı, önizleme zamanlayıcı)`).

---

### Task 2: `designer-controller` — durum makinesi

**Dosyalar:**
- Oluştur: `src/features/designer/designer-controller.ts` ve `.test.ts`, gerekirse `src/features/designer/errors.ts` (+ test)

**Arayüzler:**
- Tüketir: Task 1 komutları/modelleri; `ToastKind` bildirimi; sunucu kaydı sorgusu.
- Üretir:

```ts
export type DesignerMode = "create" | "alter";
export type DesignerStatus = "loading" | "ready" | "disconnected" | "gone";   // gone: nesne/sunucu yok
export interface DesignerModel {
  id: string;                         // sekme kimliği
  serverId: string; serverName: string; databaseOid: number;
  mode: DesignerMode; kind: ObjectKind;
  object: ObjectRef | null;           // alter: yüklenen nesne; create: başarıdan sonra created ile dolar
  original: ObjectDef | null;         // alter: ddl_load çıktısı (DEĞİŞTİRİLMEZ, `old` olarak gider); create: null
  draft: ObjectDef | null;            // formun düzenlediği tanım
  status: DesignerStatus;
  error: string | null;               // yükleme/bağlantı hatası metni
  dirty: boolean;
  view: PlanView | null; viewFor: number;       // son önizleme ve hangi taslak sürümüne ait olduğu
  version: number;                    // taslak sürümü (her değişiklikte +1)
  pending: boolean;                   // önizleme zamanlayıcıda/uçuşta
  previewError: string | null;
  applying: boolean; applyId: string | null;
  outcome: ApplyOutcome | null;       // son Applied/Failed (Failed → vurgulama/uyarı)
  gateError: string | null;           // Err(payload) kapı hatası metni (code'a göre)
  lockTimeoutMs: number;              // varsayılan 5000
  confirm: ConfirmState;
  locked: boolean; production: boolean;         // sunucu kaydından (kayıt değişince güncellenir)
  concurrently: boolean;              // yalnız index create (Task 6'da UI)
  cascade: boolean;                   // drop dialog
}
export function createDesignerController(deps: {
  api: Pick<Api, "ddlDefaults"|"ddlLoad"|"ddlLookup"|"ddlPreview"|"ddlApply"|"ddlCancel">;
  notify(text: string, kind: ToastKind): void;
  serverInfo(serverId: string): { name: string; locked: boolean; production: boolean } | null;
  refreshNode(nodeId: string): Promise<void> | void;        // Explorer yenileme
  previewDelay?: number; previewDeps?: Partial<PreviewDeps>; newId?: () => string;   // test için
}): {
  openCreate(input: { tabId: string; serverId: string; databaseOid: number; create: DdlCreate; refreshNodeId: string | null }): Promise<void>;
  openAlter(input: { tabId: string; serverId: string; databaseOid: number; object: ObjectRef; refreshNodeId: string | null }): Promise<void>;
  model(tabId: string): DesignerModel | undefined;
  edit(tabId: string, f: (draft: ObjectDef) => ObjectDef | void): void;   // taslağı değiştirir (kopya üstünde), version++, dirty, önizlemeyi zamanlar
  setLockTimeout(tabId, ms): void; setConfirm(tabId, c: Partial<ConfirmState>): void; setConcurrently(tabId, v): void; setCascade(tabId, v): void;
  previewNow(tabId): void;
  apply(tabId): Promise<void>;                              // kapı → ddlApply → sonuç işleme
  cancelApply(tabId): Promise<void>;
  reload(tabId, opts?: { force?: boolean }): Promise<void>; // alter: ddl_load; dirty ise çağıran önce onay almış olmalı (force)
  lookup(tabId, request: LookupRequest): Promise<LookupItem[]>;   // sekme/sunucu başına bellek önbelleği (aynı istek tek uçuşta)
  serverChanged(serverId: string): void;                    // kayıt değişti: locked/production/ad yeniden hesaplanır
  serverSessionsClosed(serverId: string, gone: boolean): void;    // sekmeler disconnected/gone; uçuştaki apply iptal çağrılır
  close(tabId: string): void;                               // zamanlayıcı/uçuştaki yanıtlar geçersiz; uçuştaki apply için ddlCancel
  isDirty(tabId): boolean;
  peekApplyGate(tabId): ApplyGate;                          // arayüz için hesaplanmış kapı
};
```

**Davranış:**
- **Açılış:** `openCreate` → `ddlDefaults(kind, context)` → `draft = defaults`, `original = null`, `dirty = false`, ilk önizleme *zamanlanmaz* (boş/eksik form doğrulama hatası verir; ilk önizleme yalnızca ilk düzenlemede ya da `previewNow` ile). `openAlter` → `ddlLoad(object)` → `original = load`, `draft = cloneDef(load)`, `dirty = false`; yükleme hatası → `status = "gone"` (`not_found`) ya da `"disconnected"` (bağlantı kodları) ya da genel hata (`error`).
- **Önizleme:** her `edit` → `version++`, `dirty = !defsEqual(draft, original ?? default)` (create'te dirty = varsayılandan farklı), `pending = true`, `previewer.schedule(() => ddlPreview(request(draft)))`; yanıt `view = r`, `viewFor = version at schedule time` (yalnızca hâlâ en güncel sürümse: `viewIsCurrent = viewFor === version`); hata → `previewError` (bağlantı kodları durumu `disconnected` yapar). `request(draft)`: create → `{op:"create", def: draft, concurrently}`; alter → `{op:"alter", object, old: original, new: draft}` (**`old` her zaman `original`'in dokunulmamış kopyasıdır**; denetleyici `original`'i hiçbir yerde değiştirmez, test eder).
- **Uygulama:** `apply` → `peekApplyGate`; `ready/needsConfirm/needsProductionName` ve `canSubmit` değilse **hiçbir şey göndermez** (sessizce çıkar + `gateError` yok); `applying` ise çıkar; `ddlApply(request(draft), applyOptionsFor(...))` (`applyId = newId()`). Sonuç: `Applied` → toast + `dirty = false`; create ise `created` doluysa moda geçiş (`mode = "alter"`, `object = created`, `ddlLoad` ile yeniden yükle; `created` yoksa sekme "tamam" durumunda kapanmaya hazır: `status = "ready"`, `outcome` saklanır ve arayüz kapat düğmesi sunar); alter ise `reload`; her durumda `refreshNode(refreshNodeId)`. `Failed` → `outcome` saklanır, `committed > 0` ise toast uyarısı, `applying = false`, taslak korunur, `55P03` metni; `Err(payload)` → kapı hatası: `plan_changed` → `gateError = "…yeniden yükle"` + `reload` önerisi (dirty ise otomatik yükleme YOK), `invalid_def` → önizlemeyi yeniden iste, `locked` → `locked = true`, bağlantı kodları → `disconnected`, `not_found` → `gone`, diğer → `errorText`.
- **Kapı girdisi** (`peekApplyGate`): `locked`/`production` sunucu kaydından (`serverInfo`), `viewIsCurrent`, `pending`, `applying`, `connected = status === "ready"`.
- **Yaşam döngüsü:** `serverSessionsClosed` → ilgili sekmeler `disconnected` (silindiyse `gone`), zamanlayıcı iptal, uçuştaki apply varsa `ddlCancel(applyId)` (hata yok sayılır); `serverChanged` → `locked/production/serverName` yenilenir ve (kilitlendiyse) onay durumu sıfırlanır; `close` → önizleyici dispose, uçuştaki yanıtlar yoksayılır (kuşak sayacı), uçuştaki apply için `ddlCancel`.
- **Lookup önbelleği:** anahtar `serverId|databaseOid|JSON(request)`; aynı anahtar uçuşta tek istek; hata önbelleğe alınmaz; `roles/schemas/tablespaces/available_extensions/index_methods` oturum süresince tutulur, `tables/table_columns/extension_versions` 30 sn.

- [ ] **Adım 1: Başarısız testler** (sahte `Api` + sahte zamanlayıcı; `QueryController`/`DashController` test kalıbı):

```ts
it("openCreate: defaults yüklenir, dirty=false, önizleme zamanlanmaz", async () => {});
it("openAlter: original/draft ayrı derin kopyalar; old her zaman original'in kopyası (düzenlemeler original'e sızmaz)", async () => {});
it("edit: version++, dirty, 150 ms sonra tek ddlPreview; hızlı üç edit → tek istek", async () => {});
it("bayat önizleme: yanıt geldiğinde version ilerlediyse viewIsCurrent=false, apply kapısı 'stale'", async () => {});
it("apply: kapı ready değilken ddlApply çağrılmaz (invalid/empty/stale/pending/locked/disconnected)", async () => {});
it("apply: needsConfirm onay kutusu olmadan çağrılmaz; kutu + canSubmit ile confirmed=true gider", async () => {});
it("apply: production+data_loss ad yazılmadan çağrılmaz; doğru adla confirmedProduction=true", async () => {});
it("apply çift tıklama: uçuştayken ikinci apply ikinci istek üretmez", async () => {});
it("Applied(create, created=ref): mode alter'a geçer, yeniden yüklenir, refreshNode(refreshNodeId) çağrılır, toast", async () => {});
it("Applied(alter): reload + refreshNode; dirty=false", async () => {});
it("Failed{failedIndex,committed>0}: outcome saklanır, taslak korunur, uyarı toast'ı; sqlstate 55P03 kullanıcı metni", async () => {});
it("Err plan_changed: gateError + otomatik reload YOK (dirty iken); dirty değilse reload önerilir/yapılır (karar: yapılmaz, yalnızca öneri)", async () => {});
it("Err invalid_def → önizleme yeniden istenir; Err locked → locked=true; Err not_found → gone; bağlantı kodları → disconnected", async () => {});
it("serverSessionsClosed: sekmeler disconnected/gone, zamanlayıcı iptal, uçuştaki apply için ddlCancel", async () => {});
it("close: uçuştaki önizleme/apply yanıtı durumu değiştirmez; ddlCancel çağrılır", async () => {});
it("serverChanged: locked/production yeniden hesaplanır; onay sıfırlanır", async () => {});
it("lookup önbelleği: aynı istek tek uçuşta, hata önbelleğe alınmaz, TTL (30 sn) sahte saatle", async () => {});
it("drop için cascade bayrağı request'e girer (drop request'i: object, def=original, cascade)", async () => {});
```

- [ ] **Adım 2:** Uygula (`DashController`/`QueryController` kalıbı: kuşak sayacı + `alive()` kontrolü; Solid `createStore`/signal ile model; tüm model güncellemeleri yeni nesne). `errors.ts`: `designerErrorText(p: ErrorPayload)` → `message — detail — hint` (query kodları için) ve kod → davranış eşlemesi (`isConnectionCode`, `isGateCode`) tablosu + testler; dashboard'taki `errorText` ile aynı davranış gerekiyorsa ortak yardımcıya taşı (bkz. Global Constraints).
- [ ] **Adım 3:** `bun run test`, `bun run typecheck`, `bun run build` yeşil.
- [ ] **Adım 4: Commit** (`feat(designer): designer denetleyicisi (taslak, önizleme, kapılı uygulama, yaşam döngüsü)`).

---

### Task 3: `designer` sekme türü ve kabuk (SqlPreview, ApplyBar, DesignerView)

**Dosyalar:**
- Değiştir: `src/features/query/tabs-store.ts` (+ `tabs-store.test.ts`), `src/features/query/QueryTabs.tsx`
- Oluştur: `src/features/designer/DesignerTabs.tsx`, `DesignerView.tsx`, `SqlPreview.tsx`, `ApplyBar.tsx`, `FormParts.tsx`, `src/app/strings-designer.ts`

**Arayüzler:**
- Üretir (`tabs-store.ts`; dashboard kalıbı): `DesignerTab { id: string; serverId: string; databaseOid: number; title: string; key: string }` (`key`: dedupe anahtarı — create için `create:${kind}:${serverId}:${databaseOid}:${schemaOid ?? ""}:${tableOid ?? ""}:${n}` (aynı bağlamda birden çok "yeni" sekmesi serbest → `n` artar), alter için `alter:${kind}:${serverId}:${databaseOid}:${oid}` (ikincisi mevcut sekmeyi etkinleştirir)); `TabsState.designers: DesignerTab[]`; `openDesigner(state, input): { state, id, created }`; `isDesignerTabId`; `closeTab/activateTab/renameTab` üç türü tanır; kapatma sonrası etkin sekme sırası: sorgu → dashboard → designer → "Nesne".
- Üretir (kabuk): `DesignerTabs.tsx` (tembel; `DesignerTabHeads` + `DesignerPanels`, `DashTabs.tsx` kalıbı; panel her zaman bağlı, gizlisi `invisible`), `DesignerView.tsx` (üst çubuk: kind rozeti, sunucu/veritabanı, durum rozeti (`loading/ready/disconnected/gone`), **Yeniden yükle**; gövde: `kind`'a göre form bileşeni (Task 5–6'ya kadar "Yükleniyor…/desteklenmiyor" yerine **bölüm listesi veri olarak** tanımlı: `FORMS: Record<ObjectKind, LazyComponent>`; henüz form dosyası olmayan türler için `FormMissing` bileşeni YOK — görev sonunda dört tür de bağlı olmalı; bu görevde formlar yer tutucu dosya olarak **eklenmez**, `FORMS` yalnızca Task 5–6'da dolar ve bu görevin testleri `kind` eşlemesini sahte bileşenle sınar); sağ/alt: `SqlPreview` + `ApplyBar`), `SqlPreview.tsx` (salt okunur CodeMirror — `createReadOnlyEditor`/`replaceDoc`; metin `scriptWithRanges(view.statements).text`; başarısız ifade (`outcome.failedIndex`) satır arka planı vurgusu (`Decoration.line` ya da `EditorView.decorations` + CSS değişkeni); ifade başına risk rozeti listesi (metnin yanında/altında küçük liste: sıra, risk, `transactional=false` için "transaction dışı" rozeti, `note`); uyarılar (`view.warnings`) şeridi; `view.errors` formda görünür, genel olanlar (`groupIssues().general`) önizleme üstünde; boş durumlar: "Formu tamamlayın" (`errors.length>0`), "Değişiklik yok" (`empty && errors.length===0`), "Önizleme hazırlanıyor…" (`pending`); **Kopyala** düğmesi (app clipboard yardımcısı; hata → toast)), `ApplyBar.tsx` (risk özeti rozeti, `lock_timeout` seçici 1 sn/5 sn/30 sn/sınırsız, `applyGate` sonucuna göre: "Etkisini anladım" onay kutusu, üretimde sunucu adı yazma alanı ("Onaylamak için sunucu adını yazın: <ad>"), **Uygula** (`canSubmit` + `!applying`), uçuşta **İptal** (`cancelApply`), **Query Tool'da aç** (`onOpenScript(text)` — Task 4 bağlar; bu görevde prop olarak alınır), **Vazgeç/Kapat**; `gateError`/`outcome.failed` `role="alert"` şeridi (`committed > 0` uyarısı, PG `position` bilgisi ifade numarasıyla); kilitliyse tüm değiştiren düğmeler kapalı + `title`), `FormParts.tsx` (ortak: `FieldRow({label, path, issues, hint, children})` — etiket + hata metni `aria-describedby`; `OwnerSelect` (roller lookup'ı, boş = varsayılan, **Select** yerine yazılabilir `datalist` girdisi — yüzlerce rol olabilir), `CommentField` (textarea), `FreeSqlNote`).
- Metinler: `src/app/strings-designer.ts` (`tdes`): sekme başlığı kurucuları (`createTitle(kind, server, db?)` → "Oluştur: Table · sunucu/db"; `editTitle(kind, name)` → "Düzenle: ad"), durum metinleri, ApplyBar etiketleri, risk adları (`safe`: "Güvenli", `destructive`: "Yıkıcı", `data_loss`: "Veri kaybı olabilir"), uyarı/hata metinleri, boş durumlar, kind etiketleri.

- [ ] **Adım 1: Başarısız testler.** `tabs-store.test.ts`: designer açma (kimlik çakışması yok — `taken()` üç türü kapsar), alter dedupe (aynı `key` mevcut sekmeyi etkinleştirir, `created=false`), create çoklu açma (`n` artar), `closeTab` sıralaması (sorgu → dashboard → designer → Object), `renameTab`, mevcut testler bozulmaz. `designer-model` tarafında (Task 1'de test edilen) yardımcıların bu görevde eklenen kullanımları için ek test gerekmez; bileşen mantığı ince tutulur, ancak `SqlPreview`'ın kullandığı saf yardımcılar (`statementRangesToLineRanges`, `previewEmptyState(view, pending)`) **saf işlevler** olarak `designer-model.ts`'e eklenir ve test edilir.
- [ ] **Adım 2: `activeId` denetimi (dashboard Task 3 dersi).** `activeId` artık designer kimliği olabilir: `query-controller.ts` (`prevActive`, `activate`, `requestClose`, `rename`), `QueryTab.tsx` (`activeId()`/`switchTo`), `QueryTabs.tsx` (`isObject`, F5 kuralı: designer etkinken F5 sayfayı **yenilemez** ve sorguyu çalıştırmaz — `previewNow` çağırır), taslak kaydetme/geri yükleme yolları: designer kimliği `ctl.model(id)` ile **sorgulanmamalı**, taslak kaydedilmemeli. Sorgu paneli designer etkinken gizli (`invisible`). Ctrl+F (App.tsx) etkilenmez.
- [ ] **Adım 3: Sekme çubuğu.** `QueryTabs` designer sekmelerini sorgu → dashboard → designer sırasıyla çizer (lucide `PencilRuler`/`Wrench`), sunucu rengi/etiketi/kilit rozeti sorgu sekmesiyle aynı `TabHead` kabuğundan; kirli sekme için nokta + sr-only "kaydedilmemiş"; `status` `disconnected/gone` ise soluk. **Kapatma:** kirli ise onay (`CloseTxDialog` kalıbında küçük `ConfirmDialog`: "Kaydedilmemiş değişiklikler atılsın mı?", odak **Vazgeç**), değilse doğrudan (`designer.close` + `tabs.close`); orta tık/Delete aynı yol. `beforeExit` (pencere kapanışı): kirli designer varsa mevcut toplu onay akışına **eklenmez** — uyarı gösterilmez, arka uç uçuştaki uygulamayı iptal eder, kirli taslaklar kaybolur (bilinçli karar; plan `Review Focus`'ta belgelendi) — bunu `docs/smoke-test.md` maddesi olarak yaz.
- [ ] **Adım 4: Doğrulama.** `bun run test`, `bun run typecheck`, `bun run build` (ana boyut bütçesi; tembel parçalar ayrı; `DesignerTabs-*.js`, `DesignerView-*.js`, `strings-designer-*.js` ayrı).
- [ ] **Adım 5: Commit** (`feat(designer): designer sekme türü ve kabuk (SQL önizleme, uygulama çubuğu, sekme çubuğu entegrasyonu)`).

---

### Task 4: Explorer entegrasyonu — menü eylemleri, açma akışı, App bağlama

**Dosyalar:**
- Değiştir: `src/features/explorer/actions.ts` (+ `actions.test.ts`), `src/features/explorer/ExplorerMenu.tsx`, `src/app/App.tsx`, `src/app/strings.ts` (yalnızca `t.menu.*` yeni etiketler), `src/features/query/query-controller.ts` (`openScript` — aşağıda)
- Oluştur: `src/features/designer/designer-open.ts` (+ test), `src/features/designer/ddl-kinds.ts` (menü görünürlüğü için tür kümeleri; ana pakette küçük, bağımlılıksız)

**Arayüzler:**
- Üretir (`ddl-kinds.ts`): `DDL_CREATE_COLLECTIONS: NodeKind[] = ["schemaCollection","sequenceCollection","extensionCollection","indexCollection"]`, `DDL_OBJECT_KINDS: NodeKind[] = ["schema","sequence","extension","index"]`, `canCreateIn(kind)`, `canEditObject(kind)`; yalnızca menü görünürlüğü içindir (arka uç `ddl_target_of` yine reddeder; `kind` kümesi dışı düğümde menü gösterilmez).
- Üretir (`actions.ts`): `ActionId`'ye `"createObject" | "editObject" | "renameObject" | "dropObject"`; `actionsFor`: koleksiyon düğümünde (`canCreateIn`) `createObject` (`groupOf` → `-1` grubunda, Query Tool eyleminden önce/yanında); nesne düğümünde (`canEditObject`) `editObject`, `renameObject`, `dropObject`; sunucu **bağlıysa** (ağaçta zaten öyle olduğundan ek koşul yok); `ActionDeps` ek: `openDesigner?(id: string, mode: "create" | "edit" | "rename" | "drop"): void`, `serverLocked?(nodeId: string): boolean`. **Kilitli sunucuda** bu dört eylem listede **kalır ama devre dışıdır** (menüde soluk, `title` gerekçesi: "Sunucu salt okunur kilitli"); bunun için `actionsFor` yanında `disabledReason(action, deps, nodeId): string | null` saf işlevi (test) ve `ExplorerMenu`'de `ContextMenu.Item disabled` + `title` desteği (mevcut öğelerin davranışı değişmez).
- Üretir (`designer-open.ts`): `createDesignerOpener(deps)`:

```ts
export interface DesignerOpenDeps {
  designer: DesignerController; tabs: Pick<TabsStore, "state" | "openDesigner" | "close">;
  api: Pick<Api, "ddlTargetOf">; tree: () => TreeState;
  refreshServers: () => Promise<ServerConfig[]>;
  connect: (serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
  openDrop(input: { serverId: string; databaseOid: number; object: ObjectRef; refreshNodeId: string | null; name: string }): void;   // Task 7 diyaloğu
}
export function createDesignerOpener(deps): { open(nodeId: string, mode: "create" | "edit" | "rename" | "drop"): Promise<void> };
```

Akış: `ddlTargetOf(nodeId)` → `create`/`object` yoksa "Bu düğüm için düzenleme yok" toast'ı; `databaseOid == null` → aynı; sunucu kaydı (`refreshServers`) **kilitliyse** değiştiren eylem toast'la reddedilir (menü zaten kapalı; savunma); `connect(serverId)` (gerekirse parola); `mode`: `create` → `openDesigner` (`key` create); `edit`/`rename` → alter `key`; sekme zaten varsa `created=false` ve `rename` isteği ise `designer.focusField(tabId, "name")` (denetleyiciye küçük `focusRequest` sinyali: form `name` alanına odaklanır); `drop` → `openDrop(...)` (sekme açmaz). Tek seferde bir açma (`opening` bayrağı); hata → `notify(errorText)`. `refreshNodeId` denetleyiciye iletilir.
- `query-controller.ts`: yeni dışa açık `openScript(input: { serverId: string; databaseOid: number | null; text: string }): Promise<void>` — sunucu kaydı + ortam sorgusu (mevcut `promptEnvironment`) akışını `openFromNode` gibi izler ama düğüm yerine verilen hedefle ve `text` ile sekme açar (`addTab({... text, activate: true})`); çalıştırmaz. Testi `query-controller.test.ts` kalıbıyla.
- `App.tsx`: `createDesignerController` (tembel: ilk "Oluştur/Düzenle"de `import("../features/designer/designer-open")`; dashboard'daki `dashLoad ??=` kalıbı — iki hızlı tıklamada tek kurulum), `actionDeps.openDesigner`, `actionDeps.serverLocked`, sunucu kaydı değiştiğinde (`serversChanged`/`refreshServers` sonrası) `designer.serverChanged(serverId)`, `query.sessionsClosed` kancasına (dashboard'un bağlandığı yere) `designer.serverSessionsClosed`, `QueryTabs`'a `designer` ve `onOpenScript` (designer `ApplyBar` → `query.openScript`) bağlama; Explorer yenileme `explorer.controller.refresh(nodeId)` (düğüm artık yoksa `nodeGone` yolu mevcut hata işleyişiyle).

- [ ] **Adım 1: Başarısız testler.** `actions.test.ts`: `actionsFor` koleksiyonlarda `createObject`, nesne düğümlerinde `editObject/renameObject/dropObject` içerir; tablo/kolon/rol/server/view/database düğümlerinde içermez; `disabledReason` kilitli sunucuda dolu, değilse `null`; `executeAction` `deps.openDesigner`'ı doğru `mode` ile çağırır; kilitli sunucuda `executeAction` **çağırmaz** (savunma). `designer-open.test.ts` (sahte api/tree/tabs/designer): create/edit/rename/drop akışları; `ddlTargetOf` boş → toast; `databaseOid=null` → toast; kilitli kayıt → reddet; `connect` false → sekme açılmaz; ikinci edit mevcut sekmeyi etkinleştirir; `rename` mevcut sekmede alan odağı ister; tek seferde bir açma (deferred gate). `query-controller.test.ts`: `openScript` metni sekmeye koyar, çalıştırmaz, kilitli sunucuda da açar (yalnızca çalıştırma kapısı değişmez).
- [ ] **Adım 2: Uygula** ve `ExplorerMenu` devre dışı öğe desteği (mevcut menü testleri yeşil).
- [ ] **Adım 3: Doğrulama.** `bun run test`, `bun run typecheck`, `bun run build` (ana boyut: `ddl-kinds.ts` + eylem etiketleri + App kancaları dahil bütçe içinde).
- [ ] **Adım 4: Commit** (`feat(designer): Explorer menü eylemleri (kilitliyse devre dışı), açma akışı, App ve Query Tool bağlaması`).

---

### Task 5: Schema ve Extension formları (+ ortak form parçaları)

**Dosyalar:**
- Oluştur: `src/features/designer/SchemaForm.tsx`, `ExtensionForm.tsx`; Değiştir: `FormParts.tsx`, `DesignerView.tsx` (`FORMS` eşlemesi), `strings-designer.ts`

**Arayüzler:**
- Tüketir: denetleyici (`edit`, `lookup`, `model`), `groupIssues/issuesFor`.
- Form bileşeni sözleşmesi (tüm tür formları): `Component<{ model: DesignerModel; edit: (f: (d: ObjectDef) => void) => void; issues: FieldIssues; lookup: (r: LookupRequest) => Promise<LookupItem[]>; readOnly: boolean; focusField?: string }>`; `readOnly` = `locked || status !== "ready" || applying`; alanlar `readOnly` iken `disabled`.
- **SchemaForm:** `name` (zorunlu; alter'da **yeniden adlandırma** olarak etiketlenir ve `focusField = "name"` odaklar), `owner` (`OwnerSelect`, roller lookup'ı), `comment`; sistem şeması uyarısı (arka uç reddeder; hata metni `name` altında görünür).
- **ExtensionForm:** `name` — **create**'te `available_extensions` lookup'ından aranabilir liste (yazılabilir girdi + `datalist`; `detail` "default <v> · yüklü <v|—> · açıklama" yan metin), **alter**'da salt okunur (ad değişimi desteklenmez; hata uyarısı); `schema` (şemalar lookup'ı; boş = `null` = değişmez/varsayılan); `version` — create'te boş = varsayılan sürüm, alter'da `extension_versions` lookup'ından seçilen sürüm (mevcutten farklıysa `UPDATE TO` notu gösterilir: "Eklentinin güncelleme betiği çalışır"); `cascade` (yalnız create: "Bağımlı eklentileri de kur (CASCADE)").

- [ ] **Adım 1: Başarısız testler.** Bileşen mantığı ince; test edilecek saf yardımcılar `designer-model.ts`/`forms-model.ts`'e: `extensionOptionLabel(item)`, `normalizeEmpty(s)` (`""` → `null`), `schemaEditPatch`/`extensionEditPatch` (taslak kopyası üzerinde alan güncelleme işlevleri), `issuesFor` eşlemeleri (alan yolları sözleşmesi). Denetleyici testleri Task 2'de.
- [ ] **Adım 2: Uygula** formlar (Field/inputClass kalıbı, `FieldRow` ile hata gösterimi, klavye ve etiketler); `FORMS` eşlemesine `schema`, `extension` bağla.
- [ ] **Adım 3: Doğrulama** `bun run test`, `bun run typecheck`, `bun run build`.
- [ ] **Adım 4: Commit** (`feat(designer): Schema ve Extension formları`).

---

### Task 6: Sequence ve Index formları

**Dosyalar:**
- Oluştur: `src/features/designer/SequenceForm.tsx`, `IndexForm.tsx`, `forms-model.ts` genişletmeleri; Değiştir: `DesignerView.tsx`, `strings-designer.ts`

**Arayüzler:**
- **SequenceForm:** `schema` (create: şemalar lookup'ı, bağlamdan önceden dolu), `name`, `owner`, `dataType` (smallint/integer/bigint seçici), `increment`, `minValue`, `maxValue`, `start`, `cache` (hepsi **string metin girdisi**; sayısal doğrulama `isSafeIntegerString` ile *yalnızca biçim* uyarısı — asıl doğrulama arka uçtan `FieldError`), `cycle`, `ownedBy` (üç parça: `schema` + `table` (tablolar lookup'ı) + `column` (`table_columns` lookup'ı); "Yok" seçeneği), `comment`. **Yön değişimi yardımcısı** (saf, `forms-model.ts`): `increment` işareti değişince (artı↔eksi) `start/min/max` birlikte uyumlu varsayılana çekilir — artan: `min = 1|null`, `max = null`, `start = "1"`; azalan: `min = null`, `max = "-1"`, `start = "-1"` — kullanıcı daha önce **elle** girdiği değerleri korumak için yalnızca alan *boşsa/varsayılansa* değiştirilir; test edilir. `dataType` değişince açık sınırlar (smallint/integer) hatırlatması (not). Alter'da: `start` değişimi için "Mevcut sayacı değiştirmez" notu, **yön değişimi** için "Sayaç sıfırlanır" uyarısı (arka ucun planındaki uyarılar zaten önizlemede; forma yalnızca ön bilgi), `ownedBy` + `owner`/`schema` birlikte değişiminin sunucu kuralları (arka uç plan sırasını yönetir; form yalnızca `ownedBy.schema === schema` ipucu verir).
- **IndexForm:** `schema`+`table` (create'te bağlamdan salt okunur; alter'da salt okunur — tablo taşıma desteği yok, yapısal değişiklik = yeniden oluşturma), `name`, `method` (`index_methods` lookup'ı), `unique`, kolon listesi (satır: anahtar türü seçici **Sütun** (tablo kolonları lookup'ı, yazılabilir) / **İfade** (metin alanı + `FreeSqlNote`), `opclass`, `collation`, `descending`, `nullsFirst` (üçlü: varsayılan/FIRST/LAST) — `descending`/`nullsFirst` yalnız `method === "btree"` iken etkin; satır ekle/sil/yukarı/aşağı taşı klavyeyle erişilebilir), `include` listesi (yalnız sürüm destekliyorsa: sürüm bilgisi form modelinde yoksa alan **her zaman gösterilir**, arka uç hatası metni öğretici gösterilir), `whereClause` (+ `FreeSqlNote`), `tablespace` (tablespaces lookup'ı; **temizleme yok**: seçenek "varsayılan (pg_default)" → `pg_default` yazar; `null` olarak gelen için boş görünür ama kullanıcı boşalttığında değişiklik gönderilmez), `fillfactor` (10–100 sayı girdisi; `gin/brin` için devre dışı), `comment`; create'te **`concurrently`** onay kutusu (denetleyici `setConcurrently`; "Tabloyu kilitlemeden oluşturur; transaction dışında çalışır; hata olursa geçersiz indeks kalabilir" notu); alter'da **tanım değişikliği uyarısı** (yapısal alanlardan biri değiştiyse "İndeks yeniden oluşturulur; tablo okuma/yazmaya kapanır" — metni arka ucun `warnings`'ından alma, ek olarak formda sabit bilgi); `constraint` doluysa **tüm form salt okunur** + banner "Bu indeks '<kısıt>' kısıtına aittir; kısıtı tablo düzenleyicisinden değiştirin" (silme/yeniden adlandırma da kapalı: menü yine açabilir, banner açıklar); `valid === false` ise banner "Geçersiz indeks (başarısız CONCURRENTLY inşası); silip yeniden oluşturun".

- [ ] **Adım 1: Başarısız testler (saf, `forms-model.test.ts`):** yön değişimi yardımcısı (artan→azalan boş alanları günceller, elle girilmiş değerleri korur, azalan→artan, `dataType` bağımsız), indeks kolon listesi işlemleri (ekle/sil/taşı/güncelle değişmez; sınır dışı yok sayılır), `columnKeyLabel`, `nullsFirstToSelect/selectToNullsFirst`, `btreeOnlyFields(method)`, `fillfactorEnabled(method)`, `normalizeTablespaceEdit(original, next)` (pg_default kuralı), `indexReadOnlyReason(def)` (constraint → metin, valid=false → uyarı), `structuralChange(original, draft)` (formdaki yeniden oluşturma bilgisi için; arka uçla aynı alan kümesini kullanır: method, unique, columns, include, whereClause), sequence alan biçim uyarıları.
- [ ] **Adım 2: Uygula** formlar; `FORMS`'a `sequence`, `index` bağla (dört tür tamam). Kolon satırlarında `FieldRow` yolları: `columns[i].key|opclass|collation`.
- [ ] **Adım 3: Doğrulama** `bun run test`, `bun run typecheck`, `bun run build` (bütçe).
- [ ] **Adım 4: Commit** (`feat(designer): Sequence ve Index formları`).

---

### Task 7: Silme diyaloğu, belgeler ve bütünleşik doğrulama

**Dosyalar:**
- Oluştur: `src/features/designer/DropDialog.tsx` (+ saf yardımcılar `designer-model.ts`/`drop-model.ts` + test)
- Değiştir: `src/app/App.tsx` (silme diyaloğunu bağla), `docs/smoke-test.md`, `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (durum satırı)

**Arayüzler:**
- `DropDialog` (Kobalte `Dialog`; sekme açmaz): açılırken `ddlLoad(object)` → `def` (derin kopya, **olduğu gibi** `Drop` isteğine gider) → `ddlPreview({op:"drop", object, def, cascade})`; `cascade` onay kutusu (varsayılan kapalı; değişince önizleme yenilenir, en son istek kazanır — `createPreviewer` yeniden kullanılır); plan listesi (`SqlPreview`'ın küçük sürümü: ifade + risk rozeti + uyarılar); risk onayı `applyGate` ile (silme her zaman ≥ `destructive` → onay kutusu; `cascade` → `data_loss` → üretimde sunucu adı yazma); sunucu kilitliyse düğme kapalı; **Sil** (danger stili, `--danger-fg`), **Vazgeç** (odak burada); `Failed`/`Err` hataları diyalogda şeritte (PG `detail` bağımlı nesne listesini taşır: `cascade` kapalıyken `2BP01` → "Bağımlı nesneler var; CASCADE ile silin ya da bağımlıları kaldırın" + `detail` metni); başarı → toast + `refreshNode(refreshNodeId)` + açık designer sekmesi (aynı nesne için) kapatılır ya da `gone` yapılır (denetleyiciye `objectDropped(serverId, databaseOid, object)` kancası: eşleşen alter sekmeleri `gone`); `plan_changed`/`not_found` hataları: "Nesne değişti/yok; Gezgini yenileyin".
- Saf yardımcılar (test): `dropStatusText(error)`; `dropRequest(object, def, cascade)`; `matchesObject(tab, ref)`.

- [ ] **Adım 1: Başarısız testler:** drop isteği `object` + yüklenen `def` kopyası + `cascade`; cascade değişince önizleme yeniden istenir (en son kazanır); `2BP01` hata eşlemesi (`sqlstate`/`detail`) → bağımlı nesne metni; üretim + cascade → ad yazma kapısı; kilitli → kapalı; başarılı silme sonrası aynı nesnenin designer sekmesi `gone` (denetleyici testi); çift tıklama tek istek.
- [ ] **Adım 2: Uygula** ve `App.tsx`'e bağla (`openDrop` → diyalog tek örnek, tembel).
- [ ] **Adım 3: Elle kontrol listesi** (`docs/smoke-test.md` "Nesne diyalogları (Plan 10–11)" bölümü; tüm maddeler `[—]`; mevcut `→` beklenen sonuç stili): menüler (koleksiyon: Oluştur…; nesne: Düzenle…/Yeniden adlandır…/Sil…; kilitli sunucuda soluk + gerekçe), Schema oluştur/yeniden adlandır/yorum/sahip/sil, Sequence (parametreler, yön değişimi uyarısı, OWNED BY, tür değişimi), Extension (oluştur/sürüm güncelle/şema taşı/sil), Index (oluştur: btree/hash/gin, ifade anahtarı, INCLUDE, WHERE, CONCURRENTLY; düzenle: yeniden adlandır, fillfactor, tablespace, yapısal değişiklik → drop+create uyarısı ve tablo kilidi; kısıt-bağlı indeks banner'ı; geçersiz indeks banner'ı), canlı önizleme gecikmesi ve "Formu tamamlayın/Değişiklik yok" durumları, onay kutusu ve üretimde ad yazma, `lock_timeout` seçici ve kilitli tabloda zaman aşımı mesajı (`55P03`), uygulama iptali, hata ortası `committed` uyarısı, "Query Tool'da aç", `plan_changed` (başka oturumda nesneyi değiştirip uygulama), sunucu bağlantısını kesince "bağlantı kesildi" salt okunur ve yeniden açma, sekme kapatırken kaydedilmemiş değişiklik onayı, pencere kapatırken kirli taslakların sessizce kaybolması (bilinçli), açık/koyu tema, klavye ile tam kullanım (Tab sırası, kolon satırı taşıma), ekran okuyucu etiketleri, uzun ifade/yorum alanları, `statement_timeout` olmadan uzun CREATE INDEX'te iptal.
- [ ] **Adım 4: Bütünleşik doğrulama.** `bun run test` (tümü), `bun run typecheck`, `bun run build` (ana `index-*.js` boyutu Task 1 başlangıç ölçümüne göre ≤ +3 KB; designer parçaları ve boyutları listelenir), `cargo test --workspace` (Rust değişmedi; yeşil), `cargo clippy --workspace --all-targets -- -D warnings`. Spec durum satırı "Plan 10–11" olarak güncellenir (Plan 12–13 kalan).
- [ ] **Adım 5: Kapanış greps:** `rg "console\\." src/features/designer` boş; `rg "localStorage|sessionStorage" src/features/designer` boş; `rg "dangerouslySetInnerHTML|innerHTML" src/features/designer` boş; designer modüllerinin dışından içe aktarmalar yalnızca `lazy(() => import(...))`, tip içe aktarmaları ve kabul edilen `ddl-kinds.ts` (relative yolları da ara: `rg "designer/" src --glob '!src/features/designer/**'`).
- [ ] **Adım 6: Commit** (`feat(designer): silme diyaloğu; elle kontrol listesi; durum satırları`).

---

## Kapanış kontrolü (Plan 11 bitişi)

- Dört tür için tam akış testli: açma → düzenleme → önizleme → kapılı uygulama → Explorer yenileme; silme diyaloğu.
- Ana paket bütçesi korunur; designer kodu yalnızca tembel parçalarda.
- Final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.
