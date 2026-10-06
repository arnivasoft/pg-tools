// Dashboard sekmelerinin durumu (tembel parça): sekme başına bir `DashId`, bir `Poller` ve bir model sinyali. Yenileme turu,
// bölüm listeleri, kayıp/yeniden açma ve iptal/sonlandırma akışı (onay kapısı → canlı doğrulama → `dash_signal`) buradadır.
import { createSignal, type Accessor, type Setter } from "solid-js";
import { td } from "../../app/strings-dashboard";
import type { ToastKind } from "../../app/toast-model";
import type { ActivityReport } from "../../ipc/bindings/ActivityReport";
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
import type { DashInfo } from "../../ipc/bindings/DashInfo";
import type { LockReport } from "../../ipc/bindings/LockReport";
import type { Overview } from "../../ipc/bindings/Overview";
import type { PreparedXact } from "../../ipc/bindings/PreparedXact";
import type { Setting } from "../../ipc/bindings/Setting";
import type { SignalKind } from "../../ipc/bindings/SignalKind";
import type { SignalOutcome } from "../../ipc/bindings/SignalOutcome";
import type { Statements } from "../../ipc/bindings/Statements";
import type { StatementsOrder } from "../../ipc/bindings/StatementsOrder";
import type { Api } from "../../ipc/commands";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { errorText, toPayload } from "../../ipc/errors";
import { SampleRing, toRates, type RateSeries } from "./dash-model";
import { DEFAULT_INTERVAL_MS, MIN_INTERVAL_MS, createPoller, defaultPollerDeps, type Poller, type PollerDeps } from "./poller";
import { revalidate, signalWarnings, type SignalWarning } from "./signal-model";

export type DashStatus = "connecting" | "ready" | "lost" | "gone"; // gone: sunucu silindi
export type Section = "overview" | "sessions" | "locks" | "settings" | "statements";
/** Hata anahtarı: bölümler + görünür sekmesi olmayan talep üzerine listeler (`prepared`). */
export type ErrorKey = Section | "prepared";

export interface DashModel {
  /** Sekme kimliği (arka uç `DashId`'si `info.id`). */
  id: string;
  serverId: string;
  serverName: string;
  databaseOid: number | null;
  info: DashInfo | null;
  status: DashStatus;
  /** Açılış/kayıp nedeni (`lost`/`gone` şeridinde gösterilir). */
  error: string | null;
  section: Section;
  paused: boolean;
  /** Grafik/özet aralığı; listeler `max(LIST_MIN_MS, intervalMs)`. */
  intervalMs: number;
  overview: Overview | null;
  /** Her `push` yeni anlık görüntü üretir; `reopen` yeni halka kurar. */
  ring: SampleRing;
  rates: RateSeries;
  activity: ActivityReport | null;
  locks: LockReport | null;
  settings: Setting[] | null;
  prepared: PreparedXact[] | null;
  statements: Statements | null;
  statementsOrder: StatementsOrder;
  /** Bölüm başına son hata (önceki veri korunur). */
  sectionError: Partial<Record<ErrorKey, string>>;
  /** İptal/sonlandırma isteği uçuşta (düğmeler kapalı). */
  signaling: boolean;
}

export interface DashTabInput {
  /** Sekme kimliği (verilmezse üretilir). */
  id?: string;
  serverId: string;
  serverName: string;
  databaseOid: number | null;
}

export type SignalResult = "sent" | "notFound" | "changed" | "error" | "busy";

export interface SignalRequest {
  tabId: string;
  row: ActivityRow;
  kind: SignalKind;
  warnings: SignalWarning[];
}

/** Liste bölümlerinin en kısa yenileme aralığı. */
export const LIST_MIN_MS = 5000;

/** Kimlik kapanmış/bağlantı yok → `lost` (yeniden açılabilir); sunucu kaydı yok → `gone`. Yalnızca `code` ile. */
const LOST_CODES = new Set(["not_found", "not_connected", "connection_lost"]);
const GONE_CODES = new Set(["unknown_server"]);

/**
 * `lost` şeridindeki neden: `not_found` (kapanmış `DashId`) ham arka uç metni ("Nesne artık mevcut değil: Dashboard oturumu
 * <uuid>") yerine null → şerit `td.lostNote` gösterir; bağlantı hatalarının metni korunur.
 */
const lostError = (p: ErrorPayload): string | null => (p.code === "not_found" ? null : p.message);

const emptyRates = (): RateSeries => toRates([], 1);

