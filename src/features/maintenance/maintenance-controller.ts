// Bakım diyaloğunun durumu (tembel parça; tek örnek). Akış: `ddl_target_of` → `ddl_load_maintenance_target` → işlem
// ve seçenekler → gecikmeli `ddl_preview` → kapılı `ddl_apply` (+ ~1 sn ilerleme yoklaması) → sonuç → (başarıda) hedef
// yeniden yüklenir.
//
// Kurallar:
// - Aşamalar: loading → ready ⇄ applying; `loadError` (yeniden dene). Sonuç `ready` aşamasında `result` alanında durur;
//   aynı seçeneklerle yeniden çalıştırma yeni önizleme ister.
// - Kuşak sayacı: açma/kapama/yeniden yükleme kuşağı artırır; eski kuşağın yanıtı durum yazmaz (kapalıyken gelen sonuç
//   yok sayılır).
// - Seçenek/işlem/hedef değişince sürüm ilerler: eski önizleme bayatlar (önizleme bitmeden Uygula kapalı), onay sıfırlanır.
// - `target` yüklenen haliyle değiştirilmeden gönderilir; `applyId` hem ApplyOptions'ta hem ilerleme sorgusunda aynıdır.
// - Ana modül (app/strings, explorer/*, designer/*) içe aktarılmaz: designer'daki kapı ve önizleme gecikmesi burada
//   küçük haliyle yinelenir (paylaşılan parça ana paketin `__vite__mapDeps` listesini büyütmesin).
// - Gizlilik: SQL, adlar ve iletiler `console.*`'a yazılmaz.
import { createSignal } from "solid-js";
import { tm } from "../../app/strings-maintenance";
import type { ToastKind } from "../../app/toast-model";
import type { ApplyOptions } from "../../ipc/bindings/ApplyOptions";
import type { ApplyOutcome } from "../../ipc/bindings/ApplyOutcome";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { MaintenanceOp } from "../../ipc/bindings/MaintenanceOp";
import type { MaintenanceProgress } from "../../ipc/bindings/MaintenanceProgress";
import type { MaintenanceTarget } from "../../ipc/bindings/MaintenanceTarget";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { ServerMessage } from "../../ipc/bindings/ServerMessage";
import type { Api } from "../../ipc/commands";
import {
  createProgressPoller,
  defaultOperation,
  maintenanceRequest,
  messageSummary,
  needsCcnewNotice,
  normalize,
  opBlocked,
  optionStates,
  opsFor,
  type OpName,
  type OptionState,
} from "./maintenance-model";

export const PREVIEW_DELAY_MS = 150;
export const LOCK_TIMEOUTS = [1000, 5000, 30000, 0];
export const DEFAULT_LOCK_TIMEOUT_MS = 5000;

export type MaintenancePhase = "loading" | "loadError" | "ready" | "applying";

export interface MaintenanceResult {
  kind: "applied" | "failed" | "cancelled";
  elapsedMs: number;
  messages: ServerMessage[];
  warnings: number;
  /** Başarısızlıkta PG iletisi (+ ayrıntı); iptalde null. */
  error: string | null;
  hint: string | null;
  /** Yarım kalan REINDEX CONCURRENTLY: `*_ccnew` uyarısı yinelenir. */
  ccnew: boolean;
}

export interface MaintenanceState {
  nodeId: string;
  serverId: string | null;
  databaseOid: number | null;
  object: ObjectRef | null;
  phase: MaintenancePhase;
  loadError: string | null;
  /** Yükleyici çıktısı (DEĞİŞTİRİLMEZ; isteğe olduğu gibi gider). */
  target: MaintenanceTarget | null;
  versionNum: number | null;
  operation: MaintenanceOp | null;
  version: number;
  view: PlanView | null;
  viewFor: number;
  pending: boolean;
  previewError: string | null;
  lockTimeoutMs: number;
  confirmed: boolean;
  applyId: string | null;
  startedAt: number | null;
  progress: MaintenanceProgress | null;
  /** Ardışık yoklama hatalarıyla ilerleme durdu (işlem sürüyor). */
  progressStopped: boolean;
  cancelRequested: boolean;
  closeAsk: boolean;
  /** Kapatma onaylandı: iptal istendi, sonuç bekleniyor; sonra diyalog kapanır. */
  closing: boolean;
  result: MaintenanceResult | null;
  /** Başarı/plan_changed sonrası hedef yeniden yükleniyor: seçenekler kilitli, kapı "loading". */
  reloading: boolean;
  /** Bilgi şeridi (ör. plan_changed sonrası yeniden yükleme). */
  notice: string | null;
}

