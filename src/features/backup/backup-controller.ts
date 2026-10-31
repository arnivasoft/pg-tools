// Yedekleme / geri yükleme diyaloglarının denetleyicileri (tembel parça; tek örnek). Saf kurallar `backup-model.ts`'tedir.
// Akış: aç → bağlan (`connect`) → `ddl_target_of` (oid) → araçlar/ayarlar/kodlamalar → gecikmeli plan → kapılı başlat →
// olaylar/sonuç → (geri yüklemede) Explorer ve Inspector yenileme.
//
// Kurallar:
// - Kuşak sayacı: aç/kapa kuşağı artırır; eski kuşağın yanıtı ve olayları durum yazmaz (kapalı diyaloğa gelen olay yok sayılır).
// - Seçenek/format/yol/hedef/kaynak/araç değişimi `version`'ı artırır, onayı ve yazılan adı sıfırlar ve `PLAN_DELAY_MS` sonra
//   plan ister. Yanıt yalnız `planFor === version` ise geçerlidir (eski sürümün yanıtı atılır).
// - Plan `not_connected` dönerse bir kez `connect` + yeniden plan denenir; ikincisi `planError` olur.
// - Sonuç `start` dönüşünden ya da `finished` olayından gelir (ilk gelen; ikisi eşittir). İptal arka uçtan beklenir.
// - Ana modüller (app/strings, explorer/*, designer/*, maintenance/*) içe aktarılmaz; metin `strings-backup.ts`'ten gelir.
// - Gizlilik: parola, yol, ad, SQL ve komut önizlemesi `console.*`'a, `localStorage`'a ya da bildirimlere yazılmaz.
import { createSignal } from "solid-js";
import { tb } from "../../app/strings-backup";
import type { ToastKind } from "../../app/toast-model";
import type { BackupEvent } from "../../ipc/bindings/BackupEvent";
import type { BackupOptions } from "../../ipc/bindings/BackupOptions";
import type { BackupScope } from "../../ipc/bindings/BackupScope";
import type { DumpFormat } from "../../ipc/bindings/DumpFormat";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { JobPlan } from "../../ipc/bindings/JobPlan";
import type { JobSummary } from "../../ipc/bindings/JobSummary";
import type { RestoreOptions } from "../../ipc/bindings/RestoreOptions";
import type { RestoreSource } from "../../ipc/bindings/RestoreSource";
import type { RestoreTarget } from "../../ipc/bindings/RestoreTarget";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { ToolDetection } from "../../ipc/bindings/ToolDetection";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import type { Api } from "../../ipc/commands";
import { isProductionLabel } from "../servers/server-form";
import {
  applyEvent,
  backupGate,
  backupScopeOf,
  canStart,
  confirmFlags,
  defaultBackupOptions,
  defaultRestoreOptions,
  emptyLog,
  errorText,
  normalizeBackup,
  normalizeRestore,
  restoreGate,
  saveFilters,
  serverMajor,
  suggestFileName,
  swapExtension,
  type BackupKey,
  type Gate,
  type JobLog,
  type RestoreKey,
} from "./backup-model";

export const PLAN_DELAY_MS = 150;

export type Picker = {
  save(o: { defaultPath: string; filters: { name: string; extensions: string[] }[] }): Promise<string | null>;
  openFile(): Promise<string | null>;
  openDir(): Promise<string | null>;
};

export interface BackupControllerDeps {
  api: Pick<
    Api,
    | "ddlTargetOf" | "ddlLookup" | "listChildren" | "backupTools" | "backupPlan" | "backupStart" | "backupCancel" | "backupReveal"
    | "restoreInspect" | "restorePlan" | "restoreStart" | "settingsGet" | "settingsSetPgBinDir"
  >;
  tree: () => { nodes: Map<string, TreeNode>; parent: Map<string, string> };
  servers: () => ServerConfig[];
  connect: (serverId: string) => Promise<boolean>;
  refreshNode?: (nodeId: string) => void;
  refreshInspector?: () => void;
  notify: (text: string, kind: ToastKind) => void;
  pick: Picker;
  onClosed(): void;
  newId(): string;
  now(): number;
  setTimer(fn: () => void, ms: number): unknown;
  clearTimer(h: unknown): void;
}