/** Geçersiz aralık (0, negatif) → en az `MIN_INTERVAL_MS`; sonlu değilse (NaN) varsayılan. */
export function clampInterval(ms: number): number {
  return Number.isFinite(ms) ? Math.max(MIN_INTERVAL_MS, ms) : DEFAULT_INTERVAL_MS;
}

function gate<Req, Res>(fallback: Res) {
  const [req, setReq] = createSignal<{ value: Req; resolve: (r: Res) => void } | null>(null);
  const ask = (value: Req) =>
    new Promise<Res>((resolve) => {
      req()?.resolve(fallback);
      setReq({ value, resolve });
    });
  const answer = (r: Res) => {
    const cur = req();
    if (!cur) return;
    setReq(null);
    cur.resolve(r);
  };
  return { request: () => req()?.value ?? null, ask, answer };
}

interface Entry {
  model: Accessor<DashModel>;
  set: Setter<DashModel>;
  poller: Poller;
  /** Sekme etkin mi (poller yalnızca etkin + `ready` iken çalışır). */
  tabActive: boolean;
  /** Kuşak: kayıp/yeniden açma/kapatmada artar; eski kuşaktan gelen yanıt yoksayılır. */
  gen: number;
  /** Liste bölümünün son istek zamanı (`now()`). */
  listAt: Partial<Record<Section, number>>;
  loading: Set<"settings" | "prepared">;
}

export interface DashControllerDeps {
  api: Api;
  notify(text: string, kind: ToastKind): void;
  poller?: Partial<PollerDeps>;
}