export type GateReason = keyof typeof tm.blocked;
export type Gate = { ok: true; confirm: boolean } | { ok: false; reason: GateReason };

export interface MaintenanceControllerDeps {
  api: Pick<
    Api,
    "ddlTargetOf" | "ddlLoadMaintenanceTarget" | "ddlPreview" | "ddlApply" | "ddlCancel" | "ddlMaintenanceProgress"
  >;
  /** Sunucu sürümü (bilinmiyorsa null → istemci kapısı yok) ve salt okunur kilit (canlı; tek kaynak). */
  serverInfo(serverId: string): { versionNum: number | null; locked: boolean } | null;
  notify(text: string, kind: ToastKind): void;
  /** Denetleyici kendi kapandığında (kapatma onayı/boşta kapatma) App'in düğüm kimliğini temizlemesi için. */
  onClosed(): void;
  /** Başarılı uygulama sonrası Inspector'ı yeniden okut (App `lazyDeps`). */
  refreshInspector?(): void;
  newId(): string;
  now(): number;
  setTimer(fn: () => void, ms: number): unknown;
  clearTimer(h: unknown): void;
}

const CANCELLED = "57014";

function toPayload(e: unknown): ErrorPayload {
  if (typeof e === "object" && e !== null && "code" in e && "message" in e) {
    const p = e as Partial<ErrorPayload> & { code: string; message: string };
    return { code: p.code, message: p.message, detail: p.detail ?? null, hint: p.hint ?? null };
  }
  return { code: "unknown", message: e instanceof Error ? e.message : String(e), detail: null, hint: null };
}
const errorLine = (p: ErrorPayload) => [p.message, p.detail].filter(Boolean).join(" — ");
const fullError = (p: ErrorPayload) => (p.code === "query" ? [errorLine(p), p.hint].filter(Boolean).join(" — ") : p.message);

