// Görsel EXPLAIN denetleyicisi: sekme başına seçenekler (oturum belleği; depolamaya yazılmaz) ve son plan, `qt_explain`
// akışı (onay, meşgul, hata) ve ızgaradaki kendi EXPLAIN çıktısını açma. Çerçeveye bağlı tek şey sinyallerdir; editör,
// bölme ve bildirimler bağımlılık olarak gelir (QueryTab bağlar). Ana paket modüllerini içe aktarmaz.
import { createSignal } from "solid-js";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ExplainOptions } from "../../ipc/bindings/ExplainOptions";
import type { ExplainResult } from "../../ipc/bindings/ExplainResult";
import type { HistoryEntry } from "../../ipc/bindings/HistoryEntry";
import type { Scope } from "../../ipc/bindings/Scope";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import type { ConfirmItem, Snapshot } from "./confirm-model";
import { DEFAULT_EXPLAIN, effectiveOptions, explainWithGuards, type ExplainKind } from "./explain-model";
import { parsePlan, type PlanNode, type PlanSummary } from "./plan-model";
import type { QueryController, TabNote } from "./query-controller";
import { tplan } from "./strings-plan";

/** Plan sekmesinde gösterilen plan. `key` her yeni planda artar (görünüm durumu sıfırlanır). */
export interface PlanEntry {
  key: number;
  root: PlanNode;
  summary: PlanSummary;
  /** Sunucunun döndürdüğü JSON metni (Ham JSON). */
  json: string;
  source: "explain" | "grid";
  /** Çalıştırılan EXPLAIN metni (ızgaradan açılan planda yok). */
  sql: string | null;
  rolledBack: boolean;
  elapsedMs: number | null;
}

interface TabExplain {
  options: ExplainOptions;
  plan: PlanEntry | null;
}

export type ErrorRouteKind = "toast" | "reset" | "lost" | "message";

export interface ExplainControllerDeps {
  api: {
    qtExplain: (session: string, sql: string, scope: Scope, options: ExplainOptions, confirmed: boolean) => Promise<ExplainResult>;
    qtAnalyze: (session: string, sql: string, scope: Scope) => Promise<StmtPlan[]>;
    qtHistoryPushMany: (entries: HistoryEntry[]) => Promise<number>;
  };
  ctl: Pick<QueryController, "model" | "patch" | "update" | "dispatch" | "ensureSession" | "sessionLost" | "refreshTx">;
  /** Editördeki metin ve kapsam; editör bu sekmeyi göstermiyorsa null (istek yapılmaz). */
  snapshot: (id: string) => Snapshot | null;
  /** İstekten hemen önce (tamamlamayı kapat, hata işaretini sil, denetimi geçersiz kıl). */
  beforeRun?: (id: string) => void;
  confirm: (items: ConfirmItem[], production: boolean) => Promise<boolean>;
  production: (id: string) => boolean;
  /** Sunucu sürümü (`server_version_num` yaklaşığı); bilinmiyorsa null. */
  version: (id: string) => number | null;
  showPane: (id: string, pane: "plan" | "messages") => void;
  /** `workspace-model.routeError`. */
  route: (e: ErrorPayload) => ErrorRouteKind;
  /** `errors.hintFor`. */
  hint: (e: ErrorPayload) => string | null;
  notifyBusy: () => void;
  payload: (e: unknown) => ErrorPayload;
  newId?: () => string;
  now?: () => number;
}

/** Sunucu kilidine (ve imleci kapatmaya) varmadan dönen hatalar; bunlarda ızgaranın imleci açık kalır. */
const BEFORE_LOCK = new Set<string>([
  "busy",
  "locked",
  "confirmation_required",
  "explain_scope",
  "explain_already",
  "explain_unsupported",
  "explain_option",
]);