export function createDashController(deps: DashControllerDeps) {
  const { api, notify } = deps;
  const pd: PollerDeps = { ...defaultPollerDeps(), ...deps.poller };
  const entries = new Map<string, Entry>();
  // Kayıt sinyali: `entries` düz Map'tir (tepkisiz). Sekme modelden ÖNCE çizilir (dash-open `openDashboard` →
  // `open`; store eşzamanlı akar); okuyucular bu sinyale bağlanır, girdi eklenince/silinince yeniden çalışır.
  const [registry, bump] = createSignal<void>(undefined, { equals: false });
  const signalGate = gate<SignalRequest, boolean>(false);

  const model = (tabId: string): DashModel | undefined => (registry(), entries.get(tabId)?.model());
  const update = (e: Entry, f: (m: DashModel) => DashModel) => e.set((m) => f(m));
  const patch = (e: Entry, p: Partial<DashModel>) => update(e, (m) => ({ ...m, ...p }));
  /** Yanıt hâlâ geçerli mi: sekme açık ve aynı kuşak. */
  const alive = (tabId: string, e: Entry, gen: number) => entries.get(tabId) === e && e.gen === gen;

  const syncPoller = (e: Entry) => {
    const m = e.model();
    const on = e.tabActive && m.status === "ready";
    e.poller.setActive(on);
    // Arka planda `settings` bölümüne geçildiyse talep üzerine yükleme etkinleşince yapılır.
    if (on && m.section === "settings") enterSettings(m.id);
  };

  /**
   * Liste bölümü vadesinde mi. Yarım tur pay: aksi halde 2 sn aralıkta liste 5 sn yerine bir sonraki tam tur katında
   * (6 sn) yenilenirdi; pay ile en yakın tura (4 sn) yuvarlanır; 10 sn aralıkta her tur.
   */
  const listDue = (e: Entry, section: Section, now: number) => {
    const at = e.listAt[section];
    const ms = e.model().intervalMs;
    return at === undefined || now - at >= Math.max(LIST_MIN_MS, ms) - ms / 2;
  };

  /** Hemen tur: yalnızca sekme etkin ve hazırken (görünürlük kuralı poller'da); değilse dönüşteki tur getirir. */
  const kick = (e: Entry) => {
    if (e.tabActive && e.model().status === "ready") e.poller.refreshNow();
  };

  /** Kimlik artık kullanılamaz: durum `lost`/`gone`, yenileme durur, uçuştaki yanıtlar yoksayılır. */
  const markDown = (e: Entry, status: "lost" | "gone", error: string | null) => {
    const info = e.model().info;
    e.gen++;
    patch(e, { status, error, signaling: false });
    syncPoller(e);
    // Arka uçta kalmış olabilecek kimliği bırak (kapanmışsa da zararsız).
    if (info) void api.dashClose(info.id).catch(() => {});
  };

  /** Hata `code`una göre: kayıp → durum; aksi halde `sectionError[section]` (veri korunur). */
  const fail = (e: Entry, section: ErrorKey, err: unknown) => {
    const p = toPayload(err);
    if (GONE_CODES.has(p.code)) return markDown(e, "gone", p.message);
    if (LOST_CODES.has(p.code)) return markDown(e, "lost", lostError(p));
    update(e, (m) => ({ ...m, sectionError: { ...m.sectionError, [section]: errorText(p) } }));
  };
  const clearError = (m: DashModel, section: ErrorKey): Partial<Record<ErrorKey, string>> => {
    if (!(section in m.sectionError)) return m.sectionError;
    const next = { ...m.sectionError };
    delete next[section];
    return next;
  };

  /**
   * Tek istek: yanıt geçerliyse `apply` (ve `clear` ise bölüm hatası silinir), hata `fail`. Dönüş: başarılı mı.
   * Özet + örnek aynı bölüm hatasını paylaştığından ikisi birlikte başarılıysa silinir (bkz. `tick`).
   */
  const request = async <T>(
    tabId: string, e: Entry, gen: number, section: ErrorKey, p: Promise<T>,
    apply: (m: DashModel, v: T) => Partial<DashModel>, clear = true,
  ): Promise<boolean> => {
    try {
      const v = await p;
      if (!alive(tabId, e, gen)) return false;
      update(e, (m) => ({ ...m, ...apply(m, v), sectionError: clear ? clearError(m, section) : m.sectionError }));
      return true;
    } catch (err) {
      if (alive(tabId, e, gen)) fail(e, section, err);
      return false;
    }
  };

  // ---- yenileme turu
  const tick = async (tabId: string, e: Entry) => {
    const m = e.model();
    if (!alive(tabId, e, e.gen) || m.status !== "ready" || !m.info) return;
    const gen = e.gen;
    const id = m.info.id;
    const head = Promise.all([
      request(tabId, e, gen, "overview", api.dashOverview(id), (_, overview) => ({ overview }), false),
      request(tabId, e, gen, "overview", api.dashSample(id), (cur, s) => {
        // Örnek, turun başladığı andaki aralıkla kaydedilir (aralık sonradan değişse de çifti doğru eşikle ölçülür).
        cur.ring.push(s, m.intervalMs / 1000);
        return { rates: toRates(cur.ring.samples(), cur.intervalMs / 1000, undefined, cur.ring.intervals()) };
      }, false),
    ]).then(([a, b]) => {
      if (a && b && alive(tabId, e, gen)) update(e, (cur) => ({ ...cur, sectionError: clearError(cur, "overview") }));
    });
    const jobs: Promise<unknown>[] = [head];
    const now = pd.now();
    const due = (s: Section) => {
      if (!listDue(e, s, now)) return false;
      e.listAt[s] = now;
      return true;
    };
    if (m.section === "sessions" && due("sessions")) {
      jobs.push(request(tabId, e, gen, "sessions", api.dashActivity(id), (_, activity) => ({ activity })));
    } else if (m.section === "locks" && due("locks")) {
      jobs.push(request(tabId, e, gen, "locks", api.dashLocks(id), (_, locks) => ({ locks })));
    } else if (m.section === "statements" && due("statements")) {
      const order = m.statementsOrder;
      jobs.push(
        request(tabId, e, gen, "statements", api.dashStatements(id, order), (cur, statements) =>
          // Sıralama bu arada değiştiyse eski sonuç yazılmaz (bekleyen yenileme yenisini getirir).
          cur.statementsOrder === order ? { statements } : {},
        ),
      );
    }
    await Promise.all(jobs);
  };

  /** Talep üzerine (`settings` bölümüne giriş / elle Yenile): otomatik turda istenmez. */
  const loadOnDemand = async (tabId: string, kind: "settings" | "prepared") => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || m.status !== "ready" || !m.info || e.loading.has(kind)) return;
    e.loading.add(kind);
    try {
      if (kind === "settings") await request(tabId, e, e.gen, "settings", api.dashSettings(m.info.id), (_, settings) => ({ settings }));
      else await request(tabId, e, e.gen, "prepared", api.dashPrepared(m.info.id), (_, prepared) => ({ prepared }));
    } finally {
      e.loading.delete(kind);
    }
  };
  const loadSettings = (tabId: string) => loadOnDemand(tabId, "settings");
  const loadPrepared = (tabId: string) => loadOnDemand(tabId, "prepared");
  /** `settings` bölümüne giriş: ayarlar yalnızca ilk kez, hazır transaction'lar (küçük liste) her girişte. */
  const enterSettings = (tabId: string) => {
    if (entries.get(tabId)?.model().settings === null) void loadSettings(tabId);
    void loadPrepared(tabId);
  };

  // ---- açma / kapatma
  const connect = async (tabId: string, e: Entry) => {
    const gen = ++e.gen;
    const m = e.model();
    try {
      const info = await api.dashOpen(m.serverId, m.databaseOid);
      if (!alive(tabId, e, gen)) {
        void api.dashClose(info.id).catch(() => {});
        return;
      }
      patch(e, { info, status: "ready", error: null });
      syncPoller(e);
    } catch (err) {
      if (!alive(tabId, e, gen)) return;
      const p = toPayload(err);
      const gone = GONE_CODES.has(p.code);
      patch(e, { status: gone ? "gone" : "lost", error: gone ? p.message : lostError(p) });
      syncPoller(e);
    }
  };

  /** Sekme modelini kurar (`connecting`) ve `dash_open` ile kimlik alır. Dönüş: sekme kimliği (açılış denemesi bitince). */
  const open = async (input: DashTabInput): Promise<string> => {
    const tabId = input.id ?? crypto.randomUUID();
    if (entries.has(tabId)) return tabId;
    const [m, set] = createSignal<DashModel>({
      id: tabId,
      serverId: input.serverId,
      serverName: input.serverName,
      databaseOid: input.databaseOid,
      info: null,
      status: "connecting",
      error: null,
      section: "overview",
      paused: false,
      intervalMs: DEFAULT_INTERVAL_MS,
      overview: null,
      ring: new SampleRing(),
      rates: emptyRates(),
      activity: null,
      locks: null,
      settings: null,
      prepared: null,
      statements: null,
      statementsOrder: "total_time",
      sectionError: {},
      signaling: false,
    });
    // Tur geri çağrısı yalnızca `e` kurulduktan sonra çalışır.
    let e!: Entry;
    const poller = createPoller(pd, () => tick(tabId, e));
    e = { model: m, set, poller, tabActive: false, gen: 0, listAt: {}, loading: new Set() };
    entries.set(tabId, e);
    bump();
    await connect(tabId, e);
    return tabId;
  };

  /** Sekme kapanınca: yenileme durur, uçuştaki yanıtlar yoksayılır, `dash_close`. */
  const close = (tabId: string) => {
    const e = entries.get(tabId);
    if (!e) return;
    entries.delete(tabId);
    bump();
    e.gen++;
    e.poller.dispose();
    // Bu sekmenin açık onay diyaloğu vazgeçilmiş sayılır.
    if (signalGate.request()?.tabId === tabId) signalGate.answer(false);
    const info = e.model().info;
    if (info) void api.dashClose(info.id).catch(() => {});
  };

  /** `lost` sekmeyi yeni `DashId` ile açar; grafik geçmişi sıfırlanır. `gone` yeniden açılmaz. */
  const reopen = async (tabId: string) => {
    const e = entries.get(tabId);
    if (!e || e.model().status !== "lost") return;
    e.listAt = {};
    patch(e, { status: "connecting", info: null, error: null, ring: new SampleRing(), rates: emptyRates(), sectionError: {} });
    await connect(tabId, e);
  };

  /** Arka uç sunucunun oturumlarını kapattı (sil/kes/kilit değişti): ilgili sekmeler `lost`/`gone`, yenileme durur. */
  const serverSessionsClosed = (serverId: string, gone: boolean) => {
    for (const e of entries.values()) {
      const m = e.model();
      if (m.serverId !== serverId) continue;
      if (m.status === "gone") continue;
      e.gen++;
      patch(e, { status: gone ? "gone" : "lost", info: null, signaling: false });
      syncPoller(e);
      // Arka uç her durumda kapatmaz (ör. yalnızca kilit/etiket değişimi): eski kimlik açık kalmasın. Kapanmışsa
      // `not_found` döner, zararsız.
      if (m.info) void api.dashClose(m.info.id).catch(() => {});
    }
  };

  // ---- denetimler
  const setActive = (tabId: string | null) => {
    for (const [id, e] of entries) {
      e.tabActive = id === tabId;
      syncPoller(e);
    }
  };

  const setSection = (tabId: string, section: Section) => {
    const e = entries.get(tabId);
    if (!e || e.model().section === section) return;
    patch(e, { section });
    if (section === "settings") {
      if (e.tabActive) enterSettings(tabId); // arka planda: etkinleşince (syncPoller)
      return;
    }
    // Liste bölümüne giriş: bayatsa hemen yenile (tek uçuşta kuralıyla); etkin değilse dönüşteki tur getirir.
    if (section !== "overview" && listDue(e, section, pd.now())) {
      delete e.listAt[section];
      kick(e);
    }
  };

  const setPaused = (tabId: string, paused: boolean) => {
    const e = entries.get(tabId);
    if (!e) return;
    patch(e, { paused });
    e.poller.setPaused(paused);
  };

  const setIntervalMs = (tabId: string, intervalMs: number) => {
    const e = entries.get(tabId);
    if (!e) return;
    const ms = clampInterval(intervalMs);
    patch(e, { intervalMs: ms });
    e.poller.setIntervalMs(ms);
  };

  /** Elle "Yenile": açık bölümün listesi de beklemeden istenir. */
  const refreshNow = (tabId: string) => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || m.status !== "ready") return;
    delete e.listAt[m.section];
    if (!e.tabActive) return;
    kick(e);
    if (m.section === "settings") {
      void loadSettings(tabId);
      void loadPrepared(tabId);
    }
  };

  const setStatementsOrder = (tabId: string, order: StatementsOrder) => {
    const e = entries.get(tabId);
    if (!e || e.model().statementsOrder === order) return;
    patch(e, { statementsOrder: order });
    delete e.listAt.statements;
    kick(e);
  };

  // ---- iptal / sonlandırma
  /**
   * Tek uçuşta: uçuşta iken ikinci çağrı hiçbir şey yapmaz (`busy`). Göndermeden önce canlı oturum listesi çekilir ve
   * `revalidate` ile satırın hâlâ aynı oturum olduğu doğrulanır; değilse sinyal gönderilmez.
   */
  const signal = async (tabId: string, row: ActivityRow, kind: SignalKind): Promise<SignalResult> => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m) return "error";
    if (m.signaling) return "busy";
    if (m.status !== "ready" || !m.info) return "error";
    const gen = e.gen;
    const id = m.info.id;
    patch(e, { signaling: true });
    try {
      let live: ActivityReport;
      try {
        live = await api.dashActivity(id);
      } catch (err) {
        if (!alive(tabId, e, gen)) return "error";
        const p = toPayload(err);
        if (LOST_CODES.has(p.code) || GONE_CODES.has(p.code)) fail(e, "sessions", err);
        else notify(errorText(p), "error");
        return "error";
      }
      if (!alive(tabId, e, gen)) return "error";
      patch(e, { activity: live });
      const check = revalidate(row, live.rows);
      if (!check.ok && check.reason === "gone" && live.truncated) {
        // Liste kesildi: pid kesimin ötesinde olabilir; "oturum yok" denemez, işlem yapılmaz.
        notify(td.signal.unverified, "error");
        return "error";
      }
      if (!check.ok) {
        notify(check.reason === "gone" ? td.signal.notFound : td.signal.changed, "info");
        return check.reason === "gone" ? "notFound" : "changed";
      }
      let outcome: SignalOutcome;
      try {
        outcome = await api.dashSignal(id, row.pid, kind);
      } catch (err) {
        if (!alive(tabId, e, gen)) return "error";
        const p = toPayload(err);
        if (LOST_CODES.has(p.code) || GONE_CODES.has(p.code)) {
          fail(e, "sessions", err);
          return "error";
        }
        // Yetki hatası (SQLSTATE 42501) `code: "query"` ile gelir: dostça metin + PostgreSQL'in metni (`errorText`).
        notify(p.code === "invalid" ? td.signal.ownSession : errorText(p), "error");
        return "error";
      }
      if (!alive(tabId, e, gen)) return outcome === "sent" ? "sent" : "notFound";
      if (outcome === "sent") {
        notify(td.signal.sent(kind), "success");
        refreshNow(tabId);
        return "sent";
      }
      notify(td.signal.notFound, "info");
      return "notFound";
    } finally {
      // Kuşak değiştiyse (kayıp/yeniden açma) bayrak orada sıfırlandı; yeni kuşağın sinyaline dokunulmaz.
      if (alive(tabId, e, gen)) patch(e, { signaling: false });
    }
  };

  /** Onay kapısı → (onaylanırsa) `signal`. Diyalog `signalGate.request()`'i çizer, `signalGate.answer(bool)` ile yanıtlar. */
  const confirmSignal = async (tabId: string, row: ActivityRow, kind: SignalKind): Promise<SignalResult | "cancelled"> => {
    if (!entries.has(tabId)) return "cancelled";
    const ok = await signalGate.ask({ tabId, row, kind, warnings: signalWarnings(row, kind) });
    if (!ok) return "cancelled";
    return signal(tabId, row, kind);
  };

  return {
    model,
    ids: () => (registry(), [...entries.keys()]),
    open,
    close,
    reopen,
    serverSessionsClosed,
    setActive,
    setSection,
    setPaused,
    setIntervalMs,
    refreshNow,
    setStatementsOrder,
    loadSettings,
    loadPrepared,
    signal,
    confirmSignal,
    signalGate,
  };
}

export type DashController = ReturnType<typeof createDashController>;
