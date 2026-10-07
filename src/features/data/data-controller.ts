// Veri sekmelerinin durumu (tembel parça): sekme başına model sinyali, salt okunur okuma oturumu, yükleme/sayfalama,
// düzenleme → debounce'lu önizleme → kapılı uygulama ve yaşam döngüsü (kapatma, oturum/bağlantı kaybı, sunucu kaydı).
// designer-controller.ts kalıbı (entries Map + registry sinyali + kuşak sayacı); designer/* içe aktarılmaz (Karar 2).
//
// Kurallar:
// - Okuma oturumu sekmeye özeldir (`qt_open_readonly`, autocommit, 30 sn ifade zaman aşımı); içinde YALNIZCA
//   `data_select` çıktısı çalışır (kullanıcı SQL'i yok).
// - `Loaded` içeriği değiştiğinde (yeniden yükleme, sıralama/filtre, başarılı uygulama) düzenleme durumu `EMPTY_EDIT`'e
//   sıfırlanır; sayfalama (yalnızca sona ekleme) düzenlemeyi korur. Sekme başına en çok `ROW_CAP` satır.
// - Önizleme: en son istek kazanır (`previewSeq`); kapanan sekmenin / eski kuşağın yanıtı durum yazmaz.
// - Uygulama tek uçuştadır; kapı çağrı anında sunucu kaydıyla yeniden hesaplanır; kapalıysa istek gönderilmez.
//   `outcomeUnknown` ve çakışma sonrası otomatik yeniden deneme YOKTUR.
// - Gizlilik: hücre değeri/anahtar/sürüm/filtre/SQL günlüğe, depoya, bildirime girmez (bu modül `console`/toast
//   kullanmaz). PG iletisi yalnızca `loadError`/`outcome`/`previewError` alanlarında (arayüz şeridi) durur.
// - Tepkisellik: `model(tabId)`, `isDirty`, `ids` `registry` sinyaline bağlıdır; girdi `openTab` içinde EŞZAMANLI kurulur.
import { createSignal, type Accessor, type Setter } from "solid-js";
import { tdata } from "../../app/strings-data";
import type { DataOutcome } from "../../ipc/bindings/DataOutcome";
import type { DataSource } from "../../ipc/bindings/DataSource";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { QueryEvent } from "../../ipc/bindings/QueryEvent";
import type { SessionOptions } from "../../ipc/bindings/SessionOptions";
import { errorText, toPayload } from "../../ipc/errors";
import { applyOptions, canSubmit, dataGate, type DataGate } from "./data-apply-model";
import type { DataApi } from "./data-commands";
import { classifyDataError, fieldIssues, outcomeText, type FieldIssues } from "./data-errors";
import {
  EMPTY_EDIT, applyResults, changeCount, isDirty as editDirty, resultIssues, toChangeSet, type ChangeOrigin, type EditState,
  type Loaded, type Row,
} from "./data-edit-model";
import { EMPTY_SELECT, selectErrors as mapSelectErrors, toRequest, type SelectState } from "./data-select-model";

/** İlk yükleme satır sayısı (`qt_run` limiti; `qt_more` da bu boyda parça verir). */
export const FIRST_LIMIT = 1000;
/** Sekme başına en çok yüklü satır (Karar 7). */
export const ROW_CAP = 50_000;
export const PREVIEW_DELAY_MS = 150;
export const READ_OPTIONS: SessionOptions = { mode: "autocommit", statementTimeoutMs: 30_000 };

export type DataStatus = "loading" | "ready" | "error" | "gone" | "disconnected";
export type DataOutcomeText = ReturnType<typeof outcomeText>;
export interface SelectErrors { filter: string[]; order: string[]; general: string[] }