export type Phase = "loading" | "loadError" | "ready" | "running";

interface Common {
  nodeId: string;
  serverId: string | null;
  serverName: string;
  production: boolean;
  serverMajor: number | null;
  phase: Phase;
  loadError: string | null;
  tools: ToolDetection | null;
  toolsError: string | null;
  pgBinDir: string | null;
  binDirError: string | null;
  version: number;
  plan: JobPlan | null;
  planFor: number;
  pending: boolean;
  planError: string | null;
  jobId: string | null;
  log: JobLog;
  startedAt: number | null;
  cancelRequested: boolean;
  startError: string | null;
  closeAsk: boolean;
  /** Kapatma onaylandı: iptal istendi, sonuç bekleniyor; sonra diyalog kapanır. */
  closing: boolean;
}

export interface BackupState extends Common {
  databaseOid: number | null;
  scope: BackupScope | null;
  scopeLabel: string;
  dbName: string;
  format: DumpFormat;
  options: BackupOptions;
  outputPath: string;
  encodings: string[];
}

export interface RestoreState extends Common {
  databaseOid: number | null;
  fromServer: boolean;
  dbNodeId: string | null;
  databases: { nodeId: string; label: string }[] | null;
  source: RestoreSource | null;
  inspecting: boolean;
  sourceError: string | null;
  target: RestoreTarget;
  options: RestoreOptions;
  confirm: { checked: boolean; typedName: string };
  /** Kilit canlı ağaçtan okunur (`state()` her okumada günceller). */
  locked: boolean;
}

function toPayload(e: unknown): ErrorPayload {
  if (typeof e === "object" && e !== null && "code" in e && "message" in e) {
    const p = e as Partial<ErrorPayload> & { code: string; message: string };
    return { code: p.code, message: p.message, detail: p.detail ?? null, hint: p.hint ?? null };
  }
  return { code: "unknown", message: e instanceof Error ? e.message : String(e), detail: null, hint: null };
}

type Tree = ReturnType<BackupControllerDeps["tree"]>;

/** Düğümün kendisi ya da en yakın sunucu atası. */
function serverNodeOf(tree: Tree, id: string): TreeNode | null {
  const seen = new Set<string>();
  let cur: string | undefined = id;
  while (cur !== undefined && !seen.has(cur)) {
    seen.add(cur);
    const n = tree.nodes.get(cur);
    if (!n) return null;
    if (n.kind === "server") return n;
    cur = tree.parent.get(cur);
  }
  return null;
}

function baseState(nodeId: string): Common {
  return {
    nodeId, serverId: null, serverName: "", production: false, serverMajor: null, phase: "loading", loadError: null,
    tools: null, toolsError: null, pgBinDir: null, binDirError: null, version: 0, plan: null, planFor: -1, pending: false,
    planError: null, jobId: null, log: emptyLog(), startedAt: null, cancelRequested: false, startError: null,
    closeAsk: false, closing: false,
  };
}

interface Mode<S extends Common> {
  canPlan(s: S): boolean;
  plan(s: S): Promise<JobPlan>;
  /** Sürüm artışında sıfırlanan ek alanlar (geri yüklemede onay). */
  onBump?(): Partial<S>;
}