export function createExplainController(deps: ExplainControllerDeps) {
  const { ctl } = deps;
  const [state, setState] = createSignal<Record<string, TabExplain>>({});
  let planKey = 0;

  const get = (id: string): TabExplain => state()[id] ?? { options: DEFAULT_EXPLAIN, plan: null };
  const put = (id: string, p: Partial<TabExplain>) => setState((s) => ({ ...s, [id]: { ...get(id), ...p } }));
  const addNotes = (id: string, notes: TabNote[]) => {
    if (notes.length > 0) ctl.update(id, (m) => ({ ...m, notes: [...m.notes, ...notes] }));
  };

  const blocked = (id: string): boolean => {
    const m = ctl.model(id);
    if (!m || m.serverGone || m.op !== null) return true;
    return m.run.status === "running" || m.run.status === "cancelling" || m.run.status === "opening";
  };

  const setPlan = (id: string, json: string, extra: Omit<PlanEntry, "key" | "root" | "summary" | "json">): string | null => {
    const parsed = parsePlan(json);
    if ("error" in parsed) return parsed.error;
    put(id, { plan: { key: ++planKey, root: parsed.root, summary: parsed.summary, json, ...extra } });
    return null;
  };

  function onError(id: string, e: ErrorPayload) {
    switch (deps.route(e)) {
      case "toast":
        deps.notifyBusy();
        return;
      case "lost":
        ctl.sessionLost(id);
        deps.showPane(id, "messages");
        return;
      case "reset":
        addNotes(id, [{ level: "info", text: e.message, hint: deps.hint(e) }]);
        deps.showPane(id, "messages");
        return;
      case "message":
        addNotes(id, [{ level: "error", text: tplan.explainFailed(e.message), hint: deps.hint(e) }]);
        deps.showPane(id, "messages");
        return;
    }
  }

  function onDone(id: string, r: ExplainResult) {
    const notes: TabNote[] = [];
    if (r.note) notes.push({ level: "info", text: r.note });
    for (const n of r.notices) {
      notes.push({ level: "info", text: `${n.severity}: ${n.message}${n.detail ? ` (${n.detail})` : ""}`, hint: n.hint });
    }
    const err = setPlan(id, r.json, { source: "explain", sql: r.sql, rolledBack: r.rolledBack, elapsedMs: r.elapsedMs });
    if (err) notes.push({ level: "error", text: err });
    addNotes(id, notes);
    deps.showPane(id, err ? "messages" : "plan");
    // Geçmiş: çalıştırılan EXPLAIN metni (mevcut kural: hassas SQL'i ve kapalı geçmişi arka uç yazmaz).
    const m = ctl.model(id);
    if (m?.serverId && r.sql.trim() !== "") {
      const entry: HistoryEntry = {
        id: (deps.newId ?? (() => crypto.randomUUID()))(),
        sql: r.sql,
        serverId: m.serverId,
        serverName: m.serverName,
        database: m.database,
        atMs: (deps.now ?? Date.now)(),
        elapsedMs: r.elapsedMs,
        ok: true,
      };
      void deps.api
        .qtHistoryPushMany([entry])
        .catch((e) => console.warn("geçmiş yazılamadı", deps.payload(e).code))
        .finally(() => ctl.update(id, (x) => ({ ...x, historyKey: x.historyKey + 1 })));
    }
  }

  /** Editördeki ifadenin planını alır. Sekme meşgulse ya da editör başka sekmedeyse hiçbir şey yapmaz. */
  async function explain(id: string, kind: ExplainKind): Promise<void> {
    if (blocked(id)) return;
    const session = await ctl.ensureSession(id);
    if (!session) return;
    const m = ctl.model(id);
    if (!m || m.run.status !== "idle" || m.op !== null) return;
    // Anlık görüntü oturum hazır olduktan SONRA (parola istemi sırasında yazılanlar dahil).
    const snap = deps.snapshot(id);
    if (!snap) return;
    deps.beforeRun?.(id);
    const production = deps.production(id);
    ctl.patch(id, { op: "explain", notes: [] });
    try {
      const out = await explainWithGuards(snap, {
        session,
        options: effectiveOptions(get(id).options, kind, deps.version(id)),
        explain: deps.api.qtExplain,
        analyze: deps.api.qtAnalyze,
        confirm: (items) => deps.confirm(items, production),
        production,
        payload: deps.payload,
      });
      if (!ctl.model(id)) return; // sekme kapandı
      // Sunucu kilidine ulaşan istek ızgaranın açık imlecini kapatır (`qt_set_options` gibi): "Daha fazla" kapanır.
      if (out.kind === "done" || (out.kind === "rejected" && !BEFORE_LOCK.has(out.error.code))) {
        ctl.dispatch(id, { type: "optionsChanged" });
      }
      if (out.kind === "done") onDone(id, out.result);
      else if (out.kind === "rejected") onError(id, out.error);
    } finally {
      if (ctl.model(id)?.op === "explain") ctl.patch(id, { op: null });
      // ANALYZE manuel modda transaction açabilir; hata transaction'ı bozabilir.
      if (ctl.model(id)) void ctl.refreshTx(id);
    }
  }

  /** Izgaradaki kendi `EXPLAIN (FORMAT JSON)` çıktınızı Plan sekmesinde açar; çözülemezse önceki plan kalır. */
  function openJson(id: string, json: string): boolean {
    const err = setPlan(id, json, { source: "grid", sql: null, rolledBack: false, elapsedMs: null });
    if (err) {
      addNotes(id, [{ level: "error", text: err }]);
      deps.showPane(id, "messages");
      return false;
    }
    deps.showPane(id, "plan");
    return true;
  }

  return {
    options: (id: string) => get(id).options,
    setOption: (id: string, key: keyof ExplainOptions, value: boolean) => put(id, { options: { ...get(id).options, [key]: value } }),
    plan: (id: string) => get(id).plan,
    explain,
    openJson,
    /** Kapanan sekmelerin kayıtları atılır. */
    prune: (alive: Set<string>) => {
      const s = state();
      if (Object.keys(s).some((k) => !alive.has(k))) {
        setState(Object.fromEntries(Object.entries(s).filter(([k]) => alive.has(k))));
      }
    },
  };
}

export type ExplainController = ReturnType<typeof createExplainController>;
