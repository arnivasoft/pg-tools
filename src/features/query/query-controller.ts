// Sorgu sekmelerinin yaşam döngüsü (ana pakette; CodeMirror/ızgara içe aktarmaz): sekme modelleri, oturum açma/kapama,
// taslak kalıcılığı, kapanış/çıkış/sunucu değişimi diyalogları, ortam istemi. Çalıştırma akışı tembel QueryTab'dadır.
import { createSignal, type Accessor, type Setter } from "solid-js";
import { t } from "../../app/strings";
import type { ToastKind } from "../../app/toast-model";
import type { ClosePolicy } from "../../ipc/bindings/ClosePolicy";
import type { Scope } from "../../ipc/bindings/Scope";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { SessionOptions } from "../../ipc/bindings/SessionOptions";
import type { Api } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import type { TreeState } from "../explorer/tree-model";
import { readStorage, writeStorage } from "../../app/storage";
import { initialRun, runReduce, type CloseChoice, type RunEvent, type RunState } from "./run-model";
import { createTabsStore, isQueryTabId } from "./tabs-store";
import {
  DRAFT_DEBOUNCE_MS, applyEnvChoice, bulkPolicy, choicePolicy, closePlan, createDebouncer, defaultOptions, initialSql,
  needsEnvPrompt, nextTxSince, openContext, pendingTransactions, restoreDraft, routeError, saveClosesSessions, toDraft,
  type EnvChoice, type PendingTx, type TabTxInfo,
} from "./workspace-model";

export type TabOp = "more" | "export" | "tx" | "closing" | "options" | null;

/** Mesajlar'da gösterilen yerel not (kilit, boş betik, red, oturum sıfırlandı …). */
export interface TabNote {
  level: "error" | "info";
  text: string;
  hint?: string | null;
  /** `session_reset`: "Tekrar çalıştır" düğmesi (aynı kapsamla). */
  rerun?: "stmt" | "all";
}

export interface TabModel {
  serverId: string | null;
  serverName: string;
  databaseOid: number | null;
  database: string | null;
  /** Tablodan açıldıysa tablo adı (CSV varsayılan dosya adı). */
  relation: string | null;
  options: SessionOptions;
  session: string | null;
  run: RunState;
  serverGone: boolean;
  notes: TabNote[];
  op: TabOp;
  txSince: number | null;
  /** Son çalıştırmanın metni ve kapsamı (dışa aktarmada şeritte seçili ifadeyi tercih etmek için). */
  last: { text: string; scope: Scope } | null;
  selectedStmt: number | null;
  historyKey: number;
  /** Bu sekmede yazarken denetim geçici olarak kapalı (bağlam şeridi; kalıcı değil). */
  checkOff: boolean;
}

interface Entry {
  model: Accessor<TabModel>;
  set: Setter<TabModel>;
  text: string;
}

export type BulkChoice = "commit" | "rollback" | "cancel";
export interface BulkRequest {
  message: string;
  list: PendingTx[];
}
export interface CloseRequest {
  title: string;
  choices: CloseChoice[];
  failed: boolean;
  busy: boolean;
}
export interface EnvRequest {
  config: ServerConfig;
}
export type EnvAnswer = { choice: EnvChoice; lock: boolean } | "skip" | null;

const ENV_SKIPPED_KEY = "pgt.envSkipped";

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

export interface QueryControllerDeps {
  api: Api;
  notify: (text: string, kind: ToastKind) => void;
  tree: () => TreeState;
  /** Sunucuyu katalog tarafında bağlar (gerekirse parola sorar); `qt_open` bağlı sunucu ister. */
  connect: (serverId: string) => Promise<boolean>;
  /** Sunucu kaydı arayüzden değiştiğinde (ortam etiketi) ağaç yenilensin. */
  serversChanged: () => void;
  /** Arka uç sunucunun tüm oturumlarını kapattı (sorgu sekmelerinden sonra; ör. dashboard sekmeleri için). */
  sessionsClosed?: (serverId: string, gone: boolean) => void;
}