export interface DataModel {
  tabId: string;
  serverId: string;
  serverName: string;
  databaseOid: number;
  object: ObjectRef;
  status: DataStatus;
  /** Son yüklenen kaynak (`loaded.source` ile aynı; yükleme sürerken bir öncekinin yerini almaz). */
  source: DataSource | null;
  loaded: Loaded | null;
  edit: EditState;
  /** İmleçte daha fazla satır var (ve sınıra ulaşılmadı). */
  more: boolean;
  loadingMore: boolean;
  truncatedAtCap: boolean;
  select: SelectState;
  selectErrors: SelectErrors;
  view: PlanView | null;
  /** `view` mevcut düzenlemeye ait (arada düzenleme yok). */
  viewIsCurrent: boolean;
  /** Önizleme zamanlayıcıda/uçuşta. */
  pending: boolean;
  previewError: string | null;
  applying: boolean;
  /** Her `patch`'te modelden hesaplanır (sunucu kaydı `locked`/`production` alanlarına yansıtılır). */
  gate: DataGate;
  confirm: { checked: boolean };
  /** Son önizlemenin alan hataları (`PlanView.errors` → hücre/satır/genel). */
  issues: FieldIssues | null;
  /** Çakışan var olan satırın indeksi (UPDATE/DELETE); INSERT çakışmasında `null` (`failedChange` yeni satırı gösterir). */
  conflictRow: number | null;
  /** Son `Failed`'ın değişikliği (vurgu için; Err/başarıda `null`). */
  failedChange: ChangeOrigin | null;
  outcome: DataOutcomeText | null;
  /** Kaynak tanımı bayat (`plan_changed` / yeniden bağlanınca parmak izi farklı): yeniden yükleme gerekir. */
  needsReload: boolean;
  /** Sonuç belirsiz (COMMIT SQLSTATE'siz): yeniden yüklenene dek kapı kapalı. */
  outcomeUnknown: boolean;
  loadError: string | null;
  /** Başarılı uygulama notu (`aria-live`). */
  postApplyNote: string | null;
  locked: boolean;
  production: boolean;
  /** Kirli sekmede F5 (`reloadNow`): arayüz "düzenlemeler atılır" onayı sorar, yanıt `answerReload` ile. */
  askReload: boolean;
}

export interface ServerInfo { name: string; locked: boolean; production: boolean }
export interface OpenTabInput { tabId: string; serverId: string; databaseOid: number; object: ObjectRef }

export interface DataTabCtl {
  model(tabId: string): DataModel | null;
  ids(): string[];
  isDirty(tabId: string): boolean;
  openTab(input: OpenTabInput): string;
  /** Kirli ise yalnızca `discard: true` ile (çağıran onay almıştır); aksi halde `false`. */
  reload(tabId: string, opts?: { discard: boolean }): Promise<boolean>;
  /** F5: kirli değilse yeniden yükler ve `true`; kirliyse yalnızca `askReload` kurulur, `false` (arayüz onay sorar). */
  reloadNow(tabId: string): boolean;
  /** `askReload` onayının yanıtı: bayrak düşer; `discard` ise düzenlemeler atılıp yeniden yüklenir. */
  answerReload(tabId: string, discard: boolean): Promise<boolean>;
  /** Bağlantısı kesilen sekmeyi yeniden bağlar; kaynak aynıysa düzenlemeler korunur. */
  reconnect(tabId: string): Promise<boolean>;
  loadMore(tabId: string): Promise<void>;
  /** Sıralama/filtre → yeniden yükleme; kirliyse yalnızca `discard: true` ile. */
  setSelect(tabId: string, next: SelectState, opts?: { discard: boolean }): Promise<boolean>;
  edit(tabId: string, fn: (l: Loaded, e: EditState) => EditState): void;
  setConfirm(tabId: string, checked: boolean): void;
  apply(tabId: string): Promise<void>;
  cancelApply(tabId: string): void;
  close(tabId: string): void;
  serverSessionsClosed(serverId: string, gone?: boolean): void;
  serverChanged(serverId: string): void;
}

