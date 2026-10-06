# Dashboard Ön Yüz (Plan 9) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Plan 8 arka ucunun üstüne Dashboard arayüzü: `dashboard` sekme türü, otomatik/duraklatılabilir yenileme, Canvas grafikler, Oturumlar / Kilitler / Yapılandırma / Sorgu istatistikleri görünümleri ve iptal/sonlandırma onayı.

**Mimari:** Tüm mantık saf, test edilebilir modüllerdedir (`dash-model`, `lock-tree`, `table-model`, `chart-model`, `poller`, `signal-model`); Solid bileşenleri ince kalır. Dashboard kodu **tembel bir parça**dır (ana paket büyümez). Sekme durumu mevcut `tabs-store`'a eklenir; sorgu sekmelerinin davranışı değişmez. Her dashboard sekmesi arka uçta tek `DashId` tutar (`dash_open`), yenileme yalnızca sekme etkin ve pencere görünürken çalışır.

**Teknoloji:** SolidJS, Kobalte (Dialog/Tabs), Tailwind 4, TanStack Virtual (mevcut), Canvas 2D (yeni bağımlılık yok), vitest.

**Spec:** `docs/superpowers/specs/2026-10-06-dashboard-design.md` (§4 arayüz, §1.2 başarı ölçütleri). Arka uç sözleşmesi: `docs/superpowers/plans/2026-10-06-dashboard-p8-arka-uc.md` ve `src/ipc/bindings/*.ts`.

## Global Constraints

- **Ana paket büyümez:** dashboard modülleri yalnızca `lazy(() => import(...))` ile yüklenir; ana pakette (`src/app`, `QueryTabs.tsx`, `tabs-store.ts`) yalnızca ince kancalar ve tipler bulunur. Doğrulama: `bun run build` çıktısında ana `index-*.js` boyutu Plan 9 öncesine göre en çok +3 KB artar; dashboard kodu ayrı bir parçadır.
- **Yeni çalışma zamanı bağımlılığı yok** (grafik kendi Canvas çizimimiz).
- Arayüz metinleri Türkçe, PostgreSQL terimleri İngilizce (`Sessions`, `Locks`, `pg_stat_activity`…); tüm metinler `src/app/strings.ts` altında yeni `dashboard` bölümünde.
- Renkler yalnızca CSS değişkenlerinden (`--accent`, `--danger`, `--muted`, `--border`, `--srv-*`, …); açık ve koyu temada okunur; kalın tek taraflı vurgu kenarlığı (side-stripe) kullanılmaz — rozet/etiket kullanılır.
- Yenileme: grafik/özet **2 sn**, liste **5 sn** varsayılan; seçenekler 1/2/5/10/30 sn; **tek uçuşta istek**; sekme etkin değilse, pencere gizliyse (`document.visibilityState`) ya da duraklatılmışsa **hiç istek yok**; dönünce hemen bir yenileme.
- Grafik geçmişi yalnızca bellekte, en çok **300 örnek**; sekme kapanınca serbest.
- Sorgu metinleri (`ActivityRow.query`, `BlockSession.query`, `StatementRow.query`) **günlüğe yazılmaz** (`console.*` yok), `localStorage`'a yazılmaz; yalnızca ekranda ve kullanıcı kopyalarsa panoda.
- Hata kodları yalnızca `ErrorPayload.code` ile eşlenir, metinle asla: `42501` → "yetkiniz yok"; `invalid` → kendi oturumu; diğerleri `message`'ı gösterir. `SignalOutcome` `notFound` → "oturum artık yok".
- `ActivityRow.isSelf` kendi bağlantıyı işaretler (`DashInfo.selfPid` bayatlayabilir); kendi satırında eylemler kapalıdır.
- `query == null` satırlarda boş bırakılmaz: "yetki yok" (soluk, italik) gösterilir. Yetkisiz rolde sayımlar kısmidir; özet kartında "bazı oturumlar görünmüyor" notu.
- Erişilebilirlik: tablolarda `role="grid"`/`row` ve klavye ile satır seçimi (Yukarı/Aşağı, Enter = ayrıntı, Delete = iptal isteği); grafikte `aria-label` + gizli tablo; diyalog odağı **Vazgeç**'te başlar (Enter yıkıcı eylemi onaylamaz).
- Commit eki: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- Test: `bun run test` (vitest), `bun run typecheck`; cargo testleri bu plan boyunca değişmemelidir (`cargo test --workspace` yine yeşil). Git worktree **kullanılmaz**; `src/ipc/bindings`'e satır sonu gürültüsü düşerse `git checkout -- src/ipc/bindings`.
- Elle arayüz kontrolü yalnızca kullanıcıdadır; ajan çalıştırıcılar GUI'yi elle denemez (Playwright vb. yok); doğrulama vitest + typecheck + `bun run build`.

## Review Focus