/** İki denetleyicinin ortak parçası: kuşak, plan gecikmesi, iş akışı, kapatma ve araç ayarları. */
function createCore<S extends Common>(deps: BackupControllerDeps, mode: Mode<S>) {
  const { api } = deps;
  const [raw, setRaw] = createSignal<S | null>(null);
  let gen = 0;
  let planToken = 0;
  let planTimer: unknown = null;

  const patch = (p: Partial<S>) => setRaw((s) => (s ? ({ ...s, ...p } as S) : s));

  const cancelQuietly = (id: string) => {
    try {
      void Promise.resolve(api.backupCancel(id)).catch(() => {});
    } catch {
      /* eşzamanlı hata da yok sayılır */
    }
  };
  const connectSafe = async (serverId: string): Promise<boolean> => {
    try {
      return await deps.connect(serverId);
    } catch {
      return false;
    }
  };

  // --- Plan (gecikmeli; en son istek kazanır) ---
  const stopPlan = () => {
    planToken++;
    if (planTimer !== null) deps.clearTimer(planTimer);
    planTimer = null;
  };
  const runPlan = async (attempt: number, version: number, g: number, mine: number, snap: S): Promise<void> => {
    const stale = () => {
      const cur = raw();
      return !cur || g !== gen || mine !== planToken || cur.version !== version;
    };
    let plan: JobPlan;
    try {
      plan = await mode.plan(snap);
    } catch (err) {
      if (stale()) return;
      const p = toPayload(err);
      if (p.code === "not_connected" && attempt === 0 && snap.serverId) {
        const ok = await connectSafe(snap.serverId);
        if (stale()) return;
        if (ok) return runPlan(1, version, g, mine, snap);
      }
      patch({ pending: false, plan: null, planFor: version, planError: errorText(p) } as Partial<S>);
      return;
    }
    if (stale()) return;
    patch({ plan, planFor: version, pending: false, planError: null } as Partial<S>);
  };
  const schedulePlan = (delay = PLAN_DELAY_MS) => {
    stopPlan();
    const s = raw();
    if (!s || s.phase !== "ready") return;
    if (!mode.canPlan(s)) return patch({ pending: false, plan: null, planError: null } as Partial<S>);
    patch({ pending: true } as Partial<S>);
    const mine = planToken;
    const g = gen;
    planTimer = deps.setTimer(() => {
      planTimer = null;
      if (mine !== planToken) return;
      const cur = raw();
      if (!cur || g !== gen || !mode.canPlan(cur)) return;
      void runPlan(0, cur.version, g, mine, cur);
    }, delay);
  };
  /** Plana giren bir şey değişti: sürüm artar, onay sıfırlanır, plan yeniden istenir. */
  const bump = (extra: Partial<S> = {}) => {
    const s = raw();
    if (!s) return;
    patch({ ...extra, ...(mode.onBump?.() ?? {}), version: s.version + 1, planError: null } as Partial<S>);
    schedulePlan();
  };
  const editable = () => raw()?.phase === "ready";

  /** Yeni kuşak başlatır ve başlangıç durumunu yazar. */
  const begin = (init: S): number => {
    const g = ++gen;
    stopPlan();
    setRaw(() => init);
    return g;
  };
  const isCurrent = (g: number) => g === gen && raw() !== null;
  const fail = (g: number, text: string) => {
    if (isCurrent(g)) patch({ phase: "loadError", loadError: text } as Partial<S>);
  };

  /** Sunucu bilgileri (ad, üretim, ana sürüm) ağaçtan ve sunucu listesinden. */
  const serverInfoOf = (nodeId: string) => {
    const tree = deps.tree();
    const sn = serverNodeOf(tree, nodeId);
    const serverId = tree.nodes.get(nodeId)?.serverId ?? sn?.serverId ?? null;
    const cfg = serverId ? deps.servers().find((c) => c.id === serverId) : undefined;
    return {
      tree, sn, serverId,
      serverName: cfg?.name ?? sn?.label ?? "",
      production: isProductionLabel(cfg?.label),
      serverMajor: serverMajor(sn?.detail ?? null),
    };
  };

  const detectTools = async (serverId: string): Promise<{ tools: ToolDetection | null; toolsError: string | null }> => {
    try {
      return { tools: await api.backupTools(serverId), toolsError: null };
    } catch (err) {
      return { tools: null, toolsError: errorText(toPayload(err)) };
    }
  };
  const readPgBinDir = async (): Promise<string | null> => {
    try {
      return (await api.settingsGet()).pgBinDir;
    } catch {
      return null;
    }
  };

  // --- Araç ayarları ---
  const rescanTools = async (): Promise<void> => {
    const s = raw();
    if (!s || s.phase !== "ready" || !s.serverId) return;
    const g = gen;
    const r = await detectTools(s.serverId);
    if (!isCurrent(g)) return;
    patch(r as Partial<S>);
    if (editable()) bump();
  };
  const setBinDir = async (dir: string | null): Promise<void> => {
    const s = raw();
    if (!s || s.phase !== "ready") return;
    const g = gen;
    try {
      const st = await api.settingsSetPgBinDir(dir);
      if (!isCurrent(g)) return;
      patch({ pgBinDir: st.pgBinDir, binDirError: null } as Partial<S>);
    } catch (err) {
      if (isCurrent(g)) patch({ binDirError: errorText(toPayload(err)) } as Partial<S>);
      return;
    }
    await rescanTools();
  };
  const chooseBinDir = async (): Promise<void> => {
    if (!editable()) return;
    let dir: string | null;
    try {
      dir = await deps.pick.openDir();
    } catch {
      return;
    }
    if (dir === null) return;
    await setBinDir(dir);
  };
  const clearBinDir = () => setBinDir(null);

  // --- Kapatma ---
  function closeInternal(cancelJob: boolean) {
    gen++;
    stopPlan();
    const s = raw();
    if (cancelJob && s?.phase === "running" && s.jobId) cancelQuietly(s.jobId);
    setRaw(null);
  }
  const close = () => closeInternal(true);
  const finishClose = () => {
    closeInternal(false);
    deps.onClosed();
  };
  const requestClose = () => {
    const s = raw();
    if (!s) return;
    if (s.phase === "running") {
      if (!s.closing) patch({ closeAsk: true } as Partial<S>);
      return;
    }
    close();
    deps.onClosed();
  };
  const cancel = () => {
    const s = raw();
    if (s?.phase !== "running" || !s.jobId) return;
    patch({ cancelRequested: true } as Partial<S>);
    cancelQuietly(s.jobId);
  };
  const answerClose = (doCancel: boolean) => {
    const s = raw();
    if (!s || !s.closeAsk) return;
    if (!doCancel) return patch({ closeAsk: false } as Partial<S>);
    if (s.phase !== "running") {
      finishClose();
      return;
    }
    patch({ closeAsk: false, closing: true } as Partial<S>);
    cancel();
  };

  /**
   * İşi başlatır. Sonuç `launch` dönüşünden ya da `finished` olayından gelir (ilk gelen). `side` kapanmış diyalogda da
   * bir kez çalışır (Explorer yenileme); `settled` durum yamasıdır.
   */
  const runJob = async (
    launch: (jobId: string, onEvent: (e: BackupEvent) => void) => Promise<JobSummary>,
    side: (summary: JobSummary) => void,
    settled: (summary: JobSummary) => Partial<S>,
  ): Promise<void> => {
    const jobId = deps.newId();
    const g = gen;
    stopPlan();
    patch({
      phase: "running", jobId, log: emptyLog(), startedAt: deps.now(), cancelRequested: false, startError: null,
      closeAsk: false, closing: false, pending: false,
    } as Partial<S>);
    let done = false;
    const settle = (summary: JobSummary) => {
      if (done) return;
      done = true;
      try {
        side(summary);
      } catch {
        /* yenileme sonucu etkilemez */
      }
      const cur = raw();
      if (g !== gen || !cur || cur.jobId !== jobId) return;
      if (cur.closing) return finishClose();
      const log = cur.log.summary === null ? ({ ...cur.log, summary } as JobLog) : cur.log;
      patch({ phase: "ready", closeAsk: false, log, ...settled(summary) } as Partial<S>);
    };
    const onEvent = (ev: BackupEvent) => {
      if (done || g !== gen) return;
      const cur = raw();
      if (!cur || cur.jobId !== jobId) return;
      const log = applyEvent(cur.log, ev);
      patch({ log } as Partial<S>);
      if (ev.event === "finished" && log.summary) settle(log.summary);
    };
    try {
      settle(await launch(jobId, onEvent));
    } catch (err) {
      if (done) return;
      done = true;
      const cur = raw();
      if (g !== gen || !cur || cur.jobId !== jobId) return;
      if (cur.closing) return finishClose();
      patch({ phase: "ready", closeAsk: false, startError: errorText(toPayload(err)) } as Partial<S>);
    }
  };

  return {
    raw, patch, begin, isCurrent, fail, bump, schedulePlan, editable, connectSafe, serverInfoOf, detectTools, readPgBinDir,
    rescanTools, chooseBinDir, clearBinDir, close, requestClose, answerClose, cancel, runJob,
    gen: () => gen,
  };
}