export interface DataTimer {
  set(fn: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

export interface DataControllerDeps {
  api: Pick<
    DataApi,
    "dataLoad" | "dataSelect" | "dataPreview" | "dataApply" | "dataCancel" | "qtOpenReadonly" | "qtRun" | "qtMore" | "qtClose" | "connect"
  >;
  /** Güncel sunucu kaydı (ad/kilit/üretim); kayıt yoksa `null`. */
  serverInfo(serverId: string): ServerInfo | null;
  newId?: () => string;
  previewDelay?: number;
  timer?: DataTimer;
}

interface Entry {
  model: Accessor<DataModel>;
  set: Setter<DataModel>;
  /** Kuşak: yükleme/kayıp/kapatmada artar; eski kuşağın yanıtı yoksayılır. */
  gen: number;
  /** En son önizleme isteği; daha eskinin yanıtı yoksayılır. */
  previewSeq: number;
  timer: unknown;
  session: string | null;
  applyId: string | null;
  /** `loaded` satırlarını üreten seçim (`started`'ta güncellenir); başarısız yüklemede `select` buna döner. */
  loadedSelect: SelectState;
}

type Step = "source" | "select" | "session" | "run";
class StepFailure {
  constructor(readonly step: Step, readonly error: unknown) {}
}
const step = <T>(s: Step, p: Promise<T>): Promise<T> =>
  p.catch((error: unknown) => {
    throw new StepFailure(s, error);
  });

const NO_ERRORS = (): SelectErrors => ({ filter: [], order: [], general: [] });
const failedText = (text: string): DataOutcomeText => ({ kind: "failed", text });

/** Kapı modelden hesaplanır; `locked`/`production` uygulama anında sunucu kaydıyla sıkılaştırılabilir. */
function gateOf(m: DataModel, locked = m.locked, production = m.production): DataGate {
  const l = m.loaded;
  return dataGate({
    locked,
    production,
    editable: l !== null && l.source.editable && l.hasVersion,
    view: m.view,
    viewIsCurrent: m.viewIsCurrent,
    pending: m.pending,
    applying: m.applying,
    connected: m.status !== "disconnected" && m.status !== "gone",
    // "Daha fazla" sürerken oturum meşgul ve satırlar büyüyor: uygulama kapalı.
    loading: m.status === "loading" || m.status === "error" || l === null || m.loadingMore,
    needsReload: m.needsReload,
    outcomeUnknown: m.outcomeUnknown,
    dirty: l !== null && changeCount(l, m.edit) > 0,
  });
}

export function createDataController(deps: DataControllerDeps): DataTabCtl {
  const { api } = deps;
  const newId = deps.newId ?? (() => crypto.randomUUID());
  const delay = deps.previewDelay ?? PREVIEW_DELAY_MS;
  const timer: DataTimer = deps.timer ?? {
    set: (fn, ms) => setTimeout(fn, ms),
    clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  };
  const entries = new Map<string, Entry>();
  // `entries` düz Map'tir (tepkisiz); okuyucular bu sinyale bağlanır, girdi eklenince/silinince yeniden çalışır.
  const [registry, bump] = createSignal<void>(undefined, { equals: false });

  const model = (tabId: string): DataModel | null => (registry(), entries.get(tabId)?.model() ?? null);
  const patch = (e: Entry, p: Partial<DataModel>) =>
    e.set((m) => {
      const n = { ...m, ...p };
      return { ...n, gate: gateOf(n) };
    });
  const alive = (tabId: string, e: Entry, gen: number) => entries.get(tabId) === e && e.gen === gen;

  // ---- en iyi çaba yan etkiler (hatalar yutulur; değer içermez)
  const quietly = (f: () => Promise<unknown>) => {
    try {
      void f().catch(() => {});
    } catch {
      /* eşzamanlı hata da yok sayılır */
    }
  };
  const closeSession = (session: string) => quietly(() => api.qtClose(session, "rollback"));
  const dropSession = (e: Entry) => {
    if (e.session) closeSession(e.session);
    e.session = null;
  };
  const connect = async (serverId: string) => {
    try {
      return await api.connect(serverId);
    } catch {
      return false;
    }
  };

  // ---- önizleme
  const stopPreview = (e: Entry) => {
    if (e.timer !== null) timer.clear(e.timer);
    e.timer = null;
    e.previewSeq++;
  };

  /**
   * Bağlantı/yokluk: kuşak artar, önizleme durur; düzenlemeler KORUNUR, `select` satırları üreten seçime döner.
   * Uçuşta uygulama varsa yanıtı artık okunmaz (kuşak): yazılmış olabilir → sonuç belirsiz (`outcomeUnknown`); kapı
   * `unknown` olur ve yalnızca yeniden yükleme (düzenlemeleri atarak) açar — aksi halde yeniden kaydetmek INSERT'leri
   * çoğaltabilirdi.
   */
  const markDown = (e: Entry, status: "disconnected" | "gone", closeOld: boolean) => {
    e.gen++;
    stopPreview(e);
    const inFlight = e.applyId !== null;
    if (e.applyId) {
      const id = e.applyId;
      quietly(() => api.dataCancel(id));
      e.applyId = null;
    }
    if (closeOld) dropSession(e);
    else e.session = null;
    patch(e, {
      status, pending: false, applying: false, loadingMore: false, select: e.loadedSelect,
      ...(inFlight ? { outcomeUnknown: true, view: null, viewIsCurrent: false, outcome: { kind: "unknown" as const, text: tdata.outcome.unknown } } : {}),
    });
  };

  const previewFailed = (e: Entry, p: ErrorPayload) => {
    switch (classifyDataError(p)) {
      case "disconnected":
        markDown(e, "disconnected", true);
        return;
      case "gone":
        markDown(e, "gone", true);
        return;
      case "plan_changed":
        patch(e, { pending: false, needsReload: true, view: null, viewIsCurrent: false });
        return;
      case "locked":
        patch(e, { pending: false, locked: true });
        return;
      default:
        patch(e, { pending: false, previewError: errorText(p) });
    }
  };

  /** Mevcut düzenlemenin önizlemesi (`immediately`: beklemeden). Değişiklik yoksa görünüm düşer, istek yok. */
  const schedulePreview = (tabId: string, e: Entry, immediately: boolean) => {
    stopPreview(e);
    const m = e.model();
    const l = m.loaded;
    if (!l || m.status !== "ready" || m.applying || m.needsReload || m.outcomeUnknown) {
      patch(e, { pending: false });
      return;
    }
    const cs = toChangeSet(l, m.edit);
    if (cs.changes.length === 0) {
      patch(e, { view: null, viewIsCurrent: false, pending: false, issues: null, previewError: null });
      return;
    }
    patch(e, { pending: true, viewIsCurrent: false });
    const run = async () => {
      e.timer = null;
      const id = ++e.previewSeq;
      const gen = e.gen;
      try {
        const view = await api.dataPreview(m.serverId, m.databaseOid, { object: { ...m.object }, fingerprint: l.source.fingerprint, changes: cs.changes });
        if (!alive(tabId, e, gen) || id !== e.previewSeq) return;
        const cur = e.model();
        // Plan değiştiyse (farklı hash) önceki onay geçersiz.
        const same = cur.view !== null && cur.view.hash === view.hash;
        patch(e, {
          view, viewIsCurrent: true, pending: false, previewError: null, issues: fieldIssues(view.errors, cs),
          confirm: same ? cur.confirm : { checked: false },
        });
      } catch (err) {
        if (alive(tabId, e, gen) && id === e.previewSeq) previewFailed(e, toPayload(err));
      }
    };
    if (immediately) void run();
    else e.timer = timer.set(() => void run(), delay);
  };

  // ---- yükleme
  /** `Loaded` içeriği değişti: düzenleme ve ona bağlı her şey sıfırlanır (R2). */
  const replaceLoaded = (e: Entry, loaded: Loaded | null, p: Partial<DataModel> = {}) => {
    stopPreview(e);
    patch(e, {
      loaded, source: loaded?.source ?? e.model().source, edit: EMPTY_EDIT, view: null, viewIsCurrent: false, pending: false,
      previewError: null, issues: null, conflictRow: null, failedChange: null, outcome: null, needsReload: false,
      outcomeUnknown: false, confirm: { checked: false }, more: false, truncatedAtCap: false, postApplyNote: null, ...p,
    });
  };

  /** Satırları sona ekler (düzenleme korunur); `ROW_CAP`'te kırpar ve `more`'u kapatır. */
  const appendRows = (e: Entry, incoming: Row[], more: boolean) => {
    const m = e.model();
    const l = m.loaded;
    if (!l) return;
    const room = Math.max(0, ROW_CAP - l.rows.length);
    const take = incoming.length > room ? incoming.slice(0, room) : incoming;
    const total = l.rows.length + take.length;
    const capped = incoming.length > room || (total >= ROW_CAP && more);
    patch(e, {
      loaded: take.length > 0 ? { ...l, rows: l.rows.concat(take) } : l,
      more: capped ? false : more,
      truncatedAtCap: m.truncatedAtCap || capped,
    });
  };

  const ensureSession = async (tabId: string, e: Entry, gen: number): Promise<string | null> => {
    if (e.session) return e.session;
    const m = e.model();
    const s = await step("session", api.qtOpenReadonly(m.serverId, m.databaseOid, { ...READ_OPTIONS }));
    if (entries.get(tabId) === e && e.session === null && e.gen === gen) {
      e.session = s;
      return s;
    }
    closeSession(s);
    return null;
  };

  /** Tek deneme: kaynak → seçim → oturum → `qt_run`. Hatalar `StepFailure` olarak fırlar. */
  const loadOnce = async (tabId: string, e: Entry, gen: number): Promise<void> => {
    const m0 = e.model();
    const source = await step("source", api.dataLoad(m0.serverId, m0.databaseOid, { ...m0.object }));
    if (!alive(tabId, e, gen)) return;
    // Kaynağın kilit bilgisi de kapıya yansır (sunucu kaydı ayrıca `serverChanged` ile).
    if (source.locked && !e.model().locked) patch(e, { locked: true });
    const session = await ensureSession(tabId, e, gen);
    if (!session || !alive(tabId, e, gen)) return;
    const res = await step("select", api.dataSelect(m0.serverId, m0.databaseOid, { ...m0.object }, source.fingerprint, toRequest(e.model().select)));
    if (!alive(tabId, e, gen)) return;
    if (res.result === "invalid") {
      // Alan hatası: satırlar (ve düzenlemeler) değişmez.
      const had = e.model().loaded !== null;
      patch(e, { selectErrors: mapSelectErrors(res.errors), status: had ? "ready" : "error", source: had ? e.model().source : source });
      return;
    }
    const sel = res.select;
    let bad = false;
    let failed = false;
    const onEvent = (ev: QueryEvent) => {
      if (bad || !alive(tabId, e, gen)) return;
      if (ev.type === "started") {
        const width = source.columns.length;
        const expected = width + (sel.hasVersion ? 1 : 0);
        if (ev.columns.length !== expected || (sel.hasVersion && sel.versionIndex !== width)) {
          bad = true;
          replaceLoaded(e, null, { status: "error", loadError: tdata.load.columnsMismatch, source });
          return;
        }
        e.loadedSelect = e.model().select;
        replaceLoaded(e, { source, rows: [], versionIndex: sel.versionIndex, hasVersion: sel.hasVersion }, { selectErrors: NO_ERRORS() });
      } else if (ev.type === "rows") {
        appendRows(e, ev.rows, ev.more);
      } else if (ev.type === "failed") {
        failed = true;
        // Yarım sonuç yeni sorgunun sonucu gibi gösterilmez.
        replaceLoaded(e, null, { status: "error", loadError: errorText(ev.error) });
      }
    };
    await step("run", api.qtRun(session, sel.sql, { type: "all" }, FIRST_LIMIT, false, onEvent));
    if (!alive(tabId, e, gen) || bad || failed) return;
    patch(e, { status: e.model().loaded ? "ready" : "error" });
  };

  /** Oturum kaybı/yeniden açma gerektiren kodlar (`query-controller` `ensureSession` kalıbı). */
  const retriable = (p: ErrorPayload, s: Step) =>
    p.code === "not_connected" || ((s === "session" || s === "run") && (p.code === "session_reset" || p.code === "not_found"));

  /**
   * Yükleme hatası. Bağlantı/yokluk → `markDown` (satırlar + düzenlemeler korunur, `select` geri döner; yeniden
   * bağlanınca devam). Diğer hata (`wipe`: yükleme/sıralama/filtre isteği) → eski satırlar yeni sorgunun sonucu gibi
   * gösterilmez: `loaded = null`, düzenleme sıfır (kirliyse çağıran zaten onay almıştır), `status = "error"`.
   * `wipe = false` (yeniden bağlanma): satır ve düzenlemeler korunur, yalnızca hata metni.
   */
  const loadFailed = (e: Entry, p: ErrorPayload, s: Step, wipe = true) => {
    if (p.code === "connection_lost" || p.code === "not_connected") markDown(e, "disconnected", true);
    else if (p.code === "unknown_server" || (p.code === "not_found" && s === "source")) markDown(e, "gone", true);
    else if (wipe) replaceLoaded(e, null, { status: "error", loadError: errorText(p) });
    else patch(e, { status: e.model().loaded ? "ready" : "error", loadError: errorText(p) });
  };

  const load = async (tabId: string, e: Entry): Promise<void> => {
    const gen = ++e.gen;
    stopPreview(e);
    patch(e, { status: "loading", loadError: null, loadingMore: false, pending: false });
    for (let attempt = 0; ; attempt++) {
      try {
        await loadOnce(tabId, e, gen);
        return;
      } catch (x) {
        if (!alive(tabId, e, gen)) return;
        const f = x instanceof StepFailure ? x : new StepFailure("source", x);
        const p = toPayload(f.error);
        // Bir kez: yeniden bağlan + oturumu yeniden aç + yeniden dene.
        if (attempt === 0 && retriable(p, f.step)) {
          dropSession(e);
          if ((await connect(e.model().serverId)) && alive(tabId, e, gen)) continue;
          if (!alive(tabId, e, gen)) return;
        }
        loadFailed(e, p, f.step);
        return;
      }
    }
  };

  // ---- açma
  const openTab = (input: OpenTabInput): string => {
    if (entries.has(input.tabId)) return input.tabId;
    const info = deps.serverInfo(input.serverId);
    const initial: DataModel = {
      tabId: input.tabId, serverId: input.serverId, serverName: info?.name ?? "", databaseOid: input.databaseOid,
      object: { ...input.object }, status: "loading", source: null, loaded: null, edit: EMPTY_EDIT, more: false,
      loadingMore: false, truncatedAtCap: false, select: EMPTY_SELECT, selectErrors: NO_ERRORS(), view: null,
      viewIsCurrent: false, pending: false, previewError: null, applying: false, gate: { kind: "blocked", reason: "loading" },
      confirm: { checked: false }, issues: null, conflictRow: null, failedChange: null, outcome: null, needsReload: false,
      outcomeUnknown: false, loadError: null, postApplyNote: null, locked: info?.locked ?? false, production: info?.production ?? false,
      askReload: false,
    };
    const [m, set] = createSignal<DataModel>({ ...initial, gate: gateOf(initial) });
    const e: Entry = { model: m, set, gen: 0, previewSeq: 0, timer: null, session: null, applyId: null, loadedSelect: EMPTY_SELECT };
    entries.set(input.tabId, e);
    bump();
    void load(input.tabId, e);
    return input.tabId;
  };

  const isDirty = (tabId: string): boolean => {
    const m = model(tabId);
    return m !== null && editDirty(m.edit);
  };

  const reload = async (tabId: string, opts?: { discard: boolean }): Promise<boolean> => {
    const e = entries.get(tabId);
    const m = e?.model();
    // "Daha fazla" ya da yükleme sürerken oturum meşgul: reddedilir (ikinci yükleme aynı okuma oturumunda `busy`
    // alır ve ızgarayı silerdi; arayüz düğmeleri kapatır).
    if (!e || !m || m.applying || m.loadingMore || m.status === "loading" || m.status === "gone") return false;
    if (editDirty(m.edit) && !opts?.discard) return false;
    const info = deps.serverInfo(m.serverId);
    patch(e, { askReload: false, ...(info ? { serverName: info.name, locked: info.locked, production: info.production } : {}) });
    await load(tabId, e);
    return true;
  };

  const reloadNow = (tabId: string): boolean => {
    const m = model(tabId);
    if (!m || m.applying || m.loadingMore || m.status === "loading" || m.status === "gone") return false;
    if (isDirty(tabId)) {
      const e = entries.get(tabId);
      if (e) patch(e, { askReload: true });
      return false;
    }
    void reload(tabId);
    return true;
  };

  const answerReload = async (tabId: string, discard: boolean): Promise<boolean> => {
    const e = entries.get(tabId);
    if (!e) return false;
    patch(e, { askReload: false });
    return discard ? reload(tabId, { discard: true }) : false;
  };

  const reconnect = async (tabId: string): Promise<boolean> => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || m.status !== "disconnected") return false;
    const l = m.loaded;
    if (!l || !editDirty(m.edit)) return reload(tabId, { discard: true });
    // Kirli: kaynak aynıysa düzenlemeler korunur. İmleç oturumla gittiği için "daha fazla" kapanır.
    const gen = ++e.gen;
    patch(e, { status: "loading", loadError: null });
    const ok = await connect(m.serverId);
    if (!alive(tabId, e, gen)) return false;
    if (!ok) {
      patch(e, { status: "disconnected" });
      return false;
    }
    try {
      const source = await api.dataLoad(m.serverId, m.databaseOid, { ...m.object });
      if (!alive(tabId, e, gen)) return false;
      const same = source.fingerprint === l.source.fingerprint;
      patch(e, { status: "ready", more: false, needsReload: !same, locked: e.model().locked || source.locked });
      if (same) schedulePreview(tabId, e, true);
      return true;
    } catch (err) {
      if (alive(tabId, e, gen)) loadFailed(e, toPayload(err), "source", false);
      return false;
    }
  };