export function createQueryController(deps: QueryControllerDeps) {
  const { api, notify } = deps;
  const tabs = createTabsStore();
  const entries = new Map<string, Entry>();
  const [servers, setServers] = createSignal<ServerConfig[]>([]);
  const [everQuery, setEverQuery] = createSignal(false);
  let runCounter = 0;
  let runner: ((kind: "stmt" | "all") => void) | null = null;

  const closeGate = gate<CloseRequest, CloseChoice>("cancel");
  const bulkGate = gate<BulkRequest, BulkChoice>("cancel");
  const envGate = gate<EnvRequest, EnvAnswer>(null);

  const model = (id: string): TabModel | undefined => entries.get(id)?.model();
  const titleOf = (id: string) => tabs.state.tabs.find((x) => x.id === id)?.title ?? "";
  const update = (id: string, f: (m: TabModel) => TabModel) => {
    const e = entries.get(id);
    if (e) e.set((m) => f(m));
  };
  const patch = (id: string, p: Partial<TabModel>) => update(id, (m) => ({ ...m, ...p }));

  const dispatch = (id: string, ev: RunEvent) =>
    update(id, (m) => {
      const run = runReduce(m.run, ev);
      if (run === m.run) return m;
      return { ...m, run, txSince: nextTxSince(m.run.tx, run.tx, m.txSince, Date.now()) };
    });

  // ---- taslaklar
  // Kapanan sekmeler: süren bir kayıt kaldırmadan sonra işlenirse sekme bir sonraki açılışta geri gelmesin.
  const closed = new Set<string>();
  const inflight = new Map<string, Promise<void>>();
  const saveDraft = async (id: string) => {
    const e = entries.get(id);
    if (!e || closed.has(id)) return;
    const m = e.model();
    if (m.serverGone) return; // salt okunur taslak: diskteki son hali korunur
    const p = api
      .qtDraftsSave(toDraft({ id, title: titleOf(id), serverId: m.serverId, databaseOid: m.databaseOid, database: m.database, options: m.options }), e.text)
      .then(
        () => {},
        (err) => notify(toPayload(err).message, "error"),
      );
    inflight.set(id, p);
    await p;
    if (inflight.get(id) === p) inflight.delete(id);
    if (closed.has(id)) await api.qtDraftsRemove(id).catch(() => {});
  };
  const saver = createDebouncer(DRAFT_DEBOUNCE_MS, saveDraft);

  const setText = (id: string, text: string) => {
    const e = entries.get(id);
    if (!e || e.text === text) return;
    e.text = text;
    if (!e.model().serverGone) saver.touch(id);
  };

  // ---- sunucular
  const refreshServers = async (): Promise<ServerConfig[]> => {
    try {
      const list = await api.listServers();
      setServers(list);
      for (const [id, e] of entries) {
        const m = e.model();
        const cfg = m.serverId ? list.find((s) => s.id === m.serverId) : undefined;
        if (cfg && (cfg.name !== m.serverName || m.serverGone)) patch(id, { serverName: cfg.name, serverGone: false });
        if (!cfg && m.serverId && !m.serverGone) {
          patch(id, { serverGone: true, session: null });
          dispatch(id, { type: "sessionLost" });
        }
      }
      return list;
    } catch (err) {
      notify(toPayload(err).message, "error");
      return servers();
    }
  };
  const serverConfig = (serverId: string | null) => (serverId ? (servers().find((s) => s.id === serverId) ?? null) : null);

  // ---- sekme ekleme
  const addTab = (input: {
    id?: string;
    title?: string;
    serverId: string | null;
    serverName: string;
    databaseOid: number | null;
    database: string | null;
    relation: string | null;
    options: SessionOptions;
    serverGone: boolean;
    text: string;
    activate: boolean;
  }): string => {
    const id = input.id ?? crypto.randomUUID();
    const [m, set] = createSignal<TabModel>({
      serverId: input.serverId,
      serverName: input.serverName,
      databaseOid: input.databaseOid,
      database: input.database,
      relation: input.relation,
      options: input.options,
      session: null,
      run: initialRun(),
      serverGone: input.serverGone,
      notes: [],
      op: null,
      txSince: null,
      last: null,
      selectedStmt: null,
      historyKey: 0,
      checkOff: false,
    });
    entries.set(id, { model: m, set, text: input.text });
    const prevActive = tabs.state.activeId;
    const realId = tabs.open({ id, serverId: input.serverId ?? "", serverName: input.serverName, database: input.database ?? "", title: input.title });
    if (realId !== id) {
      // Kimlik çakıştı (olmamalı): girdiyi yeni kimliğe taşı.
      entries.set(realId, entries.get(id)!);
      entries.delete(id);
    }
    if (!input.activate) tabs.activate(prevActive);
    else setEverQuery(true);
    return realId;
  };

  // Dashboard ya da "Nesne" sekmesi Query Tool parçasını (CodeMirror) yüklemez.
  const activate = (id: string) => {
    tabs.activate(id);
    if (isQueryTabId(tabs.state, id)) setEverQuery(true);
  };

  // ---- ortam istemi
  const skipped = (): Set<string> => {
    try {
      const raw = readStorage(ENV_SKIPPED_KEY);
      return new Set(raw ? (JSON.parse(raw) as string[]) : []);
    } catch {
      return new Set();
    }
  };
  const rememberSkip = (serverId: string) => writeStorage(ENV_SKIPPED_KEY, JSON.stringify([...skipped(), serverId]));

  /** Etiketsiz sunucuda ortamı sorar; seçimi kaydeder. Güncel yapılandırmayı döndürür (iptalde `null`). */
  const promptEnvironment = async (config: ServerConfig): Promise<ServerConfig | null> => {
    if (!needsEnvPrompt(config, skipped())) return config;
    const answer = await envGate.ask({ config });
    if (answer === null) return null;
    if (answer === "skip") {
      rememberSkip(config.id);
      return config;
    }
    const next = applyEnvChoice(config, answer.choice, answer.lock);
    if (saveClosesSessions(config, next) && !(await confirmServerChange(config.id, config.name))) return null;
    try {
      const saved = await api.saveServer(next, null);
      if (saveClosesSessions(config, saved)) serverSessionsClosed(saved.id, false);
      await refreshServers();
      deps.serversChanged();
      return saved;
    } catch (err) {
      notify(toPayload(err).message, "error");
      return config;
    }
  };

  // ---- açma
  let opening = false;
  const openFromNode = async (nodeId: string, opts: { draft: boolean } = { draft: true }) => {
    if (opening) return;
    const ctx = openContext(deps.tree(), nodeId);
    if (!ctx) return;
    opening = true;
    try {
      const target = await api.qtTargetOf(nodeId);
      const list = await refreshServers();
      let config = list.find((s) => s.id === target.serverId);
      if (!config) {
        notify(t.query.ctx.serverGone, "error");
        return;
      }
      const prompted = await promptEnvironment(config);
      if (!prompted) return;
      config = prompted;
      let text = "";
      if (opts.draft && ctx.relation) {
        try {
          text = initialSql(await api.quoteIdentifiers(config.id, ctx.relation));
        } catch (err) {
          notify(toPayload(err).message, "error");
        }
      }
      const id = addTab({
        serverId: config.id,
        serverName: config.name,
        databaseOid: target.databaseOid,
        database: ctx.database ?? config.maintenanceDb,
        relation: opts.draft && ctx.relation ? ctx.relation[1] : null,
        options: defaultOptions(config.label),
        serverGone: false,
        text,
        activate: true,
      });
      saver.touch(id);
    } catch (err) {
      notify(toPayload(err).message, "error");
    } finally {
      opening = false;
    }
  };

  // ---- oturum
  const sessionLost = (id: string, note = true) => {
    patch(id, { session: null, op: null });
    dispatch(id, { type: "sessionLost" });
    if (note) update(id, (m) => ({ ...m, notes: [...m.notes, { level: "info", text: t.query.results.sessionLost }] }));
  };

  /** Oturum yoksa açar (ilk çalıştırma / yeniden bağlan). Sunucu bağlı değilse önce bağlanır. */
  const ensureSession = async (id: string): Promise<string | null> => {
    const m = model(id);
    if (!m) return null;
    if (m.session && m.run.status !== "lost") return m.session;
    if (m.serverGone || !m.serverId) {
      notify(t.query.ctx.serverGone, "error");
      return null;
    }
    if (m.run.status !== "noSession" && m.run.status !== "lost") return null; // açılıyor
    dispatch(id, { type: "open" });
    const serverId = m.serverId;
    for (let attempt = 0; attempt < 2; attempt++) {
      try {
        const cur = model(id);
        if (!cur) return null;
        const session = await api.qtOpen(serverId, cur.databaseOid, cur.options);
        if (!entries.has(id)) {
          void api.qtClose(session, "rollback").catch(() => {});
          return null;
        }
        patch(id, { session });
        dispatch(id, { type: "opened" });
        return session;
      } catch (err) {
        const e = toPayload(err);
        if (e.code === "not_connected" && attempt === 0 && (await deps.connect(serverId))) continue;
        dispatch(id, { type: "openFailed" });
        if (e.code === "unknown_server") {
          patch(id, { serverGone: true });
          await refreshServers();
        }
        notify(e.message, "error");
        return null;
      }
    }
    dispatch(id, { type: "openFailed" });
    return null;
  };

  const setOptions = async (id: string, options: SessionOptions) => {
    const m = model(id);
    if (!m) return;
    patch(id, { options });
    saver.touch(id);
    if (!m.session || m.run.status === "lost") return;
    patch(id, { op: "options" });
    try {
      await api.qtSetOptions(m.session, options);
      dispatch(id, { type: "optionsChanged" });
    } catch (err) {
      const e = toPayload(err);
      patch(id, { options: m.options });
      if (routeError(e) === "lost") sessionLost(id);
      else notify(e.message, "error");
    } finally {
      if (model(id)?.op === "options") patch(id, { op: null });
    }
  };

  // ---- kapanış
  const removeTab = (id: string) => {
    closed.add(id);
    saver.cancel(id);
    tabs.close(id);
    entries.delete(id);
    // Süren bir kayıt varsa önce bitsin; kaldırma ondan sonra gider (saveDraft da kapanmışsa yeniden kaldırır).
    void (inflight.get(id) ?? Promise.resolve()).then(() => api.qtDraftsRemove(id)).catch(() => {});
  };

  const closeWith = async (id: string, session: string, policy: ClosePolicy): Promise<"closed" | "openTx" | "failed"> => {
    patch(id, { op: "closing" });
    try {
      await api.qtClose(session, policy);
      return "closed";
    } catch (err) {
      const e = toPayload(err);
      if (e.code === "not_found") return "closed";
      if (e.code === "open_transaction") return "openTx";
      notify(t.query.close.failed(e.message), "error");
      return "failed";
    } finally {
      if (entries.has(id)) patch(id, { op: null });
    }
  };

  const refreshTx = async (id: string) => {
    const s = model(id)?.session;
    if (!s) return;
    try {
      dispatch(id, { type: "txChanged", tx: await api.qtTxState(s) });
    } catch (err) {
      if (routeError(toPayload(err)) === "lost") sessionLost(id);
    }
  };

  const requestClose = async (id: string): Promise<void> => {
    const m = model(id);
    if (!m || m.op === "closing") return;
    let plan = closePlan(m.session, m.run);
    if (plan.kind === "close") {
      if (plan.policy === null || !m.session) return removeTab(id);
      if (plan.busy) notify(t.query.close.closing, "info");
      const r = await closeWith(id, m.session, plan.policy);
      if (r === "closed") return removeTab(id);
      if (r === "failed") return;
      // `refuse` reddedildi: durum bayatmış, açık transaction var → diyalogla sor.
      await refreshTx(id);
      const again = model(id);
      if (!again) return;
      plan = closePlan(again.session, again.run);
      if (plan.kind === "close") plan = { kind: "confirm", choices: ["commit", "rollback", "cancel"], busy: false };
    }
    const cur = model(id);
    if (!cur) return;
    const choice = await closeGate.ask({ title: titleOf(id), choices: plan.choices, failed: cur.run.tx === "failed", busy: plan.busy });
    if (choice === "cancel") return;
    const now = model(id);
    if (!now) return;
    if (!now.session) return removeTab(id);
    if (plan.busy) notify(t.query.close.closing, "info");
    const r = await closeWith(id, now.session, choicePolicy(choice));
    if (r === "closed") return removeTab(id);
    await refreshTx(id);
  };

  // ---- toplu (çıkış / sunucu değişimi)
  const txInfos = (): TabTxInfo[] =>
    tabs.state.tabs.flatMap((x) => {
      const m = model(x.id);
      return m ? [{ id: x.id, title: x.title, serverId: m.serverId, session: m.session, run: m.run }] : [];
    });

  const closeAll = async (list: PendingTx[], choice: "commit" | "rollback") => {
    await Promise.all(
      list.map(async (p) => {
        const s = model(p.id)?.session;
        if (!s) return;
        const r = await closeWith(p.id, s, bulkPolicy(p, choice));
        if (r === "closed") sessionLost(p.id, false);
      }),
    );
  };

  /** Sunucu silme / bağlantı kesme / kilit veya üretim değişimi öncesi: o sunucuda açık transaction varsa sorar. */
  const confirmServerChange = async (serverId: string, name: string): Promise<boolean> => {
    const list = pendingTransactions(txInfos(), serverId);
    if (list.length === 0) return true;
    const choice = await bulkGate.ask({ message: t.query.bulk.serverMessage(name), list });
    if (choice === "cancel") return false;
    await closeAll(list, choice);
    return true;
  };

  /** Arka uç sunucunun oturumlarını kapattı (sil/kes/kilit değişti): sekmeler `lost`; silindiyse "sunucu yok". */
  const serverSessionsClosed = (serverId: string, gone: boolean) => {
    for (const x of txInfos()) {
      if (x.serverId !== serverId) continue;
      if (gone) patch(x.id, { serverGone: true });
      if (x.session) sessionLost(x.id);
    }
    deps.sessionsClosed?.(serverId, gone);
  };

  /** Pencere kapatma isteği. `true`: kapanabilir (taslaklar yazıldı); `false`: kullanıcı vazgeçti. */
  const beforeExit = async (): Promise<boolean> => {
    const list = pendingTransactions(txInfos());
    if (list.length > 0) {
      const choice = await bulkGate.ask({ message: t.query.bulk.exitMessage, list });
      if (choice === "cancel") return false;
      await closeAll(list, choice);
    }
    await saver.flushAll();
    return true;
  };

  // ---- açılışta taslaklar
  const restore = async () => {
    try {
      const [drafts, list] = await Promise.all([api.qtDraftsLoad(), refreshServers()]);
      for (const [d, text] of drafts) {
        if (entries.has(d.id)) continue;
        const r = restoreDraft(d, text, list);
        addTab({ ...r, relation: null, activate: false });
      }
    } catch (err) {
      notify(toPayload(err).message, "error");
    }
  };

  /** "Geçmişi/taslakları temizle" sonrası: açık (sunucusu olan) sekmelerin taslakları hemen yeniden yazılır. */
  const resaveDrafts = async () => {
    await Promise.all(
      [...entries.keys()].map((id) => {
        saver.cancel(id);
        return saveDraft(id);
      }),
    );
  };

  const rename = (id: string, title: string) => {
    tabs.rename(id, title);
    if (entries.has(id)) saver.touch(id); // dashboard sekmesinin taslağı yok
  };

  return {
    tabs,
    servers,
    serverConfig,
    refreshServers,
    everQuery,
    model,
    text: (id: string) => entries.get(id)?.text ?? "",
    setText,
    update,
    patch,
    dispatch,
    nextRunId: () => ++runCounter,
    activate,
    rename,
    openFromNode,
    ensureSession,
    sessionLost,
    setOptions,
    refreshTx,
    requestClose,
    confirmServerChange,
    serverSessionsClosed,
    beforeExit,
    restore,
    resaveDrafts,
    /** Etkin sorgu sekmesinde F5 (genel kısayol) için QueryTab kaydeder. */
    setRunner: (fn: ((kind: "stmt" | "all") => void) | null) => (runner = fn),
    runActive: (kind: "stmt" | "all") => runner?.(kind),
    closeGate,
    bulkGate,
    envGate,
  };
}

export type QueryController = ReturnType<typeof createQueryController>;