// ---------------------------------------------------------------------------------------------------------------------
// Yedekleme

export function createBackupController(deps: BackupControllerDeps) {
  const { api } = deps;
  const requestOf = (s: BackupState) => ({
    scope: s.scope!,
    format: s.format,
    options: normalizeBackup(s.format, s.options),
    outputPath: s.outputPath,
  });
  const core = createCore<BackupState>(deps, {
    canPlan: (s) => s.databaseOid !== null && s.scope !== null && s.serverId !== null,
    plan: (s) => api.backupPlan(s.serverId!, s.databaseOid!, requestOf(s)),
  });
  const { raw, patch } = core;

  const load = async (nodeId: string) => {
    const info = core.serverInfoOf(nodeId);
    const sc = backupScopeOf(info.tree, nodeId);
    const g = core.begin({
      ...baseState(nodeId), serverId: info.serverId, serverName: info.serverName, production: info.production,
      serverMajor: info.serverMajor, databaseOid: null, scope: sc?.scope ?? null, scopeLabel: sc?.label ?? "", dbName: sc?.dbName ?? "",
      format: "custom", options: defaultBackupOptions(), outputPath: "", encodings: [],
    });
    if (!sc || !info.serverId) return core.fail(g, tb.loadFailed);
    const serverId = info.serverId;
    if (!(await core.connectSafe(serverId))) return core.fail(g, tb.loadFailed);
    if (!core.isCurrent(g)) return;
    let oid: number | null;
    try {
      const t = await api.ddlTargetOf(sc.dbNodeId);
      oid = t.databaseOid ?? (t.object?.kind === "database" ? t.object.oid : null);
    } catch (err) {
      return core.fail(g, errorText(toPayload(err)));
    }
    if (!core.isCurrent(g)) return;
    if (oid === null) return core.fail(g, tb.loadFailed);
    const [det, pgBinDir, encodings] = await Promise.all([
      core.detectTools(serverId),
      core.readPgBinDir(),
      api.ddlLookup(serverId, oid, { kind: "encodings" }).then(
        (items) => items.map((i) => i.value),
        () => [] as string[],
      ),
    ]);
    if (!core.isCurrent(g)) return;
    patch({ phase: "ready", loadError: null, databaseOid: oid, ...det, pgBinDir, encodings });
    core.schedulePlan(0);
  };

  const open = (nodeId: string) => {
    if (raw()?.phase === "running") return;
    core.close();
    void load(nodeId);
  };

  const fmtSpec = (s: BackupState) => ({ format: s.format, compress: s.options.compress });

  const setFormat = (f: DumpFormat) => {
    const s = raw();
    if (!s || !core.editable() || s.format === f) return;
    const options = normalizeBackup(f, s.options);
    const outputPath = swapExtension(s.outputPath, fmtSpec(s), { format: f, compress: options.compress });
    core.bump({ format: f, options, outputPath });
  };

  const setOption = <K extends BackupKey>(k: K, v: BackupOptions[K]) => {
    const s = raw();
    if (!s || !core.editable()) return;
    const options = normalizeBackup(s.format, { ...s.options, [k]: v });
    const outputPath = swapExtension(s.outputPath, fmtSpec(s), { format: s.format, compress: options.compress });
    core.bump({ options, outputPath });
  };

  const nameParts = (s: BackupState): string[] => {
    const sc = s.scope;
    if (!sc || sc.kind === "database") return [s.dbName];
    return sc.kind === "schema" ? [s.dbName, sc.name] : [s.dbName, sc.schema, sc.name];
  };

  const chooseOutput = async (): Promise<void> => {
    const s = raw();
    if (!s || !core.editable()) return;
    const g = core.gen();
    const suggestion = suggestFileName(nameParts(s), s.format, s.options.compress, new Date(deps.now()));
    let path: string | null;
    try {
      path = await deps.pick.save({
        defaultPath: s.outputPath !== "" ? s.outputPath : suggestion,
        filters: saveFilters(s.format, s.options.compress),
      });
    } catch {
      return;
    }
    if (path === null || !core.isCurrent(g) || !core.editable()) return;
    core.bump({ outputPath: path });
  };

  const gate = (): Gate => {
    const s = raw();
    if (!s) return { kind: "blocked", reason: "stale" };
    return backupGate({
      running: s.phase === "running", hasPath: s.outputPath !== "", pending: s.pending, planCurrent: s.planFor === s.version, plan: s.plan,
    });
  };

  const start = async (): Promise<void> => {
    const s = raw();
    if (!s || s.phase !== "ready" || gate().kind !== "ready" || !s.serverId || s.databaseOid === null || !s.scope) return;
    const serverId = s.serverId;
    const oid = s.databaseOid;
    const req = requestOf(s);
    await core.runJob(
      (jobId, onEvent) => api.backupStart(serverId, oid, req, jobId, onEvent),
      () => {},
      (summary) => (summary.outcome === "success" ? { outputPath: "" } : {}),
    );
  };

  const reveal = async (): Promise<void> => {
    const path = raw()?.log.summary?.outputPath;
    if (!path) return;
    try {
      await api.backupReveal(path);
    } catch {
      deps.notify(tb.revealFailed, "error");
    }
  };

  return {
    state: raw as () => BackupState | null,
    gate,
    open,
    close: core.close,
    requestClose: core.requestClose,
    answerClose: core.answerClose,
    setFormat,
    setOption,
    chooseOutput,
    start,
    cancel: core.cancel,
    reveal,
    rescanTools: core.rescanTools,
    chooseBinDir: core.chooseBinDir,
    clearBinDir: core.clearBinDir,
  };
}