export function createMaintenanceController(deps: MaintenanceControllerDeps) {
  const { api } = deps;
  const [state, setState] = createSignal<MaintenanceState | null>(null);
  let gen = 0;
  let previewToken = 0;
  let previewTimer: unknown = null;

  const patch = (p: Partial<MaintenanceState>) => setState((s) => (s ? { ...s, ...p } : s));
  const cancelQuietly = (id: string) => {
    try {
      void Promise.resolve(api.ddlCancel(id)).catch(() => {});
    } catch {
      /* eşzamanlı hata da yok sayılır */
    }
  };

  // --- Önizleme (gecikmeli; en son istek kazanır) ---
  const stopPreview = () => {
    previewToken++;
    if (previewTimer !== null) deps.clearTimer(previewTimer);
    previewTimer = null;
  };
  const runPreview = () => {
    const s = state();
    if (!s || s.phase !== "ready" || !s.target || !s.operation || !s.object || !s.serverId) return;
    const req = maintenanceRequest(s.object, s.target, s.operation);
    const { version } = s;
    const g = gen;
    const mine = previewToken;
    api.ddlPreview(s.serverId, s.databaseOid, req).then(
      (view) => {
        const cur = state();
        if (!cur || g !== gen || mine !== previewToken) return;
        const current = version === cur.version;
        patch({ view, viewFor: version, pending: current ? false : cur.pending, previewError: current ? null : cur.previewError });
      },
      (err: unknown) => {
        const cur = state();
        if (!cur || g !== gen || mine !== previewToken || version !== cur.version) return;
        patch({ pending: false, previewError: fullError(toPayload(err)) });
      },
    );
  };
  const schedulePreview = (delay = PREVIEW_DELAY_MS) => {
    stopPreview();
    const s = state();
    if (!s || s.phase !== "ready" || !s.operation) return patch({ pending: false });
    patch({ pending: true });
    const mine = previewToken;
    previewTimer = deps.setTimer(() => {
      previewTimer = null;
      if (mine === previewToken) runPreview();
    }, delay);
  };

  // --- İlerleme yoklaması ---
  let poller: { start(): void; stop(): void } | null = null;
  const stopPolling = () => {
    poller?.stop();
    poller = null;
  };
  const startPolling = (serverId: string, databaseOid: number, applyId: string) => {
    stopPolling();
    const g = gen;
    poller = createProgressPoller({
      fetch: () => api.ddlMaintenanceProgress(serverId, databaseOid, applyId),
      schedule: (cb, ms) => {
        const h = deps.setTimer(cb, ms);
        return () => deps.clearTimer(h);
      },
      onUpdate: (p) => {
        if (g === gen && state()?.applyId === applyId) patch({ progress: p });
      },
      onStop: () => {
        if (g === gen && state()?.applyId === applyId) patch({ progressStopped: true });
      },
    });
    poller.start();
  };

  /** Seçili işlemin kullanılabilir hali (sürüm/kapı/dışlamalara göre temizlenir). */
  const withOperation = (s: MaintenanceState, op: MaintenanceOp | null): Partial<MaintenanceState> => ({
    operation: op && s.target ? normalize(op, s.target, s.versionNum) : null,
    version: s.version + 1,
    confirmed: false,
  });

  /** İlk kullanılabilir işlem (yoksa ilk sunulan; o da yoksa null). */
  const firstOp = (t: MaintenanceTarget, v: number | null): OpName | null => {
    if (!t.editable) return null;
    const ops = opsFor(t.kind, t.partitioned, v);
    return ops.find((o) => opBlocked(o, t) === null) ?? null;
  };

  /** Hedefi (yeniden) yükler. `keep`: mevcut işlem ve seçenekler korunur (başarı/plan_changed sonrası). */
  const loadTarget = async (keep: boolean) => {
    const s = state();
    if (!s) return;
    const g = ++gen;
    stopPreview();
    patch(keep ? { pending: true, reloading: true } : { phase: "loading", reloading: false, loadError: null, view: null, viewFor: -1, pending: false, previewError: null });
    try {
      let { serverId, databaseOid, object } = s;
      if (!serverId || !object || databaseOid === null) {
        const t = await api.ddlTargetOf(s.nodeId);
        if (g !== gen) return;
        object = t.object;
        serverId = t.serverId;
        databaseOid = t.databaseOid ?? (object?.kind === "database" ? object.oid : null);
        if (!object || databaseOid === null) {
          patch({ phase: "loadError", loadError: tm.noTarget });
          return;
        }
      }
      const info = deps.serverInfo(serverId);
      const target = await api.ddlLoadMaintenanceTarget(serverId, databaseOid, object);
      if (g !== gen) return;
      setState((cur) => {
        if (!cur) return cur;
        const base: MaintenanceState = {
          // Uçuştaki uygulamanın aşaması asla ezilmez (apply kuşağı artırdığından normalde buraya gelinmez; savunma).
          ...cur, serverId, databaseOid, object, target, phase: cur.phase === "applying" ? "applying" : "ready",
          loadError: null, reloading: false,
          versionNum: info?.versionNum ?? null,
        };
        const ops = target.editable ? opsFor(target.kind, target.partitioned, base.versionNum) : [];
        const kept = keep && cur.operation && ops.includes(cur.operation.op) && opBlocked(cur.operation.op, target) === null;
        const name = kept ? cur.operation!.op : firstOp(target, base.versionNum);
        const op = kept ? cur.operation : name ? defaultOperation(name, target, base.versionNum) : null;
        return { ...base, ...withOperation(base, op) };
      });
      schedulePreview(0);
    } catch (err) {
      if (g !== gen) return;
      const text = fullError(toPayload(err));
      if (keep) patch({ notice: tm.reloadFailed, pending: false, reloading: false, view: null, viewFor: -1 });
      else patch({ phase: "loadError", loadError: text });
    }
  };

  /**
   * Diyaloğu açar. Uygulama sürerken yok sayılır: uçuştaki işlem sessizce iptal edilmez (diyalog kipli olduğundan
   * normalde ulaşılamaz; savunma).
   */
  const open = (nodeId: string) => {
    if (state()?.phase === "applying") return;
    close();
    setState({
      nodeId, serverId: null, databaseOid: null, object: null, phase: "loading", loadError: null, target: null,
      versionNum: null, operation: null, version: 0, view: null, viewFor: -1, pending: false,
      previewError: null, lockTimeoutMs: DEFAULT_LOCK_TIMEOUT_MS, confirmed: false, applyId: null, startedAt: null,
      progress: null, progressStopped: false, cancelRequested: false, closeAsk: false, closing: false, result: null,
      reloading: false, notice: null,
    });
    void loadTarget(false);
  };

  /** Diyaloğu kapatır (kuşak artar; uçuştaki uygulama için `ddl_cancel`). App'e bildirmez. */
  function close() {
    gen++;
    stopPreview();
    stopPolling();
    const id = state()?.applyId;
    if (id) cancelQuietly(id);
    setState(null);
  }

  /** Canlı kilit (sunucu düğümü `flags.locked`); kilit açılınca kendiliğinden kalkar. */
  const locked = (): boolean => {
    const id = state()?.serverId;
    return id ? (deps.serverInfo(id)?.locked ?? false) : false;
  };

  const gate = (): Gate => {
    const s = state();
    if (!s) return { ok: false, reason: "loading" };
    if (locked()) return { ok: false, reason: "locked" };
    if (s.phase === "loading" || s.phase === "loadError" || s.reloading) return { ok: false, reason: "loading" };
    if (s.phase === "applying") return { ok: false, reason: "busy" };
    if (!s.target || !s.operation || opBlocked(s.operation.op, s.target) !== null) return { ok: false, reason: "unavailable" };
    if (s.pending) return { ok: false, reason: "pending" };
    const v = s.view;
    if (!v || s.viewFor !== s.version) return { ok: false, reason: "stale" };
    if (v.errors.length > 0) return { ok: false, reason: "invalid" };
    if (v.empty) return { ok: false, reason: "empty" };
    const confirm = v.risk === "destructive" || v.risk === "data_loss";
    return { ok: true, confirm };
  };
  const canApply = () => {
    const g = gate();
    return g.ok && (!g.confirm || state()!.confirmed);
  };

  const finish = (s: MaintenanceState, out: ApplyOutcome, elapsed: number) => {
    const messages = out.messages ?? [];
    const op = s.operation!;
    if (out.outcome === "applied") {
      return { kind: "applied", elapsedMs: out.elapsedMs, messages, warnings: messageSummary(messages).warnings, error: null, hint: null, ccnew: false } satisfies MaintenanceResult;
    }
    const kind = out.sqlstate === CANCELLED ? "cancelled" : "failed";
    return {
      kind, elapsedMs: elapsed, messages, warnings: messageSummary(messages).warnings,
      error: kind === "cancelled" ? null : errorLine(out.error), hint: kind === "cancelled" ? null : out.error.hint,
      ccnew: needsCcnewNotice(op, kind),
    } satisfies MaintenanceResult;
  };

  const apply = async (): Promise<void> => {
    const s = state();
    if (!s || !canApply() || !s.view || !s.operation || !s.target || !s.object || !s.serverId || s.databaseOid === null) return;
    const g = gate() as { ok: true; confirm: boolean };
    const applyId = deps.newId();
    const options: ApplyOptions = {
      applyId, lockTimeoutMs: s.lockTimeoutMs, expectedHash: s.view.hash, confirmed: g.confirm && s.confirmed,
      confirmedProduction: false, password: null,
    };
    const req = maintenanceRequest(s.object, s.target, s.operation);
    // Kuşak artar: bekleyen yeniden yükleme/önizleme yanıtları uygulama durumunu ezemez.
    const myGen = ++gen;
    const startedAt = deps.now();
    stopPreview();
    patch({
      phase: "applying", applyId, startedAt, progress: null, progressStopped: false, cancelRequested: false,
      result: null, notice: null, pending: false,
    });
    startPolling(s.serverId, s.databaseOid, applyId);
    let out: ApplyOutcome;
    try {
      out = await api.ddlApply(s.serverId, s.databaseOid, req, options);
    } catch (err) {
      if (myGen !== gen) return;
      stopPolling();
      const p = toPayload(err);
      const cur = state()!;
      const base = { phase: "ready" as const, applyId: null, closeAsk: false, view: null, viewFor: -1, confirmed: false };
      if (cur.closing) return finishClose();
      if (p.code === "plan_changed") {
        patch({ ...base, notice: tm.planChanged });
        void loadTarget(true);
        return;
      }
      // Kilit reddi: kilit şeridi ve kapı canlı sunucu düğümünden okunur (ayrı bildirim yok).
      if (p.code === "locked") {
        // Ağaç henüz kilidi göstermiyorsa (bayat) bilgi şeridi; gösteriyorsa yalnız canlı şerit.
        patch({ ...base, notice: locked() ? null : tm.locked });
        schedulePreview(0);
        return;
      }
      const elapsed = deps.now() - startedAt;
      patch({
        ...base,
        result: { kind: "failed", elapsedMs: elapsed, messages: [], warnings: 0, error: fullError(p), hint: null, ccnew: needsCcnewNotice(s.operation, "failed") },
      });
      schedulePreview(0);
      return;
    }
    // Başarılı uygulama nesnenin DDL'ini değiştirmiş olabilir (ör. CLUSTER): diyalog kapatılmış olsa da Inspector yenilenir.
    if (out.outcome === "applied") {
      try {
        deps.refreshInspector?.();
      } catch {
        /* Inspector yenilemesi sonucu etkilemez */
      }
    }
    if (myGen !== gen) return;
    stopPolling();
    const cur = state()!;
    const result = finish(cur, out, deps.now() - startedAt);
    if (cur.closing) return finishClose();
    patch({ phase: "ready", applyId: null, closeAsk: false, result, view: null, viewFor: -1, confirmed: false });
    if (out.outcome === "applied") void loadTarget(true);
    else schedulePreview(0);
  };

  /** Kapatma onayından sonra sonuç geldi: diyalog kapanır ve App bilgilendirilir. */
  function finishClose() {
    setState((s) => (s ? { ...s, applyId: null } : s));
    close();
    deps.onClosed();
  }

  const cancelApply = () => {
    const s = state();
    if (s?.phase !== "applying" || !s.applyId) return;
    cancelQuietly(s.applyId);
    patch({ cancelRequested: true });
  };

  const requestClose = () => {
    const s = state();
    if (!s) return;
    if (s.phase === "applying") {
      if (!s.closing) patch({ closeAsk: true });
      return;
    }
    close();
    deps.onClosed();
  };
  const answerClose = (yes: boolean) => {
    const s = state();
    if (!s || !s.closeAsk) return;
    if (!yes || s.phase !== "applying") return patch({ closeAsk: false });
    patch({ closeAsk: false, closing: true });
    cancelApply();
  };

  const editable = () => {
    const s = state();
    return s !== null && s.phase === "ready" && !s.reloading && s.target !== null;
  };
  const selectOp = (name: OpName) => {
    const s = state();
    if (!editable() || !s || s.operation?.op === name || !s.target) return;
    patch(withOperation(s, defaultOperation(name, s.target, s.versionNum)));
    schedulePreview();
  };
  /** Seçili işlemin alanlarını değiştirir (bilinmeyen alanlar yok sayılır, normalize edilir). */
  const setOptions = (p: Record<string, unknown>) => {
    const s = state();
    if (!editable() || !s || !s.operation) return;
    patch(withOperation(s, { ...s.operation, ...p } as MaintenanceOp));
    schedulePreview();
  };
  const setLockTimeout = (ms: number) => {
    if (state()?.phase === "applying") return;
    patch({ lockTimeoutMs: Number.isFinite(ms) && ms >= 0 ? Math.floor(ms) : DEFAULT_LOCK_TIMEOUT_MS });
  };
  const setConfirmed = (v: boolean) => {
    if (state()?.phase === "applying") return;
    patch({ confirmed: v });
  };

  return {
    state,
    open,
    close,
    retry: () => {
      if (state()?.phase === "loadError") void loadTarget(false);
    },
    selectOp,
    setOptions,
    setLockTimeout,
    setConfirmed,
    apply,
    cancelApply,
    requestClose,
    answerClose,
    gate,
    canApply,
    locked,
    /** Seçenekler düzenlenebilir mi (hazır ve yeniden yüklenmiyor). */
    editable,
    /** Sunulan işlemler ve kullanılamayanların gerekçesi. */
    ops: (): { name: OpName; reason: string | null }[] => {
      const s = state();
      if (!s?.target || !s.target.editable) return [];
      return opsFor(s.target.kind, s.target.partitioned, s.versionNum).map((name) => ({ name, reason: opBlocked(name, s.target!) }));
    },
    optionStates: (): Record<string, OptionState> => {
      const s = state();
      return s?.operation && s.target ? optionStates(s.operation, s.target, s.versionNum) : {};
    },
  };
}

export type MaintenanceController = ReturnType<typeof createMaintenanceController>;