  const loadMore = async (tabId: string): Promise<void> => {
    const e = entries.get(tabId);
    const m = e?.model();
    // Uygulama sürerken yok: sonuç satırları ve sayfa aynı anda değişmesin (oturum da tektir).
    if (!e || !m || !m.loaded || !m.more || m.loadingMore || m.applying || m.status !== "ready" || !e.session) return;
    if (m.loaded.rows.length >= ROW_CAP) {
      patch(e, { more: false, truncatedAtCap: true });
      return;
    }
    const gen = e.gen;
    const session = e.session;
    patch(e, { loadingMore: true, loadError: null });
    try {
      await api.qtMore(session, 0, (ev) => {
        if (!alive(tabId, e, gen)) return;
        if (ev.type === "rows") appendRows(e, ev.rows, ev.more);
        else if (ev.type === "failed") patch(e, { loadError: errorText(ev.error), more: false });
      });
    } catch (err) {
      if (!alive(tabId, e, gen)) return;
      const p = toPayload(err);
      if (p.code === "connection_lost") {
        markDown(e, "disconnected", true);
        return;
      }
      // İmleç kapandı (5 dk boşta) ya da oturum düştü: yeniden yükleme ipucu.
      if (p.code === "session_reset" || p.code === "not_found" || p.code === "not_connected") dropSession(e);
      patch(e, { more: false, loadError: tdata.more.expired });
    } finally {
      if (alive(tabId, e, gen)) patch(e, { loadingMore: false });
    }
  };