- **İstek fırtınası:** sekme arka plandayken, pencere gizliyken ya da duraklatılmışken sıfır istek; aralık değişince çift zamanlayıcı yok; önceki istek bitmeden yenisi başlamaz; kapanan sekmede uçuştaki yanıt durumu bozmaz.
- **Sayaç sıfırlama / uyku:** `pg_stat_reset` veya yeniden bağlanma sayacı geriye götürürse negatif oran çıkmaz (çizgide boşluk); iki örnek arası beklenen aralığın 5 katından uzunsa (bilgisayar uyudu) boşluk bırakılır; ilk örnek yalnızca taban çizgisidir.
- **İptal/sonlandırma güvenliği:** onay diyaloğu o anki canlı satırı gösterir; onay sırasında satır kaybolduysa ya da `pid` aynı kalıp kullanıcı/uygulama değiştiyse (pid yeniden kullanımı) işlem yapılmaz ve kullanıcı uyarılır; kendi satırı, pg-tools'un kendi sorgu sekmesi (`application = "pg-tools query"`) ve açık transaction'lı oturum için ek uyarı; çift tıklama çift sinyal göndermez.
- **Engelleme zinciri:** kilit döngüsü (A→B→A deadlock'a yakın durum) sonsuz döngüye/özyinelemeye girmez; `blockedBy` içinde `0` (hazır transaction) "hazır transaction" olarak gösterilir ve eylem sunmaz; aynı engelleyiciyi bekleyen çok sayıda oturum doğru gruplanır.
- **Bağlantı/sunucu yaşam döngüsü:** sunucu bağlantısı kesilir ya da silinirse dashboard sekmesi "bağlantı kesildi" durumuna geçer, yenilemeyi durdurur ve yeniden açma düğmesi sunar; kapanmış `DashId` ile gelen `not_found` hatası kullanıcıya ham hata olarak değil durum olarak yansır; sekme kapanınca `dash_close` çağrılır.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `src/ipc/commands.ts` | `dash*` komutları (10) |
| `src/features/dashboard/dash-model.ts` | Halka tampon, sayaç → oran dönüşümü, oturum serisi, süre/boyut biçimleyiciler |
| `src/features/dashboard/lock-tree.ts` | Engelleme zincirinden ağaç kurma (döngü güvenli) |
| `src/features/dashboard/table-model.ts` | Oturum/ayar/sorgu satırları için süzme + sıralama |
| `src/features/dashboard/signal-model.ts` | Onay diyaloğu uyarıları ve "satır hâlâ aynı mı" kararı |
| `src/features/dashboard/chart-model.ts` | Ölçek, tick, çizgi/yığın yolu (saf) |
| `src/features/dashboard/poller.ts` | Tek uçuşta, görünürlük/duraklatma farkında zamanlayıcı |
| `src/features/dashboard/dash-controller.ts` | Dashboard sekmelerinin durumu, yenileme, açma/kapama, sinyal |
| `src/features/dashboard/DashboardView.tsx` (+ `Chart.tsx`, `OverviewView.tsx`, `SessionsView.tsx`, `LocksView.tsx`, `SettingsView.tsx`, `StatementsView.tsx`, `SignalDialog.tsx`, `DataTable.tsx`) | Bileşenler (tembel parça) |
| `src/features/query/tabs-store.ts`, `QueryTabs.tsx`, `query-controller.ts` | Dashboard sekmesini sekme çubuğuna/etkin sekmeye bağlama |
| `src/features/explorer/actions.ts`, `ExplorerMenu.tsx`, `src/app/App.tsx`, `src/app/strings.ts` | "Dashboard" eylemi, bağlama, metinler |
| `docs/smoke-test.md` | Dashboard elle kontrol listesi |

---

### Task 1: IPC komutları ve saf modeller

**Dosyalar:**
- Değiştir: `src/ipc/commands.ts`
- Oluştur: `src/features/dashboard/dash-model.ts`, `lock-tree.ts`, `table-model.ts`, `signal-model.ts` ve her birinin `.test.ts` dosyası

**Arayüzler:**
- Tüketir: `src/ipc/bindings/{DashInfo,Sample,Overview,ActivityRow,ActivityReport,LockReport,BlockSession,LockRow,Setting,PreparedXact,Statements,StatementRow,StatementsOrder,SignalKind,SignalOutcome}.ts`.
- Üretir (`commands.ts`, mevcut `call<T>` kalıbıyla):

```ts
dashOpen: (serverId: string, databaseOid: number | null) => call<DashInfo>("dash_open", { serverId, databaseOid }),
dashClose: (id: string) => call<null>("dash_close", { id }),
dashSample: (id: string) => call<Sample>("dash_sample", { id }),
dashOverview: (id: string) => call<Overview>("dash_overview", { id }),
dashActivity: (id: string) => call<ActivityReport>("dash_activity", { id }),
dashLocks: (id: string) => call<LockReport>("dash_locks", { id }),
dashSettings: (id: string) => call<Setting[]>("dash_settings", { id }),
dashPrepared: (id: string) => call<PreparedXact[]>("dash_prepared", { id }),
dashStatements: (id: string, order: StatementsOrder) => call<Statements>("dash_statements", { id, order }),
dashSignal: (id: string, pid: number, kind: SignalKind) => call<SignalOutcome>("dash_signal", { id, pid, kind }),
```

- Üretir (`dash-model.ts`):

```ts
export const MAX_SAMPLES = 300;
export interface RatePoint { t: number; v: number | null }          // t: örnek zamanı (sunucu epoch sn); null = boşluk
export interface RateSeries { commit: RatePoint[]; rollback: RatePoint[]; ins: RatePoint[]; upd: RatePoint[]; del: RatePoint[];
                              ret: RatePoint[]; fetched: RatePoint[]; read: RatePoint[]; hit: RatePoint[] }
export class SampleRing {                                             // en çok MAX_SAMPLES; en eski düşer
  push(s: Sample): void; clear(): void; samples(): readonly Sample[]; }
/** Ardışık örneklerden saniyelik oranlar. Sayaç geriye gittiyse ya da aralık `gapFactor * intervalSec`'ten uzunsa o nokta null. */
export function toRates(samples: readonly Sample[], intervalSec: number, gapFactor?: number): RateSeries;
export function formatDuration(secs: number | null): string;          // "—", "850 ms", "12 sn", "3 dk 05 sn", "2 sa 10 dk", "1 g 4 sa"
export function formatCount(n: number): string;                       // 1 234 567 → "1,2 M" benzeri kısa biçim (tr)
```

- Üretir (`lock-tree.ts`):

```ts
export interface LockNode { pid: number; session: BlockSession | null; /* null: hazır tx (pid 0) ya da listede olmayan */
                            kind: "session" | "prepared"; children: LockNode[]; cycle: boolean }
/** Kökler: kimseyi beklemeyen ama başkalarını engelleyen oturumlar. Döngüde (hepsi birbirini bekliyor) en küçük pid kök sayılır ve `cycle: true` işaretlenir. */
export function buildLockTree(sessions: readonly BlockSession[]): LockNode[];
export function countWaiters(node: LockNode): number;                 // alt ağaçtaki toplam bekleyen (tekrarsız)
```

- Üretir (`table-model.ts`): `filterActivity(rows, f)` (`f`: `{ search, states: Set<string>, onlyActive, idleInTxOverSecs: number | null, database: string | null }`), `sortActivity(rows, key, dir)` (sütun anahtarları: pid, database, user, application, client, state, wait, xactAge, queryAge, stateAge), `filterSettings(rows, { search, modifiedOnly, pendingRestartOnly })`, `sortStatements`-gerekmez (arka uç sıralar). Sıralama kararlı, `null` değerler her zaman sonda.
- Üretir (`signal-model.ts`):

```ts
export interface SignalWarning { kind: "self" | "pgtoolsQuery" | "idleInTx" | "terminate"; }
export function signalWarnings(row: ActivityRow, kind: SignalKind): SignalWarning[];
export type Revalidate = { ok: true } | { ok: false; reason: "gone" | "changed" };
/** Onay anında canlı listede aynı pid hâlâ aynı oturum mu? (pid + user + application + backendType). */
export function revalidate(shown: ActivityRow, live: readonly ActivityRow[]): Revalidate;
```

- [ ] **Adım 1: Başarısız testleri yaz** (hepsi saf; Tauri gerekmez). Kritik vakalar:

```ts
// dash-model.test.ts
it("ilk örnek yalnızca taban çizgisi; oranlar farktan", () => {
  const r = toRates([smp(100, { xactCommit: 1000 }), smp(102, { xactCommit: 1200 })], 2);
  expect(r.commit).toEqual([{ t: 102, v: 100 }]);                     // (1200-1000)/2
});
it("sayaç geriye giderse null (negatif oran yok)", () => {
  const r = toRates([smp(100, { xactCommit: 5000 }), smp(102, { xactCommit: 40 })], 2);
  expect(r.commit[0].v).toBeNull();
});
it("uzun aralık (uyku) boşluk bırakır", () => {
  const r = toRates([smp(100, {}), smp(100 + 60, { xactCommit: 9 })], 2, 5);
  expect(r.commit[0].v).toBeNull();                                   // 60 sn > 5 × 2 sn
});
it("halka 300 örnekte en eskiyi düşürür", () => { /* 305 push → length 300, ilk t = 5. örnek */ });
it("süre biçimleri", () => { expect(formatDuration(null)).toBe("—"); expect(formatDuration(0.85)).toBe("850 ms"); expect(formatDuration(185)).toBe("3 dk 05 sn"); });

// lock-tree.test.ts
it("zincir: 3 → 2 → 1 (1 kök)", () => { /* blockedBy: 2:[1], 3:[2] */ });
it("aynı engelleyiciyi bekleyen çoğu doğru gruplanır; countWaiters tekrarsız", () => {});
it("döngü (1↔2) sonsuz döngüye girmez; cycle işaretlenir; en küçük pid kök", () => {});
it("blockedBy içinde 0 → kind 'prepared' düğümü, çocuğu bekleyen", () => {});
it("bir oturum iki engelleyiciyi bekliyorsa (blockedBy:[1,2]) iki ağaçta da görünür ama countWaiters tekrar saymaz", () => {});

// signal-model.test.ts
it("kendi satırı → self uyarısı; pgtools sorgu uygulaması → pgtoolsQuery; 'idle in transaction' → idleInTx; terminate her zaman terminate", () => {});
it("revalidate: pid yok → gone; pid aynı ama user/application değişti → changed; aynı → ok", () => {});

// table-model.test.ts
it("filterActivity: arama sorgu metni ve kullanıcıda; onlyActive; idleInTxOverSecs eşik (stateAge)", () => {});
it("sortActivity: null'lar yönden bağımsız sonda; eşitlikte pid ile kararlı", () => {});
it("filterSettings: modifiedOnly source != 'default'; pendingRestartOnly", () => {});
```

(`smp(t, partial)`: testte `Sample` üreten yardımcı; tüm alanlar 0 varsayılan. Gerçek alan adları `src/ipc/bindings/Sample.ts`'ten alınır — `xactCommit`, `xactRollback`, `tupReturned`, `tupFetched`, `tupInserted`, `tupUpdated`, `tupDeleted`, `blksRead`, `blksHit`, `sessionsActive`, `sessionsIdle`, `sessionsIdleInTx`, `sessionsOther`, `at`.)

- [ ] **Adım 2:** `bun run test src/features/dashboard` — başarısızlığı gör.
- [ ] **Adım 3: Uygula.** `toRates`: `Δt = cur.at - prev.at`; `Δv = cur.x - prev.x`; `Δv < 0 || Δt <= 0 || Δt > gapFactor * intervalSec` → `v: null`, aksi halde `Δv / Δt`; varsayılan `gapFactor = 5`. `buildLockTree`: `byPid` haritası; kök = `blockedBy` boş olan ve en az bir kişiyi engelleyen oturum; `blockedBy`'daki `0` için sentetik `prepared` düğümü; ziyaret kümesiyle döngü kesilir (`cycle: true`); hiç kök yoksa ama bekleyenler varsa en küçük pid kök olur. `signalWarnings`: `application === "pg-tools query"` → `pgtoolsQuery`; `state?.startsWith("idle in transaction")` → `idleInTx`; `kind === "terminate"` → `terminate`; `isSelf` → `self`.
- [ ] **Adım 4:** `bun run test src/features/dashboard`, `bun run typecheck` yeşil.
- [ ] **Adım 5: Commit** (`feat(dashboard): dash* komutları; oran, engelleme ağacı, süzgeç ve sinyal modelleri`).

---

### Task 2: Yenileme zamanlayıcısı ve `dash-controller`

**Dosyalar:**
- Oluştur: `src/features/dashboard/poller.ts`, `dash-controller.ts` ve `.test.ts` dosyaları

**Arayüzler:**
- Tüketir: Task 1 komutları ve modelleri; `Api` tipi (`commands`).
- Üretir (`poller.ts`) — zaman ve görünürlük **enjekte edilir** (test sahte zamanlayıcıyla):

```ts
export interface PollerDeps {
  now(): number;
  setTimer(fn: () => void, ms: number): unknown;
  clearTimer(h: unknown): void;
  visible(): boolean;                                  // document.visibilityState === "visible"
  onVisibility(cb: () => void): () => void;            // dinleyici; geri dönüş: kaldır
}
export interface Poller {
  setActive(on: boolean): void;                        // sekme etkin mi
  setPaused(on: boolean): void;
  setIntervalMs(ms: number): void;
  refreshNow(): void;                                  // tek uçuşta kuralına uyar
  dispose(): void;
}
/** `tick`: bir yenileme turu (söz). Önceki tur bitmeden yenisi başlamaz. Etkin değil/gizli/duraklatılmış → zamanlayıcı yok. Dönüşte hemen bir tur. */
export function createPoller(deps: PollerDeps, tick: () => Promise<void>): Poller;
```

- Üretir (`dash-controller.ts`):

```ts
export type DashStatus = "connecting" | "ready" | "lost" | "gone";   // gone: sunucu silindi
export type Section = "overview" | "sessions" | "locks" | "settings" | "statements";
export interface DashModel {
  id: string; serverId: string; serverName: string; databaseOid: number | null;
  info: DashInfo | null; status: DashStatus; error: string | null;
  section: Section; paused: boolean; intervalMs: number;           // grafik aralığı; listeler max(5000, intervalMs)
  overview: Overview | null; ring: SampleRing; rates: RateSeries;
  activity: ActivityReport | null; locks: LockReport | null;
  settings: Setting[] | null; prepared: PreparedXact[] | null;
  statements: Statements | null; statementsOrder: StatementsOrder;
  sectionError: Partial<Record<Section, string>>;                  // bölüm başına son hata (veri korunur)
}
export function createDashController(deps: { api: Api; notify(text: string, kind: ToastKind): void; poller?: Partial<PollerDeps> }) {
  // open(tab: DashTabInput): kimliği `dash_open` ile alır; close(tabId); setActive(tabId | null); setSection; setPaused;
  // setIntervalMs; refreshNow; setStatementsOrder; loadSettings() (talep üzerine); signal(tabId, row, kind) -> Promise<"sent"|"notFound"|"error">
  // serverSessionsClosed(serverId, gone): ilgili sekmeleri "lost"/"gone" yapar ve yenilemeyi durdurur;
  // reopen(tabId): `lost` sekmeyi yeniden `dash_open` ile açar.
}
```

**Yenileme turu (bir `tick`)**: `overview` + `sample` her turda; yalnızca **açık bölüm** için liste isteği (`sessions` → `dashActivity`, `locks` → `dashLocks`, `statements` → `dashStatements`, `settings`/`prepared` talep üzerine yalnızca elle "Yenile" ya da bölüme girişte). Liste istekleri `max(5000, intervalMs)` aralığıyla (her turda değil). Bir isteğin hatası yalnızca `sectionError[section]`'a yazılır, öncekı veri korunur; `not_found` (kapanmış kimlik) ya da `connection lost` benzeri `code` → `status = "lost"`, zamanlayıcı durur. Başarılı `sample` → `ring.push` + `rates = toRates(ring.samples(), intervalMs/1000)`.

- [ ] **Adım 1: Başarısız testleri yaz** (sahte `now`/zamanlayıcı/görünürlük + sahte `Api`):

```ts
// poller.test.ts
it("etkin değilken ya da gizliyken ya da duraklatılmışken hiç tick yok", () => {});
it("tick bitmeden yenisi başlamaz (tek uçuşta); bitince bir sonraki aralıkta başlar", () => {});
it("aralık değişince tek zamanlayıcı kalır (çift zamanlayıcı yok)", () => {});
it("görünür olunca / etkinleşince hemen bir tick çalışır", () => {});
it("dispose zamanlayıcıyı ve dinleyiciyi kaldırır; uçuştaki tick sonucu yoksayılır", () => {});
it("tick reddederse zamanlayıcı yine kurulur (hata döngüyü öldürmez)", () => {});

// dash-controller.test.ts
it("open: dash_open çağrılır, status ready olur, ilk tick overview+sample çeker", async () => {});
it("yalnızca açık bölümün liste isteği yapılır (overview açıkken activity/locks istenmez)", async () => {});
it("bölüm hatası öncekı veriyi korur ve sectionError'a yazar", async () => {});
it("not_found → status lost, zamanlayıcı durur; reopen yeni DashId ile devam eder ve halkayı sıfırlar", async () => {});
it("serverSessionsClosed(gone=true) → status gone; reopen yok", async () => {});
it("close: dash_close çağrılır; uçuştaki yanıt modeli bozmaz", async () => {});
it("signal: sent / notFound / 42501 hatası → durum + uygun toast; çift çağrı (uçuşta) ikinci sinyali göndermez", async () => {});
it("signal onay anında revalidate başarısızsa dash_signal çağrılmaz", async () => {});
```

- [ ] **Adım 2: Uygula.** `createPoller`: iç durum `{active, paused, intervalMs, inFlight, timer}`; `schedule()` yalnızca `active && !paused && visible()` iken ve `inFlight` değilken tek zamanlayıcı kurar; her durum değişikliği `reconcile()` ile (gerekirse zamanlayıcıyı kaldırır/kurar, yeni etkinleşmede hemen `run()`); `run()` `inFlight = true` → `await tick()` (`try/finally`) → `inFlight = false` → `schedule()`. `dispose` sonrası tüm geri çağrılar no-op. Controller sekme başına bir `Poller` ve `DashModel` tutar (`createStore`/signal; `QueryController`'ın `entries` kalıbını izle).
- [ ] **Adım 3:** `bun run test src/features/dashboard`, `bun run typecheck` yeşil.
- [ ] **Adım 4: Commit** (`feat(dashboard): tek uçuşta yenileme zamanlayıcısı ve dashboard denetleyicisi`).

---

### Task 3: Sekme entegrasyonu ve "Dashboard" açma

**Dosyalar:**
- Değiştir: `src/features/query/tabs-store.ts` (+ `tabs-store.test.ts`), `src/features/query/QueryTabs.tsx`, `src/features/query/query-controller.ts` (yalnızca `activeId` kullanımlarının denetimi), `src/features/query/QueryTab.tsx` (etkin kimlik denetimi), `src/features/explorer/actions.ts` (+ `actions.test.ts`), `src/features/explorer/ExplorerMenu.tsx`, `src/app/App.tsx`, `src/app/strings.ts`, `src/app/Toolbar.tsx` (gerekirse)
- Oluştur: `src/features/dashboard/DashboardView.tsx` (iskelet: bölüm sekmeleri + üst çubuk; bölüm içerikleri sonraki görevlerde), `src/features/dashboard/open-kinds.ts` (`canOpenDashboard`)

**Arayüzler:**
- Tüketir: Task 2 `createDashController`.
- Üretir (`tabs-store.ts`): `DashTab { id: string; serverId: string; databaseOid: number | null; title: string }`; `TabsState.dashboards: DashTab[]`; `openDashboard(state, input): { state, id }`; `closeTab/activateTab/renameTab` hem sorgu hem dashboard kimliğini tanır; `isQueryTabId(state, id)`. Başlık varsayılanı: `"Dashboard · <sunucu>"` ya da `"Dashboard · <sunucu>/<veritabanı>"`. `closeTab` etkin dashboard'u kapatınca sağdaki/soldaki sekme (sorgu ya da dashboard, **görünüm sırasıyla**: önce sorgu sekmeleri, sonra dashboard'lar) etkinleşir, yoksa `OBJECT_TAB_ID`.
- Üretir (`actions.ts`): `ActionId`'ye `"openDashboard"`; `actionsFor`: `server` (bağlıyken) ve `database` düğümlerinde `openDashboard` (Query Tool eyleminin yanında; `groupOf` → `-1`); `ActionDeps.openDashboard?(id: string): void`.
- Üretir (`App.tsx`): `createDashController` oluşturulur; `actionDeps.openDashboard = (id) => void openDashboardFromNode(id)` (`api.qtTargetOf(id)` → `{serverId, databaseOid}`; sunucu bağlı değilse mevcut `connect` kancasıyla bağlanır; tek seferde bir açma); `afterServerChange` içinde `dash.serverSessionsClosed(serverId, gone)`; `QueryTabs`'a `dash` ve tembel `DashboardView` bağlanır.

- [ ] **Adım 1: Başarısız testleri yaz.** `tabs-store.test.ts`: dashboard açma (kimlik çakışması yok, `activeId` olur), kapatma sonrası etkin sekme seçimi (sorgu + dashboard karışık), `renameTab`, `isQueryTabId`, mevcut sorgu sekmesi testleri bozulmaz. `actions.test.ts`: `actionsFor` bağlı sunucuda ve veritabanında `openDashboard` içerir, bağlı olmayan sunucuda içermez; `executeAction("openDashboard")` `deps.openDashboard`'ı çağırır.
- [ ] **Adım 2: `activeId` denetimi.** `activeId` artık dashboard kimliği olabilir: `query-controller.ts:218` (`prevActive`), `QueryTab.tsx:64-65` (`activeId()`), `QueryTabs.tsx` (`isObject`, `onKeyCapture` F5 kuralı — dashboard etkinken F5 **dashboard'u yeniler** (`dash.refreshNow`), sayfa yenileme yine engellenir) ve `activate`/`requestClose` yollarını gözden geçir: dashboard kimliği `ctl.model(id)` ile sorgulanmamalı. Sorgu paneli dashboard etkinken `invisible` olur (`isObject` gibi), dashboard paneli yalnızca dashboard etkinken görünür; **her dashboard paneli bağlı kalır** (gizli panel `invisible` ile saklanır, yenileme `setActive(false)` ile durur).
- [ ] **Adım 3: Sekme çubuğu.** `QueryTabs` dashboard sekmelerini sorgu sekmelerinden sonra çizer; ikon `Activity` (lucide), sunucu rengi/etiketi/kilit rozeti sorgu sekmesiyle aynı yardımcılarla (`serverLineColor`, `isProductionLabel`); durum noktası: `lost`/`gone` ise soluk + `line-through` (gone); kapatma düğmesi `dash.close` + `tabs.close`; orta tık ve Delete aynı. Dashboard sekmesi kapatılırken onay yok.
- [ ] **Adım 4: Açma yolu + metinler.** `App.tsx`/`ExplorerMenu.tsx`/`strings.ts` (`dashboard.open`, `dashboard.tabTitle(serverName, db?)`, `dashboard.status.{connecting,lost,gone}`, `dashboard.reopen`, `dashboard.noPrivilege`, bölüm adları, sinyal metinleri — hepsini bu görevde `strings.ts`'e ekle, sonraki görevler kullanır). Araç çubuğuna yeni düğme eklenmez (menüden açılır).
- [ ] **Adım 5: `DashboardView` iskeleti** (tembel parça: `lazy(() => import("../dashboard/DashboardView"))`): üst çubuk (aralık seçimi, **Duraklat**, **Yenile**, durum rozeti), bölüm sekmeleri (`Tabs`), `lost` durumunda "Bağlantı kesildi" şeridi + **Yeniden aç**, `gone` durumunda "Sunucu silindi" notu; bölüm gövdeleri geçici `<p>` yer tutucu **değil**, sonraki görevlerin dosyalarını içe aktaran ince `Suspense` kapıları olarak bu görevde yalnızca `OverviewView` için boş bileşen dosyası (`return null` değil, "Yükleniyor…" metni) oluşturulur; diğer bölümler Task 5–6'da eklenir ve o zamana kadar bölüm sekmesi gizlidir.
- [ ] **Adım 6: Doğrulama.** `bun run test`, `bun run typecheck`, `bun run build` (ana `index-*.js` boyutu not edilir; dashboard parçası ayrı olmalı — `dist/assets` altında yeni `DashboardView-*.js`).
- [ ] **Adım 7: Commit** (`feat(dashboard): dashboard sekmesi, Explorer'dan açma ve yaşam döngüsü bağlantısı`).

---

### Task 4: Canvas grafik ve Genel (Overview) görünümü

**Dosyalar:**
- Oluştur: `src/features/dashboard/chart-model.ts` (+ test), `Chart.tsx`, `OverviewView.tsx`
- Değiştir: `DashboardView.tsx` (Overview bölümünü bağla)

**Arayüzler:**
- Üretir (`chart-model.ts`, saf):

```ts
export interface Series { name: string; color: string /* CSS değişken adı, ör. "--accent" */; points: { t: number; v: number | null }[]; stacked?: boolean }
export interface Viewport { width: number; height: number; padLeft: number; padBottom: number; padTop: number; padRight: number }
export function niceMax(max: number): number;                            // 0 → 1; 87 → 100; 1234 → 1500 (1/2/2.5/5/10 × 10^n)
export function yTicks(max: number, count?: number): number[];
export function xDomain(series: Series[], windowSec: number): [number, number]; // son noktadan geriye windowSec (varsayılan 600)
/** Çizim komutları: boşlukta (null) çizgi kesilir; yığınlıda alt-üst alanlar kümülatif. */
export function buildPaths(series: Series[], vp: Viewport, domain: [number, number], yMax: number): PathCmd[];
export function nearestIndex(points: { t: number }[], x: number, vp: Viewport, domain: [number, number]): number | null;
```

`Chart.tsx`: tek `<canvas>`; `devicePixelRatio`'ya göre ölçekler; `ResizeObserver` ile yeniden boyutlanır; renkler `getComputedStyle(document.documentElement).getPropertyValue(varName)` ile (tema değişince — `prefers-color-scheme`/`data-theme` değişimini `MutationObserver`/matchMedia ile izleyip yeniden çizer); **yalnızca veri/boyut/tema değişince** `requestAnimationFrame` ile bir kez çizer; imleç üstünde dikey kılavuz + değer ipucu (`nearestIndex`); `aria-label` (grafik adı + son değerler) ve ekran okuyucu için gizli (`sr-only`) küçük tablo (son 5 örnek). Boş veri: "Veri bekleniyor".

`OverviewView.tsx`: özet kartları (sürüm, çalışma süresi `formatDuration`, oturum / `max_connections` (`connections / maxConnections`, %80 üstü `--danger` rozeti), en eski açık transaction süresi, `activity` bölümünde değil Overview'daki `Sample.sessions*` toplamı) + **5 grafik**: Oturumlar (etkin/boşta/boşta-tx **yığılı**), İşlem/sn (commit/rollback), Satır girişi (insert/update/delete), Satır çıkışı (returned/fetched), Blok G/Ç (read/hit). Yetkisiz rol notu: `activity` yüklüyse ve `query == null` satırı varsa ya da `sessionsOther > 0` ve açıkça "yetki yok" varsa kart altında "Bazı oturumlar görünmüyor" (yalnızca `activity` bir kez yüklendiyse; Overview kendi başına bunu bilmez — notu `sessionsOther` kutusunun `title`'ında belirt, yanlış alarm üretme).

- [ ] **Adım 1: Başarısız testler** (`chart-model.test.ts`): `niceMax(0)=1`, `niceMax(87)=100`, `niceMax(1234)=1500`/`2000` (seçilen adım kümesine göre; testte kümeyi sabitle); `yTicks`; `xDomain` boş seride güvenli; `buildPaths` null'da çizgiyi keser (iki ayrı alt yol), yığınlı serilerde kümülatif toplam; `nearestIndex` kenar durumlar (boş dizi → null).
- [ ] **Adım 2: Uygula** `chart-model.ts` (saf) ve `Chart.tsx`, `OverviewView.tsx`. Grafiklerin x penceresi 10 dk; örnek aralığına bağlı değil (zaman eksenli).
- [ ] **Adım 3: Doğrulama.** `bun run test`, `bun run typecheck`, `bun run build`. (Elle görsel kontrol kullanıcıdadır.)
- [ ] **Adım 4: Commit** (`feat(dashboard): Canvas grafik bileşeni ve Genel görünüm`).

---

### Task 5: Oturumlar ve Kilitler görünümleri, iptal/sonlandırma diyaloğu

**Dosyalar:**
- Oluştur: `src/features/dashboard/DataTable.tsx`, `SessionsView.tsx`, `LocksView.tsx`, `SignalDialog.tsx`
- Değiştir: `DashboardView.tsx` (iki bölümü bağla), `dash-controller.ts` (diyalog kapısı: `signalGate`, `QueryController`'daki `gate()` kalıbı)

**Arayüzler:**
- `DataTable<T>`: TanStack Virtual ile sanal tablo; `columns: { key, header, width, cell(row), sortable?, align? }[]`; `rows`, `rowKey`, `selectedKey`, `onSelect`, `onActivate` (Enter), `onDelete`; `role="grid"`; başlığa tıklayınca sıralama (`aria-sort`); ok tuşlarıyla satır seçimi. Mevcut `ResultGrid.tsx`'in sanal kaydırma kalıbını izle; bileşeni sorgu ızgarasından **ayrı** tut (ızgara hücre/NULL/tip başlığı özeldir).
- `SessionsView`: araç çubuğu (arama, durum süzgeci çoklu seçim, "yalnızca etkin", "boşta-tx ≥ N sn" seçici, veritabanı süzgeci — sunucu düzeyinde), tablo (sütunlar: pid, veritabanı, kullanıcı, uygulama, istemci, durum rozeti, bekleme (`waitEventType:waitEvent`), xact süresi, sorgu süresi, durum süresi, sorgu tek satır), alttaki ayrıntı paneli (tam sorgu metni — `<pre>`, **Kopyala**; `query == null` → "Yetki yok: başka bir rolün sorgusu görüntülenemez"), satır eylemleri **İptal** ve **Sonlandır** düğmeleri (kendi satırında `disabled` + `title`). `ActivityReport.truncated` ise üstte "İlk 5000 oturum gösteriliyor" bandı. Satır sayısı ve süzgeç sonucu göstergesi.
- `LocksView`: üstte engelleme ağacı (`buildLockTree`; `LockNode` başına pid, kullanıcı, uygulama, durum, bekleme süresi, sorgu özeti; `prepared` düğümü "Hazır transaction" etiketiyle, eylemsiz; döngü rozeti "döngü"; kökte **İptal/Sonlandır** kısayolu), altta düz kilit listesi (`LockRow`: pid, tür, mod, verildi mi, nesne) — verilmeyenler üstte (arka uç sıralı); `truncated` bandı; engelleme yoksa "Engellenen oturum yok" boş durumu. Ağaç düğümü eylemi, `activity` listesindeki ilgili satırı bulamazsa (Sessions bölümü hiç yüklenmemiş olabilir) satırı `BlockSession`'dan türetilen asgari `ActivityRow` ile gösterir ve `isSelf: false` kabul eder; **kendi pid'i zaten arka uç `locks` sonucundan dışlanmıştır**.
- `SignalDialog` (`ConfirmDialog` stilinde, `Dialog` bileşeniyle): başlık "Sorguyu iptal et" / "Oturumu sonlandır"; gövde pid, kullanıcı, uygulama, durum, sorgunun ilk 3 satırı; `signalWarnings`'a göre uyarı satırları (`pgtoolsQuery`: "Bu, pg-tools'un kendi sorgu sekmesi."; `idleInTx`: "Açık transaction geri alınacak."; `terminate`: "Oturum kapanır; bağlı istemci hata alır."); odak **Vazgeç**'te; onay düğmesi `Sonlandır` için `danger`. Onayda: `revalidate(shown, canlıListe)` — `gone` → "Oturum artık yok." (işlem yapılmaz), `changed` → "Bu pid artık başka bir oturuma ait; işlem yapılmadı." (işlem yapılmaz); yalnızca `ok` ise `dash_signal`; uçuşta iken düğme `disabled` (çift sinyal yok). Sonuç: `sent` → başarı toast'ı ve hemen bir yenileme (`refreshNow`); `notFound` → "Oturum artık yok."; hata `code === "42501"` → "Bu oturumu durdurma yetkiniz yok (yalnızca kendi oturumlarınız ya da pg_signal_backend üyeliği)."; `invalid` → kendi oturumunuz; diğerleri `message`.

- [ ] **Adım 1: Başarısız testler.** Bileşenler DOM testi gerektirmeyecek biçimde ince tutulur; mantık Task 1–2 modellerinde zaten testli. Bu görevde eklenecek testler: `dash-controller.test.ts`'e `signalGate` akışı (istek → onay → revalidate → `dash_signal`; iptal → çağrı yok; ikinci istek ilkini `false` ile çözer, `QueryController` kapıları gibi), ve `table-model`'e `SessionsView`'ın kullandığı `durationBucket`/durum rozeti eşlemesi gibi saf yardımcılar eklenirse testleri.
- [ ] **Adım 2: Uygula** bileşenleri ve kapıyı. Klavye: tabloda Delete = **İptal** isteği (diyalog açar, doğrudan göndermez).
- [ ] **Adım 3: Doğrulama.** `bun run test`, `bun run typecheck`, `bun run build`.
- [ ] **Adım 4: Commit** (`feat(dashboard): Oturumlar, Kilitler görünümleri ve iptal/sonlandırma diyaloğu`).

---

### Task 6: Yapılandırma, hazır transaction'lar ve Sorgu istatistikleri

**Dosyalar:**
- Oluştur: `src/features/dashboard/SettingsView.tsx`, `StatementsView.tsx`
- Değiştir: `DashboardView.tsx`, `dash-controller.ts` (`loadSettings`, `loadPrepared`, `statements` bölümü sıralaması)

**Arayüzler:**
- `SettingsView`: `Setting[]` tablosu (ad, değer + birim, kaynak, bağlam, "yeniden başlatma bekliyor" rozeti, açıklama); süzgeçler: arama (ad/açıklama), "yalnızca değiştirilmiş" (`source !== 'default'`), "yeniden başlatma bekliyor"; `value == null` → "—" (yetkisiz). Yalnızca bölüme girişte ve **Yenile** ile yüklenir (otomatik yenilenmez). Altta **Hazır transaction'lar** bölümü (`PreparedXact`: gid, transaction, sahip, veritabanı, yaş): 1 saatten eskiler "uzun süredir açık" rozetiyle (`--danger`), boşsa "Hazır transaction yok". Hazır tx için eylem yok (kapsam dışı; yalnızca bilgi).
- `StatementsView`: `Statements.kind === "unavailable"` → `reason` metni bir boş durum olarak gösterilir (eksik eklenti / yüklenmemiş / arama yolu — mesaj arka uçtan gelir, arayüz yorumlamaz); `available` → tablo (sorgu tek satır + tam metin ayrıntı paneli ve **Kopyala**, çağrı, toplam ms, ortalama ms, satır, `shared_blks_hit/read` ve isabet oranı); sıralama seçici (Toplam süre / Ortalama / Çağrı) → `statementsOrder` değişince hemen yeniden yüklenir; `query == null` → "Yetki yok". Otomatik yenileme liste aralığıyla.
- Bölüm sekmeleri: Genel, Oturumlar, Kilitler, Yapılandırma, Sorgu istatistikleri. İstatistik sekmesi bir kez `unavailable` döndüyse etiketi soluk olur ama tıklanabilir kalır (neden metni görünsün).

- [ ] **Adım 1: Başarısız testler:** `dash-controller.test.ts`: `settings` bölümü ilk girişte bir kez yüklenir, otomatik tick'te istenmez; `setStatementsOrder` hemen `dashStatements(id, order)` çağırır; `unavailable` sonucu modelde saklanır; `table-model`: `filterSettings` (Task 1'de), `preparedAgeBadge(ageSecs)` (≥ 3600 → `"old"`) için saf yardımcı + test.
- [ ] **Adım 2: Uygula** bileşenleri.
- [ ] **Adım 3: Doğrulama.** `bun run test`, `bun run typecheck`, `bun run build`.
- [ ] **Adım 4: Commit** (`feat(dashboard): Yapılandırma, hazır transaction'lar ve pg_stat_statements görünümleri`).

---

### Task 7: Doğrulama, belgeler ve ölçüm

**Dosyalar:**
- Değiştir: `docs/smoke-test.md` (yeni "Dashboard" bölümü), spec durumu (`docs/superpowers/specs/2026-10-06-dashboard-design.md` → "Uygulandı (Plan 8–9)")

- [ ] **Adım 1: Elle kontrol listesi** (`docs/smoke-test.md`, her madde `[—]` ile — kullanıcı sonra işaretler): Dashboard'u sunucu ve veritabanı düğümünden açma; sekme rengi/etiketi/kilit; grafiklerin akması, duraklat/devam, aralık değiştirme; sekme arka plandayken isteklerin durması (sunucu `pg_stat_activity`'de dashboard bağlantısının `state`/`query_start` değişmemesi); Oturumlar süzgeçleri, ayrıntı, kopyala; yetkisiz rolle bağlanıp "Yetki yok" gösterimi; iptal ve sonlandırma (uyarı metinleri, `pgtoolsQuery`, boşta-tx), kendi satırının kapalı olması; iki oturumla engelleme zinciri ve hazır transaction; Yapılandırma süzgeçleri; `pg_stat_statements` varken/yokken; sunucu bağlantısını kesince "bağlantı kesildi" + **Yeniden aç**; sunucuyu silince "Sunucu silindi"; uygulamayı yeniden yüklemeden (F5 engelli) açıkken kapatma; açık/koyu tema.
- [ ] **Adım 2: Bütünleşik doğrulama.** `bun run test` (tümü), `bun run typecheck`, `bun run build` (çıktıdaki ana `index-*.js` boyutu Plan 8 sonuna göre ≤ +3 KB; dashboard ayrı parça), `cargo test --workspace` (hiç değişmemiş olmalı), `cargo clippy --workspace --all-targets -- -D warnings`. Rapor: ana paket boyutu önce/sonra, dashboard parçası boyutu.
- [ ] **Adım 3: Ölçüm (isteğe bağlı, kullanıcı çalıştırır).** `bun run tauri build --no-bundle` + `scripts/measure.ps1` açılış/bellek karşılaştırması; ajan çalıştırıcı bu adımı yalnızca derleme başarısı için yapar, rakamları yorumlamaz.
- [ ] **Adım 4: Commit** (`docs(dashboard): elle kontrol listesi, spec durumu`).

---

## Kapanış kontrolü (Plan 9 bitişi)

- `rg "console\\." src/features/dashboard` boş (sorgu metni sızıntısı yok); `rg "localStorage|sessionStorage" src/features/dashboard` boş.
- Dashboard kodu yalnızca `lazy` içe aktarmalarla yüklenir (`rg "features/dashboard" src --glob '!src/features/dashboard/**'` yalnızca `lazy(() => import(...))` ve tip içe aktarmalarını gösterir).
- Final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.