export type BackupController = ReturnType<typeof createBackupController>;

// ---------------------------------------------------------------------------------------------------------------------
// Geri yükleme

export function createRestoreController(deps: BackupControllerDeps) {
  const { api } = deps;
  const srcFormat = (s: RestoreState) => s.source?.format ?? "custom";
  const requestOf = (s: RestoreState) => ({
    path: s.source!.path,
    target: s.target,
    options: normalizeRestore(srcFormat(s), s.target, s.options),
  });
  const oidOf = (s: RestoreState) => (s.target === "existing" ? s.databaseOid : null);

  const core = createCore<RestoreState>(deps, {
    canPlan: (s) => s.source !== null && !s.inspecting && s.serverId !== null && (s.target === "create_from_archive" || s.databaseOid !== null),
    plan: (s) => api.restorePlan(s.serverId!, oidOf(s), requestOf(s)),
    onBump: () => ({ confirm: { checked: false, typedName: "" } }),
  });
  const { raw, patch } = core;
  let inspectToken = 0;

  /** Kilit canlı ağaçtaki sunucu düğümünden okunur (bakım diyaloğuyla aynı tek kaynak). */
  const liveLocked = (s: RestoreState): boolean => serverNodeOf(deps.tree(), s.nodeId)?.flags.locked ?? false;
  const state = (): RestoreState | null => {
    const s = raw();
    if (!s) return null;
    const locked = liveLocked(s);
    return locked === s.locked ? s : { ...s, locked };
  };

  const load = async (nodeId: string) => {
    inspectToken++;
    const info = core.serverInfoOf(nodeId);
    const node = info.tree.nodes.get(nodeId);
    const fromServer = node?.kind === "server";
    const g = core.begin({
      ...baseState(nodeId), serverId: info.serverId, serverName: info.serverName, production: info.production,
      serverMajor: info.serverMajor, databaseOid: null, fromServer, dbNodeId: node?.kind === "database" ? nodeId : null,
      databases: null, source: null, inspecting: false, sourceError: null,
      target: fromServer ? "create_from_archive" : "existing", options: defaultRestoreOptions(),
      confirm: { checked: false, typedName: "" }, locked: info.sn?.flags.locked ?? false,
    });
    if (!node || !info.serverId || (node.kind !== "server" && node.kind !== "database")) return core.fail(g, tb.loadFailed);
    const serverId = info.serverId;
    if (!(await core.connectSafe(serverId))) return core.fail(g, tb.loadFailed);
    if (!core.isCurrent(g)) return;
    let oid: number | null = null;
    if (!fromServer) {
      try {
        const t = await api.ddlTargetOf(nodeId);
        oid = t.databaseOid ?? (t.object?.kind === "database" ? t.object.oid : null);
      } catch (err) {
        return core.fail(g, errorText(toPayload(err)));
      }
      if (!core.isCurrent(g)) return;
      if (oid === null) return core.fail(g, tb.loadFailed);
    }
    const [det, pgBinDir] = await Promise.all([core.detectTools(serverId), core.readPgBinDir()]);
    if (!core.isCurrent(g)) return;
    patch({ phase: "ready", loadError: null, databaseOid: oid, ...det, pgBinDir });
  };

  const open = (nodeId: string) => {
    if (raw()?.phase === "running") return;
    core.close();
    void load(nodeId);
  };

  const chooseSource = async (kind: "file" | "dir"): Promise<void> => {
    if (!core.editable()) return;
    const g = core.gen();
    let path: string | null;
    try {
      path = await (kind === "file" ? deps.pick.openFile() : deps.pick.openDir());
    } catch {
      return;
    }
    if (path === null || !core.isCurrent(g) || !core.editable()) return;
    const mine = ++inspectToken;
    core.bump({ source: null, inspecting: true, sourceError: null });
    const stale = () => !core.isCurrent(g) || mine !== inspectToken;
    try {
      const src = await api.restoreInspect(path);
      if (stale()) return;
      const cur = raw()!;
      const target: RestoreTarget = src.format === "plain" ? "existing" : cur.target;
      core.bump({ source: src, inspecting: false, target, options: normalizeRestore(src.format, target, cur.options) });
    } catch (err) {
      if (stale()) return;
      core.bump({ source: null, inspecting: false, sourceError: errorText(toPayload(err)) });
    }
  };

  const setTarget = (t: RestoreTarget) => {
    const s = raw();
    if (!s || !core.editable() || s.target === t) return;
    if (t === "create_from_archive" && s.source?.format === "plain") return;
    core.bump({ target: t, options: normalizeRestore(srcFormat(s), t, s.options) });
  };

  /** Sunucu düğümünden açıldıysa mevcut hedef seçicisinin listesi tembel yüklenir (sunucu → koleksiyon → veritabanları). */
  const loadDatabases = async (): Promise<void> => {
    const s = raw();
    if (!s || !s.fromServer || s.databases !== null) return;
    const g = core.gen();
    let list: { nodeId: string; label: string }[] = [];
    try {
      const kids = await api.listChildren(s.nodeId);
      const col = kids.find((n) => n.kind === "databaseCollection");
      if (col) {
        const dbs = await api.listChildren(col.id);
        list = dbs.filter((n) => n.kind === "database").map((n) => ({ nodeId: n.id, label: n.label }));
      }
    } catch {
      /* liste boş kalır; hedef seçilemez */
    }
    if (core.isCurrent(g)) patch({ databases: list });
  };

  const setTargetDb = async (nodeId: string): Promise<void> => {
    if (!core.editable()) return;
    const g = core.gen();
    try {
      const t = await api.ddlTargetOf(nodeId);
      if (!core.isCurrent(g) || !core.editable()) return;
      const oid = t.databaseOid ?? (t.object?.kind === "database" ? t.object.oid : null);
      if (oid === null) {
        patch({ planError: tb.loadFailed });
        return;
      }
      core.bump({ databaseOid: oid, dbNodeId: nodeId });
    } catch (err) {
      if (core.isCurrent(g)) patch({ planError: errorText(toPayload(err)) });
    }
  };

  const setOption = <K extends RestoreKey>(k: K, v: RestoreOptions[K]) => {
    const s = raw();
    if (!s || !core.editable()) return;
    core.bump({ options: normalizeRestore(srcFormat(s), s.target, { ...s.options, [k]: v }) });
  };

  const setConfirm = (c: Partial<{ checked: boolean; typedName: string }>) => {
    const s = raw();
    if (!s || s.phase === "running") return;
    patch({ confirm: { ...s.confirm, ...c } });
  };

  const gate = (): Gate => {
    const s = state();
    if (!s) return { kind: "blocked", reason: "stale" };
    return restoreGate({
      locked: s.locked, running: s.phase === "running", inspecting: s.inspecting, hasSource: s.source !== null,
      hasTarget: s.target === "create_from_archive" || s.databaseOid !== null, pending: s.pending,
      planCurrent: s.planFor === s.version, plan: s.plan, production: s.production, serverName: s.serverName,
    });
  };

  const start = async (): Promise<void> => {
    const s = state();
    if (!s || s.phase !== "ready" || !s.source || !s.serverId) return;
    const g = gate();
    if (!canStart(g, s.confirm)) return;
    const { confirmed, confirmedProduction } = confirmFlags(g, s.confirm);
    const serverId = s.serverId;
    const oid = oidOf(s);
    const req = requestOf(s);
    // Yenilenecek düğüm başlangıçta yakalanır: diyalog kapansa da iş bitince Explorer güncellenir.
    const refreshId = s.target === "existing" ? s.dbNodeId : (serverNodeOf(deps.tree(), s.nodeId)?.id ?? s.nodeId);
    await core.runJob(
      (jobId, onEvent) => api.restoreStart(serverId, oid, req, jobId, confirmed, confirmedProduction, onEvent),
      (summary) => {
        if (summary.outcome !== "success" && summary.outcome !== "completed_with_errors") return;
        if (refreshId) deps.refreshNode?.(refreshId);
        deps.refreshInspector?.();
      },
      () => ({ confirm: { checked: false, typedName: "" } }),
    );
  };

  return {
    state,
    gate,
    open,
    close: core.close,
    requestClose: core.requestClose,
    answerClose: core.answerClose,
    chooseSource,
    setTarget,
    setTargetDb,
    loadDatabases,
    setOption,
    setConfirm,
    start,
    cancel: core.cancel,
    rescanTools: core.rescanTools,
    chooseBinDir: core.chooseBinDir,
    clearBinDir: core.clearBinDir,
  };
}

export type RestoreController = ReturnType<typeof createRestoreController>;