  const setSelect = async (tabId: string, next: SelectState, opts?: { discard: boolean }): Promise<boolean> => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || m.applying || m.loadingMore || m.status === "loading" || m.status === "gone") return false;
    if (editDirty(m.edit) && !opts?.discard) return false;
    patch(e, { select: { order: [...next.order], filter: next.filter } });
    await load(tabId, e);
    return true;
  };

  // ---- düzenleme
  const edit = (tabId: string, fn: (l: Loaded, e: EditState) => EditState): void => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || !m.loaded || m.status !== "ready" || m.applying) return;
    const next = fn(m.loaded, m.edit);
    if (next === m.edit) return;
    patch(e, { edit: next, postApplyNote: null, outcome: m.outcomeUnknown ? m.outcome : null });
    schedulePreview(tabId, e, false);
  };

  const setConfirm = (tabId: string, checked: boolean) => {
    const e = entries.get(tabId);
    if (e) patch(e, { confirm: { checked } });
  };

  // ---- uygulama
  const rejected = (tabId: string, e: Entry, p: ErrorPayload) => {
    switch (classifyDataError(p)) {
      case "disconnected":
        markDown(e, "disconnected", true);
        return;
      case "gone":
        markDown(e, "gone", true);
        return;
      case "plan_changed":
        patch(e, { needsReload: true, view: null, viewIsCurrent: false, outcome: failedText(tdata.gate.planChanged) });
        return;
      case "locked":
        patch(e, { locked: true, outcome: failedText(tdata.gate.locked) });
        return;
      case "confirmation_required":
        patch(e, { confirm: { checked: false }, outcome: failedText(tdata.gate.confirmationRequired) });
        schedulePreview(tabId, e, true);
        return;
      case "invalid_def":
        patch(e, { outcome: failedText(tdata.gate.invalidDef) });
        schedulePreview(tabId, e, true);
        return;
      default:
        patch(e, { outcome: failedText(errorText(p)) });
        schedulePreview(tabId, e, true);
    }
  };

  const apply = async (tabId: string): Promise<void> => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || m.applying || m.loadingMore || !m.loaded) return;
    // Kapı çağrı anında: sunucu kaydı (kilit/üretim) modelden katı olanla birleşir.
    const info = deps.serverInfo(m.serverId);
    const gate = gateOf(m, m.locked || (info?.locked ?? false), m.production || (info?.production ?? false));
    if (!canSubmit(gate, m.confirm.checked)) {
      // Görünüm bayat ama istek yok: önce önizleme.
      if (gate.kind === "blocked" && gate.reason === "pending" && !m.pending) schedulePreview(tabId, e, true);
      return;
    }
    const l = m.loaded;
    const view = m.view;
    const cs = toChangeSet(l, m.edit);
    if (!view || cs.changes.length === 0) return;
    const applyId = newId();
    stopPreview(e);
    e.applyId = applyId;
    const gen = e.gen;
    patch(e, { applying: true, outcome: null, conflictRow: null, failedChange: null, postApplyNote: null, previewError: null });
    let out: DataOutcome;
    try {
      out = await api.dataApply(
        m.serverId, m.databaseOid, { object: { ...m.object }, fingerprint: l.source.fingerprint, changes: cs.changes },
        applyOptions(applyId, view, m.confirm.checked),
      );
    } catch (err) {
      if (!alive(tabId, e, gen)) return;
      e.applyId = null;
      patch(e, { applying: false, confirm: { checked: false } });
      rejected(tabId, e, toPayload(err));
      return;
    }
    if (!alive(tabId, e, gen)) return;
    e.applyId = null;
    if (out.outcome === "applied") {
      // GÜNCEL satırlar üzerine işlenir: uygulama sırasında gelen sayfa satırları (yalnızca sona ekleme; indeksler
      // kaymaz, `origin.rowIndex` geçerli) kaybolmaz. Not: INSERT'ler sona eklendiğinden satır sayısı `ROW_CAP`'i
      // bilerek aşabilir (kullanıcının yazdığı satırlar atılmaz); sınırda `more` zaten kapalıdır.
      const cur = e.model();
      const base = cur.loaded ?? l;
      const issues = resultIssues(base, cs.origin, out.results);
      const note = tdata.status.applied(out.statements);
      replaceLoaded(e, applyResults(base, cs.origin, out.results), {
        applying: false, status: "ready", more: cur.more, truncatedAtCap: cur.truncatedAtCap,
        postApplyNote: issues.length > 0 ? `${note} ${tdata.status.refreshHint}` : note,
      });
      return;
    }
    const origin = cs.origin[out.failedIndex] ?? null;
    const outcome = outcomeText(out, { changeKind: origin?.kind ?? null });
    const base: Partial<DataModel> = { applying: false, confirm: { checked: false }, outcome, failedChange: origin };
    if (out.outcomeUnknown) {
      // Kalıcılaşmış olabilir: yeniden yükleme zorunlu, otomatik deneme yok.
      patch(e, { ...base, outcomeUnknown: true, view: null, viewIsCurrent: false });
      return;
    }
    if (out.conflict) {
      patch(e, { ...base, conflictRow: origin && origin.kind !== "insert" ? origin.rowIndex : null });
    } else {
      // Kısıt/tür hatası, iptal…: hangi değişikliğin başarısız olduğu metne eklenir (1 tabanlı).
      patch(e, { ...base, outcome: { ...outcome, text: tdata.outcome.failed(out.failedIndex, outcome.text) } });
    }
    // Düzenlemeler korunur; görünüm yenilenir (yeniden göndermek kullanıcının işi).
    schedulePreview(tabId, e, true);
  };

  const cancelApply = (tabId: string) => {
    const e = entries.get(tabId);
    const id = e?.applyId;
    if (id) quietly(() => api.dataCancel(id));
  };

  // ---- yaşam döngüsü
  const close = (tabId: string) => {
    const e = entries.get(tabId);
    if (!e) return;
    entries.delete(tabId);
    bump();
    e.gen++;
    stopPreview(e);
    if (e.applyId) quietly(() => api.dataCancel(e.applyId!));
    e.applyId = null;
    dropSession(e);
  };

  const serverSessionsClosed = (serverId: string, gone = false) => {
    for (const e of entries.values()) {
      const m = e.model();
      if (m.serverId !== serverId || m.status === "gone") continue;
      // Arka uç oturumları zaten kapattı: yeniden kapatma çağrısı yok.
      markDown(e, gone ? "gone" : "disconnected", false);
    }
  };

  const serverChanged = (serverId: string) => {
    const info = deps.serverInfo(serverId);
    if (!info) return;
    for (const e of entries.values()) {
      const m = e.model();
      if (m.serverId !== serverId) continue;
      // Kaynağın kilidi (yükleme anı) kayıttan bağımsız alt sınırdır.
      const locked = info.locked || (m.source?.locked ?? false);
      if (m.serverName === info.name && m.locked === locked && m.production === info.production) continue;
      patch(e, { serverName: info.name, locked, production: info.production, confirm: { checked: false } });
    }
  };

  return {
    model,
    ids: () => (registry(), [...entries.keys()]),
    isDirty,
    openTab,
    reload,
    reloadNow,
    answerReload,
    reconnect,
    loadMore,
    setSelect,
    edit,
    setConfirm,
    apply,
    cancelApply,
    close,
    serverSessionsClosed,
    serverChanged,
  };
}
